PRAGMA application_id = 1263354434;

CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY CHECK (version > 0),
  name TEXT NOT NULL UNIQUE CHECK (length(name) > 0),
  applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE settings (
  key TEXT PRIMARY KEY CHECK (length(key) > 0),
  value_json TEXT NOT NULL CHECK (json_valid(value_json)),
  value_type TEXT NOT NULL CHECK (
    value_type IN ('string', 'number', 'boolean', 'object', 'array', 'null')
  ),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE accounts (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  provider_kind TEXT NOT NULL CHECK (
    provider_kind IN ('gmail', 'microsoft', 'generic_imap_smtp')
  ),
  display_email TEXT NOT NULL CHECK (length(display_email) > 0),
  display_name TEXT,
  username TEXT NOT NULL CHECK (length(username) > 0),
  auth_kind TEXT NOT NULL CHECK (
    auth_kind IN ('oauth2', 'app_password', 'manual_password')
  ),
  credential_ref TEXT NOT NULL UNIQUE CHECK (length(credential_ref) > 0),
  imap_host TEXT,
  imap_port INTEGER CHECK (imap_port IS NULL OR imap_port BETWEEN 1 AND 65535),
  imap_security TEXT CHECK (
    imap_security IS NULL OR imap_security IN ('tls', 'starttls', 'none')
  ),
  smtp_host TEXT,
  smtp_port INTEGER CHECK (smtp_port IS NULL OR smtp_port BETWEEN 1 AND 65535),
  smtp_security TEXT CHECK (
    smtp_security IS NULL OR smtp_security IN ('tls', 'starttls', 'none')
  ),
  signature_html TEXT,
  auth_status TEXT NOT NULL DEFAULT 'disconnected' CHECK (
    auth_status IN ('connected', 'reconnect_needed', 'disconnected', 'removed')
  ),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_auth_at TEXT,
  removed_at TEXT,
  CHECK (
    provider_kind != 'generic_imap_smtp'
    OR (
      imap_host IS NOT NULL
      AND imap_port IS NOT NULL
      AND imap_security IS NOT NULL
      AND smtp_host IS NOT NULL
      AND smtp_port IS NOT NULL
      AND smtp_security IS NOT NULL
    )
  ),
  UNIQUE (provider_kind, username)
) STRICT;

CREATE TABLE folders (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  remote_id TEXT NOT NULL CHECK (length(remote_id) > 0),
  name TEXT NOT NULL CHECK (length(name) > 0),
  display_name TEXT NOT NULL CHECK (length(display_name) > 0),
  kind TEXT NOT NULL DEFAULT 'custom' CHECK (
    kind IN (
      'inbox',
      'sent',
      'drafts',
      'archive',
      'trash',
      'junk',
      'custom',
      'all',
      'important'
    )
  ),
  special_use TEXT CHECK (
    special_use IS NULL
    OR special_use IN ('inbox', 'sent', 'drafts', 'archive', 'trash', 'junk')
  ),
  parent_folder_id TEXT REFERENCES folders (id) ON DELETE SET NULL,
  uid_validity INTEGER CHECK (uid_validity IS NULL OR uid_validity >= 0),
  uid_next INTEGER CHECK (uid_next IS NULL OR uid_next >= 0),
  highest_modseq TEXT,
  delimiter TEXT,
  unread_count INTEGER NOT NULL DEFAULT 0 CHECK (unread_count >= 0),
  total_count INTEGER NOT NULL DEFAULT 0 CHECK (total_count >= 0),
  sync_state TEXT NOT NULL DEFAULT 'idle' CHECK (
    sync_state IN ('idle', 'syncing', 'error', 'paused')
  ),
  last_synced_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE,
  UNIQUE (account_id, id),
  UNIQUE (account_id, remote_id),
  UNIQUE (account_id, name)
) STRICT;

CREATE TABLE messages (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  folder_id TEXT NOT NULL,
  imap_uid INTEGER CHECK (imap_uid IS NULL OR imap_uid > 0),
  uid_validity INTEGER CHECK (uid_validity IS NULL OR uid_validity >= 0),
  rfc_message_id TEXT,
  provider_message_id TEXT,
  thread_id TEXT,
  in_reply_to TEXT,
  references_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(references_json)),
  subject TEXT NOT NULL DEFAULT '',
  sender_name TEXT,
  sender_email TEXT,
  to_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(to_addresses_json)),
  cc_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(cc_addresses_json)),
  bcc_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(bcc_addresses_json)),
  reply_to_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(reply_to_addresses_json)),
  date_received TEXT,
  internal_date TEXT,
  is_read INTEGER NOT NULL DEFAULT 0 CHECK (is_read IN (0, 1)),
  is_flagged INTEGER NOT NULL DEFAULT 0 CHECK (is_flagged IN (0, 1)),
  is_answered INTEGER NOT NULL DEFAULT 0 CHECK (is_answered IN (0, 1)),
  is_draft INTEGER NOT NULL DEFAULT 0 CHECK (is_draft IN (0, 1)),
  is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
  has_attachments INTEGER NOT NULL DEFAULT 0 CHECK (has_attachments IN (0, 1)),
  flags_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(flags_json)),
  body_text TEXT,
  body_html TEXT,
  snippet TEXT,
  size_bytes INTEGER CHECK (size_bytes IS NULL OR size_bytes >= 0),
  sync_state TEXT NOT NULL DEFAULT 'synced' CHECK (
    sync_state IN ('local_only', 'pending', 'synced', 'failed')
  ),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE,
  FOREIGN KEY (account_id, folder_id) REFERENCES folders (account_id, id) ON DELETE CASCADE,
  CHECK (
    (imap_uid IS NULL AND uid_validity IS NULL)
    OR (imap_uid IS NOT NULL AND uid_validity IS NOT NULL)
  ),
  UNIQUE (account_id, folder_id, id)
) STRICT;

