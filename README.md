# Komorebi

Komorebi is a privacy-first Windows 10/11 desktop email client built with Tauri v2, React, TypeScript, Vite, and Rust.

The current implementation is an executable, state-driven mail shell with no seeded mailbox data. In Tauri runtime it can create, reauthenticate, remove, and read local account metadata through typed commands, read account, folder, unread-count, and recent-message navigation data from the local SQLite cache, preview/save/download stored reader attachments through typed backend validation, run app-managed live IMAP sync workers for configured accounts, send through configured SMTP accounts from the typed composer command, reopen active local drafts, and upload pending Microsoft/generic Sent copies through IMAP APPEND after confirmed SMTP acceptance. Composer drafts are local-only until sent or discarded; provider Drafts-folder sync is intentionally deferred. The app still needs opt-in live IMAP/provider smoke coverage, external attachment open/extract decisions, and release packaging before production mail-provider claims.

## Toolchain

Use the pinned versions below for local development:

- Node.js `24.16.0`, pinned in `.node-version` and `package.json`.
- pnpm `10.33.2`, pinned in `package.json` via `packageManager`.
- Rust `1.95.0` for `x86_64-pc-windows-msvc`, pinned in `rust-toolchain.toml`.
- Cargo lockfile: `src-tauri/Cargo.lock`.
- pnpm lockfile: `pnpm-lock.yaml`.

If `pnpm` is not available but Node is installed, install the pinned package manager:

```powershell
npm install -g pnpm@10.33.2
```

## Windows Prerequisites

Tauri development on Windows requires Microsoft C++ Build Tools and Microsoft Edge WebView2.

Install the Visual Studio Build Tools with the `Desktop development with C++` workload. Keep the Rust host toolchain on the MSVC target, `x86_64-pc-windows-msvc`, because this project pins and verifies that target.

WebView2 is the rendering engine used by Tauri on Windows. It is normally present on Windows 10 version 1803 and later, and on Windows 11. If a supported machine is missing WebView2, install the Microsoft Edge WebView2 Evergreen Runtime before running the app. Release packaging still needs a dedicated verification pass before public distribution.

Reference: https://v2.tauri.app/start/prerequisites/

## Security Model

The accepted credential architecture is documented in [docs/architecture/credential-model.md](docs/architecture/credential-model.md). Komorebi uses Windows Credential Manager/DPAPI for account secrets. Plaintext passwords and tokens are forbidden in SQLite, Tauri Store, browser storage, logs, diagnostics, and frontend state.

Provider authentication policy is documented in [docs/architecture/provider-auth.md](docs/architecture/provider-auth.md). Gmail and Microsoft account setup is OAuth2/XOAUTH2-only through a backend-owned loopback authorization flow; React supplies only account metadata plus OAuth client ID and never receives codes or tokens. Generic password/app-password setup is limited to compatible generic IMAP/SMTP providers. Generic setup testing now performs backend-owned IMAP LOGIN plus SMTP AUTH over TLS or STARTTLS before account metadata is persisted; background sync starts only for active configured accounts and live provider smoke tests remain opt-in.

Token refresh, reconnect-needed state, provider revocation, and account-removal cleanup are documented in [docs/architecture/token-lifecycle.md](docs/architecture/token-lifecycle.md). React receives redacted auth status only; token refresh and revocation handling stay in Rust.

Tauri capability policy is documented in [docs/architecture/tauri-capabilities.md](docs/architecture/tauri-capabilities.md). The main webview currently grants only reviewed listen-only sync status events plus mail-shell, remote-content preference, typed mail-action, external-link opening, reader-attachment, composer, account-management, and OAuth loopback command permissions: `core:event:allow-listen`, `core:event:allow-unlisten`, `allow-load-mail-shell`, `allow-search-mail`, `allow-allow-remote-content`, `allow-perform-mail-action`, `allow-open-external-link`, `allow-preview-attachment`, `allow-save-attachment`, `allow-download-attachment`, `allow-autosave-composer-draft`, `allow-discard-composer-draft`, `allow-stage-composer-attachment`, `allow-send-composer-email`, `allow-list-recent-recipients`, `allow-list-composer-drafts`, `allow-create-account`, `allow-reauth-account`, `allow-remove-account`, `allow-update-account-settings`, `allow-test-account-connection`, `allow-get-account-auth-status`, `allow-start-oauth-account-setup`, and `allow-start-oauth-account-reauth`. SQL, filesystem, opener plugin, shell/process, HTTP, frontend dialog, frontend event emit, and unreviewed custom-command access are denied unless a future feature adds a scoped capability and test coverage. Rust initializes the dialog plugin only for the backend-owned `save_attachment` native save dialog.

The desktop shell also uses a reviewed Tauri CSP. Production script execution is local-only, connect sources are limited to local app/Tauri IPC, object/frame/worker/form surfaces are blocked, and the only current content allowances are inline CSS for rich editor/email styles, `data:`/`blob:` images for staged inline composer images, and HTTPS image loading after the remote-content preference workflow explicitly allows a message or sender.

