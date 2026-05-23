# Provider Authentication

Status: accepted and implemented for backend auth policy, OAuth/XOAUTH2 protocol helpers, and account-management command enforcement.
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
- `src-tauri/src/token_lifecycle.rs`: backend-only refresh/reconnect/remove workflow.
- `src-tauri/src/credentials.rs`: Windows Credential Manager secret storage for long-lived secrets.
- `src-tauri/src/account_management.rs`: typed setup, reauthentication, removal, configuration validation, and redacted auth-status commands that enforce this policy before secret writes or account metadata inserts.

The settings UI can now submit account setup, reauthentication, removal, configuration validation, and redacted auth-status requests through typed Tauri commands. Gmail and Microsoft remain OAuth2/XOAUTH2-only. Until the OAuth loopback callback task lands, those provider forms accept only an OAuth refresh-token handoff as the secret value; passwords and app passwords are rejected before credential or SQLite writes. The loopback flow must replace this handoff with backend-owned authorization-code exchange.

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

`create_account` writes the approved secret to Windows Credential Manager before inserting account metadata. If credential storage fails, no account row is inserted. If account metadata insertion fails after a credential write, the command best-effort deletes the just-written account secrets. `reauth_account` rotates only the existing account's approved credential slot and never returns the secret to React. `remove_account` deletes account secrets before deleting or soft-removing local account metadata.

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

OAuth authorization requests use PKCE S256, an unguessable state value, and provider-specific offline-access parameters. Token exchange and refresh run in Rust. React must never receive access tokens, refresh tokens, authorization codes after exchange, authorization headers, raw token response JSON, or credential target names from Rust. The current pre-loopback setup form may hold a user-entered refresh-token handoff long enough to submit `create_account`; do not broaden that temporary path, echo it in errors, persist it in frontend storage, or use it for access tokens.

XOAUTH2 SASL initial responses are generated in Rust as base64-encoded `user=<address>\x01auth=Bearer <access-token>\x01\x01`.

## Frontend Rules

- Gmail and Microsoft setup screens must use OAuth2 and must not include password, app-password, or "advanced generic" fallback fields. Until loopback is implemented, the only allowed major-provider secret field is the temporary OAuth refresh-token handoff.
- Generic setup must clearly say it is for compatible IMAP/SMTP providers only.
- Generic setup copy must not mention Gmail, Outlook, Microsoft 365, Exchange Online, or Hotmail as examples.
- Errors should use status language such as `oauth_required`, `oauth_callback_failed`, `token_refresh_failed`, or `reconnect_needed`; never expose token or credential internals.

## Credential Storage Relationship

This policy controls which secrets are allowed to reach the credential backend defined in `docs/architecture/credential-model.md`.

- Gmail/Microsoft OAuth refresh tokens use the `oauth-refresh` credential slot.
- Gmail/Microsoft access tokens are memory-only by default.
- Generic password/app-password secrets use the `generic-password` credential slot.
- Rejected provider/auth pairs must never write to Windows Credential Manager or SQLite.

## Remaining Work

- Add provider setup documentation for registering OAuth clients and redirect URIs.
- Implement the Tauri/Rust command that opens the provider URL and receives the loopback callback without exposing secrets to React.
- Replace the temporary refresh-token handoff with backend-owned token exchange and Credential Manager storage.
- Add UI states for pending authorization, callback failure, reconnect-needed, and account removal.
- Add live generic IMAP/SMTP login testing once the live adapters exist. Current `test_account_connection` validates only configuration and credential policy.

## References

- Gmail IMAP/SMTP OAuth2 support: https://developers.google.com/workspace/gmail/imap/imap-smtp
- Gmail XOAUTH2 protocol details: https://developers.google.com/workspace/gmail/imap/xoauth2-protocol
- Microsoft OAuth support for IMAP/POP/SMTP: https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- Microsoft authorization-code flow with PKCE: https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow
- Microsoft Basic authentication deprecation in Exchange Online: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online
- Token lifecycle policy: `docs/architecture/token-lifecycle.md`
