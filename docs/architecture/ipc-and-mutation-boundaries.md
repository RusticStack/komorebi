# IPC And Mutation Boundaries

Last updated: 2026-05-23

## Current State

Komorebi has reviewed frontend-callable Rust commands for the shell read model, FTS search, account management, backend-owned OAuth loopback, and the TipTap composer MVP. There is still no frontend SQL access. `load_mail_shell` is a read-only, inputless local-cache snapshot command for account/folder/message navigation. `search_mail` is a read-only, typed, validated FTS query command for subject/sender/body search over the local cache. Account-management commands are typed Rust boundaries for credential-backed generic account setup, live generic setup login validation, display/signature/preference persistence, generic reauthentication, removal, and redacted auth status; they also notify the backend sync scheduler after account lifecycle changes. OAuth loopback commands are typed Rust boundaries for Gmail/Microsoft browser authorization, callback validation, token exchange, refresh-token storage, and non-secret OAuth runtime metadata persistence. Composer commands are typed Rust boundaries for local-only draft autosave/discard, staged attachment writes, local recent-recipient lookup, and SMTP send orchestration. Live sync adapters, pending Sent APPEND upload, and background scheduler orchestration exist; reply/forward routing and attachment preview/save remain future work. Provider Drafts upload is intentionally deferred and the composer discloses that drafts stay local until sent or discarded.

The accepted boundary is:

- React may own presentation, selection, form state, and read-only query composition.
- Rust owns every operation that can mutate credentials, local cache, account metadata, attachment files, migrations, or remote mailbox state.
- Direct frontend SQL is disabled today. If it is introduced later, it is read-only only and must be justified by a documented table/query scope.

## Current Frontend-Callable Commands

`load_mail_shell` lives in `src-tauri/src/mail_shell.rs`. It:

- accepts no frontend-supplied request fields
- opens the app-data SQLite cache through Rust
- applies Rust-owned migrations before reading
- returns only redacted account metadata, folder counts, and a bounded recent-message snapshot
- excludes removed accounts
- does not expose raw SQL, filesystem paths, credential refs, passwords, tokens, or attachment storage paths

`search_mail` also lives in `src-tauri/src/mail_shell.rs`. It:

- accepts a typed query/scope request
- validates query size, raw SQL-shaped input, raw HTML, optional account id, and optional folder id
- opens the same app-data SQLite cache through Rust
- applies Rust-owned migrations before reading
- uses parameterized FTS5 search over subject, sender name/email, plain body, and HTML body
- excludes removed accounts
- returns full safe message rows plus display-safe attachment metadata so search results outside the recent snapshot can still open in the reader
- reports a missing FTS index as `unavailable` instead of exposing raw SQLite errors to React
- does not expose raw SQL, filesystem paths, credential refs, passwords, tokens, or attachment storage paths

Composer commands live in `src-tauri/src/composer.rs`:

- `autosave_composer_draft` accepts typed account/draft ids, address arrays, subject, sanitized body text/HTML, and backend attachment ids. It validates ids and payload bounds, applies migrations, verifies staged attachment ownership, and writes through `src-tauri/src/draft_persistence.rs`.
- `discard_composer_draft` validates account/draft ownership, intentionally discards the draft, clears private draft content through `draft_persistence`, deletes staged composer attachment rows, and removes staged files best-effort from backend-generated app-data paths.
- `stage_composer_attachment` accepts user-selected bytes from the WebView File API, never a client file path. It validates account/draft ownership, MIME/type/size through `attachment_safety`, generates the storage id/path in Rust under app data, stores metadata in `composer_attachments`, and returns only display-safe attachment metadata plus inline CID.
- `send_composer_email` validates and re-autosaves the active draft, resolves account/auth/credentials in Rust, streams staged attachments from app-generated paths, calls the existing `smtp_send::send_email` boundary, records confirmed send state only after SMTP acceptance, and cleans staged composer attachments after success.
- `list_recent_recipients` accepts a typed account/query/limit request, validates account scope and bounded query text, reads local-only `recent_recipients` rows, and returns only display name, email address, use count, and last-used timestamp for composer autocomplete.

