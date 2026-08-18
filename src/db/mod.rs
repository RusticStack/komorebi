//! Local SQLite mail cache.
//!
//! Komorebi is local-first: every account, folder, message, draft and AI
//! artifact is cached in a SQLite database under the user's config directory.
//! All cache mutations happen here; the UI and mail/AI workers only read or
//! hand data to this module.
//!
//! Secrets (passwords, OAuth tokens, AI API keys) are **never** stored here  - 
//! they live in the OS credential store (`crate::secrets`).

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// (version, sql) pairs applied in order via `PRAGMA user_version`.
const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
        CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            email TEXT NOT NULL UNIQUE,
            display_name TEXT,
            provider_kind TEXT NOT NULL,
            auth_mode TEXT NOT NULL,
            username TEXT,
            imap_host TEXT,
            imap_port INTEGER,
            imap_security TEXT,
            smtp_host TEXT,
            smtp_port INTEGER,
            smtp_security TEXT,
            signature_html TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS folders (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            remote_id TEXT,
            name TEXT NOT NULL,
            display_name TEXT,
            special_use TEXT,
            attributes_json TEXT,
            unread_count INTEGER NOT NULL DEFAULT 0,
            total_count INTEGER NOT NULL DEFAULT 0,
            last_sync_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_folders_account ON folders(account_id);

        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
            imap_uid INTEGER,
            uid_validity INTEGER,
            rfc_message_id TEXT,
            subject TEXT,
            sender_name TEXT,
            sender_email TEXT,
            to_addresses TEXT,
            cc_addresses TEXT,
            bcc_addresses TEXT,
            date_received INTEGER,
            is_read INTEGER NOT NULL DEFAULT 0,
            is_flagged INTEGER NOT NULL DEFAULT 0,
            is_answered INTEGER NOT NULL DEFAULT 0,
            snippet TEXT,
            body_text TEXT,
            body_html TEXT,
            size_bytes INTEGER,
            has_attachments INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS idx_messages_folder_date ON messages(folder_id, date_received DESC);
        CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            subject, sender_name, sender_email, body_text,
            content='messages',
            content_rowid='rowid'
        );
        CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, body_text)
            VALUES (new.rowid, new.subject, new.sender_name, new.sender_email, new.body_text);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, body_text)
            VALUES ('delete', old.rowid, old.subject, old.sender_name, old.sender_email, old.body_text);
        END;

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS drafts (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            subject TEXT,
            to_addresses TEXT,
            body TEXT,
            updated_at INTEGER NOT NULL DEFAULT (unixepoch())
        );

        CREATE TABLE IF NOT EXISTS attachments (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            display_name TEXT NOT NULL,
            mime TEXT,
            size_bytes INTEGER NOT NULL DEFAULT 0,
            data BLOB,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(message_id);
    "#,
    ),
    (
        2,
        r#"
        ALTER TABLE messages ADD COLUMN ai_category TEXT;
        ALTER TABLE messages ADD COLUMN ai_summary TEXT;
        ALTER TABLE messages ADD COLUMN ai_triaged_at INTEGER;

        CREATE TABLE IF NOT EXISTS ai_actions (
            id TEXT PRIMARY KEY,
            account_id TEXT,
            message_id TEXT,
            kind TEXT NOT NULL,
            provider TEXT,
            model TEXT,
            output TEXT,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS idx_ai_actions_message ON ai_actions(message_id);
    "#,
    ),
];

