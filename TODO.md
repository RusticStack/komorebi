# TODO.md - Komorebi Production Readiness Ledger

Last audit refresh: 2026-05-22

## Audit Scope

- Local docs inspected: `Project Komorebi - Email Client Plan.md`, `README.md`, `docs/architecture/schema.md`, `docs/architecture/sync-state-machine.md`, `docs/architecture/mime-normalization.md`, `docs/architecture/smtp-send.md`, `docs/architecture/local-cache-encryption.md`, `docs/architecture/credential-model.md`, `docs/architecture/provider-auth.md`, `docs/architecture/token-lifecycle.md`, `docs/architecture/tauri-capabilities.md`, `docs/architecture/ipc-and-mutation-boundaries.md`, `docs/architecture/ipc-validation.md`, `docs/architecture/message-rendering.md`, `docs/architecture/attachment-safety.md`, and `docs/architecture/destructive-mail-actions.md`.
- Local code inspected: accountless React app shell in `src/`, safe email rendering boundary in `src/mail/rendering/`, Tauri/Rust shell in `src-tauri/`, SQLite migration SQL in `src-tauri/migrations/`, Rust schema migration runner in `src-tauri/src/schema.rs`, Rust IMAP sync state machine, folder IDLE/polling worker, server flag/folder semantics normalization, and SQLite cache in `src-tauri/src/imap_sync.rs`, Rust MIME normalization in `src-tauri/src/mime_normalization.rs`, Rust SMTP send boundary in `src-tauri/src/smtp_send.rs`, Rust auth policy in `src-tauri/src/auth.rs`, Rust OAuth provider implementation in `src-tauri/src/oauth.rs`, Windows Credential Manager wrapper in `src-tauri/src/credentials.rs`, Rust token lifecycle in `src-tauri/src/token_lifecycle.rs`, Rust IPC boundary policy in `src-tauri/src/ipc_boundary.rs`, Rust IPC validation policy in `src-tauri/src/ipc_validation.rs`, Rust attachment safety policy in `src-tauri/src/attachment_safety.rs`, and Rust destructive mail-action policy in `src-tauri/src/mail_actions.rs`.
- Local tests inspected: `src/App.test.tsx`; `src/mail/rendering/SafeEmailBody.test.tsx`; `src-tauri/capabilities/capabilities.test.ts`; `src-tauri/src/schema.rs` unit tests for migration application, schema objects, constraints, JSON checks, identity uniqueness, folder semantics migration, SMTP send receipt migration, and foreign-key integrity; `src-tauri/src/imap_sync.rs` unit tests for fake IMAP folder discovery, initial fetch, incremental UID fetch, flag updates, full flag preservation, folder special-use/attribute normalization, unknown custom folder usability, expunge handling, UIDVALIDITY reset, IDLE mailbox wakeups, IDLE timeout refresh, polling fallback, capability-probe reconnect, IDLE-wait reconnect, cancellation, account removal, backoff, invalid fixture responses, and unknown-account denial; `src-tauri/src/mime_normalization.rs` unit tests for encoded headers, charsets, multipart alternatives, inline CID images, attachment metadata, malformed MIME, SQLite metadata persistence, and denial paths; `src-tauri/src/smtp_send.rs` unit tests for account transport resolution, MIME composition, auth resolution, attachment streaming/path denial, SMTP failure behavior, confirmed send receipts, and send validation denial paths; `src-tauri/src/auth.rs` unit tests for provider/auth policy; `src-tauri/src/oauth.rs` unit tests for OAuth URLs, PKCE, token exchange/refresh, revocation, and XOAUTH2; `src-tauri/src/credentials.rs` unit tests for credential target naming and redaction; `src-tauri/src/token_lifecycle.rs` unit tests for token refresh, reconnect-needed state, and account removal cleanup; `src-tauri/src/ipc_boundary.rs` unit tests for Rust-only privileged mutation categories and disabled frontend SQL policy; `src-tauri/src/ipc_validation.rs` unit tests for shared IPC denial paths; `src-tauri/src/attachment_safety.rs` unit tests for storage path, display filename, type/size, preview, and auto-open safety; `src-tauri/src/mail_actions.rs` unit tests for local mailbox action semantics.
- Local scripts inspected: `package.json` scripts for Vite build/dev/preview, Tauri CLI, frontend/Rust lint, frontend/Rust test, Rust format check, and root `check`.
- Local config inspected: `.node-version`, `eslint.config.js`, `package.json`, `pnpm-lock.yaml`, `rust-toolchain.toml`, `tsconfig.json`, `tsconfig.node.json`, `vite.config.ts`, `vitest.config.ts`, `src-tauri/build.rs`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src-tauri/capabilities/default.json`.
- Local generated/runtime surfaces inspected: `dist/`, `node_modules/`, `src-tauri/target/`, and `src-tauri/gen/` were generated during verification and are ignored.
- Repository state inspected: `git status --short --branch` succeeds on `main...origin/main`; current P2 SMTP send code/doc changes are uncommitted until the user asks for a commit.
- Generated docs from this audit: `AGENTS.md` and `TODO.md`.
- External research inspected: OpenAI Codex `AGENTS.md` guidance, AGENTS.md standard, Codex best practices and execution-plan guidance, Tauri v2 SQL/security/capabilities/Stronghold/WebView2 docs, Windows Credential Manager Win32 docs, DOMPurify docs, RFC 9051, RFC 6851, `async-imap`, `lettre`, `mail-parser`, SQLite CREATE TABLE/foreign-key/PRAGMA/FTS5 external-content, rebuild, and integrity-check docs, Gmail IMAP/SMTP OAuth/XOAUTH2 docs, Microsoft Exchange Online OAuth/basic-auth docs, Microsoft authorization-code/PKCE docs, OWASP XSS/file-upload/path-traversal guidance, TanStack Virtual/Query docs, i18next docs, and TipTap docs.

## P0 - Repository Substrate And Executable Baseline

Purpose: make the project real enough for agents and humans to build, test, and review. Risk: critical because no implementation can be verified yet. Scope guardrails: do not implement mail features before the scaffold, toolchain, and baseline checks exist.

### Project Seed

- [x] Preserve the product plan as the initial source of truth.
  Evidence: `Project Komorebi - Email Client Plan.md` exists and defines Windows 10/11, Tauri v2, Rust, React/TypeScript, SQLite, multi-account IMAP/SMTP, i18n, search, and composer goals.
- [x] Add a Codex operating contract.
  Evidence: `AGENTS.md` exists from this audit and records current baseline failures, project-specific safety rules, architecture boundaries, and verification gates.
- [x] Add a production-readiness ledger.
  Evidence: this `TODO.md` exists and records audited surfaces, missing implementation, priorities, acceptance criteria, and deferred work.
- [x] Verify source control.
  Evidence: `git status --short --branch` succeeds on `main...origin/main`; `git ls-files` tracks the project plan, `.gitignore`, `AGENTS.md`, and `TODO.md`.
- [x] Add repository ignore rules and decide whether to commit the audit docs.
  Evidence: tracked `.gitignore` excludes Node dependencies, frontend build/cache output, Rust/Tauri targets and bundle artifacts, local runtime/user-data directories, disposable SQLite databases, secrets, logs, diagnostics, and editor/OS noise. Current audit/code changes remain uncommitted because no commit was requested; `AGENTS.md` requires an explicit user request before committing.
- [x] Scaffold the Tauri v2 + React + TypeScript + Vite app.
  Evidence: `package.json`, `pnpm-lock.yaml`, `src/`, `src-tauri/`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json` now exist. `pnpm install` and `pnpm build` pass from the repo root. A controlled `pnpm tauri dev --no-watch` run compiled and launched `target\debug\komorebi.exe`, then the process tree was stopped after verification. The first screen is an accountless 3-pane mail-client shell with no seeded mailbox data, not the starter Tauri/Vite/React marketing page.
