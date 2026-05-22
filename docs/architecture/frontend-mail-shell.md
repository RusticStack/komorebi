# Frontend Mail Shell

Status: 3-pane shell, multi-account local-cache navigation, and keyboard/accessibility workflows implemented; live account setup and sync adapters are not registered.
Last reviewed: 2026-05-23.

## Current Scope

The code source of truth is:

- `src/mail/shell/mailShellState.ts`
- `src/mail/shell/mailShellDataSource.ts`
- `src/mail/shell/MailClientShell.tsx`
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

Tests may inject fixture mail through `initialData` to verify workflows. In Tauri runtime, `load_mail_shell` reads the local app-data SQLite cache and returns the current account/folder/message shell snapshot. In a plain browser/Vite runtime, the shell stays empty because no Tauri IPC bridge exists.

## State Model

`MailShellState` tracks:

- accounts
- folders
- messages
- account/folder/message loading status
- unified inbox mode
- selected account
- selected folder
- selected message
- search query

The navigation state is frontend-local only. Cache reads come from the Rust `load_mail_shell` command. The shell does not mutate SQLite, sync state, credentials, attachments, or provider state.

## Local Cache Read Model

`src-tauri/src/mail_shell.rs` owns the shell read model. The command:

- opens `mail-cache.sqlite3` under the Tauri app data directory
- applies Rust-owned migrations before reading
- excludes removed accounts
- returns redacted account metadata, folder counts, and a bounded recent-message snapshot
- maps folder unread counts from sync state into account/folder/unified unread badges
- never returns `credential_ref`, passwords, tokens, attachment storage paths, or raw SQL results

The command has no frontend-supplied arguments. It is exposed only through `load_mail_shell`, `src-tauri/build.rs`, and the `allow-load-mail-shell` capability permission.

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

Search filters the current account/folder scope by sender, sender email, subject, preview, and plain body text.

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

Message body rendering reuses `SafeEmailBody`. Do not add another untrusted HTML sink in the shell.

## Composer Entry

The current composer drawer is an entry-point shell only. It provides To, Subject, Message, Save draft, and Send controls for workflow placement, but it is not TipTap, it is not connected to draft persistence, and it does not call SMTP.

The full composer remains a separate P3 task.

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
- unified inbox visibility
- account navigation
- folder navigation
- selected-message preservation for compatible scope changes
- message search
- message selection and reader rendering
- keyboard search focus, list navigation, folder navigation, reader focus, dialog focus trapping, Escape close, and focus return
- composer drawer entry
- settings drawer entry
- loading/error shell states

Rust tests in `src-tauri/src/mail_shell.rs` cover reading accounts, folders, unread counts, recent messages, removed-account exclusion, and credential-reference redaction from disposable SQLite databases.

No test uses real mailbox data, real credentials, or live provider connections.

## Not Implemented

- Real account setup commands.
- TanStack Query integration.
- TipTap composer.
- Draft autosave command wiring.
- SMTP send command wiring.
- Virtualized large message list.
- i18n dictionaries.
- E2E accessibility, axe, or real screen-reader harness.

## Documentation Sync

Keep this document synchronized with:

- `src/mail/shell/mailShellState.ts`
- `src/mail/shell/MailClientShell.tsx`
- `src/App.test.tsx`
- `docs/architecture/message-rendering.md`
- `docs/architecture/account-signatures.md`
- `README.md`
- `AGENTS.md`
- `TODO.md`
