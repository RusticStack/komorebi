# Draft And Sent Persistence

Status: backend local draft persistence, frontend autosave/discard command wiring, local recent-recipient persistence, provider-aware Sent-copy planning, and IMAP APPEND upload for pending Sent copies implemented; provider Drafts upload is intentionally deferred and drafts are local-only until sent or discarded.
Last reviewed: 2026-05-23.

## Current Scope

The code source of truth is `src-tauri/src/draft_persistence.rs` and the frontend-callable command wrapper is `src-tauri/src/composer.rs`.

Komorebi persists composer drafts locally in SQLite and links successful sends to provider-aware Sent-copy state:

- Active drafts are stored in `composer_drafts`.
- Active drafts are local-only; they are not uploaded to provider Drafts folders.
- Autosave updates the same active draft and increments `autosave_revision`.
- Restart recovery loads active drafts by account from SQLite.
- Intentional discard marks the draft `discarded` and clears subject, recipients, body, and attachment ids.
- Confirmed send marks the draft `sent`, links `sent_receipt_id`, sets `sent_at`, and clears private draft content.
- Every confirmed send gets a `sent_mail_copies` record that says whether provider Sent upload is unnecessary, pending, or unsupported.
- Confirmed composer sends update local-only `recent_recipients` rows for account-scoped autocomplete.
- Confirmed sends that require IMAP Sent upload keep a temporary RFC822 payload in SQLite until the scheduler appends or safely recognizes the message already exists in the provider Sent folder.

This persistence boundary is now exposed through reviewed Tauri commands for the TipTap composer MVP:

- `autosave_composer_draft`
- `discard_composer_draft`
- `send_composer_email`
- `list_recent_recipients`

The commands keep draft mutation in Rust, validate ids/payloads/ownership, and are granted only through narrow capability permissions.

## Schema

Migration `005_drafts_and_sent_mail.sql` adds:

- `composer_drafts`
- `sent_mail_copies`
- `smtp_send_receipts.source_draft_id`

`composer_drafts` stores editable local draft fields while a draft is active:

- account id
- subject
- To/CC/BCC/Reply-To JSON
- text and sanitized HTML bodies, including the reviewed composer rich-formatting subset
- attachment id JSON
- reply/reference metadata
- autosave revision and timestamps

Migration `006_composer_attachments.sql` adds `composer_attachments` for user-selected outbound files staged by the composer. These rows store account id, draft id, display filename, canonical MIME, byte size, backend-generated storage path, SHA-256, disposition, and optional inline CID. Staged files are plaintext app-data files and are deleted best-effort after discard or confirmed send.

Migration `007_recent_recipients.sql` adds `recent_recipients` for local autocomplete. Rows are scoped by account, deduplicated by normalized email, store optional display name, use count, and last-used timestamp, and are populated only after confirmed composer sends.

Migration `009_sent_mail_append_payloads.sql` adds pending Sent-upload retry state to `sent_mail_copies`: `append_payload`, `append_payload_sha256`, `append_attempt_count`, `last_append_attempt_at`, `uploaded_at`, and the partial pending-append index. The payload is plaintext RFC822 content and is retained only for rows that need IMAP APPEND; it is cleared after a successful upload or remote duplicate detection.

Discarded or sent drafts intentionally clear private content. The row remains only as state evidence and, for sent drafts, a link to the confirmed send receipt.

`sent_mail_copies` stores the Sent-folder plan for a confirmed send:

- provider kind
- policy: `provider_auto_copies`, `imap_append`, or `local_receipt_only`
- sync state: `not_required`, `pending_upload`, `uploaded`, `unsupported`, or `failed`
- optional Sent folder id/remote id
- safe local error text for unavailable append behavior
- temporary RFC822 append payload and SHA-256 while upload is pending or failed
- append attempt count, last attempt time, and upload completion time

## Autosave

`autosave_draft` validates:

- account id and draft id shape
- account exists and is not removed
- bounded subject/body/reference payloads
- JSON array fields for addresses, references, and attachment ids
- sanitized HTML wrapper input

Autosave never accepts raw filesystem paths or attachment filenames; only backend attachment ids can be persisted.

The frontend TipTap composer debounces autosave and also exposes a manual Save draft action. It sends sanitized editor HTML and plain text to Rust; the backend wraps HTML in `DraftHtml` and rejects script tags, JavaScript URLs, inline event handlers, oversized payloads, invalid ids, unknown accounts, and unknown staged attachment ids. Rich formatting is allowed only after `src/mail/composer/composerRichText.ts` reduces it to the supported table, code, alignment, font, color, and highlight subset.

