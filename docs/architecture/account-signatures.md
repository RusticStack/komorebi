# Account Signatures

Status: per-account signature storage, sanitization, deterministic insertion, and settings editor component implemented.
Last reviewed: 2026-05-22.

## Current Scope

The backend source of truth is `src-tauri/src/account_signatures.rs`.

The frontend sources of truth are:

- `src/mail/signatures/signatureHtml.ts`
- `src/settings/AccountSignatureSettings.tsx`

Komorebi stores one optional sanitized HTML signature per account in `accounts.signature_html`. Signatures are account metadata, not credentials. They are plaintext local SQLite content and must not be described as encrypted.

The app shell now has a Settings entry and the signature settings component supports account selection, editing, clearing, sanitized preview, and save callbacks. The current app still has no account setup or frontend-callable persistence command, so the production wiring from real account rows to this component remains part of the account setup/IPC work.

## Sanitization

Backend sanitization uses the Rust `ammonia` allowlist. Frontend sanitization uses DOMPurify with the same narrow intent.

Allowed signature HTML is limited to simple inline/block formatting and safe links:

- Text structure: `p`, `div`, `br`, `blockquote`, `ul`, `ol`, `li`.
- Formatting: `strong`, `b`, `em`, `i`, `u`, `span`.
- Links: `a[href]` with only `http`, `https`, and `mailto`.

The boundary strips or rejects unsafe behavior:

- scripts and script-like URLs
- inline event handlers
- styles
- images and remote resources
- embedded credentials in URLs
- relative URLs
- form, iframe, SVG, MathML, media, and executable-style content

Saved links get `rel="noreferrer noopener nofollow"` and no `target`.

## Persistence

`update_account_signature` validates:

- account id shape
- account exists and is not removed
- signature payload size
- sanitized output size

Empty or fully stripped signatures store `NULL` in `accounts.signature_html`.

The account schema already included `signature_html` in migration `001_initial_schema.sql`, so no new migration is required for this task.

## Composer Insertion

Managed composer insertion uses these HTML markers:

```html
<!-- komorebi-signature:start -->
<!-- komorebi-signature:end -->
```

`insert_signature_into_body_html` removes an existing managed block before appending the current account signature. This keeps account switching and settings edits deterministic and prevents duplicate signatures.

For text-only sends, `insert_signature_into_body_text` derives a plain-text signature and appends it with the standard `-- ` delimiter.

`src-tauri/src/smtp_send.rs` loads `accounts.signature_html` with the sender account and applies the backend insertion boundary during MIME composition. Stored legacy/fixture values are sanitized again before insertion.

## Settings Editor

`AccountSignatureSettings` is a controlled settings component:

- account selection loads that account's current signature
- edits are sanitized before save
- Clear intentionally saves `NULL`
- preview renders through `SafeEmailBody`, keeping the existing raw-HTML sink rule intact

The component does not call Tauri directly. Future IPC wiring must use a narrow Rust command around `update_account_signature`, account ownership checks, and a reviewed capability grant.

## Tests

Rust tests cover:

- storing sanitized per-account signatures
- clearing signatures to `NULL`
- invalid and unknown account denial
- predictable insert/replace/remove behavior
- plain-text signature derivation
- SMTP MIME composition with sanitized account signatures

Frontend tests cover:

- DOMPurify signature sanitization
- unsafe link/resource stripping
- single managed composer block insertion/replacement/removal
- settings editor account switching, sanitization before save, and intentional clear
- accountless app Settings entry without seeded account data

No test uses a real account, live mailbox, or real credential.

## Documentation Sync

Keep this document synchronized with:

- `src-tauri/src/account_signatures.rs`
- `src-tauri/src/smtp_send.rs`
- `src/mail/signatures/signatureHtml.ts`
- `src/settings/AccountSignatureSettings.tsx`
- `docs/architecture/schema.md`
- `docs/architecture/smtp-send.md`
- `docs/architecture/ipc-and-mutation-boundaries.md`
- `README.md`
- `AGENTS.md`
- `TODO.md`
