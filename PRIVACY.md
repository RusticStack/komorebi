# Komorebi Privacy Statement

Last updated: 2026-08-17

Komorebi is a local-first, zero-telemetry desktop email client. It does not collect analytics, run ads, upload crash reports, send remote diagnostics, sell data, or sync mailbox content through a Komorebi service. This statement describes the current implementation. It is not a claim that cached mail is encrypted at rest.

## What Stays Local

Komorebi stores its application data on the local Windows device under the user's config directory (`komorebi/cache.db`). The local cache currently includes:

- Account metadata: account id, email, display name, provider kind, auth mode, username, and IMAP/SMTP host/port/security metadata.
- Folder metadata, special-use attributes, unread/total counts, and sync state.
- Cached message metadata: subject, sender name/email, recipients, received date, read/flagged state, snippet, plain-text body, HTML body, and search index terms.
- Attachment metadata and attachment bytes for messages that have been synced.
- Local drafts.
- Local settings, including the AI provider configuration (provider id, base URL, model name).
- A local ledger of AI actions and per-message AI category/summary results.

The local mail cache is unencrypted in the current implementation. Cached message bodies, subjects, FTS/search terms, drafts, attachments, and AI summaries are stored as plaintext local data. Account secrets and AI API keys are handled separately.

## Secrets

Komorebi stores secrets only in Windows Credential Manager through the `keyring` crate:

- Generic provider passwords or app passwords.
- OAuth refresh tokens (and transient access tokens) for Gmail and Microsoft.
- AI API keys for hosted AI providers.

Komorebi must not store passwords, app passwords, refresh tokens, access tokens, OAuth authorization codes, authorization headers, AI API keys, or credential blobs in SQLite, logs, diagnostics exports, screenshots, or UI state.

## AI Features

AI features are opt-in and default to local models running on your machine.

- **Local providers (Ollama, vLLM, Unsloth):** when a local provider is configured, the selected message text (sender, subject, and body, or a draft) is sent only to the local model endpoint on your machine. Nothing is transmitted to a third party.
- **Hosted OpenAI-compatible APIs:** when you configure a hosted endpoint, the prompt (which includes the message text you choose to act on) is sent to that endpoint. Komorebi sends only the minimum text required for the selected action (summarize, draft reply, polish, or triage), and only after you invoke it.
- AI API keys are stored in Windows Credential Manager and are never written to SQLite or logs.
- AI results are cached locally alongside the message.

## Network Calls

Komorebi makes network calls only for the user-configured mail functionality or explicit user actions:

- IMAP sync for configured accounts (TLS/STARTTLS password or XOAUTH2).
- SMTP send for configured accounts.
- OAuth authorization, token exchange, and token refresh for Gmail/Microsoft.
- Autodiscovery requests to `autoconfig.<domain>` and DNS SRV lookups when adding an account.
- AI requests to the configured AI endpoint when the user runs an AI action.

Komorebi does not make background network calls for analytics, telemetry, advertising, remote logging, product tracking, or diagnostics upload.

Provider servers still receive the information needed to operate email. Those provider interactions are governed by the user's mail provider. AI endpoint operators receive the message text the user explicitly submits for an AI action.

## Remote Content And Links

Remote email content is not loaded by default. The current GPUI reader renders cached plain text only and never loads remote images or other remote resources. Links are shown as text. Safe external link handling and remote-content consent are future work.

Attachments are never auto-opened. In the current scaffold, attachments are cached but there is no preview/save/download UI yet; safe attachment handling is future work.

## Account Removal And Cache Removal

Account removal is backend-owned. Secrets (passwords, OAuth tokens, and the account's AI key reference) are handled through the OS credential store and should be deleted before account metadata/cache cleanup. Local cache rows cascade with the account row.

Provider-side revocation is not implemented yet. Local credential deletion and local cleanup are the current behavior.

## What Komorebi Never Collects

Komorebi does not collect or transmit to a Komorebi-controlled service:

- Analytics events, advertising identifiers, usage telemetry, or product tracking.
- Crash reports, remote logs, or diagnostics bundles.
- Mailbox contents, message bodies, subjects, snippets, attachments, or address books.
- Credentials, tokens, OAuth codes, authorization headers, or AI API keys.

Any future feature that uploads diagnostics, telemetry, crash reports, or mailbox data would require a separate explicit design, user-facing disclosure, and opt-in consent. That feature does not exist today.

## Current Limitations

- The local mail cache is plaintext and unencrypted.
- Backup/export/import is unsupported.
- Diagnostics health/export, safe HTML rendering, remote-content consent, and full attachment safety are not yet restored in the GPUI remake.
- Release packaging and provider setup documentation still need final production verification.
- Opt-in live-provider smoke tests are not part of normal automated tests.

## Related Documentation

- [README.md](README.md)
