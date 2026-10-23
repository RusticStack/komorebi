<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/logo-light.png">
    <img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/logo.png" width="320" alt="Komorebi logo: a sun behind an envelope">
  </picture>
</p>

<h1 align="center">Komorebi</h1>

<p align="center"><b>A small, fast email client for Windows.</b><br>
Read and write mail without getting in the way. No AI, no telemetry, no web engine.</p>

<p align="center">
  <a href="https://github.com/RusticStack/komorebi/releases/latest"><img src="https://img.shields.io/badge/Download_for_Windows-Komorebi-0078D7?style=for-the-badge&logo=windows11&logoColor=white" height="44" alt="Download Komorebi for Windows"></a>
</p>

<p align="center">
  <a href="https://github.com/RusticStack/komorebi/releases/latest"><img src="https://img.shields.io/github/v/release/RusticStack/komorebi?style=flat-square&label=latest&color=0078D7" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-2A2A30?style=flat-square" alt="Windows 10 and 11">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-2A2A30?style=flat-square" alt="GPL-3.0-or-later"></a>
  <a href="https://github.com/RusticStack/komorebi/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/RusticStack/komorebi/ci.yml?branch=main&style=flat-square&label=CI" alt="CI status"></a>
  <img src="https://img.shields.io/badge/no%20AI-no%20telemetry-2A2A30?style=flat-square" alt="No AI, no telemetry">
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-personal-dark.webp">
    <img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-personal-light.webp" width="820" alt="Komorebi showing an inbox with a message open in the reading pane">
  </picture>
</p>

<p align="center"><i>Komorebi is Japanese for sunlight filtering through leaves.</i></p>

## What it does

- **Fast and light.** A window in under 200 ms (about 40 to 60 ms measured on a
  quiet machine), a cached folder listed in under 600 ms, and an installer of
  about 8.9 MB.
- **IMAP and SMTP** with a local SQLite cache and several accounts. Presets for
  Purelymail and Postale.io, manual settings for any other server, and SMTP
  without sign-in for a relay on your own network.
- **Reads HTML mail with its own layout engine.** There is no web engine, and
  remote content is never loaded unless you allow it.
- **Writes formatted mail.** Fonts, colours, lists, tables, named signatures,
  recipient chips, attachments and drafts.
- **Tabs and separate windows**, keyboard shortcuts, meeting invitations.
- **Rules** that file and flag mail for you, with Copy as Sieve.
- **Contacts** with groups and CardDAV, and a **calendar** with CalDAV: month,
  week, day and agenda views, event editing and opt-in reminders.
- **Notifications and tray mode**, both opt-in.
- **Safe connections.** Certificate pinning (exact certificate, server key or
  issuing CA) and an optional encrypted cache.
- **English and European Portuguese**, following the Windows display language.
- **Signed self-updates** and a per-user installer with Start menu shortcuts and
  a mail-client registration.

Every feature in detail: [docs/FEATURES.md](docs/FEATURES.md).

## Screenshots

<table>
  <tr>
    <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-light.webp" alt="Inbox with remote images blocked"></picture><br><sub>Inbox, remote images blocked</sub></td>
    <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/compose-table-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/compose-table-light.webp" alt="Composer with a table"></picture><br><sub>Rich-text composer</sub></td>
    <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/calendar-month-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/calendar-month-light.webp" alt="Calendar, month view"></picture><br><sub>Calendar</sub></td>
  </tr>
  <tr>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/rules-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/rules-light.webp" alt="Rules list"></picture><br><sub>Rules</sub></td>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/contacts-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/contacts-light.webp" alt="Contacts"></picture><br><sub>Contacts</sub></td>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/tabs-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/tabs-light.webp" alt="Messages open in tabs"></picture><br><sub>Tabs</sub></td>
  </tr>
  <tr>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/cert-prompt-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/cert-prompt-light.webp" alt="Certificate review prompt"></picture><br><sub>Certificate review</sub></td>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/settings-privacy-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/settings-privacy-light.webp" alt="Privacy settings"></picture><br><sub>Privacy settings</sub></td>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/welcome-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/welcome-light.webp" alt="Welcome screen"></picture><br><sub>Welcome</sub></td>
  </tr>
</table>

The screenshots show made-up sample mail from the repository's offline test data.

## Getting started

### Install

