# Tauri Capability Model

Last updated: 2026-05-24

## Current Policy

Komorebi's main webview grants only reviewed feature-specific Tauri permissions. The enabled capability set is explicit in `src-tauri/tauri.conf.json`:

- `app.security.capabilities` must be `["default"]` until another window or capability is intentionally introduced.
- `src-tauri/capabilities/default.json` applies only to the `main` window label.
- `default.permissions` contains only reviewed listen-only sync status event access plus mail-shell, remote-content preference, typed mail-action, external-link opening, reader-attachment, composer, account-management, and OAuth loopback commands: `core:event:allow-listen`, `core:event:allow-unlisten`, `allow-load-mail-shell`, `allow-search-mail`, `allow-allow-remote-content`, `allow-perform-mail-action`, `allow-open-external-link`, `allow-preview-attachment`, `allow-save-attachment`, `allow-download-attachment`, `allow-autosave-composer-draft`, `allow-discard-composer-draft`, `allow-stage-composer-attachment`, `allow-stage-forwarded-attachments`, `allow-send-composer-email`, `allow-list-recent-recipients`, `allow-list-composer-drafts`, `allow-create-account`, `allow-reauth-account`, `allow-remove-account`, `allow-update-account-settings`, `allow-test-account-connection`, `allow-get-account-auth-status`, `allow-start-oauth-account-setup`, and `allow-start-oauth-account-reauth`.
- No remote origins are allowed to use Tauri APIs.
- `app.security.csp` is a reviewed production content security policy. It keeps script execution local-only, blocks object/frame/worker/form/embed surfaces, denies remote script/network sources, permits only Tauri IPC in `connect-src`, and permits `data:`/`blob:` images plus `https:` image loads. The `https:` image source exists only for the explicit-consent remote-content workflow; the sanitizer still removes remote image URLs unless a persisted message/sender preference allows them.
- `app.security.devCsp` is separate and limited to local Vite/HMR plus Tauri IPC. Do not copy development `unsafe-eval`, localhost, or WebSocket allowances into the production CSP.

Tauri capability files are kept as standalone files and referenced by identifier in `tauri.conf.json`. This avoids the Tauri default where every file in `src-tauri/capabilities` is enabled automatically when `security.capabilities` is omitted.

## Content Security Policy

Production CSP in `src-tauri/tauri.conf.json` is:

```text
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; media-src 'none'; object-src 'none'; frame-src 'none'; worker-src 'none'; child-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'; manifest-src 'self'
```

The intentional allowances are narrow:

- `style-src 'unsafe-inline'` is required for current rich editor and sanitized rich email/signature style attributes.
- `img-src data: blob:` is required for user-selected inline composer images before they are staged through the backend.
- `img-src https:` is required only for user-consented remote email image loads after `SafeEmailBody` validates and preserves a stored message/sender preference. It does not grant frontend HTTP client permissions or bypass sanitizer checks.
- `connect-src ipc: http://ipc.localhost` is required for Tauri IPC.

The production CSP must not include wildcard sources, `unsafe-eval`, localhost development servers, remote `http` sources, remote `https` sources outside `img-src`, broad frame/object/media/worker allowances, or unreviewed custom schemes. Future features that need workers, media, or non-`data:`/`blob:` attachment materialization must update this policy and its tests with the feature-specific threat model.

## Denied By Default

The main webview must not receive these permissions by default:

- SQL plugin permissions, especially execute/write-style access.
- Filesystem permissions.
- Opener permissions. Gmail/Microsoft OAuth and message external links currently open through reviewed Rust Win32 calls in typed Rust commands, not through the frontend opener plugin.
- Shell or process permissions.
- HTTP client permissions.
- Dialog plugin permissions. The Rust backend initializes `tauri-plugin-dialog` only so `save_attachment` can open a backend-owned native save dialog; React must not receive `dialog:*` permissions.
- Wildcard, `allow-all`, or broad plugin default permissions.
- Frontend event emit permissions. The webview may listen/unlisten for backend-owned sync status events but must not emit events into Rust.
- Custom application command access before the command is added to the app ACL manifest, covered by tests, and granted through a narrow capability permission.

Absence of a permission is the denial mechanism. Do not add broad explicit deny lists to make unsafe capability growth look reviewed.

