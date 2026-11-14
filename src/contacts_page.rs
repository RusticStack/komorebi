//! The Contacts view without a toolkit: what both apps' contacts pages do.
//! The list over contact ids (search and a group filter), the details of
//! the contact shown, the editor's form and how it becomes a valid
//! [`Contact`], delete, the conflict choice, and who "New message to"
//! writes to and from which account.
//!
//! No sentences here (the library never words anything): a failed save is a
//! [`Problem`], a writing target a [`Compose`]. The address-book sync is
//! `contacts_sync`.

use crate::contacts::{Contact, Email, Invalid, Phone};
use crate::db::contacts::{BOOK_READ_ONLY, ContactRow, Group};
use crate::db::{Account, Db};
use anyhow::Result;

/// The rows built for the list at most (the rest show after a search).
pub const MAX_ROWS: usize = 10_000;

/// The contact shown, with what its details pane says about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    pub contact: Contact,
    /// The name shown ([`Contact::display`]).
    pub name: String,
    /// The preferred address (empty: none): the avatar's initials and
    /// colour come from it when the name is empty.
    pub first_email: String,
    /// The synced address book's name; empty for the local book.
    pub book: String,
    /// The book syncs read only: no edit, no delete.
    pub read_only: bool,
    /// The server changed it while the user edited it.
    pub conflict: bool,
}

impl Detail {
    /// `contact` with the facts the cache has about its book.
    pub fn load(db: &Db, contact: Contact) -> Self {
        let book = db.address_book(contact.book_id).ok().flatten();
        let conflict = db.contact_conflict(contact.id).ok().flatten().is_some();
        Self {
            name: contact.display(),
            first_email: contact
                .preferred_email()
                .map(|e| e.addr.clone())
                .unwrap_or_default(),
            book: book
                .as_ref()
                .filter(|b| b.account_id.is_some())
                .map(|b| b.name.clone())
                .unwrap_or_default(),
            read_only: book.is_some_and(|b| b.mode == BOOK_READ_ONLY),
            conflict,
            contact,
        }
    }
}

/// The list's state: read again with [`Page::reload`].
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// Every contact of the list, in order.
    pub ids: Vec<i64>,
    /// The groups after "All contacts".
    pub groups: Vec<Group>,
    /// The group filter (none: all contacts).
    pub group: Option<i64>,
    pub query: String,
    /// The rows built (at most [`MAX_ROWS`]).
    pub rows: Vec<ContactRow>,
    shown: Option<Detail>,
    /// A contact to show after the next reload (one just saved).
    want: Option<i64>,
}

impl Page {
    /// Read the list again (and the groups when `groups`), keeping the
    /// contact shown when it is still in the list.
    pub fn reload(&mut self, db: &Db, groups: bool) {
        if groups {
            self.groups = db.contact_groups(None).unwrap_or_default();
            if self
                .group
                .is_some_and(|id| !self.groups.iter().any(|g| g.id == id))
            {
                self.group = None;
            }
        }
        self.ids = db
            .contact_ids(None, self.group, &self.query)
            .unwrap_or_default();
        let shown = self.ids.get(..MAX_ROWS).unwrap_or(&self.ids);
        self.rows = db.contact_rows(shown).unwrap_or_default();
        let keep = self
            .want
            .take()
            .or_else(|| self.shown.as_ref().map(|d| d.contact.id));
        let at = keep.and_then(|id| self.ids.iter().position(|x| *x == id));
        self.select(db, at);
    }

    /// The group filter by position in the picker: 0 is "All contacts".
    pub fn pick_group(&mut self, db: &Db, index: usize) {
        self.group = index
            .checked_sub(1)
            .and_then(|i| self.groups.get(i))
            .map(|g| g.id);
        self.reload(db, false);
    }

    /// The group filter's position in the picker.
    pub fn group_index(&self) -> usize {
        self.group
            .and_then(|id| self.groups.iter().position(|g| g.id == id))
            .map_or(0, |i| i + 1)
    }

