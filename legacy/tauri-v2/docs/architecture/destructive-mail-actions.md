# Destructive Mail Action Semantics

Last updated: 2026-05-24

## Current State

Komorebi exposes destructive and server-affecting reader actions through the typed `perform_mail_action` Tauri command in `src-tauri/src/mail_actions.rs`. The React reader calls this command for mark-read/unread, move, archive, delete-to-trash, and expunge, and it never receives raw IMAP commands, local paths, credentials, or backend folder-resolution details.

No automated test uses a real mailbox. Tests cover the action planner with in-memory stores, command denial paths with fake stores, UI command payloads with injected handlers, E2E fixture behavior, and local socket protocol fixtures for the live IMAP adapter.

## Typed Actions

The current typed action surface is:

- `MarkRead { read }`: message-level flag update.
- `Move { target_folder_id }`: message-level folder move.
- `Archive { archive_folder_id }`: message-level move to the account's archive folder.
- `DeleteToTrash { trash_folder_id }`: message-level soft delete by moving to Trash.
- `ExpungeDeleted`: folder-level permanent removal of messages already marked deleted by the server.

Each action is wrapped in a typed request with an account id, folder id, optional message id, user-intent flag, confirmation state, and retry attempt number. Commands validate account/folder/message ids before server mutation. `Archive` and `DeleteToTrash` resolve the account's special-use Archive/Trash folder in Rust, not from a frontend-provided remote folder name.

## Confirmation And Reversibility

Mark read/unread, move, and archive do not require destructive confirmation. They produce an undo action:

- Mark read/unread reverses to the opposite read state.
- Move reverses by moving from the target folder back to the source folder.
- Archive reverses by moving from Archive back to the source folder.

Delete-to-trash requires explicit user confirmation and remains reversible by moving from Trash back to the source folder while the message still exists there. Delete is implemented as a move to the account's Trash folder where available, not as a default hard delete.

Expunge requires explicit user confirmation and is irreversible. It does not produce an undo action. The UI must present expunge as permanent removal.

## Delete Policy

Delete should prefer move-to-trash where the provider exposes a Trash folder. Do not implement a direct hard-delete message action as the default path. Permanent removal is represented as folder-level expunge of already-deleted messages after confirmation.

If a provider lacks Trash, a future provider-specific policy must document whether delete is unsupported, archive-like, or a confirmed permanent operation.

## Retry Semantics

Server-affecting actions use a retry policy with `MAX_MAIL_ACTION_ATTEMPTS` attempts. Transient server failures return a retry decision until the limit is exhausted. Validation errors, missing account/folder/message targets, confirmation failures, and permanent server errors are not retryable.

The local mailbox-state tests verify transient failures do not mutate local test state before retry.

`perform_mail_action` returns redacted retry metadata to React. The reader shows Retry only for retryable failures and resubmits the same typed request with an incremented attempt count.

## IMAP Mapping

The current live IMAP mapping is:

- Mark read/unread: `UID STORE` with `\Seen` flag changes.
- Move/archive/delete-to-trash: `UID MOVE` to the validated target folder.
- Expunge: folder `EXPUNGE` for messages already marked deleted by the server/cache.

Do not use sequence numbers for durable action identity. The command resolves local account/folder/message ids to account id, folder id, remote folder id, UIDVALIDITY, UID, RFC Message-ID, and current flag JSON before remote calls.

`UID MOVE` success is reconciled through `COPYUID` when the server returns it. If no `COPYUID` is returned, Komorebi searches the destination folder by RFC Message-ID. If the destination UID cannot be determined, the command returns a retryable failure rather than inventing a local UID.

`perform_mail_action` stops the account's background sync worker before issuing the server-affecting mutation and restarts it afterward when possible. Successful mutations update the local SQLite cache and folder unread/message counts immediately; later background sync remains the reconciliation path.

Expunge deletes local cache rows for deleted messages in the selected folder after the server accepts `EXPUNGE` and removes associated attachment files best-effort under app data. It never exposes attachment paths to React.

## UI Behavior

The reader exposes:

- Mark read/unread with undo.
- Move to another folder in the same account with undo.
- Archive when the account has an archive folder, with undo.
- Delete with a confirmation panel and undo.
- Expunge only from Trash-like folders, with a permanent confirmation panel and no undo.

The UI does not expose direct hard delete. Missing special-use target folders disable the corresponding action instead of guessing provider behavior.

## Tests

`src-tauri/src/mail_actions.rs` tests cover:

- Mark-read state change and undo.
- Move state change and undo.
- Archive state change and undo.
- Delete-to-trash confirmation and undo.
- Expunge confirmation and irreversible removal.
- Transient retry without state mutation before retry.
- Unknown and invalid target rejection before mutation.
- Frontend-command request conversion, special-folder resolution, retry metadata, and confirmation enforcement.

Additional coverage:

- `src-tauri/src/imap_live.rs` tests map `UID STORE`, `UID MOVE`, `COPYUID`, Message-ID search, and `EXPUNGE` behavior through a local socket fixture.
- `src-tauri/tests/ipc_denial_paths.rs` rejects missing user intent, invalid IDs, unowned folders, confirmation gaps, and unauthorized mail actions before fake-store mutation.
- `src/App.test.tsx` covers reader mark-read, move, archive, delete, expunge, undo, confirmation, and retry UI flows.
- `e2e/mail-shell.smoke.spec.ts` covers the browser-level fixture mail-action bridge.

`pnpm check` runs these tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- RFC 9051 IMAP4rev2: https://www.rfc-editor.org/rfc/rfc9051.html
- RFC 6851 IMAP MOVE extension: https://www.rfc-editor.org/rfc/rfc6851