Account-management commands live in `src-tauri/src/account_management.rs`:

- `create_account` accepts typed generic-provider auth metadata plus a one-time secret, rejects Gmail/Microsoft direct secret handoff and unsupported provider/auth pairs through `src-tauri/src/auth.rs`, runs live backend IMAP LOGIN and SMTP AUTH checks over TLS/STARTTLS before credential persistence, writes the credential to Windows Credential Manager, inserts SQLite metadata with only `credential_ref`, then starts the scheduler for the new account.
- `reauth_account` validates the account id and replacement secret, rejects existing Gmail/Microsoft OAuth accounts so React cannot rotate refresh tokens directly, loads the existing generic provider/auth policy, writes the replacement credential to the approved slot, updates redacted local auth status, then restarts the scheduler for that account.
- `start_oauth_account_setup` validates Gmail/Microsoft account metadata and OAuth client ID, opens the provider URL from Rust, listens on `127.0.0.1` loopback, validates state/PKCE, exchanges the authorization code in Rust, requires a refresh token, stores it through the credential backend, inserts account metadata only after the credential write succeeds, stores non-secret OAuth runtime metadata, then starts the scheduler for the new account.
- `start_oauth_account_reauth` validates account id and either a provided OAuth client ID or stored non-secret OAuth client metadata, requires the existing account to be Gmail/Microsoft OAuth2, opens the backend-owned loopback flow, maps cancellation/timeout/state mismatch/provider/invalid-grant errors to redacted reasons, rotates the refresh token without returning secrets to React, stores non-secret OAuth runtime metadata, then restarts the scheduler for that account.
- `remove_account` validates the account id, stops the scheduler for that account, deletes account secrets before deleting or soft-removing local account metadata, deletes local scheduler/OAuth runtime settings, and returns only cleanup booleans.
- `update_account_settings` validates active account ownership, display-name payload size, signature HTML payload/sanitized output size, and serde-owned local preferences, updates `accounts.display_name`, `accounts.signature_html`, and the `settings` key `account:{id}:local_preferences`, then restarts the scheduler for the account.
- `test_account_connection` validates generic configuration shape and credential policy, then runs live backend IMAP LOGIN and SMTP AUTH over TLS/STARTTLS without writing credentials or metadata.
- `get_account_auth_status` validates account id, checks credential presence through the credential backend, persists `reconnect_needed` when the approved credential is missing, and returns provider/auth/status booleans without exposing credential targets or secrets.

These commands are listed in `src-tauri/build.rs`, granted only by matching `allow-*` permissions in `src-tauri/capabilities/default.json`, and covered by Rust, frontend, and capability tests.

## Rust-Only Privileged Operations

These operations must be typed Rust commands or internal Rust worker calls:

- Server-affecting mail actions: send, move, archive, delete, mark read/unread, flag, draft upload, Sent upload, folder rename/delete, and any IMAP/SMTP command that changes provider state. Use `docs/architecture/destructive-mail-actions.md` for move/delete/archive/mark-read/expunge semantics.
- Credential access: account creation, reauthentication, token refresh, credential lookup, credential rotation, credential deletion, and OAuth revocation.
- Account metadata mutation: account display metadata, provider metadata, per-account local preferences, and per-account signatures. Use `docs/architecture/account-signatures.md` for signature sanitization and insertion rules.
- Attachment writes: downloading, saving, moving, deleting, preview preparation, inline CID materialization, staged composer uploads, and any file-path selection after a message is parsed. Use `docs/architecture/attachment-safety.md` for path, filename, type, size, and preview rules.
- Destructive deletes: account removal, cache deletion, attachment cleanup, local message purge, folder cache reset, and provider delete/expunge.
- Migrations: schema creation, schema upgrades, FTS setup/rebuild, trigger creation, integrity checks, and rollback/recovery.
- Sync cache mutation: folder discovery, UID/UIDVALIDITY state, message upsert, flag update, expunge handling, body/attachment metadata persistence, search index updates, and retry bookkeeping.

