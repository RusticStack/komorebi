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
    (
        3,
        r#"
        CREATE TABLE IF NOT EXISTS reminders (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL,
            message_id TEXT NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            label TEXT,
            due_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (unixepoch())
        );
        CREATE INDEX IF NOT EXISTS idx_reminders_due ON reminders(due_at);
        CREATE INDEX IF NOT EXISTS idx_reminders_account ON reminders(account_id);
    "#,
    ),
    (
        4,
        r#"
        ALTER TABLE messages ADD COLUMN in_reply_to TEXT;
        ALTER TABLE messages ADD COLUMN references_header TEXT;
        ALTER TABLE messages ADD COLUMN thread_id TEXT;
        CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id);
        CREATE INDEX IF NOT EXISTS idx_messages_rfc_id ON messages(rfc_message_id);
    "#,
    ),
    (
        5,
        r#"
        CREATE TABLE IF NOT EXISTS labels (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            color TEXT,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            UNIQUE(account_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_labels_account ON labels(account_id);

        CREATE TABLE IF NOT EXISTS message_labels (
            message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
            PRIMARY KEY (message_id, label_id)
        );
        CREATE INDEX IF NOT EXISTS idx_message_labels_label ON message_labels(label_id);
    "#,
    ),
    (
        6,
        r#"
        CREATE TABLE IF NOT EXISTS outbox (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            uid INTEGER NOT NULL,
            kind TEXT NOT NULL,
            target_folder TEXT,
            created_at INTEGER NOT NULL DEFAULT (unixepoch()),
            applied_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_outbox_account ON outbox(account_id);
        CREATE INDEX IF NOT EXISTS idx_outbox_pending ON outbox(applied_at);
    "#,
    ),
    (
        7,
        r#"
        ALTER TABLE outbox ADD COLUMN source_folder TEXT;
        CREATE TABLE IF NOT EXISTS message_notes (
            message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
            body TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
    "#,
    ),
    (
        8,
        r#"
        ALTER TABLE drafts ADD COLUMN cc_addresses TEXT;
        ALTER TABLE drafts ADD COLUMN bcc_addresses TEXT;
        ALTER TABLE drafts ADD COLUMN send_at INTEGER;
        CREATE INDEX IF NOT EXISTS idx_drafts_send_at ON drafts(send_at);
    "#,
    ),
    (
        9,
        r#"
        CREATE TABLE IF NOT EXISTS rules (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            field TEXT NOT NULL,
            value TEXT NOT NULL,
            action TEXT NOT NULL,
            action_arg TEXT,
            position INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS contacts (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            email TEXT NOT NULL,
            name TEXT,
            count INTEGER NOT NULL DEFAULT 1,
            last_seen INTEGER NOT NULL,
            UNIQUE(account_id, email)
        );
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
        let mut conn = self.conn.lock().expect("cache mutex poisoned");
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
        let conn = self.conn.lock().expect("cache mutex poisoned");
        Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("cache mutex poisoned")
    }
}

// Domain modules: each holds part of the impl Database surface plus its
// row types. Re-exported here so crate::db::{...} paths keep working.
mod accounts;
mod ai_ledger;
mod contacts;
mod drafts;
mod folders;
mod labels;
mod messages;
mod outbox;
mod reminders;
mod rules;
mod settings;
#[cfg(test)]
mod tests;

pub use accounts::{Account, AccountDraft};
pub use ai_ledger::AiAction;
pub use drafts::{Attachment, Draft};
pub use folders::Folder;
pub use labels::Label;
pub use messages::{MessageDetail, MessageDraft, MessageRow};
pub use outbox::OutboxEntry;
pub use reminders::Reminder;
pub use rules::Rule;
