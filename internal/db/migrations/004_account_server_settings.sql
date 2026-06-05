-- Per-account IMAP/SMTP settings for the Wails remake.
-- Host and port columns are created in 001_core_schema.sql; this migration
-- adds the security-mode columns needed by the account setup UI.

ALTER TABLE accounts ADD COLUMN imap_security TEXT DEFAULT 'tls';  -- tls, starttls, none

ALTER TABLE accounts ADD COLUMN smtp_security TEXT DEFAULT 'starttls'; -- tls, starttls, none

-- Add index for quick lookup
CREATE INDEX IF NOT EXISTS idx_accounts_email ON accounts(email);
