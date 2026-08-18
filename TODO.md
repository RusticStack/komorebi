# TODO.md - Komorebi GPUI Readiness Ledger

Last refresh: 2026-08-18 (full `src/` audit against `Project Komorebi - Email Client Plan.md`)

## Current State

Komorebi is a single native GPUI + Rust desktop client at the repository root
(`Cargo.toml`, `src/`). The earlier Tauri v2 and Wails v3 prototypes have been
removed.

The app builds, runs, and has a working four-pane Japandi shell, but it is not
production-complete. This ledger records the gap between the product plan and
the code on disk, module by module.

## Verified On 2026-08-18

- `cargo check --all-targets` - clean
- `cargo test` - 47 passed, 1 ignored
- `cargo build` - clean
- `cargo fmt --check` - clean
- `cargo clippy --all-targets` - no warnings in `src/`

## Legend

- ✅ Implemented (with caveats where noted)
- ⚠️ Partial / placeholder / visual-only
- ❌ Missing

---

## Part 1 - Mail foundation

### 1.1 Account onboarding

- ❌ **Server settings are never collected or persisted through the UI.**
  `add_account` (`src/ui/mod.rs`) inserts an account with only email + display
  name and hardcodes `provider_kind = "custom"`, `auth_mode = "password"`. The
  IMAP/SMTP host/port/security columns exist in `db` but nothing writes them
  from the GUI, so a UI-added account can never sync or send.
- ❌ **Password/OAuth credentials are not collected in setup.** No secret is
  stored for a UI-added account, so the first sync/send fails with
  "no password stored".
- ❌ **Autodiscovery is not wired into onboarding.** `mail::discovery` (autoconfig
  XML + RFC 6186 SRV) is implemented and tested but never called from `ui`.
- ❌ **OAuth is not wired into onboarding.** `mail::oauth` (PKCE + loopback +
  refresh) is implemented and tested but there is no Gmail/Microsoft setup path
  in the UI, and `add_account` never picks an OAuth provider.
- ❌ **No pause/reorder/resync/identity/signature UI** for accounts.

### 1.2 Sync engine

- ⚠️ **IMAP sync exists but is not reachable from the UI.** `mail::sync_account`
  / `sync_folder` do incremental UID fetch, folder discovery, and MIME caching,
  but `src/ui/mod.rs` never calls them (no sync button, no background schedule,
  no first-run sync after account creation).
- ❌ **No visible sync state machine.** The plan requires synced / syncing /
  offline / error status; today only a `SyncSummary` struct exists with no UI.
- ❌ **No background scheduler or offline queue.** Archive/delete/star/move are
  cache-only (`db`) and are never reconciled to the server; conflicts are not
  surfaced.
- ❌ **No JMAP, Gmail API, or Microsoft Graph connectors.** Only IMAP/SMTP
  (`src/mail/imap.rs`, `smtp.rs`). The plan lists these as optional connectors.
- ✅ Encrypted-only posture (TLS/STARTTLS, plaintext refused), XOAUTH2 + password
  auth, folder special-use detection, and UID dedupe are implemented.

### 1.3 Local storage & model

- ❌ **No threads/conversations.** There is no `threads` table and no grouping on
  `rfc_message_id` / `in_reply_to` / `references`. The reader opens one message,
  not a conversation.
- ❌ **No labels table.** Labels are faked by writing a single free-text string
  into `messages.ai_category`. There is no label create/rename/delete/apply
  model, and the sidebar's Work/Design/Personal/Finance/Travel are hardcoded
  rather than data-driven.
- ❌ **No reminders/snooze table.** The Snoozed sidebar item is a cosmetic view;
  `snooze_selected` only marks read and toasts "Snoozed until tomorrow · undo"
  with no stored due date and no undo.
- ❌ **No rules/automation table.** The plan's deterministic rules layer is absent.
- ❌ **No contacts/participants model.** Sender is stored as name/email strings;
  there is no participant entity, avatar source, or contact intelligence.
- ✅ Accounts, folders, messages + FTS5, drafts, attachments (with blobs), settings,
  and an AI action ledger exist with foreign-key cascades.

### 1.4 Reading a conversation

- ⚠️ **Attachment cards render but are inert.** Download, open, and preview are
  not wired - `db::get_attachment_data` exists but no UI handler writes the blob
  to disk or renders a preview. The download/drive icons are visual only.
- ❌ **No safe HTML rendering or remote-content consent.** The reader renders
  plain text line-by-line (`render_body`); `body_html` is cached but unused, and
  there is no sanitizer or remote-content gate.
