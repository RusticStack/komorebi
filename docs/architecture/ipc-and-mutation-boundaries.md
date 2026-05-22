# IPC And Mutation Boundaries

Last updated: 2026-05-22

## Current State

Komorebi has no frontend-callable Rust commands and no frontend SQL access yet. That is intentional until the database schema, migration path, sync engine, and input validation layer exist.

The accepted boundary is:

- React may own presentation, selection, form state, and read-only query composition.
- Rust owns every operation that can mutate credentials, local cache, attachment files, migrations, or remote mailbox state.
- Direct frontend SQL is disabled today. If it is introduced later, it is read-only only and must be justified by a documented table/query scope.

## Rust-Only Privileged Operations

These operations must be typed Rust commands or internal Rust worker calls:

- Server-affecting mail actions: send, move, archive, delete, mark read/unread, flag, draft upload, Sent upload, folder rename/delete, and any IMAP/SMTP command that changes provider state.
- Credential access: account creation, reauthentication, token refresh, credential lookup, credential rotation, credential deletion, and OAuth revocation.
- Attachment writes: downloading, saving, moving, deleting, preview preparation, inline CID materialization, and any file-path selection after a message is parsed. Use `docs/architecture/attachment-safety.md` for path, filename, type, size, and preview rules.
- Destructive deletes: account removal, cache deletion, attachment cleanup, local message purge, folder cache reset, and provider delete/expunge.
- Migrations: schema creation, schema upgrades, FTS setup/rebuild, trigger creation, integrity checks, and rollback/recovery.
- Sync cache mutation: folder discovery, UID/UIDVALIDITY state, message upsert, flag update, expunge handling, body/attachment metadata persistence, search index updates, and retry bookkeeping.

The code source of truth is `src-tauri/src/ipc_boundary.rs`. Its tests require each privileged category to remain Rust-owned.

## Frontend SQL Policy

The Tauri SQL plugin is not installed and `src-tauri/capabilities/default.json` grants no `sql:*` permissions.

Direct frontend SQL may be considered only for read-heavy views after schema work lands. A future read-only policy must include:

- `sql:allow-select` only, not `sql:allow-execute`.
- No SQL plugin migrations from frontend `load()` paths.
- A narrow list of readable tables or views.
- No access to account secrets, credential references, token metadata beyond redacted auth status, attachment local paths that could expose filesystem structure, or hidden sync bookkeeping needed for correctness.
- A test that rejects `INSERT`, `UPDATE`, `DELETE`, `CREATE`, `ALTER`, `DROP`, `PRAGMA` mutation, and multiple-statement SQL from any frontend-facing query helper.

Until that policy exists, React must request data through typed Rust commands or mocked fixture state only.

## Typed Command Requirements

Each future frontend-callable command must have:

- A specific request and response type. Avoid untyped JSON maps for durable APIs.
- Ownership checks for account, folder, message, attachment, and draft ids.
- Input validation through `docs/architecture/ipc-validation.md` before any credential lookup, filesystem access, SQL mutation, or network call.
- Redacted errors safe to show in React and detailed local diagnostics that do not include secrets or message bodies.
- A command permission generated through `src-tauri/build.rs` and a narrow capability grant.
- Unit tests for success and denial paths before UI wiring.

Do not expose commands that accept raw SQL, arbitrary filesystem paths, arbitrary URLs, provider command strings, or raw MIME/HTML intended to be persisted without validation.

## Enforcement

- `src-tauri/src/ipc_boundary.rs` tests enforce that the privileged categories stay Rust-only.
- `src-tauri/src/ipc_validation.rs` tests enforce the shared IPC denial policy before any command is registered.
- `src-tauri/capabilities/capabilities.test.ts` rejects SQL plugin dependencies, `sql:*` capability permissions, frontend SQL imports, forbidden local-system plugins, and unregistered custom commands.
- `docs/architecture/tauri-capabilities.md` defines the capability permission policy.
- `pnpm check` runs these tests through Vitest and Rust.

## References

- Tauri SQL plugin permissions: https://v2.tauri.app/plugin/sql/
- Tauri permissions: https://v2.tauri.app/security/permissions/
- Tauri command scopes: https://v2.tauri.app/security/scope/
- Tauri runtime authority: https://v2.tauri.app/security/runtime-authority/
