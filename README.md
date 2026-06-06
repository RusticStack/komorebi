# Komorebi

Privacy-first, local-first Windows desktop email client.

This branch is a Wails v3 + Go + SolidJS remake of the earlier Tauri v2 reference implementation. The old implementation is retained under `legacy/tauri-v2/` for reference while the active source lives at the repository root, `internal/`, and `frontend/`.

## Current Stack

- Desktop shell: Wails v3 alpha on Windows/WebView2
- Backend: Go, Wails services, `modernc.org/sqlite`, Windows Credential Manager through `wincred`
- Mail protocols: `github.com/emersion/go-imap/v2` + `github.com/emersion/go-sasl` for encrypted IMAP, `github.com/wneessen/go-mail` for SMTP
- Frontend: SolidJS, TypeScript, Vite, Tailwind CSS v4, Ark UI, TanStack Solid Router/Query/Virtual
- Local cache: SQLite database under the user's config directory

## Quick Start

```powershell
cd frontend
npm install
npm run check
npm run build
cd ..

powershell -NoProfile -ExecutionPolicy Bypass -File scripts/GenerateBindings.ps1
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
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/GenerateBindings.ps1
wails3 build
```

Browser smoke checks should cover the Vite shell at the active local dev URL for desktop and narrow widths when viewport control is available. Generated outputs such as `frontend/dist`, `frontend/node_modules`, `bin`, and `test-results` are disposable.

## Implemented

- Wails desktop bootstrap with a frameless Windows-style shell.
- SQLite migrations for accounts, folders, messages, FTS, drafts, attachments, and account server settings.
- Seeded local demo account/messages/draft for accountless development.
- SolidJS mail shell with account setup, diagnostics, language/density controls, sidebar, search, virtualized message list, safe reader, drafts drawer, and composer dialog.
- Local SQLite operations for shell load, message paging, FTS search, mark-read, draft save/list/delete, sanitized body loading, and local sent-message creation.
- Windows Credential Manager wrapper for generic account passwords and OAuth token payloads.
- Provider policy enforcement: Gmail/Microsoft password setup is refused, generic IMAP/SMTP requires encrypted settings, and SMTP send loads account-specific settings.
- OAuth PKCE start/complete flow for Gmail/Microsoft account creation, localhost callback capture with manual fallback, token exchange isolated in Go, and token storage outside SQLite.
- Account autodiscovery for custom domains using Thunderbird-style autoconfig XML and RFC 6186 DNS SRV. MX records are diagnostic only; provider-specific MX templates are not used.
- Account setup validates IMAP and SMTP authentication before storing a generic account password in Windows Credential Manager.
- Encrypted IMAP adapter based on `emersion/go-imap/v2`, with password auth plus OAUTHBEARER/XOAUTH2 SASL for OAuth providers, persisted UID sync state, and an in-memory background sync start/stop loop.
- Strict Wails request objects and backend validation for accounts, search, message IDs, attachments, rendering, preferences, and sync/send requests.
- Safe HTML sanitization for stored drafts and rendered messages; remote content tags are stripped before WebView rendering.
- Remote-content consent supports message-level and sender-level image loading, with links opened through a validated backend OS-browser handoff.
- Attachment workflow supports safe local staging, listing, small text/image preview, fixed-folder save, and local cache deletion.
- Redacted diagnostics health surface and English/pt-PT UI strings.
- Redacted diagnostics export bundles under the app data directory.
- Release scripts for Windows signing and release artifact validation.
- Wails TypeScript binding generation.
- Go tests for migrations/reopen, seed data, local mail behavior, unsafe account rejection, OAuth PKCE and localhost callback completion, OAuth SASL fake-server auth, standards-based account discovery, pre-save account validation, fake IMAP incremental sync, background sync, fake SMTP send, attachment workflow, diagnostics export, remote-content consent, and HTML sanitization.

## Important Limits

Komorebi is not production-complete yet.

- Live IMAP sync is encrypted, supports password/OAuth SASL, persists auto-sync preferences, and syncs provider-advertised special folders conservatively. Full server mutation semantics are still intentionally limited.
- Real-provider validation should be done through the in-app account wizard with explicit test accounts only; see `docs/REAL_PROVIDER_SMOKE.md`. Automated tests use fake IMAP/SMTP servers and must not use real mailbox data.
- Release signing requires a real code-signing certificate and Windows SDK `signtool.exe`; unsigned local validation is development-only.
- Destructive server mail actions, installer packaging, and clean-machine release validation with signed artifacts still require manual release work.
- The local mail cache is plaintext. Credentials must remain in Windows Credential Manager only.
- Do not test against real mailboxes or credentials unless explicitly authorized for that exact operation.

## Philosophy

- Local-first, zero telemetry, zero ads.
- Correct mail semantics and data safety outrank UI polish.
- Drafts are local-only until send behavior is fully provider-safe.
- The first screen should remain the working mail client, not a landing page.