The boundary source of truth is `src-tauri/src/ipc_boundary.rs`. Initial sync cache mutation and folder worker semantics live in `src-tauri/src/imap_sync.rs` and are tested with fake IMAP fixtures. App-managed live sync orchestration lives in `src-tauri/src/sync_scheduler.rs` and is tested with fake scheduler workers plus disposable SQLite caches. Backend SMTP send semantics live in `src-tauri/src/smtp_send.rs` and are tested with fake SMTP sender/attachment/auth stores. Draft/Sent persistence lives in `src-tauri/src/draft_persistence.rs` and is tested with disposable SQLite fixtures. Account signature persistence lives in `src-tauri/src/account_signatures.rs`; frontend-callable account settings persistence lives in `src-tauri/src/account_management.rs`; both are tested with disposable SQLite fixtures. Boundary tests require each privileged category to remain Rust-owned.

## Frontend SQL Policy

The Tauri SQL plugin is not installed and `src-tauri/capabilities/default.json` grants no `sql:*` permissions.

Direct frontend SQL may be considered only for read-heavy views after schema work lands. A future read-only policy must include:

- `sql:allow-select` only, not `sql:allow-execute`.
- No SQL plugin migrations from frontend `load()` paths.
- A narrow list of readable tables or views.
- No access to account secrets, credential references, token metadata beyond redacted auth status, attachment local paths that could expose filesystem structure, or hidden sync bookkeeping needed for correctness.
- A test that rejects `INSERT`, `UPDATE`, `DELETE`, `CREATE`, `ALTER`, `DROP`, `PRAGMA` mutation, and multiple-statement SQL from any frontend-facing query helper.

Until that policy exists, React must request data through typed Rust commands or explicit test fixture state only.

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
- `src-tauri/src/imap_sync.rs` tests enforce initial Rust-owned sync cache mutation and folder IDLE/polling worker semantics before any frontend command is registered.
- `src-tauri/src/smtp_send.rs` tests enforce backend SMTP send composition, auth resolution, attachment streaming, and confirmed send receipt behavior.
- `src-tauri/src/draft_persistence.rs` tests enforce backend-owned draft autosave/discard/sent conversion and provider-aware Sent-copy state.
- `src-tauri/src/composer.rs` tests enforce composer command mapping, staged attachment storage/cleanup, invalid attachment denial, invalid address denial, duplicate recipient normalization, recent-recipient lookup/persistence, and fake SMTP send through the typed boundary.
- `src-tauri/src/account_management.rs` tests enforce provider/auth policy, live generic setup login before credential persistence, credential-before-metadata ordering, credential-write failure rollback, generic reauthentication, public Gmail/Microsoft secret-handoff denial, account settings display/signature/preference persistence, account removal, and redacted auth status persistence.
- `src-tauri/src/oauth_loopback.rs` tests enforce browser URL opening, backend code exchange, callback denial, state mismatch, timeout, invalid grant, missing refresh token, generic-provider denial, token storage, and local TCP loopback callback behavior.
- `src-tauri/src/account_signatures.rs` tests enforce backend-owned per-account signature storage, sanitization, clearing, and deterministic insertion.
- `src-tauri/src/ipc_validation.rs` tests enforce the shared IPC denial policy before any command is registered.
- `src-tauri/capabilities/capabilities.test.ts` rejects SQL plugin dependencies, `sql:*` capability permissions, frontend SQL imports, forbidden local-system plugins, and unreviewed custom commands.
- `docs/architecture/tauri-capabilities.md` defines the capability permission policy.
- `pnpm check` runs these tests through Vitest and Rust.

## References

- Tauri SQL plugin permissions: https://v2.tauri.app/plugin/sql/
- Tauri permissions: https://v2.tauri.app/security/permissions/
- Tauri command scopes: https://v2.tauri.app/security/scope/
- Tauri runtime authority: https://v2.tauri.app/security/runtime-authority/
