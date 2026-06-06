# TODO.md - Komorebi Wails Remake Readiness Ledger

Last refresh: 2026-06-06

## Current State

The active app is the Wails v3 + Go + SolidJS remake on `main`. The previous Tauri v2 + Rust + React implementation has been moved to `legacy/tauri-v2/` as a reference.

The remake is now buildable and has a small test baseline, but it is not production-complete. Treat README feature claims and future roadmap items as source only when they match the active Wails/Go/Solid code.

## Verified On 2026-06-06

- `npm install` in `frontend`
- `npm run check` in `frontend`
- `npm run build` in `frontend`
- `npm audit` in `frontend`
- `go test ./...`
- `go build ./...`
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/GenerateBindings.ps1`
- `wails3 build`
- Browser DOM smoke at 1280x720: title, shell, account/sidebar/list/reader regions, account setup panel, discovery fallback candidate, and no horizontal overflow
- Browser DOM smoke at 390x760: account setup panel fits viewport, reader pane hides at the narrow breakpoint, and no horizontal overflow

## Implemented In The Remake

- Wails v3 app bootstrap with a frameless main window.
- Go service boundary exposed to the Wails frontend.
- Pure-Go SQLite cache with embedded migrations.
- Local demo account/folders/messages/draft seeding for accountless shell development.
- SolidJS mail shell with account setup, diagnostics, language/density controls, sidebar, search, virtualized message list, safe reader, drafts drawer, and composer dialog.
- Local cache operations for shell load, paging, search, read-state mutation, sanitized message detail loading, draft save/list/delete, local sent-message creation, account list/add/remove, attachment staging, generic TLS IMAP sample sync, background sync start/stop/status, and account-specific SMTP send.
- Windows Credential Manager wrapper for generic passwords and OAuth token payloads.
- Provider policy enforcement: Gmail/Microsoft password setup is refused, generic IMAP/SMTP requires encrypted settings, SMTP uses account-specific settings, and encrypted IMAP supports password, OAUTHBEARER, and XOAUTH2 auth.
- Account autodiscovery uses Thunderbird-style autoconfig XML and RFC 6186 DNS SRV for custom domains. MX is diagnostic only; provider-specific MX templates are not used.
- OAuth PKCE start/complete flow for Gmail/Microsoft account creation, with localhost callback capture and manual authorization-code fallback.
- Generic account setup validates IMAP and SMTP authentication before credentials are stored.
- Incremental IMAP metadata sync stores UIDVALIDITY, last UID, and highest modseq in SQLite, with persisted background sync preferences and start/stop/status controls exposed to the sidebar.
- Strict Wails request objects and backend validation for provider, account, message, search, attachment, rendering, preference, sync, and send inputs.
- Safe HTML sanitization for stored drafts and rendered messages, with remote content stripped by default and explicit message/sender consent for safe remote images.
- Attachment staging, listing, safe text/image preview, fixed-folder save, and local cache deletion.
- Redacted diagnostics health/export surface and English/pt-PT UI strings.
- Windows signing and release artifact validation scripts.
- Wails binding generation under `frontend/bindings` through `scripts/GenerateBindings.ps1`, which works around a Windows rename failure in direct frontend output.
- Go tests for migrations/reopen, seed data, shell loading, search, read-state mutation, draft persistence, local sent-message creation, unsafe account rejection, OAuth PKCE and localhost callback completion, OAuth SASL fake-server auth, discovery, pre-save account validation, fake IMAP incremental sync, background sync, fake SMTP send, attachment workflow, diagnostics export, remote-content consent, and HTML sanitization.
- Real-provider validation is a manual QA path through the in-app account wizard with explicit test accounts only; automated tests use fake servers and must not use real mailbox data.
- Minimal Wails Taskfile so `wails3 build` produces `bin/Komorebi.exe`.

## Active Gaps

- Add OAuth SASL fake-server coverage.
- Run and record the documented real-provider manual smoke matrix in `docs/REAL_PROVIDER_SMOKE.md` with explicit test accounts entered through the account wizard.
- Add destructive server mail actions only after provider-specific semantics are reviewed.
- Finish provider-safe sent-copy strategy across providers.
- Add installer packaging and clean-machine validation for signed Wails output.
- Add frontend unit tests and E2E/browser screenshot harnesses for shell, composer, search, drafts, and responsive layout.
- Review whether generated `frontend/bindings` should be committed or regenerated in CI for every build.

## Deferred

- Block-based templates.
- Provider Drafts-folder sync.
- External attachment auto-open or archive extraction.
- Non-Windows targets.
- Cloud sync, analytics, AI features, or hosted services.
