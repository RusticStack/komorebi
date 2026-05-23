# Provider Authentication

Status: accepted and implemented for backend auth policy, OAuth/XOAUTH2 protocol helpers, backend-owned loopback setup/reauthentication, and account-management command enforcement.
Last reviewed: 2026-05-23.

## Decision

Komorebi recognizes three provider classes:

- `gmail`
- `microsoft`
- `generic_imap_smtp`

Gmail and Microsoft account setup must use OAuth2 with XOAUTH2 for IMAP/SMTP. They must never fall back to manual passwords, app passwords, Basic authentication, or generic IMAP/SMTP password setup.

The Rust implementation is split across:

- `src-tauri/src/auth.rs`: provider/auth allow, reject, and unsupported policy.
- `src-tauri/src/oauth.rs`: OAuth authorization URL construction, PKCE, token exchange, token refresh, provider revocation, and XOAUTH2 initial response generation.
- `src-tauri/src/oauth_loopback.rs`: backend-owned Gmail/Microsoft authorization browser launch, loopback callback listener, state validation, token exchange, and refresh-token storage handoff.
- `src-tauri/src/token_lifecycle.rs`: backend-only refresh/reconnect/remove workflow.
- `src-tauri/src/credentials.rs`: Windows Credential Manager secret storage for long-lived secrets.
- `src-tauri/src/account_management.rs`: typed generic setup, generic reauthentication, removal, live generic setup connection validation, redacted auth-status commands, and public-command denial for direct Gmail/Microsoft secret handoff.
- `src-tauri/src/account_connection.rs`: backend-owned generic IMAP LOGIN and SMTP AUTH setup checks over TLS or STARTTLS.

The settings UI can now submit generic account setup, Gmail/Microsoft OAuth setup, OAuth reauthentication, generic reauthentication, removal, live generic setup validation, and redacted auth-status requests through typed Tauri commands. Gmail and Microsoft remain OAuth2/XOAUTH2-only. Their setup and reauthentication forms collect only non-secret account metadata plus an OAuth client ID, then call backend-owned loopback commands. React never receives authorization codes, access tokens, refresh tokens, provider token JSON, or credential target names.

## Provider Matrix

| Provider | Backend auth status | Required auth | Password/app-password policy |
| --- | --- | --- | --- |
| Gmail | OAuth2/XOAUTH2 protocol layer implemented | OAuth2 + XOAUTH2 for IMAP/SMTP | Rejected. Do not use generic password flow for Gmail. |
| Microsoft Outlook/Microsoft 365/Exchange Online | OAuth2/XOAUTH2 protocol layer implemented | OAuth2 + XOAUTH2 for IMAP/SMTP | Rejected. Do not use Basic auth or generic password flow. |
| Generic IMAP/SMTP | Password/app-password policy implemented | Provider-specific; may be password/app-password today | Allowed only as generic provider setup, stored through the credential backend. |

## Backend Policy

Every account-creation or reauthentication command must call `evaluate_account_setup_auth_policy` before storing credentials or account metadata.

Current policy:

- Gmail + OAuth2 -> `Allowed` with XOAUTH2 and the `oauth-refresh` credential slot.
- Gmail + manual password/app password -> `Rejected`.
- Microsoft + OAuth2 -> `Allowed` with XOAUTH2 and the `oauth-refresh` credential slot.
- Microsoft + manual password/app password -> `Rejected`.
- Generic IMAP/SMTP + manual password/app password -> `Allowed` with password-style SASL for compatible providers.
- Generic IMAP/SMTP + OAuth2 -> `UnsupportedUntilImplemented` until provider-specific issuer/scopes/token behavior are defined.