- [x] Pin the development toolchain.
  Evidence: `.node-version` pins Node `24.16.0`; `package.json` pins Node `24.16.0` and pnpm `10.33.2`; `pnpm-lock.yaml` exists; `rust-toolchain.toml` pins Rust `1.95.0`, `rustfmt`, `clippy`, and `x86_64-pc-windows-msvc`; `src-tauri/Cargo.lock` exists; `README.md` documents Windows C++ Build Tools and WebView2 expectations.
- [x] Establish root check commands.
  Evidence: `package.json` now defines `pnpm lint`, `pnpm lint:frontend`, `pnpm lint:rust`, `pnpm test`, `pnpm test:frontend`, `pnpm test:rust`, `pnpm fmt:check`, `pnpm build`, and `pnpm check`. `pnpm check` runs Rust format check, frontend ESLint, Rust clippy, frontend Vitest, Rust tests, and frontend build. `src/App.test.tsx` verifies the accountless mail shell, disabled no-account controls, empty states, absence of fake mailbox data, and guards against the starter Tauri page returning. No baseline check failures remain for this task.

## P1 - Privacy, Credential Safety, And User-Data Loss Prevention

Purpose: prevent the earliest design choices from leaking credentials, exposing local system access, or losing mail. Risk: critical because email clients handle sensitive account credentials and private message content. Scope guardrails: this priority should land before real account sync or send is connected.

### Credential And Provider Auth

- [x] Define the credential model before implementing account add.
  Evidence: `docs/architecture/credential-model.md` chooses Windows Credential Manager/DPAPI as the approved Windows-first secret backend, rejects Tauri Store/SQLite/browser storage/logs/diagnostics/frontend state for secrets, forbids plaintext passwords and tokens at rest, defines SQLite account metadata vs Credential Manager entries, and documents account removal, credential rotation, runtime access, and diagnostics clearing rules. `src-tauri/src/credentials.rs` implements namespaced Windows Credential Manager target names and `CredWriteW`/`CredReadW`/`CredDeleteW` storage behind the token lifecycle `CredentialStore` trait.
