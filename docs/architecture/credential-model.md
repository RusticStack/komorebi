# Credential Model

Status: accepted and implemented for the Windows-first secret-store layer plus typed account-management command use.
Last reviewed: 2026-05-23.

## Decision

Komorebi will store mail-account secrets in Windows Credential Manager through the Rust backend. Windows Credential Manager entries are user-scoped and protected by the Windows credential subsystem/DPAPI. The frontend must never receive raw passwords, refresh tokens, access tokens, credential blobs, or decrypted secret material.

The secret-store implementation lives in `src-tauri/src/credentials.rs`. It uses a narrow Rust abstraction over the Windows `CredWriteW`, `CredReadW`, and `CredDeleteW` APIs through `windows-sys`, so call sites do not depend on raw Win32 API shape. Generic account setup, generic reauthentication, removal, and redacted auth-status commands live in `src-tauri/src/account_management.rs`; live generic setup login validation lives in `src-tauri/src/account_connection.rs`; Gmail/Microsoft loopback setup and OAuth reauthentication live in `src-tauri/src/oauth_loopback.rs`.

No credential storage may be implemented with browser storage, Tauri Store, SQLite, plain files, logs, diagnostics bundles, or environment variables. `tauri-plugin-store` may hold non-secret preferences only.

## Why This Backend

Komorebi targets Windows 10/11. Using Windows Credential Manager keeps long-lived mail secrets bound to the current Windows user instead of inventing a vault-password lifecycle inside the app.

Tauri Stronghold remains an approved fallback if Windows-only scope changes or if an explicit product decision requires an app-managed encrypted vault. It is not the first choice for the Windows-only implementation because it introduces a Stronghold password/key-derivation flow, unlock UX, and recovery policy that the product has not specified yet.

## Plaintext Rules

- Plaintext account passwords are forbidden at rest.
- Plaintext OAuth refresh tokens are forbidden at rest.
- Plaintext app passwords are forbidden at rest.
- Plaintext access tokens are memory-only unless a future design explicitly justifies persisting them in Windows Credential Manager with an expiry.
- Secrets must not be copied into React state, localStorage/sessionStorage, IndexedDB, Tauri Store, SQLite rows, telemetry, crash reports, diagnostics, screenshots, or logs.
- Any debug output that includes provider responses must redact tokens, authorization headers, usernames where practical, email addresses where practical, subject lines, and message bodies.

## Account Metadata Records

The local SQLite `accounts` record stores only non-secret account metadata and credential references:

| Field | Stored in SQLite | Secret | Notes |
| --- | --- | --- | --- |
| `id` | yes | no | Stable local UUID. |
| `provider_kind` | yes | no | `gmail`, `microsoft`, or `generic_imap_smtp`. |
| `display_email` | yes | no | User-visible address; treat as private in logs. |
| `display_name` | yes | no | Optional local label. |
| `imap_host`, `imap_port`, `imap_security` | yes | no | Generic-provider connection metadata. |
| `smtp_host`, `smtp_port`, `smtp_security` | yes | no | Generic-provider connection metadata. |
| `username` | yes | no | Login identifier; treat as private in logs. |
| `auth_kind` | yes | no | `oauth2`, `app_password`, or `manual_password`. |
| `credential_ref` | yes | no | Opaque reference to Windows Credential Manager target name. |
| `signature_html` | yes | no | Sanitized account signature, not a credential. |
| `auth_status` | yes | no | Redacted auth state such as `connected` or `reconnect_needed`. |
| `created_at`, `updated_at`, `last_auth_at`, `removed_at` | yes | no | Local timestamps. |

SQLite must not store `password`, `app_password`, `access_token`, `refresh_token`, `client_secret`, `authorization_code`, `authorization_header`, token response JSON, or provider cookies.

The current account table is defined in `src-tauri/migrations/001_initial_schema.sql` and summarized in `docs/architecture/schema.md`.

## Credential Manager Entries

Credential target names must be deterministic, per-account, and namespaced:

```text
Komorebi/mail-account/{account_id}/oauth-refresh
Komorebi/mail-account/{account_id}/oauth-access
Komorebi/mail-account/{account_id}/generic-password
```

Rules:

- `account_id` is the local UUID, never the raw email address.
- `oauth-refresh` stores provider refresh tokens for Gmail and Microsoft OAuth2.
- `oauth-access` is optional and may store an access token only with an expiry timestamp in the protected payload. Prefer memory-only access tokens.
- `generic-password` stores a provider app password or manual password only for compatible generic IMAP/SMTP providers. Gmail and Microsoft must use OAuth2/XOAUTH2, as defined in `docs/architecture/provider-auth.md`.
- Credential payloads are serialized by Rust only. React sends generic-provider setup secrets once over a typed command and never receives the stored value back. Gmail/Microsoft setup sends only account metadata plus OAuth client ID; the backend-owned loopback command receives the authorization code, exchanges it, and stores the refresh token without exposing it to React.
- The Windows credential username may be `provider_kind:account_id`; the display email stays in SQLite metadata.

