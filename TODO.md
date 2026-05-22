# TODO.md - Komorebi Production Readiness Ledger

Last audit refresh: 2026-05-22

## Audit Scope

- Local docs inspected: `Project Komorebi - Email Client Plan.md`, `README.md`, `docs/architecture/credential-model.md`, `docs/architecture/provider-auth.md`, `docs/architecture/token-lifecycle.md`, `docs/architecture/tauri-capabilities.md`, `docs/architecture/ipc-and-mutation-boundaries.md`, and `docs/architecture/ipc-validation.md`.
- Local code inspected: scaffolded React app in `src/`, Tauri/Rust shell in `src-tauri/`, Rust auth policy in `src-tauri/src/auth.rs`, Rust token lifecycle in `src-tauri/src/token_lifecycle.rs`, Rust IPC boundary policy in `src-tauri/src/ipc_boundary.rs`, and Rust IPC validation policy in `src-tauri/src/ipc_validation.rs`.
- Local tests inspected: `src/App.test.tsx`; `src-tauri/capabilities/capabilities.test.ts`; `src-tauri/src/auth.rs` unit tests for provider/auth policy; `src-tauri/src/token_lifecycle.rs` unit tests for token refresh, reconnect-needed state, and account removal cleanup; `src-tauri/src/ipc_boundary.rs` unit tests for Rust-only privileged mutation categories and disabled frontend SQL policy; `src-tauri/src/ipc_validation.rs` unit tests for shared IPC denial paths.
- Local scripts inspected: `package.json` scripts for Vite build/dev/preview, Tauri CLI, frontend/Rust lint, frontend/Rust test, Rust format check, and root `check`.
- Local config inspected: `.node-version`, `eslint.config.js`, `package.json`, `pnpm-lock.yaml`, `rust-toolchain.toml`, `tsconfig.json`, `tsconfig.node.json`, `vite.config.ts`, `vitest.config.ts`, `src-tauri/build.rs`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src-tauri/capabilities/default.json`.
- Local generated/runtime surfaces inspected: `dist/`, `node_modules/`, `src-tauri/target/`, and `src-tauri/gen/` were generated during verification and are ignored.
- Repository state inspected: `git status --short --branch` succeeds on `main...origin/main`; scaffold, README, toolchain pins, docs, and ignore rules are tracked; IPC boundary and validation architecture docs/tests/code are modified until committed.
- Generated docs from this audit: `AGENTS.md` and `TODO.md`.
- External research inspected: OpenAI Codex `AGENTS.md` guidance, AGENTS.md standard, Codex best practices and execution-plan guidance, Tauri v2 SQL/security/capabilities/Stronghold/WebView2 docs, `async-imap`, `lettre`, `mail-parser`, SQLite FTS5 docs, Gmail IMAP/SMTP OAuth docs, Microsoft Exchange Online OAuth/basic-auth docs, OWASP XSS/file-upload guidance, TanStack Virtual/Query docs, i18next docs, and TipTap docs.

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
  Evidence: tracked `.gitignore` excludes Node dependencies, frontend build/cache output, Rust/Tauri targets and bundle artifacts, local runtime/user-data directories, disposable SQLite databases, secrets, logs, diagnostics, and editor/OS noise. The new scaffold files remain untracked because no commit was requested; `AGENTS.md` requires an explicit user request before committing.
- [x] Scaffold the Tauri v2 + React + TypeScript + Vite app.
  Evidence: `package.json`, `pnpm-lock.yaml`, `src/`, `src-tauri/`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json` now exist. `pnpm install` and `pnpm build` pass from the repo root. A controlled `pnpm tauri dev --no-watch` run compiled and launched `target\debug\komorebi.exe`, then the process tree was stopped after verification. The first screen is a placeholder 3-pane mail-client shell, not the starter Tauri/Vite/React marketing page.