- [x] Support modern authentication paths for major providers.
  Evidence: `docs/architecture/provider-auth.md` requires OAuth2/XOAUTH2 for Gmail and Microsoft, rejects Gmail/Microsoft manual password and app-password fallback, and limits password/app-password setup to compatible generic IMAP/SMTP providers. `src-tauri/src/auth.rs` enforces the provider/auth matrix with Rust tests covering Gmail/Microsoft denial paths, current OAuth/XOAUTH2 allowed status, and generic password allowance. `src-tauri/src/oauth.rs` implements Gmail and Microsoft OAuth authorization URLs, provider scopes, PKCE S256, authorization-code exchange, refresh-token exchange, Google revocation, Microsoft revocation-unsupported reporting, and XOAUTH2 SASL response construction.
- [x] Implement token refresh and revocation handling.
  Evidence: `docs/architecture/token-lifecycle.md` defines backend-only token refresh, reconnect-needed status, provider revocation policy, local cleanup, and storage rules. `src-tauri/src/token_lifecycle.rs` implements the lifecycle behind credential-store, access-cache, OAuth-client, and account-data traits. `src-tauri/src/oauth.rs` provides the real provider HTTP client implementation used by the lifecycle, and `src-tauri/src/credentials.rs` provides the Windows credential backend. Tests prove expired tokens refresh without putting secrets in frontend-safe views, failed/missing refresh enters reconnect-needed state, account removal clears credentials/access cache/local cache/metadata, local cleanup still happens when provider revocation fails, invalid grants map safely, and provider revocation behavior is explicit.

### Tauri And IPC Security

- [x] Define least-privilege Tauri capabilities.
  Evidence: `src-tauri/tauri.conf.json` explicitly enables only the `default` capability; `src-tauri/capabilities/default.json` targets only the `main` window and grants no frontend Tauri permissions; `docs/architecture/tauri-capabilities.md` documents the denial policy for SQL, filesystem, opener, shell/process, HTTP, remote-origin, wildcard, and custom-command access. `src-tauri/build.rs` keeps the app command ACL list explicit and empty. `src-tauri/capabilities/capabilities.test.ts` reviews all capability JSON files plus Tauri config, forbidden Rust plugin registrations, and custom-command registration. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.
- [x] Keep privileged mutations in Rust commands.
  Evidence: `docs/architecture/ipc-and-mutation-boundaries.md` defines Rust-only ownership for server-affecting mail actions, credential access, attachment writes, destructive deletes, migrations, and sync cache mutation. `src-tauri/src/ipc_boundary.rs` encodes the same categories as `BoundaryOwner::RustCommand` and sets `FRONTEND_SQL_POLICY` to disabled. Rust tests prove every required privileged category has a boundary and frontend SQL remains disabled until a read-only scope is documented. `src-tauri/capabilities/capabilities.test.ts` rejects SQL plugin dependency/imports, frontend SQL loading, `sql:*` capability grants, forbidden local-system plugins, and unregistered custom commands. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.
- [x] Add input validation for every IPC command.
  Evidence: no frontend-callable IPC commands are registered yet, and the validation layer now exists before command exposure. `docs/architecture/ipc-validation.md` defines the command validation contract. `src-tauri/src/ipc_validation.rs` provides shared validators for payload bounds, client-supplied path rejection, raw SQL rejection, checked `http`/`https` URLs without embedded credentials, raw HTML rejection, entity-id shape, and account/folder/message scope lookup. Rust tests cover denial paths for arbitrary paths, raw SQL, unchecked URLs, unsanitized HTML, oversized payloads, invalid ids, unknown account/folder/message IDs, and redacted validation errors. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.

### Message Rendering And Attachments

- [x] Build a safe email HTML rendering boundary.
  Evidence: `docs/architecture/message-rendering.md` defines the rendering boundary. `src/mail/rendering/emailHtml.ts` sanitizes untrusted email HTML with DOMPurify, removes scripts/event handlers/style/resource-loading attributes, blocks remote and inline resources by default, rewrites safe `http`/`https`/`mailto` links for delegated opening, strips unsafe links, and exposes plain text. `src/mail/rendering/SafeEmailBody.tsx` is the only sanctioned `dangerouslySetInnerHTML` sink and renders plain-text fallback without interpreting markup. `src/mail/rendering/xssFixtures.ts` and `src/mail/rendering/SafeEmailBody.test.tsx` cover hostile HTML fixtures, remote resource blocking, safe/unsafe links, and fallback rendering. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.
- [x] Design attachment storage and preview safety.
  Evidence: `docs/architecture/attachment-safety.md` defines attachment storage and preview policy. `src-tauri/src/attachment_safety.rs` generates storage paths from validated account/folder/message/attachment IDs and canonical extensions, keeps sender filenames as sanitized display-only metadata, rejects path traversal in IDs, enforces 25 MiB storage and 5 MiB preview limits, validates declared type against sniffed bytes, rejects explicitly dangerous types, allows preview only for user-requested plain text and raster images, and keeps `AUTO_OPEN_ATTACHMENTS` false. Rust tests cover storage type validation, mismatches, oversized payloads, malicious filenames, path traversal IDs, filename sanitization, preview opt-in/type/size gates, and auto-open denial. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.
