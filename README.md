# Komorebi

Privacy-first, local-first Windows desktop email client.

This branch is a Wails v3 + Go + SolidJS remake of the earlier Tauri v2 reference implementation. The old implementation is retained under `legacy/tauri-v2/` for reference while the active source lives at the repository root, `internal/`, and `frontend/`.

## Current Stack

- Desktop shell: Wails v3 alpha on Windows/WebView2
- Backend: Go, Wails services, `modernc.org/sqlite`, Windows Credential Manager through `wincred`
- Frontend: SolidJS, TypeScript, Vite, Tailwind CSS v4, Ark UI, TanStack Solid Router/Query/Virtual
- Local cache: SQLite database under the user's config directory

## Quick Start

```powershell
cd frontend
npm install
npm run check
npm run build
cd ..

wails3 generate bindings -ts
go test ./...
wails3 build
```

`wails3 build` writes `bin/Komorebi.exe`. The first cache load creates a local SQLite database under the user's config directory and seeds a local demo account so the shell can be exercised without real mailbox data.

## Verification

Current source checks:

```powershell
cd frontend
npm install
npm run check
npm run build
npm audit
cd ..

go test ./...
go build ./...
wails3 generate bindings -ts
wails3 build
```

Browser smoke checks should cover the Vite shell at `http://127.0.0.1:34115` for both desktop and narrow widths. Generated outputs such as `frontend/dist`, `frontend/node_modules`, `bin`, and `test-results` are disposable.

## Implemented

- Wails desktop bootstrap with a frameless Windows-style shell.
- SQLite migrations for accounts, folders, messages, FTS, drafts, attachments, and account server settings.
- Seeded local demo account/messages/draft for accountless development.
- SolidJS three-pane mail shell with sidebar, search, virtualized message list, reader, drafts drawer, and composer dialog.
- Local SQLite operations for shell load, message paging, FTS search, mark-read, draft save/list/delete, and local sent-message creation.
- Windows Credential Manager wrapper for account secrets.
- Basic IMAP and SMTP integration scaffolding.
- Wails TypeScript binding generation.
- Basic Go tests for migrations, seed data, and local mail service behavior.

## Important Limits

Komorebi is not production-complete yet.

- Live IMAP sync is still simplified and not validated against real providers.
- SMTP send has hard-coded/simplified provider behavior and needs provider-specific transport/auth handling before real use.
- OAuth, diagnostics export, i18n, safe rich email rendering, remote-content consent, destructive server mail actions, installer signing, and release validation are not implemented in the Wails remake.
- The local mail cache is plaintext. Credentials must remain in Windows Credential Manager only.
- Do not test against real mailboxes or credentials unless explicitly authorized for that exact operation.

## Philosophy

- Local-first, zero telemetry, zero ads.
- Correct mail semantics and data safety outrank UI polish.
- Drafts are local-only until send behavior is fully provider-safe.
- The first screen should remain the working mail client, not a landing page.
