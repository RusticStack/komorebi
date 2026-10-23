//! The Contacts view (`ui/contacts.slint`): the list over contact ids with
//! search and a group filter, the details of one contact, the inline
//! editor, and "New message to". Nothing here runs at launch: the view
//! loads when it first opens. The sync pane lives in `contacts_sync.rs`.

use super::*;
use crate::{ContactItem, ContactLine, Contacts};
use komorebi::contacts::{Contact, Email, Invalid, Phone};
use komorebi::db::contacts::BOOK_READ_ONLY;

/// The rows built for the list at most (the rest show after a search).
const MAX_ROWS: usize = 10_000;

/// The view's state on the UI thread.
#[derive(Default)]
pub(super) struct ContactsState {
    /// The list, in order.
    ids: Vec<i64>,
    /// The group of each choice after "All contacts".
    groups: Vec<i64>,
    group: Option<i64>,
    query: String,
    /// The contact shown, and the one being edited (`Some(0)`: a new one).
    shown: Option<Contact>,
    pub(super) editing: Option<Contact>,
    /// Searches wait for typing to pause.
    search_timer: Option<slint::Timer>,
    pub(super) sync: super::contacts_sync::SyncState,
}

fn lines_of<T>(items: &[T], f: impl Fn(&T) -> (String, String)) -> ModelRc<ContactLine> {
    let rows: Vec<ContactLine> = items
        .iter()
        .map(|i| {
            let (text, label) = f(i);
            ContactLine {
                text: text.into(),
                label: label_word(&label).into(),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// A vCard TYPE shown as a translated word.
fn label_word(label: &str) -> String {
    match label {
        "work" => t!(invoke_contact_label_work),
        "home" => t!(invoke_contact_label_home),
        "cell" | "mobile" => t!(invoke_contact_label_mobile),
        "" => String::new(),
        other => other.to_string(),
    }
}

impl App {
    /// The Contacts view opened or closed.
    pub(super) fn contacts_toggled(&mut self, open: bool) {
        if open {
            self.reload_contacts(true);
            self.contacts_kick();
        } else {
            self.contacts.editing = None;
            self.ui().global::<Contacts>().set_editing(false);
        }
    }

    /// Read the groups and the list again, keeping the contact shown.
    pub(super) fn reload_contacts(&mut self, groups: bool) {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        if groups {
            let list = self.db.contact_groups(None).unwrap_or_default();
            if self
                .contacts
                .group
                .is_some_and(|id| !list.iter().any(|x| x.id == id))
            {
                self.contacts.group = None;
            }
            let mut names: Vec<SharedString> = vec![t!(invoke_contacts_all).into()];
            names.extend(list.iter().map(|x| SharedString::from(x.name.as_str())));
            self.contacts.groups = list.iter().map(|x| x.id).collect();
            let at = self
                .contacts
                .group
                .and_then(|id| self.contacts.groups.iter().position(|x| *x == id))
                .map_or(0, |i| i as i32 + 1);
            g.set_groups(ModelRc::new(VecModel::from(names)));
            g.set_group_index(at);
        }
        self.contacts.ids = self
            .db
            .contact_ids(None, self.contacts.group, &self.contacts.query)
            .unwrap_or_default();
        let shown: &[i64] = self
            .contacts
            .ids
            .get(..MAX_ROWS)
            .unwrap_or(&self.contacts.ids);
        let rows: Vec<ContactItem> = self
            .db
            .contact_rows(shown)
            .unwrap_or_default()
            .into_iter()
            .map(|r| ContactItem {
                id: r.id as i32,
                initials: format::initials(&r.name, &r.email).into(),
                color: format::avatar_color(if r.email.is_empty() {
                    &r.name
                } else {
                    &r.email
                }),
                name: r.name.into(),
                email: r.email.into(),
            })
            .collect();
        g.set_total(self.contacts.ids.len() as i32);
        g.set_rows(ModelRc::new(VecModel::from(rows)));
        let keep = self.contacts.shown.as_ref().map(|c| c.id);
        match keep.and_then(|id| self.contacts.ids.iter().position(|x| *x == id)) {
            Some(i) => self.select_contact(i as i32),
            None => self.select_contact(-1),
        }
    }

    pub(super) fn contacts_search(&mut self, text: &str) {
        self.contacts.query = text.to_string();
        let timer = self
            .contacts
            .search_timer
            .get_or_insert_with(slint::Timer::default);
        timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(150),
            || {
                with_app(|a| a.reload_contacts(false));
            },
        );
    }

    pub(super) fn contacts_group(&mut self, index: i32) {
        self.contacts.group = usize::try_from(index - 1)
            .ok()
            .and_then(|i| self.contacts.groups.get(i))
            .copied();
        self.reload_contacts(false);
    }

    /// Show the contact at list position `index` (-1: none).
    pub(super) fn select_contact(&mut self, index: i32) {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        let contact = usize::try_from(index)
            .ok()
            .and_then(|i| self.contacts.ids.get(i))
            .and_then(|id| self.db.contact(*id).ok().flatten());
        g.set_current(if contact.is_some() { index } else { -1 });
        let Some(c) = contact else {
            self.contacts.shown = None;
            return;
        };
        let book = self.db.address_book(c.book_id).ok().flatten();
        let name = c.display();
        let first = c
            .preferred_email()
            .map(|e| e.addr.clone())
            .unwrap_or_default();
        g.set_name(name.as_str().into());
        g.set_initials(format::initials(&name, &first).into());
        g.set_color(format::avatar_color(if first.is_empty() {
            &name
        } else {
            &first
        }));
        let org = match (c.title.is_empty(), c.org.is_empty()) {
            (false, false) => t!(
                invoke_contact_title_at,
                c.title.as_str().into(),
                c.org.as_str().into()
            ),
            (false, true) => c.title.clone(),
            (true, _) => c.org.clone(),
        };
        g.set_org_line(org.into());
        g.set_emails(lines_of(&c.emails, |e| (e.addr.clone(), e.label.clone())));
        g.set_phones(lines_of(&c.phones, |p| (p.number.clone(), p.label.clone())));
        g.set_note(c.note.as_str().into());
        g.set_group_line(c.groups.join(", ").into());
        g.set_book(
            book.as_ref()
                .filter(|b| b.account_id.is_some())
                .map(|b| b.name.as_str())
                .unwrap_or("")
                .into(),
        );
        g.set_read_only(book.is_some_and(|b| b.mode == BOOK_READ_ONLY));
        g.set_conflict(self.db.contact_conflict(c.id).ok().flatten().is_some());
        self.contacts.shown = Some(c);
    }

    /// Fill the editor from `c` and show it.
    fn edit_contact_form(&mut self, c: Contact) {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        g.set_e_name(c.name.as_str().into());
        g.set_e_given(c.given.as_str().into());
        g.set_e_family(c.family.as_str().into());
        g.set_e_org(c.org.as_str().into());
        g.set_e_title(c.title.as_str().into());
        g.set_e_note(c.note.as_str().into());
        g.set_e_groups(c.groups.join(", ").into());
        let mut emails: Vec<SharedString> =
            c.emails.iter().map(|e| e.addr.as_str().into()).collect();
        if emails.is_empty() {
            emails.push(SharedString::new());
        }
        g.set_e_emails(ModelRc::new(VecModel::from(emails)));
        let phones: Vec<SharedString> = c.phones.iter().map(|p| p.number.as_str().into()).collect();
        g.set_e_phones(ModelRc::new(VecModel::from(phones)));
        g.set_e_error(SharedString::new());
        g.set_editing(true);
        self.contacts.editing = Some(c);
    }

    pub(super) fn new_contact(&mut self) {
        let Ok(book) = self.db.local_book() else {
            return;
        };
        self.contacts.sync.close(&self.ui());
        self.edit_contact_form(Contact {
            book_id: book,
            ..Contact::default()
        });
    }

    pub(super) fn edit_contact(&mut self) {
        if let Some(c) = self.contacts.shown.clone() {
            self.edit_contact_form(c);
        }
    }

    /// The editor's fields as a contact (emails and phones keep their
    /// labels by position).
    fn contact_from_form(&self) -> Option<Contact> {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        let mut c = self.contacts.editing.clone()?;
        c.name = g.get_e_name().to_string();
        c.given = g.get_e_given().to_string();
        c.family = g.get_e_family().to_string();
        c.org = g.get_e_org().to_string();
        c.title = g.get_e_title().to_string();
        c.note = g.get_e_note().to_string();
        c.groups = g
            .get_e_groups()
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();
        let old = std::mem::take(&mut c.emails);
        c.emails = g
            .get_e_emails()
            .iter()
            .enumerate()
            .map(|(i, a)| Email {
                addr: a.to_string(),
                ..old.get(i).cloned().unwrap_or_default()
            })
            .collect();
        let old = std::mem::take(&mut c.phones);
        c.phones = g
            .get_e_phones()
            .iter()
            .enumerate()
            .map(|(i, n)| Phone {
                number: n.to_string(),
                ..old.get(i).cloned().unwrap_or_default()
            })
            .collect();
        Some(c)
    }

    pub(super) fn save_contact(&mut self) {
        let Some(mut c) = self.contact_from_form() else {
            return;
        };
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        if let Err(e) = c.clone().normalize() {
            let msg = match e {
                Invalid::Empty => t!(invoke_contact_empty),
                Invalid::Email(i) => t!(
                    invoke_contact_bad_email,
                    c.emails
                        .get(i)
                        .map_or("", |e| e.addr.as_str())
                        .trim()
                        .into()
                ),
            };
            g.set_e_error(msg.into());
            return;
        }
        match self.db.save_contact(&mut c) {
            Ok(_) => {
                self.contacts.editing = None;
                g.set_editing(false);
                self.contacts.shown = Some(c);
                self.contacts.query.clear();
                g.set_query(SharedString::new());
                self.reload_contacts(true);
                self.contacts_kick();
            }
            Err(e) => g.set_e_error(text::detail(&e).into()),
        }
    }

    /// Cancel: ask first when something changed.
    pub(super) fn cancel_contact(&mut self) {
        let changed = match (&self.contacts.editing, self.contact_from_form()) {
            (Some(before), Some(now)) => {
                let (mut a, mut b) = (before.clone(), now);
                a.normalize().ok();
                b.normalize().ok();
                b.uid.clone_from(&a.uid);
                a != b
            }
            _ => false,
        };
        if changed {
            self.ui().global::<Contacts>().set_confirm_discard(true);
        } else {
            self.discard_contact();
        }
    }

    pub(super) fn discard_contact(&mut self) {
        self.contacts.editing = None;
        self.ui().global::<Contacts>().set_editing(false);
    }

    pub(super) fn delete_contact(&mut self) {
        let Some(c) = self.contacts.shown.take() else {
            return;
        };
        if let Err(e) = self.db.delete_contact(c.id) {
            self.set_status(&text::detail(&e), true);
        }
        self.reload_contacts(true);
        self.contacts_kick();
    }

    /// Change one repeatable row of the editor (`emails`: which list).
    pub(super) fn edit_contact_row(&mut self, emails: bool, op: RowOp) {
        let ui = self.ui();
        let g = ui.global::<Contacts>();
        let model = if emails {
            g.get_e_emails()
        } else {
            g.get_e_phones()
        };
        let mut rows: Vec<SharedString> = model.iter().collect();
        if let Some(c) = self.contacts.editing.as_mut() {
            match op {
                RowOp::Add => {
                    rows.push(SharedString::new());
                }
                RowOp::Remove(i) if i < rows.len() => {
                    rows.remove(i);
                    if emails && i < c.emails.len() {
                        c.emails.remove(i);
                    } else if !emails && i < c.phones.len() {
                        c.phones.remove(i);
                    }
                }
                RowOp::Set(i, text) => {
                    if let Some(r) = rows.get_mut(i) {
                        *r = text.into();
                    }
                    // The text box keeps its own text: no new model.
                    if let Some(vm) = model.as_any().downcast_ref::<VecModel<SharedString>>() {
                        vm.set_row_data(i, rows.get(i).cloned().unwrap_or_default());
                    }
                    return;
                }
                RowOp::Remove(_) => return,
            }
        }
        let rows = ModelRc::new(VecModel::from(rows));
        if emails {
            g.set_e_emails(rows);
        } else {
            g.set_e_phones(rows);
        }
    }

    /// New message to `addr`, or to the shown contact's preferred address
    /// (empty `addr`), from the account that owns the book (the default
    /// account for the local book).
    pub(super) fn write_to_contact(&mut self, addr: &str) {
        let Some(c) = self.contacts.shown.clone() else {
            return;
        };
        let addr = if addr.is_empty() {
            match c.preferred_email() {
                Some(e) => e.addr.clone(),
                None => return,
            }
        } else {
            addr.to_string()
        };
        let account = self
            .db
            .address_book(c.book_id)
            .ok()
            .flatten()
            .and_then(|b| b.account_id)
            .filter(|id| self.accounts.iter().any(|a| a.id == *id))
            .or_else(|| self.accounts.first().map(|a| a.id));
        let Some(account) = account else {
            return self.set_status(&t!(invoke_add_account_write), false);
        };
        self.ui().set_show_contacts(false);
        self.start_draft(composer::Draft {
            to: mail::smtp::mailbox(&c.display(), &addr),
            ..composer::Draft::new(account)
        });
    }

    pub(super) fn copy_contact_text(&mut self, text: &str) {
        self.ui().invoke_r_copy_text(text.into());
        self.set_status(&t!(invoke_contact_copied), false);
    }

    /// Keep the local version of a conflicted contact: it replaces the
    /// server's at the next push.
    pub(super) fn keep_mine(&mut self) {
        let Some(c) = self.contacts.shown.clone() else {
            return;
        };
        if let Some(raw) = self.db.contact_conflict(c.id).ok().flatten()
            && let Ok(card) = mail::vcard::parse(&raw)
        {
            let mut mine = card.contact;
            mine.id = c.id;
            mine.book_id = c.book_id;
            mine.uid = c.uid.clone();
            if self.db.save_contact(&mut mine).is_ok() {
                self.db.set_contact_conflict(c.id, None).ok();
                self.contacts_kick();
            }
        }
        self.reload_contacts(true);
    }

    pub(super) fn keep_server(&mut self) {
        if let Some(c) = &self.contacts.shown {
            self.db.set_contact_conflict(c.id, None).ok();
        }
        self.reload_contacts(false);
    }
}

/// A change to one repeatable row of the editor.
pub(super) enum RowOp {
    Add,
    Remove(usize),
    Set(usize, String),
}

pub(super) fn wire(ui: &AppWindow) {
    ui.on_contacts_toggled(|open| {
        with_app(|a| a.contacts_toggled(open));
    });
    let g = ui.global::<Contacts>();
    g.on_search(|t| {
        with_app(|a| a.contacts_search(&t));
    });
    g.on_group_chosen(|i| {
        with_app(|a| a.contacts_group(i));
    });
    g.on_select(|i| {
        with_app(|a| a.select_contact(i));
    });
    g.on_new_contact(|| {
        with_app(|a| a.new_contact());
    });
    g.on_edit(|| {
        with_app(|a| a.edit_contact());
    });
    g.on_delete_confirmed(|| {
        with_app(|a| a.delete_contact());
    });
    g.on_save(|| {
        with_app(|a| a.save_contact());
    });
    g.on_cancel(|| {
        with_app(|a| a.cancel_contact());
    });
    g.on_discard(|| {
        with_app(|a| a.discard_contact());
    });
    g.on_add_email(|| {
        with_app(|a| a.edit_contact_row(true, RowOp::Add));
    });
    g.on_remove_email(|i| {
        with_app(|a| a.edit_contact_row(true, RowOp::Remove(i.max(0) as usize)));
    });
    g.on_email_edited(|i, t| {
        with_app(|a| a.edit_contact_row(true, RowOp::Set(i.max(0) as usize, t.to_string())));
    });
    g.on_add_phone(|| {
        with_app(|a| a.edit_contact_row(false, RowOp::Add));
    });
    g.on_remove_phone(|i| {
        with_app(|a| a.edit_contact_row(false, RowOp::Remove(i.max(0) as usize)));
    });
    g.on_phone_edited(|i, t| {
        with_app(|a| a.edit_contact_row(false, RowOp::Set(i.max(0) as usize, t.to_string())));
    });
    g.on_write_to(|addr| {
        with_app(|a| a.write_to_contact(&addr));
    });
    g.on_copy(|t| {
        with_app(|a| a.copy_contact_text(&t));
    });
    g.on_keep_mine(|| {
        with_app(|a| a.keep_mine());
    });
    g.on_keep_server(|| {
        with_app(|a| a.keep_server());
    });
    super::contacts_sync::wire(ui);
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=contacts`, `contact-edit`,
/// `contacts-sync`): made-up contacts in the throwaway cache (offline
/// only), and the view open on the first one.
pub fn sample_contacts(action: &str) {
    with_app(|a| {
        if a.db.contact_ids(None, None, "").map_or(0, |v| v.len()) == 0 {
            let Ok(book) = a.db.local_book() else {
                return;
            };
            let people = [
                (
                    "Ana Conceição",
                    "ana.conceicao@example.pt",
                    "Mutualismo",
                    "Família",
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
                    "Clínica do Bairro",
                    "Família",
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
                        "Coordenação".into()
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
                a.db.save_contact(&mut c).ok();
            }
        }
        let ui = a.ui();
        ui.set_show_contacts(true);
        a.contacts_toggled(true);
        a.select_contact(0);
        match action {
            "contact-edit" => a.edit_contact(),
            "contacts-sync" => a.open_sync_pane(),
            _ => {}
        }
    });
}