- [x] Define destructive mail action semantics.
  Evidence: `docs/architecture/destructive-mail-actions.md` defines typed semantics for mark read/unread, move, archive, delete-to-trash, and expunge. `src-tauri/src/mail_actions.rs` implements typed `MailActionCommand` requests with user-confirmation state, retry attempts, action plans, undo metadata, retry decisions, validation before mutation, and a `MailActionStore` interface. Local mailbox-state tests prove mark-read, move, archive, and delete-to-trash mutate only the test store and produce undo where possible; delete-to-trash and expunge require confirmation; expunge removes deleted messages irreversibly; transient failures return retry advice without mutating state; invalid/unknown targets are rejected before mutation. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.

## P2 - Data Model, Migrations, And Sync Correctness

Purpose: make local cache and remote mail state accurate, recoverable, and searchable. Risk: high because incorrect sync can duplicate, hide, or delete mail. Scope guardrails: do not optimize UI before schema and sync invariants are nailed down.

### Schema And Migrations

- [x] Replace the rough schema with production SQLite migrations.
  Evidence: `src-tauri/migrations/001_initial_schema.sql` defines `schema_migrations`, `settings`, `accounts`, `folders`, `messages`, `attachments`, `templates`, and `template_versions` with primary keys, foreign keys, unique constraints, partial unique indexes, query indexes, timestamp columns/triggers, JSON checks, non-secret account metadata, account/folder/message relations, attachment records, and future-compatible template versioning. `src-tauri/migrations/003_folder_semantics.sql` adds folder provider-attribute preservation through `folders.attributes_json`. `src-tauri/src/schema.rs` applies migrations transactionally, enables foreign keys, records `PRAGMA user_version`, rejects newer databases, and tests migration idempotency, required schema objects, constraints, cascades, timestamp triggers, folder semantics upgrades, and `PRAGMA foreign_key_check`.
- [x] Separate IMAP UID identity from RFC message identity.
  Evidence: `messages.id` is the local row id, while `messages.imap_uid`, `messages.uid_validity`, `messages.rfc_message_id`, and `messages.provider_message_id` are distinct columns. The partial unique index `ux_messages_imap_identity` prevents duplicate `(account_id, folder_id, uid_validity, imap_uid)` rows when IMAP identity exists, while RFC `Message-ID` is indexed but not unique. `schema.rs` tests prove duplicate UID in the same folder/UIDVALIDITY is rejected and the same UID/RFC message can exist in a different folder without collision.
- [x] Implement FTS5 with consistency controls.
  Evidence: `src-tauri/migrations/002_messages_fts.sql` creates `messages_fts` as an FTS5 external-content table over `messages.rowid`, indexes subject, sender name/email, text body, and HTML body, adds insert/update/delete triggers for indexed columns, and rebuilds the index during migration so version-1 databases with existing messages are backfilled. `src-tauri/src/schema.rs` exposes `search_messages_fts`, `rebuild_messages_fts`, and `check_messages_fts_integrity`. Tests prove search returns expected fixture rows, update/delete triggers keep results consistent, rebuild restores missing entries, version-1 upgrade rebuilds existing rows, and FTS integrity checks pass.
- [x] Decide message-body and attachment encryption at rest.
  Evidence: `docs/architecture/local-cache-encryption.md` explicitly decides that the current local mail cache is unencrypted: SQLite message metadata, `body_text`, `body_html`, FTS5 indexed terms, attachment files, and attachment metadata are plaintext local cache content. `Project Komorebi - Email Client Plan.md`, `README.md`, `docs/architecture/schema.md`, `docs/architecture/attachment-safety.md`, and `AGENTS.md` now distinguish local-first/zero-telemetry privacy from encrypted cached mail and forbid stronger encryption claims until a reviewed encryption design, migration, and tests exist. Account secrets remain separate in Windows Credential Manager.

### Sync Engine

- [x] Build the initial IMAP sync state machine.
  Evidence: `docs/architecture/sync-state-machine.md` documents the implemented state flow and current non-live scope. `src-tauri/src/imap_sync.rs` defines the provider-agnostic `ImapSyncServer` trait, `ImapSyncEngine`, cancellation token, backoff policy/waiters, sync reports, remote folder/message/flag models, and `SqliteSyncCache`. The engine discovers folders, upserts folder UIDVALIDITY/UIDNEXT/counts/special-use metadata, performs initial fetch for missing UIDs, fetches only new UIDs on incremental sync, refreshes flags for retained messages, deletes expunged UIDs, clears and refetches folders on UIDVALIDITY change, reconnects after transient server failures, respects cancellation during retry waits and cache mutation loops, and records bounded backoff delays. Fake IMAP fixture tests cover initial sync into migrated SQLite, FTS visibility, incremental UID fetch, flag changes, expunge handling, UIDVALIDITY reset, reconnect/backoff, cancellation, invalid server responses, and unknown account rejection before server calls. Live `async-imap` adapter, attachment extraction/storage, and account UI remain separate TODOs.
- [x] Implement IMAP IDLE with polling fallback.
  Evidence: `src-tauri/src/imap_sync.rs` now defines `ImapIdleServer`, `IdleCapability`, `IdleWaitOutcome`, `FolderWorkerConfig`, `FolderWorkerReport`, and `ImapSyncEngine::run_folder_worker`. Each account/folder worker can wait for IDLE mailbox changes, refresh on IDLE timeout, fall back to polling when IDLE is unavailable or rejected, refresh through `sync_folder_by_remote_id`, reconnect and back off after transient IDLE capability-probe or wait drops, stop cleanly on account removal, and stop cleanly on app-exit cancellation. Fake IMAP tests cover mailbox-change wakeups, timeout refresh, polling fallback, reconnect/backoff after network drop, account removal, and cancellation. `docs/architecture/sync-state-machine.md`, `README.md`, and `AGENTS.md` document that this is provider-agnostic worker logic; the live `async-imap` adapter remains future work.
