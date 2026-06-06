# Real Provider Manual Smoke Matrix

Run these checks only with explicit test accounts created for Komorebi validation. Do not use personal mailboxes, production customer mailboxes, or credentials stored in test fixtures, scripts, environment variables, logs, screenshots, or diagnostics bundles.

## Providers

Cover at least these account shapes before a production release:

- Custom domain with outsourced mail hosting, for example a domain whose MX points at a provider such as postale.io.
- Generic hosting-provider mailbox where IMAP/SMTP hosts differ from the email domain, for example Amen.pt-style hosting.
- Workspace provider with standards-based autoconfig or DNS SRV discovery.
- OAuth provider account for Gmail or Microsoft, using a dedicated app registration and test mailbox.

## Account Setup

1. Open the in-app account wizard.
2. Enter the test account email address and display name.
3. Run discovery.
4. Confirm the selected candidate came from autoconfig or DNS SRV when the provider publishes those records.
5. Confirm MX appears only as diagnostics and is not converted into hardcoded IMAP/SMTP templates.
6. Enter the test credential or complete OAuth in the wizard.
7. Press Test and confirm IMAP and SMTP authentication both pass.
8. Save the account only after Test succeeds.

## Mail Flow

1. Start one-shot sync from the sidebar and confirm INBOX metadata appears.
2. Start auto-sync and confirm the status shows a recent run.
3. Confirm Sent, Drafts, Archive, and Trash sync only when the server advertises matching folders.
4. Send a message from the test account to another test mailbox.
5. Confirm the message body is sanitized in the local sent copy.
6. Confirm attachments can be listed, previewed when safe, saved to the fixed Downloads folder, and deleted from the local cache.

## Rendering And Privacy

1. Open a message with remote images.
2. Confirm remote images are blocked by default.
3. Use Load once and confirm only that message loads safe remote images.
4. Use Always load sender only for a test sender, then confirm other senders remain blocked.
5. Click an external link and confirm it opens through the OS browser rather than navigating inside the mail WebView.
6. Export diagnostics and confirm the bundle contains no raw credentials, no full email addresses, no mailbox bodies, and no local attachment storage paths.

## Release Gate

Before promoting a build:

1. Run `wails3 build`.
2. Sign `bin/Komorebi.exe` with `task windows:sign CERT_THUMBPRINT=<thumbprint>`.
3. Run `task windows:validate-release`.
4. Install on a clean Windows 10/11 machine with no existing Komorebi app data.
5. Repeat the account setup and rendering checks above with dedicated test accounts.
