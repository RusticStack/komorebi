ALTER TABLE sent_mail_copies
ADD COLUMN append_payload BLOB;

ALTER TABLE sent_mail_copies
ADD COLUMN append_payload_sha256 TEXT CHECK (
  append_payload_sha256 IS NULL
  OR (
    length(append_payload_sha256) = 64
    AND append_payload_sha256 GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
  )
);

ALTER TABLE sent_mail_copies
ADD COLUMN append_attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (append_attempt_count >= 0);

ALTER TABLE sent_mail_copies
ADD COLUMN last_append_attempt_at TEXT;

ALTER TABLE sent_mail_copies
ADD COLUMN uploaded_at TEXT;

CREATE INDEX idx_sent_mail_copies_pending_append
ON sent_mail_copies (account_id, created_at, id)
WHERE sent_folder_policy = 'imap_append'
  AND sync_state IN ('pending_upload', 'failed');
