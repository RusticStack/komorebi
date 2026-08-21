# TODO.md - Komorebi GPUI Readiness Ledger

Last refresh: 2026-08-21 (UI modularization + composer/onboarding feature push)

## Current State

Komorebi is a single native GPUI + Rust desktop client at the repository root
(`Cargo.toml`, `src/`). The app builds, runs, and has a working four-pane
Japandi shell with onboarding, sync, OAuth, autodiscovery, attachment handling,
threading, snooze/reminders, AI tone, safe HTML rendering with remote-content
consent, AI health check, model-based action items, desktop notifications, i18n
with a locale picker, user labels, and an offline mutation outbox - all wired
into the UI. The former 5,500-line `src/ui/mod.rs` god file is now split into
focused submodules (`types`, `sidebar`, `message_list`, `reader`, `ai_panel`,
`composer`, `accounts`, `theme`, `helpers`). First-run onboarding routes new
users to account setup (demo mailbox is opt-in), and the composer gained
CC/BCC fields, per-account signatures, and debounced draft autosave with
restore-on-failure. It is not yet production-complete.

## Verified On 2026-08-21

- `cargo check --all-targets` - clean
- `cargo test` - 140 passed, 1 ignored
- `cargo build` - clean
- `cargo fmt --check` - clean
- `cargo clippy --all-targets` - no warnings in `src/` (only a transitive
  `imap-proto` future-incompat notice from the `imap` crate)

## Legend

- ✅ Implemented (with caveats where noted)
- ⚠️ Partial / placeholder / visual-only
- ❌ Missing

---

## Part 1 - Mail foundation

### 1.1 Account onboarding
- ✅ Server settings + password + OAuth + autodiscovery all wired off-thread.
- ✅ **First-run onboarding**: zero accounts lands on a welcome/setup panel;
  the demo mailbox is now opt-in ("Explore with demo data").
- ⚠️ No pause/reorder/resync/identity UI.

### 1.2 Sync engine
- ✅ IMAP sync reachable from UI (`sync_now` → `sync_account` off-thread).
- ⚠️ Sync state visible but minimal (string status; no full state machine).
- ✅ Offline mutation outbox (migration v6). Server-side replay is the remaining step.
- ❌ No JMAP/Gmail API/Microsoft Graph connectors (optional per plan).
- ✅ Encrypted-only posture, XOAUTH2, folder special-use, UID dedupe.

### 1.3 Local storage & model
- ✅ Threads/conversations (migration v4).
- ✅ **User labels are data-driven.** The sidebar Labels section reads the `labels` table: create via the "+" inline composer (auto-assigned palette color), rename/delete via right-click menu, apply/remove via a per-message context-menu submenu, and selecting a label lists its messages (`list_messages_by_label`). The old hardcoded Work/Design/Personal/Finance/Travel views are gone (AI triage categories still render as pills on messages).
- ✅ Reminders/snooze + follow-up (migration v3).
- ❌ No rules/automation table.
- ❌ No contacts/participants model.

### 1.4 Reading a conversation
- ✅ Attachment download/open/preview wired off-thread.
- ✅ **Safe HTML rendering + remote-content consent gate.** `src/mail/html.rs`
  sanitizer wired into the reader via `render_message_body`: renders sanitized
  HTML (stripped to safe text) when `body_html` is available, with a
  privacy-first consent banner ("Load remote content") that gates remote
  `http(s)://` resources. 29 sanitizer tests.
- ✅ Thread collapse / per-message expand.
- ✅ Reader toolbar, tag pills, sender header, quick-action pills.

### 1.5 Composer
- ✅ Compose, reply, forward, attachments, inline AI-drafted-reply insertion.
- ✅ **CC/BCC fields** with show/hide toggle; reply-all now splits original
  To/CC recipients correctly (own address excluded).
- ✅ **Draft autosave**: debounced flush to the `drafts` table while composing,
  saved on exit, restored on next compose, consumed on send, re-saved when a
  send fails, discardable.
- ✅ **Per-account signatures**: editor in the Accounts panel
  (`db.update_account_signature`), appended to new messages.
- ❌ Rich text; scheduled/undo send; inline images.
- ⚠️ Internal note tab renders but has no persistence.

### 1.6 Search & command navigation
- ⚠️ Text search only (FTS5); no structured filters or NL search.
- ❌ Command palette.
- ⚠️ Shortcuts bound but missing reply-all, snooze, undo, Esc, customization.

### 1.7 Offline behavior
- ✅ Read/search/compose/organize locally.
- ✅ Offline mutation queue enqueued (replay wiring is the remaining step).
- ✅ AI summaries persist on-device.

### 1.8 Notifications
- ✅ **Desktop notification system.** `src/notify.rs` (via `notify-rust`):
  `NotificationRule` (account/folder/sender/priority filters), `QuietHours`
  (overnight wrap), `NotificationConfig`, `notify_message`. **Wired into the
  UI**: notifications fire on sync when new mail arrives, with an on/off toggle
  in the settings panel (persisted as `notify.enabled`). 20 tests.

---

## Part 2 - Contextual intelligence

### 2.1 Capabilities (`src/ai/`)

