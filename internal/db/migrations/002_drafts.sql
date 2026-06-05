-- Real drafts table (local only, as per original design)
CREATE TABLE IF NOT EXISTS composer_drafts (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject TEXT,
    to_addresses TEXT,  -- JSON
    cc_addresses TEXT,
    bcc_addresses TEXT,
    body_html TEXT,
    body_text TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_drafts_account ON composer_drafts(account_id, updated_at DESC);
