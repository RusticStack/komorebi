# Backup, Export, And Import Posture

Status: unsupported for current baseline.
Last reviewed: 2026-05-25.

## Decision

Komorebi does not currently provide a mailbox backup, full data export, or data import feature.

The only export-like feature in the current app is the redacted diagnostics bundle documented in `docs/architecture/diagnostics-and-logging.md`. That bundle is not a backup. It intentionally excludes mailbox content, attachment bytes, subjects, addresses, credentials, tokens, credential references, app-data paths, raw provider responses, and raw local cache files.

Users should not rely on Komorebi as the authoritative backup of their mail. Provider mailboxes remain the source of truth for synchronized messages until a reviewed backup/export/import design is implemented.

## Current Unsupported Surfaces

The following are intentionally not implemented:

- Full local SQLite cache export.
- Mailbox export to mbox, EML, Maildir, PST, ZIP, JSON, or another portable format.
- Attachment archive export.
- Account/settings export.
- Credential, password, app-password, refresh-token, access-token, OAuth-code, or credential-reference export.
- Restore from a previous Komorebi app-data directory.
- Import of messages, attachments, accounts, settings, contacts, recent recipients, signatures, diagnostics bundles, or arbitrary SQLite files.
- Cross-device sync, cloud backup, or Komorebi-hosted recovery.

## User-Facing Posture

Documentation and UI must distinguish diagnostics export from backup/export/import.

Allowed wording:

- "diagnostics export"
- "redacted diagnostics bundle"
- "backup/export/import is not currently supported"
- "the local mail cache is currently unencrypted"
- "account secrets are not exportable"

Forbidden wording until implemented:

- "backup your mailbox"
- "export your mail"
- "import your Komorebi data"
- "restore from backup"
- "portable account export"
- "encrypted backup"
- "safe archive of local mail"

## Security Rules For Future Work

Any future backup/export/import feature needs a separate architecture review before implementation. It must define:

- Scope: mail messages, attachments, drafts, Sent APPEND payloads, signatures, preferences, search index data, recent recipients, diagnostics, or account metadata.
- Secret handling: credentials, passwords, app passwords, OAuth refresh tokens, access tokens, authorization codes, credential refs, and Credential Manager target names must not be exported.
- Encryption posture: whether exported cache content is plaintext or encrypted, with clear user-facing warnings either way.
- Format: schema/version metadata, integrity checks, compatibility rules, and rejection of unknown or newer formats.
- Ownership validation: imports must reject arbitrary paths, raw SQL, malformed ids, oversized payloads, unsafe attachment paths, executable auto-open behavior, and provider/server mutation side effects.
- Conflict behavior: duplicate accounts, duplicate IMAP UID identity, RFC Message-ID collisions, existing attachments, draft conflicts, and partial restore rollback.
- Provider semantics: imports must not imply messages were uploaded to providers unless an explicit, tested server-side upload path exists.
- Cleanup and rollback: temporary files, failed imports, interrupted exports, and partial restore state must be safe and recoverable.
- Tests: disposable databases and fixtures only; no real user mailbox data or real credentials.

## Local Cache Warning

The current local mail cache is plaintext. If a user manually copies the app-data directory or SQLite database outside the app, that copy may contain message bodies, subjects, FTS terms, draft content, pending Sent APPEND payloads, signatures, recent recipients, and attachment files in readable form. Account secrets are stored separately in Windows Credential Manager and are not recoverable from a copied cache.

Manual copying of app data is outside Komorebi's supported workflow. It may produce inconsistent state if background sync or SMTP/Sent upload is running, and future schema versions are not guaranteed to accept copied cache files as an import source.

## Diagnostics Boundary

Diagnostics exports must remain redacted support artifacts, not backups. A diagnostics bundle may include counts, status summaries, redacted logs, privacy flags, and explicit synthetic fixtures. It must not include real mailbox content, attachment bytes, local cache files, source paths, credentials, tokens, private subjects, real addresses, or raw provider responses.

## Stop-And-Ask Triggers

Stop and ask before:

- Adding any command named or described as backup, restore, import, full export, archive, migrate, transfer, or portable data.
- Returning SQLite database bytes, cache file paths, attachment directories, raw message RFC822 bytes, or raw attachment archives to React.
- Accepting user-selected import paths or archive files.
- Exporting or importing account metadata in a way that could be confused with credential portability.
- Changing diagnostics export to include real mailbox content or attachment bytes.

## Documentation Sync

Keep this document synchronized with:

- `README.md`
- `PRIVACY.md`
- `Project Komorebi - Email Client Plan.md`
- `TODO.md`
- `AGENTS.md`
- `docs/architecture/diagnostics-and-logging.md`
- `docs/architecture/local-cache-encryption.md`
- `docs/architecture/credential-model.md`
