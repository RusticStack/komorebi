CREATE UNIQUE INDEX ux_messages_account_message_id
  ON messages (account_id, id);

CREATE TABLE remote_content_preferences (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  scope TEXT NOT NULL CHECK (scope IN ('message', 'sender')),
  message_id TEXT,
  sender_email TEXT,
  normalized_sender_email TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE,
  FOREIGN KEY (account_id, message_id)
    REFERENCES messages (account_id, id)
    ON DELETE CASCADE,
  CHECK (
    (
      scope = 'message'
      AND message_id IS NOT NULL
      AND sender_email IS NULL
      AND normalized_sender_email IS NULL
    )
    OR (
      scope = 'sender'
      AND message_id IS NULL
      AND sender_email IS NOT NULL
      AND length(sender_email) > 0
      AND normalized_sender_email IS NOT NULL
      AND length(normalized_sender_email) > 0
    )
  )
) STRICT;

CREATE UNIQUE INDEX ux_remote_content_preferences_message
  ON remote_content_preferences (account_id, message_id)
  WHERE scope = 'message';

CREATE UNIQUE INDEX ux_remote_content_preferences_sender
  ON remote_content_preferences (account_id, normalized_sender_email)
  WHERE scope = 'sender';

CREATE INDEX idx_remote_content_preferences_account
  ON remote_content_preferences (account_id, scope, updated_at DESC);

CREATE TRIGGER remote_content_preferences_touch_updated_at
AFTER UPDATE ON remote_content_preferences
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE remote_content_preferences
  SET updated_at = CURRENT_TIMESTAMP
  WHERE id = NEW.id;
END;