    /// Show the contact at list position `index` (none: nothing).
    pub fn select(&mut self, db: &Db, index: Option<usize>) {
        self.shown = index
            .and_then(|i| self.ids.get(i))
            .and_then(|id| db.contact(*id).ok().flatten())
            .map(|c| Detail::load(db, c));
    }

    /// Show contact `id` once the list is read again (a contact just saved).
    pub fn show_after_reload(&mut self, id: i64) {
        self.want = Some(id);
    }

    pub fn shown(&self) -> Option<&Detail> {
        self.shown.as_ref()
    }

    /// The list position of the contact shown.
    pub fn current(&self) -> Option<usize> {
        let id = self.shown.as_ref()?.contact.id;
        self.ids.iter().position(|x| *x == id)
    }

    /// Contacts in the list (rows built or not).
    pub fn total(&self) -> usize {
        self.ids.len()
    }

    /// Delete the contact shown, and read the list again.
    pub fn delete_shown(&mut self, db: &Db) -> Result<()> {
        let Some(d) = self.shown.take() else {
            return Ok(());
        };
        let result = db.delete_contact(d.contact.id);
        self.reload(db, true);
        result
    }

    /// Read the contact shown again (its conflict was settled).
    pub fn refresh_shown(&mut self, db: &Db) {
        self.reload(db, false);
    }
}

/// What is wrong with a contact that cannot be saved yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// "Add a name, an email address or a phone number."
    Empty,
    /// This is not an email address (as typed, trimmed).
    Email(String),
}

/// The editor: the form's fields over the contact being edited, or a new
/// one in the local book.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Form {
    /// What the form does not show (id, book, uid, the labels of the emails
    /// and phones, which stay by position); removing a line removes its
    /// label too.
    base: Contact,
    /// The contact as the form opened, for [`Form::changed`].
    original: Contact,
    pub name: String,
    pub given: String,
    pub family: String,
    pub org: String,
    pub title: String,
    pub note: String,
    /// The groups, separated by commas.
    pub groups: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

impl Form {
    /// A blank form for a new contact in book `book_id`.
    pub fn blank(book_id: i64) -> Self {
        Self::open(&Contact {
            book_id,
            ..Contact::default()
        })
    }

    /// The form for `c`: it always has an email line to type in.
    pub fn open(c: &Contact) -> Self {
        let mut emails: Vec<String> = c.emails.iter().map(|e| e.addr.clone()).collect();
        if emails.is_empty() {
            emails.push(String::new());
        }
        Self {
            base: c.clone(),
            original: c.clone(),
            name: c.name.clone(),
            given: c.given.clone(),
            family: c.family.clone(),
            org: c.org.clone(),
            title: c.title.clone(),
            note: c.note.clone(),
            groups: c.groups.join(", "),
            emails,
            phones: c.phones.iter().map(|p| p.number.clone()).collect(),
        }
    }

    /// Whether this edits a saved contact (false: a new one).
    pub fn is_saved(&self) -> bool {
        self.base.id != 0
    }

    /// The form's fields as a contact (emails and phones keep their labels
    /// by position), not yet checked.
    pub fn contact(&self) -> Contact {
        let mut c = self.base.clone();
        c.name.clone_from(&self.name);
        c.given.clone_from(&self.given);
        c.family.clone_from(&self.family);
        c.org.clone_from(&self.org);
        c.title.clone_from(&self.title);
        c.note.clone_from(&self.note);
        c.groups = self
            .groups
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        let old = std::mem::take(&mut c.emails);
        c.emails = self
            .emails
            .iter()
            .enumerate()
            .map(|(i, a)| Email {
                addr: a.clone(),
                ..old.get(i).cloned().unwrap_or_default()
            })
            .collect();
        let old = std::mem::take(&mut c.phones);
        c.phones = self
            .phones
            .iter()
            .enumerate()
            .map(|(i, n)| Phone {
                number: n.clone(),
                ..old.get(i).cloned().unwrap_or_default()
            })
            .collect();
        c
    }

