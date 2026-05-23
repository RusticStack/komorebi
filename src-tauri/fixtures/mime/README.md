# MIME Fixture Corpus

These `.eml` files are synthetic parser fixtures. They are not copied from real
mailboxes and must stay free of real addresses, credentials, tokens, or private
message content.

Coverage:

- `encoded-portuguese-japanese-alternative.eml`: RFC 2047 encoded headers,
  Portuguese text, Japanese/non-ASCII display names, ISO-8859-1 plain text, and
  UTF-8 HTML/plain alternatives.
- `nested-related-attachments.eml`: nested `multipart/mixed` +
  `multipart/related`, CID inline image metadata, and sender attachment filename
  traversal.
- `malformed-boundary.eml`: missing multipart closing boundary with recoverable
  plain text.
- `phishing-xss-html-alternative.eml`: phishing links, remote resources, script
  tags, inline event handlers, and a plain-text fallback.

The production MIME normalization tests load this directory from
`CARGO_MANIFEST_DIR`, so fixture additions should be accompanied by parser
assertions in `src-tauri/src/mime_normalization.rs`.
