# SQLite Schema And Migrations

Status: initial production schema, FTS5 search index and shell search command, SMTP send receipts, draft persistence, staged composer attachments, local recent recipients, Sent-copy planning/upload state, per-account signatures, per-account local preferences, remote-content preferences, and MIME parse-warning storage implemented.
Last reviewed: 2026-05-24.

## Implementation

The SQLite schema is versioned in `src-tauri/migrations/*.sql` and applied by `src-tauri/src/schema.rs`.

The Rust backend owns migrations. React and frontend SQL must not create, migrate, or mutate schema. The migration runner:

- Enables `PRAGMA foreign_keys = ON` for the connection before migration work.
- Uses `PRAGMA application_id = 1263354434` to identify Komorebi databases.
- Uses `PRAGMA user_version` plus the `schema_migrations` table for version tracking.
- Applies migrations transactionally and rejects databases newer than the compiled schema version.
- Uses `rusqlite` with bundled SQLite so tests and Windows builds do not depend on a system SQLite install.

Initial sync cache mutation is implemented in `src-tauri/src/imap_sync.rs` through `SqliteSyncCache`. The sync state machine writes folders and messages through backend-generated local IDs while preserving remote `remote_id`, UID, and UIDVALIDITY identity.

Inbound MIME normalization is implemented in `src-tauri/src/mime_normalization.rs`. It parses raw message bytes into the message columns used by `SqliteSyncCache`, stable attachment metadata records that match the `attachments` table shape, checked parser-warning JSON, and an idempotent SQLite persistence helper for message plus attachment metadata.

Backend SMTP send receipt persistence is implemented in `src-tauri/src/smtp_send.rs`. Confirmed sends are recorded only after the SMTP transport accepts the message.

Draft and Sent-copy persistence is implemented in `src-tauri/src/draft_persistence.rs` and exposed to the composer through `src-tauri/src/composer.rs`. Active drafts autosave sanitized composer HTML, including the reviewed rich-formatting subset; discarded/sent drafts clear private draft content; confirmed sends get provider-aware Sent-copy records. Migration `009_sent_mail_append_payloads.sql` adds the temporary RFC822 payload, payload hash, attempt counter, last attempt time, upload time, and pending-append index needed to retry provider Sent uploads safely.

Staged composer attachment persistence is implemented by migration `006_composer_attachments.sql` and `src-tauri/src/composer.rs`. User-selected outbound bytes are written to backend-generated app-data paths and linked to active draft ids until discard or confirmed send cleanup.

Local recent-recipient persistence is implemented by migration `007_recent_recipients.sql` and `src-tauri/src/composer.rs`. Confirmed composer sends update account-scoped rows used only for local autocomplete.

Per-account signature persistence is implemented in `src-tauri/src/account_signatures.rs` and `src-tauri/src/account_management.rs` through the existing `accounts.signature_html` column. Signatures are sanitized account metadata and are not credentials.

Per-account local preferences are implemented in `src-tauri/src/account_management.rs` through the existing JSON `settings` table using keys shaped as `account:{account_id}:local_preferences`. The current preference is `includeInUnifiedInbox`, which `load_mail_shell` and `search_mail` use to exclude opted-out accounts from unified views while leaving explicit account scope usable. No schema migration was required because `settings` already stores validated JSON objects.

Remote-content preferences are implemented by migration `010_remote_content_preferences.sql` and `src-tauri/src/mail_shell.rs`. The table stores explicit user consent for either one stored message or one normalized sender within an account. React chooses only the scope; Rust validates the account/folder/message and derives sender addresses from the cached message row.

## Tables

The current schema defines:

- `schema_migrations`: applied migration history.
- `settings`: global and scoped preferences stored as validated JSON values, including per-account local preferences under `account:{account_id}:local_preferences`.
- `accounts`: non-secret account metadata and credential references only.
- `folders`: provider folders, special-use classification, preserved provider attributes, sync counters, UIDVALIDITY, and sync status.
- `messages`: local message rows with separate local id, IMAP UID, UIDVALIDITY, RFC `Message-ID`, provider metadata, and MIME parser-warning JSON.
- `attachments`: attachment metadata with display filenames and app-generated storage references.
- `smtp_send_receipts`: confirmed outbound SMTP send receipts recorded after provider acceptance.
- `composer_drafts`: local-only composer draft autosave state; rows are not provider Drafts-folder uploads.
- `composer_attachments`: staged outbound composer attachment metadata and backend-generated storage references.
- `recent_recipients`: account-scoped local-only autocomplete rows populated after confirmed composer sends.
- `sent_mail_copies`: provider-aware Sent-folder copy/upload state for confirmed sends, including temporary pending APPEND payloads for providers that require IMAP Sent upload.
- `remote_content_preferences`: local-only message/sender remote-image consent rows.
- `templates`: future composer template records.
- `template_versions`: version history for future template editing.
- `messages_fts`: FTS5 external-content search index over message subject, sender, and body columns.

