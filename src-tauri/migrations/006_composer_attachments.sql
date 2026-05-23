CREATE TABLE composer_attachments (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  draft_id TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN ('attachment', 'inline')),
  content_id TEXT,
  display_filename TEXT NOT NULL CHECK (length(display_filename) > 0),
  canonical_mime TEXT NOT NULL CHECK (length(canonical_mime) > 0),
  size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
  storage_relative_path TEXT NOT NULL CHECK (length(storage_relative_path) > 0),
  sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE,
  FOREIGN KEY (draft_id) REFERENCES composer_drafts (id) ON DELETE CASCADE,
  UNIQUE (account_id, draft_id, id)
) STRICT;

CREATE INDEX idx_composer_attachments_draft
ON composer_attachments (account_id, draft_id, created_at, id);

CREATE UNIQUE INDEX ux_composer_attachments_content_id
ON composer_attachments (account_id, draft_id, content_id)
WHERE content_id IS NOT NULL;

CREATE TRIGGER composer_attachments_touch_updated_at
AFTER UPDATE ON composer_attachments
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE composer_attachments SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;
