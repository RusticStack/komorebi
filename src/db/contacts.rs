//! The contacts store: address books (the local one "On this computer" and
//! the CardDAV books an account syncs), their contacts, groups, and the
//! per-account DAV settings. User data, not cache: a future "clean slate"
//! migration must keep these tables. The derived `addresses` index is not
//! touched here; [`Db::suggest_recipients`] reads it last, as a fallback.

use super::{Db, Recipient};
use crate::contacts::{self, Contact, Email, Phone};
use crate::fold::fold;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::{HashMap, HashSet};

/// The tables (fresh installs and the v14 to v15 step). Idempotent, so a
/// migration that runs over an older copy of them changes nothing.
pub(super) const CONTACTS: &str = r#"
CREATE TABLE IF NOT EXISTS address_books (
    id INTEGER PRIMARY KEY,
    -- NULL: the local book, shared by every account.
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL DEFAULT '',
    href TEXT,
    sync_token TEXT,
    ctag TEXT,
    -- 0 local, 1 read only, 2 read and write.
    mode INTEGER NOT NULL DEFAULT 0,
    vcard_version INTEGER NOT NULL DEFAULT 3,
    enabled INTEGER NOT NULL DEFAULT 1,
    -- 1: the server lists sync-collection.
    sync_collection INTEGER NOT NULL DEFAULT 0,
    last_sync INTEGER,
    last_error INTEGER
);
CREATE INDEX IF NOT EXISTS address_books_account ON address_books(account_id);
CREATE TABLE IF NOT EXISTS contacts (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES address_books(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    href TEXT,
    etag TEXT,
    fn TEXT NOT NULL DEFAULT '',
    given TEXT NOT NULL DEFAULT '',
    family TEXT NOT NULL DEFAULT '',
    sort_key TEXT NOT NULL DEFAULT '',
    org TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    -- The server's vCard as last synced: the base a patch and a merge
    -- start from.
    raw BLOB,
    -- 0 clean, 1 changed, 2 new, 3 deleted (waiting to be pushed).
    dirty INTEGER NOT NULL DEFAULT 0,
    updated INTEGER NOT NULL DEFAULT 0,
    UNIQUE (book_id, uid)
);
CREATE INDEX IF NOT EXISTS contacts_sort ON contacts(sort_key);
CREATE INDEX IF NOT EXISTS contacts_href ON contacts(book_id, href);
CREATE TABLE IF NOT EXISTS contact_emails (
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    addr TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    pref INTEGER NOT NULL DEFAULT 0,
    pos INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS contact_emails_contact ON contact_emails(contact_id);
CREATE TABLE IF NOT EXISTS contact_phones (
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    number TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    pos INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS contact_phones_contact ON contact_phones(contact_id);
CREATE TABLE IF NOT EXISTS contact_terms (
    term TEXT NOT NULL,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS contact_terms_term ON contact_terms(term);
CREATE INDEX IF NOT EXISTS contact_terms_contact ON contact_terms(contact_id);
CREATE TABLE IF NOT EXISTS contact_groups (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES address_books(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    folded TEXT NOT NULL,
    uid TEXT,
    href TEXT,
    etag TEXT,
    -- 0 from CATEGORIES, 1 a KIND:group card (read only).
    kind INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS contact_groups_book ON contact_groups(book_id, folded);
CREATE TABLE IF NOT EXISTS contact_group_members (
    group_id INTEGER NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, contact_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS contact_group_members_contact ON contact_group_members(contact_id);
-- A local edit the server overrode (both sides changed a field): the
-- local version as JSON-free text lines, until the user picks one.
CREATE TABLE IF NOT EXISTS contact_conflicts (
    contact_id INTEGER PRIMARY KEY REFERENCES contacts(id) ON DELETE CASCADE,
    local BLOB NOT NULL
);
-- Per-account DAV settings, the one definition shared by CardDAV and
-- CalDAV (the calendar's schema does not create it again).
CREATE TABLE IF NOT EXISTS dav_accounts (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    url TEXT NOT NULL DEFAULT '',
    user TEXT NOT NULL DEFAULT '',
    pin BLOB,
    -- Discovery results: the principal, the calendar and address book
    -- homes, and when they were found (unix seconds).
    principal TEXT,
    cal_home TEXT,
    card_home TEXT,
    found_at INTEGER,
    -- Contacts: 0 off, 1 read only, 2 read and write.
    mode INTEGER NOT NULL DEFAULT 0,
    -- 1: the password is `account:{id}:dav`, not the mail password.
    separate_secret INTEGER NOT NULL DEFAULT 0,
    interval_min INTEGER NOT NULL DEFAULT 30,
    -- 1: the user allowed a server on the local network.
    allow_private INTEGER NOT NULL DEFAULT 0
);
INSERT INTO address_books (account_id, name, mode)
    SELECT NULL, '', 0
    WHERE NOT EXISTS (SELECT 1 FROM address_books WHERE account_id IS NULL AND mode = 0);
"#;

/// Book modes.
pub const BOOK_LOCAL: i64 = 0;
pub const BOOK_READ_ONLY: i64 = 1;
pub const BOOK_READ_WRITE: i64 = 2;

/// `contacts.dirty`.
pub const CLEAN: i64 = 0;
pub const CHANGED: i64 = 1;
pub const NEW: i64 = 2;
pub const DELETED: i64 = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Book {
    pub id: i64,
    pub account_id: Option<i64>,
    pub name: String,
    pub href: Option<String>,
    pub sync_token: Option<String>,
    pub ctag: Option<String>,
    pub mode: i64,
    pub vcard_version: i64,
    pub enabled: bool,
    pub sync_collection: bool,
    pub last_sync: Option<i64>,
}

/// A row of the Contacts list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactRow {
    pub id: i64,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub members: i64,
    /// A `KIND:group` card: shown, not edited.
    pub read_only: bool,
}

/// What a suggestion is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestKind {
    /// A contact's address.
    Person,
    /// A group: choosing it adds every member.
    Group(Vec<Recipient>),
    /// An address from the cached headers.
    Seen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggested {
    pub name: String,
    /// Empty for a group.
    pub addr: String,
    pub kind: SuggestKind,
}

/// id, href, etag, raw, dirty.
type DirtyRow = (i64, Option<String>, Option<String>, Option<Vec<u8>>, i64);

/// A contact waiting to be pushed to its server.
#[derive(Debug, Clone, Default)]
pub struct Dirty {
    pub contact: Contact,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub raw: Option<Vec<u8>>,
    pub dirty: i64,
}

/// The DAV settings of one account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavAccount {
    pub account_id: i64,
    pub url: String,
    pub user: String,
    pub pin: Option<Vec<u8>>,
    pub mode: i64,
    pub separate_secret: bool,
    pub interval_min: i64,
    pub allow_private: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn book_of(r: &rusqlite::Row) -> rusqlite::Result<Book> {
    Ok(Book {
        id: r.get(0)?,
        account_id: r.get(1)?,
        name: r.get(2)?,
        href: r.get(3)?,
        sync_token: r.get(4)?,
        ctag: r.get(5)?,
        mode: r.get(6)?,
        vcard_version: r.get(7)?,
        enabled: r.get(8)?,
        sync_collection: r.get(9)?,
        last_sync: r.get(10)?,
    })
}

const BOOK_COLS: &str = "id, account_id, name, href, sync_token, ctag, mode, vcard_version,
    enabled, sync_collection, last_sync";

/// Write `c`'s child rows, terms and groups (inside the caller's
/// transaction), replacing any there.
fn write_children(tx: &rusqlite::Connection, id: i64, book_id: i64, c: &Contact) -> Result<()> {
    for sql in [
        "DELETE FROM contact_emails WHERE contact_id = ?1",
        "DELETE FROM contact_phones WHERE contact_id = ?1",
        "DELETE FROM contact_terms WHERE contact_id = ?1",
        "DELETE FROM contact_group_members WHERE contact_id = ?1
            AND group_id IN (SELECT id FROM contact_groups WHERE kind = 0)",
    ] {
        tx.prepare_cached(sql)?.execute([id])?;
    }
    for (i, e) in c.emails.iter().enumerate() {
        tx.prepare_cached(
            "INSERT INTO contact_emails (contact_id, addr, label, pref, pos) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![id, e.addr, e.label, e.pref, i as i64])?;
    }
    for (i, p) in c.phones.iter().enumerate() {
        tx.prepare_cached(
            "INSERT INTO contact_phones (contact_id, number, label, pos) VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![id, p.number, p.label, i as i64])?;
    }
    for t in contacts::terms(c) {
        tx.prepare_cached("INSERT INTO contact_terms (term, contact_id) VALUES (?1, ?2)")?
            .execute(params![t, id])?;
    }
    for g in &c.groups {
        let folded = fold(g).into_owned();
        let gid: Option<i64> = tx
            .prepare_cached(
                "SELECT id FROM contact_groups WHERE book_id = ?1 AND folded = ?2 AND kind = 0",
            )?
            .query_row(params![book_id, folded], |r| r.get(0))
            .optional()?;
        let gid = match gid {
            Some(g) => g,
            None => {
                tx.prepare_cached(
                    "INSERT INTO contact_groups (book_id, name, folded, kind) VALUES (?1, ?2, ?3, 0)",
                )?
                .execute(params![book_id, g, folded])?;
                tx.last_insert_rowid()
            }
        };
        tx.prepare_cached(
            "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id) VALUES (?1, ?2)",
        )?
        .execute(params![gid, id])?;
    }
    // Groups nobody is in any more go.
    tx.prepare_cached(
        "DELETE FROM contact_groups WHERE book_id = ?1 AND kind = 0
           AND NOT EXISTS (SELECT 1 FROM contact_group_members m WHERE m.group_id = contact_groups.id)",
    )?
    .execute([book_id])?;
    Ok(())
}

impl Db {
    /// The local book's id ("On this computer").
    pub fn local_book(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT id FROM address_books WHERE account_id IS NULL AND mode = 0 ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )?)
    }

    /// The books: the local one first, then each account's by name.
    pub fn address_books(&self) -> Result<Vec<Book>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {BOOK_COLS} FROM address_books ORDER BY account_id IS NOT NULL, account_id, name"
        ))?;
        let rows = stmt.query_map([], book_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn address_book(&self, id: i64) -> Result<Option<Book>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {BOOK_COLS} FROM address_books WHERE id = ?1"
            ))?
            .query_row([id], book_of)
            .optional()?)
    }

    /// Ids of the contacts in `book` (all books: `None`) and `group`, whose
    /// terms start with each folded word of `query`, in list order.
    pub fn contact_ids(
        &self,
        book: Option<i64>,
        group: Option<i64>,
        query: &str,
    ) -> Result<Vec<i64>> {
        let q = fold(query.trim()).into_owned();
        let words: Vec<&str> = q.split_whitespace().take(4).collect();
        let mut sql = String::from(
            "SELECT c.id FROM contacts c JOIN address_books b ON b.id = c.book_id
             WHERE c.dirty <> 3 AND b.enabled = 1 AND (?1 IS NULL OR c.book_id = ?1)
               AND (?2 IS NULL OR c.id IN (SELECT contact_id FROM contact_group_members WHERE group_id = ?2))",
        );
        let mut args: Vec<String> = Vec::new();
        for w in &words {
            let n = args.len();
            sql.push_str(&format!(
                " AND c.id IN (SELECT contact_id FROM contact_terms WHERE term >= ?{} AND term < ?{})",
                n + 3,
                n + 4
            ));
            args.push((*w).to_string());
            args.push(format!("{w}{}", char::MAX));
        }
        sql.push_str(" ORDER BY c.sort_key, c.id");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut all: Vec<&dyn rusqlite::ToSql> = vec![&book, &group];
        for a in &args {
            all.push(a);
        }
        let rows = stmt.query_map(all.as_slice(), |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The list rows for `ids` (in that order; missing ones left out).
    pub fn contact_rows(&self, ids: &[i64]) -> Result<Vec<ContactRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.fn, c.given, c.family, c.org,
                (SELECT addr FROM contact_emails e WHERE e.contact_id = c.id ORDER BY pref DESC, pos LIMIT 1)
             FROM contacts c WHERE c.id = ?1",
        )?;
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            let row = stmt
                .query_row([id], |r| {
                    let c = Contact {
                        name: r.get(0)?,
                        given: r.get(1)?,
                        family: r.get(2)?,
                        org: r.get(3)?,
                        ..Contact::default()
                    };
                    let email: Option<String> = r.get(4)?;
                    let email = email.unwrap_or_default();
                    let mut name = c.display();
                    if name.is_empty() {
                        name.clone_from(&email);
                    }
                    Ok(ContactRow { id, name, email })
                })
                .optional()?;
            out.extend(row);
        }
        Ok(out)
    }

    /// One contact with its emails, phones and groups.
    pub fn contact(&self, id: i64) -> Result<Option<Contact>> {
        let c = self
            .conn
            .prepare_cached(
                "SELECT book_id, uid, fn, given, family, org, title, note FROM contacts WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok(Contact {
                    id,
                    book_id: r.get(0)?,
                    uid: r.get(1)?,
                    name: r.get(2)?,
                    given: r.get(3)?,
                    family: r.get(4)?,
                    org: r.get(5)?,
                    title: r.get(6)?,
                    note: r.get(7)?,
                    ..Contact::default()
                })
            })
            .optional()?;
        let Some(mut c) = c else {
            return Ok(None);
        };
        c.emails = self
            .conn
            .prepare_cached(
                "SELECT addr, label, pref FROM contact_emails WHERE contact_id = ?1 ORDER BY pos",
            )?
            .query_map([id], |r| {
                Ok(Email {
                    addr: r.get(0)?,
                    label: r.get(1)?,
                    pref: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        c.phones = self
            .conn
            .prepare_cached(
                "SELECT number, label FROM contact_phones WHERE contact_id = ?1 ORDER BY pos",
            )?
            .query_map([id], |r| {
                Ok(Phone {
                    number: r.get(0)?,
                    label: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        c.groups = self
            .conn
            .prepare_cached(
                "SELECT g.name FROM contact_groups g JOIN contact_group_members m ON m.group_id = g.id
                 WHERE m.contact_id = ?1 AND g.kind = 0 ORDER BY g.folded",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(c))
    }

    /// Save `c` (new when `c.id == 0`) after [`Contact::normalize`]: one
    /// transaction for the row, its emails, phones, terms and groups. In a
    /// synced book the contact is marked to be pushed. Returns the id.
    pub fn save_contact(&self, c: &mut Contact) -> Result<i64> {
        c.normalize()
            .map_err(|e| anyhow::anyhow!("invalid contact: {e:?}"))?;
        let tx = self.write()?;
        let mode: i64 = tx.query_row(
            "SELECT mode FROM address_books WHERE id = ?1",
            [c.book_id],
            |r| r.get(0),
        )?;
        anyhow::ensure!(mode != BOOK_READ_ONLY, "read-only address book");
        let key = contacts::sort_key(c);
        let id = if c.id == 0 {
            let dirty = if mode == BOOK_LOCAL { CLEAN } else { NEW };
            tx.prepare_cached(
                "INSERT INTO contacts (book_id, uid, fn, given, family, sort_key, org, title, note, dirty, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?
            .execute(params![
                c.book_id, c.uid, c.name, c.given, c.family, key, c.org, c.title, c.note, dirty,
                now()
            ])?;
            tx.last_insert_rowid()
        } else {
            tx.prepare_cached(
                "UPDATE contacts SET fn = ?2, given = ?3, family = ?4, sort_key = ?5, org = ?6,
                   title = ?7, note = ?8, updated = ?9,
                   dirty = CASE WHEN ?10 = 0 THEN 0 WHEN dirty = 2 THEN 2 ELSE 1 END
                 WHERE id = ?1",
            )?
            .execute(params![
                c.id,
                c.name,
                c.given,
                c.family,
                key,
                c.org,
                c.title,
                c.note,
                now(),
                mode
            ])?;
            c.id
        };
        write_children(&tx, id, c.book_id, c)?;
        tx.commit()?;
        c.id = id;
        Ok(id)
    }

    /// Delete a contact: at once in the local book (or one never pushed),
    /// otherwise marked for the next push.
    pub fn delete_contact(&self, id: i64) -> Result<()> {
        let tx = self.write()?;
        let row: Option<(i64, i64)> = tx
            .query_row(
                "SELECT b.mode, c.dirty FROM contacts c JOIN address_books b ON b.id = c.book_id WHERE c.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((BOOK_READ_ONLY, _)) => anyhow::bail!("read-only address book"),
            Some((BOOK_LOCAL, _) | (_, NEW)) => {
                tx.execute("DELETE FROM contacts WHERE id = ?1", [id])?;
            }
            Some(_) => {
                tx.execute("UPDATE contacts SET dirty = 3 WHERE id = ?1", [id])?;
            }
            None => {}
        }
        tx.execute(
            "DELETE FROM contact_groups WHERE kind = 0
               AND NOT EXISTS (SELECT 1 FROM contact_group_members m JOIN contacts c ON c.id = m.contact_id
                               WHERE m.group_id = contact_groups.id AND c.dirty <> 3)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The groups of `book` (all books: `None`), by name, with how many
    /// contacts each has.
    pub fn contact_groups(&self, book: Option<i64>) -> Result<Vec<Group>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT g.id, g.name, g.kind,
                (SELECT count(*) FROM contact_group_members m JOIN contacts c ON c.id = m.contact_id
                 WHERE m.group_id = g.id AND c.dirty <> 3)
             FROM contact_groups g JOIN address_books b ON b.id = g.book_id
             WHERE b.enabled = 1 AND (?1 IS NULL OR g.book_id = ?1) ORDER BY g.folded, g.id",
        )?;
        let rows = stmt.query_map([book], |r| {
            Ok(Group {
                id: r.get(0)?,
                name: r.get(1)?,
                read_only: r.get::<_, i64>(2)? == 1,
                members: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The members of group `id`, each with the preferred address.
    pub fn group_members(&self, id: i64) -> Result<Vec<Recipient>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.fn, c.given, c.family, c.org,
                (SELECT addr FROM contact_emails e WHERE e.contact_id = c.id ORDER BY pref DESC, pos LIMIT 1) a
             FROM contact_group_members m JOIN contacts c ON c.id = m.contact_id
             WHERE m.group_id = ?1 AND c.dirty <> 3 AND a IS NOT NULL ORDER BY c.sort_key",
        )?;
        let rows = stmt.query_map([id], |r| {
            let c = Contact {
                name: r.get(0)?,
                given: r.get(1)?,
                family: r.get(2)?,
                org: r.get(3)?,
                ..Contact::default()
            };
            Ok(Recipient {
                name: c.display(),
                addr: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Recipient suggestions for `typed` in a message from `account_id`:
    /// that account's books and the local book first (each contact's
    /// preferred address first), then groups by name, then the derived
    /// index of seen addresses. An address shows once, with the contact's
    /// name. With no contacts this is exactly `suggest_addresses`.
    pub fn suggest_recipients(
        &self,
        account_id: i64,
        typed: &str,
        limit: usize,
    ) -> Result<Vec<Suggested>> {
        let low = fold(typed.trim()).into_owned();
        if low.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let high = format!("{low}{}", char::MAX);
        let mut out: Vec<Suggested> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        {
            let mut stmt = self.conn.prepare_cached(
                "SELECT c.fn, c.given, c.family, c.org, e.addr
                 FROM (SELECT DISTINCT t.contact_id id FROM contact_terms t
                       WHERE t.term >= ?2 AND t.term < ?3 LIMIT 200) hit
                 JOIN contacts c ON c.id = hit.id
                 JOIN address_books b ON b.id = c.book_id
                 JOIN contact_emails e ON e.contact_id = c.id
                 WHERE b.enabled = 1 AND c.dirty <> 3 AND (b.account_id = ?1 OR b.account_id IS NULL)
                 ORDER BY c.sort_key, c.id, e.pref DESC, e.pos LIMIT ?4",
            )?;
            let rows = stmt.query_map(params![account_id, low, high, (limit * 2) as i64], |r| {
                let c = Contact {
                    name: r.get(0)?,
                    given: r.get(1)?,
                    family: r.get(2)?,
                    org: r.get(3)?,
                    ..Contact::default()
                };
                Ok((c.display(), r.get::<_, String>(4)?))
            })?;
            for row in rows {
                let (name, addr) = row?;
                if seen.insert(addr.clone()) {
                    out.push(Suggested {
                        name,
                        addr,
                        kind: SuggestKind::Person,
                    });
                }
            }
        }
        let groups: Vec<(i64, String)> = self
            .conn
            .prepare_cached(
                "SELECT g.id, g.name FROM contact_groups g JOIN address_books b ON b.id = g.book_id
                 WHERE b.enabled = 1 AND (b.account_id = ?1 OR b.account_id IS NULL)
                   AND g.folded >= ?2 AND g.folded < ?3 ORDER BY g.folded LIMIT ?4",
            )?
            .query_map(params![account_id, low, high, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (id, name) in groups {
            let members = self.group_members(id)?;
            if !members.is_empty() {
                out.push(Suggested {
                    name,
                    addr: String::new(),
                    kind: SuggestKind::Group(members),
                });
            }
        }
        for r in self.suggest_addresses(account_id, typed, limit + seen.len())? {
            if seen.insert(r.addr.clone()) {
                out.push(Suggested {
                    name: r.name,
                    addr: r.addr,
                    kind: SuggestKind::Seen,
                });
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    // ---- DAV ------------------------------------------------------------

    pub fn dav_account(&self, account_id: i64) -> Result<Option<DavAccount>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT url, user, pin, mode, separate_secret, interval_min, allow_private
                 FROM dav_accounts WHERE account_id = ?1",
            )?
            .query_row([account_id], |r| {
                Ok(DavAccount {
                    account_id,
                    url: r.get(0)?,
                    user: r.get(1)?,
                    pin: r.get(2)?,
                    mode: r.get(3)?,
                    separate_secret: r.get(4)?,
                    interval_min: r.get(5)?,
                    allow_private: r.get(6)?,
                })
            })
            .optional()?)
    }

    /// Store the DAV settings. Turning contacts off removes the account's
    /// synced books (and their contacts) from this computer; the server is
    /// never touched.
    pub fn set_dav_account(&self, d: &DavAccount) -> Result<()> {
        let tx = self.write()?;
        tx.execute(
            "INSERT INTO dav_accounts (account_id, url, user, pin, mode, separate_secret, interval_min, allow_private)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (account_id) DO UPDATE SET url = ?2, user = ?3, pin = ?4, mode = ?5,
               separate_secret = ?6, interval_min = ?7, allow_private = ?8",
            params![
                d.account_id, d.url, d.user, d.pin, d.mode, d.separate_secret, d.interval_min,
                d.allow_private
            ],
        )?;
        if d.mode == 0 {
            tx.execute(
                "DELETE FROM address_books WHERE account_id = ?1",
                [d.account_id],
            )?;
        } else {
            tx.execute(
                "UPDATE address_books SET mode = ?2 WHERE account_id = ?1",
                params![d.account_id, d.mode],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Make the account's synced books the ones found on the server
    /// (`(href, name, vcard 4 supported)`): new ones are added (enabled),
    /// gone ones removed with their contacts, the rest renamed.
    pub fn set_found_books(
        &self,
        account_id: i64,
        mode: i64,
        found: &[(String, String, bool)],
    ) -> Result<()> {
        let tx = self.write()?;
        let have: Vec<(i64, String)> = tx
            .prepare("SELECT id, href FROM address_books WHERE account_id = ?1")?
            .query_map([account_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for (id, href) in &have {
            if !found.iter().any(|(h, _, _)| h == href) {
                tx.execute("DELETE FROM address_books WHERE id = ?1", [id])?;
            }
        }
        for (href, name, v4) in found {
            let version = if *v4 { 4 } else { 3 };
            match have.iter().find(|(_, h)| h == href) {
                Some((id, _)) => {
                    tx.execute(
                        "UPDATE address_books SET name = ?2, vcard_version = ?3, mode = ?4 WHERE id = ?1",
                        params![id, name, version, mode],
                    )?;
                }
                None => {
                    tx.execute(
                        "INSERT INTO address_books (account_id, name, href, mode, vcard_version, enabled)
                         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                        params![account_id, name, href, mode, version],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Turn sync off (or on) without touching the synced books: Off with
    /// "Keep" leaves them on this computer, no longer synced.
    pub fn set_dav_mode(&self, account_id: i64, mode: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE dav_accounts SET mode = ?2 WHERE account_id = ?1",
            params![account_id, mode],
        )?;
        Ok(())
    }

    pub fn set_book_enabled(&self, book_id: i64, on: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE address_books SET enabled = ?2 WHERE id = ?1",
            params![book_id, on],
        )?;
        Ok(())
    }

    /// After a pull: the new sync token or ctag, and whether the server
    /// offers sync-collection.
    pub fn set_book_state(
        &self,
        book_id: i64,
        token: Option<&str>,
        ctag: Option<&str>,
        sync_collection: bool,
        error: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE address_books SET sync_token = ?2, ctag = ?3, sync_collection = ?4,
               last_sync = CASE WHEN ?5 IS NULL THEN ?6 ELSE last_sync END, last_error = ?5 WHERE id = ?1",
            params![book_id, token, ctag, sync_collection, error, now()],
        )?;
        Ok(())
    }

    /// The href and ETag of every contact and group card in `book_id`.
    pub fn book_etags(&self, book_id: i64) -> Result<HashMap<String, String>> {
        let mut out = HashMap::new();
        for sql in [
            "SELECT href, etag FROM contacts WHERE book_id = ?1 AND href IS NOT NULL",
            "SELECT href, etag FROM contact_groups WHERE book_id = ?1 AND href IS NOT NULL",
        ] {
            let mut stmt = self.conn.prepare_cached(sql)?;
            let rows = stmt.query_map([book_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })?;
            for row in rows {
                let (h, e) = row?;
                out.insert(h, e);
            }
        }
        Ok(out)
    }

    /// Store one pulled batch in one transaction: each `(href, etag, raw,
    /// card)` is upserted by href (a person, or a `KIND:group` card), and
    /// each href in `gone` is removed. A contact with an unpushed local
    /// change is left alone and its href returned, for the merge.
    pub fn store_cards(
        &self,
        book_id: i64,
        cards: &[(String, String, Vec<u8>, crate::mail::vcard::Card)],
        gone: &[String],
    ) -> Result<Vec<String>> {
        let tx = self.write()?;
        let mut held = Vec::new();
        for (href, etag, raw, card) in cards {
            if card.group {
                let name = card.contact.display();
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM contact_groups WHERE book_id = ?1 AND href = ?2",
                        params![book_id, href],
                        |r| r.get(0),
                    )
                    .optional()?;
                let gid = match existing {
                    Some(id) => {
                        tx.execute(
                            "UPDATE contact_groups SET name = ?2, folded = ?3, etag = ?4, uid = ?5 WHERE id = ?1",
                            params![id, name, fold(&name), etag, card.contact.uid],
                        )?;
                        tx.execute(
                            "DELETE FROM contact_group_members WHERE group_id = ?1",
                            [id],
                        )?;
                        id
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO contact_groups (book_id, name, folded, uid, href, etag, kind)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                            params![book_id, name, fold(&name), card.contact.uid, href, etag],
                        )?;
                        tx.last_insert_rowid()
                    }
                };
                for m in &card.members {
                    tx.execute(
                        "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id)
                         SELECT ?1, id FROM contacts WHERE book_id = ?2 AND uid = ?3",
                        params![gid, book_id, m],
                    )?;
                }
                continue;
            }
            let mut c = card.contact.clone();
            c.book_id = book_id;
            if c.uid.is_empty() {
                c.uid.clone_from(href);
            }
            let existing: Option<(i64, i64)> = tx
                .query_row(
                    "SELECT id, dirty FROM contacts WHERE book_id = ?1 AND (href = ?2 OR uid = ?3)",
                    params![book_id, href, c.uid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let key = contacts::sort_key(&c);
            let id = match existing {
                Some((_, dirty)) if dirty != CLEAN => {
                    held.push(href.clone());
                    continue;
                }
                Some((id, _)) => {
                    tx.execute(
                        "UPDATE contacts SET uid = ?2, href = ?3, etag = ?4, fn = ?5, given = ?6, family = ?7,
                           sort_key = ?8, org = ?9, title = ?10, note = ?11, raw = ?12, updated = ?13 WHERE id = ?1",
                        params![
                            id, c.uid, href, etag, c.name, c.given, c.family, key, c.org, c.title, c.note,
                            raw, now()
                        ],
                    )?;
                    id
                }
                None => {
                    tx.execute(
                        "INSERT INTO contacts (book_id, uid, href, etag, fn, given, family, sort_key, org, title, note, raw, updated)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                        params![
                            book_id, c.uid, href, etag, c.name, c.given, c.family, key, c.org, c.title,
                            c.note, raw, now()
                        ],
                    )?;
                    tx.last_insert_rowid()
                }
            };
            write_children(&tx, id, book_id, &c)?;
        }
        for href in gone {
            let dirty: Option<i64> = tx
                .query_row(
                    "SELECT dirty FROM contacts WHERE book_id = ?1 AND href = ?2",
                    params![book_id, href],
                    |r| r.get(0),
                )
                .optional()?;
            if matches!(dirty, Some(d) if d != CLEAN && d != DELETED) {
                held.push(href.clone());
                continue;
            }
            tx.execute(
                "DELETE FROM contacts WHERE book_id = ?1 AND href = ?2",
                params![book_id, href],
            )?;
            tx.execute(
                "DELETE FROM contact_groups WHERE book_id = ?1 AND href = ?2",
                params![book_id, href],
            )?;
        }
        tx.commit()?;
        Ok(held)
    }

    /// The contacts of `book_id` waiting to be pushed.
    pub fn dirty_contacts(&self, book_id: i64) -> Result<Vec<Dirty>> {
        let rows: Vec<DirtyRow> = self
            .conn
            .prepare_cached(
                "SELECT id, href, etag, raw, dirty FROM contacts WHERE book_id = ?1 AND dirty <> 0 ORDER BY id",
            )?
            .query_map([book_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (id, href, etag, raw, dirty) in rows {
            if let Some(contact) = self.contact(id)? {
                out.push(Dirty {
                    contact,
                    href,
                    etag,
                    raw,
                    dirty,
                });
            }
        }
        Ok(out)
    }

    /// A push went through: the contact is clean with the server's href,
    /// ETag and card (`raw`), or gone when it was a delete.
    pub fn contact_pushed(
        &self,
        id: i64,
        href: &str,
        etag: Option<&str>,
        raw: Option<&[u8]>,
    ) -> Result<()> {
        let dirty: Option<i64> = self
            .conn
            .query_row("SELECT dirty FROM contacts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if dirty == Some(DELETED) {
            self.conn
                .execute("DELETE FROM contacts WHERE id = ?1", [id])?;
        } else {
            self.conn.execute(
                "UPDATE contacts SET dirty = 0, href = ?2, etag = ?3, raw = coalesce(?4, raw) WHERE id = ?1",
                params![id, href, etag, raw],
            )?;
        }
        Ok(())
    }

    /// Mark a contact as changed again (to push it as new after the server
    /// removed it, or after a merge).
    pub fn set_contact_dirty(&self, id: i64, dirty: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE contacts SET dirty = ?2 WHERE id = ?1",
            params![id, dirty],
        )?;
        Ok(())
    }

    /// Keep the local version of a contact the server overrode.
    pub fn set_contact_conflict(&self, id: i64, local: Option<&[u8]>) -> Result<()> {
        match local {
            Some(l) => self.conn.execute(
                "INSERT OR REPLACE INTO contact_conflicts (contact_id, local) VALUES (?1, ?2)",
                params![id, l],
            )?,
            None => self
                .conn
                .execute("DELETE FROM contact_conflicts WHERE contact_id = ?1", [id])?,
        };
        Ok(())
    }

    pub fn contact_conflict(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT local FROM contact_conflicts WHERE contact_id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn contact_by_href(&self, book_id: i64, href: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM contacts WHERE book_id = ?1 AND href = ?2",
                params![book_id, href],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The card last synced for a contact (the merge base).
    pub fn contact_raw(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT raw FROM contacts WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    /// The server's card is now the base of a contact (after a merge).
    pub fn set_contact_synced(&self, id: i64, href: &str, etag: &str, raw: &[u8]) -> Result<()> {
        self.conn.execute(
            "UPDATE contacts SET href = ?2, etag = ?3, raw = ?4 WHERE id = ?1",
            params![id, href, etag, raw],
        )?;
        Ok(())
    }

    /// The server removed a contact that has a local edit: push it again
    /// as new.
    pub fn forget_href(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE contacts SET href = NULL, etag = NULL, raw = NULL, dirty = 2 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Forget a contact's ETag, so the next pull fetches it again.
    pub fn clear_etag(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE contacts SET etag = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    /// How many contacts `account_id`'s synced books hold.
    pub fn synced_contact_count(&self, account_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM contacts c JOIN address_books b ON b.id = c.book_id
             WHERE b.account_id = ?1 AND c.dirty <> 3",
            [account_id],
            |r| r.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::vcard;

    fn account(db: &Db) -> i64 {
        let s = crate::providers::Server {
            host: "imap.example.com".into(),
            port: 993,
            security: crate::providers::Security::Tls,
        };
        db.add_account(&super::super::NewAccount {
            email: "ana@example.com".into(),
            name: "Ana".into(),
            imap: s.clone(),
            smtp: s,
            save_sent: true,
            smtp_no_auth: false,
            imap_user: String::new(),
            smtp_user: String::new(),
        })
        .unwrap()
    }

    fn person(book: i64, name: &str, addr: &str) -> Contact {
        Contact {
            book_id: book,
            name: name.into(),
            emails: vec![Email {
                addr: addr.into(),
                ..Email::default()
            }],
            ..Contact::default()
        }
    }

    /// Undo v15 (contacts).
    pub(crate) const DROP_V15: &str = "DROP TABLE contact_group_members; DROP TABLE contact_groups;
        DROP TABLE contact_terms; DROP TABLE contact_phones; DROP TABLE contact_emails;
        DROP TABLE contact_conflicts; DROP TABLE contacts; DROP TABLE address_books;
        DROP TABLE dav_accounts;";
    /// Undo v18 (SMTP without sign-in), the newest.
    const DROP_V18: &str = "ALTER TABLE accounts DROP COLUMN smtp_no_auth;";
    /// Undo v17 (the calendar, `cal::store`), after v18.
    const DROP_V17: &str = "DROP TABLE alarms_done; DROP TABLE cal_queue; DROP TABLE occurrences;
        DROP TABLE events; DROP TABLE calendars;";
    /// Undo v16 (rules, `db::rule_store`), which comes after v15.
    const DROP_V16: &str = "DROP TABLE rule_log; DROP TABLE rule_conditions; DROP TABLE rules;
        ALTER TABLE folders DROP COLUMN rules_uidvalidity;
        ALTER TABLE folders DROP COLUMN rules_uidnext;";

    fn schema(db: &Db) -> Vec<String> {
        db.conn
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn v14_upgrades_with_a_local_book_and_untouched_addresses() {
        let dir =
            std::env::temp_dir().join(format!("komorebi-contacts-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.sqlite3");
        std::fs::remove_file(&path).ok();
        let fresh = {
            let db = Db::open(&path).unwrap();
            let acc = account(&db);
            db.conn
                .execute_batch(&format!(
                    "INSERT INTO addresses (account_id, addr, name, folded, last_seen, weight)
                     VALUES ({acc}, 'a@b.pt', 'A', 'a', 1, 1);
                     {DROP_V18} {DROP_V17} {DROP_V16} {DROP_V15} PRAGMA user_version = 14;"
                ))
                .unwrap();
            Db::in_memory().map(|m| schema(&m)).unwrap()
        };
        let db = Db::open(&path).unwrap();
        assert_eq!(schema(&db), fresh);
        assert!(db.local_book().is_ok());
        assert_eq!(db.address_books().unwrap().len(), 1);
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM addresses", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_search_group_and_delete() {
        let db = Db::in_memory().unwrap();
        let local = db.local_book().unwrap();
        let mut a = person(local, "João Conceição", "joao@example.pt");
        a.groups = vec!["Família".into()];
        let a_id = db.save_contact(&mut a).unwrap();
        let mut b = person(local, "Beatriz Lopes", "bia@example.org");
        b.family = "Lopes".into();
        db.save_contact(&mut b).unwrap();

        assert_eq!(db.contact_ids(None, None, "").unwrap().len(), 2);
        assert_eq!(db.contact_ids(None, None, "joao").unwrap(), [a_id]);
        assert_eq!(db.contact_ids(None, None, "CONCEI jo").unwrap(), [a_id]);
        assert_eq!(db.contact_ids(None, None, "example.org").unwrap().len(), 1);
        let groups = db.contact_groups(None).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, 1);
        assert_eq!(
            db.contact_ids(None, Some(groups[0].id), "").unwrap(),
            [a_id]
        );

        let got = db.contact(a_id).unwrap().unwrap();
        assert_eq!(got.groups, ["Família"]);
        assert_eq!(got.emails[0].addr, "joao@example.pt");
        let rows = db.contact_rows(&[a_id]).unwrap();
        assert_eq!(rows[0].email, "joao@example.pt");

        // Editing replaces the children; dropping the group removes it.
        let mut edit = got;
        edit.groups.clear();
        edit.phones.push(Phone {
            number: "+351 1".into(),
            label: String::new(),
        });
        db.save_contact(&mut edit).unwrap();
        assert!(db.contact_groups(None).unwrap().is_empty());
        assert_eq!(db.contact(a_id).unwrap().unwrap().phones.len(), 1);

        db.delete_contact(a_id).unwrap();
        assert!(db.contact(a_id).unwrap().is_none());
    }

    #[test]
    fn suggestions_rank_contacts_then_groups_then_seen() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        // Without contacts the result equals the address index's.
        db.conn
            .execute_batch(&format!(
                "INSERT INTO addresses (account_id, addr, name, folded, last_seen, weight) VALUES
                 ({acc}, 'ana@example.pt', 'Ana (index)', 'ana (index)', 1, 5),
                 ({acc}, 'anabela@x.pt', 'Anabela', 'anabela', 1, 1);"
            ))
            .unwrap();
        let plain = db.suggest_addresses(acc, "ana", 10).unwrap();
        let mixed = db.suggest_recipients(acc, "ana", 10).unwrap();
        assert_eq!(
            plain.iter().map(|r| (&r.name, &r.addr)).collect::<Vec<_>>(),
            mixed.iter().map(|r| (&r.name, &r.addr)).collect::<Vec<_>>()
        );
        assert!(mixed.iter().all(|s| s.kind == SuggestKind::Seen));

        let local = db.local_book().unwrap();
        let mut c = person(local, "Ana Contact", "ana@example.pt");
        c.groups = vec!["Analysts".into()];
        db.save_contact(&mut c).unwrap();
        let s = db.suggest_recipients(acc, "ana", 10).unwrap();
        assert_eq!(s[0].name, "Ana Contact");
        assert_eq!(s[0].kind, SuggestKind::Person);
        assert!(matches!(&s[1].kind, SuggestKind::Group(m) if m.len() == 1));
        assert_eq!(s[2].addr, "anabela@x.pt");
        // The contact's address shows once, with the contact's name.
        assert_eq!(s.iter().filter(|x| x.addr == "ana@example.pt").count(), 1);
    }

    #[test]
    fn synced_books_store_cards_and_cascade() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        db.set_dav_account(&DavAccount {
            account_id: acc,
            url: "https://dav.example.com/".into(),
            user: "ana".into(),
            mode: BOOK_READ_ONLY,
            interval_min: 30,
            ..DavAccount::default()
        })
        .unwrap();
        db.set_found_books(
            acc,
            BOOK_READ_ONLY,
            &[("/ab/".into(), "Work".into(), false)],
        )
        .unwrap();
        let book = db
            .address_books()
            .unwrap()
            .into_iter()
            .find(|b| b.account_id == Some(acc))
            .unwrap();
        let raw = b"BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Rui\r\nEMAIL:rui@example.com\r\nEND:VCARD\r\n".to_vec();
        let card = vcard::parse(&raw).unwrap();
        db.store_cards(
            book.id,
            &[("/ab/u1.vcf".into(), "\"1\"".into(), raw, card)],
            &[],
        )
        .unwrap();
        assert_eq!(db.synced_contact_count(acc).unwrap(), 1);
        assert_eq!(db.book_etags(book.id).unwrap()["/ab/u1.vcf"], "\"1\"");
        let id = db.contact_ids(Some(book.id), None, "").unwrap()[0];
        // A read-only book refuses edits.
        let mut c = db.contact(id).unwrap().unwrap();
        assert!(db.save_contact(&mut c).is_err());
        assert!(db.delete_contact(id).is_err());
        db.store_cards(book.id, &[], &["/ab/u1.vcf".into()])
            .unwrap();
        assert_eq!(db.synced_contact_count(acc).unwrap(), 0);

        db.remove_account(acc).unwrap();
        assert!(
            db.address_books()
                .unwrap()
                .iter()
                .all(|b| b.account_id.is_none())
        );
        assert!(db.dav_account(acc).unwrap().is_none());
    }
}
