//! Local SQLite cache.
//!
//! Each thread opens its own connection: the UI thread reads, and the sync
//! workers write. WAL mode lets the reads continue during a sync.
//! Passwords are never stored here (see `secrets`).

use crate::providers::{Security, Server};
use crate::role::Role;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 2;

/// Flag bits as packed by the sync (`mail::imap::flag_bits`).
pub const SEEN: u8 = 1;
pub const FLAGGED: u8 = 2;
pub const ANSWERED: u8 = 4;

const SCHEMA: &str = r#"
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL DEFAULT '',
    imap_host TEXT NOT NULL,
    imap_port INTEGER NOT NULL,
    imap_tls INTEGER NOT NULL,
    smtp_host TEXT NOT NULL,
    smtp_port INTEGER NOT NULL,
    smtp_tls INTEGER NOT NULL,
    save_sent INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE folders (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    display TEXT NOT NULL,
    depth INTEGER NOT NULL DEFAULT 0,
    role TEXT NOT NULL DEFAULT '',
    uidvalidity INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, name)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL,
    msgid TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    from_name TEXT NOT NULL DEFAULT '',
    from_addr TEXT NOT NULL DEFAULT '',
    to_addrs TEXT NOT NULL DEFAULT '',
    cc_addrs TEXT NOT NULL DEFAULT '',
    date INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    seen INTEGER NOT NULL DEFAULT 0,
    flagged INTEGER NOT NULL DEFAULT 0,
    answered INTEGER NOT NULL DEFAULT 0,
    has_attach INTEGER NOT NULL DEFAULT 0,
    preview TEXT NOT NULL DEFAULT '',
    has_body INTEGER NOT NULL DEFAULT 0,
    UNIQUE(folder_id, uid)
);
-- Matches the list's ORDER BY exactly, so a folder lists without a sort.
CREATE INDEX messages_folder_date ON messages(folder_id, date DESC, id DESC);
CREATE INDEX messages_folder_seen ON messages(folder_id, seen);

-- Raw messages live apart from the list rows, so scanning and searching the
-- list never pages through message bodies.
CREATE TABLE bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw BLOB NOT NULL
);
"#;

/// v1 kept raw bodies inline in `messages`.
const MIGRATE_V1_TO_V2: &str = r#"
CREATE TABLE bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw BLOB NOT NULL
);
INSERT INTO bodies (message_id, raw) SELECT id, raw FROM messages WHERE raw IS NOT NULL;
ALTER TABLE messages ADD COLUMN has_body INTEGER NOT NULL DEFAULT 0;
UPDATE messages SET has_body = 1 WHERE raw IS NOT NULL;
ALTER TABLE messages DROP COLUMN raw;
DROP INDEX messages_folder_date;
CREATE INDEX messages_folder_date ON messages(folder_id, date DESC, id DESC);
"#;

/// `%LOCALAPPDATA%\komorebi\cache.sqlite3` (local, not roaming: it holds
/// plaintext mail), or `KOMOREBI_DB` if set (a throwaway cache for testing).
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KOMOREBI_DB") {
        return PathBuf::from(p);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("komorebi")
        .join("cache.sqlite3")
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub imap: Server,
    pub smtp: Server,
    pub save_sent: bool,
}

#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub name: String,
    pub imap: Server,
    pub smtp: Server,
    pub save_sent: bool,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub display: String,
    pub depth: i64,
    pub role: Role,
}

const FOLDER_COLUMNS: &str = "id, account_id, name, display, depth, role";

impl Folder {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Folder {
            id: r.get(0)?,
            account_id: r.get(1)?,
            name: r.get(2)?,
            display: r.get(3)?,
            depth: r.get(4)?,
            role: Role::from_key(&r.get::<_, String>(5)?),
        })
    }
}

/// One row of the message list. Deliberately excludes bodies.
#[derive(Debug, Clone)]
pub struct Header {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub has_attach: bool,
    pub preview: String,
}

