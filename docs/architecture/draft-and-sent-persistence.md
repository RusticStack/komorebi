# Draft And Sent Persistence

Status: backend local draft persistence and provider-aware Sent-copy planning implemented; frontend composer wiring and IMAP APPEND upload are not registered.
Last reviewed: 2026-05-22.

## Current Scope

The code source of truth is `src-tauri/src/draft_persistence.rs`.

Komorebi persists composer drafts locally in SQLite and links successful sends to provider-aware Sent-copy state:

- Active drafts are stored in `composer_drafts`.
- Autosave updates the same active draft and increments `autosave_revision`.
- Restart recovery loads active drafts by account from SQLite.
- Intentional discard marks the draft `discarded` and clears subject, recipients, body, and attachment ids.
- Confirmed send marks the draft `sent`, links `sent_receipt_id`, sets `sent_at`, and clears private draft content.
- Every confirmed send gets a `sent_mail_copies` record that says whether provider Sent upload is unnecessary, pending, or unsupported.

This is a backend persistence boundary only. It is not exposed through Tauri commands yet because the composer UI, command ACL, capability grant, and frontend validation states do not exist.

## Schema

Migration `005_drafts_and_sent_mail.sql` adds:

- `composer_drafts`
- `sent_mail_copies`
- `smtp_send_receipts.source_draft_id`

`composer_drafts` stores editable local draft fields while a draft is active:

- account id
- subject
- To/CC/BCC/Reply-To JSON
- text and sanitized HTML bodies
- attachment id JSON
- reply/reference metadata
- autosave revision and timestamps

Discarded or sent drafts intentionally clear private content. The row remains only as state evidence and, for sent drafts, a link to the confirmed send receipt.

`sent_mail_copies` stores the Sent-folder plan for a confirmed send:

- provider kind
- policy: `provider_auto_copies`, `imap_append`, or `local_receipt_only`
- sync state: `not_required`, `pending_upload`, `uploaded`, `unsupported`, or `failed`
- optional Sent folder id/remote id
- safe local error text for unavailable append behavior

## Autosave

`autosave_draft` validates:

- account id and draft id shape
- account exists and is not removed
- bounded subject/body/reference payloads
- JSON array fields for addresses, references, and attachment ids
- sanitized HTML wrapper input

Autosave never accepts raw filesystem paths or attachment filenames; only backend attachment ids can be persisted.

The same draft id can be autosaved repeatedly while active. Autosaving a discarded or sent draft is rejected instead of reviving stale private content.

## Discard

`discard_draft` requires an active draft. It then marks the row `discarded`, sets `discarded_at`, and clears:

- subject
- recipients
- body text
- body HTML
- attachment ids

Discarded drafts no longer appear in `list_active_drafts`.

## Send Conversion

`send_email` in `src-tauri/src/smtp_send.rs` accepts an optional `draft_id`.

When a draft id is present:

1. The active draft is validated before auth resolution, credential lookup, attachment streaming, MIME composition, or SMTP network activity.
2. SMTP transport must accept the message.
3. In one SQLite transaction, Rust inserts `smtp_send_receipts`, marks the draft `sent`, clears draft content, and inserts `sent_mail_copies`.

If SMTP delivery fails, no receipt is inserted and the draft remains active.

## Provider Sent Policy

Provider policy is intentionally conservative to avoid duplicate Sent messages:

| Provider | Current policy | Reason |
| --- | --- | --- |
| Gmail | `provider_auto_copies` / `not_required` | Gmail Help says SMTP-sent messages are automatically copied to Gmail/Sent and clients should not save another sent copy on the server. |
| Microsoft | `imap_append` / `pending_upload` when a Sent folder is discovered; otherwise `local_receipt_only` / `unsupported` | Microsoft documents SMTP AUTH as SMTP client submission for sending. It does not define SMTP as Sent-folder persistence, so Komorebi records a pending IMAP APPEND plan when a Sent folder is known. |
| Generic IMAP/SMTP | `imap_append` / `pending_upload` when a Sent folder is discovered; otherwise `local_receipt_only` / `unsupported` | Generic SMTP has no portable Sent-folder persistence guarantee. Komorebi must use provider-discovered IMAP Sent folders or keep only the local receipt until configured. |

Microsoft Support also documents that duplicate Sent items happen when a server creates a Sent entry and a client uploads another one. Komorebi therefore records a provider policy before upload instead of blindly appending every SMTP send.

## Tests

`src-tauri/src/draft_persistence.rs` tests cover:

- Autosave and reload from SQLite.
- Autosave revision increments.
- Intentional discard clears private content.
- Sent conversion clears private content after confirmed receipt.
- Gmail/Microsoft/generic Sent policy resolution.
- Sanitized HTML and JSON-array denial paths.

`src-tauri/src/smtp_send.rs` tests cover:

- Draft validation before send.
- Failed SMTP delivery leaving the draft active.
- Confirmed SMTP delivery linking the receipt and marking the draft sent.
- Sent-copy state written with confirmed sends.

`src-tauri/src/schema.rs` tests cover migration from schema version 4 to version 5 and valid draft/Sent-copy rows.

No test uses a live mailbox, live SMTP server, real account, or real credential.

## Not Implemented

- Frontend composer autosave UI.
- Tauri commands for autosave, discard, draft reload, or send.
- IMAP APPEND worker for pending Sent uploads.
- Provider-specific Sent upload conflict handling.
- Draft upload to provider Drafts folders.
- Live provider verification.

## References

- Gmail IMAP client settings: https://support.google.com/mail/answer/78892
- Microsoft SMTP AUTH client submission: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/authenticated-client-smtp-submission
- Microsoft OAuth for IMAP/POP/SMTP: https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- Microsoft Support on duplicate IMAP Sent messages: https://support.microsoft.com/en-us/office/imap-sent-messages-in-outlook-for-windows-are-duplicated-and-unread-in-sent-items-folder-f9cb7d98-b6be-4740-89f8-3b7c2277a615

## Documentation Sync

Keep this document synchronized with:

- `src-tauri/src/draft_persistence.rs`
- `src-tauri/src/smtp_send.rs`
- `src-tauri/src/schema.rs`
- `src-tauri/migrations/005_drafts_and_sent_mail.sql`
- `docs/architecture/smtp-send.md`
- `docs/architecture/schema.md`
- `README.md`
- `AGENTS.md`
- `TODO.md`
