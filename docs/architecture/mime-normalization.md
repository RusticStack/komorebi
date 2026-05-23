# MIME Normalization

Status: inbound parser boundary and live-fetch persistence implemented in Rust.
Last reviewed: 2026-05-23.

## Current Scope

The code source of truth is `src-tauri/src/mime_normalization.rs`.

Komorebi normalizes raw inbound RFC822/RFC5322 message bytes with the Rust `mail-parser` crate before message bodies or attachment metadata become cache records. The parser boundary is backend-only and produces schema-shaped records:

- `RemoteMessage` for the `messages` table fields owned by sync.
- `ParsedAttachmentRecord` for the `attachments` table fields plus decoded bytes only when the part passes attachment storage validation.
- `MimeParseWarning` entries for accepted-but-risky parts, such as decoding problems or attachments that fail storage safety checks.

`parse_inbound_mime` maps fetched RFC822/RFC5322 bytes into a `RemoteMessage`, attachment metadata, storage-safe decoded attachment bytes, and parser warnings. The live IMAP adapter calls this boundary for `BODY.PEEK[]` responses and hands the schema-shaped records to `SqliteSyncCache`, which persists message bodies, attachment metadata, warning JSON, storage-safe attachment files, and FTS-indexed content in the normal sync path. `persist_parsed_inbound_mime` remains a direct test/helper path for writing the same normalized records to SQLite without an app-data storage root.

This task implements normalization, metadata persistence, live-adapter raw MIME persistence, warning storage, storage-safe inbound attachment files, and fixture tests. It does not expose parser commands or local attachment paths to React.

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
- `storage` contains decoded bytes only when the part passes the attachment storage allowlist, declared/sniffed type checks, size checks, and hash checks.

The parser runs `validate_attachment_for_storage` for every parsed attachment. Attachments that fail storage validation are still represented as metadata with a warning, but they do not carry bytes into the sync cache and are never written to disk.

Attachment metadata persistence is idempotent by stable attachment id. When `SqliteSyncCache` has an app-data directory, safe attachment bytes are written under app-generated relative paths shaped as `attachments/{account_id}/{folder_id}/{message_id}/{attachment_id}.{ext}` and `attachments.storage_relative_path` is set. Without an app-data directory, metadata persistence remains valid and `storage_relative_path` stays `NULL`.

Live sync replaces a live-fetched message's attachment metadata with the current parsed set when the message was derived from raw MIME. It cleans replaced, UIDVALIDITY-reset, and expunged attachment files best-effort. Fixture-only `RemoteMessage` values that are already normalized keep the older fake-server behavior and do not delete attachment rows.

## Malformed Input

The boundary rejects empty messages and invalid account/folder/UID identity. Parser-level failures return `MimeParseError::Unparseable`.

Malformed-but-parseable messages are accepted when stable records can still be produced. Parts marked by `mail-parser` as having encoding problems produce `MimeParseWarning::EncodingProblem` instead of crashing or discarding the entire message.

Accepted warnings are serialized into `messages.mime_parse_warnings_json` as checked JSON. This keeps risky-but-parseable state available for future diagnostics without logging private message content.

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
- Storage-safe attachment byte carry, generated storage path persistence, hash checks, and expunge cleanup through sync-cache tests.
- Missing multipart closing boundary with a recoverable body.
- Empty message, invalid UID, and invalid account/folder id denial paths.
- Raw-message parsing used by the live IMAP adapter.
- Live IMAP fixture sync that persists parsed body text, safe attachment bytes, attachment metadata, parser warnings, and FTS-searchable terms from `BODY.PEEK[]`.

No test uses a real mailbox, real account, or real user message. Fixture files are synthetic and documented in `src-tauri/fixtures/mime/README.md`.

## Not Implemented

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
