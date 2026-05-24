# IMAP Sync State Machine

Status: Rust state machine, live IMAP protocol adapter, app-managed background scheduler, pending Sent APPEND upload, folder worker, folder/flag semantics, and SQLite cache integration implemented.
Last reviewed: 2026-05-24.

## Current Scope

The code sources of truth are `src-tauri/src/imap_sync.rs`, `src-tauri/src/imap_live.rs`, and `src-tauri/src/sync_scheduler.rs`.

The sync engine is provider-agnostic and still runs behind `ImapSyncServer` and `ImapIdleServer` traits so sync semantics can be tested without real accounts. `src-tauri/src/imap_live.rs` now implements those traits with blocking `native-tls`/TCP IMAP protocol I/O. The project has not adopted Tokio or `async-imap`; evaluate that separately before changing the runtime model.

Implemented now:

- Account-level sync entry point with cancellation checks.
- Folder discovery from the remote server.
- Folder `UIDVALIDITY`, `UIDNEXT`, unread count, total count, special-use, kind, delimiter, and provider attribute cache updates.
- Initial message fetch for all remote UIDs missing from the local cache.
- Incremental UID fetch for newly observed remote UIDs only.
- Existing-message flag refresh for retained UIDs.
- Expunge handling by deleting local messages whose UIDs disappeared from the selected folder.
- UIDVALIDITY reset handling by clearing the affected folder cache before refetch.
- Transient server retry through reconnect and bounded exponential backoff.
- SQLite cache integration through `SqliteSyncCache`.
- Per-account/per-folder worker loop through `ImapSyncEngine::run_folder_worker`.
- Folder IDLE waits through `ImapIdleServer::idle_wait`.
- Polling fallback when IDLE is unavailable or rejected.
- Periodic folder refresh after IDLE timeout.
- Clean stop reports for app-exit cancellation and account removal.
- Fake IMAP server fixtures that simulate initial sync, incremental sync, flag changes, expunges, UIDVALIDITY reset, IDLE wakeups, IDLE timeout refresh, polling fallback, reconnect, backoff, account removal, and cancellation.
- Local-loopback protocol IMAP fixtures in `src-tauri/tests/protocol_fixtures.rs` that simulate forced reconnect after greeting, LOGIN auth failure, SELECT metadata, expunge notifications, IDLE EXISTS/EXPUNGE updates, and a 10,000 UID mailbox without a real account.
- Stable message ids shared with the MIME normalization boundary.
- Server flag normalization for `\Seen`, `\Flagged`, `\Answered`, `\Draft`, `\Deleted`, `\Recent`, and provider/custom keywords.
- Folder attribute normalization for IMAP LIST/XLIST/SPECIAL-USE style attributes, including supported special-use detection and preservation of unknown provider attributes.
- Live IMAP connection setup over TLS or STARTTLS, with plaintext available only in tests.
- Generic password `LOGIN` and Gmail/Microsoft-style OAuth `XOAUTH2` authentication without returning secrets to React.
- Live `LIST`, `SELECT`, `UID SEARCH`, chunked `UID FETCH`, flag `UID STORE`, `EXPUNGE`, `CAPABILITY`, `IDLE`, reconnect, cancellation, timeout, max-literal, and redacted-error handling.
- Live fetch parsing from RFC822/RFC5322 bytes through the MIME normalization boundary, including SQLite persistence for message bodies, safe attachment files, attachment metadata, parser-warning JSON, and FTS-visible content.
- Local socket tests for live adapter password auth, XOAUTH2, folder discovery, UID search/fetch, flag fetch/update, expunge, IDLE timeout/change handling, reconnect after network drop, cancellation, redacted auth errors, and parser edge cases.
- App-managed background scheduler registered in `src-tauri/src/lib.rs`.
- Startup loading of active `connected` accounts from the local SQLite cache.
- Account worker restart on account setup, reauthentication, and account settings save.
- Account worker stop on account removal and Tauri window close.
- Bounded concurrent account workers and capped per-account folder workers.
- Worker status persisted to `settings` as `account:{account_id}:sync_worker_status`, including state, update time, preserved last successful sync time, redacted user-facing message, action code, and error code. `load_mail_shell` projects active sync/error/auth state into account status.
- Tauri status events emitted as `komorebi://sync-status` after status-row writes; React listens only to refresh the trusted `load_mail_shell` snapshot.
- Non-secret OAuth client runtime metadata persisted as `account:{account_id}:oauth_client` so backend sync workers can refresh Gmail/Microsoft access tokens without exposing tokens to React.
- Scheduler tests for active-account startup, restart, removal stop, concurrency bounds, status persistence, and OAuth runtime metadata cleanup.