- [x] Normalize MIME parsing.
  Evidence: `mail-parser` is selected and locked in `src-tauri/Cargo.toml`/`src-tauri/Cargo.lock`. `docs/architecture/mime-normalization.md` documents the backend-only normalization boundary. `src-tauri/src/mime_normalization.rs` parses raw RFC822/RFC5322 bytes into decoded `RemoteMessage` records plus stable `ParsedAttachmentRecord` metadata that matches the `attachments` schema shape, uses the same stable local message id derivation as `SqliteSyncCache`, and persists normalized message plus attachment metadata idempotently to migrated SQLite. Tests cover RFC2047 encoded headers, ISO-8859-1 and UTF-8 charset decoding, multipart plain/html alternatives, inline CID images, attachment metadata with path-like sender filenames, stable attachment ids, SQLite metadata persistence, malformed multipart recovery, invalid account/folder/UID denial, and empty-message rejection. Live `async-imap` adapter integration and attachment file persistence remain separate follow-ups.
- [x] Preserve server flags and folder semantics.
  Evidence: `src-tauri/src/imap_sync.rs` now normalizes raw IMAP flag tokens through `RemoteFlag::from_imap_token`, maps `\Seen`, `\Flagged`, `\Answered`, `\Draft`, and `\Deleted` into boolean columns, preserves `\Recent` and provider/custom keywords in `flags_json`, detects folder roles from IMAP LIST/XLIST/SPECIAL-USE style attributes through `RemoteFolder::from_imap_attributes`, and stores normalized provider attributes in `folders.attributes_json` via migration `003_folder_semantics`. Tests cover standard flag mapping, custom flag preservation, sent/junk/all folder attribute detection, unknown provider attributes, and syncing a custom folder with messages intact.

### Sending And Drafts

- [x] Implement SMTP send with account-specific transport.
  Evidence: `docs/architecture/smtp-send.md` documents the backend-only send boundary. `src-tauri/src/smtp_send.rs` implements typed `send_email` request/report/error models, account-specific SMTP host/port/security resolution, Gmail/Microsoft default SMTP metadata, provider/auth policy enforcement, backend-only OAuth access-token refresh through `TokenLifecycle`, generic password lookup through Windows Credential Manager slots, outbound MIME composition through `lettre`, attachment ID-only streaming through app-generated storage paths, plaintext authenticated SMTP denial, progress events, safe transport errors, and confirmed send receipt insertion only after SMTP acceptance. `src-tauri/migrations/004_smtp_send_receipts.sql` records confirmed local send receipts. Tests cover generic send, Gmail XOAUTH2 defaults, failed transport without receipt insertion, BCC header stripping, fake attachment chunk streaming, path traversal denial, URL-like SMTP host denial, unsafe HTML/recipient/body/id denial, plaintext SMTP denial, and secret redaction. The boundary is not exposed to React yet because Tauri command ACL/capability, composer UI, drafts, and Sent-folder behavior remain separate tasks.
- [ ] Define sent-mail and draft persistence.
  Evidence: plan mentions composer and signatures but not draft autosave or Sent folder upload.
  Acceptance: drafts autosave locally, can be discarded intentionally, survive restart, and are cleared or converted to sent state after send; Sent folder behavior is documented per provider capability.
- [ ] Add per-account signatures.
  Evidence: schema sketch includes `signature_html`, but no validation or editor behavior exists.
  Acceptance: signatures are stored per account, sanitized, inserted predictably, and editable through settings.

## P3 - Core Frontend UX

Purpose: deliver the actual usable personal email client promised by the plan. Risk: high because a mail client must be fast, legible, accessible, and safe under large message counts. Scope guardrails: avoid decorative or marketing UI; build repeated workflows first.

### Layout And Navigation

- [ ] Build the 3-pane mail shell.
  Evidence: an accountless 3-pane app shell exists with empty states and no seeded mailbox data, but it has no account/folder/message state model, unified inbox toggle, settings entry, or connected workflows yet.
  Acceptance: first screen includes account/folder navigation, unified inbox toggle, message list, message detail, search entry, composer entry, and settings entry with loading/empty/error states.
- [ ] Implement multi-account navigation.
  Evidence: plan requires unified and per-account views, but no state model exists.
  Acceptance: users can switch unified/all accounts vs one account, see unread counts, and navigate folders without losing selected message state unexpectedly.
- [ ] Add keyboard and accessibility support.
  Evidence: the accountless shell has basic landmarks and disabled controls, but no implemented keyboard workflow for list navigation, reading, composing, search, folder switching, or dialogs exists yet.
  Acceptance: list navigation, reading, composing, search, folder switching, and dialogs are keyboard reachable; focus states are visible; screen-reader labels exist for controls.

### Message List, Reading, And Search

