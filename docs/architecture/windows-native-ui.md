# Windows Native UI Direction

Status: accepted direction for the Tauri/WebView frontend.
Last reviewed: 2026-05-25.

Komorebi remains a Tauri v2 app, not a WinUI 3 desktop app. The UI should still feel native on Windows 11 by following Microsoft Windows app design and Fluent guidance where it maps cleanly to web technologies: integrated app chrome, quiet navigation, layered surfaces, visible focus, predictable settings flows, and restrained motion/elevation.

## Sources Reviewed

- Microsoft Windows app design guidance: https://learn.microsoft.com/windows/apps/design/
- Microsoft WinUI 3 overview: https://learn.microsoft.com/windows/apps/winui/winui3/
- Tauri v2 window customization: https://v2.tauri.app/learn/window-customization/
- Tauri v2 capabilities and core permissions: https://v2.tauri.app/security/capabilities/ and https://v2.tauri.app/reference/acl/core-permissions/

## Implemented Direction

- The Tauri main window disables native decorations and uses an app-owned title bar in `MailClientShell`.
- The title bar uses Tauri drag regions plus the narrow core window permissions for start-dragging, minimize, toggle-maximize, and close.
- The app shell uses a Windows 11-style navigation layout: compact account rail, mailbox/folder navigation, virtualized message list, and reader pane.
- The visual language combines Fluent layering/elevation with the Komorebi product tone: calm neutral surfaces, sage accents, soft translucency, and reduced decorative noise.
- Account setup is a guided wizard inside settings. It detects common provider settings locally from the email domain before the typed backend validation runs.

## Native Boundaries

- Do not add a WinUI 3 dependency unless the product moves away from Tauri/WebView. WinUI remains a design reference here, not a runtime dependency.
- Do not grant broad frontend OS permissions to make the custom chrome work. Window controls are limited to the reviewed `core:window` permissions in `src-tauri/capabilities/default.json`.
- Do not use frontend filesystem, shell, opener, process, HTTP, or dialog permissions for design features.
- Do not make account setup look automatic if backend validation has not succeeded. Domain detection may prefill fields, but live generic IMAP/SMTP tests and OAuth loopback remain the authority before account metadata or credentials persist.
- Do not imply provider Drafts-folder sync or live provider support beyond the implemented account setup/sync/send paths.

## Account Setup Detection

The settings wizard can infer:

- Gmail and Googlemail: Gmail OAuth2/XOAUTH2 flow.
- Outlook, Hotmail, Live, MSN, and Office 365 domains: Microsoft OAuth2/XOAUTH2 flow.
- Fastmail, iCloud, Yahoo, Zoho, GMX, and Mail.com: compatible generic IMAP/SMTP presets.
- Unknown domains: common `imap.<domain>` and `smtp.<domain>` fallback, prefilled for user review.

Detection is convenience state only. Generic setup still requires TLS/STARTTLS metadata, a user-provided password/app-password, backend live IMAP/SMTP login checks, and successful credential storage before SQLite metadata is written. Gmail and Microsoft setup must use the backend-owned OAuth loopback flow and must never fall back to password fields.

## Regression Rules

- Custom titlebar changes must keep desktop and narrow screenshots free of overlap or horizontal overflow.
- Keep titlebar buttons keyboard reachable and labelled.
- Keep the account-add entry reachable from the first screen even when no accounts exist.
- Any added provider preset must be covered by a frontend test and documented here or in `docs/provider-setup.md`.
- Capability tests must fail if custom-window permissions are removed accidentally or broadened beyond the reviewed window commands.
