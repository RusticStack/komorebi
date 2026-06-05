# IPC Input Validation

Last updated: 2026-05-24

## Current State

Komorebi has reviewed frontend-callable IPC commands:

- `load_mail_shell`: inputless, read-only local-cache shell snapshot.
- `load_message_page`: read-only local-cache message page query with a typed scope/cursor request.
- `search_mail`: read-only local-cache FTS query with a typed request.
- `allow_remote_content`: typed remote-content preference persistence for a stored message or sender.
- `get_ui_preferences`: inputless local UI preference lookup for the optional language override.
- `update_ui_preferences`: typed local UI preference persistence for the optional language override.
- `perform_mail_action`: typed server-affecting reader action for mark-read/unread, move, archive, delete-to-trash, and expunge.
- `open_external_link`: typed external link opening for delegated, sanitized message links.
- `preview_attachment`: typed reader preview for stored local-cache attachment content.
- `save_attachment`: typed reader save for stored local-cache attachment content through a backend-owned native save dialog.
- `download_attachment`: typed reader byte download for stored local-cache attachment content.
- `autosave_composer_draft`: typed local draft autosave.
- `discard_composer_draft`: typed local draft discard and staged attachment cleanup.
- `stage_composer_attachment`: typed user-selected byte staging with backend-generated storage paths.
- `stage_forwarded_attachments`: typed copy of validated cached reader attachment bytes into a composer draft.
- `send_composer_email`: typed send request routed through Rust SMTP/draft/credential boundaries.
- `list_recent_recipients`: typed local recent-recipient autocomplete query.
- `list_composer_drafts`: inputless local draft reload/reopen query.
- `discover_account_settings`: typed email-only account discovery that validates address/domain shape, builds only backend-owned HTTPS autoconfig URLs, rejects invalid host shapes from discovery XML, ignores plaintext autoconfig servers, and returns manual state instead of guessed hostnames when discovery fails.
- `create_account`: typed generic account setup that enforces provider/auth policy, runs live generic IMAP/SMTP login checks before credential and metadata writes, and rejects direct Gmail/Microsoft secret handoff.
- `reauth_account`: typed generic credential rotation for an existing account and rejection path for Gmail/Microsoft OAuth accounts.
- `start_oauth_account_setup`: typed Gmail/Microsoft OAuth loopback setup that validates account metadata/client id, opens the browser from Rust, validates callback state, exchanges the authorization code in Rust, requires a refresh token, stores it without exposing secrets to React, persists non-secret OAuth runtime metadata, and starts the backend sync scheduler for the account.
- `start_oauth_account_reauth`: typed Gmail/Microsoft OAuth loopback reauthentication that validates account id and either a provided client id or stored non-secret OAuth client metadata, rotates the refresh token only after backend token exchange, persists non-secret OAuth runtime metadata, and restarts the backend sync scheduler for the account.
- `remove_account`: typed account removal with credential deletion before metadata/cache cleanup.
- `update_account_settings`: typed account display-name, signature HTML, and local-preference persistence.
- `test_account_connection`: typed generic IMAP/SMTP setup login validation without persistence.
- `get_account_auth_status`: typed redacted auth-status lookup.
- `export_diagnostics_bundle`: typed diagnostics export through a backend-owned native save dialog.
- `get_diagnostics_health`: inputless diagnostics health/status lookup.

`load_message_page` validates account/folder/unified-inbox scope, optional backend-issued cursor shape/scope/sort key, and bounded page limit before querying SQLite. Unified pages cannot include account or folder ids; non-unified pages must include an account or folder scope. The command rejects invalid IDs, unknown accounts, folders outside the requested account scope, forged or out-of-scope cursors, raw SQL-shaped cursor text, raw HTML/markup-shaped cursor text, and oversized cursors. It does not accept raw SQL, offset SQL, paths, URLs, message bodies, HTML to persist, or cache filenames from React.

`search_mail` validates the query and optional account/folder scope before executing backend-owned SQLite reads. It rejects raw SQL-shaped input, raw HTML/markup, URL/path-shaped input, oversized queries, invalid IDs, unknown accounts, and folders outside the requested account scope. Supported advanced syntax is parsed in Rust into sanitized FTS terms plus parameterized metadata filters for `from:`, `subject:`, `body:`, `has:attachment`, and `is:read|unread|flagged|starred`; unsupported `has:`/`is:` values and duplicate/conflicting filters are denial paths. It does not accept raw SQL, raw FTS `MATCH` syntax, paths, URLs, message bodies, HTML to persist, account/folder names as query syntax, or cache filenames from React.