The composer status copy tells users that drafts stay local to the device until sent or discarded. Do not remove or soften this copy unless provider Drafts upload exists with retry, delete, conflict, and cleanup semantics.

The same draft id can be autosaved repeatedly while active. Autosaving a discarded or sent draft is rejected instead of reviving stale private content.

## Discard

`discard_draft` requires an active draft. It then marks the row `discarded`, sets `discarded_at`, and clears:

- subject
- recipients
- body text
- body HTML
- attachment ids

Discarded drafts no longer appear in `list_active_drafts`.

`discard_composer_draft` additionally deletes staged composer attachment rows and removes their app-data files best-effort. It does not delete inbound message attachments.

## Send Conversion

`send_email` in `src-tauri/src/smtp_send.rs` accepts an optional `draft_id`.

`send_composer_email` in `src-tauri/src/composer.rs` re-autosaves the active draft request, validates staged attachments, streams staged files through `ComposerAttachmentStore`, calls `send_email`, and cleans staged composer attachment rows/files after confirmed SMTP acceptance. Failed SMTP delivery leaves the active draft and staged attachments available for retry.

When a draft id is present:

1. The active draft is validated before auth resolution, credential lookup, attachment streaming, MIME composition, or SMTP network activity.
2. SMTP transport must accept the message.
3. In one SQLite transaction, Rust inserts `smtp_send_receipts`, marks the draft `sent`, clears draft content, and inserts `sent_mail_copies`.
4. If the provider policy is `imap_append`, the same transaction stores the exact RFC822 payload needed for later APPEND retry.

If SMTP delivery fails, no receipt is inserted and the draft remains active.

After SMTP acceptance, the composer wrapper best-effort updates local recent-recipient rows from the deduplicated To/CC/BCC set. This cache is local-only and must not be synced or treated as a provider contact list.

## Provider Drafts Policy

Provider Drafts-folder upload is not part of the current product surface. Active drafts remain in the local SQLite cache and staged composer attachment directory only. They are cleared when the user intentionally discards the draft or after SMTP acceptance converts the draft to sent state.

This avoids partial server Drafts writes, duplicate provider drafts, stale remote draft cleanup problems, and provider-specific conflict semantics until a dedicated upload worker is designed. Any future provider Drafts sync must define IMAP APPEND/replace/delete behavior, retry state, duplicate detection, provider-specific folder selection, and failure recovery before exposing UI that implies server-synced drafts.

## Provider Sent Policy

Provider policy is intentionally conservative to avoid duplicate Sent messages:

| Provider | Current policy | Reason |
| --- | --- | --- |
| Gmail | `provider_auto_copies` / `not_required` | Gmail Help says SMTP-sent messages are automatically copied to Gmail/Sent and clients should not save another sent copy on the server. |
| Microsoft | `imap_append` / `pending_upload` when a Sent folder is discovered; otherwise `local_receipt_only` / `unsupported` | Microsoft documents SMTP AUTH as SMTP client submission for sending. It does not define SMTP as Sent-folder persistence, so Komorebi queues a provider-discovered IMAP Sent APPEND when a Sent folder is known. |
| Generic IMAP/SMTP | `imap_append` / `pending_upload` when a Sent folder is discovered; otherwise `local_receipt_only` / `unsupported` | Generic SMTP has no portable Sent-folder persistence guarantee. Komorebi uses provider-discovered IMAP Sent folders or keeps only the local receipt until configured. |

Microsoft Support also documents that duplicate Sent items happen when a server creates a Sent entry and a client uploads another one. Komorebi therefore records a provider policy before upload instead of blindly appending every SMTP send.

## Sent APPEND Upload

`src-tauri/src/sync_scheduler.rs` uploads pending Sent copies after the account sync pass and `send_composer_email` asks the scheduler to refresh the account when a confirmed send creates a `pending_upload` row.

The upload worker:

- Loads only `sent_mail_copies` rows for the same account where `sent_folder_policy = 'imap_append'`, `sync_state IN ('pending_upload', 'failed')`, a Sent folder remote id exists, and a payload exists.
- Ignores Gmail and other `provider_auto_copies` rows, so Gmail SMTP sends are not duplicated in Gmail/Sent.
- Uses `LiveImapAdapter::sent_message_exists` to `UID SEARCH HEADER Message-ID` in the target Sent folder before APPEND. If the server already has the message, the row becomes `uploaded` without sending another copy.
- Uses `LiveImapAdapter::append_sent_message` to issue a literal `APPEND "<Sent folder>" (\Seen) {bytes}` with the stored RFC822 payload.
- Marks successful or already-present rows `uploaded`, sets `uploaded_at`, clears `last_error`, and clears `append_payload` plus `append_payload_sha256`.
- Marks failed rows `failed`, increments `append_attempt_count`, stores redacted local error text, and keeps the payload and local send receipt for a later retry.