Not implemented by this task:

- Tokio/`async-imap` integration or migration.
- External attachment open/extract and non-plain/raster preview types.
- Opt-in live-provider smoke tests using user-provided test accounts.

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

## Folder Worker

`ImapSyncEngine::run_folder_worker` runs a single account/folder worker:

1. Validate worker config and reject empty account or folder ids.
2. Stop cleanly if the cancellation token is set before work.
3. Stop cleanly if the account has been removed from the local cache.
4. Check whether the folder can use IDLE.
5. If IDLE is available, wait for mailbox change or timeout.
6. If IDLE is unavailable or rejected, wait the configured polling interval.
7. Refresh the folder through `sync_folder_by_remote_id`.
8. Reconnect and back off after transient IDLE/server drops.
9. Continue until cancellation, account removal, or an optional test cycle limit.

The worker returns `FolderWorkerReport` instead of treating normal shutdown as a failure. `Cancelled` represents app exit or account-worker stop. `AccountRemoved` represents account deletion or local metadata removal. Both paths avoid live server mutation and are safe to surface as stopped worker states.

## Background Scheduler

`BackgroundSyncScheduler` is the only app-managed path that starts live sync work automatically.

Startup behavior:

1. Tauri setup opens the app data SQLite cache and applies migrations.
2. Active accounts are loaded from `accounts` where `removed_at IS NULL` and `auth_status = 'connected'`.
3. Workers already owned by the scheduler but no longer active are cancelled and joined.
4. Each active account gets one account worker, subject to the global account-worker concurrency limiter.

Account lifecycle behavior:

1. Generic account setup starts a worker only after live IMAP/SMTP setup checks, credential storage, and account metadata persistence succeed.
2. OAuth account setup and reauthentication store only the refresh token in Windows Credential Manager and store non-secret OAuth client runtime metadata in SQLite before the scheduler starts or restarts that account.
3. Generic reauthentication restarts the account worker after credential rotation.
4. Account settings save restarts the account worker so sync ownership observes current account state.
5. Account removal cancels and joins the worker before credentials/local metadata are removed, and removal deletes scheduler status/OAuth metadata rows.
6. Window close calls `stop_all`, cancelling and joining all owned workers.

Worker behavior:

1. The account worker checks local account eligibility before network work.
2. Generic accounts read the approved generic credential from Windows Credential Manager.
3. Gmail/Microsoft accounts refresh OAuth access tokens in Rust through `TokenLifecycle`; React never receives refresh tokens or access tokens.
4. The worker runs one account-level sync to discover/update folders and messages.
5. It then starts a capped, priority-ordered set of per-folder IDLE/poll workers, with Inbox prioritized.
6. Removed or non-connected accounts are rejected before live network work; `SqliteSyncCache::account_exists` also stops worker loops after removal.
7. Worker errors write a redacted status row, preserve the previous last-successful-sync timestamp, and emit a status event; credential/token failures mark the account `reconnect_needed`.

Concurrency rules:

- The scheduler currently allows two concurrent account workers by default.
- Each account starts at most six folder workers by default, ordered by folder kind and display name.
- Folder workers use the sync engine's backoff policy and cancellation-aware waits.

## Cache Rules

`SqliteSyncCache` mutates only the existing schema:

- Folder rows are matched by `(account_id, remote_id)`.
- Local folder IDs are backend-generated stable IDs derived from account and remote folder identity.
- Folder `attributes_json` preserves normalized provider LIST/XLIST/SPECIAL-USE attributes. Unknown provider attributes do not make the folder unusable.
- Message rows are matched by `(account_id, folder_id, uid_validity, imap_uid)`.
- Local message IDs are backend-generated stable IDs derived from account, folder, UIDVALIDITY, and UID.
- MIME normalization uses the same stable message ID derivation for attachment records.
- Live-fetched MIME warnings are stored as checked JSON in `messages.mime_parse_warnings_json`.
- Live-fetched attachment metadata is replaced from the parsed MIME part set when a raw-message fetch is persisted. Storage-safe decoded bytes are written under generated app-data paths and recorded in `attachments.storage_relative_path`; unsafe parts remain metadata-only with warnings.
- Replaced, UIDVALIDITY-reset, expunged, and account-cache-deleted attachment files are cleaned best-effort.
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