| Plan capability      | Status | Notes                                                                 |
| -------------------- | ------ | --------------------------------------------------------------------- |
| Summarize thread     | ✅     | `summarize_thread` on the formatted conversation. |
| Draft a reply        | ✅     | Editable, never auto-sent; inserted into composer.                    |
| Improve writing      | ⚠️     | `polish` on the selected message body, not the composer draft. |
| Adjust tone          | ✅     | Tone from persisted `ai.tone`, passed into `draft_reply`/`polish`. |
| Detect action items  | ✅     | **Model-based** `extract_action_items` (src/ai/actions.rs): JSON array of {task, owner, due}. **Wired into the UI**: `run_detect_actions` now calls the model off-thread and records results in the AI ledger, replacing the old local bullet heuristic. |
| Categorize email     | ✅     | `triage` + `apply_triage` writes one `ai_category`. |
| Schedule follow-up   | ✅     | `schedule_followup` stores a real reminder with undo. |
| Explain content      | ❌     | Not implemented.                                                       |
| Search naturally     | ❌     | No natural-language → filter translation.                             |

- ❌ No streaming (`complete` is blocking; SSE streaming is future work).
- ✅ **Local-model health check wired into the UI.** `check_health` probes
  `/models` then `/api/tags`; the AI settings panel has a "Check connection"
  button that shows reachable status + model list.
- ✅ Uncertainty instruction in all prompt templates.

### 2.2 Insights
- ⚠️ AI Insights block: thread participant count and message count now computed from real data; response-rate placeholder removed.

### 2.3 Safety rules
- ✅ Never auto-sends; AI output is an editable draft.
- ✅ Does not delete/archive/label/move mail.
- ✅ Smallest context; excludes attachments.
- ✅ Original messages remain visible; summaries additive.
- ✅ Uncertainty instruction encoded in all prompts.
- ⚠️ AI action ledger has no view/delete UI.
- ❌ No global / per-account / per-conversation AI disable or setup opt-in.

---

## Part 3 - Advanced workflow
- ❌ Rules & automations.
- ❌ Scheduled send, send-later, undo-send.
- ❌ Saved searches.
- ❌ Contact intelligence / calendar-aware suggestions.
- ❌ Multiple identities & per-identity signatures.
- ❌ Command palette expansion.
- ❌ Shared inboxes, assignments, internal notes, collision detection, team
  policies, plugin/extension API (Phase 4).

---

## Part 4 - Privacy & security
- ✅ Credentials (passwords, OAuth tokens, AI keys, OAuth client secrets) in
  the OS credential store only.
- ✅ Zero telemetry/ads.
- ✅ Account removal cascades local data.
- ❌ Local cache is plaintext (SQLCipher/DPAPI not implemented).
- ❌ No AI opt-in during account setup, redaction controls, retention policy,
  or view/delete AI artifacts UI.
- ⚠️ "No training by default" not enforced in the request.
- ❌ Enterprise policy controls not implemented.

---

## Part 5 - Settings & accessibility
- ⚠️ Appearance: dark/light toggle only. **Japandi design system refined** - tiered radius tokens (6px/8px), theme radius set; honest UI (fake formatting bar removed, real AI loading state); compose/accounts field labels; section dividers; refined brand mark, empty states, drag preview; AI insights use real thread data; live sync indicator in list header.
- ⚠️ AI settings: model + base URL + tone editable; provider picker not wired to presets.
- ✅ **i18n wired into the UI.** `src/i18n.rs` (8 locales, 25 keys in en-US/pt-PT/ja-JP,
  `t()`/`t!()`). **Wired**: sidebar labels (Compose, Priority, Sent, Drafts,
  Archive, Snoozed, Trash, Unread, Has attachments) now use `i18n::t()` with
  the active locale; a locale picker in the settings panel persists the
  choice as `ui.locale`. 17 tests.
- ✅ **Notifications settings UI**: on/off toggle in the settings panel,
  persisted as `notify.enabled`.
- ❌ Composer settings, shortcuts customization, rules pages.
- ❌ Diagnostics health/export with redaction.
- ⚠️ Accessibility: tooltips + shortcuts exist; no focus/contrast/font/reduced-motion audit.
- ❌ Installer packaging, code signing, clean-machine validation.

---

## Prioritized next-up

### P0 - complete the foundations
1. Replay the offline outbox to the server on reconnect (IMAP STORE/MOVE),
   surfacing conflicts.
3. Encrypt the local cache at rest (SQLCipher or OS DPAPI on `cache.db`).

### P1 - complete the composer + sync
4. Scheduled/undo send; draft list UI (autosave itself is done).
5. Visible sync state machine (syncing/offline/error) with background schedule
   and first-run sync.
6. Per-account/mailbox/sender notification rule UI + quiet hours editor.

### P1 - AI depth
7. Wire `t!()` into more UI strings (reader, composer, settings).
8. View/delete UI for cached AI artifacts; global/per-account AI disable.
9. Compute the AI Insights block from real data.

### P2 - workflow & polish
10. Structured search filters + natural-language search; command palette.
11. Accessibility pass (focus, contrast, font scaling, reduced motion),
    diagnostics export, installer signing.
