# Provider Setup Guide

Last reviewed: 2026-05-25.

This guide documents the provider setup posture implemented in Komorebi today. It is not a live-account test result. Normal automated tests use fake/local protocol fixtures only.

## Common Rules

- Account secrets are stored only through Windows Credential Manager/DPAPI.
- React never receives OAuth authorization codes, access tokens, refresh tokens, passwords, app passwords, or credential target names.
- The Settings account wizard may prefill a provider plan from the email domain, but detection is only local convenience state. OAuth loopback or live generic IMAP/SMTP validation must still succeed before credentials or account metadata persist.
- Gmail and Microsoft setup must use OAuth2/XOAUTH2. Password and app-password setup for these providers is intentionally rejected.
- Generic IMAP/SMTP setup may use a password or app password only for providers that still support that model.
- Generic setup requires TLS or STARTTLS for both IMAP and SMTP. Plaintext authenticated mail transport is rejected.
- Provider Drafts upload is not implemented. Composer drafts are local-only until sent or discarded.
- Provider mailboxes remain the source of truth for synchronized mail. Komorebi has no mailbox backup/export/import feature.

## Gmail

Komorebi uses Gmail OAuth with SASL XOAUTH2 for IMAP and SMTP.

Current implemented profile:

- Authorization endpoint: `https://accounts.google.com/o/oauth2/v2/auth`
- Token endpoint: `https://oauth2.googleapis.com/token`
- Revocation endpoint: `https://oauth2.googleapis.com/revoke`
- Scope: `https://mail.google.com/`
- Redirect: backend-owned loopback listener on `127.0.0.1`
- PKCE: required by Komorebi

Setup expectations:

1. Create a Google Cloud project and OAuth client for a desktop/installed app.
2. Use a loopback redirect URI compatible with the Komorebi OAuth callback path shown in Settings.
3. Enable and consent to the Gmail mail scope used for IMAP/SMTP access.
4. Expect Google app verification requirements for broad mail access scopes before public distribution.
5. Do not enter a Gmail password or app password in Komorebi. Those paths are denied for Gmail.

Official references:

- Gmail XOAUTH2 for IMAP/SMTP: https://developers.google.com/workspace/gmail/imap/xoauth2-protocol
- Gmail IMAP/SMTP OAuth overview: https://developers.google.com/workspace/gmail/imap/imap-smtp
- Google OAuth installed-app flow: https://developers.google.com/identity/protocols/oauth2
- Google loopback flow notes: https://developers.google.com/identity/protocols/oauth2/resources/loopback-migration

## Microsoft / Outlook / Microsoft 365

Komorebi uses Microsoft OAuth with SASL XOAUTH2 for IMAP and SMTP.

Current implemented profile:

- Authorization endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/authorize`
- Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`
- Revocation endpoint: not implemented by provider profile; Komorebi reports revocation unsupported and still deletes local secrets.
- Scopes:
  - `offline_access`
  - `https://outlook.office.com/IMAP.AccessAsUser.All`
  - `https://outlook.office.com/SMTP.Send`
- Redirect: backend-owned loopback listener on `127.0.0.1`
- PKCE: required by Komorebi

Setup expectations:

1. Register an application in Microsoft Entra.
2. Configure a native/public-client redirect URI using the exact loopback callback path shown in Komorebi Settings.
3. Grant delegated IMAP and SMTP scopes appropriate for the account type.
4. Make sure IMAP and SMTP AUTH are enabled for the mailbox/tenant where required.
5. Do not enter a Microsoft password or app password in Komorebi. Those paths are denied for Microsoft accounts.

Official references:

- Microsoft OAuth for IMAP/SMTP: https://learn.microsoft.com/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth/
- Microsoft redirect URI guidance: https://learn.microsoft.com/entra/identity-platform/reply-url
- Microsoft modern-auth troubleshooting for POP/IMAP/SMTP: https://learn.microsoft.com/exchange/troubleshoot/administration/cannot-connect-mailbox-pop-imap-outlook

## Generic IMAP/SMTP

Use this flow only for providers that explicitly support password or app-password authentication over IMAP and SMTP.

The wizard currently checks, in order: Gmail/Microsoft OAuth routing, Thunderbird-compatible HTTPS autoconfig from the provider domain, Thunderbird ISPDB, and reviewed generic presets for Fastmail, iCloud, Yahoo, Zoho, GMX, and Mail.com domains. Unknown domains are not guessed as `imap.<domain>`/`smtp.<domain>`; the wizard opens manual server fields and requires the user to enter settings from the provider before testing.

Komorebi requires:

- Display email and username.
- IMAP host, port, and security mode.
- SMTP host, port, and security mode.
- TLS or STARTTLS for authenticated connections.
- A provider-supported password or app password.
- Successful live IMAP LOGIN and SMTP AUTH before account metadata is persisted.

Komorebi rejects:

- Plaintext authenticated IMAP/SMTP.
- Hostnames shaped like URLs.
- CR/LF/NUL characters in secrets.
- Oversized fields.
- Gmail/Microsoft password fallback through the generic form.
- Any setup path that would return the password or credential reference to React.

## Opt-In Live Smoke Tests

Normal checks do not use live provider accounts. The SMTP live smoke test is ignored by default and requires explicit user-provided test-account environment variables:

```powershell
$env:KOMOREBI_LIVE_SMTP_SMOKE = "1"
cargo test --manifest-path src-tauri/Cargo.toml --test live_smtp_smoke -- --ignored --nocapture
```

Do not run live smoke tests against a personal mailbox unless the user explicitly authorizes that exact account and understands that provider servers will receive login/send traffic.