`allow_remote_content` validates account/folder/message id shape, active account/folder/message ownership, and `userRequested=true` before writing a SQLite preference row. The request may choose only `message` or `sender` scope; React never supplies sender email or remote URLs. Sender-scoped preferences derive the sender from the stored message row and reject messages without a usable sender address.

`get_ui_preferences` accepts no request fields and reads only the optional `settings` row `ui:language`. `update_ui_preferences` accepts only the optional enum values `en`, `pt-PT`, or `null`; `null` deletes the override so first-launch language detection applies again. The command rejects unknown language values through serde before persistence and never accepts raw setting keys, JSON strings, SQL, paths, account ids, or arbitrary preference maps from React.

`perform_mail_action` validates account/folder/message id shape, active ownership, `userRequested=true`, confirmation for delete-to-trash and expunge, target-folder ownership for moves, archive/trash special-folder availability, and retry attempts before live IMAP mutation or local cache update. Rust resolves remote folder names and stored UIDs from SQLite. React cannot submit raw IMAP commands, raw SQL, credential refs, local paths, remote provider folder names for archive/delete, or hard-delete requests. The command stops the account sync worker around accepted mutations, returns redacted retry metadata for transient failures, and returns undo metadata only for reversible actions.

`open_external_link` validates `userRequested=true`, URL length, absolute URL parsing, scheme, host policy, credential absence, and command-specific `mailto` fields before opening. `http` and `https` links require public hosts and reject loopback, localhost, link-local, and private network hosts. `mailto` links require at least one valid recipient and allow only `to`, `cc`, `bcc`, and `subject`; body/attachment fields, encoded control characters, local paths, scripts, and custom schemes are denied. Errors must not echo rejected URLs.

Attachment reader commands validate account/folder/message/attachment id shape, active ownership, `userRequested=true`, stored app-data-relative source paths, canonicalized source containment under app data, persisted size, persisted SHA-256, sniffed-vs-declared MIME type, type/size allowlists, and preview allowlists before returning bytes or writing a selected file. React never provides local source or destination paths. `save_attachment` receives the native save-dialog result in Rust, enforces the canonical extension, refuses overwrite, and returns only display-safe labels.

Composer commands validate account/draft/attachment ids, payload sizes, address shapes on send, sanitized body wrappers, reply/reference Message-ID values, staged attachment ownership, MIME type/size, local recent-recipient account scope, and app-data storage scope before side effects. They never accept frontend-supplied filesystem paths, raw SQL, credential material, or arbitrary provider commands. `stage_composer_attachment` accepts bytes from a browser `File` selected by the user and writes only to a Rust-generated app-data path. `stage_forwarded_attachments` accepts only account/folder/message/attachment ids and `userRequested=true`; Rust reuses the reader attachment validation path for source ownership, app-data source path, hash, size, and MIME sniffing, then writes new composer attachment rows under generated paths. `list_composer_drafts` accepts no request fields and returns active local drafts with typed address/body/thread fields and display-safe attachment metadata only; it does not expose storage paths or hashes.

The composer frontend sanitizer is `src/mail/composer/composerRichText.ts`. It is the reviewed source for rich HTML that may cross IPC: only the supported TipTap subset is preserved, and inline styles are reduced to whitelisted alignment, color, background-color, font-size, and font-family values before Rust accepts `DraftHtml` or `SanitizedHtml` wrappers.

Account-management commands validate provider kind, auth kind, account id shape, email/username shape, host names, host labels, ports, security mode, display-name and secret payload size, and credential backend success before side effects. Secret values containing CR/LF/NUL are rejected before protocol work. Gmail and Microsoft password/app-password setup is rejected before any credential write. Public generic account commands reject Gmail/Microsoft refresh-token handoff so those providers must use `start_oauth_account_setup`/`start_oauth_account_reauth`. OAuth loopback commands additionally validate callback path, state, provider error codes, authorization-code bounds, provided or stored OAuth client ID, timeout bounds, and refresh-token presence before metadata, credential, scheduler, or OAuth-runtime-setting mutation. Generic `test_account_connection` validates the setup shape, requires TLS or STARTTLS, performs backend-owned IMAP LOGIN and SMTP AUTH, and writes no credentials or metadata. Generic `create_account` runs the same live login check before writing the credential, then inserts account metadata only after credential storage succeeds and starts the scheduler after persistence. `update_account_settings` validates active account ownership, display-name size, sanitized signature size/output, and serde-owned local preferences before mutating `accounts` or `settings`, then restarts the scheduler. `remove_account` stops scheduler work and deletes scheduler/OAuth runtime settings along with account metadata. `get_account_auth_status` updates the stored account status to `reconnect_needed` when the approved credential is missing and still returns only redacted status.