All durable tables include `created_at`; mutable tables include `updated_at` with timestamp triggers.

Message bodies, FTS indexed terms, attachment metadata, and app-generated attachment files are not encrypted at rest in the current schema/storage model. The accepted cache-encryption posture is documented in `docs/architecture/local-cache-encryption.md`.

Sync state-machine behavior is documented in `docs/architecture/sync-state-machine.md`.

Inbound MIME normalization behavior is documented in `docs/architecture/mime-normalization.md`.

SMTP send behavior is documented in `docs/architecture/smtp-send.md`.

Draft and Sent persistence behavior is documented in `docs/architecture/draft-and-sent-persistence.md`.

Account signature behavior is documented in `docs/architecture/account-signatures.md`.

## FTS5 Search

`src-tauri/migrations/002_messages_fts.sql` creates `messages_fts` as an FTS5 external-content table linked to `messages.rowid`. It indexes:

- `subject`
- `sender_name`
- `sender_email`
- `body_text`
- `body_html`

The migration adds insert, update, and delete triggers for indexed columns. It also issues `INSERT INTO messages_fts(messages_fts) VALUES ('rebuild')` so upgrading a version-1 database with existing messages backfills the index.

`src-tauri/src/schema.rs` exposes:

- `search_messages_fts`: parameterized FTS search joined back to `messages`.
- `rebuild_messages_fts`: explicit FTS rebuild command for repair or future indexed-column migrations.
- `check_messages_fts_integrity`: FTS5 `integrity-check` command with external-content table validation.

`src-tauri/src/mail_shell.rs` exposes the frontend-facing `search_mail` command. It converts user text into bounded FTS terms, parses reviewed advanced filters into parameterized sender/subject/body/read/flag/attachment predicates, validates optional account/folder scope, excludes removed accounts, returns full safe shell message rows with display-only attachment metadata plus parsed-filter/highlight metadata, and reports a missing FTS table as `indexStatus: "unavailable"`.

## Identity Rules

`messages.id` is the local stable row id. It is not an IMAP UID and not an RFC `Message-ID`.

Remote identity is stored separately:

- `account_id`
- `folder_id`
- `uid_validity`
- `imap_uid`
- `rfc_message_id`
- `provider_message_id`

The unique partial index `ux_messages_imap_identity` prevents duplicate `(account_id, folder_id, uid_validity, imap_uid)` rows when IMAP identity exists. The RFC `Message-ID` is indexed but not unique because providers can expose the same RFC message across folders, accounts, or duplicate deliveries.

## Constraints

The schema uses primary keys, foreign keys, unique constraints, partial unique indexes, JSON checks, enum-style checks, and non-negative counter checks. Notable invariants:

