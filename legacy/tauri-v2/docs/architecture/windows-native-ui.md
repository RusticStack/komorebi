# Windows Native UI Direction

Status: accepted direction for the Tauri/WebView frontend.
Last reviewed: 2026-05-25.

Komorebi remains a Tauri v2 app, not a WinUI 3 desktop app. The UI should still feel native on Windows 11 by following Microsoft Windows app design and Fluent guidance where it maps cleanly to web technologies: integrated app chrome, quiet navigation, layered surfaces, visible focus, predictable settings flows, and restrained motion/elevation.

## Sources Reviewed

- Microsoft Windows app design guidance: https://learn.microsoft.com/windows/apps/design/
- Microsoft WinUI 3 overview: https://learn.microsoft.com/windows/apps/winui/winui3/
- Tauri v2 window customization: https://v2.tauri.app/learn/window-customization/
- Tauri v2 capabilities and core permissions: https://v2.tauri.app/security/capabilities/ and https://v2.tauri.app/reference/acl/core-permissions/
- shadcn/ui component model: https://ui.shadcn.com/docs
- TanStack Router and Query documentation: https://tanstack.com/router/latest and https://tanstack.com/query/latest
- Thunderbird automatic account configuration UX reference: https://support.mozilla.org/en-US/kb/automatic-account-configuration
- Zen Browser product/UI reference: https://zen-browser.app/

## Implemented Direction

- The Tauri main window disables native decorations and uses an app-owned title bar in `MailClientTitleBar`.
- The title bar uses Tauri drag regions plus the narrow core window permissions for start-dragging, minimize, toggle-maximize, and close.
- The app shell uses a Windows 11-style navigation layout split across focused React components: `MailSidebar` for account/folder navigation, `MessageListPane` for virtualized mail rows, `ReaderPane` for safe message reading/actions, and routed settings for account management.
- Settings is a dedicated routed page in the app runtime, with the original drawer retained only as an isolated shell fallback for tests and non-router embedding.
- The visual language combines Fluent layering/elevation with the Komorebi product tone: calm neutral surfaces, sage accents, soft translucency, and reduced decorative noise.
- Account setup is an email-first wizard inside settings. Rust owns provider discovery; the UI hides server fields unless discovery returns manual input or the user opens them.
- Reusable UI primitives are local shadcn-style components layered on Radix/Tailwind utilities; app-specific layout and tone remain in `src/App.css`.

## Native Boundaries

- Do not add a WinUI 3 dependency unless the product moves away from Tauri/WebView. WinUI remains a design reference here, not a runtime dependency.
- Do not grant broad frontend OS permissions to make the custom chrome work. Window controls are limited to the reviewed `core:window` permissions in `src-tauri/capabilities/default.json`.
- Do not use frontend filesystem, shell, opener, process, HTTP, or dialog permissions for design features.
- Do not make account setup look automatic if backend validation has not succeeded. Discovery may prefill fields, but live generic IMAP/SMTP tests and OAuth loopback remain the authority before account metadata or credentials persist.
- Do not imply provider Drafts-folder sync or live provider support beyond the implemented account setup/sync/send paths.

## Account Setup Detection

The settings wizard discovers:

- Gmail and Googlemail: Gmail OAuth2/XOAUTH2 flow.
- Outlook, Hotmail, Live, MSN, and Office 365 domains: Microsoft OAuth2/XOAUTH2 flow.
- Thunderbird-compatible HTTPS provider autoconfig and Thunderbird ISPDB generic IMAP/SMTP settings.
- Fastmail, iCloud, Yahoo, Zoho, GMX, and Mail.com: compatible generic IMAP/SMTP presets.
- Unknown domains: no host guessing; the wizard asks for manual provider settings.

Detection is convenience state only. Generic setup still requires TLS/STARTTLS metadata, a user-provided password/app-password, backend live IMAP/SMTP login checks, and successful credential storage before SQLite metadata is written. Gmail and Microsoft setup must use the backend-owned OAuth loopback flow and must never fall back to password fields.

## Regression Rules

- Custom titlebar changes must keep desktop and narrow screenshots free of overlap or horizontal overflow.
- Routed settings changes must keep desktop and narrow screenshots free of overlap or horizontal overflow.
- Keep titlebar buttons keyboard reachable and labelled.
- Keep the account-add entry reachable from the first screen even when no accounts exist.
- Any added provider preset or discovery source must be covered by tests and documented here or in `docs/provider-setup.md`.
- Capability tests must fail if custom-window permissions are removed accidentally or broadened beyond the reviewed window commands.