- [x] Pin the development toolchain.
  Evidence: `.node-version` pins Node `24.16.0`; `package.json` pins Node `24.16.0` and pnpm `10.33.2`; `pnpm-lock.yaml` exists; `rust-toolchain.toml` pins Rust `1.95.0`, `rustfmt`, `clippy`, and `x86_64-pc-windows-msvc`; `src-tauri/Cargo.lock` exists; `README.md` documents Windows C++ Build Tools and WebView2 expectations.
- [x] Establish root check commands.
  Evidence: `package.json` now defines `pnpm lint`, `pnpm lint:frontend`, `pnpm lint:rust`, `pnpm test`, `pnpm test:frontend`, `pnpm test:rust`, `pnpm fmt:check`, `pnpm build`, and `pnpm check`. `pnpm check` runs Rust format check, frontend ESLint, Rust clippy, frontend Vitest, Rust tests, and frontend build. `src/App.test.tsx` verifies the placeholder mail shell and guards against the starter Tauri page returning. No baseline check failures remain for this task.

## P1 - Privacy, Credential Safety, And User-Data Loss Prevention

Purpose: prevent the earliest design choices from leaking credentials, exposing local system access, or losing mail. Risk: critical because email clients handle sensitive account credentials and private message content. Scope guardrails: this priority should land before real account sync or send is connected.

### Credential And Provider Auth

- [x] Define the credential model before implementing account add.
  Evidence: `docs/architecture/credential-model.md` chooses Windows Credential Manager/DPAPI as the approved Windows-first secret backend, rejects Tauri Store/SQLite/browser storage/logs/diagnostics/frontend state for secrets, forbids plaintext passwords and tokens at rest, defines SQLite account metadata vs Credential Manager entries, and documents account removal, credential rotation, runtime access, and diagnostics clearing rules.
- [x] Support modern authentication paths for major providers.
  Evidence: `docs/architecture/provider-auth.md` requires OAuth2/XOAUTH2 for Gmail and Microsoft, explicitly marks those providers unsupported until OAuth is implemented, rejects Gmail/Microsoft manual password and app-password fallback, and limits password/app-password setup to compatible generic IMAP/SMTP providers. `src-tauri/src/auth.rs` enforces the provider/auth matrix with Rust tests covering Gmail/Microsoft denial paths, OAuth-pending unsupported status, future XOAUTH2 allowed status, and generic password allowance.
- [x] Implement token refresh and revocation handling.
  Evidence: `docs/architecture/token-lifecycle.md` defines backend-only token refresh, reconnect-needed status, provider revocation policy, local cleanup, and storage rules. `src-tauri/src/token_lifecycle.rs` implements the lifecycle behind credential-store, access-cache, OAuth-client, and account-data traits. Tests prove expired tokens refresh without putting secrets in frontend-safe views, failed/missing refresh enters reconnect-needed state, account removal clears credentials/access cache/local cache/metadata, and local cleanup still happens when provider revocation fails.

### Tauri And IPC Security

- [x] Define least-privilege Tauri capabilities.
  Evidence: `src-tauri/tauri.conf.json` explicitly enables only the `default` capability; `src-tauri/capabilities/default.json` targets only the `main` window and grants no frontend Tauri permissions; `docs/architecture/tauri-capabilities.md` documents the denial policy for SQL, filesystem, opener, shell/process, HTTP, remote-origin, wildcard, and custom-command access. `src-tauri/build.rs` keeps the app command ACL list explicit and empty. `src-tauri/capabilities/capabilities.test.ts` reviews all capability JSON files plus Tauri config, forbidden Rust plugin registrations, and custom-command registration. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.
- [x] Keep privileged mutations in Rust commands.
  Evidence: `docs/architecture/ipc-and-mutation-boundaries.md` defines Rust-only ownership for server-affecting mail actions, credential access, attachment writes, destructive deletes, migrations, and sync cache mutation. `src-tauri/src/ipc_boundary.rs` encodes the same categories as `BoundaryOwner::RustCommand` and sets `FRONTEND_SQL_POLICY` to disabled. Rust tests prove every required privileged category has a boundary and frontend SQL remains disabled until a read-only scope is documented. `src-tauri/capabilities/capabilities.test.ts` rejects SQL plugin dependency/imports, frontend SQL loading, `sql:*` capability grants, forbidden local-system plugins, and unregistered custom commands. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.
