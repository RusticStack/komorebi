# Tauri Capability Model

Last updated: 2026-05-23

## Current Policy

Komorebi's main webview grants only reviewed feature-specific Tauri permissions. The enabled capability set is explicit in `src-tauri/tauri.conf.json`:

- `app.security.capabilities` must be `["default"]` until another window or capability is intentionally introduced.
- `src-tauri/capabilities/default.json` applies only to the `main` window label.
- `default.permissions` contains only reviewed mail-shell and composer commands: `allow-load-mail-shell`, `allow-search-mail`, `allow-autosave-composer-draft`, `allow-discard-composer-draft`, `allow-stage-composer-attachment`, `allow-send-composer-email`, and `allow-list-recent-recipients`.
- No remote origins are allowed to use Tauri APIs.

Tauri capability files are kept as standalone files and referenced by identifier in `tauri.conf.json`. This avoids the Tauri default where every file in `src-tauri/capabilities` is enabled automatically when `security.capabilities` is omitted.

## Denied By Default

The main webview must not receive these permissions by default:

- SQL plugin permissions, especially execute/write-style access.
- Filesystem permissions.
- Opener permissions.
- Shell or process permissions.
- HTTP client permissions.
- Wildcard, `allow-all`, or broad plugin default permissions.
- Custom application command access before the command is added to the app ACL manifest, covered by tests, and granted through a narrow capability permission.

Absence of a permission is the denial mechanism. Do not add broad explicit deny lists to make unsafe capability growth look reviewed.

## Custom Commands

`src-tauri/build.rs` keeps the application command ACL list as `APP_COMMANDS`. The reviewed command list is currently:

- `load_mail_shell`: read-only app-data SQLite snapshot for accounts, folders, unread counts, and a bounded recent-message list.
- `search_mail`: read-only app-data SQLite FTS query for validated subject/sender/body search results.
- `autosave_composer_draft`: typed local draft autosave through Rust-owned SQLite persistence.
- `discard_composer_draft`: typed local draft discard plus staged composer attachment cleanup.
- `stage_composer_attachment`: typed user-selected byte staging into backend-generated app-data paths; no frontend filesystem path permission.
- `send_composer_email`: typed SMTP send orchestration through the Rust send boundary, credential resolver, staged attachment streamer, and confirmed-send persistence.
- `list_recent_recipients`: read-only account-scoped local SQLite autocomplete suggestions from confirmed sends.

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
- The default capability targets only the main window and grants only the reviewed mail-shell and composer command permissions.
- Forbidden plugin permissions, remote origins, wildcard windows, and unreviewed custom-command registrations are absent.
- Rust does not register forbidden Tauri plugins.

Run `pnpm tauri build --no-bundle` after capability, Tauri config, or build script changes to make Tauri validate the configuration without requiring installer tooling.

## References

- Tauri v2 capabilities: https://v2.tauri.app/security/capabilities/
- Tauri v2 core permissions: https://v2.tauri.app/reference/acl/core-permissions/
