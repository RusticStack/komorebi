CREATE TABLE IF NOT EXISTS sync_state (
    account_id TEXT NOT NULL,
    folder_name TEXT NOT NULL,
    uid_validity INTEGER NOT NULL DEFAULT 0,
    last_uid INTEGER NOT NULL DEFAULT 0,
    highest_modseq INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (account_id, folder_name),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
);