    /// The contact, trimmed and checked, or what is wrong with it.
    pub fn check(&self) -> Result<Contact, Problem> {
        let c = self.contact();
        let mut normal = c.clone();
        match normal.normalize() {
            Ok(()) => Ok(normal),
            Err(Invalid::Empty) => Err(Problem::Empty),
            Err(Invalid::Email(i)) => Err(Problem::Email(
                c.emails
                    .get(i)
                    .map_or(String::new(), |e| e.addr.trim().to_string()),
            )),
        }
    }

    /// Save to the cache. The error is the cache's.
    pub fn save(&self, db: &Db) -> Result<Result<Contact, Problem>> {
        let mut c = match self.check() {
            Ok(c) => c,
            Err(p) => return Ok(Err(p)),
        };
        db.save_contact(&mut c)?;
        Ok(Ok(c))
    }

    /// Whether the user changed anything since the form opened (Cancel asks
    /// before it throws that away).
    pub fn changed(&self) -> bool {
        let (mut before, mut now) = (self.original.clone(), self.contact());
        before.normalize().ok();
        now.normalize().ok();
        now.uid.clone_from(&before.uid);
        before != now
    }

    pub fn add_email(&mut self) {
        self.emails.push(String::new());
    }

    pub fn remove_email(&mut self, i: usize) {
        if i < self.emails.len() {
            self.emails.remove(i);
            if i < self.base.emails.len() {
                self.base.emails.remove(i);
            }
        }
    }

    pub fn set_email(&mut self, i: usize, text: &str) {
        if let Some(e) = self.emails.get_mut(i) {
            *e = text.to_string();
        }
    }

    pub fn add_phone(&mut self) {
        self.phones.push(String::new());
    }

    pub fn remove_phone(&mut self, i: usize) {
        if i < self.phones.len() {
            self.phones.remove(i);
            if i < self.base.phones.len() {
                self.base.phones.remove(i);
            }
        }
    }

    pub fn set_phone(&mut self, i: usize, text: &str) {
        if let Some(p) = self.phones.get_mut(i) {
            *p = text.to_string();
        }
    }
}

/// A new contact's form, in the local book.
pub fn new_form(db: &Db) -> Option<Form> {
    db.local_book().ok().map(Form::blank)
}

/// Where "New message to" goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compose {
    /// The contact has no address to write to.
    Nothing,
    /// No mail account to write from: "Add an account first".
    NoAccount,
    /// A new draft from `account` to `to` (a mailbox with the name).
    To { account: i64, to: String },
}

/// New message to `addr`, or to `c`'s preferred address (empty `addr`),
/// from the account that owns the contact's book (the first account for
/// the local book).
pub fn compose_to(db: &Db, accounts: &[Account], c: &Contact, addr: &str) -> Compose {
    let addr = if addr.is_empty() {
        match c.preferred_email() {
            Some(e) => e.addr.clone(),
            None => return Compose::Nothing,
        }
    } else {
        addr.to_string()
    };
    let account = db
        .address_book(c.book_id)
        .ok()
        .flatten()
        .and_then(|b| b.account_id)
        .filter(|id| accounts.iter().any(|a| a.id == *id))
        .or_else(|| accounts.first().map(|a| a.id));
    match account {
        Some(account) => Compose::To {
            account,
            to: crate::mail::smtp::mailbox(&c.display(), &addr),
        },
        None => Compose::NoAccount,
    }
}

/// Keep the local version of a conflicted contact: it replaces the
/// server's at the next push. True when it was saved (the caller starts a
/// sync).
pub fn keep_mine(db: &Db, c: &Contact) -> bool {
    let Some(raw) = db.contact_conflict(c.id).ok().flatten() else {
        return false;
    };
    let Ok(card) = crate::mail::vcard::parse(&raw) else {
        return false;
    };
    let mut mine = card.contact;
    mine.id = c.id;
    mine.book_id = c.book_id;
    mine.uid.clone_from(&c.uid);
    if db.save_contact(&mut mine).is_ok() {
        db.set_contact_conflict(c.id, None).ok();
        return true;
    }
    false
}

