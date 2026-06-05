# TODO.md - Komorebi Wails Remake Readiness Ledger

Last refresh: 2026-06-05

## Current State

The active app is the Wails v3 + Go + SolidJS remake on `remake/wails-solidjs-ark`. The previous Tauri v2 + Rust + React implementation has been moved to `legacy/tauri-v2/` as a reference.

The remake is now buildable and has a small test baseline, but it is not production-complete. Treat README feature claims and future roadmap items as source only when they match the active Wails/Go/Solid code.

## Verified On 2026-06-05

- `npm install` in `frontend`
- `npm run check` in `frontend`
- `npm run build` in `frontend`
- `npm audit` in `frontend`
- `go test ./...`
- `go build ./...`
- `wails3 generate bindings -ts`
- `wails3 build`
- Browser DOM smoke at desktop width: title, shell, drafts, search input, and no horizontal overflow
- Browser DOM smoke at 390px width: title, shell, drafts, search input, and no horizontal overflow

Screenshot capture through the in-app browser timed out during the 2026-06-05 check, so the current responsive validation is DOM/overflow based rather than screenshot-artifact based.

## Implemented In The Remake

- Wails v3 app bootstrap with a frameless main window.
- Go service boundary exposed to the Wails frontend.
- Pure-Go SQLite cache with embedded migrations.
- Local demo account/folders/messages/draft seeding for accountless shell development.
- SolidJS mail shell with sidebar, search, virtualized message list, reader, drafts drawer, and composer dialog.
- Local cache operations for shell load, paging, search, read-state mutation, draft save/list/delete, local sent-message creation, account list/add/remove, attachment staging, basic IMAP sample sync, and basic SMTP send scaffolding.
- Windows Credential Manager wrapper for secret storage.
- Wails binding generation under `frontend/bindings`.
- Minimal Go tests for migrations, seed data, shell loading, search, read-state mutation, draft persistence, and local sent-message creation.
- Minimal Wails Taskfile so `wails3 build` produces `bin/Komorebi.exe`.

## Active Gaps

- Replace demo/hard-coded IMAP and SMTP behavior with provider/account-specific configuration.
- Add real provider-safe OAuth and XOAUTH2 flows for Gmail and Microsoft before enabling those providers.
- Validate generic IMAP/SMTP against local fake servers before any real-account smoke.
- Add strict IPC/input validation around every Wails service method.
- Add safe HTML email rendering, remote-content blocking/consent, and safe external-link handling.
- Add attachment preview/save/download validation before exposing attachment actions broadly.
- Replace local-only `SendDraft` behavior with a provider-safe send pipeline and sent-copy strategy.
- Add i18n back if English/pt-PT remains a product requirement.
- Add diagnostics/logging/export with redaction.
- Add installer/signing/release verification for Wails output.
- Add frontend unit tests and E2E/browser screenshot harnesses for shell, composer, search, drafts, and responsive layout.
- Review whether generated `frontend/bindings` should be committed or regenerated in CI for every build.

## Deferred

- Block-based templates.
- Provider Drafts-folder sync.
- External attachment auto-open or archive extraction.
- Non-Windows targets.
- Cloud sync, analytics, AI features, or hosted services.