- [ ] Implement virtualized message list.
  Evidence: plan requires `@tanstack/react-virtual`; no code exists.
  Acceptance: 10,000+ fixture messages render without UI lag; row selection, unread, flagged, sender, subject, date, and snippet remain stable during scroll.
- [ ] Implement safe message detail view.
  Evidence: the sanitizer and `SafeEmailBody` boundary are implemented and tested, but the app shell currently has no selected message workflow, attachment list, reply/forward routing, remote-content preference UI, or fixture-backed mailbox state.
  Acceptance: plain and sanitized HTML views work; remote images are blocked by default; attachment list is safe; reply/forward actions are available but disabled or routed until implemented.
- [ ] Implement FTS-backed search UX.
  Evidence: backend FTS5 schema and search helper exist, but no frontend search workflow or query state exists.
  Acceptance: search spans subject, sender, and body across active accounts; results identify account/folder; empty/no-index states are handled; query input is debounced and cancellable.

### Composer

- [ ] Build the TipTap composer MVP.
  Evidence: no editor exists.
  Acceptance: To/CC/BCC, subject, body, basic formatting, lists, blockquote, links, attachments, inline images, account selector, signature insertion, draft state, send progress, and validation errors work.
- [ ] Add address validation and autocomplete.
  Evidence: plan requires autocomplete but no contacts/history store exists.
  Acceptance: typed addresses validate before send, duplicate recipients are handled, recent recipients are local-only, and malformed addresses produce clear errors.
- [ ] Add rich formatting beyond MVP only behind tests.
  Evidence: plan lists tables, code blocks, font controls, colors, highlights, and alignment.
  Acceptance: each added extension persists, sanitizes, serializes to outgoing MIME HTML, and has regression tests.

### Localization

- [ ] Add i18n setup for English and `pt-PT`.
  Evidence: no translation files exist.
  Acceptance: language detection works on first launch, fallback is English, `pt-PT` is complete for implemented UI, and no touched user-facing strings remain hard-coded.
- [ ] Add locale-aware formatting.
  Evidence: plan mentions language but not dates/times/counts.
  Acceptance: dates, times, unread counts, sizes, and status messages format according to selected language/locale.

## P4 - Tests, Fixtures, And Verification Harnesses

Purpose: create evidence that the client works without risking real mailboxes. Risk: high because mail parsing, sync, and rendering bugs are easy to miss manually. Scope guardrails: fixtures and fake servers should be small enough to run in CI.

### Test Infrastructure

- [ ] Add Rust unit and integration tests.
  Evidence: Rust unit tests now cover schema migrations, constraints, FTS5 search/rebuild/integrity behavior, folder semantics and SMTP send receipt migrations, fake IMAP sync state-machine and IDLE/polling worker behavior, flag/folder semantics normalization, MIME normalization, SMTP send boundary behavior, provider auth policy, OAuth provider behavior, token lifecycle, IPC mutation-boundary policy, shared IPC input validation, attachment safety, and destructive mail-action semantics. No real frontend-callable command, live IMAP adapter, protocol-level SMTP server, or credential-backend integration tests exist yet.
  Acceptance: `cargo test --manifest-path src-tauri/Cargo.toml --all-features` runs sync, schema, command validation, MIME, attachment, and credential-boundary tests.
- [ ] Add frontend tests.
  Evidence: `src/App.test.tsx` is a smoke test for the accountless shell only; `SafeEmailBody.test.tsx` covers renderer safety, but no state/query, i18n, or composer tests exist yet.
  Acceptance: Vitest/React Testing Library cover state/query behavior, renderer safety, i18n, composer validation, and core navigation.
- [ ] Add end-to-end UI smoke tests.
  Evidence: no Playwright/WebDriver setup exists.
  Acceptance: tests launch the frontend or Tauri shell, load fixture data, navigate inbox/search/message/detail/composer, and capture failure artifacts.

### Fixture Contracts

- [ ] Create a MIME fixture corpus.
  Evidence: `src-tauri/src/mime_normalization.rs` includes focused inline Rust fixtures for parser behavior, but no reusable fixture-file corpus exists yet.
  Acceptance: fixtures include encoded headers, Portuguese text, Japanese/non-ASCII names, HTML + plain alternatives, nested multipart, CID images, attachments, malformed boundaries, phishing links, and XSS payloads.
- [ ] Create fake IMAP/SMTP test surfaces.
  Evidence: `src-tauri/src/imap_sync.rs` includes a fake IMAP fixture for sync state-machine and folder-worker tests, including reconnects, expunges, UIDVALIDITY reset, IDLE wakeups, IDLE timeout refresh, polling fallback, account removal, cancellation, and invalid responses. `src-tauri/src/smtp_send.rs` includes fake SMTP sender, auth resolver, and attachment store surfaces for backend send tests. There is still no protocol-level IMAP fixture, no protocol-level SMTP fixture, no auth-failure server fixture, and no large-mailbox fixture.
  Acceptance: sync and send tests run locally without real accounts; fake server can simulate reconnects, IDLE updates, expunges, auth failures, and large mailboxes.
- [ ] Create performance fixtures.
  Evidence: plan promises sub-50MB memory and instant search but no measurement harness exists.
  Acceptance: generated local fixtures can populate 10,000+ messages and representative attachments; benchmark scripts report memory, search latency, and list interaction metrics.

