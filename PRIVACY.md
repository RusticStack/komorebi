# Komorebi Privacy Statement

Last updated: 2026-05-25

Komorebi is designed as a local-first, zero-telemetry desktop email client. It does not collect analytics, run ads, upload crash reports, send remote diagnostics, sell data, or sync mailbox content through a Komorebi service.

This statement describes the current implementation. It is not a claim that cached mail is encrypted at rest.

## What Stays Local

Komorebi stores its application data on the local Windows device under the Tauri app-data directory. The local cache currently includes:

- Account metadata such as account id, provider kind, display email, display name, username, auth status, server host/port/security metadata, credential reference, and non-secret OAuth client metadata.
- Folder metadata, provider folder attributes, unread counts, total counts, UID/UIDVALIDITY sync state, and sync status.
- Cached message metadata, sender/recipient display data, subject, snippet, received date, flags, plain-text body, sanitized/normalized HTML body, parser warnings, and search index terms.
- Attachment metadata and safe inbound attachment files written under backend-generated paths.
- Active local drafts, draft recipients, subject, body text, body HTML, staged attachment metadata, and staged attachment files until the draft is discarded or sent.
- Temporary RFC822 payloads for Microsoft/generic Sent-folder APPEND retry only until upload succeeds or duplicate detection marks the Sent copy uploaded.
- Local-only recent-recipient autocomplete rows created after confirmed sends.
- Local settings such as language preference, unified-inbox inclusion preference, remote-content consent, and per-account sanitized signatures.
- Local structured diagnostics logs under app data, after redaction.

The local mail cache is unencrypted in the current implementation. Cached message bodies, subjects, FTS/search terms, drafts, pending Sent APPEND payloads, signatures, recent recipients, and attachment files are stored as plaintext local data. Account secrets are handled separately.

## Secrets

Komorebi stores account secrets only in Windows Credential Manager/DPAPI through the Rust backend:

- Generic provider passwords or app passwords.
- OAuth refresh tokens for Gmail and Microsoft.

Komorebi must not store passwords, app passwords, refresh tokens, access tokens, OAuth authorization codes, authorization headers, or credential blobs in SQLite, browser storage, Tauri Store, diagnostics logs, diagnostic exports, screenshots, or React state.

OAuth access tokens are backend-only and memory-scoped. React receives only redacted auth status, such as connected, reconnect-needed, disconnected, or removed.

## Network Calls

Komorebi makes network calls only for the user-configured mail functionality or explicit user actions:

- IMAP sync for configured accounts, including TLS/STARTTLS LOGIN or OAuth XOAUTH2 authentication, folder discovery, message fetch, flag updates, moves, expunge, IDLE, and Sent-folder APPEND where required.
- SMTP send for configured accounts after the user sends a message.
- OAuth authorization, token exchange, token refresh, and supported provider revocation for Gmail/Microsoft account flows.
- Generic IMAP/SMTP setup testing when the user asks to test or add a compatible generic account.
- External `http`, `https`, and `mailto` links only after the user intentionally opens a sanitized message link and the Rust backend validates it.
- Remote email images only after the user explicitly allows remote content for a message or sender.

Komorebi does not make background network calls for analytics, telemetry, advertising, remote logging, product tracking, or diagnostics upload.

Provider servers still receive the information needed to operate email: authentication requests, IMAP/SMTP commands, message fetch/send data, recipient data for sent messages, and remote-content requests that the user explicitly allows. Those provider interactions are governed by the user's mail provider.

## Diagnostics

Diagnostics are local-only by default.

Komorebi writes structured local logs under app data for app health, migration status, sync state, retry/backoff events, provider/server category summaries, and selected command failures. Logs are redacted before they are written. The redaction boundary is designed to remove or replace credentials, tokens, authorization material, OAuth codes, message bodies, raw MIME, private subjects, address-like fields, credential references, app-data paths, and raw provider responses.

The settings drawer can show a diagnostics health view. That view is aggregate/status-only: cache and migration state, counts, retry/backoff history, provider/server categories, attachment storage health counts, and remediation steps. It does not show mailbox content, subjects, message bodies, addresses, attachment bytes, local paths, credentials, tokens, raw log messages, or raw provider responses.

The settings drawer can also export a diagnostic bundle only after explicit user action. The export:

- Is written through a backend-owned save dialog.
- Requires a `.json` destination and refuses overwrite.
- Includes privacy flags, database counts/status counts, redacted logs, and optional synthetic fixtures only when explicitly requested.
- Excludes credentials, tokens, account email addresses, usernames, display names, message bodies, subjects, snippets, address lists, attachment bytes, attachment storage paths, app-data paths, credential references, and raw provider responses.