- [x] Add input validation for every IPC command.
  Evidence: no frontend-callable IPC commands are registered yet, and the validation layer now exists before command exposure. `docs/architecture/ipc-validation.md` defines the command validation contract. `src-tauri/src/ipc_validation.rs` provides shared validators for payload bounds, client-supplied path rejection, raw SQL rejection, checked `http`/`https` URLs without embedded credentials, raw HTML rejection, entity-id shape, and account/folder/message scope lookup. Rust tests cover denial paths for arbitrary paths, raw SQL, unchecked URLs, unsanitized HTML, oversized payloads, invalid ids, unknown account/folder/message IDs, and redacted validation errors. Verification: `pnpm check` and `pnpm tauri build --no-bundle` pass.

### Message Rendering And Attachments

- [ ] Build a safe email HTML rendering boundary.
  Evidence: plan requires `body_html` and safe previewing, but no sanitizer/rendering design exists.
  Acceptance: untrusted HTML is sanitized, scripts/event handlers are removed, remote resources are blocked by default, links are rewritten or opened safely, plain-text fallback exists, and XSS fixture tests fail before and pass after.
- [ ] Design attachment storage and preview safety.
  Evidence: plan stores `attachments.local_path`, but no filename/path or preview policy exists.
  Acceptance: app-generated storage paths are used, sender filenames are display-only, path traversal is impossible, size/type limits exist, previews are opt-in and type-checked, and auto-open is forbidden.
- [ ] Define destructive mail action semantics.
  Evidence: plan mentions mark-as-read and organization but not deletes, moves, trash, expunge, or confirmation flows.
  Acceptance: move/delete/archive/mark-read commands are typed, reversible where possible, retry-aware, and tested against local fake server state.

## P2 - Data Model, Migrations, And Sync Correctness

Purpose: make local cache and remote mail state accurate, recoverable, and searchable. Risk: high because incorrect sync can duplicate, hide, or delete mail. Scope guardrails: do not optimize UI before schema and sync invariants are nailed down.

### Schema And Migrations

- [ ] Replace the rough schema with production SQLite migrations.
  Evidence: the plan has table sketches only; no migrations exist.
  Acceptance: migrations define primary keys, foreign keys, unique constraints, indexes, timestamps, account/folder/message relations, attachment records, settings, and future-compatible template tables where needed.
- [ ] Separate IMAP UID identity from RFC message identity.
  Evidence: plan labels `messages.message_id` as "IMAP UID", which conflates two different concepts.
  Acceptance: schema stores account id, folder id, UID, UIDVALIDITY, RFC `Message-ID`, and provider metadata distinctly; uniqueness rules prevent cross-folder/account collisions.
- [ ] Implement FTS5 with consistency controls.
  Evidence: plan names `messages_fts`, but not triggers, rebuild, or integrity checks.
  Acceptance: FTS table has insert/update/delete triggers or an equivalent tested update path; migrations can rebuild FTS; integrity check exists after indexed-column migrations; search returns expected fixture rows.
- [ ] Decide message-body and attachment encryption at rest.
  Evidence: plan promises privacy but does not say whether cached message content is encrypted.
  Acceptance: docs either implement encrypted local mail storage or clearly state the local cache is unencrypted; UI and README avoid stronger claims than the implementation.

### Sync Engine

- [ ] Build the initial IMAP sync state machine.
  Evidence: no Rust sync code exists.
  Acceptance: initial folder discovery, initial message fetch, incremental UID fetch, flag updates, expunge handling, reconnect, cancellation, and backoff are implemented and tested with fake server fixtures.
- [ ] Implement IMAP IDLE with polling fallback.
  Evidence: plan requires concurrent IDLE connections but no timeout/reconnect policy exists.
  Acceptance: each account/folder worker can idle, periodically refresh, recover from network drop, and stop cleanly on account removal or app exit.
