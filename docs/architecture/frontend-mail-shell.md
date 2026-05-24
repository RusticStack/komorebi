# Frontend Mail Shell

Status: 3-pane shell, multi-account local-cache navigation, FTS-backed search UX, keyboard/accessibility workflows, virtualized message rendering, safe message detail rendering with typed attachment preview/save/download, English/pt-PT i18n, TipTap composer with tested rich formatting, local-only draft disclosure, and draft reload/reopen UI, local recent-recipient autocomplete, settings account-management/preference/signature persistence UI, Gmail/Microsoft OAuth loopback UI wiring, and background sync/auth status surface implemented.
Last reviewed: 2026-05-24.

## Current Scope

The code source of truth is:

- `src/mail/shell/mailShellState.ts`
- `src/mail/shell/mailShellDataSource.ts`
- `src/mail/shell/MailClientShell.tsx`
- `src/mail/composer/TipTapComposer.tsx`
- `src/mail/composer/composerRichText.ts`
- `src/i18n/`
- `src/App.tsx`
- `src/App.css`
- `src-tauri/src/attachment_commands.rs`
- `src-tauri/src/mail_shell.rs`
- `src-tauri/src/sync_scheduler.rs`

The shell is the first screen of the app. It is not a landing page and it does not seed fake mailbox data in production/default render.

The default state is an accountless local shell with:

- account navigation
- disabled add-account entry
- disabled unified inbox toggle
- folder navigation area
- message list area
- search entry
- disabled composer entry
- reader pane
- settings entry
- empty states

Tests may inject fixture mail through `initialData` to verify workflows. In Tauri runtime, `load_mail_shell` reads the local app-data SQLite cache and returns the current account/folder/message shell snapshot; `search_mail` queries the same cache through FTS5 when the user searches; reader attachment actions call typed preview/save/download commands; composer data-source functions call the typed Rust draft, attachment staging, recent-recipient, and send commands. In a plain browser/Vite runtime, the shell stays empty because no Tauri IPC bridge exists.

## State Model

`MailShellState` tracks:

- accounts
- folders
- messages
- message attachments
- account/folder/message loading status
- unified inbox mode
- selected account
- selected folder
- selected message
- search query

The navigation state is frontend-local. Cache reads come from the Rust `load_mail_shell` and `search_mail` commands. Reader attachment preview/save/download and composer draft/save/send actions go through typed Rust commands; the shell still does not expose frontend SQL, credentials, arbitrary paths, or provider command strings.

## Local Cache Read Model

`src-tauri/src/mail_shell.rs` owns the shell read model. The command:

- opens `mail-cache.sqlite3` under the Tauri app data directory
- applies Rust-owned migrations before reading
- excludes removed accounts
- returns redacted account metadata, folder counts, and a bounded recent-message snapshot
- returns only display-safe attachment metadata for snapshot messages: id, sanitized display filename, MIME type, byte size, and disposition
- maps folder unread counts from sync state into account/folder/unified unread badges
- maps `account:{account_id}:sync_worker_status` rows plus folder `sync_state` into account `syncing`, `idle`, `offline`, `reconnect-needed`, and `error` status values for the shell
- returns redacted sync status metadata only: status update time, last successful sync time, safe user-facing status message, and safe action code
- never returns `credential_ref`, passwords, tokens, attachment storage paths, storage-relative filenames, checksums, or raw SQL results

`load_mail_shell` has no frontend-supplied arguments. It is exposed through `load_mail_shell`, `src-tauri/build.rs`, and the `allow-load-mail-shell` capability permission.

`search_mail` accepts only a typed query/scope request. It validates query length, raw SQL-shaped input, raw HTML, account id, and folder id before executing a parameterized FTS query. It returns full safe message rows with the same attachment metadata redaction policy as the shell snapshot, so results outside the bounded recent-message snapshot can still open in the reader. It is exposed through `search_mail`, `src-tauri/build.rs`, and the `allow-search-mail` capability permission.

## Navigation Behavior

The unified inbox toggle controls whether the list is scoped to all accounts or a selected account.

In unified mode:

- the virtual `Unified Inbox` folder shows inbox messages across accounts
- account buttons remain visible but selecting an account switches into account-scoped mode

In account mode:

- account selection scopes folders and messages to that account
- folder selection scopes messages to that folder
- selecting a different account or folder preserves the selected message when it still belongs to the new scope
- selecting a scope that does not contain the selected message clears the selection

## Search Behavior

When no Tauri search function is available, test fixtures use local in-memory filtering by sender, sender email, subject, preview, and plain body text.

In Tauri runtime, non-empty search input uses `search_mail` after a debounce. Search is cancellable from React's perspective: changing the query or scope aborts/ignores stale responses. Backend search spans subject, sender name/email, plain body, and HTML body across active accounts. In unified mode it searches all active-account folders; in account mode it searches the selected account; when a folder is selected it narrows to that folder. Result rows show account/folder origin.

Search UI states:

- debouncing/loading: `Searching local index`
- ready with results: result count plus virtualized rows
- ready with no results: no-match state
- unavailable index: no-index state
- backend error: retryable search error state

## Sync And Auth Status

Account rows show the current account status, unread count, and last successful sync time using the active locale. The Rust read model derives status from account auth state, scheduler status rows, and folder sync state:

- `syncing`: account worker or folder sync is active.
- `idle`: the account is connected and no current sync work is reported.
- `offline`: the account is disconnected or stopped without an active auth error.
- `reconnect-needed`: backend credential or token refresh state requires reauthentication.
- `error`: a sync worker reported a redacted non-auth failure.

`src-tauri/src/sync_scheduler.rs` emits `komorebi://sync-status` when a worker status row changes. The frontend data source subscribes through Tauri's listen-only event permission and reloads `load_mail_shell`; it does not trust event payloads as the durable account model.

The account status drawer shows:

- status
- last successful sync
- last status update
- redacted actionable error detail
- provider-specific reconnect controls

OAuth reconnect for Gmail/Microsoft calls `start_oauth_account_reauth` with the account id and no secret; Rust reuses the stored non-secret OAuth client configuration and opens the system browser. Generic IMAP/SMTP reconnect asks for a replacement password/app-password and calls `reauth_account`. React never receives passwords after submission, OAuth codes, access tokens, refresh tokens, or credential target names.

## Localization

The frontend uses `i18next` and `react-i18next` through `src/i18n/index.ts`.

- Supported languages are English (`en`) and Portuguese (`pt-PT`).
- First-launch language detection reads `navigator.languages` and `navigator.language`, maps any Portuguese locale to `pt-PT`, maps English locales to `en`, and falls back to English.
- Translation resources live in `src/i18n/locales/en.json` and `src/i18n/locales/pt-PT.json`.
- Runtime shell, reader, settings, safe-message, and composer strings route through `t(...)`; the app name remains literal branding.
- Missing sender and attachment display-name fallbacks are applied in React through i18n keys, not synthesized as English labels in the Rust cache read model.
- `src/i18n/localeFormat.ts` centralizes `Intl` formatting for message dates/times, unread and search-result counts, attachment sizes, and pluralized count/status text.
- Message rows and reader headers format raw cache timestamps for the active locale. Unparseable legacy display strings are shown unchanged.
- Account/folder unread badges, search result status counts, and attachment-size labels use the active locale's number formatting.
- `pt-PT` resource completeness is tested against English resources.

There is no language picker, persisted locale setting, relative date wording, or broad long-string screenshot corpus yet.

## Message List Virtualization

The message list uses `@tanstack/react-virtual` in `MailClientShell.tsx`.

- Rows use deterministic message IDs as virtual item keys.
- Row height is fixed at `136px`; sender, subject, date, flag state, and snippet are clamped so scrolling does not shift row geometry.
- The virtualizer overscans a small row window instead of mounting every visible-scope message.
- Message rows keep `aria-setsize`, `aria-posinset`, `aria-current`, unread styling, visible flagged state, sender, subject, date, and snippet while scrolling.
- Keyboard navigation scrolls off-screen targets into the virtual window before focus is restored.
- Tests inject 10,000 messages and assert the DOM stays bounded while first/last row state, selection, focus, and reader state remain stable.