/// Header fields captured during sync, before a row exists.
#[derive(Debug, Clone, Default)]
pub struct NewHeader {
    pub uid: u32,
    pub msgid: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub date: i64,
    pub size: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub has_attach: bool,
}

/// Everything the reader and composer need about one message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub account_id: i64,
    pub folder_id: i64,
    pub uid: u32,
    pub msgid: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: String,
    pub cc: String,
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    pub raw: Option<Vec<u8>>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
             PRAGMA journal_size_limit = 16777216;",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            conn.execute_batch(&format!(
                "BEGIN; {SCHEMA} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
            ))
            .context("creating cache schema")?;
        } else if version == 1 {
            conn.execute_batch(&format!(
                "BEGIN; {MIGRATE_V1_TO_V2} PRAGMA user_version = 2; COMMIT;"
            ))
            .context("upgrading cache schema")?;
            // Reclaim the space the inline bodies used (needs no transaction).
            conn.execute_batch("VACUUM;").ok();
        } else if version != SCHEMA_VERSION {
            anyhow::bail!(
                "cache schema v{version} is not supported; delete the cache file to reset it"
            );
        }
        Ok(Self { conn })
    }

    // ---- accounts ------------------------------------------------------

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, email, name, imap_host, imap_port, imap_tls, smtp_host, smtp_port,
                    smtp_tls, save_sent
             FROM accounts ORDER BY position, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                email: r.get(1)?,
                name: r.get(2)?,
                imap: Server {
                    host: r.get(3)?,
                    port: r.get(4)?,
                    security: Security::from_i64(r.get(5)?),
                },
                smtp: Server {
                    host: r.get(6)?,
                    port: r.get(7)?,
                    security: Security::from_i64(r.get(8)?),
                },
                save_sent: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_account(&self, a: &NewAccount) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO accounts (email, name, imap_host, imap_port, imap_tls,
                    smtp_host, smtp_port, smtp_tls, save_sent, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                    (SELECT COALESCE(MAX(position), 0) + 1 FROM accounts))",
                params![
                    a.email,
                    a.name,
                    a.imap.host,
                    a.imap.port,
                    a.imap.security.as_i64(),
                    a.smtp.host,
                    a.smtp.port,
                    a.smtp.security.as_i64(),
                    a.save_sent,
                ],
            )
            .context("saving account (is it already added?)")?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn remove_account(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---- folders -------------------------------------------------------

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!("SELECT {FOLDER_COLUMNS} FROM folders"))?;
        let mut folders: Vec<Folder> = stmt
            .query_map([], Folder::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        folders.sort_by(|a, b| {
            (a.account_id, a.role.rank(), a.name.to_lowercase()).cmp(&(
                b.account_id,
                b.role.rank(),
                b.name.to_lowercase(),
            ))
        });
        Ok(folders)
    }

    pub fn folder(&self, id: i64) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {FOLDER_COLUMNS} FROM folders WHERE id = ?1"
            ))?
            .query_row([id], Folder::from_row)
            .optional()?)
    }

    pub fn folder_by_role(&self, account_id: i64, role: Role) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {FOLDER_COLUMNS} FROM folders WHERE account_id = ?1 AND role = ?2
                 ORDER BY id LIMIT 1"
            ))?
            .query_row(params![account_id, role.key()], Folder::from_row)
            .optional()?)
    }

    /// Insert or update a folder by its server name and return its id.
    pub fn upsert_folder(
        &self,
        account_id: i64,
        name: &str,
        display: &str,
        depth: i64,
        role: Role,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO folders (account_id, name, display, depth, role)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account_id, name) DO UPDATE SET
                display = excluded.display, depth = excluded.depth, role = excluded.role",
            params![account_id, name, display, depth, role.key()],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |r| r.get(0),
        )?)
    }

    /// Drop folders that no longer exist on the server.
    pub fn retain_folders(&self, account_id: i64, keep: &[i64]) -> Result<()> {
        let keep: HashSet<i64> = keep.iter().copied().collect();
        let existing: Vec<i64> = self
            .folders()?
            .into_iter()
            .filter(|f| f.account_id == account_id && !keep.contains(&f.id))
            .map(|f| f.id)
            .collect();
        for id in existing {
            self.conn
                .execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    /// Record a folder's UIDVALIDITY. If it changed, every cached UID is
    /// meaningless, so the folder's messages are dropped. Returns true when
    /// the cache was reset.
    pub fn set_uidvalidity(&self, folder_id: i64, uidvalidity: u32) -> Result<bool> {
        let old: u32 = self.conn.query_row(
            "SELECT uidvalidity FROM folders WHERE id = ?1",
            [folder_id],
            |r| r.get(0),
        )?;
        if old == uidvalidity {
            return Ok(false);
        }
        self.conn
            .execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
        self.conn.execute(
            "UPDATE folders SET uidvalidity = ?2 WHERE id = ?1",
            params![folder_id, uidvalidity],
        )?;
        Ok(old != 0)
    }

    pub fn unread_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT folder_id, COUNT(*) FROM messages WHERE seen = 0 GROUP BY folder_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- message list --------------------------------------------------

    /// Ids of the messages in the given folders, newest first. `search`
    /// filters on subject, sender and preview (case-insensitive substring).
    /// With the list index, one folder is read in order without sorting.
    pub fn message_ids(&self, folder_ids: &[i64], search: &str) -> Result<Vec<i64>> {
        if folder_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = folder_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let search = search.trim();
        let filter = if search.is_empty() {
            ""
        } else {
            "AND (subject LIKE ?1 ESCAPE '\\' OR from_name LIKE ?1 ESCAPE '\\'
                  OR from_addr LIKE ?1 ESCAPE '\\' OR preview LIKE ?1 ESCAPE '\\')"
        };
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT id FROM messages WHERE folder_id IN ({ids}) {filter}
             ORDER BY date DESC, id DESC"
        ))?;
        let rows = if filter.is_empty() {
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        } else {
            let pattern = format!("%{}%", escape_like(search));
            stmt.query_map([pattern], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        Ok(rows)
    }

    /// One list row (read on demand as rows scroll into view).
    pub fn header(&self, id: i64) -> Result<Option<Header>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT subject, from_name, from_addr, date, seen, flagged, answered,
                        has_attach, preview, to_addrs
                 FROM messages WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok(Header {
                    subject: r.get(0)?,
                    from_name: r.get(1)?,
                    from_addr: r.get(2)?,
                    date: r.get(3)?,
                    seen: r.get(4)?,
                    flagged: r.get(5)?,
                    answered: r.get(6)?,
                    has_attach: r.get(7)?,
                    preview: r.get(8)?,
                    to: r.get(9)?,
                })
            })
            .optional()?)
    }

    // ---- sync ----------------------------------------------------------

    pub fn known_uids(&self, folder_id: i64) -> Result<HashSet<u32>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT uid FROM messages WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_uids(&mut self, folder_id: i64, uids: &[u32]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("DELETE FROM messages WHERE folder_id = ?1 AND uid = ?2")?;
            for uid in uids {
                stmt.execute(params![folder_id, uid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn insert_headers(
        &mut self,
        account_id: i64,
        folder_id: i64,
        headers: &[NewHeader],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO messages (account_id, folder_id, uid, msgid, subject,
                    from_name, from_addr, to_addrs, cc_addrs, date, size, seen, flagged,
                    answered, has_attach)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            for h in headers {
                stmt.execute(params![
                    account_id,
                    folder_id,
                    h.uid,
                    h.msgid,
                    h.subject,
                    h.from_name,
                    h.from_addr,
                    h.to,
                    h.cc,
                    h.date,
                    h.size,
                    h.seen,
                    h.flagged,
                    h.answered,
                    h.has_attach,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Bring cached flags in line with the server's (UID → flag bits),
    /// leaving `skip` (UIDs with a local change in flight) untouched.
    /// Returns how many rows changed.
    pub fn apply_flags(
        &mut self,
        folder_id: i64,
        remote: &HashMap<u32, u8>,
        skip: &HashSet<u32>,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        {
            let mut select = tx.prepare_cached(
                "SELECT id, uid, seen, flagged, answered FROM messages WHERE folder_id = ?1",
            )?;
            let rows: Vec<(i64, u32, bool, bool, bool)> = select
                .query_map([folder_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let mut update = tx.prepare_cached(
                "UPDATE messages SET seen = ?2, flagged = ?3, answered = ?4 WHERE id = ?1",
            )?;
            for (id, uid, seen, fl, ans) in rows {
                let Some(&bits) = remote.get(&uid) else {
                    continue;
                };
                if skip.contains(&uid) {
                    continue;
                }
                let (s, f, a) = (bits & SEEN != 0, bits & FLAGGED != 0, bits & ANSWERED != 0);
                if (s, f, a) != (seen, fl, ans) {
                    update.execute(params![id, s, f, a])?;
                    changed += 1;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Messages in a folder whose body is not cached yet, newest first.
    pub fn missing_bodies(
        &self,
        folder_id: i64,
        limit: usize,
        max_size: i64,
    ) -> Result<Vec<(i64, u32)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, uid FROM messages
             WHERE folder_id = ?1 AND has_body = 0 AND size <= ?2
             ORDER BY date DESC, id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![folder_id, max_size, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- single messages ----------------------------------------------

    pub fn message(&self, id: i64) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, account_id, folder_id, uid, msgid, subject, from_name, from_addr,
                        to_addrs, cc_addrs, date, seen, flagged,
                        (SELECT raw FROM bodies WHERE message_id = messages.id)
                 FROM messages WHERE id = ?1",
                [id],
                |r| {
                    Ok(Message {
                        id: r.get(0)?,
                        account_id: r.get(1)?,
                        folder_id: r.get(2)?,
                        uid: r.get(3)?,
                        msgid: r.get(4)?,
                        subject: r.get(5)?,
                        from_name: r.get(6)?,
                        from_addr: r.get(7)?,
                        to: r.get(8)?,
                        cc: r.get(9)?,
                        date: r.get(10)?,
                        seen: r.get(11)?,
                        flagged: r.get(12)?,
                        raw: r.get(13)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_body(&mut self, id: i64, raw: &[u8], preview: &str, has_attach: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO bodies (message_id, raw) VALUES (?1, ?2)",
            params![id, raw],
        )?;
        tx.execute(
            "UPDATE messages SET preview = ?2, has_attach = ?3, has_body = 1 WHERE id = ?1",
            params![id, preview, has_attach],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_seen(&self, id: i64, seen: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET seen = ?2 WHERE id = ?1",
            params![id, seen],
        )?;
        Ok(())
    }

    pub fn set_flagged(&self, id: i64, flagged: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET flagged = ?2 WHERE id = ?1",
            params![id, flagged],
        )?;
        Ok(())
    }

    pub fn set_answered(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE messages SET answered = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_message(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(db: &Db) -> i64 {
        db.add_account(&NewAccount {
            email: "me@example.com".into(),
            name: "Me".into(),
            imap: Server {
                host: "imap.example.com".into(),
                port: 993,
                security: Security::Tls,
            },
            smtp: Server {
                host: "smtp.example.com".into(),
                port: 587,
                security: Security::StartTls,
            },
            save_sent: true,
        })
        .unwrap()
    }

    fn header(uid: u32, date: i64, subject: &str) -> NewHeader {
        NewHeader {
            uid,
            subject: subject.into(),
            from_name: "Jane".into(),
            from_addr: "jane@example.com".into(),
            date,
            ..Default::default()
        }
    }

    #[test]
    fn account_roundtrip() {
        let db = Db::in_memory().unwrap();
        let id = account(&db);
        let a = db.accounts().unwrap().remove(0);
        assert_eq!(a.id, id);
        assert_eq!(a.email, "me@example.com");
        assert_eq!(a.smtp.security, Security::StartTls);
        assert!(db.add_account(&NewAccount { ..a_to_new(&a) }).is_err());
        db.remove_account(id).unwrap();
        assert!(db.accounts().unwrap().is_empty());
    }

    fn a_to_new(a: &Account) -> NewAccount {
        NewAccount {
            email: a.email.clone(),
            name: a.name.clone(),
            imap: a.imap.clone(),
            smtp: a.smtp.clone(),
            save_sent: a.save_sent,
        }
    }

    #[test]
    fn headers_sorted_and_searchable() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(
            acc,
            f,
            &[
                header(1, 100, "Old news"),
                header(2, 300, "Invoice 100%"),
                header(3, 200, "Lunch?"),
            ],
        )
        .unwrap();
        let subjects = |search: &str| -> Vec<String> {
            db.message_ids(&[f], search)
                .unwrap()
                .into_iter()
                .map(|id| db.header(id).unwrap().unwrap().subject)
                .collect()
        };
        assert_eq!(subjects(""), ["Invoice 100%", "Lunch?", "Old news"]);
        assert_eq!(subjects("lunch").len(), 1);
        assert_eq!(subjects("100%").len(), 1);
        assert_eq!(subjects("jane").len(), 3);
        // Inserting the same UID again is ignored.
        db.insert_headers(acc, f, &[header(1, 999, "dup")]).unwrap();
        assert_eq!(db.message_ids(&[f], "").unwrap().len(), 3);
    }

    #[test]
    fn flags_and_deletions_follow_server() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(
            acc,
            f,
            &[header(1, 1, "a"), header(2, 2, "b"), header(3, 3, "c")],
        )
        .unwrap();
        assert_eq!(db.unread_counts().unwrap()[&f], 3);
        // UID 2 was just marked read locally and the change is still in
        // flight, so the server's stale "unseen" must not undo it.
        let b = db.message_ids(&[f], "").unwrap()[1];
        db.set_seen(b, true).unwrap();
        // Server: 1 seen+flagged, 2 and 3 unseen.
        let remote = HashMap::from([(1, SEEN | FLAGGED), (2, 0), (3, 0)]);
        let changed = db.apply_flags(f, &remote, &HashSet::from([2])).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(db.unread_counts().unwrap()[&f], 1);
        assert!(db.message(b).unwrap().unwrap().seen);
        db.delete_uids(f, &[3]).unwrap();
        assert_eq!(db.known_uids(f).unwrap(), HashSet::from([1, 2]));
        assert!(!db.unread_counts().unwrap().contains_key(&f));
    }

    #[test]
    fn uidvalidity_change_resets_folder() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        assert!(!db.set_uidvalidity(f, 7).unwrap());
        db.insert_headers(acc, f, &[header(1, 1, "a")]).unwrap();
        assert!(!db.set_uidvalidity(f, 7).unwrap());
        assert_eq!(db.known_uids(f).unwrap().len(), 1);
        assert!(db.set_uidvalidity(f, 8).unwrap());
        assert!(db.known_uids(f).unwrap().is_empty());
    }

    #[test]
    fn folders_ordered_by_role_and_retained() {
        let db = Db::in_memory().unwrap();
        let acc = account(&db);
        let zed = db.upsert_folder(acc, "Zed", "Zed", 0, Role::Other).unwrap();
        let trash = db
            .upsert_folder(acc, "Trash", "Trash", 0, Role::Trash)
            .unwrap();
        let inbox = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        let ids: Vec<i64> = db.folders().unwrap().iter().map(|f| f.id).collect();
        assert_eq!(ids, [inbox, trash, zed]);
        db.retain_folders(acc, &[inbox, trash]).unwrap();
        assert_eq!(db.folders().unwrap().len(), 2);
        assert_eq!(
            db.folder_by_role(acc, Role::Trash).unwrap().unwrap().id,
            trash
        );
    }

    #[test]
    fn single_folder_list_needs_no_sort() {
        let db = Db::in_memory().unwrap();
        let plan: Vec<String> = db
            .conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM messages WHERE folder_id IN (1)
                 ORDER BY date DESC, id DESC",
            )
            .unwrap()
            .query_map([], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
    }

    #[test]
    fn migrates_v1_inline_bodies() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL DEFAULT '', imap_host TEXT NOT NULL, imap_port INTEGER NOT NULL,
                imap_tls INTEGER NOT NULL, smtp_host TEXT NOT NULL, smtp_port INTEGER NOT NULL,
                smtp_tls INTEGER NOT NULL, save_sent INTEGER NOT NULL DEFAULT 1,
                position INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE folders (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL
                REFERENCES accounts(id) ON DELETE CASCADE, name TEXT NOT NULL, display TEXT NOT NULL,
                depth INTEGER NOT NULL DEFAULT 0, role TEXT NOT NULL DEFAULT '',
                uidvalidity INTEGER NOT NULL DEFAULT 0, UNIQUE(account_id, name));
             CREATE TABLE messages (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL
                REFERENCES accounts(id) ON DELETE CASCADE, folder_id INTEGER NOT NULL
                REFERENCES folders(id) ON DELETE CASCADE, uid INTEGER NOT NULL,
                msgid TEXT NOT NULL DEFAULT '', subject TEXT NOT NULL DEFAULT '',
                from_name TEXT NOT NULL DEFAULT '', from_addr TEXT NOT NULL DEFAULT '',
                to_addrs TEXT NOT NULL DEFAULT '', cc_addrs TEXT NOT NULL DEFAULT '',
                date INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0,
                seen INTEGER NOT NULL DEFAULT 0, flagged INTEGER NOT NULL DEFAULT 0,
                answered INTEGER NOT NULL DEFAULT 0, has_attach INTEGER NOT NULL DEFAULT 0,
                preview TEXT NOT NULL DEFAULT '', raw BLOB, UNIQUE(folder_id, uid));
             CREATE INDEX messages_folder_date ON messages(folder_id, date DESC);
             CREATE INDEX messages_folder_seen ON messages(folder_id, seen);
             INSERT INTO accounts VALUES (1, 'a@x.org', '', 'h', 993, 1, 'h', 465, 1, 1, 0);
             INSERT INTO folders VALUES (1, 1, 'INBOX', 'Inbox', 0, 'inbox', 7);
             INSERT INTO messages (id, account_id, folder_id, uid, raw) VALUES (1, 1, 1, 10, x'6869');
             INSERT INTO messages (id, account_id, folder_id, uid) VALUES (2, 1, 1, 11);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        assert_eq!(
            db.message(1).unwrap().unwrap().raw.as_deref(),
            Some(&b"hi"[..])
        );
        assert!(db.message(2).unwrap().unwrap().raw.is_none());
        assert_eq!(db.missing_bodies(1, 10, 1_000_000).unwrap(), [(2, 11)]);
        // Deleting a message also deletes its body.
        db.delete_message(1).unwrap();
        let bodies: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM bodies", [], |r| r.get(0))
            .unwrap();
        assert_eq!(bodies, 0);
    }

    #[test]
    fn bodies_and_missing_list() {
        let mut db = Db::in_memory().unwrap();
        let acc = account(&db);
        let f = db
            .upsert_folder(acc, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.insert_headers(acc, f, &[header(1, 1, "a"), header(2, 2, "b")])
            .unwrap();
        let missing = db.missing_bodies(f, 10, 1_000_000).unwrap();
        assert_eq!(missing.iter().map(|m| m.1).collect::<Vec<_>>(), [2, 1]);
        db.set_body(missing[0].0, b"raw", "preview", true).unwrap();
        assert_eq!(db.missing_bodies(f, 10, 1_000_000).unwrap().len(), 1);
        let m = db.message(missing[0].0).unwrap().unwrap();
        assert_eq!(m.raw.as_deref(), Some(&b"raw"[..]));
        let first = db.message_ids(&[f], "").unwrap()[0];
        assert_eq!(db.header(first).unwrap().unwrap().preview, "preview");
    }
}
