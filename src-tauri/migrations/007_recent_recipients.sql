CREATE TABLE recent_recipients (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  display_name TEXT,
  email TEXT NOT NULL CHECK (length(email) > 0),
  normalized_email TEXT NOT NULL CHECK (
    length(normalized_email) > 0
    AND normalized_email = lower(normalized_email)
  ),
  source TEXT NOT NULL DEFAULT 'composer_send' CHECK (source IN ('composer_send')),
  use_count INTEGER NOT NULL DEFAULT 1 CHECK (use_count > 0),
  last_used_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE,
  UNIQUE (account_id, normalized_email)
) STRICT;

CREATE INDEX idx_recent_recipients_account_last_used
ON recent_recipients (account_id, last_used_at DESC, normalized_email);

CREATE INDEX idx_recent_recipients_account_email
ON recent_recipients (account_id, normalized_email);

CREATE TRIGGER recent_recipients_touch_updated_at
AFTER UPDATE ON recent_recipients
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE recent_recipients SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;
