# Komorebi Privacy Statement

Last updated: 2026-10-07

Komorebi is a local-first email client with zero telemetry. It does not
collect analytics, run ads, upload crash reports, or send mailbox content
through any Komorebi service. There is no Komorebi service.

## What stays on your computer

The cache lives at `%LOCALAPPDATA%\komorebi\cache.sqlite3` and holds:

- Account settings: email address, display name, IMAP/SMTP host, port and
  security mode, and the "save sent mail" choice.
- Folder names and roles.
- Message headers (subject, sender, recipients, date, size, flags), a short
  preview line, and the full raw message (including attachments) for each
  message you opened or that was prefetched.

The cache is **not encrypted**. Deleting the file resets the app; your mail
stays on the server.

## Passwords

Passwords (or app passwords) are stored only in Windows Credential Manager.
They are never written to the cache, logs or the UI state. Removing an
account deletes its saved password.

## Network calls

Komorebi connects only to the servers you configure:

- IMAP (TLS or STARTTLS) to sync folders, headers, flags and message bodies,
  and to apply your actions (read, flag, move, delete, save to Sent).
- SMTP (TLS or STARTTLS) to send mail.

Plaintext connections are refused. Komorebi makes no other network
requests: no analytics, no update checks, no remote fonts, and no remote
images.

## Remote content and attachments

HTML mail is converted to plain text before it is shown, so scripts never run
and remote images or trackers never load. Link targets appear as text.
Attachments are never opened automatically; you can save them to a folder
you choose. Attachment file names are sanitized before saving.

## Not collected

Komorebi never collects or transmits to a third party: usage data,
identifiers, crash reports, mailbox contents, address books or credentials.
