# Local Cache Encryption

Status: accepted decision for the current Windows-first baseline.
Last reviewed: 2026-05-22.

## Decision

Komorebi's local mail cache is not encrypted at rest in the current implementation.

This means:

- Message metadata, `body_text`, and `body_html` stored in SQLite are plaintext database content.
- FTS5 indexed message terms in `messages_fts` are plaintext index content.
- Attachment files stored under the app-generated attachment directory are plaintext files.
- Attachment metadata in SQLite is plaintext metadata.
- Account credentials, app passwords, OAuth refresh tokens, and access-token persistence remain forbidden in SQLite and are handled separately through Windows Credential Manager as documented in `docs/architecture/credential-model.md`.

Komorebi may still describe itself as privacy-first only in the local-first and zero-telemetry sense: no ads, no analytics, no tracking, no cloud sync, no remote logging, and no credential secrets in the local database. Do not describe cached mail, message bodies, FTS content, or attachments as encrypted until encryption is implemented and tested.

## Rationale

The project does not yet have an approved encrypted-mail-cache design, key lifecycle, recovery model, search-index design, or attachment encryption design. Adding casual encryption without those decisions would create fragile data-loss and recovery risks.

The current explicit posture is safer for users and future agents:

- Long-lived secrets use Windows Credential Manager/DPAPI.
- Local mailbox content is stored in ordinary SQLite rows and app-generated attachment files.
- Documentation is direct about what is and is not protected.
- Encryption can be added later through a reviewed migration and threat model instead of accidental partial coverage.

## Required Wording

Allowed:

- "local-first"
- "zero telemetry"
- "account secrets are stored in Windows Credential Manager"
- "cached mail is stored locally"
- "the current local mail cache is unencrypted"

Forbidden until implemented:

- "encrypted local mail cache"
- "encrypted message bodies"
- "encrypted attachments"
- "end-to-end encrypted local storage"
- "encrypted search index"
- "all private data is encrypted at rest"

## Future Encryption Requirements

If local mail encryption is promoted later, it must cover the full cache lifecycle:

- SQLite message bodies and private metadata.
- FTS/search content or a documented decision to disable/rebuild FTS under encryption constraints.
- Attachment file bytes and preview materialization.
- Key storage or derivation, unlock UX, recovery posture, backup/export behavior, and account removal cleanup.
- Migration from plaintext cache to encrypted cache with rollback and failure handling.
- Tests proving plaintext bodies and attachment bytes are not recoverable from the database, FTS index, or attachment directory after encryption.

Stop and ask before starting that work because it changes the product's storage and recovery model.

## Documentation Sync

Keep this document synchronized with:

- `README.md`
- `Project Komorebi - Email Client Plan.md`
- `docs/architecture/schema.md`
- `docs/architecture/attachment-safety.md`
- `docs/architecture/credential-model.md`
- `AGENTS.md`
- `TODO.md`