- [ ] Normalize MIME parsing.
  Evidence: plan stores `body_text`, `body_html`, and attachments, but no parser is selected.
  Acceptance: inbound messages parse encoded headers, charsets, multipart alternatives, inline images, attachments, malformed input, and non-ASCII addresses into stable database records.
- [ ] Preserve server flags and folder semantics.
  Evidence: plan lists folder types and `is_read`/`is_flagged`, but not provider special-use or flag mapping.
  Acceptance: seen, flagged, answered, draft, deleted, recent/custom flags are mapped or preserved; special-use folders are detected; unknown folders remain usable.

### Sending And Drafts

- [ ] Implement SMTP send with account-specific transport.
  Evidence: no `send_email` command exists.
  Acceptance: selected sender account resolves SMTP host/auth safely, composes MIME through `lettre`, streams attachments without excessive memory, reports progress/errors, and records sent state only after confirmed send.
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
  Evidence: no frontend exists.
  Acceptance: first screen includes account/folder navigation, unified inbox toggle, message list, message detail, search entry, composer entry, and settings entry with loading/empty/error states.
- [ ] Implement multi-account navigation.
  Evidence: plan requires unified and per-account views, but no state model exists.
  Acceptance: users can switch unified/all accounts vs one account, see unread counts, and navigate folders without losing selected message state unexpectedly.
- [ ] Add keyboard and accessibility support.
  Evidence: no UI exists.
  Acceptance: list navigation, reading, composing, search, folder switching, and dialogs are keyboard reachable; focus states are visible; screen-reader labels exist for controls.

### Message List, Reading, And Search

- [ ] Implement virtualized message list.
  Evidence: plan requires `@tanstack/react-virtual`; no code exists.
  Acceptance: 10,000+ fixture messages render without UI lag; row selection, unread, flagged, sender, subject, date, and snippet remain stable during scroll.
- [ ] Implement safe message detail view.
  Evidence: no reader exists.
  Acceptance: plain and sanitized HTML views work; remote images are blocked by default; attachment list is safe; reply/forward actions are available but disabled or routed until implemented.
- [ ] Implement FTS-backed search UX.
  Evidence: plan requires instant FTS5 search but no frontend or DB exists.
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
  Evidence: Rust unit tests now cover provider auth policy, token lifecycle, IPC mutation-boundary policy, and shared IPC input validation, but no sync, schema, real command, MIME, attachment, or credential-backend integration tests exist yet.
  Acceptance: `cargo test --manifest-path src-tauri/Cargo.toml --all-features` runs sync, schema, command validation, MIME, attachment, and credential-boundary tests.
- [ ] Add frontend tests.
  Evidence: `src/App.test.tsx` is a smoke test for the placeholder shell only; no state/query, renderer-safety, i18n, or composer tests exist yet.
  Acceptance: Vitest/React Testing Library cover state/query behavior, renderer safety, i18n, composer validation, and core navigation.
- [ ] Add end-to-end UI smoke tests.
  Evidence: no Playwright/WebDriver setup exists.
  Acceptance: tests launch the frontend or Tauri shell, load fixture data, navigate inbox/search/message/detail/composer, and capture failure artifacts.

### Fixture Contracts

- [ ] Create a MIME fixture corpus.
  Evidence: no fixtures exist.
  Acceptance: fixtures include encoded headers, Portuguese text, Japanese/non-ASCII names, HTML + plain alternatives, nested multipart, CID images, attachments, malformed boundaries, phishing links, and XSS payloads.
- [ ] Create fake IMAP/SMTP test surfaces.
  Evidence: live-provider testing would be unsafe and no local fake server exists.
  Acceptance: sync and send tests run locally without real accounts; fake server can simulate reconnects, IDLE updates, expunges, auth failures, and large mailboxes.