/// Thread-safe handle to the local cache. Clone the `Arc` to share it with
/// worker threads; every method locks internally.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (and create if needed) the database at `path`, applying migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating cache dir {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    /// In-memory database for tests.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn migrate(&self) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .context("reading user_version")?;
        for (target, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > version) {
            tx.execute_batch(sql)
                .with_context(|| format!("applying migration v{target}"))?;
            tx.pragma_update(None, "user_version", *target)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider_kind: String,
    pub auth_mode: String,
    pub username: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
    pub created_at: i64,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct AccountDraft {
    pub email: String,
    pub display_name: Option<String>,
    pub provider_kind: String,
    pub auth_mode: String,
    pub username: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<String>,
}

impl Database {
    pub fn insert_account(&self, draft: &AccountDraft) -> Result<Account> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let account = Account {
            id: id.clone(),
            email: draft.email.clone(),
            display_name: draft.display_name.clone(),
            provider_kind: draft.provider_kind.clone(),
            auth_mode: draft.auth_mode.clone(),
            username: draft.username.clone(),
            imap_host: draft.imap_host.clone(),
            imap_port: draft.imap_port,
            imap_security: draft.imap_security.clone(),
            smtp_host: draft.smtp_host.clone(),
            smtp_port: draft.smtp_port,
            smtp_security: draft.smtp_security.clone(),
            created_at: now,
            updated_at: None,
        };
        let conn = self.conn();
        conn.execute(
            "INSERT INTO accounts (id, email, display_name, provider_kind, auth_mode, username,
                imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                account.id,
                account.email,
                account.display_name,
                account.provider_kind,
                account.auth_mode,
                account.username,
                account.imap_host,
                account.imap_port,
                account.imap_security,
                account.smtp_host,
                account.smtp_port,
                account.smtp_security,
                account.created_at,
            ],
        )?;
        Ok(account)
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, email, display_name, provider_kind, auth_mode, username,
                    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security,
                    created_at, updated_at
             FROM accounts ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], row_to_account)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, email, display_name, provider_kind, auth_mode, username,
                    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security,
                    created_at, updated_at
             FROM accounts WHERE id = ?1",
            [id],
            row_to_account,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Delete an account and cascade its folders/messages/drafts/attachments.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn update_account_server(
        &self,
        id: &str,
        username: Option<&str>,
        imap_host: Option<&str>,
        imap_port: Option<i64>,
        imap_security: Option<&str>,
        smtp_host: Option<&str>,
        smtp_port: Option<i64>,
        smtp_security: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE accounts SET username=?2, imap_host=?3, imap_port=?4, imap_security=?5,
                smtp_host=?6, smtp_port=?7, smtp_security=?8, updated_at=unixepoch()
             WHERE id=?1",
            params![
                id,
                username,
                imap_host,
                imap_port,
                imap_security,
                smtp_host,
                smtp_port,
                smtp_security
            ],
        )?;
        Ok(())
    }
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        email: row.get(1)?,
        display_name: row.get(2)?,
        provider_kind: row.get(3)?,
        auth_mode: row.get(4)?,
        username: row.get(5)?,
        imap_host: row.get(6)?,
        imap_port: row.get(7)?,
        imap_security: row.get(8)?,
        smtp_host: row.get(9)?,
        smtp_port: row.get(10)?,
        smtp_security: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

// ---------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub remote_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub special_use: Option<String>,
    pub unread_count: i64,
    pub total_count: i64,
    pub last_sync_at: Option<i64>,
}

