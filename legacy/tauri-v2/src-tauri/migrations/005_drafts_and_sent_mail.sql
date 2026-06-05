CREATE TABLE composer_drafts (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  draft_state TEXT NOT NULL DEFAULT 'active' CHECK (
    draft_state IN ('active', 'discarded', 'sent')
  ),
  subject TEXT NOT NULL DEFAULT '',
  to_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(to_addresses_json)),
  cc_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(cc_addresses_json)),
  bcc_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(bcc_addresses_json)),
  reply_to_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(reply_to_addresses_json)),
  body_text TEXT,
  body_html TEXT,
  attachment_ids_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(attachment_ids_json)),
  in_reply_to TEXT,
  references_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(references_json)),
  autosave_revision INTEGER NOT NULL DEFAULT 1 CHECK (autosave_revision > 0),
  last_autosaved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  discarded_at TEXT,
  sent_receipt_id TEXT REFERENCES smtp_send_receipts (id) ON DELETE SET NULL,
  sent_at TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE,
  CHECK (
    draft_state != 'discarded'
    OR (
      discarded_at IS NOT NULL
      AND subject = ''
      AND to_addresses_json = '[]'
      AND cc_addresses_json = '[]'
      AND bcc_addresses_json = '[]'
      AND reply_to_addresses_json = '[]'
      AND body_text IS NULL
      AND body_html IS NULL
      AND attachment_ids_json = '[]'
    )
  ),
  CHECK (
    draft_state != 'sent'
    OR (
      sent_receipt_id IS NOT NULL
      AND sent_at IS NOT NULL
      AND subject = ''
      AND to_addresses_json = '[]'
      AND cc_addresses_json = '[]'
      AND bcc_addresses_json = '[]'
      AND reply_to_addresses_json = '[]'
      AND body_text IS NULL
      AND body_html IS NULL
      AND attachment_ids_json = '[]'
    )
  )
) STRICT;

ALTER TABLE smtp_send_receipts
ADD COLUMN source_draft_id TEXT REFERENCES composer_drafts (id) ON DELETE SET NULL;

CREATE TABLE sent_mail_copies (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  send_receipt_id TEXT NOT NULL UNIQUE,
  account_id TEXT NOT NULL,
  rfc_message_id TEXT NOT NULL CHECK (length(rfc_message_id) > 0),
  provider_kind TEXT NOT NULL CHECK (
    provider_kind IN ('gmail', 'microsoft', 'generic_imap_smtp')
  ),
  sent_folder_policy TEXT NOT NULL CHECK (
    sent_folder_policy IN ('provider_auto_copies', 'imap_append', 'local_receipt_only')
  ),
  sync_state TEXT NOT NULL CHECK (
    sync_state IN ('not_required', 'pending_upload', 'uploaded', 'unsupported', 'failed')
  ),
  sent_folder_id TEXT REFERENCES folders (id) ON DELETE SET NULL,
  sent_folder_remote_id TEXT,
  last_error TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (send_receipt_id) REFERENCES smtp_send_receipts (id) ON DELETE CASCADE,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_composer_drafts_account_state_updated
ON composer_drafts (account_id, draft_state, updated_at DESC);

CREATE INDEX idx_sent_mail_copies_account_state
ON sent_mail_copies (account_id, sync_state, created_at DESC);

CREATE TRIGGER composer_drafts_touch_updated_at
AFTER UPDATE ON composer_drafts
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE composer_drafts SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;

CREATE TRIGGER sent_mail_copies_touch_updated_at
AFTER UPDATE ON sent_mail_copies
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
  UPDATE sent_mail_copies SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;