`create_account` is now the public generic-provider setup command. Its public IPC path rejects Gmail and Microsoft so a frontend cannot submit provider refresh tokens directly. For generic IMAP/SMTP, Rust validates host names, ports, TLS/STARTTLS modes, username, and provider/auth policy, then performs live IMAP LOGIN and SMTP AUTH from the backend before writing the credential or inserting account metadata. If the live login check fails, no credential or account row is written. If credential storage fails after a successful check, no account row is inserted. If account metadata insertion fails after a credential write, the command best-effort deletes the just-written account secrets. The OAuth loopback helper exchanges the authorization code inside Rust and then uses the lower-level account insertion boundary to write the approved refresh token to Windows Credential Manager before inserting account metadata. `reauth_account` is the public generic credential-rotation command and rejects existing Gmail/Microsoft OAuth accounts; `start_oauth_account_reauth` rotates those accounts only after the backend loopback/token exchange succeeds. `remove_account` deletes account secrets before deleting or soft-removing local account metadata.

## OAuth Details

Gmail uses:

- Authorization endpoint: `https://accounts.google.com/o/oauth2/v2/auth`
- Token endpoint: `https://oauth2.googleapis.com/token`
- Revocation endpoint: `https://oauth2.googleapis.com/revoke`
- Scope: `https://mail.google.com/`

Microsoft uses:

- Authorization endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/authorize`
- Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`
- Revocation endpoint: unsupported in Komorebi until a Microsoft-specific mechanism is reviewed
- Scopes: `offline_access`, `https://outlook.office.com/IMAP.AccessAsUser.All`, `https://outlook.office.com/SMTP.Send`

OAuth authorization requests use PKCE S256, an unguessable state value, and provider-specific offline-access parameters. The loopback listener binds `127.0.0.1` on a random available port and sends the root loopback redirect URI (`http://127.0.0.1:{port}`) for desktop-provider compatibility. The command opens the system browser from Rust, waits for the provider callback, validates the returned state, maps denied consent and provider errors to redacted UI-safe reasons, exchanges the code in Rust, requires a refresh token, and stores only that refresh token in Windows Credential Manager. React must never receive access tokens, refresh tokens, authorization codes, authorization headers, raw token response JSON, or credential target names from Rust.

XOAUTH2 SASL initial responses are generated in Rust as base64-encoded `user=<address>\x01auth=Bearer <access-token>\x01\x01`.

## Frontend Rules

- Gmail and Microsoft setup screens must use OAuth2 loopback and must not include password, app-password, refresh-token, or "advanced generic" fallback fields.
- Generic setup must clearly say it is for compatible IMAP/SMTP providers only.
- Generic setup copy must not mention Gmail, Outlook, Microsoft 365, Exchange Online, or Hotmail as examples.
- Generic setup testing must show live IMAP/SMTP login success only from the redacted `connection-ready` response and must not surface passwords, SMTP AUTH payloads, IMAP LOGIN commands, credential target names, or raw protocol lines.
- OAuth setup errors should map cancellation, callback timeout, state mismatch, invalid grant, and provider errors to UI-safe text; never expose token or credential internals.

## Credential Storage Relationship

This policy controls which secrets are allowed to reach the credential backend defined in `docs/architecture/credential-model.md`.

- Gmail/Microsoft OAuth refresh tokens use the `oauth-refresh` credential slot.
- Gmail/Microsoft access tokens are memory-only by default.
- Generic password/app-password secrets use the `generic-password` credential slot.
- Rejected provider/auth pairs must never write to Windows Credential Manager or SQLite.
- Generic `test_account_connection` runs the live login check without persistence. Generic `create_account` runs the same check before secret storage and account metadata insertion.

## Remaining Work

- Add provider setup documentation for registering OAuth clients and redirect URIs.
- Add richer UI states for pending authorization, callback failure, reconnect-needed, and account removal.
- Add background live-sync worker orchestration, live SMTP adapter verification, provider setup docs, and opt-in live-account smoke tests before claiming broad provider support.

## References

- Gmail IMAP/SMTP OAuth2 support: https://developers.google.com/workspace/gmail/imap/imap-smtp
- Gmail XOAUTH2 protocol details: https://developers.google.com/workspace/gmail/imap/xoauth2-protocol
- Microsoft OAuth support for IMAP/POP/SMTP: https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- Microsoft authorization-code flow with PKCE: https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow
- Microsoft Basic authentication deprecation in Exchange Online: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online
- Token lifecycle policy: `docs/architecture/token-lifecycle.md`
