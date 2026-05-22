# Destructive Mail Action Semantics

Last updated: 2026-05-22

## Current State

Komorebi has no frontend-callable mail-action IPC commands yet. The destructive-action semantics exist first in `src-tauri/src/mail_actions.rs` so future UI and sync work have typed behavior before connecting real IMAP accounts.

No live mailbox is used by these tests. `src-tauri/src/mail_actions.rs` includes a local in-memory mailbox store for action-state verification.

## Typed Actions

The current typed action surface is:

- `MarkRead { read }`: message-level flag update.
- `Move { target_folder_id }`: message-level folder move.
- `Archive { archive_folder_id }`: message-level move to the account's archive folder.
- `DeleteToTrash { trash_folder_id }`: message-level soft delete by moving to Trash.
- `ExpungeDeleted`: folder-level permanent removal of messages already marked deleted by the server.

Each action is wrapped in `MailActionCommand` with an account id, typed target, user-confirmation state, and retry attempt number. Commands validate account/folder/message ids before server mutation.

## Confirmation And Reversibility

Mark read/unread, move, and archive do not require destructive confirmation. They produce an undo action:

- Mark read/unread reverses to the opposite read state.
- Move reverses by moving from the target folder back to the source folder.
- Archive reverses by moving from Archive back to the source folder.

Delete-to-trash requires explicit user confirmation and remains reversible by moving from Trash back to the source folder while the message still exists there.

Expunge requires explicit user confirmation and is irreversible. It does not produce an undo action. The UI must present expunge as permanent removal.

## Delete Policy

Delete should prefer move-to-trash where the provider exposes a Trash folder. Do not implement a direct hard-delete message action as the default path. Permanent removal is represented as folder-level expunge of already-deleted messages after confirmation.

If a provider lacks Trash, a future provider-specific policy must document whether delete is unsupported, archive-like, or a confirmed permanent operation.

## Retry Semantics

Server-affecting actions use a retry policy with `MAX_MAIL_ACTION_ATTEMPTS` attempts. Transient server failures return `RetryDecision::RetryNow` until the limit is exhausted. Validation errors, missing account/folder/message targets, confirmation failures, and permanent server errors are not retryable.

The local mailbox-state tests verify transient failures do not mutate local test state before retry.

Future sync/cache integration must record pending, applied, retrying, failed, and undoable action states separately from server state. A command returning success means the remote mutation was accepted by the mail-action layer; cache updates still need reconciliation with sync.

## IMAP Mapping

The intended IMAP mapping is:

- Mark read/unread: `UID STORE` with `\Seen` flag changes.
- Move/archive/delete-to-trash: `UID MOVE` when supported, with provider fallback planned separately if needed.
- Expunge: `UID EXPUNGE` where available, or provider-specific folder expunge after a reviewed compatibility decision.

Do not use sequence numbers for durable action identity. Future implementation must resolve local account/folder/message ids to account id, folder id, UIDVALIDITY, and UID before remote calls.

## Tests

`src-tauri/src/mail_actions.rs` tests cover:

- Mark-read state change and undo.
- Move state change and undo.
- Archive state change and undo.
- Delete-to-trash confirmation and undo.
- Expunge confirmation and irreversible removal.
- Transient retry without state mutation before retry.
- Unknown and invalid target rejection before mutation.

`pnpm check` runs these tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- RFC 9051 IMAP4rev2: https://www.rfc-editor.org/rfc/rfc9051.html
- RFC 6851 IMAP MOVE extension: https://www.rfc-editor.org/rfc/rfc6851