### Security Regression Tests

- [ ] Test HTML sanitization bypass cases.
  Evidence: `src/mail/rendering/SafeEmailBody.test.tsx` covers current XSS fixtures, but there is no broader bypass corpus for CSS abuse variants, SVG/MathML edge cases beyond the current fixtures, malformed parser cases, and post-sanitization mutation cases.
  Acceptance: script tags, event handlers, `javascript:` URLs, data URLs where disallowed, CSS abuse, SVG/MathML abuse, and post-sanitization mutation cases are covered.
- [ ] Test attachment path traversal and unsafe open paths.
  Evidence: `src-tauri/src/attachment_safety.rs` unit tests cover malicious sender filenames, path traversal in storage IDs, and auto-open denial. Real download/open commands and archive extraction do not exist yet, so integration tests for filesystem writes and opener denial remain missing.
  Acceptance: when attachment commands exist, malicious filenames cannot escape the app directory, overwrite existing files unexpectedly, or auto-open executable content.
- [ ] Test IPC denial paths.
  Evidence: no commands exist.
  Acceptance: invalid IDs, arbitrary paths, raw SQL, unowned account/folder/message IDs, oversized payloads, and unauthorized operations are rejected.

## P5 - CI, Packaging, And Release Readiness

Purpose: make releases repeatable and auditable on Windows. Risk: medium-high because unsigned or unverified desktop releases can fail users or train agents to trust broken builds. Scope guardrails: release automation should come after the scaffold and basic tests are stable.

### CI

- [ ] Add Windows CI.
  Evidence: no `.github/workflows` or equivalent CI config exists.
  Acceptance: CI runs frontend install/lint/test/build, Rust fmt/clippy/test, and Tauri build or package smoke on Windows.
- [ ] Add dependency and license review.
  Evidence: dependency manifests exist and `rusqlite` with bundled SQLite was added for schema migrations, but there is no dependency/license/advisory review workflow yet.
  Acceptance: production dependency additions are reviewed for license/security; Rust and npm advisories are checked by CI or documented manual commands.
- [ ] Add generated-artifact checks.
  Evidence: `.gitignore` excludes generated outputs, but there is no CI/review automation yet to reject generated dependency/build/runtime artifacts.
  Acceptance: CI or review docs prevent committing `node_modules`, `dist`, `target`, runtime DBs, fixture-generated blobs, and installer outputs unless explicitly intended.

### Windows Release

- [ ] Configure app identity and installer metadata.
  Evidence: `src-tauri/tauri.conf.json` defines product name, version, bundle identifier, window defaults, and icon paths, but publisher metadata, installer behavior, uninstall behavior, and release-package verification are not complete.
  Acceptance: product name, bundle identifier, icons, version, publisher metadata, install path, and uninstall behavior are configured and tested.
- [ ] Define code-signing plan.
  Evidence: no release docs exist.
  Acceptance: signing certificate requirements, local/CI signing flow, unsigned-build policy, and verification commands are documented before public release.
- [ ] Verify WebView2 prerequisite behavior.
  Evidence: plan relies on WebView2 but no installer exists.
  Acceptance: installer behavior is tested on supported Windows targets or documented according to Tauri behavior; failure path is user-readable.
- [ ] Defer updater until security prerequisites exist.
  Evidence: no updater exists.
  Acceptance: if updater is added, update URL, signing, rollback, user consent, and privacy behavior are documented and tested.

## P6 - Observability, Privacy Operations, And Supportability

Purpose: help users diagnose sync issues without violating the zero-telemetry promise. Risk: medium because silent failures make email clients unusable, but logs can leak private data. Scope guardrails: local diagnostics only unless an explicit opt-in is implemented.

### Local Diagnostics

- [ ] Add structured local logging with redaction.
  Evidence: no logging exists.
  Acceptance: logs include sync state, provider errors, retries, and command failures while redacting tokens, passwords, message bodies, subject lines by default, and sensitive addresses where practical.
- [ ] Add a diagnostics/export flow.
  Evidence: no UI exists.
  Acceptance: user can export a redacted diagnostic bundle that excludes credentials and private message content; export path is user-selected.
- [ ] Add health/status indicators.
  Evidence: no sync worker or UI exists.
  Acceptance: per-account status shows connected, syncing, idle, offline, auth needed, error, and last successful sync without requiring logs.

### Privacy And Data Lifecycle

- [ ] Write a zero-telemetry privacy statement.
  Evidence: README and architecture docs state zero telemetry and local-first constraints, but there is no complete user-facing privacy statement covering all stored local data, account network calls, diagnostics, and removal/cache behavior.
  Acceptance: README or docs state what data is stored locally, what network calls happen, what is never collected, and how users remove account/cache data.
- [ ] Define account removal and cache cleanup.
  Evidence: `docs/architecture/token-lifecycle.md` defines credential/token cleanup behavior and local cache deletion hooks for account removal, but user-facing retention choices, UI flow, sync-worker stop orchestration, and full cache cleanup implementation remain missing.
  Acceptance: removing an account revokes/wipes credentials, stops sync, and offers clear local cache deletion/retention behavior.
