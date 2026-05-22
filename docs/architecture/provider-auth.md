# Provider Authentication

Status: accepted and implemented for the backend OAuth/XOAUTH2 protocol layer.
Last reviewed: 2026-05-22.

## Decision

Komorebi recognizes three provider classes:

- `gmail`
- `microsoft`
- `generic_imap_smtp`

Gmail and Microsoft account setup must use OAuth2 with XOAUTH2 for IMAP/SMTP. They must never fall back to manual passwords, app passwords, Basic authentication, or generic IMAP/SMTP password setup.

The Rust implementation is split across:

- `src-tauri/src/auth.rs`: provider/auth allow, reject, and unsupported policy.
- `src-tauri/src/oauth.rs`: OAuth authorization URL construction, PKCE, token exchange, token refresh, provider revocation, and XOAUTH2 initial response generation.
- `src-tauri/src/token_lifecycle.rs`: backend-only refresh/reconnect/remove workflow.
- `src-tauri/src/credentials.rs`: Windows Credential Manager secret storage for long-lived secrets.

The account setup UI and loopback callback listener remain future TODO work, so users cannot add accounts from the app in this P0/P1 baseline. When that UI lands, Gmail and Microsoft must call this backend OAuth path only.

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

OAuth authorization requests use PKCE S256, an unguessable state value, and provider-specific offline-access parameters. Token exchange and refresh run in Rust. React must never receive access tokens, refresh tokens, authorization codes after exchange, authorization headers, raw token response JSON, or credential target names.

XOAUTH2 SASL initial responses are generated in Rust as base64-encoded `user=<address>\x01auth=Bearer <access-token>\x01\x01`.

## Frontend Rules

- Gmail and Microsoft setup screens must start OAuth and must not include password, app-password, or "advanced generic" fallback fields.
- Generic setup must clearly say it is for compatible IMAP/SMTP providers only.
- Generic setup copy must not mention Gmail, Outlook, Microsoft 365, Exchange Online, or Hotmail as examples.
- Errors should use status language such as `oauth_required`, `oauth_callback_failed`, `token_refresh_failed`, or `reconnect_needed`; never expose token or credential internals.

## Credential Storage Relationship

This policy controls which secrets are allowed to reach the credential backend defined in `docs/architecture/credential-model.md`.

- Gmail/Microsoft OAuth refresh tokens use the `oauth-refresh` credential slot.
- Gmail/Microsoft access tokens are memory-only by default.
- Generic password/app-password secrets use the `generic-password` credential slot.
- Rejected provider/auth pairs must never write to Windows Credential Manager or SQLite.

## Remaining Work Before User-Facing Account Add

- Add provider setup documentation for registering OAuth clients and redirect URIs.
- Implement the Tauri/Rust command that opens the provider URL and receives the loopback callback without exposing secrets to React.
- Persist non-secret account metadata only after token exchange and Credential Manager storage succeed.
- Add UI states for pending authorization, callback failure, reconnect-needed, and account removal.

## References

- Gmail IMAP/SMTP OAuth2 support: https://developers.google.com/workspace/gmail/imap/imap-smtp
- Gmail XOAUTH2 protocol details: https://developers.google.com/workspace/gmail/imap/xoauth2-protocol
- Microsoft OAuth support for IMAP/POP/SMTP: https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- Microsoft authorization-code flow with PKCE: https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow
- Microsoft Basic authentication deprecation in Exchange Online: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online
- Token lifecycle policy: `docs/architecture/token-lifecycle.md`
