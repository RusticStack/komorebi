# SQLite Schema And Migrations

Status: initial production schema and FTS5 search index implemented.
Last reviewed: 2026-05-22.

## Implementation

The SQLite schema is versioned in `src-tauri/migrations/*.sql` and applied by `src-tauri/src/schema.rs`.

The Rust backend owns migrations. React and frontend SQL must not create, migrate, or mutate schema. The migration runner:

- Enables `PRAGMA foreign_keys = ON` for the connection before migration work.
- Uses `PRAGMA application_id = 1263354434` to identify Komorebi databases.
- Uses `PRAGMA user_version` plus the `schema_migrations` table for version tracking.
- Applies migrations transactionally and rejects databases newer than the compiled schema version.
- Uses `rusqlite` with bundled SQLite so tests and Windows builds do not depend on a system SQLite install.

Initial sync cache mutation is implemented in `src-tauri/src/imap_sync.rs` through `SqliteSyncCache`. The sync state machine writes folders and messages through backend-generated local IDs while preserving remote `remote_id`, UID, and UIDVALIDITY identity.

Inbound MIME normalization is implemented in `src-tauri/src/mime_normalization.rs`. It parses raw message bytes into the message columns used by `SqliteSyncCache`, stable attachment metadata records that match the `attachments` table shape, and an idempotent SQLite persistence helper for message plus attachment metadata.

## Tables

The current schema defines:

- `schema_migrations`: applied migration history.
- `settings`: global preferences stored as validated JSON values.
- `accounts`: non-secret account metadata and credential references only.
- `folders`: provider folders, special-use classification, preserved provider attributes, sync counters, UIDVALIDITY, and sync status.
- `messages`: local message rows with separate local id, IMAP UID, UIDVALIDITY, RFC `Message-ID`, and provider metadata.
- `attachments`: attachment metadata with display filenames and app-generated storage references.
- `templates`: future composer template records.
- `template_versions`: version history for future template editing.
- `messages_fts`: FTS5 external-content search index over message subject, sender, and body columns.

All durable tables include `created_at`; mutable tables include `updated_at` with timestamp triggers.

Message bodies and FTS indexed terms are not encrypted at rest in the current schema. The accepted cache-encryption posture is documented in `docs/architecture/local-cache-encryption.md`.

Sync state-machine behavior is documented in `docs/architecture/sync-state-machine.md`.

Inbound MIME normalization behavior is documented in `docs/architecture/mime-normalization.md`.

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
- Generic IMAP/SMTP accounts must include IMAP and SMTP host, port, and security metadata.
- Folders are unique by `(account_id, remote_id)` and `(account_id, name)`.
- `folders.attributes_json` must be valid JSON and stores normalized provider LIST/XLIST/SPECIAL-USE attributes for roles the current enum does not model directly.
- Messages must store `imap_uid` and `uid_validity` together, or neither for local-only states.
- Attachments reference their owning message through `(account_id, folder_id, message_id)` and cascade with the account/message.
- Template JSON and setting JSON must pass `json_valid`.
- Global template names and per-account template names are deduplicated separately.

## Deferred Schema Work

Encrypted local mail storage is future work. This schema stores message bodies and FTS terms as plaintext SQLite content until a reviewed encryption design and migration are implemented.

## Tests

`src-tauri/src/schema.rs` tests cover:

- Initial migration application, version recording, idempotency, and application id.
- Newer-database rejection.
- Required tables, indexes, and timestamp triggers.
- Required account, folder, message, attachment, and template columns.
- Absence of secret columns from account metadata.
- Foreign-key enforcement and account cascade behavior.
- IMAP UID identity separated from RFC `Message-ID`.
- FTS5 search returns expected fixture rows.
- FTS5 insert/update/delete triggers keep indexed content consistent.
- FTS5 rebuild restores missing index entries and backfills rows when upgrading a version-1 database.
- FTS5 integrity checks pass after trigger and rebuild paths.
- Folder semantics migration adds `attributes_json` to version-2 databases and defaults existing rows to `[]`.
- JSON validation for settings and templates.
- `PRAGMA foreign_key_check` on valid seed data.

`pnpm check` runs these tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- SQLite CREATE TABLE, constraints, and STRICT tables: https://www.sqlite.org/lang_createtable.html
- SQLite foreign-key enforcement: https://www.sqlite.org/foreignkeys.html
- SQLite PRAGMA `foreign_keys`, `foreign_key_check`, `application_id`, and `user_version`: https://www.sqlite.org/pragma.html
- SQLite FTS5 external-content rebuild guidance: https://www.sqlite.org/fts5.html
