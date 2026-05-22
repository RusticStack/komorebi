# Credential Model

Status: accepted for the Windows-first implementation.
Last reviewed: 2026-05-22.

## Decision

Komorebi will store mail-account secrets in Windows Credential Manager through the Rust backend. Windows Credential Manager entries are user-scoped and protected by the Windows credential subsystem/DPAPI. The frontend must never receive raw passwords, refresh tokens, access tokens, credential blobs, or decrypted secret material.

The first implementation should use a narrow Rust credential-store abstraction so the call sites do not depend on one crate or Win32 API shape. Acceptable implementation choices are:

- A reviewed Rust wrapper over Windows Credential Manager.
- Direct use of the Windows `CredWrite`, `CredRead`, and `CredDelete` APIs through the `windows` crate.

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
| `created_at`, `updated_at`, `last_auth_at` | yes | no | Local timestamps. |

SQLite must not store `password`, `app_password`, `access_token`, `refresh_token`, `client_secret`, `authorization_code`, `authorization_header`, token response JSON, or provider cookies.

## Credential Manager Entries

Credential target names must be deterministic, per-account, and namespaced:

```text
Komorebi/mail-account/{account_id}/oauth-refresh
Komorebi/mail-account/{account_id}/oauth-access
Komorebi/mail-account/{account_id}/generic-password
```

Rules:

- `account_id` is the local UUID, never the raw email address.
- `oauth-refresh` stores the provider refresh token when OAuth2 is implemented.
- `oauth-access` is optional and may store an access token only with an expiry timestamp in the protected payload. Prefer memory-only access tokens.
- `generic-password` stores a provider app password or manual password only for compatible generic IMAP/SMTP providers. Gmail and Microsoft must use OAuth2/XOAUTH2 or remain unsupported until implemented, as defined in `docs/architecture/provider-auth.md`.
- Credential payloads are serialized by Rust only. React sends setup input once over a typed command and never receives the stored value back.
- The Windows credential username may be `provider_kind:account_id`; the display email stays in SQLite metadata.

## Runtime Access Flow

1. React submits account setup data to a typed Rust command.
2. Rust validates provider/auth mode and rejects plaintext persistence outside the credential backend.
3. Rust writes the secret to Windows Credential Manager.
4. Rust writes the SQLite account metadata with `credential_ref` only after the credential write succeeds.
5. Sync/send code resolves credentials by `account_id` inside Rust.
6. Rust refreshes OAuth tokens inside Rust and stores new refresh tokens only through the credential backend, following `docs/architecture/token-lifecycle.md`.
7. Rust returns auth status, errors, and expiry metadata to React, never the secret value.

## Clearing And Rotation

Account removal must run as a single typed backend workflow:

1. Stop sync workers for the account.
2. Clear any in-memory access token or SMTP/IMAP auth cache.
3. Revoke OAuth refresh tokens with the provider when provider support exists.
4. Delete all Windows Credential Manager entries for the account.
5. Delete or retain local mail cache according to the user's explicit cache-removal choice.
6. Remove account metadata, folder sync state, drafts, and signatures only after secret deletion has succeeded or after the user confirms a documented retry/failure path.
7. Write a redacted local audit event such as `account_removed`, never the email password or token.

Credential rotation must write the replacement credential first, verify it can authenticate or refresh, then replace `credential_ref`/metadata. Failed rotation leaves the previous credential intact.

Token refresh, reconnect-needed status, and provider revocation handling are defined in `docs/architecture/token-lifecycle.md` and enforced by `src-tauri/src/token_lifecycle.rs`.

## Frontend And IPC Boundaries

- Every account setup or reauthentication command must evaluate the provider/auth policy in `src-tauri/src/auth.rs` before storing credentials or account metadata.
- No frontend API may expose `get_password`, `get_token`, `export_secret`, raw credential target names, or arbitrary credential reads.
- Allowed account commands are narrowly typed, for example `create_account`, `reauth_account`, `remove_account`, `get_account_auth_status`, and `test_account_connection`.
- `get_account_auth_status` may return booleans, expiry times, provider names, and redacted error codes only.
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
