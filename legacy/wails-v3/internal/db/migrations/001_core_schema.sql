-- Real SQLite schema for Komorebi local mail cache (Wails v3 remake)
-- Simplified but production-minded from the original detailed design.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    provider_kind TEXT NOT NULL, -- 'gmail' | 'microsoft' | 'generic' | 'local'
    username TEXT,
    imap_host TEXT,
    imap_port INTEGER,
    smtp_host TEXT,
    smtp_port INTEGER,
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
    special_use TEXT, -- inbox, sent, drafts, trash, archive, junk, etc.
    attributes_json TEXT,
    unread_count INTEGER NOT NULL DEFAULT 0,
    total_count INTEGER NOT NULL DEFAULT 0,
    last_sync_at INTEGER
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    imap_uid INTEGER,
    uid_validity INTEGER,
    rfc_message_id TEXT,
    provider_message_id TEXT,
    subject TEXT,
    sender_name TEXT,
    sender_email TEXT,
    to_addresses TEXT,   -- JSON array
    cc_addresses TEXT,
    bcc_addresses TEXT,
    date_received INTEGER,
    internal_date INTEGER,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_flagged INTEGER NOT NULL DEFAULT 0,
    is_answered INTEGER NOT NULL DEFAULT 0,
    is_draft INTEGER NOT NULL DEFAULT 0,
    snippet TEXT,
    body_text TEXT,
    body_html TEXT,
    size_bytes INTEGER,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_messages_folder_date ON messages(folder_id, date_received DESC);
CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id);

-- Lightweight FTS for search (real, not mock)
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    subject, sender_name, sender_email, body_text,
    content='messages',
    content_rowid='rowid'
);

-- Keep FTS in sync (basic triggers)
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, sender_name, sender_email, body_text)
    VALUES (new.rowid, new.subject, new.sender_name, new.sender_email, new.body_text);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, sender_name, sender_email, body_text)
    VALUES ('delete', old.rowid, old.subject, old.sender_name, old.sender_email, old.body_text);
END;

-- Settings for UI prefs, sync state, etc.
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT
);

-- Remote content consent (real model)
CREATE TABLE IF NOT EXISTS remote_content_preferences (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    scope TEXT NOT NULL, -- 'message' or 'sender'
    target_id TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(account_id, scope, target_id)
);