`\Recent` and provider keywords are preserved in `flags_json`.

`RemoteFlag::from_imap_token` maps standard flags case-insensitively and preserves non-standard keywords such as `$Forwarded` or provider-specific `\NonJunk` as keywords. The sync cache updates boolean columns for standard state and always writes the complete normalized flag set into `flags_json`.

## Folder Semantics

`RemoteFolder::from_imap_attributes` is the adapter boundary for folder roles. It normalizes provider attributes into `attributes_json`, detects supported special-use roles, and falls back to `custom` for unknown folders.

Supported detected roles:

- `INBOX` or `\Inbox` -> `kind = inbox`, `special_use = inbox`
- `\Sent` -> `sent`
- `\Drafts` -> `drafts`
- `\Trash` -> `trash`
- `\Junk` or `\Spam` -> `junk`
- `\Archive` -> `archive`
- `\All` or `\AllMail` -> `kind = all`, raw attribute preserved
- `\Important` -> `kind = important`, raw attribute preserved

Unsupported or provider-specific roles remain usable as `kind = custom`, `special_use = NULL`, with their attributes preserved in `attributes_json`.

## Cancellation And Backoff

`SyncCancellationToken` is checked before account work, folder work, IDLE waits, polling waits, retry waits, expunge deletion, flag updates, and message upserts. Account/folder workers convert cancellation into a clean stop report.

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
- IDLE mailbox-change wakeups refreshing a folder.
- IDLE timeout periodic refresh.
- Polling fallback when IDLE is unavailable.
- IDLE capability-probe reconnect and backoff.
- IDLE network-drop reconnect and backoff.
- Clean worker stop on app-exit cancellation.
- Clean worker stop on account removal.
- Invalid fake-server responses for missing requested UIDs.
- Unknown account rejection before server access.
- Standard and custom IMAP flag normalization/preservation.
- Special-use folder detection, unknown provider attribute preservation, and sync of messages in custom folders.
- Storage-safe inbound attachment bytes are written under generated paths, malicious sender filenames do not affect storage, hashes/sizes are preserved, and expunge cleanup removes stored files.
- Pending provider Sent uploads are processed after account sync: Message-ID duplicate checks run before literal APPEND, successful rows clear their temporary payload, and failures preserve receipts/payloads for retry.

`src-tauri/tests/protocol_fixtures.rs` tests cover a socket-level IMAP fixture for future adapter work:

- Forced reconnect after the server drops a connection post-greeting.
- LOGIN auth failure without credential leakage.
- Large mailbox SEARCH over 10,000 UIDs and high-UID FETCH.
- SELECT metadata with UIDVALIDITY, UIDNEXT, EXISTS, and EXPUNGE.
- IDLE continuation, EXISTS wakeup, EXPUNGE notification, and clean DONE completion.
- Literal APPEND handling for local Sent upload fixtures.

`src-tauri/src/imap_live.rs` tests cover:

- Live adapter mapping for password auth, LIST, SELECT, UID SEARCH, UID FETCH, flag fetch/update, EXPUNGE, CAPABILITY, and IDLE.
- XOAUTH2 SASL response construction and redacted authentication failure.
- IDLE timeout and cancellation behavior.
- Reconnect after a dropped socket and reauthentication before further protocol work.
- LIST and FETCH parser edge cases, including quoted folder names and missing UID denial.
- Raw `BODY.PEEK[]` sync persistence for parsed body text, safe attachment bytes, attachment metadata, MIME warning JSON, and FTS search results.
- Sent-folder Message-ID search and literal APPEND to the selected Sent folder without exposing live credentials.

No test uses a real mailbox or real credentials.

## Documentation Sync

Keep this document synchronized with:

- `src-tauri/src/imap_sync.rs`
- `src-tauri/src/imap_live.rs`
- `src-tauri/src/sync_scheduler.rs`
- `src-tauri/src/mime_normalization.rs`
- `src-tauri/src/schema.rs`
- `src-tauri/migrations/*.sql`
- `docs/architecture/schema.md`
- `docs/architecture/ipc-and-mutation-boundaries.md`
- `AGENTS.md`
- `TODO.md`
