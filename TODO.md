# TODO.md - Komorebi GPUI Remake Readiness Ledger

Last refresh: 2026-08-17

## Current State

The active app is the GPUI + Rust remake at the repository root (`Cargo.toml`, `src/`). The previous Wails v3 + Go + SolidJS implementation is reference-only under `legacy/wails-v3/`; the older Tauri v2 implementation is under `legacy/tauri-v2/`.

The remake builds, runs, and has a small test baseline, but it is not production-complete. Treat README feature claims and future roadmap items as source only when they match the active GPUI/Rust code.

## Verified On 2026-08-17

- `cargo check --all-targets`
- `cargo test` (42 passed, 1 ignored)
- `cargo build`

## Implemented In The GPUI Remake

- Native GPUI app bootstrap with a titled main window.
- Rust crate split into `db`, `mail`, `ai`, `secrets`, and `ui` modules.
- SQLite cache through `rusqlite` with embedded migrations (accounts, folders, messages + FTS, drafts, attachments, settings, AI fields and AI action ledger).
- Local demo account/folders/messages seeding for accountless shell development.
- GPUI mail shell with a folder sidebar, message list, reader, composer, account management, and an AI assistant panel.
- Local cache operations for shell load, paging, FTS search, read-state mutation, draft save/list/delete, local sent-message creation, and account list/add/remove.
- Windows Credential Manager wrapper (`keyring`) for account passwords, OAuth tokens, and AI API keys.
- Provider policy enforcement: Gmail/Microsoft password setup is refused, generic IMAP/SMTP requires encrypted settings, and SMTP uses account-specific settings.
- Standards-based account autodiscovery: Thunderbird-style autoconfig XML and RFC 6186 DNS SRV for custom domains.
- OAuth PKCE start/complete flow for Gmail/Microsoft, with a loopback callback capture.
- Encrypted IMAP adapter (`imap` crate): password and XOAUTH2 auth, folder listing with special-use attributes, incremental UID sync, and flag updates.
- SMTP sending via `lettre` with TLS/STARTTLS and password/XOAUTH2 auth.
- MIME parsing via `mail-parser`.
- AI-first module: OpenAI-compatible chat client, local-first provider presets (Ollama, vLLM, Unsloth), prompt templates (summarize, draft reply, polish, triage), and persistence of results into the cache.
- Rust tests for migrations/reopen, seed data, local mail behavior, provider policy, OAuth PKCE and loopback callback, discovery, MIME, plaintext-refusal for IMAP/SMTP, AI config/client/prompts, and AI action persistence. Mail protocol tests use fake servers and must not use real mailbox data.

## Active Gaps

- Real-provider validation: run a manual smoke matrix with explicit test accounts only (not automated).
- Add destructive server mail actions (move, delete, expunge, Sent-folder APPEND) only after provider-specific semantics are reviewed.
- Finish provider-safe sent-copy strategy across providers.
- Wire account autodiscovery and OAuth into the GPUI account setup flow end-to-end.
- Add safe HTML rendering for the reader (no remote content by default) and a remote-content consent path.
- Add attachment staging, preview, and safe save.
- Add diagnostics health/export with redaction.
- Add i18n (English + pt-PT was the previous baseline).
- Add installer packaging, code signing, and clean-machine validation.
- Add UI/E2E coverage (layout, composer, search, drafts, responsive) and screenshot verification.
- Add OAuth SASL fake-server coverage for IMAP and SMTP.
- Add local AI model health checks (e.g. an Ollama `/api/tags` reachability check) and streaming.

## Deferred

- Block-based templates.
- Provider Drafts-folder sync.
- External attachment auto-open or archive extraction.
- Non-Windows targets.
- Cloud sync, analytics, telemetry, or hosted services.