The worker never deletes `smtp_send_receipts` on APPEND failure. A failed upload means the local send was still confirmed by SMTP, while provider Sent synchronization remains pending retry.

## Tests

`src-tauri/src/draft_persistence.rs` tests cover:

- Autosave and reload from SQLite.
- Autosave revision increments.
- Intentional discard clears private content.
- Sent conversion clears private content after confirmed receipt.
- Gmail/Microsoft/generic Sent policy resolution.
- Pending IMAP APPEND payload retention, upload-state transitions, retry-state preservation, Gmail duplicate-upload avoidance, and payload cleanup after upload.
- Sanitized HTML and JSON-array denial paths.

`src-tauri/src/smtp_send.rs` tests cover:

- Draft validation before send.
- Failed SMTP delivery leaving the draft active.
- Confirmed SMTP delivery linking the receipt and marking the draft sent.
- Sent-copy state and pending APPEND RFC822 payload written with confirmed sends.

`src-tauri/src/schema.rs` tests cover migration from schema version 4 to version 5, migration from version 5 to version 6, migration from version 6 to version 7, migration from version 7 to version 8, migration from version 8 to version 9, valid draft/Sent-copy rows, valid staged composer attachment rows, recent-recipient uniqueness, MIME-warning JSON, and Sent APPEND retry-state constraints.

`src-tauri/src/sync_scheduler.rs` tests cover pending Sent upload success, remote Message-ID duplicate detection without APPEND, and APPEND failure preserving the receipt and payload for retry.

`src-tauri/src/imap_live.rs` and `src-tauri/tests/protocol_fixtures.rs` tests cover literal IMAP APPEND against local socket fixtures without live accounts.

`src-tauri/src/composer.rs` tests cover:

- Autosave command mapping.
- Rich composer HTML autosave persistence.
- Staged attachment storage under backend-generated app-data paths.
- Unsafe attachment content denial.
- Fake SMTP send through the typed composer wrapper.
- Staged attachment cleanup after confirmed send.
- Invalid send address denial before SMTP work.
- Duplicate recipient normalization and recent-recipient autocomplete persistence after confirmed fake SMTP send.

`src/App.test.tsx` covers frontend autosave/save, address validation, duplicate recipient handling, keyboard recent-recipient autocomplete, attachment and inline-image staging request shapes, signature insertion, send request shape, progress rendering, and redacted SMTP failure mapping.

Normal tests use no live mailbox, live SMTP server, real account, or real credential. `src-tauri/tests/live_smtp_smoke.rs` is ignored and env-gated for explicit user-provided live SMTP test accounts only.

## Not Implemented

- Frontend draft reload/reopen UI after restart.
- Provider Drafts-folder upload remains intentionally deferred; current UI/docs disclose local-only drafts.
- Live IMAP/provider verification beyond the ignored SMTP smoke test.

## References

- Gmail IMAP client settings: https://support.google.com/mail/answer/78892
- Microsoft SMTP AUTH client submission: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/authenticated-client-smtp-submission
- Microsoft OAuth for IMAP/POP/SMTP: https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- Microsoft Support on duplicate IMAP Sent messages: https://support.microsoft.com/en-us/office/imap-sent-messages-in-outlook-for-windows-are-duplicated-and-unread-in-sent-items-folder-f9cb7d98-b6be-4740-89f8-3b7c2277a615

## Documentation Sync

Keep this document synchronized with:

- `src-tauri/src/draft_persistence.rs`
- `src-tauri/src/composer.rs`
- `src-tauri/src/smtp_send.rs`
- `src-tauri/src/sync_scheduler.rs`
- `src-tauri/src/imap_live.rs`
- `src-tauri/src/schema.rs`
- `src-tauri/migrations/005_drafts_and_sent_mail.sql`
- `src-tauri/migrations/006_composer_attachments.sql`
- `src-tauri/migrations/007_recent_recipients.sql`
- `src-tauri/migrations/009_sent_mail_append_payloads.sql`
- `src/mail/composer/TipTapComposer.tsx`
- `src/mail/composer/composerRichText.ts`
- `docs/architecture/smtp-send.md`
- `docs/architecture/schema.md`
- `README.md`
- `AGENTS.md`
- `TODO.md`