- ❌ **No thread collapse / per-message expand.** Blocked on threads (1.3).
- ✅ Reader toolbar (back/archive/delete/mark-unread/snooze/star/more), subject
  + tag pills, sender header, quick-action pills, and per-message summary are
  present.

### 1.5 Composer

- ✅ Compose, reply, reply-all, forward, attachments (drag-drop → SMTP multipart
  → sent-copy persistence), and inline AI-drafted-reply insertion.
- ❌ **Rich text.** Composer is plain-text only; the formatting bar buttons are
  no-ops (`format_button`). `body_html` is supported in SMTP but never produced.
- ❌ **Draft autosave.** `db::save_draft`/`list_drafts` exist but the composer
  never autosaves; the Drafts folder is only demo-seeded.
- ❌ **Signatures.** `accounts.signature_html` column exists with no UI or
  insertion into composed mail.
- ❌ **Scheduled send, undo send, send-and-archive.** None implemented.
- ⚠️ **Internal note** tab renders but has no persistence or handler.
- ❌ **Inline images / links** beyond pasted text.

### 1.6 Search & command navigation

- ⚠️ **Text search only.** FTS5 keyword search works, but the plan's structured
  filters (`from:`, `label:`, `has:attachment`, `is:unread`, `after:`, `before:`)
  and natural-language-to-filter translation are not implemented.
- ❌ **Command palette.** `Ctrl+K` focuses the search box; there is no palette.
- ⚠️ **Shortcuts.** Compose, search, sidebar, theme, j/k, archive, star, reply,
  and forward are bound. Missing from the plan: reply-all (`A`), snooze, undo
  (`Z`), Esc-to-close, and any customization/discoverability UI. Shortcuts only
  fire when a non-input element has focus (standard GPUI behavior).

### 1.7 Offline behavior

- ✅ Read cached mail, search locally, compose locally, and organize locally.
- ❌ **Offline queue + reconciliation** for archive/label/snooze/delete (see 1.2).
- ✅ AI summaries persist on-device in `messages.ai_summary`.

### 1.8 Notifications

- ❌ **Desktop notifications are entirely absent.** No tray icon, no notify
  crate, no per-account/mailbox/sender/priority/quiet-hours controls.

---

## Part 2 - Contextual intelligence

### 2.1 Capabilities (`src/ai/`)

| Plan capability      | Status | Notes                                                                 |
| -------------------- | ------ | --------------------------------------------------------------------- |
| Summarize thread     | ⚠️     | Works per **message**, not per conversation; stores to `ai_summary`.   |
| Draft a reply        | ✅     | Editable, never auto-sent; inserted into composer.                    |
| Improve writing      | ⚠️     | `polish` operates on the selected message body, not the composer draft or a selected text range. |
| Adjust tone          | ⚠️     | Tone is passed as a prompt string, but the UI tone footer is cosmetic - `perform_ai` hardcodes "friendly and concise" / "clear and professional". |
| Detect action items  | ⚠️     | Local bullet-scan heuristic (`detect_action_items`), not model-based; no task/owner/date extraction. |
| Categorize email     | ✅     | `triage` + `apply_triage` writes one `ai_category`. No multi-label or accept/correct flow. |
| Schedule follow-up   | ❌     | Clicking the card only shows a "on the roadmap" toast.                |
| Explain content      | ❌     | Not implemented.                                                       |
| Search naturally     | ❌     | No natural-language → filter translation.                             |

