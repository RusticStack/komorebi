# Tauri Capability Model

Last updated: 2026-05-23

## Current Policy

Komorebi's main webview grants only reviewed feature-specific Tauri permissions. The enabled capability set is explicit in `src-tauri/tauri.conf.json`:

- `app.security.capabilities` must be `["default"]` until another window or capability is intentionally introduced.
- `src-tauri/capabilities/default.json` applies only to the `main` window label.
- `default.permissions` contains only reviewed listen-only sync status event access plus mail-shell, composer, account-management, and OAuth loopback commands: `core:event:allow-listen`, `core:event:allow-unlisten`, `allow-load-mail-shell`, `allow-search-mail`, `allow-autosave-composer-draft`, `allow-discard-composer-draft`, `allow-stage-composer-attachment`, `allow-send-composer-email`, `allow-list-recent-recipients`, `allow-create-account`, `allow-reauth-account`, `allow-remove-account`, `allow-update-account-settings`, `allow-test-account-connection`, `allow-get-account-auth-status`, `allow-start-oauth-account-setup`, and `allow-start-oauth-account-reauth`.
- No remote origins are allowed to use Tauri APIs.
- `app.security.csp` is a reviewed production content security policy. It keeps script execution local-only, blocks object/frame/worker/form/embed surfaces, denies remote network/image/script sources by default, permits only Tauri IPC in `connect-src`, and permits `data:`/`blob:` images plus inline CSS for the current TipTap rich editor and sanitized rich HTML behavior.
- `app.security.devCsp` is separate and limited to local Vite/HMR plus Tauri IPC. Do not copy development `unsafe-eval`, localhost, or WebSocket allowances into the production CSP.

Tauri capability files are kept as standalone files and referenced by identifier in `tauri.conf.json`. This avoids the Tauri default where every file in `src-tauri/capabilities` is enabled automatically when `security.capabilities` is omitted.

## Content Security Policy

Production CSP in `src-tauri/tauri.conf.json` is:

```text
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; media-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; child-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'; manifest-src 'self'
```

The intentional allowances are narrow:

- `style-src 'unsafe-inline'` is required for current rich editor and sanitized rich email/signature style attributes.
- `img-src data: blob:` is required for user-selected inline composer images before they are staged through the backend.
- `connect-src ipc: http://ipc.localhost` is required for Tauri IPC.

The production CSP must not include wildcard sources, `unsafe-eval`, localhost development servers, remote `http`/`https` sources, broad frame/object/media/worker allowances, or unreviewed custom schemes. Future features that need external link opening, remote content, attachment preview materialization, workers, or media must update this policy and its tests with the feature-specific threat model.

## Denied By Default

The main webview must not receive these permissions by default:

- SQL plugin permissions, especially execute/write-style access.
- Filesystem permissions.
- Opener permissions. Gmail/Microsoft OAuth currently opens the system browser through a reviewed Rust Win32 call in `src-tauri/src/oauth_loopback.rs`, not through the frontend opener plugin.
- Shell or process permissions.
- HTTP client permissions.
- Wildcard, `allow-all`, or broad plugin default permissions.
- Frontend event emit permissions. The webview may listen/unlisten for backend-owned sync status events but must not emit events into Rust.
- Custom application command access before the command is added to the app ACL manifest, covered by tests, and granted through a narrow capability permission.

Absence of a permission is the denial mechanism. Do not add broad explicit deny lists to make unsafe capability growth look reviewed.

## Custom Commands

`src-tauri/build.rs` keeps the application command ACL list as `APP_COMMANDS`. The reviewed command list is currently:

