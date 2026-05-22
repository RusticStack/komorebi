# IMAP Sync State Machine

Status: initial Rust state machine and SQLite cache integration implemented.
Last reviewed: 2026-05-22.

## Current Scope

The code source of truth is `src-tauri/src/imap_sync.rs`.

The initial sync layer is provider-agnostic. It does not connect to a live IMAP socket yet; instead, it defines an `ImapSyncServer` trait that a future `async-imap` adapter can implement and tests it with deterministic fake-server fixtures. This keeps UID, flag, expunge, cancellation, reconnect, and cache-mutation semantics testable without real accounts.

Implemented now:

- Account-level sync entry point with cancellation checks.
- Folder discovery from the remote server.
- Folder `UIDVALIDITY`, `UIDNEXT`, unread count, total count, special-use, kind, and delimiter cache updates.
- Initial message fetch for all remote UIDs missing from the local cache.
- Incremental UID fetch for newly observed remote UIDs only.
- Existing-message flag refresh for retained UIDs.
- Expunge handling by deleting local messages whose UIDs disappeared from the selected folder.
- UIDVALIDITY reset handling by clearing the affected folder cache before refetch.
- Transient server retry through reconnect and bounded exponential backoff.
- SQLite cache integration through `SqliteSyncCache`.
- Fake IMAP server fixtures that simulate initial sync, incremental sync, flag changes, expunges, UIDVALIDITY reset, reconnect, backoff, and cancellation.

Not implemented by this task:

- Live `async-imap` network adapter.
- IMAP IDLE or polling fallback loop.
- MIME parsing or attachment extraction.
- SMTP send fixtures.
- Frontend account setup or sync-status UI.

## State Flow

`ImapSyncEngine::sync_account` performs one account sync run:

1. Reject missing local accounts before any server call.
2. List remote folders.
3. For each folder, select it and read `UIDVALIDITY`, `UIDNEXT`, counts, and the server's existing UID set.
4. Upsert the local folder and mark it `syncing`.
5. If cached UIDVALIDITY differs, clear local messages for that folder.
6. Delete local messages whose UIDs no longer exist remotely.
7. Fetch flags for retained known UIDs and update read/flagged/answered/draft/deleted state.
8. Fetch full message records only for UIDs missing from the local cache.
9. Mark the folder `idle`, update sync counters, and set `last_synced_at`.

Transient server failures retry within `BackoffPolicy`. Each retry calls `ImapSyncServer::reconnect`, records the delay in `SyncAccountReport`, and waits through a `BackoffWaiter` that can be cancelled. Permanent server failures, local cache errors, invalid remote states, and unknown accounts do not retry.

## Cache Rules

`SqliteSyncCache` mutates only the existing schema:

- Folder rows are matched by `(account_id, remote_id)`.
- Local folder IDs are backend-generated stable IDs derived from account and remote folder identity.
- Message rows are matched by `(account_id, folder_id, uid_validity, imap_uid)`.
- Local message IDs are backend-generated stable IDs derived from account, folder, UIDVALIDITY, and UID.
- IMAP flags are stored in `flags_json`; standard flags also update boolean columns.
- Message inserts and body updates flow through the existing `messages_fts` triggers.

The state machine treats IMAP UID as folder-scoped. It never uses UID as a global message identity.

## Flag Mapping

The current standard flag mapping is:

- `\Seen` -> `messages.is_read`
- `\Flagged` -> `messages.is_flagged`
- `\Answered` -> `messages.is_answered`
- `\Draft` -> `messages.is_draft`
- `\Deleted` -> `messages.is_deleted`

`\Recent` and provider keywords are preserved in `flags_json`. Special-use folder detection is represented in folder metadata; provider-specific folder behavior remains a follow-up under "Preserve server flags and folder semantics."

## Cancellation And Backoff

`SyncCancellationToken` is checked before account work, folder work, retry waits, expunge deletion, flag updates, and message upserts. Cancellation returns `SyncError::Cancelled` and must not be presented as a sync failure.

`BackoffPolicy` bounds retry attempts, initial delay, maximum delay, and multiplier. Tests use a recording waiter; production workers should use a waiter that sleeps in small slices and checks cancellation.

## Tests

`src-tauri/src/imap_sync.rs` tests cover:

- Initial folder discovery and message fetch into a migrated SQLite database.
- FTS visibility for fetched message bodies.
- Incremental UID fetch without refetching known messages.
- Flag updates for retained messages.
- Expunge removal when a server UID disappears.
- UIDVALIDITY reset and folder cache rebuild.
- Reconnect and backoff after transient server failure.
- Cancellation during backoff before retry.
- Invalid fake-server responses for missing requested UIDs.
- Unknown account rejection before server access.

No test uses a real mailbox or real credentials.

## Documentation Sync

Keep this document synchronized with:

- `src-tauri/src/imap_sync.rs`
- `src-tauri/src/schema.rs`
- `src-tauri/migrations/*.sql`
- `docs/architecture/schema.md`
- `docs/architecture/ipc-and-mutation-boundaries.md`
- `AGENTS.md`
- `TODO.md`
