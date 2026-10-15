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

- [ ] Installer, on a real desktop (nothing here was installed): update
      from the previous release with the app running in the tray (window
      hidden, close to tray on), with the window open, and with a second
      cache open. Check that the Restart Manager's close request
      (`WM_QUERYENDSESSION` then `WM_ENDSESSION`, which the app now
      answers by quitting) really reaches the hidden tray window, that the
      silent update starts Komorebi again (the `[Run]` entry for
      `/RESTARTAPPLICATIONS`; an app older than this change passes that
      switch too), that it is not elevated after an all-users install,
      and that the "close it first" text appears for a manual install.
- [ ] Measure the release exe size with `ed25519-compact` and `sha2`
      (limit 20.0 MB).

### Installer and release

- [ ] `mailto:` links on a real desktop: make Komorebi the default mail
      app in Settings, click a mail link in a browser with the app closed,
      open in the tray, and open in the window. Known limits: a second
      launch hands the URL to the running copy through its tray window,
      so with no tray icon (or while the first copy is still starting) it
      only comes forward and the link is dropped; with no account the
      composer says to add one.
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
      whose id has the IMAP host's registrable domain; tested for host
      name, domain, foreign, missing and spoofed headers), and that the
      server strips such headers arriving from outside. Unverified for every
      provider: a provider whose id is in another domain shows no DMARC
      notice.
- [ ] Confirm whether Purelymail saves sent mail server-side; if so, set
      `save_sent: false` on its preset in `src/providers.rs`.
- [ ] Verify the IDLE silent-server detection and the server draft Edit
      (`IdleHealth`, `adopt_server_draft`) on real Postale.io and Purelymail
      accounts; both are tested on the local fakes only.

### Reader and composer

- [x] Dark logos on a clear ground get a light plate in dimmed mail.
      Left: the plate follows the whole message being dimmed, not the
      colour right behind the image, so a dark logo on a mid-tone brand
      header gets one too.
- [x] Copying formatted text out of the composer. Left: a selection with a
      signature or a stored quote inside it (not at its ends) copies plain
      text only.
- [x] Arrow keys inside the colour flyout.
- [x] A caret blink in the composer.
- [x] Narrator on a message over 16 KB: told its length and that it reads
      the part around the cursor (Slint gives the proxy one value; a whole
      long message there would lay out on every key).
- [ ] Recipient chips stay on one line (the field scrolls to the text being
      typed); wrapping them onto more lines as the field fills is left.

### Windows 11 look

- Not planned: Mica. The software renderer draws opaque pixels, so Mica is
  not used, for performance; the solid fallback WinUI itself uses is shown.
- [ ] Check by hand on Windows that the tray menu and the system menu turn
      dark with the app (uxtheme ordinals 135/136; snapshots draw no native
      menu) and that moving the Text size slider resizes the text at once.

### Tabs and windows

- [ ] Below 720 px (one pane) the reading pane's "open in its own window"
      placeholder for a popped-out draft is not shown (the list shows);
      it needs a Back to list there first.
- [ ] Removing or re-downloading an account, or a folder vanishing, while
      one of its messages shows in a message tab leaves the tab's reader
      empty; it should say the message is gone, as a sync does.
- [ ] Settings opened from a message tab show without the folder pane, and
      F6 there still aims at the pane.

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
   - Composer windows (Ctrl+O on the composer or a Drafts row): files
     dragged onto one attach to its draft (accent border while hovering);
     Bring to front in the main window's pane brings it forward; quit from
     the tray with one open and check the draft is in Drafts.
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
- Release 0.4.1 is published; the installer is not code-signed with a
  certificate (updates are verified with an Ed25519 signature instead).
- Dovecot-style servers that keep a selected folder do not report later
  deliveries on that session. Komorebi handles it by re-selecting before an
  action on a UID at or above the folder's UIDNEXT, and by running its
  LIST-STATUS, STATUS and keepalive NOOP with no folder selected.
- The `imap` crate's parsers reject some legal unsolicited responses while
  a folder is selected, so the sync worker leaves its folder whenever it
  goes idle.