- [ ] Define backup/export/import posture.
  Evidence: not covered by plan.
  Acceptance: either implemented or documented as unsupported; local cache backup does not expose unencrypted secrets.

## P7 - Documentation, Maintainability, And Agent Continuity

Purpose: keep future autonomous work scoped and reviewable. Risk: medium because stale docs can cause unsafe implementation choices. Scope guardrails: docs should describe real behavior, not intended behavior, unless clearly marked as planned.

### Human Docs

- [x] Add README setup and usage docs.
  Evidence: `README.md` covers project goals, pinned toolchain, Windows prerequisites, setup, dev/build/check commands, generated output rules, privacy/security posture, and current limitations for account setup, live adapters, frontend command exposure, drafts, Sent-folder upload, and local-cache encryption.
- [x] Add architecture documentation.
  Evidence: `docs/architecture/` now documents credential storage, provider auth, token lifecycle, Tauri capabilities, IPC/mutation boundaries, IPC validation, schema/migrations/FTS, sync state machine, MIME normalization, SMTP send, local cache encryption posture, safe message rendering, attachment safety, and destructive mail-action semantics.
- [ ] Add provider setup guides.
  Evidence: no account setup docs exist.
  Acceptance: Gmail, Microsoft/Outlook, and generic IMAP/SMTP setup requirements are documented with supported auth modes and known limitations.

### Agent Docs

- [ ] Keep `AGENTS.md` updated as implementation lands.
  Evidence: `AGENTS.md` names the scaffolded commands, generated paths, known baseline gaps, current toolchain/check setup, and implemented credential/auth/token/capability/IPC/rendering/attachment/destructive-action/schema/sync/MIME/SMTP boundaries; this remains an ongoing maintenance task as implementation lands.
  Acceptance: AGENTS stays synchronized with actual commands, known baseline failures, generated paths, and project-specific traps from real bugs.
- [ ] Keep this ledger current.
  Evidence: this ledger is updated through the backend SMTP send boundary work on 2026-05-22.
  Acceptance: completed tasks are checked with evidence; broad tasks are reopened as narrower follow-ups; last audit date is updated after each full audit.
- [ ] Add execution-plan template only if long-running tasks need it.
  Evidence: no `.agent/PLANS.md` exists.
  Acceptance: if added, `AGENTS.md` explains when to use it and the template is self-contained enough for multi-hour Codex work.

## P8 - Performance, Compatibility, And Product Claims

Purpose: prove the product promises under realistic Windows desktop conditions. Risk: medium because premature claims create trust and release risk. Scope guardrails: benchmarks must use representative fixtures and release builds.

### Performance

- [ ] Measure memory footprint.
  Evidence: plan promises sub-50MB memory footprint but no app or measurement exists.
  Acceptance: release-build measurements on Windows are recorded for idle, 10,000-message inbox, search, sync, and composer scenarios; docs only claim numbers that are measured.
- [ ] Measure search latency.
  Evidence: plan promises instantaneous local search and backend FTS5 exists, but no benchmark harness or frontend search workflow exists.
  Acceptance: benchmark reports query latency across representative message counts and body sizes; slow paths have regression thresholds.
- [ ] Measure sync and attachment handling.
  Evidence: fake-backed sync, attachment safety, MIME metadata parsing, and SMTP attachment streaming boundaries exist, but no live adapter, attachment file persistence worker, or benchmark harness exists.
  Acceptance: benchmarks cover initial sync, incremental sync, reconnect, large mailbox, large attachment metadata, and attachment download without excessive memory growth.

### Compatibility

- [ ] Validate Windows 10/11 support.
  Evidence: plan targets Windows 10/11 but no builds exist.
  Acceptance: tested Windows versions, WebView2 behavior, installer behavior, and known OS-specific limitations are documented.
- [ ] Validate international content.
  Evidence: plan includes English and Portuguese, codename contains Japanese text, and Rust MIME parser tests cover some non-ASCII headers/addresses; no i18n UI or broad international fixture corpus exists yet.
  Acceptance: fixtures verify non-ASCII names, subjects, body text, filenames, dates, and right-sized UI rendering in English and `pt-PT`.

## Deferred And Future Scope

These items are not active production-readiness blockers for the first stable personal email client unless the user promotes them.

- [ ] Block-based template builder.
  Evidence: plan labels templates as future/post-launch.
  Acceptance if promoted: TipTap custom node schema, drag/drop UX, persistence, sanitization, preview, send serialization, and tests exist.
- [ ] Template save/edit/load workflow.
  Evidence: plan labels template persistence as future.
  Acceptance if promoted: templates are stored locally, versioned, sanitized, searchable, and insertable into composer without corrupting drafts.
- [ ] Non-Windows platforms.
  Evidence: plan targets Windows 10/11 only.
  Acceptance if promoted: macOS/Linux prerequisites, WebView behavior, packaging, signing, and OS credential storage are researched and tested.
- [ ] Cloud sync, AI features, analytics, or remote services.
  Evidence: plan promises local-first zero telemetry and does not require these.
  Acceptance if promoted: explicit user approval, privacy review, threat model, opt-in UX, and docs are required before implementation.
