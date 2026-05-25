# Diagnostics And Logging

Last updated: 2026-05-25

## Current State

Komorebi has a backend-owned local diagnostics subsystem in `src-tauri/src/diagnostics.rs`. It writes structured NDJSON logs under Tauri app data and exposes two typed diagnostics commands: `export_diagnostics_bundle`, which opens a Rust-owned native save dialog, and inputless `get_diagnostics_health`, which returns a redacted health/status view for the settings drawer. React can request an export only with `userRequested=true`; it never receives log paths, app-data paths, raw mailbox data, credentials, tokens, attachment bytes, or destination paths.

The diagnostics UI lives in `src/settings/AccountSignatureSettings.tsx` and is wired through `src/mail/shell/MailClientShell.tsx`, `src/mail/shell/mailShellDataSource.ts`, `src/App.tsx`, and the Tauri capability allowlist. The health view shows current cache/schema status, account/sync counts, retry/backoff history, provider/server categories, attachment storage health, and remediation steps. The export writes a redacted JSON bundle to a user-selected `.json` file with no overwrite.

The user-facing privacy summary for diagnostics is [../../PRIVACY.md](../../PRIVACY.md). Keep it synchronized with this document whenever diagnostic logs, health output, export contents, or exclusions change.

## Log Rules

Logs are local-only. There is no telemetry, upload, remote diagnostics endpoint, or automatic sharing.

Log file:

```text
<tauri-app-data>/logs/komorebi.ndjson
```

Each line is a structured JSON object with:

- `timestampUnixMs`
- `level`
- `component`
- `event`
- `fields`

Current log sources include startup migration status, sync worker state/error/retry status, selected IPC command failures, and diagnostics export completion. Sync logs are written from `src-tauri/src/sync_scheduler.rs` after the scheduler persists its redacted worker status. Startup migration logs are written during Tauri setup before background sync starts.

Sensitive material is redacted before it is written. Field keys containing credentials, passwords, secrets, tokens, authorization data, OAuth codes, subjects, bodies, raw MIME, email addresses, usernames, senders, recipients, or address lists are replaced with `[redacted]`. Free-form error text is additionally scanned for email-like addresses, bearer/token markers, long token-shaped values, and subject/body markers.

Do not log:

- Passwords, app passwords, refresh tokens, access tokens, OAuth codes, credential refs, authorization headers, or credential target names.
- Raw message bodies, sanitized or unsanitized HTML bodies, raw MIME, private subject lines, address lists, attachment bytes, or local app-data paths.
- Provider responses that may contain submitted secrets unless they are first mapped to a reviewed redacted error.

## Export Rules

`export_diagnostics_bundle` accepts only:

```json
{
  "userRequested": true,
  "includeSyntheticFixtures": false
}
```

The Rust command opens the save dialog itself, suggests `komorebi-diagnostics.json`, requires a `.json` destination, and refuses to overwrite existing files. It returns only display-safe metadata: destination file name, bytes written, exported log count, and whether synthetic fixtures were included.

The bundle contains:

- Privacy flags documenting excluded categories.
- Database summary counts only, such as accounts, folders, messages, attachments, settings, auth-status counts, and sync-status counts.
- Recent redacted diagnostic log entries.
- Optional synthetic fixture data when explicitly requested.

The bundle does not contain account email addresses, usernames, display names, credential refs, provider secrets, token material, raw message bodies, subject lines, snippets, address lists, attachment bytes, attachment storage paths, or save/open paths.

Synthetic fixtures are generated examples only. They are safe to include for reproduction because they do not come from the user mailbox or app data.

## Health View Rules

`get_diagnostics_health` accepts no request body and returns only aggregate, display-safe status:

- Cache presence, current/latest schema version, migration state, account/folder/message/attachment/settings counts, auth-status counts, and sync-status counts.
- Retry/backoff summary from redacted logs, including warning/error counts, retry event count, last retry timestamp, event category counts, and a bounded recent event list.
- Attachment storage health counts for stored metadata rows, checked files, missing files, unreadable files, invalid stored paths, oversized files, size mismatches, and hash mismatches.
- Remediation codes mapped in the frontend to user-facing steps such as reconnect accounts, restart for migrations, wait through backoff, retry a command, or export diagnostics.

The health view does not return log messages, provider responses, local paths, attachment storage paths, account ids, message ids, email addresses, subjects, snippets, message bodies, attachment bytes, credential refs, tokens, or secrets. Recent events include only timestamp, level, component, event name, category, state, action, and code.

Attachment health treats persisted storage paths as untrusted cache state. It accepts only app-data-relative paths under the backend-generated `attachments/` namespace, checks parent containment under app data, validates stored file size and SHA-256 for bounded files, and reports only counts.

## Capability Boundary

The main webview grants only `allow-export-diagnostics-bundle` and `allow-get-diagnostics-health` for diagnostics. Frontend filesystem, dialog, opener, shell, process, SQL, HTTP, and broad local-system permissions remain denied by `src-tauri/capabilities/capabilities.test.ts`.

Do not add a diagnostics command that accepts arbitrary output paths from React. User-selected destinations must stay backend-owned through the native dialog plugin.

## Tests

`src-tauri/src/diagnostics.rs` tests cover:

- Redaction of secrets, bearer/token text, email addresses, subjects, and bodies before logs are written.
- Export of a disposable SQLite cache containing private account/message/attachment data without leaking those private values.
- Explicit user-intent enforcement.
- No-overwrite export behavior.
- Startup migration, sync status, and command-failure log events.
- Health view generation for cache status, retry/category history, attachment storage issues, remediation codes, and private-data exclusion.

Frontend tests cover the settings diagnostics UI, health refresh rendering, redacted health display, and the `export_diagnostics_bundle`/`get_diagnostics_health` data-source mappings.

Capability tests cover reviewed command registration and permission allowlisting.

## Regression Rules

- Any new logged field must be reviewed against the redaction rules before it is written.
- Any new bundle section must be count/shape/status data only unless it is synthetic fixture data gated by `includeSyntheticFixtures`.
- Health view additions must stay aggregate/status-only and must not expose ids, paths, provider responses, subjects, bodies, addresses, tokens, or credential refs.
- Any command failure logging must pass only the command name and a redacted, user-safe reason.
- Any future diagnostic archive format still must exclude credentials, tokens, raw message bodies, private subjects, and attachment bytes by default.