The code source of truth is `src-tauri/src/ipc_validation.rs`.

## Required Validation Before Side Effects

Every IPC command with frontend-supplied input must validate that input before credential lookup, filesystem access, SQL access, network calls, local cache mutation, or server-affecting mail actions. Commands with no input, such as `load_mail_shell`, must stay inputless unless this document and denial tests are updated first.

The command must reject:

- Arbitrary paths: React must not provide local file paths for cache, attachment, database, diagnostics, or config writes. Backend code generates and scopes paths.
- Raw SQL: commands must not accept SQL strings from React. Query helpers, when added, must be typed and scoped.
- Unchecked URLs: URLs must be parsed and limited to approved schemes and hosts for the command's purpose. The shared validators cover checked `http`/`https` URLs and stricter external-link URLs for `http`/`https`/`mailto`.
- Unsanitized HTML: no command may persist or send raw HTML from React unless it uses a reviewed sanitizer and the command accepts a sanitized wrapper type. Current non-composer validation rejects markup by default.
- Unbounded payloads: commands must set byte limits for each text, binary, HTML, MIME, attachment, and diagnostics payload.
- Unknown or unowned ids: account, folder, message, attachment, and draft ids must be format-checked and resolved against backend ownership before work begins.

Validation errors must be safe for React. Do not echo credentials, email addresses, subject lines, message bodies, raw URLs with secrets, local filesystem paths, SQL, or raw HTML in user-facing errors.

## Shared Validators

`src-tauri/src/ipc_validation.rs` currently provides:

- `validate_payload_size` and `validate_bounded_text` for payload limits.
- `reject_client_supplied_path` for commands that must never accept frontend-provided paths.
- `reject_raw_sql` for denial of raw SQL-shaped payloads.
- `validate_checked_url` for parsed `http`/`https` URLs without embedded credentials.
- `validate_external_link_url` for parsed external `http`/`https`/`mailto` links without embedded credentials, private/local hosts, unsafe `mailto` fields, control-character injection, local paths, or custom schemes.
- `reject_unsanitized_html` for rejecting raw markup until a sanitizer-backed type exists.
- `validate_entity_id` for internal id shape.
- `validate_message_scope` plus `EntityLookup` for account/folder/message existence and ownership checks.
- Attachment-specific path, filename, type, size, and preview validation lives in `src-tauri/src/attachment_safety.rs`.
- Destructive mail-action target validation and retry semantics live in `src-tauri/src/mail_actions.rs`.

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

`load_message_page` and `search_mail` are the read-only examples of accepted input-bearing commands. `load_message_page` request fields are typed, limit and cursor text are bounded, account/folder/unified scope is validated against active cache rows, and the cursor is used only as a keyset value in backend-owned SQL. `search_mail` request fields are typed, query length is bounded, user input is converted into sanitized FTS terms and parameterized metadata filters rather than raw `MATCH` syntax, account/folder scope is validated against active cache rows, and denial paths are covered in `src-tauri/src/mail_shell.rs`.

Reader attachment commands are the file-operation examples: request fields are typed, IDs are ownership-checked against active account/folder/message rows, persisted source paths are treated as untrusted cache state and canonicalized under app data, content is revalidated by hash/size/sniffing before any IPC byte response or file write, save destinations come from a backend-owned native dialog, overwrites are refused, and external open remains unavailable.

Mail actions are the server-mutation examples: request fields are typed, IDs are ownership-checked against active cache rows, delete and expunge require confirmation, archive/delete special folders are backend-resolved, retries are bounded, and local cache updates occur only after the live adapter accepts the remote mutation. Delete-to-trash is the default delete path; direct hard delete is not a command variant.

