# Roadmap and known gaps

Scope and non-goals are in [`docs/SCOPE.md`](docs/SCOPE.md). The history of
what shipped is in [`CHANGELOG.md`](CHANGELOG.md). This file lists what is
open, what is deferred on purpose, what to check by hand before a release,
and the gaps we know about.

## Product budgets

Measured on a release build, software renderer, offline against throwaway
caches (see `docs/PERF.md` for the method):

- Launch to window under 200 ms; cached message list under 600 ms.
- Window created in about 6 ms; 55,000 cached messages listed in about
  30 ms (the list is a lazy model over ids, so cost barely grows with
  mailbox size).
- Private memory about 9 to 11 MB with the cache loaded.
- Release binary under 20.0 MB (CI enforces it).

## Open

### Self-update from GitHub releases

- [x] UI: Settings > About > Updates (`src/app/updates.rs`), the quiet
      "Version X is available" bar, Later until the next day, save and
      quit after a verified download (`KOMOREBI_SNAPSHOT_ACTION=update:*`).
- [ ] Download progress: `fetch::get_with` reports no bytes, so the bar
      is indeterminate; show the share downloaded.
- [ ] Snapshots still to take: `update:checking`, `uptodate`,
      `downloading`, `failed` in both languages and sizes, and high
      contrast.
- [ ] Release workflow: sign the installer with `examples/sign_update.rs`
      and upload the `.sha256` and `.sig` files next to it. The installer
      asset name must stay `komorebi-<version>-setup.exe` (or change
      `installer_name()` in `src/update.rs` to match).
- [ ] Installer: silent updates rely on the Restart Manager closing the
      running app; `AppMutex=Komorebi.App` (a fixed mutex every running
      copy holds, closed by the self-update just before the installer
      starts) stops other installs while it runs. Test an update with the
      app running in the tray and with a second cache open.
- [ ] Measure the release exe size with `ed25519-compact` and `sha2`
      (limit 20.0 MB).

### Installer and release

- [ ] `mailto:` links: the installer registers `komorebi.exe "%1"` for
      `mailto`, but the app does not yet read it from the command line.
      It should open a composer, ideally in the running instance.