- [ ] Create performance fixtures.
  Evidence: plan promises sub-50MB memory and instant search but no measurement harness exists.
  Acceptance: generated local fixtures can populate 10,000+ messages and representative attachments; benchmark scripts report memory, search latency, and list interaction metrics.

### Security Regression Tests

- [ ] Test HTML sanitization bypass cases.
  Evidence: no renderer exists.
  Acceptance: script tags, event handlers, `javascript:` URLs, data URLs where disallowed, CSS abuse, SVG/MathML abuse, and post-sanitization mutation cases are covered.
- [ ] Test attachment path traversal and unsafe open paths.
  Evidence: attachment design is missing.
  Acceptance: malicious filenames cannot escape the app directory, overwrite existing files unexpectedly, or auto-open executable content.
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
  Evidence: no dependency manifests exist.
  Acceptance: production dependency additions are reviewed for license/security; Rust and npm advisories are checked by CI or documented manual commands.
- [ ] Add generated-artifact checks.
  Evidence: no gitignore or generated output exists.
  Acceptance: CI or review docs prevent committing `node_modules`, `dist`, `target`, runtime DBs, fixture-generated blobs, and installer outputs unless explicitly intended.

### Windows Release

- [ ] Configure app identity and installer metadata.
  Evidence: no Tauri config exists.
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
  Evidence: plan promises zero telemetry/ads but no user-facing doc exists.
  Acceptance: README or docs state what data is stored locally, what network calls happen, what is never collected, and how users remove account/cache data.
- [ ] Define account removal and cache cleanup.
  Evidence: no data lifecycle docs exist.
  Acceptance: removing an account revokes/wipes credentials, stops sync, and offers clear local cache deletion/retention behavior.
- [ ] Define backup/export/import posture.
  Evidence: not covered by plan.
  Acceptance: either implemented or documented as unsupported; local cache backup does not expose unencrypted secrets.

## P7 - Documentation, Maintainability, And Agent Continuity

Purpose: keep future autonomous work scoped and reviewable. Risk: medium because stale docs can cause unsafe implementation choices. Scope guardrails: docs should describe real behavior, not intended behavior, unless clearly marked as planned.

### Human Docs

- [ ] Add README setup and usage docs.
  Evidence: no README exists.
  Acceptance: README covers project goals, prerequisites, install, dev, test, build, privacy posture, and current limitations.
- [ ] Add architecture documentation.
  Evidence: only the plan exists.
  Acceptance: docs describe process boundaries, IPC, capabilities, schema, sync state machine, credential storage, rendering safety, and attachment lifecycle.
- [ ] Add provider setup guides.
  Evidence: no account setup docs exist.
  Acceptance: Gmail, Microsoft/Outlook, and generic IMAP/SMTP setup requirements are documented with supported auth modes and known limitations.

### Agent Docs

- [ ] Keep `AGENTS.md` updated as implementation lands.
  Evidence: `AGENTS.md` names the scaffolded commands, generated paths, known baseline gaps, and current toolchain/check setup; this remains an ongoing maintenance task as implementation lands.
  Acceptance: AGENTS stays synchronized with actual commands, known baseline failures, generated paths, and project-specific traps from real bugs.
- [ ] Keep this ledger current.
  Evidence: this is the first audit refresh.
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
  Evidence: plan promises instantaneous local search but no FTS implementation exists.
  Acceptance: benchmark reports query latency across representative message counts and body sizes; slow paths have regression thresholds.
- [ ] Measure sync and attachment handling.
  Evidence: no sync or attachment code exists.
  Acceptance: benchmarks cover initial sync, incremental sync, reconnect, large mailbox, large attachment metadata, and attachment download without excessive memory growth.

### Compatibility

- [ ] Validate Windows 10/11 support.
  Evidence: plan targets Windows 10/11 but no builds exist.
  Acceptance: tested Windows versions, WebView2 behavior, installer behavior, and known OS-specific limitations are documented.
- [ ] Validate international content.
  Evidence: plan includes English and Portuguese, and codename contains Japanese text, but no parser or UI exists.
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
