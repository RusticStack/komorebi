# IPC Input Validation

Last updated: 2026-05-22

## Current State

Komorebi still has no frontend-callable IPC commands. The validation layer exists first so future commands have a shared denial policy before they can touch credentials, files, SQL, network URLs, or mailbox state.

The code source of truth is `src-tauri/src/ipc_validation.rs`.

## Required Validation Before Side Effects

Every future IPC command must validate inputs before credential lookup, filesystem access, SQL access, network calls, local cache mutation, or server-affecting mail actions.

The command must reject:

- Arbitrary paths: React must not provide local file paths for cache, attachment, database, diagnostics, or config writes. Backend code generates and scopes paths.
- Raw SQL: commands must not accept SQL strings from React. Query helpers, when added, must be typed and scoped.
- Unchecked URLs: URLs must be parsed and limited to approved schemes and hosts for the command's purpose. The shared validator currently allows only absolute `http` and `https` URLs with a host and no embedded credentials.
- Unsanitized HTML: no command may persist or send raw HTML from React until a sanitizer is implemented and the command accepts a sanitized wrapper type. Current validation rejects markup by default.
- Unbounded payloads: commands must set byte limits for each text, binary, HTML, MIME, attachment, and diagnostics payload.
- Unknown or unowned ids: account, folder, message, attachment, and draft ids must be format-checked and resolved against backend ownership before work begins.

Validation errors must be safe for React. Do not echo credentials, email addresses, subject lines, message bodies, raw URLs with secrets, local filesystem paths, SQL, or raw HTML in user-facing errors.

## Shared Validators

`src-tauri/src/ipc_validation.rs` currently provides:

- `validate_payload_size` and `validate_bounded_text` for payload limits.
- `reject_client_supplied_path` for commands that must never accept frontend-provided paths.
- `reject_raw_sql` for denial of raw SQL-shaped payloads.
- `validate_checked_url` for parsed `http`/`https` URLs without embedded credentials.
- `reject_unsanitized_html` for rejecting raw markup until a sanitizer-backed type exists.
- `validate_entity_id` for internal id shape.
- `validate_message_scope` plus `EntityLookup` for account/folder/message existence and ownership checks.

These helpers are not a substitute for command-specific validation. They are the minimum shared policy.

## Future Command Checklist

Before registering a command in `invoke_handler`, add:

- A typed request and response, not raw JSON maps.
- A command-specific validation function that calls the shared validators.
- Backend ownership checks for every referenced account, folder, message, attachment, and draft.
- Bounded payload sizes for every user-provided string or byte array.
- Redacted error mapping for frontend display.
- Tests for every denial path and at least one success path.
- A generated command permission in `src-tauri/build.rs` and a narrow capability entry in `src-tauri/capabilities`.

Do not add a command if validation depends on real mailboxes, real credentials, or real app-data paths. Use fake stores, fixtures, and temp directories.

## Current Denial Tests

Rust tests cover:

- Client-supplied local paths.
- Raw SQL verbs, comments, semicolons, and CTE-shaped SQL.
- Unsafe URL schemes, missing hosts, and embedded credentials.
- Raw HTML, script URLs, and inline event handlers.
- Oversized payloads.
- Unknown account, folder, and message ids.
- Invalid id shapes before lookup.
- Validation errors not echoing sensitive input values.

`pnpm check` runs the validation tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- Tauri command permissions: https://v2.tauri.app/security/permissions/
- Tauri command scopes: https://v2.tauri.app/security/scope/