- SQLite account rows never include password, app-password, access-token, refresh-token, client-secret, authorization-code, or authorization-header columns.
- `accounts.credential_ref` is the only persisted secret reference and is unique.
- `accounts.signature_html` stores optional sanitized HTML account metadata. Empty signatures store `NULL`.
- Per-account local preference rows in `settings` store validated JSON objects and are deleted during account removal.
- Remote-content preference rows reference an account and either one message or one normalized sender. Message-scoped rows cascade when the message/account is deleted; sender-scoped rows cascade when the account is deleted. Uniqueness allows one row per `(account_id, message_id)` message scope and one row per `(account_id, normalized_sender_email)` sender scope.
- Generic IMAP/SMTP accounts must include IMAP and SMTP host, port, and security metadata.
- Folders are unique by `(account_id, remote_id)` and `(account_id, name)`.
- `folders.attributes_json` must be valid JSON and stores normalized provider LIST/XLIST/SPECIAL-USE attributes for roles the current enum does not model directly.
- Messages must store `imap_uid` and `uid_validity` together, or neither for local-only states.
- `messages.mime_parse_warnings_json` must be valid JSON and stores parser warnings from accepted raw MIME fetches without logging message content.
- Attachments reference their owning message through `(account_id, folder_id, message_id)` and cascade with the account/message. `attachments.storage_relative_path`, when present, is backend-generated under `attachments/{account_id}/{folder_id}/{message_id}/` and never derived from sender filenames.
- Template JSON and setting JSON must pass `json_valid`.
- Global template names and per-account template names are deduplicated separately.
- SMTP send receipts reference accounts, store JSON address snapshots, restrict SMTP security/auth enum values, and are inserted only by the Rust send boundary after confirmed transport acceptance.
- Composer drafts reference accounts, require JSON address/reference/attachment arrays, and clear private content when discarded or marked sent.
- Composer attachments reference accounts and active draft rows, store only backend-generated storage-relative paths, require SHA-256 and MIME metadata, and are cleaned after discard or confirmed send.
- Recent recipients reference accounts, deduplicate by `(account_id, normalized_email)`, store optional display name plus use count and last-used timestamp, and are local cache data only.
- Sent mail copy records reference confirmed send receipts and account/provider metadata, and distinguish provider auto-copy, pending IMAP append, uploaded, failed, and local-receipt-only states. Pending/failed IMAP append rows may temporarily store plaintext RFC822 payload bytes plus SHA-256 for retry; uploaded rows clear those payload columns.

## Deferred Schema Work

Encrypted local mail storage is future work. This schema/storage model stores message bodies and FTS terms as plaintext SQLite content and attachment files as plaintext app-data files until a reviewed encryption design and migration are implemented.

## Tests

`src-tauri/src/schema.rs` tests cover:

- Initial migration application, version recording, idempotency, and application id.
- Newer-database rejection.
- Required tables, indexes, and timestamp triggers.
- Required account, folder, message, attachment, and template columns.
- Absence of secret columns from account metadata.
- Presence of `accounts.signature_html` as non-secret account metadata.
- Foreign-key enforcement and account cascade behavior.
- IMAP UID identity separated from RFC `Message-ID`.
- FTS5 search returns expected fixture rows.
- FTS5 insert/update/delete triggers keep indexed content consistent.
- FTS5 rebuild restores missing index entries and backfills rows when upgrading a version-1 database.
- FTS5 integrity checks pass after trigger and rebuild paths.
- Frontend-facing shell search covers active-account FTS results, account/folder scoping, attachment metadata projection, missing-index reporting, and denial paths in `src-tauri/src/mail_shell.rs`.
- Folder semantics migration adds `attributes_json` to version-2 databases and defaults existing rows to `[]`.
- SMTP send receipt migration adds `smtp_send_receipts` to version-3 databases and accepts valid confirmed send records.
- Draft/Sent migration adds `composer_drafts`, `sent_mail_copies`, and `smtp_send_receipts.source_draft_id` to version-4 databases.
- Composer attachment migration adds `composer_attachments`, staged attachment indexes, and timestamp trigger to version-5 databases.
- Recent-recipient migration adds `recent_recipients`, autocomplete indexes, uniqueness, and timestamp trigger to version-6 databases.
- MIME parse-warning migration adds `messages.mime_parse_warnings_json` with a checked `[]` default to version-7 databases.
- Sent APPEND payload migration adds retry payload/hash/attempt/upload columns and a pending-append index to version-8 databases.
- Remote-content preference migration adds message/sender consent rows, uniqueness, account/message foreign keys, timestamp trigger, and account cascade behavior to version-9 databases.
- JSON validation for settings and templates.
- `PRAGMA foreign_key_check` on valid seed data.

`pnpm check` runs these tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- SQLite CREATE TABLE, constraints, and STRICT tables: https://www.sqlite.org/lang_createtable.html
- SQLite foreign-key enforcement: https://www.sqlite.org/foreignkeys.html
- SQLite PRAGMA `foreign_keys`, `foreign_key_check`, `application_id`, and `user_version`: https://www.sqlite.org/pragma.html
- SQLite FTS5 external-content rebuild guidance: https://www.sqlite.org/fts5.html
