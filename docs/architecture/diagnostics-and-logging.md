# Diagnostics And Logging

Last updated: 2026-05-25

## Current State

Komorebi has a backend-owned local diagnostics subsystem in `src-tauri/src/diagnostics.rs`. It writes structured NDJSON logs under Tauri app data and exposes a single typed export command, `export_diagnostics_bundle`, that opens a Rust-owned native save dialog. React can request an export only with `userRequested=true`; it never receives log paths, app-data paths, raw mailbox data, credentials, tokens, attachment bytes, or destination paths.

The diagnostics UI lives in `src/settings/AccountSignatureSettings.tsx` and is wired through `src/mail/shell/MailClientShell.tsx`, `src/mail/shell/mailShellDataSource.ts`, `src/App.tsx`, and the Tauri capability allowlist. The export writes a redacted JSON bundle to a user-selected `.json` file with no overwrite.

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

## Capability Boundary

The main webview grants only `allow-export-diagnostics-bundle` for diagnostics. Frontend filesystem, dialog, opener, shell, process, SQL, HTTP, and broad local-system permissions remain denied by `src-tauri/capabilities/capabilities.test.ts`.

Do not add a diagnostics command that accepts arbitrary output paths from React. User-selected destinations must stay backend-owned through the native dialog plugin.

## Tests

`src-tauri/src/diagnostics.rs` tests cover:

- Redaction of secrets, bearer/token text, email addresses, subjects, and bodies before logs are written.
- Export of a disposable SQLite cache containing private account/message/attachment data without leaking those private values.
- Explicit user-intent enforcement.
- No-overwrite export behavior.
- Startup migration, sync status, and command-failure log events.

Frontend tests cover the settings diagnostics UI and the `export_diagnostics_bundle` data-source mapping.

Capability tests cover reviewed command registration and permission allowlisting.

## Regression Rules

- Any new logged field must be reviewed against the redaction rules before it is written.
- Any new bundle section must be count/shape/status data only unless it is synthetic fixture data gated by `includeSyntheticFixtures`.
- Any command failure logging must pass only the command name and a redacted, user-safe reason.
- Any future diagnostic archive format still must exclude credentials, tokens, raw message bodies, private subjects, and attachment bytes by default.