## Custom Commands

`src-tauri/build.rs` keeps the application command ACL list as `APP_COMMANDS`. The reviewed command list is currently:

- `load_mail_shell`: read-only app-data SQLite snapshot for accounts, folders, unread counts, and a bounded recent-message list.
- `search_mail`: read-only app-data SQLite FTS query for validated subject/sender/body search results.
- `allow_remote_content`: typed remote-content preference command that validates account/folder/message ownership and `userRequested=true`, derives sender scope from the stored message sender, and persists only message/sender preference metadata.
- `perform_mail_action`: typed server-affecting reader command for mark-read/unread, move, archive, delete-to-trash, and expunge. It validates user intent, account/folder/message ownership, confirmation requirements, special-folder targets, retry attempts, and redacted errors before calling live IMAP mutation methods and updating the local cache.
- `open_external_link`: typed external-link command that validates `http`, `https`, and `mailto` URLs plus `userRequested=true` before opening through a backend-owned system opener. It rejects embedded credentials, private/local hosts, local paths, scripts, custom schemes, unsafe `mailto` fields, and control-character injection.
- `preview_attachment`: typed reader attachment preview command that validates ownership, app-data source path, size, SHA-256, MIME sniffing, preview allowlist, and explicit user intent before returning plain text or raster-image bytes.
- `save_attachment`: typed reader attachment save command that validates the stored source, opens a backend-owned native save dialog, enforces canonical extension/no overwrite, writes from Rust, and returns no source paths.
- `download_attachment`: typed reader attachment download command that validates the stored source and returns bytes plus a safe export filename without opener handles or local source paths.
- `autosave_composer_draft`: typed local draft autosave through Rust-owned SQLite persistence.
- `discard_composer_draft`: typed local draft discard plus staged composer attachment cleanup.
- `stage_composer_attachment`: typed user-selected byte staging into backend-generated app-data paths; no frontend filesystem path permission.
- `stage_forwarded_attachments`: typed copy of validated cached reader attachment bytes into a composer draft as normal attachments; no source paths, hashes, or sender filenames are accepted from React.
- `send_composer_email`: typed SMTP send orchestration through the Rust send boundary, credential resolver, staged attachment streamer, confirmed-send persistence, and scheduler refresh when provider Sent APPEND is pending.
- `list_recent_recipients`: read-only account-scoped local SQLite autocomplete suggestions from confirmed sends.
- `list_composer_drafts`: inputless read-only local SQLite query for active drafts, returning composer fields and display-safe staged attachment metadata without filesystem paths or hashes.
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
- Opener: keep frontend opener plugin permissions absent. Message links use `open_external_link`; attachments must keep using typed attachment commands and must not auto-open.
- Shell/process: denied for this product unless a future release task explicitly approves a constrained sidecar.
- HTTP: mail-provider OAuth and sync networking should run in Rust. Frontend HTTP client permissions are denied unless a local-only or provider-specific need is documented.
- Dialog: direct frontend dialog permissions are denied. If a future UI needs a picker, prefer a typed Rust command that owns the dialog and validates the result before returning display-safe state.

## Verification

`src-tauri/capabilities/capabilities.test.ts` is part of `pnpm test:frontend` and `pnpm check`. It asserts that:

- `tauri.conf.json` explicitly enables only `default`.
- All capability files are parseable and reviewed by the test.
- The default capability targets only the main window and grants only the reviewed listen/unlisten event permissions plus mail-shell, remote-content preference, typed mail-action, external-link, reader-attachment, composer, account-management, and OAuth loopback command permissions.
- Forbidden plugin permissions, remote origins, wildcard windows, and unreviewed custom-command registrations are absent.
- Rust does not register forbidden Tauri plugins.
- Production CSP exactly matches the reviewed source list and rejects wildcard, `unsafe-eval`, localhost, and remote HTTPS broadening outside `img-src`.
- Development CSP exactly matches the reviewed local Vite/HMR and Tauri IPC source list.

Run `pnpm tauri build --no-bundle` after capability, Tauri config, or build script changes to make Tauri validate the configuration without requiring installer tooling.

## References

- Tauri v2 capabilities: https://v2.tauri.app/security/capabilities/
- Tauri v2 core permissions: https://v2.tauri.app/reference/acl/core-permissions/
