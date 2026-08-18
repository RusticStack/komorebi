-- Real attachments table for the Wails remake (safe, app-generated paths)
CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    display_filename TEXT,
    mime_type TEXT,
    size_bytes INTEGER,
    storage_path TEXT,  -- real app-generated path under cache/attachments/
    sha256 TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);