Komorebi does not upload diagnostics automatically.

## Backup, Export, And Import

Komorebi does not currently provide mailbox backup, full local-cache export, account/settings export, restore, import, cross-device sync, cloud backup, or Komorebi-hosted recovery.

The diagnostics export is not a backup. It is a redacted support artifact and intentionally excludes real mailbox content, attachment bytes, credentials, tokens, credential references, local cache files, app-data paths, private subjects, real addresses, and raw provider responses.

Account secrets are not exportable. Passwords, app passwords, OAuth refresh tokens, access tokens, OAuth codes, authorization headers, credential blobs, and Credential Manager target names must not be included in any export surface.

Manual copying of Komorebi app data is outside the supported workflow. The local mail cache is currently unencrypted plaintext, so a manual copy may expose cached message bodies, subjects, search terms, drafts, pending Sent APPEND payloads, signatures, recent recipients, and attachment files. It also may be inconsistent if background sync or send/upload work is running, and future schema versions are not guaranteed to import copied cache files.

Provider mailboxes remain the source of truth for synchronized messages until a reviewed backup/export/import feature exists.

## Remote Content And Links

Remote email resources are blocked by default. Loading remote images can reveal information to the sender or remote host, such as that the message was opened and the network address used to fetch the image. Komorebi allows remote images only after explicit consent for a message or sender.

Message links are rewritten into inert links and delegated to the typed Rust `open_external_link` command. The backend validates supported schemes and rejects embedded credentials, local/private network targets, custom schemes, scripts, unsafe `mailto` fields, and control-character injection before opening.

Attachments are never auto-opened. Preview, save, and download require explicit user action and backend validation of ownership, path containment, size, hash, and MIME/type policy.

## Account Removal And Cache Removal

Users remove accounts from Settings. Account removal is backend-owned and deletes account credentials from Windows Credential Manager before account metadata/cache cleanup.

Removal stops background sync for the account first. For OAuth accounts, Komorebi attempts provider refresh-token revocation where the provider implementation supports it; Gmail is supported, Microsoft revocation is currently reported as unsupported. Local credential deletion and cache cleanup continue even if provider revocation fails or is unsupported. If Credential Manager deletion fails, local account metadata/cache rows are left unchanged and the app attempts to restart sync for the account.

When removing an account, the user can choose whether to delete local cached mail:

- If local cache deletion is enabled, Komorebi deletes the account row from SQLite, cascades account-owned cache rows, deletes account-local settings, deletes sync scheduler settings, and best-effort deletes that account's inbound attachment directory under app data.
- If local cache deletion is not enabled, Komorebi marks the account removed, clears credential references and auth metadata, deletes account-local settings, and keeps remaining local cached rows according to the current schema behavior. Secrets are still deleted.

Provider-side revocation is attempted only where supported by the provider implementation. Local credential deletion and local cleanup proceed even if provider revocation is unsupported or fails.

Drafts are local-only until sent or discarded. Discarding a draft or sending successfully clears private active draft content and staged composer attachments best-effort. Provider Drafts-folder upload is intentionally not implemented yet.

## What Komorebi Never Collects

Komorebi does not collect or transmit to a Komorebi-controlled service:

- Analytics events.
- Advertising identifiers.
- Usage telemetry.
- Product tracking events.
- Crash reports.
- Remote logs.
- Diagnostics bundles.
- Mailbox contents.
- Message bodies, subjects, snippets, attachments, or address books.
- Credentials, tokens, OAuth codes, or authorization headers.

Any future feature that uploads diagnostics, telemetry, crash reports, or mailbox data would require a separate explicit design, user-facing disclosure, and opt-in consent. That feature does not exist today.

## Current Limitations

- The local mail cache is plaintext and unencrypted.
- Backup/export/import is unsupported; diagnostics export is redacted and is not a mailbox backup.
- Release packaging and provider setup documentation still need final production verification.
- Opt-in live-provider smoke tests are not part of normal automated tests.
- External attachment open/extract behavior remains future work; current attachment actions are preview, save, and download only.

## Related Documentation

- [README.md](README.md)
- [docs/architecture/local-cache-encryption.md](docs/architecture/local-cache-encryption.md)
- [docs/architecture/backup-export-import.md](docs/architecture/backup-export-import.md)
- [docs/architecture/credential-model.md](docs/architecture/credential-model.md)
- [docs/architecture/diagnostics-and-logging.md](docs/architecture/diagnostics-and-logging.md)
- [docs/architecture/token-lifecycle.md](docs/architecture/token-lifecycle.md)
- [docs/architecture/attachment-safety.md](docs/architecture/attachment-safety.md)
- [docs/architecture/message-rendering.md](docs/architecture/message-rendering.md)
