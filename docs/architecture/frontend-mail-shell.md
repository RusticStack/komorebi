# Frontend Mail Shell

Status: state-driven 3-pane shell implemented; live account/query IPC is not registered.
Last reviewed: 2026-05-22.

## Current Scope

The code source of truth is:

- `src/mail/shell/mailShellState.ts`
- `src/mail/shell/MailClientShell.tsx`
- `src/App.tsx`
- `src/App.css`

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

Tests may inject fixture mail through `initialData` to verify workflows, but runtime defaults stay empty until real account setup/query commands exist.

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

The state model is frontend-local only. It does not mutate SQLite, sync state, credentials, attachments, or provider state.

## Navigation Behavior

The unified inbox toggle controls whether the list is scoped to all accounts or a selected account.

In unified mode:

- the virtual `Unified Inbox` folder shows inbox messages across accounts
- account buttons remain visible but selecting an account switches into account-scoped mode

In account mode:

- account selection scopes folders and messages to that account
- folder selection scopes messages to that folder
- selecting a different account or folder clears the selected message

Search filters the current account/folder scope by sender, sender email, subject, preview, and plain body text.

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

These states are UI contract states for future Rust command/query wiring.

## Tests

`src/App.test.tsx` covers:

- accountless default render without fake mailbox data
- unified inbox visibility
- account navigation
- folder navigation
- message search
- message selection and reader rendering
- composer drawer entry
- settings drawer entry
- loading/error shell states

No test uses real mailbox data, real credentials, live IPC, or SQLite.

## Not Implemented

- Real account setup/query commands.
- TanStack Query integration.
- Backend read models for accounts/folders/messages.
- TipTap composer.
- Draft autosave command wiring.
- SMTP send command wiring.
- Virtualized large message list.
- i18n dictionaries.

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