## Keyboard And Accessibility

The shell must remain usable from the keyboard before live sync data exists:

- `/` and Ctrl/Cmd+F focus the search field when message data is ready; shortcuts are ignored from editable fields and while a drawer is open.
- Account, folder, and message lists support Arrow Up/Down/Left/Right plus Home/End to move focus and update the active scope or message.
- Enter or Space on a focused message selects it and moves focus to the reader pane.
- The reader pane is programmatically focusable so keyboard users have a stable reading target after selecting a message.
- Account, folder, and message controls expose screen-reader labels with account email/status/unread counts, folder counts, and message read/flagged/sender/subject/date state.
- Composer and Settings drawers render as modal dialogs with `aria-modal`, initial focus, trapped Tab navigation at the dialog edges, Escape close, and focus return to the opener.
- Visible focus states are required for buttons, form fields, reader focus, and drawer focus.

## Reader

The reader pane supports:

- no selected message
- loading state
- error state
- selected message state
- plain-text fallback through React text output
- sanitized HTML through `SafeEmailBody`
- default remote-resource blocking status
- attachment metadata list with typed Preview, Save, and Download actions when the backend commands are available
- disabled reply, forward, archive, mark-read, and remote-content controls until typed backend commands exist

Message body rendering reuses `SafeEmailBody`. Do not add another untrusted HTML sink in the shell.

Attachment rows in the shell snapshot/search response are metadata only. The frontend receives display filenames, MIME type, size, and disposition; it does not receive `storage_relative_path`, local filesystem paths, sender filenames as storage hints, hashes, or preview URLs. Preview/Save/Download buttons are enabled only when the corresponding typed function is injected by Tauri runtime or tests. They send account/folder/message/attachment ids plus `userRequested=true`; Rust revalidates ownership, source path, hash, size, MIME/type sniffing, preview allowlist, and save destination policy before returning bytes or writing a file. External open remains unavailable.

Remote content remains blocked by the sanitizer. The reader exposes a disabled `Load remote content` control as the future preference entry point, but no per-message or per-sender remote-content preference is persisted yet.

## TipTap Composer

The composer drawer uses TipTap through `src/mail/composer/TipTapComposer.tsx`. It includes:

- account selector
- To, CC, BCC, and subject fields
- address parsing/validation for send
- duplicate-recipient normalization across To/CC/BCC before draft save or send
- keyboard-accessible local recent-recipient autocomplete for To/CC/BCC
- formatting controls: bold, italic, strikethrough, inline code, code block, bullet list, numbered list, block quote, alignment, tables, font family, font size, text color, and highlights
- safe link application/removal for `http`, `https`, and `mailto` URLs without embedded credentials
- attachment picker using the browser `File` API
- inline image picker for raster image types, inserted into the editor as a data URL and persisted/sent as a backend staged attachment CID
- per-account signature insertion using the existing signature sanitizer/helper
- debounced local draft autosave plus manual Save draft
- draft-retention status copy stating drafts stay local to this device until sent or discarded
- Drafts toolbar entry and Drafts drawer for reopening active local drafts from SQLite
- Discard action
- Send action with visible progress states returned by the Rust send boundary

The Tauri runtime wiring is:

- `autosave_composer_draft` for local draft rows
- `discard_composer_draft` for discard and staged attachment cleanup
- `stage_composer_attachment` for backend-generated app-data storage and display-safe metadata
- `send_composer_email` for draft validation, SMTP send, confirmed send persistence, and staged attachment cleanup after success
- `list_recent_recipients` for account-scoped local autocomplete suggestions from SQLite
- `list_composer_drafts` for active local draft reload/reopen data with display-safe staged attachment metadata
- `preview_attachment`, `save_attachment`, and `download_attachment` for reader attachment byte access after backend validation

The frontend never receives attachment storage paths and never sends local filesystem paths to Rust. It sends user-selected bytes from the WebView file input to the composer staging command. Reader save uses a backend-owned native dialog; React sends only attachment identity and explicit intent. The backend validates MIME/type/size and owns the final app-data path.