## Runtime Access Flow

1. React submits generic IMAP/SMTP setup data, including the provider password/app-password, to `create_account`; Gmail/Microsoft setup submits only metadata plus OAuth client ID to `start_oauth_account_setup`.
2. Rust validates provider/auth mode and rejects plaintext persistence outside the credential backend. Public `create_account` and `reauth_account` reject Gmail/Microsoft refresh-token handoff so the loopback command is the only frontend-callable major-provider path.
3. For Gmail/Microsoft, Rust opens the system browser, receives the loopback callback, validates state, exchanges the authorization code, and extracts the refresh token inside the backend.
4. For generic IMAP/SMTP setup, Rust runs the backend-owned live IMAP LOGIN and SMTP AUTH check over TLS or STARTTLS before persisting the secret. This check returns only a redacted success/error status to React.
5. Rust writes the approved secret to Windows Credential Manager.
6. Rust writes the SQLite account metadata with `credential_ref` only after the credential write succeeds.
7. Sync/send code resolves credentials by `account_id` inside Rust.
8. Rust refreshes OAuth tokens inside Rust and stores new refresh tokens only through the credential backend, following `docs/architecture/token-lifecycle.md`.
9. Rust returns auth status, errors, and expiry metadata to React, never the secret value.

## Clearing And Rotation

Account removal must run as a single typed backend workflow:

1. Stop sync workers for the account.
2. Clear any in-memory access token or SMTP/IMAP auth cache.
3. Revoke OAuth refresh tokens with the provider when provider support exists.
4. Delete all Windows Credential Manager entries for the account.
5. Delete or retain local mail cache according to the user's explicit cache-removal choice.
6. Remove account metadata, folder sync state, drafts, and signatures only after secret deletion has succeeded or after the user confirms a documented retry/failure path.
7. Write a redacted local audit event such as `account_removed`, never the email password or token.

Credential rotation must preserve the previous credential until replacement storage succeeds. Once live reauth verification is wired, it must also verify provider acceptance before marking the account connected.

Current `create_account` and `test_account_connection` perform live generic IMAP/SMTP setup login checks in Rust without returning secrets, protocol payloads, or credential refs to React. Current `reauth_account` performs validated generic credential-slot replacement and marks the local account connected; it does not yet run the live generic login check, so credential rotation must not be described as proof of provider acceptance. Current `start_oauth_account_reauth` performs backend-owned Gmail/Microsoft authorization-code exchange before rotating the refresh token.

Token refresh, reconnect-needed status, provider revocation handling, and the provider OAuth HTTP client are defined in `docs/architecture/token-lifecycle.md` and enforced by `src-tauri/src/token_lifecycle.rs` plus `src-tauri/src/oauth.rs`.

## Frontend And IPC Boundaries

- Every account setup or reauthentication command must evaluate the provider/auth policy in `src-tauri/src/auth.rs` before storing credentials or account metadata.
- No frontend API may expose `get_password`, `get_token`, `export_secret`, raw credential target names, or arbitrary credential reads.
- Current account commands are narrowly typed: `create_account`, `reauth_account`, `start_oauth_account_setup`, `start_oauth_account_reauth`, `remove_account`, `get_account_auth_status`, and `test_account_connection`.
- `get_account_auth_status` may return booleans, expiry times, provider names, and redacted error codes only.
- `test_account_connection` performs a live generic IMAP/SMTP setup login check without persistence. It validates TLS/STARTTLS configuration and provider/auth policy before opening network connections.
- IPC validation must reject unknown account IDs, unsupported provider/auth combinations, oversized payloads, arbitrary credential target names, and raw SQL.

## Diagnostics

Diagnostics may include:

- Provider kind.
- Auth kind.
- Credential presence as a boolean.
- Token expiry timestamp.
- Redacted provider error code.
- Last successful auth/sync timestamp.

Diagnostics must exclude:

- Refresh tokens.
- Access tokens.
- Passwords and app passwords.
- Authorization codes.
- Authorization headers.
- Full provider token responses.
- Message bodies and subject lines.

## References

- Tauri Store is a persistent key-value store for application state and is not approved for Komorebi secrets: https://v2.tauri.app/plugin/store/
- Tauri Stronghold is an approved fallback vault option if Komorebi needs app-managed encrypted storage later: https://v2.tauri.app/plugin/stronghold/
- Microsoft Credential Manager lets Windows users view/delete saved application credentials: https://support.microsoft.com/en-us/windows/credential-manager-in-windows-1b5c916a-6a16-889f-8581-fc16e8165ac0
- Microsoft `CredWrite` creates or modifies a credential in the current user's credential set: https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritea
