# MIME Normalization

Status: inbound parser boundary implemented in Rust.
Last reviewed: 2026-05-23.

## Current Scope

The code source of truth is `src-tauri/src/mime_normalization.rs`.

Komorebi normalizes raw inbound RFC822/RFC5322 message bytes with the Rust `mail-parser` crate before message bodies or attachment metadata become cache records. The parser boundary is backend-only and produces schema-shaped records:

- `RemoteMessage` for the `messages` table fields owned by sync.
- `ParsedAttachmentRecord` for the `attachments` table fields that can be derived before attachment bytes are written to app storage.
- `MimeParseWarning` entries for accepted-but-risky parts, such as decoding problems or attachments that fail storage safety checks.

`parse_remote_message_from_raw_mime` maps fetched RFC822/RFC5322 bytes into a `RemoteMessage` without persisting attachment metadata. The live IMAP adapter uses this narrower helper for protocol fetches. `persist_parsed_inbound_mime` writes the normalized message and attachment metadata to a migrated SQLite database in one transaction. It does not write attachment bytes to disk.

This task implements normalization, metadata persistence, a live-adapter message mapping helper, and fixture tests. It does not persist attachment files or expose parser commands to React.

## Message Record Rules

The normalized `RemoteMessage` preserves IMAP identity separately from RFC identity:

- `uid_validity` and `uid` come from the selected folder state.
- `rfc_message_id` comes from the decoded `Message-ID` header.
- `provider_message_id` remains `NULL` unless a provider adapter supplies one.
- The stable local message id is generated from account id, folder id, UIDVALIDITY, and UID through the sync cache id algorithm.

Decoded headers populate:

- `subject`
- `sender_name`
- `sender_email`
- `to_addresses_json`
- `cc_addresses_json`
- `bcc_addresses_json`
- `reply_to_addresses_json`
- `in_reply_to`
- `references_json`
- `date_received`

Address JSON records are arrays of `{ "name": string | null, "address": string | null }`. Encoded words, non-ASCII names, and charset conversions are decoded before serialization.

Body rules:

- `body_text` comes from the preferred text/plain body.
- `body_html` comes from the preferred text/html body.
- Multipart alternatives may populate both fields.
- `snippet` comes from the parser's body preview.
- `size_bytes` is the raw message byte length.
- HTML is not sanitized here; display sanitization remains the frontend rendering boundary in `src/mail/rendering/`.

## Attachment Record Rules

Attachment metadata is normalized without trusting sender-controlled paths:

- `id` is a stable backend-generated attachment id derived from account, folder, UIDVALIDITY, UID, part index, content id, sender filename, declared MIME type, and content hash.
- `message_id` matches the sync cache's stable local message id.
- `sender_filename` is preserved only as display metadata.
- `display_filename` is sanitized and path-stripped.
- `content_id` is stripped of wrapping angle brackets for CID lookup.
- `disposition` is `inline` when the MIME part is inline, has inline binary content, or has a content id; otherwise it is `attachment`.
- `declared_content_type` is lower-cased from MIME headers with `application/octet-stream` fallback.
- `sniffed_content_type`, `preview_kind`, `size_bytes`, and `sha256` are computed from decoded part bytes.

The parser runs `validate_attachment_for_storage` for every parsed attachment. Attachments that fail storage validation are still represented as metadata with a warning, but future storage/download code must not write or preview those bytes without passing the attachment safety boundary.

Attachment metadata persistence is idempotent by stable attachment id. `storage_relative_path` stays `NULL` until a later download/storage worker writes bytes under an app-generated path.

## Malformed Input

The boundary rejects empty messages and invalid account/folder/UID identity. Parser-level failures return `MimeParseError::Unparseable`.

Malformed-but-parseable messages are accepted when stable records can still be produced. Parts marked by `mail-parser` as having encoding problems produce `MimeParseWarning::EncodingProblem` instead of crashing or discarding the entire message.

## Tests

`src-tauri/src/mime_normalization.rs` tests cover:

- RFC2047 encoded headers.
- Non-ASCII address display names.
- Charset decoding for ISO-8859-1 and UTF-8 bodies.
- Multipart plain-text and HTML alternatives.
- Reusable `.eml` fixture-file corpus in `src-tauri/fixtures/mime/`.
- Portuguese body text and Japanese/non-ASCII names.
- Nested multipart structures.
- Inline CID image parts.
- Attachment metadata records with path-like sender filenames.
- Stable attachment ids across repeated parsing.
- Phishing links and XSS HTML payload preservation for the frontend rendering boundary.
- Idempotent SQLite persistence of normalized message rows and attachment metadata.
- Missing multipart closing boundary with a recoverable body.
- Empty message, invalid UID, and invalid account/folder id denial paths.
- Raw-message-to-`RemoteMessage` mapping used by the live IMAP adapter.

No test uses a real mailbox, real account, or real user message. Fixture files are synthetic and documented in `src-tauri/fixtures/mime/README.md`.

## Not Implemented

- Live-fetched attachment metadata persistence through `persist_parsed_inbound_mime`.
- Persisting attachment bytes to app-generated storage paths.
- Inline image resolution in the React reader.

Outbound SMTP MIME generation is a separate backend boundary documented in `docs/architecture/smtp-send.md`.

## Documentation Sync

Keep this document synchronized with:

- `src-tauri/src/mime_normalization.rs`
- `src-tauri/src/imap_sync.rs`
- `src-tauri/src/imap_live.rs`
- `src-tauri/src/attachment_safety.rs`
- `docs/architecture/sync-state-machine.md`
- `docs/architecture/attachment-safety.md`
- `docs/architecture/message-rendering.md`
- `docs/architecture/schema.md`
- `AGENTS.md`
- `TODO.md`