IPC and mutation boundaries are documented in [docs/architecture/ipc-and-mutation-boundaries.md](docs/architecture/ipc-and-mutation-boundaries.md). Server-affecting actions, credential access, attachment writes, destructive deletes, migrations, and sync cache mutation stay in typed Rust commands or internal Rust workers. Frontend SQL is disabled until a read-only policy is implemented and tested.

Frontend mail-shell behavior is documented in [docs/architecture/frontend-mail-shell.md](docs/architecture/frontend-mail-shell.md). The React shell now has local-cache account/folder/message loading, safe attachment metadata projection, FTS-backed search with debounce/cancellation/no-index states, unified inbox scoping with persisted per-account opt-out, unread counts from sync state, background sync status projection with last-successful-sync detail, listen-only status event refresh, provider-specific reconnect drawer, virtualized message rows for large folders, keyboard navigation, screen-reader labels, sanitized message detail rendering, remote-resource blocking by default, typed remote-content preferences for a message or sender, typed external link opening for sanitized `http`/`https`/`mailto` links, typed mail actions for mark-read/unread, move, archive, delete-to-trash, and expunge with confirmation/undo/retry UI, typed attachment preview/save/download controls, reader focus, a TipTap composer with tested rich formatting, duplicate handling, keyboard recent-recipient autocomplete, local-only draft retention copy, Drafts drawer reload/reopen UI, settings drawer account setup/reauth/removal/OAuth loopback/settings persistence flows, loading/empty/error states, and selection preservation across compatible navigation changes. Reply/forward routing and external attachment open/extract remain future work.

Localization uses `i18next` and `react-i18next` with English (`en`) and Portuguese (`pt-PT`) resources. First launch detects `navigator.languages`/`navigator.language`, maps Portuguese locales to `pt-PT`, and falls back to English. Message timestamps, counts, search-result status text, attachment sizes, and missing sender/attachment fallback labels format through the active locale. A language picker and persisted locale preference remain future work.

SQLite schema and migrations are documented in [docs/architecture/schema.md](docs/architecture/schema.md). The migrations define settings, accounts, folders, preserved folder attributes, messages, attachments, remote-content preferences, templates, template versions, FTS5 message search, MIME parser-warning storage, primary/foreign keys, uniqueness rules, indexes, timestamps, and schema-version tracking.

IMAP sync state-machine behavior is documented in [docs/architecture/sync-state-machine.md](docs/architecture/sync-state-machine.md). The Rust backend now has a provider-agnostic sync engine, folder IDLE worker with polling fallback, server flag/folder semantics normalization, SQLite cache integration tested against fake IMAP fixtures, a blocking `native-tls` live IMAP adapter for TLS/STARTTLS, LOGIN/XOAUTH2, UID search/fetch, STORE, EXPUNGE, IDLE, Sent-folder Message-ID search, literal APPEND, reconnect, cancellation, timeouts, and redacted errors, plus app-managed background sync orchestration for configured accounts with pending Sent upload, redacted status rows, last-successful-sync timestamps, and backend-owned status events. Opt-in live-provider smoke tests and any future Tokio/`async-imap` migration remain future work.

Inbound MIME normalization is documented in [docs/architecture/mime-normalization.md](docs/architecture/mime-normalization.md). Raw message bytes are parsed in Rust with `mail-parser` into decoded message fields, stable attachment metadata records, storage-safe attachment bytes, and parser-warning JSON. Live IMAP sync now feeds fetched RFC822/RFC5322 bytes through this boundary and persists message bodies, safe attachment files, attachment metadata, warning JSON, and FTS-visible content to SQLite. Reader preview/save/download commands revalidate stored source paths, hashes, size, and sniffed MIME before returning bytes or writing selected files.

SMTP send behavior is documented in [docs/architecture/smtp-send.md](docs/architecture/smtp-send.md). The Rust backend resolves account-specific SMTP metadata/auth, loads stored OAuth client configuration for Gmail/Microsoft token refresh, composes outbound MIME with `lettre`, preserves sanitized rich composer HTML, inserts sanitized account signatures, streams attachment sources through backend-owned paths, records local send receipts only after SMTP acceptance, and retains temporary RFC822 payloads only for provider policies that require later IMAP Sent upload. The TipTap composer calls the typed `send_composer_email` wrapper; active drafts are not uploaded to provider Drafts folders.

Draft and Sent persistence is documented in [docs/architecture/draft-and-sent-persistence.md](docs/architecture/draft-and-sent-persistence.md). The Rust backend stores active drafts locally, reloads active drafts through `list_composer_drafts`, intentionally clears discarded or sent draft content, stages outbound composer attachments under backend-generated app-data paths, converts a source draft after confirmed SMTP acceptance, updates local-only recent-recipient autocomplete rows, records provider-aware Sent-copy state, and uploads pending Microsoft/generic Sent copies through backend IMAP APPEND while avoiding Gmail duplicate copies. The composer tells users drafts stay local to this device until sent or discarded.