impl Database {
    pub fn upsert_folder(
        &self,
        account_id: &str,
        name: &str,
        display_name: Option<&str>,
        special_use: Option<&str>,
        remote_id: Option<&str>,
    ) -> Result<Folder> {
        let conn = self.conn();
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM folders WHERE account_id=?1 AND name=?2",
                params![account_id, name],
                |r| r.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE folders SET display_name=?3, special_use=?4, remote_id=?5
                     WHERE id=?1 AND account_id=?2",
                    params![id, account_id, display_name, special_use, remote_id],
                )?;
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO folders (id, account_id, name, display_name, special_use, remote_id)
                     VALUES (?1,?2,?3,?4,?5,?6)",
                    params![id, account_id, name, display_name, special_use, remote_id],
                )?;
                id
            }
        };
        let row = conn.query_row(
            "SELECT id, account_id, remote_id, name, display_name, special_use,
                    unread_count, total_count, last_sync_at
             FROM folders WHERE id=?1",
            [&id],
            |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    remote_id: r.get(2)?,
                    name: r.get(3)?,
                    display_name: r.get(4)?,
                    special_use: r.get(5)?,
                    unread_count: r.get(6)?,
                    total_count: r.get(7)?,
                    last_sync_at: r.get(8)?,
                })
            },
        )?;
        Ok(row)
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, remote_id, name, display_name, special_use,
                    unread_count, total_count, last_sync_at
             FROM folders WHERE account_id=?1 ORDER BY
                CASE special_use WHEN 'inbox' THEN 0 WHEN 'sent' THEN 1 WHEN 'drafts' THEN 2
                                 WHEN 'trash' THEN 3 WHEN 'archive' THEN 4 WHEN 'junk' THEN 5
                                 ELSE 6 END, name",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            Ok(Folder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                remote_id: r.get(2)?,
                name: r.get(3)?,
                display_name: r.get(4)?,
                special_use: r.get(5)?,
                unread_count: r.get(6)?,
                total_count: r.get(7)?,
                last_sync_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_folder(&self, id: &str) -> Result<Option<Folder>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, account_id, remote_id, name, display_name, special_use,
                    unread_count, total_count, last_sync_at
             FROM folders WHERE id=?1",
            [id],
            |r| {
                Ok(Folder {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    remote_id: r.get(2)?,
                    name: r.get(3)?,
                    display_name: r.get(4)?,
                    special_use: r.get(5)?,
                    unread_count: r.get(6)?,
                    total_count: r.get(7)?,
                    last_sync_at: r.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_folder_counts(&self, id: &str, unread: i64, total: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE folders SET unread_count=?2, total_count=?3, last_sync_at=unixepoch()
             WHERE id=?1",
            params![id, unread, total],
        )?;
        Ok(())
    }

    /// Delete a folder, cascading to its messages and attachments.
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM folders WHERE id=?1", [id])?;
        Ok(())
    }

    /// Highest cached IMAP UID for a folder, or 0.
    pub fn folder_max_uid(&self, folder_id: &str) -> Result<u32> {
        let conn = self.conn();
        let uid: Option<i64> = conn
            .query_row(
                "SELECT MAX(imap_uid) FROM messages WHERE folder_id=?1",
                [folder_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(uid.unwrap_or(0).max(0) as u32)
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub id: String,
    pub folder_id: String,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub date_received: Option<i64>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub snippet: Option<String>,
    pub ai_category: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageDetail {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub imap_uid: Option<i64>,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub date_received: Option<i64>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
    pub ai_category: Option<String>,
    pub ai_summary: Option<String>,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MessageDraft {
    pub account_id: String,
    pub folder_id: String,
    pub imap_uid: Option<u32>,
    pub uid_validity: Option<u32>,
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub date_received: Option<i64>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub size_bytes: Option<i64>,
    pub has_attachments: bool,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn insert_message(&self, m: &MessageDraft) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO messages (id, account_id, folder_id, imap_uid, uid_validity, subject,
                sender_name, sender_email, to_addresses, cc_addresses, bcc_addresses,
                date_received, is_read, is_flagged, snippet, body_text, body_html, size_bytes,
                has_attachments)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
             ON CONFLICT(id) DO NOTHING",
            params![
                id,
                m.account_id,
                m.folder_id,
                m.imap_uid.map(|v| v as i64),
                m.uid_validity.map(|v| v as i64),
                m.subject,
                m.sender_name,
                m.sender_email,
                m.to_addresses,
                m.cc_addresses,
                m.bcc_addresses,
                m.date_received,
                m.is_read as i64,
                m.is_flagged as i64,
                m.snippet,
                m.body_text,
                m.body_html,
                m.size_bytes,
                m.has_attachments as i64,
            ],
        )?;
        Ok(id)
    }

    /// Find a message by folder + IMAP UID (dedupe across syncs).
    pub fn find_message_by_uid(&self, folder_id: &str, imap_uid: u32) -> Result<Option<String>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id FROM messages WHERE folder_id=?1 AND imap_uid=?2 LIMIT 1",
            params![folder_id, imap_uid as i64],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_messages(
        &self,
        folder_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, folder_id, subject, sender_name, sender_email, date_received,
                    is_read, is_flagged, snippet, ai_category
             FROM messages WHERE folder_id=?1
             ORDER BY date_received DESC, created_at DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![folder_id, limit, offset], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn search_messages(&self, query: &str, limit: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.folder_id, m.subject, m.sender_name, m.sender_email, m.date_received,
                    m.is_read, m.is_flagged, m.snippet, m.ai_category
             FROM messages_fts f JOIN messages m ON m.rowid = f.rowid
             WHERE messages_fts MATCH ?1
             ORDER BY m.date_received DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All starred (flagged) messages across folders, newest first.
    pub fn list_flagged_messages(&self, limit: i64, offset: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, folder_id, subject, sender_name, sender_email, date_received,
                    is_read, is_flagged, snippet, ai_category
             FROM messages WHERE is_flagged=1
             ORDER BY date_received DESC, created_at DESC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit, offset], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All messages across an account's folders, newest first (for smart views).
    pub fn list_account_messages(&self, account_id: &str, limit: i64) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, folder_id, subject, sender_name, sender_email, date_received,
                    is_read, is_flagged, snippet, ai_category
             FROM messages WHERE account_id=?1
             ORDER BY date_received DESC, created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![account_id, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Set of message ids that have at least one cached attachment.
    pub fn list_message_ids_with_attachments(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT message_id FROM attachments")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()?)
    }

    pub fn get_message_detail(&self, id: &str) -> Result<Option<MessageDetail>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, account_id, folder_id, imap_uid, subject, sender_name, sender_email,
                    to_addresses, cc_addresses, date_received, is_read, is_flagged, body_text,
                    snippet, ai_category, ai_summary, has_attachments
             FROM messages WHERE id=?1",
            [id],
            |r| {
                Ok(MessageDetail {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    folder_id: r.get(2)?,
                    imap_uid: r.get(3)?,
                    subject: r.get(4)?,
                    sender_name: r.get(5)?,
                    sender_email: r.get(6)?,
                    to_addresses: r.get(7)?,
                    cc_addresses: r.get(8)?,
                    date_received: r.get(9)?,
                    is_read: r.get(10)?,
                    is_flagged: r.get(11)?,
                    body_text: r.get(12)?,
                    snippet: r.get(13)?,
                    ai_category: r.get(14)?,
                    ai_summary: r.get(15)?,
                    has_attachments: r.get(16)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn mark_read(&self, id: &str, read: bool) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET is_read=?2 WHERE id=?1",
            params![id, read as i64],
        )?;
        Ok(())
    }

    pub fn mark_flagged(&self, id: &str, flagged: bool) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET is_flagged=?2 WHERE id=?1",
            params![id, flagged as i64],
        )?;
        Ok(())
    }

    /// Move a message into another folder (local cache move; a later sync
    /// reconciles the server-side flags/location).
    pub fn move_message(&self, id: &str, folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET folder_id=?2 WHERE id=?1",
            params![id, folder_id],
        )?;
        Ok(())
    }

    /// Mark every message in a folder as read.
    pub fn mark_folder_read(&self, folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET is_read=1 WHERE folder_id=?1",
            [folder_id],
        )?;
        Ok(())
    }

    /// Move every message from one folder into another.
    pub fn move_all_messages(&self, from_folder_id: &str, to_folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET folder_id=?2 WHERE folder_id=?1",
            params![from_folder_id, to_folder_id],
        )?;
        Ok(())
    }

    /// Permanently delete every message in a folder (used to empty trash).
    pub fn delete_all_messages(&self, folder_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM messages WHERE folder_id=?1", [folder_id])?;
        Ok(())
    }

    pub fn set_ai_fields(
        &self,
        id: &str,
        category: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE messages SET ai_category=?2, ai_summary=?3,
                ai_triaged_at = CASE WHEN ?2 IS NOT NULL THEN unixepoch() ELSE ai_triaged_at END
             WHERE id=?1",
            params![id, category, summary],
        )?;
        Ok(())
    }
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        subject: row.get(2)?,
        sender_name: row.get(3)?,
        sender_email: row.get(4)?,
        date_received: row.get(5)?,
        is_read: row.get(6)?,
        is_flagged: row.get(7)?,
        snippet: row.get(8)?,
        ai_category: row.get(9)?,
    })
}