- [ ] Code signing: choose a provider and certificate, then add the
      `SIGN_CERT_PFX` and `SIGN_CERT_PASSWORD` secrets (or replace
      `scripts/sign.ps1` with the provider's step). Until then the binary
      and installer are unsigned and Windows SmartScreen will warn.
- [ ] Run the release workflow once by hand (`workflow_dispatch`, the dry
      run) and check the setup, the zip and `SHA256SUMS` it keeps.
- [ ] On a clean machine: install, first run, add an account, "Remove all
      data", uninstall (both answers to the cache prompt).
- [ ] Smoke-test the static-CRT release build (TLS sign-in and Credential
      Manager) on a real account before a release.
- [ ] A cold-start measurement on a clean machine
      (`KOMOREBI_TIMING=1` prints "first frame drawn").

### Providers and servers

- [ ] Validate against more real providers (Purelymail, a self-hosted
      Dovecot, Stalwart, Cyrus, Proton Bridge). Manual accounts,
      certificate pinning, "Find settings" and the capability fallbacks are
      tested only on the local fakes (`src/mail/fake.rs`).
- [ ] Check which `authserv-id` each provider writes in
      `Authentication-Results` (the sender notice trusts the topmost header
      whose id has the IMAP host's registrable domain), and that the server
      strips such headers arriving from outside. Until then a provider whose
      id is in another domain shows no DMARC notice.
- [ ] Confirm whether Purelymail saves sent mail server-side; if so,
      default "save a copy to Sent" off for that preset.
- [ ] Servers that never wake IDLE for new mail: consider dropping the
      third (IDLE) connection to save a connection.
- [ ] Open a server draft saved from another device in the composer
      (deferred until server draft copies are proven on real providers).

### Reader and composer

- [ ] Dimmed mail in the dark theme leaves images alone, so a transparent
      dark logo can vanish ("Show original colours" is the remedy). If this
      shows up often, put a light plate behind transparent images.
- [ ] Copying formatted text out of the composer (Ctrl+C copies plain text).
- [ ] Arrow keys inside the colour flyout (Tab reaches the swatches).
- [ ] A caret blink in the composer.
- [ ] Narrator reads only the proxy's part of a message over 16 KB.
- [ ] Recipients are text, not person tokens.

### Notifications, tray and background mode

- [ ] Per-account notification switch in each account's page.
- [ ] Re-add the tray icon after an Explorer restart (`TaskbarCreated` is
      not handled yet).
- [ ] Snapshots of Settings > General in en and pt-PT at 1280x800 and
      900x700.

### Windows 11 look

- [ ] The native context and More menus can look light in the dark theme (a
      dark menu needs the undocumented uxtheme `SetPreferredAppMode`).
- [ ] No Mica: the software renderer draws opaque pixels, so the solid
      fallback WinUI itself uses is shown.
- [ ] The text size follows at the next start if the slider sends no
      `WM_SETTINGCHANGE`.
- [ ] Slint's standard ComboBox has no contrast colours of its own
      (`ComboEdge` adds the edge) and keeps its animations when animation
      effects are off.
- [ ] The navigation pill jumps instead of sliding.
- [ ] The onboarding welcome's entrance is opacity only (the 8 px rise
      needs a transform animation), and the card's height does not animate
      between steps.

### Tabs and windows

- [ ] Removing or re-downloading an account, or a folder vanishing, while
      one of its messages shows in a message tab leaves the tab's reader
      empty; it should say the message is gone, as a sync does.
- [ ] Settings opened from a message tab show without the folder pane, and
      F6 there still aims at the pane.

### Maintenance

- [ ] On every Slint upgrade, check the TextInput's internal
      `anchor-position-byte-offset` and `cursor-position-byte-offset` that
      `ComposeBody` reads (the formatting test must pass).
- [ ] An intermittent failure of the reader's resize-debounce test was seen
      once in a full run; if it returns, capture its name and fix the
      timing assumption.

## Deferred on purpose

- Code-signed release builds (waiting for a certificate).
- SPKI or CA pinning of server certificates, SMTP without sign-in, and a
  native-tls submission client.
- An encrypted local cache (the cache is plaintext today).
- Calendar, contacts sync, rules engine, and anything on the non-goals
  list in `docs/SCOPE.md`.

## Manual test checklist

Snapshots cannot capture native menus, window chrome or real servers, so
check these by hand before a release. Use throwaway or self-addressed mail
only.

1. Lists and menus
   - Mouse-wheel scrolling of the folder pane and of the message list with
     many rows.
   - The row menu opens at the pointer on a right-click (also on a row
     scrolled down the list, and at 900 px wide); the More menu opens under
     its button; Move to lists the folders; the labels follow the selection
     (Mark as read or unread, Not junk in Junk).
   - The overlay pane's smoke takes clicks (closes the pane).
2. Composer
   - Drag files from Explorer onto the composer (accent border while
     hovering, all files attached, ignored with the composer closed); copy
     files in Explorer and Ctrl+V in the composer.
   - Open on a PDF (Protected View or the downloaded-file warning) and on an
     `.exe` (refused).
   - Server draft copies: write a draft to yourself, wait 30 s, edit it,
     close the composer; the Drafts folder must hold exactly one copy,
     flagged as a draft, with the last text. Send it and check the copy is
     gone and Sent has the message. Note whether the server has UIDPLUS.
3. Saving and forwarding
   - Save as .eml from the More menu and with Ctrl+S (the dialog offers
     `<subject> (<date>).eml` in Downloads; the file opens in another mail
     app), also on a message not yet downloaded; Forward as attachment to
     yourself.
4. Windows integration
   - With "Flash the taskbar button" on and the window in the background,
     new mail flashes the button until the window comes to the front.
   - With the theme on System, switching Windows between light and dark
     follows at once.
   - Ctrl+, opens Settings on a non-US keyboard layout.
   - After a real sleep of a few minutes the next check finds new mail
     within seconds; unplugging the network shows "Can't reach the server"
     once, and a flag change made meanwhile reaches the server later.
   - Narrator reads the folder pane, the list, the reader and the composer.
5. Notifications and tray
   - With the window in the background, new mail shows one Windows
     notification; clicking it brings the window back and opens that
     message.
   - No notification while the window is focused, when paused, or under
     Focus Assist.
   - Tray icon: left click opens; the menu has Open, Check mail and Pause.
   - Close with "Keep running in the notification area" on: the window
     hides and the app keeps checking.
   - "Start with Windows" on: sign out and in; Komorebi starts hidden in
     the tray.
6. Languages and sizes
   - Run once with `KOMOREBI_LANG=pt-PT`; check 1280x800 and 900x700 and
     that nothing clips.

## Known gaps

- Not yet validated against real Purelymail or Postale.io accounts beyond
  the maintainer's own testing, nor against real self-hosted servers; see
  "Providers and servers".
- The local mail cache is plaintext.
- No release is published yet; the installer and release workflow exist but
  the binary is not code-signed.
- Dovecot-style servers that keep a selected folder do not report later
  deliveries on that session. Komorebi handles it by re-selecting before an
  action on a UID at or above the folder's UIDNEXT, and by running its
  LIST-STATUS, STATUS and keepalive NOOP with no folder selected.
- The `imap` crate's parsers reject some legal unsolicited responses while
  a folder is selected, so the sync worker leaves its folder whenever it
  goes idle.
