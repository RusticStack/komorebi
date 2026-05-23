# Frontend Mail Shell

Status: 3-pane shell, multi-account local-cache navigation, FTS-backed search UX, keyboard/accessibility workflows, virtualized message rendering, safe message detail rendering, English/pt-PT i18n, TipTap composer with tested rich formatting, and local recent-recipient autocomplete implemented; live account setup and sync adapters are not registered.
Last reviewed: 2026-05-23.

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
- `src-tauri/src/mail_shell.rs`

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

Tests may inject fixture mail through `initialData` to verify workflows. In Tauri runtime, `load_mail_shell` reads the local app-data SQLite cache and returns the current account/folder/message shell snapshot; `search_mail` queries the same cache through FTS5 when the user searches; composer data-source functions call the typed Rust draft, attachment staging, recent-recipient, and send commands. In a plain browser/Vite runtime, the shell stays empty because no Tauri IPC bridge exists.

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

The navigation state is frontend-local. Cache reads come from the Rust `load_mail_shell` and `search_mail` commands. Composer draft/save/send actions are the only current frontend-triggered mutations and go through typed Rust commands; the shell still does not expose frontend SQL, credentials, arbitrary paths, or provider command strings.

## Local Cache Read Model

`src-tauri/src/mail_shell.rs` owns the shell read model. The command:

- opens `mail-cache.sqlite3` under the Tauri app data directory
- applies Rust-owned migrations before reading
- excludes removed accounts
- returns redacted account metadata, folder counts, and a bounded recent-message snapshot
- returns only display-safe attachment metadata for snapshot messages: id, sanitized display filename, MIME type, byte size, and disposition
- maps folder unread counts from sync state into account/folder/unified unread badges
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

The shell must remain usable from the keyboard before live account setup exists:

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
- inert attachment metadata list
- disabled reply, forward, archive, mark-read, and remote-content controls until typed backend commands exist

Message body rendering reuses `SafeEmailBody`. Do not add another untrusted HTML sink in the shell.

Attachment rows are metadata only. The frontend receives display filenames, MIME type, size, and disposition; it does not receive `storage_relative_path`, local filesystem paths, sender filenames as storage hints, hashes, or preview URLs. Preview and save buttons are intentionally disabled until a future typed attachment command validates user intent, ownership, path, size, and content type.

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
- Discard action
- Send action with visible progress states returned by the Rust send boundary

The Tauri runtime wiring is:

- `autosave_composer_draft` for local draft rows
- `discard_composer_draft` for discard and staged attachment cleanup
- `stage_composer_attachment` for backend-generated app-data storage and display-safe metadata
- `send_composer_email` for draft validation, SMTP send, confirmed send persistence, and staged attachment cleanup after success
- `list_recent_recipients` for account-scoped local autocomplete suggestions from SQLite

The frontend never receives attachment storage paths and never sends local filesystem paths to Rust. It sends user-selected bytes from the WebView file input to the staging command. The backend validates MIME/type/size and owns the final app-data path.

Rich composer HTML is centralized in `src/mail/composer/composerRichText.ts`. The TipTap extension list and sanitizer allow only the supported strikethrough/table/code/span/mark/link/image subset, restrict inline styles to whitelisted `text-align`, `color`, `background-color`, `font-size`, and `font-family` values, normalize table span attributes, rewrite staged inline image data URLs to CIDs, and remove unsafe links, event handlers, scripts, remote image URLs, and unsupported CSS before autosave or send.

Recent-recipient autocomplete is local-only. Confirmed sends update the `recent_recipients` table in the local cache after SMTP acceptance; suggestions expose only display name, email address, use count, and last-used timestamp. Duplicate recipients are removed with first occurrence winning in To, then CC, then BCC order, and the composer shows a non-error notice while preserving malformed-address validation errors.

Current composer limits:

- Draft reload after app restart is backend-supported but not surfaced in the drawer yet.
- Provider Drafts-folder upload is not implemented.
- Reply/forward routing into the composer is not implemented.
- Sent-folder APPEND for providers that need it remains future work.
- Live SMTP success depends on real account setup and credentials, which are not implemented in the UI yet.

## Settings Entry

The Settings drawer uses `AccountSignatureSettings` for signature editing. With no accounts it shows the accountless settings state. With injected account data it can edit the local in-memory account signature model.

Persisting settings to SQLite through Tauri commands remains future IPC/account setup work.

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
- safe message detail rendering with sanitized HTML, remote image blocking, inert attachment metadata, and disabled reply/forward/remote-content controls
- local fixture search and FTS-backed search with debounce, cancellation, result origin labels, no-match state, no-index state, and retryable error state
- TipTap composer address validation, duplicate recipient handling, keyboard autocomplete from recent recipients, signature insertion, draft save, attachment staging, inline image CID conversion, send command request shape, send progress, and validation errors
- TipTap rich formatting extension output and sanitizer behavior in `src/mail/composer/TipTapComposer.test.tsx`
- message selection and reader rendering
- keyboard search focus, list navigation, folder navigation, reader focus, dialog focus trapping, Escape close, and focus return
- 10,000-message virtualized rendering with bounded DOM row count and stable row state during scroll
- composer drawer entry
- settings drawer entry
- loading/error shell states
- first-launch language detection, English fallback, `pt-PT` resource completeness, locale-aware date/time/count/size/status formatting, localized missing sender/attachment fallbacks, and first-run `pt-PT` shell rendering in `src/i18n/i18n.test.tsx`

`src/mail/shell/mailShellState.test.ts` covers pure shell state rules: unified/account/folder scope filtering, local search fallback behavior, cache-reload selection preservation, and stale backend row clearing.

`src/mail/shell/mailShellDataSource.test.ts` covers the frontend backend boundary: Tauri runtime detection, typed `invoke` command payloads for shell load/search, recent-recipient lookup and composer autosave, and cancellation before invoke or after a stale response.

`e2e/mail-shell.smoke.spec.ts` covers a browser-level Playwright smoke path against the test-only `/e2e/` Vite entry. It loads fixture mail through `src/e2e/main.tsx`, opens inbox and archive messages, runs backend-shaped search, opens a search result in the reader, opens the composer, sends through typed fixture handlers, and retains screenshots, video, and traces on failure through `playwright.config.ts`.

Rust tests in `src-tauri/src/mail_shell.rs` cover reading accounts, folders, unread counts, recent messages, safe attachment metadata projection, missing-label fallback ownership, removed-account exclusion, credential-reference redaction, attachment storage-path redaction, FTS search success, account/folder scoping, validation denial paths, and missing-index reporting from disposable SQLite databases.

Rust composer and SMTP tests cover sanitized rich composer HTML persistence and outgoing MIME serialization for strikethrough, tables, code blocks, font styles, colors, highlights, and alignment.

No test uses real mailbox data, real credentials, or live provider connections.

## Not Implemented

- Real account setup commands.
- TanStack Query integration.
- Live reply/forward routing.
- Remote-content preference persistence and allowlisting.
- Attachment preview/save commands.
- Language picker, persisted locale preference, relative date wording, and broad translated-layout screenshot coverage.
- E2E accessibility, axe, or real screen-reader harness.

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
