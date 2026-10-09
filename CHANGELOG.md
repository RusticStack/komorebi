# Changelog

All notable changes to Komorebi are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/). Each entry says what it
changes in the local data (the cache, its folders and Credential Manager).

## [0.2.0] - Unreleased

The first version of the rewrite: a small Slint app in place of the
GPUI/AI prototype. It has not been published as a release yet (the
installer and release workflow exist; code signing waits for a
certificate; see TODO.md).

### Added

- Accounts: Purelymail and Postale.io presets and manual IMAP/SMTP
  settings (SSL/TLS or STARTTLS), checked by signing in before they are
  saved; editing in place, signatures, an Enabled switch, ordering and a
  colour dot. Passwords live only in Windows Credential Manager.
- Sync: header-first, newest first; one-command steady-state passes with
  LIST-STATUS and CONDSTORE; IDLE on the inbox; inbox body prefetch;
  resume after sleep; one retry of a failed action on reconnect.
- Reading: a built-in HTML renderer (no web engine, no scripts, remote
  images only with consent), scaling of fixed-width mail, dimming in the
  dark theme, colour emoji, text selection, links behind a confirm,
  attachments (Save, Open with a risky-type block, Mark of the Web), the
  sender notice, encrypted-mail state, List-Unsubscribe, meeting invites
  (card, RSVP to the organizer, Save .ics), Save as .eml, Show original,
  Forward as attachment.
- Writing: drafts saved locally and to the server Drafts folder, an Outbox
  with undo send, send guards, Markdown-lite formatting sent as an HTML
  alternative, HTML quoted replies, recipient suggestions, attachments by
  dialog, drop or paste.
- Organizing: multi-select, a row menu, Junk and Not junk, undo for
  archive, delete, move and junk, Unread and Flagged filters, Mark all
  read, accent-folded search in one folder or all.
- Settings in four sections (Accounts, General, Privacy, About), theme
  choice, check interval, mark-as-read delay, an opt-in taskbar flash, an
  opt-in diagnostics log; the unread count in the window title; window
  place restored; keyboard access throughout (focus ring, F6 panes,
  shortcuts).
- Reliability: a crash note (`crash.log`, no mail content), a damaged
  cache renamed aside, a clear message for a cache from a newer version,
  one app per cache.
- Release foundations: LICENSE (Apache-2.0), NOTICE, this changelog, the
  app icon (also in the title bar and taskbar), version information and a
  manifest in `komorebi.exe` (UTF-8 code page, Windows 10/11, asInvoker),
  a title bar that follows the app's theme, and CI gates: clippy with
  warnings as errors, a release build with a 15.0 MB size check, an
  offline boot snapshot on a seeded cache, cargo-deny for licenses and
  advisories, and `scripts/check-pins.ps1`.
- Installer and data removal: a per-user Inno Setup installer (no
  administrator rights, `%LOCALAPPDATA%\Programs\Komorebi`, Start menu
  shortcut, an uninstaller that asks, default No, whether to delete the
  mail cache folder), a portable zip, a release workflow (tag check,
  tests, boot snapshot, optional signing, `SHA256SUMS`, a draft release
  with the `.pdb` beside it), and Settings > Privacy > Remove all data.

### Local data

The cache (`%LOCALAPPDATA%\komorebi\cache.sqlite3`) stays plaintext. Each
schema version migrates the one before it in place; see PRIVACY.md for
everything kept on disk.

- v2: message bodies in their own table.
- v3: each folder's sync fingerprint (its last STATUS).
- v4: the senders whose remote images load without asking.
- v5: saved drafts and their attachment copies (`cache.drafts` beside the
  cache).
- v6: the Outbox: each message as it will be sent, attachments encoded in
  it, until it is sent, discarded or its account removed.
- v7: account signatures and a settings table.
- v8: an address index (addresses and display names of senders and of the
  recipients of sent mail), deleted with the account.
- v9: each draft's change count and where its server copy is; drafts are
  also kept in the account's own Drafts folder on the server.
- v10: answers to meeting invitations (UID, occurrence, version, answer,
  time; nothing else from the invite).
- v11: each account's Enabled flag; settings now include the window's
  place.
- Beside the cache: `crash.log` and `crash.seen`, the opt-in `logs` folder
  (metadata only), `cache.open` (opened attachments, deleted at the next
  start) and, after damage, `cache.damaged-<date>.sqlite3`.
- Credential Manager: one generic credential per account,
  `account:<n>:password.komorebi`.
- The release-foundations work changes no local data.
- Remove all data deletes every account's credential, then the cache and
  everything beside it listed above, then the `komorebi` folder if it is
  empty. The uninstaller deletes `%LOCALAPPDATA%\komorebi` only if you
  answer Yes, and never touches Credential Manager. No schema change.

[0.2.0]: https://github.com/RusticStack/komorebi/releases/tag/v0.2.0