- ❌ **No streaming.** `ai::client::complete` is blocking + non-streaming
  (fine for now, but the plan's responsiveness goal will want SSE streaming).
- ❌ **No local-model health check.** No Ollama `/api/tags` reachability probe,
  no model-availability check, no vLLM/Unsloth connection test.

### 2.2 Insights

- ⚠️ **AI Insights block is mostly hardcoded.** Topic uses `ai_category`, but key
  people avatars, "2h 15m" last-response, and "90%" response rate are static
  sample values, not computed. Unanswered-questions, commitments, related
  conversations, suggested label, and response patterns are not computed.

### 2.3 Safety rules

- ✅ Never sends without a deliberate user action; AI output is always an
  editable draft.
- ✅ Does not delete/archive/label/move mail (only writes `ai_category`/`ai_summary`).
- ✅ Uses the smallest context (single message body) and excludes attachments.
- ✅ Original messages always remain visible; summaries are additive.
- ⚠️ **No uncertainty instruction** in prompts ("identify uncertainty instead of
  inventing facts" is not yet encoded).
- ⚠️ **AI action ledger has no UI.** `ai_actions` is written and can be listed,
  but the user cannot view or delete cached AI artifacts.
- ❌ **No global / per-account / per-conversation AI disable.** AI config exists
  but there is no enable toggle, and no opt-in during account setup.

---

## Part 3 - Advanced workflow

- ❌ Rules & automations (deterministic conditions → label/folder/priority/notify).
- ❌ Scheduled send, send-later, undo-send.
- ❌ Saved searches.
- ❌ Contact intelligence / calendar-aware suggestions.
- ❌ Multiple identities & per-identity signatures.
- ❌ Command palette expansion.
- ❌ Shared inboxes, assignments, internal notes, collision detection, team
  policies, plugin/extension API (Phase 4).

---

## Part 4 - Privacy & security

- ✅ Credentials (passwords, OAuth tokens, AI keys) live only in the OS
  credential store via `keyring` (`src/secrets.rs`); never in SQLite/logs/UI state.
- ✅ Zero telemetry/ads; no telemetry code exists.
- ✅ Account removal cascades local data (`PRAGMA foreign_keys = ON`).
- ❌ **Local cache is plaintext** (acknowledged in README/AGENTS). Encryption at
  rest (SQLCipher or OS DPAPI on the file) is not implemented.
- ❌ **No AI opt-in during account setup**, no redaction controls, no retention
  policy, no "view and delete AI artifacts" UI.
- ⚠️ **"No training by default"** is not enforced in the request - relies on the
  provider's contract.
- ❌ **Enterprise policy controls** (disable external AI, restrict attachment
  processing, retention limits, audit events) are not implemented.

---

## Part 5 - Settings & accessibility

- ⚠️ **Appearance**: dark/light toggle only. No density, text size, panel
  visibility, or reduced-motion settings.
- ⚠️ **AI settings**: model + base URL editable; provider picker is not wired to
  presets, and no tone/context/retention controls.
- ❌ **Composer settings** (default format, signature, undo-send duration, reply
  behavior), **Notifications**, **Shortcuts customization**, and **Rules** pages.
- ❌ **i18n** (no localization; previous pt-PT baseline not carried over).
- ❌ **Diagnostics health/export with redaction.**
- ⚠️ **Accessibility**: icon tooltips and keyboard shortcuts exist, but there is
  no focus-visible audit, no screen-reader pass, no font scaling, no reduced
  motion, and no WCAG 2.2 AA contrast verification.
- ❌ **Installer packaging, code signing, clean-machine validation.**

---

## Prioritized next-up

### P0 - make a real account usable end-to-end

1. Wire onboarding: collect server settings (or run autodiscovery), collect and
   store the password/OAuth token, then run `mail::sync_account` off the UI
   thread (`src/ui/mod.rs` + `src/mail/discovery.rs` + `src/mail/oauth.rs`).
2. Add a sync trigger + visible sync state (syncing/offline/error) in the
   titlebar or sidebar, with background sync on launch.
3. Surface sync/send/auth errors as retryable UI states instead of dropping them.

### P0 - complete the reader

4. Safe HTML rendering with sanitization + remote-content consent (`src/mail/mime.rs`
   already has `strip_html`; add a renderer + consent gate).
5. Wire attachment download/open (and safe preview for images/PDF) from
   `db::get_attachment_data`.

### P1 - organization & offline

6. Add a real `labels` table and label CRUD/apply, replacing the `ai_category`
   hack for user labels.
7. Add a `reminders`/`snooze` model so Snoozed actually returns mail when due,
   plus an offline mutation queue that reconciles to the server on reconnect.
8. Add conversation/thread grouping (`in_reply_to`/`references` or provider
   thread ids) and thread-level summarize.

### P1 - AI depth

9. Feed the tone selector into `draft_reply`/`polish` and persist the tone
   preference.
10. Replace the heuristic action-item scan with a model call that extracts
    task/owner/date and creates a confirmed reminder.
11. Add streaming responses and an Ollama `/api/tags` health check; expose
    view/delete for cached AI artifacts; add global/per-account AI disable.

### P2 - workflow & polish

12. Draft autosave + signatures; scheduled/undo send.
13. Structured search filters + natural-language search; command palette.
14. Desktop notifications (account/mailbox/sender rules, quiet hours).
15. Accessibility pass (focus, contrast, font scaling, reduced motion), i18n,
    diagnostics export, installer signing.