1. [Download the latest installer](https://github.com/RusticStack/komorebi/releases/latest)
   (`komorebi-<version>-setup.exe`, Windows 10 or 11, 64-bit) and run it. It installs
   for your user by default, with no administrator rights. A portable zip is on the
   same page.
2. Windows may show a SmartScreen warning ("Windows protected your PC"), because the
   installer is not code-signed yet (see [Code signing policy](#code-signing-policy)).
   Choose **More info**, then **Run anyway**. You can first compare the download with
   `SHA256SUMS` on the release page.

More about the installer, silent installs, the portable zip and checksums:
[docs/USAGE.md](docs/USAGE.md).

### Add an account

Start Komorebi and choose **Add your email account**. Enter your address and
Continue: for Purelymail and Postale.io the server settings are built in, and for
other domains Komorebi looks them up from your domain name (only the domain is sent,
never the password). Enter your password (or an app password if your provider uses
two-step sign-in); it is saved in Windows Credential Manager, never in Komorebi's
files. Optionally add your name and a signature, then open your inbox. The steps in
detail are in [docs/USAGE.md](docs/USAGE.md).

### Updates

You can check for updates in Settings, About, or turn on the daily automatic check
(one request to GitHub for the latest release). Nothing is installed until you press
Download and install, and Komorebi installs an
update only if the installer matches the published SHA-256 and carries the
maintainer's Ed25519 signature, which is checked against a key built into the app.

## Privacy

- No telemetry, no analytics, no ads, no crash reports. There is no Komorebi service.
- Nothing is sent anywhere except to your own mail servers, a lookup of your domain's
  settings only when you press Continue while adding an account, and the update check
  to GitHub (see PRIVACY.md for each request).
- Passwords live in Windows Credential Manager. The mail cache is a local SQLite file
  in `%LOCALAPPDATA%\komorebi`, optionally encrypted.
- Remote images and other remote content in mail are not loaded unless you allow it.

The full statement is [PRIVACY.md](PRIVACY.md).

## Code signing policy

Free code signing provided by SignPath.io, certificate by SignPath Foundation

Code signing is being set up: the application to the SignPath Foundation is in
progress, and until it is approved the installer is not Authenticode-signed
(SmartScreen may warn, see [Install](#install)). The policy below applies to
releases once signing is active.

- **Roles.** Author and approver: Mário Silva.
- **Where releases are built.** Releases are built from this repository on GitHub
  Actions ([`release.yml`](.github/workflows/release.yml)) and submitted for signing
  from there. The in-app update signature is a separate Ed25519 signature made by
  the maintainer on a local machine; its private key never goes to CI.
- **Privacy.** Komorebi does not transfer any information to other networked systems
  unless you ask it to. See [PRIVACY.md](PRIVACY.md).

How a release is made: [docs/RELEASING.md](docs/RELEASING.md).

## For developers

Komorebi is written in Rust with [Slint](https://slint.dev) (Fluent style, software
renderer), using `rusqlite`, `imap`, `lettre` and `mail-parser`.

```powershell
git clone https://github.com/RusticStack/komorebi
cd komorebi
cargo build --release --locked   # target\release\komorebi.exe
cargo test --locked
```

| Document | What is in it |
|---|---|
| [docs/BUILDING.md](docs/BUILDING.md) | Build from source, the checks, verifying a build |
| [docs/ENVIRONMENT.md](docs/ENVIRONMENT.md) | Environment variables and development switches, the offline snapshot tool |
| [docs/FEATURES.md](docs/FEATURES.md) | Every feature in detail |
| [docs/USAGE.md](docs/USAGE.md) | Installing, adding an account, keyboard shortcuts |
| [docs/TRANSLATIONS.md](docs/TRANSLATIONS.md) | Adding and checking translations |
| [docs/RELEASING.md](docs/RELEASING.md) | The release and signing flow |
| [docs/SCOPE.md](docs/SCOPE.md) | What Komorebi will and will not do |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute |
| [CHANGELOG.md](CHANGELOG.md) | Changes by version |
| [SECURITY.md](SECURITY.md) | Reporting a vulnerability |

Found a bug or have an idea? [Open an issue](https://github.com/RusticStack/komorebi/issues/new/choose).

## License

GPL-3.0-or-later ([LICENSE](LICENSE)). Third-party notices are in [NOTICE](NOTICE).
Slint is used under its GPL-3.0 option; its attribution still appears in Settings,
About.