Composer commands are the mutating examples: request fields are typed, HTML is accepted only through sanitizer-backed frontend output plus backend `DraftHtml`/`SanitizedHtml` wrappers, rich formatting is constrained to the reviewed TipTap/sanitizer whitelist, attachments are staged by bytes rather than paths, staged ids are ownership-checked, duplicate recipients are normalized before persistence/send, recent-recipient lookup is account-scoped and read-only, and send calls the existing SMTP boundary plus pending Sent APPEND scheduler path instead of exposing credentials or transport details to React.

Diagnostics commands are the local-operations examples. `export_diagnostics_bundle` accepts only `userRequested=true` and the boolean synthetic-fixture flag, owns the native save dialog in Rust, rejects overwrite, writes only a redacted JSON bundle, and returns only display-safe export metadata. It must not accept frontend paths, raw log queries, database table names, account ids, message ids, subjects, bodies, attachment ids, credential refs, or arbitrary include/exclude maps. `get_diagnostics_health` accepts no request fields and returns only aggregate cache/migration status, sync retry/category history, attachment storage health counts, and remediation codes; it must not accept frontend paths, filters, ids, table names, log queries, or include maps.

Cross-command denial coverage lives in `src-tauri/tests/ipc_denial_paths.rs`. Keep it focused on public command-level entry points, disposable SQLite databases, temp app-data directories, and fake stores; it should fail before any new IPC surface accepts invalid IDs, raw SQL-shaped text, frontend paths, unowned scope, oversized payloads, or unauthorized server-affecting actions.

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
- `load_message_page` denial paths for missing scope, mixed unified/account scope, unowned folders, invalid IDs, and raw SQL/HTML-shaped cursor text.
- `search_mail` denial paths for raw SQL-shaped queries, raw HTML, URL/path-shaped queries, unsupported advanced filters, invalid account IDs, unknown scoped folders, and missing FTS index reporting.
- Reader attachment denial paths for missing user intent, invalid IDs, unowned scope, missing stored content, persisted path traversal, hash mismatch, size/type mismatch, unpreviewable types, spoofed selected-save extensions, and overwrite refusal.
- Remote-content preference denial paths for missing user intent, invalid IDs, unknown or unowned account/folder/message scope, and missing usable sender addresses before persistence.
- Mail-action denial paths for missing user intent, invalid IDs, unknown or unowned account/folder/message scope, missing delete/expunge confirmation, missing special-use targets, transient retry metadata, and unauthorized action rejection before fake-store mutation.
- External-link denial paths for missing user intent, scripts, local paths, custom schemes, embedded credentials, private/local hosts, unsupported `mailto` fields, encoded control characters, and redacted unsafe URL errors before opener calls.
- Composer denial paths for unsafe attachment MIME/content, invalid recipient addresses, invalid ids, unknown staged attachments, unsafe storage-relative paths, duplicate recipient normalization, recent-recipient scope/search behavior, sanitized rich-HTML handling, and send validation before SMTP work.
- Account-management denial paths for unsupported provider/auth combinations, invalid account ids, invalid hosts, plaintext transport, protocol-injection secrets, oversized account fields, direct Gmail/Microsoft secret-handoff rejection, live generic login failure before credential persistence, credential-store failure before metadata persistence, unknown account removal, account settings invalid/unknown/oversized field rejection, and redacted auth status.
- OAuth loopback denial paths for unsupported generic-provider setup, denied consent/cancellation, callback state mismatch, timeout, invalid grant, provider errors, missing refresh token, and non-OAuth reauthentication targets.
- Diagnostics denial/unit paths for missing user intent, unsafe destination shape in the backend helper, overwrite refusal, log redaction, private mailbox-data exclusion from exported bundles, health aggregation, attachment-health path validation, and private-data exclusion from health output.
- `src-tauri/tests/ipc_denial_paths.rs` integration coverage for command-level invalid IDs, frontend path denial, raw SQL/HTML search and page-cursor input, unowned account/folder/message/attachment scope, remote-content missing-intent/scope denial, external-link missing-intent/unsafe-URL denial, oversized search/composer/recent-recipient/attachment/account payloads, account settings denial paths, account credential failure, reader attachment source denial, and unauthorized mail actions rejected before fake-store mutation.

`pnpm check` runs the validation tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- Tauri command permissions: https://v2.tauri.app/security/permissions/
- Tauri command scopes: https://v2.tauri.app/security/scope/
