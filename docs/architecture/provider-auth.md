# Provider Authentication

Status: accepted policy; OAuth account flows are not implemented yet.
Last reviewed: 2026-05-22.

## Decision

Komorebi recognizes three provider classes:

- `gmail`
- `microsoft`
- `generic_imap_smtp`

Gmail and Microsoft account setup must use OAuth2 with XOAUTH2 for IMAP/SMTP. Until those OAuth flows are implemented, Gmail and Microsoft account setup are explicitly unsupported in UI and backend policy. They must not fall back to manual passwords, app passwords, Basic authentication, or generic IMAP/SMTP password setup.

Generic IMAP/SMTP accounts may use manual password or provider-issued app-password authentication only when the provider is compatible with password-based SASL flows. Generic password setup must not be presented as a Gmail or Microsoft workaround.

The enforceable Rust policy lives in `src-tauri/src/auth.rs`.

## Provider Matrix

| Provider | Status today | Required future auth | Password/app-password policy |
| --- | --- | --- | --- |
| Gmail | Unsupported until OAuth is implemented | OAuth2 + XOAUTH2 for IMAP/SMTP | Rejected. Do not use generic password flow for Gmail. |
| Microsoft Outlook/Microsoft 365/Exchange Online | Unsupported until OAuth is implemented | OAuth2 + XOAUTH2 for IMAP/SMTP | Rejected. Do not use Basic auth or generic password flow. |
| Generic IMAP/SMTP | Allowed for compatible providers | Provider-specific; may be password/app-password today | Allowed only as generic provider setup, stored through the credential backend. |

## Backend Policy

Every account-creation or reauthentication command must call the auth policy before storing credentials or account metadata.

Policy outcomes:

- `Allowed`: the provider/auth pair is permitted and names the SASL mechanism plus credential slot.
- `UnsupportedUntilImplemented`: the provider/auth pair is valid in principle but Komorebi does not implement the flow yet. Return a non-secret status/error to the frontend.
- `Rejected`: the provider/auth pair is forbidden. Do not store credentials, do not create account metadata, and do not attempt network authentication.

Current policy:

- Gmail + OAuth2 -> `UnsupportedUntilImplemented` while OAuth implementation is pending; `Allowed` with XOAUTH2 once OAuth is implemented.
- Gmail + manual password/app password -> `Rejected`.
- Microsoft + OAuth2 -> `UnsupportedUntilImplemented` while OAuth implementation is pending; `Allowed` with XOAUTH2 once OAuth is implemented.
- Microsoft + manual password/app password -> `Rejected`.
- Generic IMAP/SMTP + manual password/app password -> `Allowed` with password-style SASL for compatible providers.
- Generic IMAP/SMTP + OAuth2 -> `UnsupportedUntilImplemented` until provider-specific issuer/scopes/token behavior are defined.

## Frontend Rules

- Gmail and Microsoft provider cards may be shown only with a disabled or "OAuth setup coming soon" state until OAuth is implemented.
- Gmail and Microsoft setup screens must not include password, app-password, or "advanced generic" fallback fields.
- Generic setup must clearly say it is for compatible IMAP/SMTP providers only.
- Generic setup copy must not mention Gmail, Outlook, Microsoft 365, Exchange Online, or Hotmail as examples.
- Errors should use status language such as `oauth_required` or `oauth_not_implemented`; never expose token or credential internals.

## Credential Storage Relationship

This policy controls which secrets are allowed to reach the credential backend defined in `docs/architecture/credential-model.md`.

- Gmail/Microsoft OAuth refresh tokens use the `oauth-refresh` credential slot after OAuth is implemented.
- Gmail/Microsoft access tokens should be memory-only where possible, or stored in `oauth-access` only with expiry metadata if a later design justifies it.
- Generic password/app-password secrets use the `generic-password` credential slot.
- Rejected provider/auth pairs must never write to Windows Credential Manager or SQLite.

## Implementation Requirements Before Enabling Gmail/Microsoft

Before Gmail or Microsoft account setup is enabled, Komorebi needs:

- OAuth client registration and redirect-loopback design.
- Provider-specific scopes for IMAP/SMTP access.
- Token exchange, refresh, expiry, and revocation handling inside Rust.
- XOAUTH2 SASL string generation inside Rust.
- Credential Manager storage for refresh tokens only after successful exchange.
- Tests proving password/app-password paths remain rejected for Gmail and Microsoft.
- Provider setup documentation for users.

## References

- Gmail IMAP/SMTP OAuth2 support: https://developers.google.com/workspace/gmail/imap/imap-smtp
- Gmail XOAUTH2 protocol details: https://developers.google.com/workspace/gmail/imap/xoauth2-protocol
- Microsoft OAuth support for IMAP/POP/SMTP: https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- Microsoft Basic authentication deprecation in Exchange Online: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online