- `load_mail_shell`: read-only app-data SQLite snapshot for accounts, folders, unread counts, and a bounded recent-message list.
- `search_mail`: read-only app-data SQLite FTS query for validated subject/sender/body search results.
- `autosave_composer_draft`: typed local draft autosave through Rust-owned SQLite persistence.
- `discard_composer_draft`: typed local draft discard plus staged composer attachment cleanup.
- `stage_composer_attachment`: typed user-selected byte staging into backend-generated app-data paths; no frontend filesystem path permission.
- `send_composer_email`: typed SMTP send orchestration through the Rust send boundary, credential resolver, staged attachment streamer, confirmed-send persistence, and scheduler refresh when provider Sent APPEND is pending.
- `list_recent_recipients`: read-only account-scoped local SQLite autocomplete suggestions from confirmed sends.
- `create_account`: typed generic-provider account setup that runs live IMAP/SMTP login validation before secret storage, stores secrets in Windows Credential Manager before metadata insert, and rejects direct Gmail/Microsoft secret handoff.
- `reauth_account`: typed generic replacement secret storage for an existing account's approved credential slot and rejection path for Gmail/Microsoft OAuth accounts.
- `remove_account`: typed credential deletion and account metadata/cache cleanup.
- `update_account_settings`: typed account display-name, sanitized signature HTML, and local unified-inbox preference persistence with active-account validation.
- `test_account_connection`: typed live generic IMAP/SMTP login validation without persistence.
- `get_account_auth_status`: typed redacted account status and credential-presence lookup.
- `start_oauth_account_setup`: typed Gmail/Microsoft OAuth browser loopback setup; validates state/PKCE, exchanges the callback code in Rust, stores refresh tokens through the credential backend, and returns only redacted account setup status.
- `start_oauth_account_reauth`: typed Gmail/Microsoft OAuth browser loopback reauthentication for existing OAuth accounts; rotates refresh tokens only after backend exchange and may reuse the stored non-secret OAuth client id for account-status reconnect.

`src-tauri/src/lib.rs` must not register any command outside `APP_COMMANDS`, and every registered command must have a matching capability permission.

Each future command needs:

- A typed Rust function with validated inputs.
- A command permission generated through `tauri_build::AppManifest`.
- A narrow capability entry for the window that needs it.
- Rust tests for allow/deny behavior and frontend tests where the UI calls it.

## Adding Plugin Permissions

Plugin permissions must be introduced feature-by-feature. A future permission entry must explain why React needs direct access instead of a typed Rust command.

Rules for likely future plugins:

- SQL: frontend access is disabled today. If introduced later, it must follow `docs/architecture/ipc-and-mutation-boundaries.md`: read-only, narrowly scoped, and never `sql:allow-execute`.
- Filesystem: prefer Rust commands using app-generated paths. Frontend filesystem access requires scoped app-data paths only.
- Opener: avoid broad URL/file opening. Links and attachments need validation and explicit user intent.
- Shell/process: denied for this product unless a future release task explicitly approves a constrained sidecar.
- HTTP: mail-provider OAuth and sync networking should run in Rust. Frontend HTTP client permissions are denied unless a local-only or provider-specific need is documented.

## Verification

`src-tauri/capabilities/capabilities.test.ts` is part of `pnpm test:frontend` and `pnpm check`. It asserts that:

- `tauri.conf.json` explicitly enables only `default`.
- All capability files are parseable and reviewed by the test.
- The default capability targets only the main window and grants only the reviewed listen/unlisten event permissions plus mail-shell, composer, account-management, and OAuth loopback command permissions.
- Forbidden plugin permissions, remote origins, wildcard windows, and unreviewed custom-command registrations are absent.
- Rust does not register forbidden Tauri plugins.
- Production CSP exactly matches the reviewed source list and rejects wildcard, `unsafe-eval`, localhost, and remote HTTPS broadening.
- Development CSP exactly matches the reviewed local Vite/HMR and Tauri IPC source list.

Run `pnpm tauri build --no-bundle` after capability, Tauri config, or build script changes to make Tauri validate the configuration without requiring installer tooling.

## References

- Tauri v2 capabilities: https://v2.tauri.app/security/capabilities/
- Tauri v2 core permissions: https://v2.tauri.app/reference/acl/core-permissions/