Active local drafts load on app startup and after composer/account lifecycle changes. Reopening a draft hydrates the same draft id, account, recipients, subject, rich body/signature content, and staged attachment metadata into TipTap. The account selector is locked for reopened drafts so the frontend cannot accidentally save an existing draft under another account. Discard and confirmed send remove the local draft from the Drafts list.

Rich composer HTML is centralized in `src/mail/composer/composerRichText.ts`. The TipTap extension list and sanitizer allow only the supported strikethrough/table/code/span/mark/link/image subset, restrict inline styles to whitelisted `text-align`, `color`, `background-color`, `font-size`, and `font-family` values, normalize table span attributes, rewrite staged inline image data URLs to CIDs, and remove unsafe links, event handlers, scripts, remote image URLs, and unsupported CSS before autosave or send.

Recent-recipient autocomplete is local-only. Confirmed sends update the `recent_recipients` table in the local cache after SMTP acceptance; suggestions expose only display name, email address, use count, and last-used timestamp. Duplicate recipients are removed with first occurrence winning in To, then CC, then BCC order, and the composer shows a non-error notice while preserving malformed-address validation errors.

Current composer limits:

- Provider Drafts-folder upload is intentionally deferred; active drafts are local-only until sent or discarded.
- Reply/forward routing into the composer is not implemented.
- Live SMTP sends require configured account credentials and stored OAuth client config for Gmail/Microsoft; pending Microsoft/generic Sent APPEND upload is handled by the backend scheduler after confirmed send, while live smoke coverage is ignored and user-opt-in only.

## Settings Entry

The Settings drawer uses `AccountSignatureSettings` for account management and signature editing. It exposes typed UI flows for:

- compatible generic IMAP/SMTP setup with host, port, security, username, and password/app-password fields
- Gmail and Microsoft setup as OAuth2-only forms using non-secret account metadata plus OAuth client ID, then `start_oauth_account_setup`
- live generic IMAP/SMTP setup validation through `test_account_connection`
- generic reauthentication through `reauth_account`
- Gmail/Microsoft OAuth reauthentication through `start_oauth_account_reauth`
- account removal through `remove_account`
- redacted auth status through `get_account_auth_status`
- display-name, signature HTML, and per-account unified-inbox preference persistence through `update_account_settings`, followed by a `load_mail_shell` cache reload

With no accounts it still shows the accountless settings state plus the add-account form. With account data it edits persisted account display metadata, local unified-inbox preferences, and sanitized signatures through typed account commands. Signature previews continue to use `SafeEmailBody`.

The Gmail/Microsoft setup and reauth controls never ask for passwords, app passwords, refresh tokens, authorization codes, or access tokens. The backend command opens the system browser and returns only a redacted setup response or error reason. Status-drawer OAuth reconnect reuses the stored non-secret OAuth client id from Rust so the status surface can launch the provider-specific flow without adding a secret field. The generic Test setup button is disabled for OAuth providers because provider consent is validated by the loopback flow, not by a secret-bearing configuration test.

## Loading, Empty, And Error States

The shell exposes loading/empty/error states for:

- account list
- folder list
- message list
- reader pane

These states are wired to the local-cache command when running in Tauri. Retry buttons reload the same read-only cache snapshot.

## Tests

`src/App.test.tsx` covers:

- accountless default render without fake mailbox data
- local-cache loader behavior through an injected data source
- retry after a failed backend cache load
- unified inbox visibility
- account navigation
- folder navigation
- selected-message preservation for compatible scope changes
- safe message detail rendering with sanitized HTML, remote image blocking, typed attachment preview/save/download handlers, and disabled reply/forward/remote-content controls
- local fixture search and FTS-backed search with debounce, cancellation, result origin labels, no-match state, no-index state, and retryable error state
- TipTap composer address validation, duplicate recipient handling, keyboard autocomplete from recent recipients, signature insertion, local-only draft disclosure, active draft list/reopen hydration, draft save, attachment staging, inline image CID conversion, send command request shape, send progress, and validation errors
- TipTap rich formatting extension output and sanitizer behavior in `src/mail/composer/TipTapComposer.test.tsx`
- message selection and reader rendering
- keyboard search focus, list navigation, folder navigation, reader focus, dialog focus trapping, Escape close, and focus return
- 10,000-message virtualized rendering with bounded DOM row count and stable row state during scroll
- composer drawer entry
- settings drawer entry
- settings account preference/signature save command and cache reload
- account sync status drawer, last-successful-sync display, generic reconnect, OAuth reconnect, and backend sync-status event refresh
- loading/error shell states
- first-launch language detection, English fallback, `pt-PT` resource completeness, locale-aware date/time/count/size/status formatting, localized missing sender/attachment fallbacks, and first-run `pt-PT` shell rendering in `src/i18n/i18n.test.tsx`

