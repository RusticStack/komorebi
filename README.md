# Komorebi

Privacy-first, AI-first, local-first Windows desktop email client built on Zed GPUI.

Komorebi is a native desktop mail client focused on self-hosted and custom-hosting mail (Mailcow, Zimbra, Poste.io, and any generic IMAP/SMTP server). Gmail and Microsoft 365 are supported through OAuth. AI features default to local models (Ollama, vLLM, Unsloth) and also work with any OpenAI-compatible API.

Komorebi is a single native GPUI + Rust application; the earlier Tauri v2 and Wails v3 prototypes have been removed.

## Current Stack

- Desktop shell: Zed GPUI (`gpui` 0.2.x) with `gpui-component` 0.5.x
- Backend: one Rust crate (`src/`), with `db`, `mail`, `ai`, `secrets`, and `ui` modules
- Local cache: SQLite through `rusqlite` (bundled), under the user's config directory
- Credentials: Windows Credential Manager through the `keyring` crate
- Mail: `imap`, `lettre`, `mail-parser`, `native-tls`; autoconfig XML + RFC 6186 DNS SRV discovery; OAuth PKCE
- AI: OpenAI-compatible chat client targeting Ollama, vLLM, Unsloth, and hosted APIs

## Quick Start

```powershell
cargo check --all-targets
cargo test
cargo build
```

`cargo build` writes `target\debug\komorebi.exe`. The first launch creates a local SQLite database under the user's config directory and seeds a local demo account so the shell can be exercised without real mailbox data.

## Verification

```powershell
cargo check --all-targets
cargo test
cargo build
cargo clippy --all-targets
```

Generated outputs such as `target/` are disposable and not committed.

## Implemented

- Native GPUI desktop shell with a sidebar, message list, reader, composer, account management, and an AI assistant panel.
- SQLite migrations for accounts, folders, messages, FTS search, drafts, attachments, settings, and AI fields/ledger.
- Seeded local demo account/messages for accountless development.
- Local SQLite operations for shell load, message paging, FTS search, read-state mutation, drafts, and local sent-message creation.
- Windows Credential Manager wrapper (`keyring`) for account passwords, OAuth tokens, and AI API keys.
- Provider policy enforcement: Gmail/Microsoft password setup is refused, generic IMAP/SMTP requires encrypted settings, and SMTP send loads account-specific settings.
- OAuth PKCE start/complete flow for Gmail/Microsoft account creation, with a localhost callback capture.
- Account autodiscovery for custom domains using Thunderbird-style autoconfig XML and RFC 6186 DNS SRV.
- Encrypted IMAP adapter (`imap` crate) with password login and XOAUTH2 SASL, folder listing with special-use attributes, incremental UID sync, and flag updates.
- SMTP sending via `lettre` with TLS/STARTTLS, password login, and XOAUTH2.
- MIME parsing via `mail-parser` for envelope, body, and attachments.
- AI-first features: OpenAI-compatible client, local-first provider presets (Ollama, vLLM, Unsloth), prompt templates (summarize, draft reply, polish, triage), and persistence of AI results into the local cache.
- Rust tests for migrations/reopen, seed data, folder/message flow, FTS search, drafts/attachments/settings, provider policy, OAuth PKCE and loopback callback, discovery, MIME parsing, plaintext-refusal for IMAP/SMTP, AI config/client/prompts, and AI action persistence. Mail protocol tests use fake servers and never real mailbox data.

## Important Limits

Komorebi is not production-complete yet.

- Live IMAP sync and SMTP send are encrypted and provider-safe in shape, but real-provider validation is a manual QA path with explicit test accounts only.
- Release signing requires a real code-signing certificate and Windows SDK `signtool.exe`; unsigned local validation is development-only.
- Destructive server mail actions, installer packaging, and clean-machine release validation are not implemented.
- The local mail cache is plaintext. Credentials must remain in Windows Credential Manager only.
- Do not test against real mailboxes or credentials unless explicitly authorized for that exact operation.

## Philosophy

- Local-first, zero telemetry, zero ads.
- AI-first, local-first: prefer models running on your machine.
- Correct mail semantics and data safety outrank UI polish.
- Drafts are local-only until send behavior is fully provider-safe.
- The first screen should remain the working mail client, not a landing page.