/// Keep the server's version: the local one is let go.
pub fn keep_server(db: &Db, c: &Contact) {
    db.set_contact_conflict(c.id, None).ok();
}

/// Made-up contacts for snapshots, in the local book, when it is empty.
/// Offline only is the caller's rule (they are written to the cache).
pub fn sample(db: &Db) {
    if db.contact_ids(None, None, "").map_or(0, |v| v.len()) != 0 {
        return;
    }
    let Ok(book) = db.local_book() else {
        return;
    };
    let people = [
        (
            "Ana Concei\u{e7}\u{e3}o",
            "ana.conceicao@example.pt",
            "Mutualismo",
            "Fam\u{ed}lia",
        ),
        ("Bruno Esteves", "bruno@example.com", "", "Trabalho"),
        (
            "Carla Mendes",
            "carla.mendes@example.org",
            "Biblioteca Municipal",
            "Trabalho",
        ),
        ("Diogo Ferreira", "diogo@example.net", "", ""),
        (
            "Eva Rodrigues",
            "eva.rodrigues@example.pt",
            "Cl\u{ed}nica do Bairro",
            "Fam\u{ed}lia",
        ),
        ("Filipe Sousa", "filipe.sousa@example.com", "", ""),
    ];
    for (name, addr, org, group) in people {
        let mut parts = name.splitn(2, ' ');
        let mut c = Contact {
            book_id: book,
            name: name.into(),
            given: parts.next().unwrap_or("").into(),
            family: parts.next().unwrap_or("").into(),
            org: org.into(),
            title: if org.is_empty() {
                String::new()
            } else {
                "Coordena\u{e7}\u{e3}o".into()
            },
            note: if org.is_empty() {
                String::new()
            } else {
                "Prefers mail in the morning.".into()
            },
            emails: vec![Email {
                addr: addr.into(),
                label: "work".into(),
                pref: true,
            }],
            phones: vec![Phone {
                number: "+351 912 345 678".into(),
                label: "cell".into(),
            }],
            groups: if group.is_empty() {
                Vec::new()
            } else {
                vec![group.into()]
            },
            ..Contact::default()
        };
        db.save_contact(&mut c).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::in_memory().expect("cache")
    }

    fn person(book: i64, name: &str, addr: &str, group: &str) -> Contact {
        Contact {
            book_id: book,
            name: name.into(),
            emails: vec![Email {
                addr: addr.into(),
                label: "work".into(),
                pref: true,
            }],
            groups: if group.is_empty() {
                Vec::new()
            } else {
                vec![group.into()]
            },
            ..Contact::default()
        }
    }

    #[test]
    fn the_list_searches_filters_and_keeps_the_contact_shown() {
        let db = db();
        sample(&db);
        let mut page = Page::default();
        page.reload(&db, true);
        assert_eq!(page.total(), 6);
        assert_eq!(page.rows.len(), 6);
        assert!(page.shown().is_none());
        assert_eq!(page.groups.len(), 2);

        page.select(&db, Some(2));
        let name = page.shown().expect("shown").name.clone();
        assert_eq!(page.current(), Some(2));

        // A search narrows the list; the contact shown stays when it matches.
        page.query = name.split(' ').next().unwrap_or("").to_string();
        page.reload(&db, false);
        assert_eq!(page.total(), 1);
        assert_eq!(page.current(), Some(0));
        page.query = "nobody".into();
        page.reload(&db, false);
        assert_eq!(page.total(), 0);
        assert!(page.shown().is_none());

        // A group filter by position: 0 is all contacts.
        page.query.clear();
        page.pick_group(&db, 1);
        assert!(page.group.is_some());
        assert_eq!(page.group_index(), 1);
        assert!(page.total() > 0 && page.total() < 6);
        page.pick_group(&db, 0);
        assert_eq!((page.group, page.total()), (None, 6));
    }

    #[test]
    fn delete_removes_the_contact_shown() {
        let db = db();
        sample(&db);
        let mut page = Page::default();
        page.reload(&db, true);
        page.select(&db, Some(0));
        page.delete_shown(&db).expect("deleted");
        assert_eq!(page.total(), 5);
        assert!(page.shown().is_none());
    }

    #[test]
    fn a_form_saves_a_new_contact_and_says_what_is_wrong() {
        let db = db();
        let book = db.local_book().expect("book");
        let mut form = Form::blank(book);
        assert_eq!(form.emails, [""]);
        assert!(!form.is_saved());
        assert!(!form.changed());
        assert_eq!(form.check(), Err(Problem::Empty));

        form.name = "Marta Silva".into();
        form.set_email(0, " Marta@Example.PT ");
        assert!(form.changed());
        form.add_email();
        form.set_email(1, "not an address");
        assert_eq!(form.check(), Err(Problem::Email("not an address".into())));
        form.remove_email(1);
        form.groups = "Family, Work, family".into();
        form.add_phone();
        form.set_phone(0, "+351 912 345 678");

        let saved = form.save(&db).expect("store").expect("valid");
        assert_eq!(saved.emails[0].addr, "marta@example.pt");
        assert_eq!(saved.groups, ["Family", "Work"]);
        assert_eq!(saved.phones.len(), 1);
        assert!(saved.id > 0);

        let mut page = Page::default();
        page.show_after_reload(saved.id);
        page.reload(&db, true);
        assert_eq!(page.shown().expect("shown").name, "Marta Silva");
    }

    #[test]
    fn editing_keeps_labels_by_position_and_notices_changes() {
        let db = db();
        let book = db.local_book().expect("book");
        let mut c = person(book, "Rui", "rui@example.com", "");
        c.emails.push(Email {
            addr: "rui@work.example".into(),
            label: "home".into(),
            pref: false,
        });
        db.save_contact(&mut c).expect("saved");
        let mut form = Form::open(&c);
        assert!(form.is_saved());
        assert!(!form.changed());
        form.remove_email(0);
        // The second address is now the first and keeps its own label.
        let now = form.contact();
        assert_eq!(now.emails.len(), 1);
        assert_eq!(now.emails[0].label, "home");
        assert!(form.changed());
        let mut again = Form::open(&c);
        again.name = "Rui Mendes".into();
        assert!(again.changed());
        again.name = "Rui".into();
        assert!(!again.changed());
    }

    #[test]
    fn writing_goes_to_the_preferred_address_from_the_books_account() {
        let mut db = db();
        let seeded = crate::seed::seed(&mut db, 1).expect("seed");
        let accounts = db.accounts().expect("accounts");
        let book = db.local_book().expect("book");
        let mut c = person(book, "Marta Silva", "marta@example.pt", "");
        db.save_contact(&mut c).expect("saved");
        assert_eq!(
            compose_to(&db, &accounts, &c, ""),
            Compose::To {
                account: seeded.account,
                to: "Marta Silva <marta@example.pt>".into()
            }
        );
        assert_eq!(
            compose_to(&db, &accounts, &c, "other@example.org"),
            Compose::To {
                account: seeded.account,
                to: "Marta Silva <other@example.org>".into()
            }
        );
        assert_eq!(compose_to(&db, &[], &c, ""), Compose::NoAccount);
        let none = Contact::default();
        assert_eq!(compose_to(&db, &accounts, &none, ""), Compose::Nothing);
    }

    #[test]
    fn the_detail_says_when_a_book_is_read_only_or_synced() {
        let db = db();
        let book = db.local_book().expect("book");
        let mut c = person(book, "Ana", "ana@example.pt", "");
        db.save_contact(&mut c).expect("saved");
        let d = Detail::load(&db, c);
        assert_eq!(d.name, "Ana");
        assert_eq!(d.first_email, "ana@example.pt");
        assert!(d.book.is_empty() && !d.read_only && !d.conflict);
    }
}