`src/mail/shell/mailShellState.test.ts` covers pure shell state rules: unified/account/folder scope filtering, local search fallback behavior, cache-reload selection preservation, and stale backend row clearing.

`src/mail/shell/mailShellDataSource.test.ts` covers the frontend backend boundary: Tauri runtime detection, typed `invoke` command payloads for shell load/search, attachment preview/save/download, recent-recipient lookup, composer draft list loading, composer autosave, generic account commands, OAuth loopback commands, sync-status event subscription, and cancellation before invoke or after a stale response.

`e2e/mail-shell.smoke.spec.ts` covers a browser-level Playwright smoke path against the test-only `/e2e/` Vite entry. It loads fixture mail through `src/e2e/main.tsx`, opens inbox and archive messages, previews/saves a fixture attachment through typed handlers, runs backend-shaped search, opens a search result in the reader, opens the Drafts drawer, reopens a saved local draft in the composer, verifies the local-only draft disclosure, sends through typed fixture handlers with the original draft id, and retains screenshots, video, and traces on failure through `playwright.config.ts`.

`perf/mail-shell.benchmark.spec.ts` covers a browser-level benchmark path against the same test-only `/e2e/` entry with `?fixture=performance`. It uses the deterministic generator in `src/perf/mailPerformanceFixtures.ts` to populate 10,000 synthetic messages and representative attachment metadata, then reports Chromium heap usage, search latency, virtualized scroll timing, selected-row timing, and mounted row count through `pnpm perf:mail-shell` and `playwright.perf.config.ts`. The benchmark also guards the browser virtualizer path by requiring fewer than 80 mounted message rows for the 10,000-message fixture.

Rust tests in `src-tauri/src/mail_shell.rs` cover reading accounts, folders, unread counts, recent messages, safe attachment metadata projection, missing-label fallback ownership, removed-account exclusion, credential-reference redaction, attachment storage-path redaction, FTS search success, account/folder scoping, validation denial paths, and missing-index reporting from disposable SQLite databases.

Rust composer and SMTP tests cover sanitized rich composer HTML persistence and outgoing MIME serialization for strikethrough, tables, code blocks, font styles, colors, highlights, and alignment.

Normal automated tests use no real mailbox data, real credentials, or live provider connections. Generic setup includes a backend live-login path for user-entered providers at runtime; tests cover it with local protocol fixtures and fake testers only. The live SMTP smoke test is ignored by default and requires explicit user-provided test-account environment variables.

## Not Implemented

- TanStack Query integration.
- Live reply/forward routing.
- Remote-content preference persistence and allowlisting.
- External attachment open, archive extraction, and attachment preview types beyond plain text/raster images.
- Language picker, persisted locale preference, relative date wording, and broad translated-layout screenshot coverage.
- E2E accessibility, axe, or real screen-reader harness.
- Release-build Tauri-shell performance measurements against live adapters and persisted SQLite fixture databases.

## Documentation Sync

Keep this document synchronized with:

- `src/mail/shell/mailShellState.ts`
- `src/mail/shell/mailShellDataSource.ts`
- `src/mail/shell/MailClientShell.tsx`
- `src/mail/composer/TipTapComposer.tsx`
- `src/mail/composer/composerRichText.ts`
- `src/i18n/`
- `src-tauri/src/composer.rs`
- `src/App.test.tsx`
- `docs/architecture/message-rendering.md`
- `docs/architecture/account-signatures.md`
- `README.md`
- `AGENTS.md`
- `TODO.md`
