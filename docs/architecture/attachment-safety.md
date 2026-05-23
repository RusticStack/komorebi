# Attachment Storage And Preview Safety

Last updated: 2026-05-23

## Current State

Komorebi has no reader-facing attachment preview, save, export, or open commands yet. Live MIME sync persists storage-safe inbound attachment bytes and inert metadata from fetched messages, and the reader can display that local-cache metadata, but it does not receive storage paths or preview URLs. The TipTap composer can stage user-selected outbound attachment bytes through a typed Rust command. The attachment safety layer exists so sync, MIME normalization, reader UI, and composer staging cannot persist or preview untrusted attachment content with ad hoc path handling.

The storage/preview code source of truth is `src-tauri/src/attachment_safety.rs`. The safe reader projection is `src-tauri/src/mail_shell.rs`. Composer staging is implemented in `src-tauri/src/composer.rs`.

## Storage Rules

Attachment storage paths are generated only by Rust. React, MIME sender filenames, `Content-Disposition` filenames, and provider metadata must never become local filesystem paths.

The current path shape is:

```text
<tauri-app-data>/attachments/<account_id>/<folder_id>/<message_id>/<attachment_id>.<type-extension>
```

The persisted database value is the app-data-relative path:

```text
attachments/<account_id>/<folder_id>/<message_id>/<attachment_id>.<type-extension>
```

Composer-staged outbound files use:

```text
<tauri-app-data>/composer-attachments/<account_id>/<draft_id>/<attachment_id>.<type-extension>
```

Inbound sync writes decoded attachment bytes only after `validate_attachment_for_storage` accepts the declared/sniffed type, byte count, and safety policy. Unsafe inbound parts remain metadata-only with parser warnings and `storage_relative_path = NULL`. `SqliteSyncCache` writes safe inbound bytes under generated paths, records the SHA-256 and relative path, and cleans replaced, UIDVALIDITY-reset, expunged, and account-cache-deleted files best-effort.

The WebView provides selected composer bytes through the browser File API, not a local filesystem path. Rust validates type/size, generates the attachment id/path, stores metadata in `composer_attachments`, streams those files during `send_composer_email`, and deletes staged files best-effort after discard or confirmed send.

Composer streaming treats `composer_attachments.storage_relative_path` as untrusted persisted state. Before a staged file is opened, Rust rejects empty, absolute, prefixed, root, or parent-directory paths, canonicalizes the app-data root and target file, and requires the final path to remain under app data.

Path segments are validated internal IDs with only ASCII letters, digits, hyphen, and underscore. The stored filename is derived from the backend attachment id and a canonical extension for the validated content type.

Sender filenames are display-only metadata. They are sanitized for display by stripping path components, replacing Windows-reserved filename characters, removing controls, trimming leading/trailing dots, and bounding length. Sanitized display filenames are not used for storage, preview URLs, download destinations, or opener commands.

The frontend mail shell may receive only attachment id, sanitized display filename, MIME type, size, and disposition. It must not receive `storage_relative_path`, local paths, hashes, sender filename storage hints, preview URLs, or opener/download targets through `load_mail_shell`.

Attachment bytes are not encrypted at rest in the current implementation. This is documented in `docs/architecture/local-cache-encryption.md`; do not claim attachment storage is encrypted until a reviewed encryption design covers attachment files and preview materialization.

## Size And Type Limits

Current limits:

- Maximum stored attachment payload: 25 MiB.
- Maximum preview payload: 5 MiB.
- Maximum sanitized display filename: 255 bytes.

Current storage allowlist:

- `text/plain`
- `image/png`
- `image/jpeg`
- `image/gif`
- `image/webp`
- `application/pdf`
- `application/zip`

The safety layer sniffs content bytes and compares them to the declared MIME type when a declared type is present. `application/octet-stream` may be accepted only when sniffing identifies an allowed type. Explicitly dangerous declared types such as HTML, SVG, JavaScript, shell/batch/MSI/downloadable executable types, and legacy Office binary types are rejected.

`src-tauri/src/mime_normalization.rs` applies this validation while parsing inbound MIME parts. A parsed attachment that fails storage validation may remain as metadata with a warning, but it does not carry bytes into sync storage and must not be previewed or saved later unless a future command revalidates user intent, ownership, path, size, and type.

Any expansion of the allowlist must add tests for declared type, sniffed bytes, storage extension, preview behavior, and unsafe-open denial.

## Preview Rules

Preview is opt-in only. The app must never preview an attachment as a side effect of selecting a message or focusing an attachment row.

Previewable types are currently limited to:

- Plain text.
- Raster images: PNG, JPEG, GIF, WebP.

PDF and ZIP can be stored when validated but are not previewable in the current safety model. SVG and HTML are rejected instead of previewed because they can execute active content in browser-like surfaces.

`AUTO_OPEN_ATTACHMENTS` is false and must stay false. Opening a file externally is not implemented. If an open/save flow is added, it needs explicit user intent, a typed Rust command, path/type validation, and narrow Tauri capabilities. Do not add the opener plugin broadly to the main webview.

Current Tauri capability tests deny attachment open, preview, save, download, and archive-extraction command names as well as frontend opener APIs. Adding one of these surfaces requires updating the capability tests and this document with the reviewed policy.

## Tests

`src-tauri/src/attachment_safety.rs` tests cover:

- Stored type validation from sniffed bytes and declared MIME type.
- Oversized attachment rejection.
- Dangerous and mismatched content-type rejection.
- App-generated storage paths that ignore sender filenames.
- Path traversal rejection in IDs.
- Display filename sanitization.
- Preview user-intent, type allowlist, and size-limit checks.
- Auto-open remaining forbidden.
- App-data-relative inbound attachment storage paths that ignore sender filenames.

`src-tauri/src/imap_sync.rs` and `src-tauri/src/imap_live.rs` tests cover live-fetched inbound attachment byte storage, malicious sender filenames not affecting storage, unsafe parts staying metadata-only, hash/size persistence, and cleanup after expunge.

`src-tauri/src/mail_shell.rs` tests cover safe attachment metadata projection, display filename sanitization in the reader snapshot, and storage-path redaction from serialized shell data.

`src-tauri/src/composer.rs` tests cover composer attachment staging under generated app-data paths, malicious display filenames not creating or overwriting sibling files, corrupted stored relative paths being rejected before file open/streaming, unsafe type denial, staged attachment streaming during fake SMTP send, and cleanup after confirmed send.

`src-tauri/capabilities/capabilities.test.ts` covers opener/download/preview/extraction denial by absence until typed attachment commands exist.

`pnpm check` runs these tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- OWASP File Upload Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html
- OWASP Path Traversal: https://owasp.org/www-community/attacks/Path_Traversal
