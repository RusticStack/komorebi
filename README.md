<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/logo-light.png">
    <img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/logo.png" width="320" alt="Komorebi logo: a grey seal with a white K, and the wordmark komorebi">
  </picture>
</p>

<h1 align="center">Komorebi</h1>

<p align="center"><b>A fast email client for Windows.</b><br>
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
    <img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-personal-light.webp" width="820" alt="Komorebi showing an inbox with a formatted newsletter open in the reading pane">
  </picture>
</p>

<p align="center"><i>Komorebi is Japanese for sunlight filtering through leaves.</i></p>

## What it does

- **Fast.** A window in under 200 ms (77 ms to the first frame was measured
  on one development PC with a 2,000-message cache), and a cached folder
  listed in under 600 ms. Komorebi 1.0 draws its interface with GPUI, which
  keeps more memory in use while it idles than Komorebi 0.7 did; that is
  accepted for 1.0, and reducing it is planned for 1.1.
- **IMAP and SMTP** with a local SQLite cache and several accounts. A searchable
  list of 26 providers with their settings built in (Gmail, iCloud, Yahoo,
  Fastmail, Proton Mail through Bridge, Purelymail, Postale.io and more; each
  says whether it needs an app password), your own domain recognised from its
  MX records when a listed provider hosts it, manual settings for any other
  server, and SMTP without sign-in for a relay on your own network.
  Outlook.com and Microsoft 365 are listed but not supported yet: they need
  Microsoft sign-in, which Komorebi doesn't have.
- **Reads HTML mail with its own layout engine.** There is no web engine, and
  remote content is never loaded unless you allow it.
- **Writes formatted mail.** Fonts, colours, lists, tables, named signatures,
  recipient chips, attachments and drafts.
- **Tabs and separate windows**, keyboard shortcuts, meeting invitations.
- **Rules** that file and flag mail for you, with Copy as Sieve.
- **Contacts** with groups and CardDAV. There is no calendar in 1.0: the one in
  0.7 is left out for now and planned to return in 1.1 (an invitation in a
  message still shows on a card, and Save .ics takes it to your calendar app).
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
    <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/inbox-light.webp" alt="Inbox with a message and its attachment open"></picture><br><sub>Inbox with an attachment</sub></td>
    <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/compose-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/compose-light.webp" alt="Rich-text composer"></picture><br><sub>Rich-text composer</sub></td>
  </tr>
  <tr>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/rules-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/rules-light.webp" alt="Rules list"></picture><br><sub>Rules</sub></td>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/contacts-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/contacts-light.webp" alt="Contacts"></picture><br><sub>Contacts</sub></td>
  </tr>
  <tr>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/settings-privacy-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/settings-privacy-light.webp" alt="Privacy settings"></picture><br><sub>Privacy settings</sub></td>
    <td><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/welcome-dark.webp"><img src="https://raw.githubusercontent.com/RusticStack/komorebi/main/.github/assets/screenshots/welcome-light.webp" alt="Welcome screen"></picture><br><sub>Welcome</sub></td>
  </tr>
</table>

The screenshots show a made-up demo mailbox (`examples/demo_mailbox.rs`): invented people on example.com and example.org, no real mail.

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
Continue: for the built-in providers (Gmail, iCloud, Fastmail, Purelymail,
Postale.io and others) the server settings are built in, and for other domains
Komorebi looks them up from your domain name (only the domain is sent, never the
password): a domain hosted by a listed provider is recognised from its MX records.
If nothing is found, search the provider list, or choose **Add provider** to enter
any other server by hand. Enter your password (or an app password if your provider
needs one; the list says which); it is saved in Windows Credential Manager, never in Komorebi's
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

Komorebi is written in Rust, using `rusqlite`, `imap`, `lettre` and `mail-parser`.
Komorebi 1.0 draws its interface with [GPUI](https://gpui.rs) and GPUI Kit: the app
is `komorebi.exe` (the package `komorebi-gpui`, in `crates/komorebi-gpui`), over the
library in `src/lib.rs` ([docs/GPUI-MIGRATION.md](docs/GPUI-MIGRATION.md)). The release
exe must stay under 21.5 MB (21,500,000 bytes). Komorebi 0.7's Slint interface is gone
from the repository; its mail engine, cache and calendar code are the library.

```powershell
git clone https://github.com/RusticStack/komorebi
cd komorebi
cargo build -p komorebi-gpui     # the app, Komorebi 1.0 (docs/GPUI-BUILD.md)
cargo build --release --locked -p komorebi-gpui   # target\release\komorebi.exe
cargo test --locked --workspace
cargo xtask check                # the gate before a commit (docs/GPUI-TESTING.md)
```

| Document | What is in it |
|---|---|
| [docs/BUILDING.md](docs/BUILDING.md) | Build from source, the checks, verifying a build |
| [docs/GPUI-BUILD.md](docs/GPUI-BUILD.md) | Building the GPUI app: fxc.exe, vendored crates and patches, switches |
| [docs/GPUI-MIGRATION.md](docs/GPUI-MIGRATION.md) | The move to GPUI for 1.0: phases, decisions, measurements |
| [docs/GPUI-TESTING.md](docs/GPUI-TESTING.md) | Testing the GPUI app: the harness, snapshots on WARP, lints, `cargo xtask check` |
| [docs/GPUI-TEST-PORT.md](docs/GPUI-TEST-PORT.md) | Every test of the old Slint app and where it went in the GPUI app |
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
The app includes GPUI, GPUI Kit (Apache-2.0) and the Lucide icons.
