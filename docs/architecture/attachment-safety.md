# Attachment Storage And Preview Safety

Last updated: 2026-05-22

## Current State

Komorebi has no live attachment download, preview, or open commands yet. The attachment safety layer exists first so sync, MIME normalization, and reader UI work cannot persist or preview untrusted attachment content with ad hoc path handling.

The code source of truth is `src-tauri/src/attachment_safety.rs`.

## Storage Rules

Attachment storage paths are generated only by Rust. React, MIME sender filenames, `Content-Disposition` filenames, and provider metadata must never become local filesystem paths.

The current path shape is:

```text
<tauri-app-data>/attachments/<account_id>/<folder_id>/<message_id>/<attachment_id>.<type-extension>
```

Path segments are validated internal IDs with only ASCII letters, digits, hyphen, and underscore. The stored filename is derived from the backend attachment id and a canonical extension for the validated content type.

Sender filenames are display-only metadata. They are sanitized for display by stripping path components, replacing Windows-reserved filename characters, removing controls, trimming leading/trailing dots, and bounding length. Sanitized display filenames are not used for storage, preview URLs, download destinations, or opener commands.

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

`src-tauri/src/mime_normalization.rs` applies this validation while parsing inbound MIME parts. A parsed attachment that fails storage validation may remain as metadata with a warning, but future storage/download code must not write or preview the bytes unless the attachment safety boundary accepts them.

Any expansion of the allowlist must add tests for declared type, sniffed bytes, storage extension, preview behavior, and unsafe-open denial.

## Preview Rules

Preview is opt-in only. The app must never preview an attachment as a side effect of selecting a message or focusing an attachment row.

Previewable types are currently limited to:

- Plain text.
- Raster images: PNG, JPEG, GIF, WebP.

PDF and ZIP can be stored when validated but are not previewable in the current safety model. SVG and HTML are rejected instead of previewed because they can execute active content in browser-like surfaces.

`AUTO_OPEN_ATTACHMENTS` is false and must stay false. Opening a file externally is not implemented. If an open/save flow is added, it needs explicit user intent, a typed Rust command, path/type validation, and narrow Tauri capabilities. Do not add the opener plugin broadly to the main webview.

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

`pnpm check` runs these tests through `cargo test --manifest-path src-tauri/Cargo.toml --all-features`.

## References

- OWASP File Upload Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/File_Upload_Cheat_Sheet.html
- OWASP Path Traversal: https://owasp.org/www-community/attacks/Path_Traversal