// ---------------------------------------------------------------------------
// Drafts, attachments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub to_addresses: Option<String>,
    pub body: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub display_name: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub created_at: i64,
}

impl Database {
    pub fn save_draft(
        &self,
        account_id: &str,
        subject: Option<&str>,
        to_addresses: Option<&str>,
        body: Option<&str>,
    ) -> Result<String> {
        let conn = self.conn();
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO drafts (id, account_id, subject, to_addresses, body)
             VALUES (?1,?2,?3,?4,?5)",
            params![id, account_id, subject, to_addresses, body],
        )?;
        Ok(id)
    }

    pub fn list_drafts(&self) -> Result<Vec<Draft>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, subject, to_addresses, body, updated_at
             FROM drafts ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Draft {
                id: r.get(0)?,
                account_id: r.get(1)?,
                subject: r.get(2)?,
                to_addresses: r.get(3)?,
                body: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn delete_draft(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM drafts WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn insert_attachment(
        &self,
        message_id: &str,
        display_name: &str,
        mime: Option<&str>,
        data: &[u8],
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO attachments (id, message_id, display_name, mime, size_bytes, data)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, message_id, display_name, mime, data.len() as i64, data],
        )?;
        Ok(id)
    }

    pub fn list_attachments(&self, message_id: &str) -> Result<Vec<Attachment>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, message_id, display_name, mime, size_bytes, created_at
             FROM attachments WHERE message_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([message_id], |r| {
            Ok(Attachment {
                id: r.get(0)?,
                message_id: r.get(1)?,
                display_name: r.get(2)?,
                mime: r.get(3)?,
                size_bytes: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_attachment_data(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn();
        conn.query_row("SELECT data FROM attachments WHERE id=?1", [id], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_attachment(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM attachments WHERE id=?1", [id])?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Settings (key/value, JSON values)
// ---------------------------------------------------------------------------

impl Database {
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(Into::into)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn list_settings(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

// ---------------------------------------------------------------------------
// AI actions ledger
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiAction {
    pub id: String,
    pub account_id: Option<String>,
    pub message_id: Option<String>,
    pub kind: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub output: Option<String>,
    pub created_at: i64,
}

impl Database {
    pub fn insert_ai_action(
        &self,
        account_id: Option<&str>,
        message_id: Option<&str>,
        kind: &str,
        provider: Option<&str>,
        model: Option<&str>,
        output: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO ai_actions (id, account_id, message_id, kind, provider, model, output)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, account_id, message_id, kind, provider, model, output],
        )?;
        Ok(id)
    }

    pub fn list_ai_actions(&self, message_id: &str, limit: i64) -> Result<Vec<AiAction>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, message_id, kind, provider, model, output, created_at
             FROM ai_actions WHERE message_id=?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![message_id, limit], |r| {
            Ok(AiAction {
                id: r.get(0)?,
                account_id: r.get(1)?,
                message_id: r.get(2)?,
                kind: r.get(3)?,
                provider: r.get(4)?,
                model: r.get(5)?,
                output: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), 2);

        // Reopen applies nothing new.
        let db2 = Database::open(&path).unwrap();
        assert_eq!(db2.schema_version().unwrap(), 2);
    }

    #[test]
    fn account_crud() {
        let db = Database::in_memory().unwrap();
        let acc = db
            .insert_account(&AccountDraft {
                email: "me@example.org".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                imap_host: Some("imap.example.org".into()),
                imap_port: Some(993),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(db.list_accounts().unwrap().len(), 1);
        assert_eq!(
            db.get_account(&acc.id).unwrap().unwrap().email,
            "me@example.org"
        );
        db.delete_account(&acc.id).unwrap();
        assert!(db.list_accounts().unwrap().is_empty());
    }

    #[test]
    fn folder_and_message_flow() {
        let db = Database::in_memory().unwrap();
        let acc = db
            .insert_account(&AccountDraft {
                email: "a@b.test".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        let inbox = db
            .upsert_folder(&acc.id, "INBOX", Some("Inbox"), Some("inbox"), None)
            .unwrap();
        let id = db
            .insert_message(&MessageDraft {
                account_id: acc.id.clone(),
                folder_id: inbox.id.clone(),
                imap_uid: Some(1),
                subject: Some("Hello".into()),
                sender_email: Some("x@y.test".into()),
                snippet: Some("hi there".into()),
                date_received: Some(1_700_000_000),
                ..Default::default()
            })
            .unwrap();
        let rows = db.list_messages(&inbox.id, 50, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject.as_deref(), Some("Hello"));

        // Dedupe: same UID resolves to same message.
        let found = db.find_message_by_uid(&inbox.id, 1).unwrap().unwrap();
        assert_eq!(found, id);

        let detail = db.get_message_detail(&id).unwrap().unwrap();
        assert_eq!(detail.sender_email.as_deref(), Some("x@y.test"));

        db.mark_read(&id, true).unwrap();
        assert!(db.get_message_detail(&id).unwrap().unwrap().is_read);

        db.set_ai_fields(&id, Some("work"), Some("short summary"))
            .unwrap();
        let d = db.get_message_detail(&id).unwrap().unwrap();
        assert_eq!(d.ai_category.as_deref(), Some("work"));
    }

    #[test]
    fn fts_search() {
        let db = Database::in_memory().unwrap();
        let acc = db
            .insert_account(&AccountDraft {
                email: "a@b.test".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        let inbox = db
            .upsert_folder(&acc.id, "INBOX", None, None, None)
            .unwrap();
        db.insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: inbox.id.clone(),
            subject: Some("Quarterly report".into()),
            body_text: Some("the zebra crossed the river".into()),
            date_received: Some(1_700_000_000),
            ..Default::default()
        })
        .unwrap();
        let hits = db.search_messages("zebra", 10).unwrap();
        assert_eq!(hits.len(), 1);
        let none = db.search_messages("giraffe", 10).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn drafts_attachments_settings_ai() {
        let db = Database::in_memory().unwrap();
        let acc = db
            .insert_account(&AccountDraft {
                email: "a@b.test".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        let inbox = db
            .upsert_folder(&acc.id, "INBOX", None, None, None)
            .unwrap();
        let msg = db
            .insert_message(&MessageDraft {
                account_id: acc.id.clone(),
                folder_id: inbox.id.clone(),
                ..Default::default()
            })
            .unwrap();

        let draft_id = db
            .save_draft(&acc.id, Some("Hi"), Some("a@b.test"), Some("body"))
            .unwrap();
        assert_eq!(db.list_drafts().unwrap().len(), 1);
        db.delete_draft(&draft_id).unwrap();

        let att = db
            .insert_attachment(&msg, "file.txt", Some("text/plain"), b"hello bytes")
            .unwrap();
        assert_eq!(
            db.get_attachment_data(&att).unwrap().unwrap(),
            b"hello bytes"
        );
        assert_eq!(db.list_attachments(&msg).unwrap().len(), 1);

        db.set_setting("ui.density", "compact").unwrap();
        assert_eq!(db.get_setting("ui.density").unwrap().unwrap(), "compact");

        db.insert_ai_action(
            Some(&acc.id),
            Some(&msg),
            "summarize",
            Some("ollama"),
            Some("llama3.2"),
            "sum",
        )
        .unwrap();
        assert_eq!(db.list_ai_actions(&msg, 10).unwrap().len(), 1);
    }

    #[test]
    fn folder_bulk_actions_and_delete() {
        let db = Database::in_memory().unwrap();
        let acc = db
            .insert_account(&AccountDraft {
                email: "a@b.test".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        let inbox = db
            .upsert_folder(&acc.id, "INBOX", None, Some("inbox"), None)
            .unwrap();
        let archive = db
            .upsert_folder(&acc.id, "Archive", None, Some("archive"), None)
            .unwrap();
        let trash = db
            .upsert_folder(&acc.id, "Trash", None, Some("trash"), None)
            .unwrap();
        let custom = db
            .upsert_folder(&acc.id, "Projects", None, None, None)
            .unwrap();

        for uid in 1..=3 {
            db.insert_message(&MessageDraft {
                account_id: acc.id.clone(),
                folder_id: inbox.id.clone(),
                imap_uid: Some(uid),
                date_received: Some(1_700_000_000),
                ..Default::default()
            })
            .unwrap();
        }

        // Mark all read.
        db.mark_folder_read(&inbox.id).unwrap();
        assert!(
            db.list_messages(&inbox.id, 10, 0)
                .unwrap()
                .iter()
                .all(|m| m.is_read)
        );

        // Move all to archive.
        db.move_all_messages(&inbox.id, &archive.id).unwrap();
        assert_eq!(db.list_messages(&inbox.id, 10, 0).unwrap().len(), 0);
        assert_eq!(db.list_messages(&archive.id, 10, 0).unwrap().len(), 3);

        // Empty trash permanently deletes messages.
        db.move_all_messages(&archive.id, &trash.id).unwrap();
        db.delete_all_messages(&trash.id).unwrap();
        assert_eq!(db.list_messages(&trash.id, 10, 0).unwrap().len(), 0);

        // Deleting a folder cascades its messages.
        db.insert_message(&MessageDraft {
            account_id: acc.id.clone(),
            folder_id: custom.id.clone(),
            date_received: Some(1_700_000_000),
            ..Default::default()
        })
        .unwrap();
        db.delete_folder(&custom.id).unwrap();
        assert!(db.get_folder(&custom.id).unwrap().is_none());
    }
}