Account signatures are documented in [docs/architecture/account-signatures.md](docs/architecture/account-signatures.md). Per-account signatures are stored as sanitized account metadata, edited through the React settings component, saved through the typed `update_account_settings` command with display name and local preferences, inserted through managed composer markers, and applied again by the SMTP MIME boundary.

Local cache encryption posture is documented in [docs/architecture/local-cache-encryption.md](docs/architecture/local-cache-encryption.md). The current local mail cache is unencrypted: cached message bodies, active draft content, pending Sent APPEND payloads, FTS index terms, and attachment files are stored locally as plaintext. Account secrets remain separate and are stored through Windows Credential Manager.

IPC input validation is documented in [docs/architecture/ipc-validation.md](docs/architecture/ipc-validation.md). Future commands must reject arbitrary paths, raw SQL, unchecked URLs, unsanitized HTML, unbounded payloads, and unknown or unowned account/folder/message/attachment IDs before side effects.

Safe email rendering is documented in [docs/architecture/message-rendering.md](docs/architecture/message-rendering.md). Untrusted email HTML is sanitized through a single React boundary, remote resources are blocked by default, links are rewritten for typed backend opening with explicit user intent, and plain-text fallback is preserved.

Attachment safety is documented in [docs/architecture/attachment-safety.md](docs/architecture/attachment-safety.md). Attachment storage paths are app-generated in Rust, sender filenames are display-only, path traversal is rejected, size/type limits are enforced, previews are opt-in and type-checked, and auto-open is forbidden.

Destructive mail action semantics are documented in [docs/architecture/destructive-mail-actions.md](docs/architecture/destructive-mail-actions.md). Move, archive, delete-to-trash, mark-read, and expunge behavior is exposed through typed Rust commands, confirmation-aware, reversible where possible, backed by live IMAP mutation mapping, and tested with fake stores, local protocol fixtures, frontend tests, and browser E2E fixtures. Delete defaults to move-to-trash; direct hard delete is not exposed.

## Setup

```powershell
pnpm install
```

## Development

Run the Vite frontend:

```powershell
pnpm dev
```

Run the Tauri desktop shell:

```powershell
pnpm tauri dev
```

Build the frontend:

```powershell
pnpm build
```

Validate Tauri configuration without producing installers:

```powershell
pnpm tauri build --no-bundle
```

Run the full root check suite:

```powershell
pnpm check
```

Run individual root checks:

```powershell
pnpm fmt:check
pnpm lint
pnpm test
pnpm build
```

The root scripts fan out to frontend and Rust checks:

```powershell
pnpm lint:frontend
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
pnpm test:frontend
cargo test --manifest-path src-tauri/Cargo.toml --all-features
cargo fmt --manifest-path src-tauri/Cargo.toml --all --check
```

Run the Playwright fixture smoke test:

```powershell
pnpm test:e2e
```

Run the local mail-shell performance benchmark:

```powershell
pnpm perf:mail-shell
```

The benchmark launches the test-only `/e2e/` Vite entry with a generated 10,000-message mailbox and representative attachment metadata. It reports browser heap usage, search latency, and virtualized list interaction timings in the Playwright output and attached JSON report under ignored `test-results/perf/`.

If Chromium browser binaries are missing on a fresh machine, install them with:

```powershell
pnpm exec playwright install chromium
```

## Opt-In Live SMTP Smoke

The ignored Rust live SMTP smoke test sends one message through a user-provided test account and deletes its temporary credential entry afterward. It never runs in normal `cargo test` or `pnpm check`.

Generic IMAP/SMTP requires `KOMOREBI_LIVE_SMTP_SMOKE=1`, `KOMOREBI_LIVE_SMTP_PROVIDER=generic_imap_smtp`, `KOMOREBI_LIVE_SMTP_FROM`, `KOMOREBI_LIVE_SMTP_TO`, `KOMOREBI_LIVE_SMTP_SECRET`, IMAP host/port/security, and SMTP host/port/security. Gmail/Microsoft require `KOMOREBI_LIVE_SMTP_PROVIDER=gmail` or `microsoft`, `KOMOREBI_LIVE_SMTP_REFRESH_TOKEN`, `KOMOREBI_LIVE_SMTP_OAUTH_CLIENT_ID`, and `KOMOREBI_LIVE_SMTP_OAUTH_REDIRECT_URI`.

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test live_smtp_smoke -- --ignored --nocapture
```

## Generated Outputs

Do not commit generated dependency, build, or runtime outputs such as `node_modules/`, `dist/`, `src-tauri/target/`, `src-tauri/gen/`, local app-data directories, disposable SQLite databases, or diagnostics logs.