CREATE TABLE attachments (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  folder_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  content_id TEXT,
  disposition TEXT NOT NULL DEFAULT 'attachment' CHECK (
    disposition IN ('attachment', 'inline')
  ),
  sender_filename TEXT,
  display_filename TEXT NOT NULL CHECK (length(display_filename) > 0),
  canonical_mime TEXT NOT NULL CHECK (length(canonical_mime) > 0),
  content_type TEXT,
  size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
  storage_relative_path TEXT,
  sha256 TEXT CHECK (sha256 IS NULL OR length(sha256) = 64),
  is_inline INTEGER NOT NULL DEFAULT 0 CHECK (is_inline IN (0, 1)),
  preview_kind TEXT CHECK (
    preview_kind IS NULL OR preview_kind IN ('plain_text', 'raster_image')
  ),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id, folder_id, message_id)
    REFERENCES messages (account_id, folder_id, id)
    ON DELETE CASCADE,
  UNIQUE (account_id, folder_id, message_id, id)
) STRICT;

CREATE TABLE templates (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT REFERENCES accounts (id) ON DELETE SET NULL,
  name TEXT NOT NULL CHECK (length(name) > 0),
  description TEXT,
  block_data_json TEXT NOT NULL CHECK (json_valid(block_data_json)),
  html_output TEXT NOT NULL DEFAULT '',
  text_output TEXT,
  schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version > 0),
  is_archived INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TABLE template_versions (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  template_id TEXT NOT NULL,
  version_number INTEGER NOT NULL CHECK (version_number > 0),
  block_data_json TEXT NOT NULL CHECK (json_valid(block_data_json)),
  html_output TEXT NOT NULL DEFAULT '',
  text_output TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (template_id) REFERENCES templates (id) ON DELETE CASCADE,
  UNIQUE (template_id, version_number)
) STRICT;

CREATE UNIQUE INDEX ux_messages_imap_identity
  ON messages (account_id, folder_id, uid_validity, imap_uid)
  WHERE uid_validity IS NOT NULL AND imap_uid IS NOT NULL;

CREATE UNIQUE INDEX ux_attachments_content_id
  ON attachments (account_id, folder_id, message_id, content_id)
  WHERE content_id IS NOT NULL;

CREATE UNIQUE INDEX ux_templates_global_name
  ON templates (name)
  WHERE account_id IS NULL;

CREATE UNIQUE INDEX ux_templates_account_name
  ON templates (account_id, name)
  WHERE account_id IS NOT NULL;

CREATE INDEX idx_accounts_provider_email
  ON accounts (provider_kind, display_email);

CREATE INDEX idx_folders_account_kind
  ON folders (account_id, kind);

CREATE INDEX idx_folders_account_sync_state
  ON folders (account_id, sync_state);

CREATE INDEX idx_messages_folder_received
  ON messages (account_id, folder_id, date_received DESC, id);

CREATE INDEX idx_messages_account_rfc_message_id
  ON messages (account_id, rfc_message_id)
  WHERE rfc_message_id IS NOT NULL;

CREATE INDEX idx_messages_unread
  ON messages (account_id, folder_id, is_read, date_received DESC);

CREATE INDEX idx_messages_flagged
  ON messages (account_id, is_flagged, date_received DESC);

CREATE INDEX idx_attachments_message
  ON attachments (account_id, folder_id, message_id);

CREATE INDEX idx_templates_account
  ON templates (account_id, is_archived, name);

CREATE TRIGGER settings_touch_updated_at
AFTER UPDATE ON settings
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE settings SET updated_at = CURRENT_TIMESTAMP WHERE key = NEW.key;
END;

CREATE TRIGGER accounts_touch_updated_at
AFTER UPDATE ON accounts
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE accounts SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TRIGGER folders_touch_updated_at
AFTER UPDATE ON folders
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE folders SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TRIGGER messages_touch_updated_at
AFTER UPDATE ON messages
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE messages SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TRIGGER attachments_touch_updated_at
AFTER UPDATE ON attachments
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE attachments SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TRIGGER templates_touch_updated_at
AFTER UPDATE ON templates
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE templates SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;
