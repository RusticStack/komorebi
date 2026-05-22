CREATE TABLE smtp_send_receipts (
  id TEXT PRIMARY KEY CHECK (length(id) > 0),
  account_id TEXT NOT NULL,
  rfc_message_id TEXT NOT NULL UNIQUE CHECK (length(rfc_message_id) > 0),
  subject TEXT NOT NULL DEFAULT '',
  from_address TEXT NOT NULL CHECK (length(from_address) > 0),
  to_addresses_json TEXT NOT NULL CHECK (json_valid(to_addresses_json)),
  cc_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(cc_addresses_json)),
  bcc_addresses_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(bcc_addresses_json)),
  attachment_ids_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(attachment_ids_json)),
  smtp_host TEXT NOT NULL CHECK (length(smtp_host) > 0),
  smtp_port INTEGER NOT NULL CHECK (smtp_port BETWEEN 1 AND 65535),
  smtp_security TEXT NOT NULL CHECK (smtp_security IN ('tls', 'starttls', 'none')),
  auth_mechanism TEXT NOT NULL CHECK (auth_mechanism IN ('plain', 'xoauth2')),
  message_size_bytes INTEGER NOT NULL CHECK (message_size_bytes >= 0),
  provider_response TEXT,
  sent_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_smtp_send_receipts_account_sent
ON smtp_send_receipts (account_id, sent_at DESC);
