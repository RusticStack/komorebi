# Komorebi

A small, fast email client for Windows 10/11. It is free and open source,
uses a Fluent 2 look, and does one job: reading and writing mail without
getting in the way.

- **Providers:** built-in presets for [Purelymail](https://purelymail.com) and
  [Postale.io](https://postale.io), plus manual settings for any IMAP/SMTP
  server (TLS or STARTTLS only).
- **Multiple accounts**, each with its own folders, plus a combined
  "All inboxes" view. Adding an account signs in to both servers first
  (IMAP, then SMTP; nothing is sent). Settings → Edit changes an account in
  place, keeping its downloaded mail: name, password (left empty, the saved
  one stays; it is never shown), servers with SSL/TLS or STARTTLS for IMAP
  and SMTP, the Sent copy and the signature. A new password or other
  servers are checked the same way before anything is saved, and the error
  says which server refused. Each account in Settings shows its last sync
  ("Up to date, checked 12:41", "Syncing…", "Can't reach …"). A rejected
  sign-in marks the account in the navigation (a red dot; in the narrow
  rail, a red divider) and offers
  "Update password" (on the status line and in Settings). Remove asks
  first; mail on the server is not affected. Each account has an Enabled
  checkbox: switched off, it keeps its downloaded mail (shown greyed in
  the navigation) but opens no connections, and mail sent from it waits
  in the Outbox until it is switched on again. Up and down buttons set the
  account order; with more than one account, each has a colour dot that
  marks its rows in All inboxes and its name in the navigation.
- **Settings** (Ctrl+, or the button under the navigation; Esc closes,
  or goes back from the account form) has four sections. Accounts: the
  list above, and Add account, which opens the form as a page of its own.
  General: the theme (System follows Windows, or always Light or Dark;
  an open HTML message is laid out again), how often to check for new
  mail (every 1, 2, 5 or 15 minutes; every folder is still synced in
  full at least hourly), the undo-send delay (off, 5, 10, 20 or 30
  seconds), when an opened message is marked read (right away, after 1 or
  3 seconds, or never), and an opt-in taskbar flash when new mail arrives
  while the window is in the background. Privacy: the senders whose
  images always load, Download again per account, which forgets that
  account's downloaded mail and fetches it once more (it asks first; mail
  on the server, drafts, the Outbox and the settings are not affected),
  an opt-in diagnostics log (off by default; see Connection trouble), and
  Remove all data: after a prompt it deletes every account's saved
  password from Credential Manager, then everything in the data folder
  (downloaded mail, drafts, the Outbox, settings, logs, crash notes and
  damaged caches set aside), and closes Komorebi; mail on the server is
  not affected. If a password can't be deleted, it stops there and no
  file is removed.
  About: the version, the license, the data folder and the repository.
- **Title and window:** the window title shows the inboxes' unread count
  ("Inbox (7) - Komorebi"). The window opens where it was closed, at the
  same size (maximized if it was), moved onto a screen that still exists
  if its monitor is gone. The title bar is dark or light with the theme
  chosen in Settings.
- **Reading:** HTML mail is drawn by a small built-in renderer (no web
  engine, no scripts); plain-text mail and anything the renderer can't
  handle show as text. Fixed-width mail (a 600 px newsletter or statement)
  in a narrower reader is shown scaled down to fit, sharp, down to 60%;
  below that it scrolls sideways (Shift+wheel). Emoji are drawn in colour
  (Segoe UI Emoji). In the dark theme, mail that has no dark design of its
  own is dimmed: light backgrounds turn dark and dark text light, keeping
  text at least as readable as the sender made it (images are left as they
  are); "Show original colours" in the toolbar turns that off for the
  message shown. A paragraph longer than 64 KB is cut, with a note, and
  "View as plain text" shows all of it. Remote images are blocked: a bar above the message
  says how many, and loads them once or always for that exact sender
  address (Settings lists those senders, with Remove and Forget all).
  Images that still could not be loaded (no HTTPS on the host, not found,
  too large, too slow) are counted in the bar ("2 images couldn't be
  loaded") once the rest are in.
  Images load over HTTPS only: `http` image addresses are requested over
  HTTPS, and plain HTTP is never used. Remote stylesheets and fonts and tracking pixels never load. Text on the page
  can be selected (drag, double-click for a word, triple-click for a
  paragraph, Ctrl+A) and copied (Ctrl+C or right-click), and "View as
  plain text" in the toolbar shows any HTML message as text. Links show
  their real target at the bottom of the reader; web links open in your
  browser only after a prompt that shows the full address, its site in
  bold, and warnings for look-alike tricks (link text naming another site,
  international look-alike names, a user name before the site, a bare IP
  address). `mailto:` links open the composer, links within the message
  scroll to their target, and every other kind of link does nothing. Tab
  and Enter work the links from the keyboard. Attachments can be saved or
  opened (a click on the attachment): Open writes a copy beside the cache
  (cleared at the next start) and hands it to the app for its type;
  programs and scripts (`.exe`, `.bat`, `.js`, `.lnk`, `.msi`, ... also
  `invoice.pdf.exe`) are never opened, only saved. Saved and opened copies
  are marked as downloaded from the internet (Mark of the Web), so Windows
  and apps such as Office treat them with care. The list leaves out images the page already draws, the calendar copy of
  an invite that also comes as an `.ics` file, and the signature part of
  signed (S/MIME or PGP) mail, which is not verified. To and Cc show
  display names.
- **Message source:** the reader's More (…) menu and the row menu offer, for one
  message: Save as .eml (Ctrl+S; the message exactly as the server sent
  it, offered as `<subject> (<date>).eml` in Downloads, marked as
  downloaded), Show original (Ctrl+U; every header field unfolded and
  decoded, then the body source, cut at 256 KB with how much is left out;
  control and invisible reordering characters show as `<U+XXXX>`; Copy
  headers and Copy all), Copy Message-ID (`<id@host>`, also shown in the
  original view), and Forward as attachment (a new message with the
  original attached whole as `<subject>.eml`, type message/rfc822, as it
  was received with all its header fields; the usual size limit applies).
  A message not downloaded yet is fetched first ("Fetching message…").
- **Sender notice:** one quiet line above the message says when to look
  twice at the sender, with at most two reasons: your own server recorded
  a failed DMARC check (or failed SPF and DKIM with nothing passing; only
  the topmost `Authentication-Results` header whose server name is in your
  IMAP server's domain counts, so ones the sender wrote further down are
  ignored), the display name names another domain than the address has
  ("paypal.com Security" from `mailer.example.net`), the sender's domain
  is an international (`xn--`) name (shown decoded), or a part of it mixes
  Latin with Cyrillic or Greek letters. The real address always shows
  next to the name. Nothing is verified or looked up; the domain
  comparison uses the last two labels (three under `co.uk`, `com.br`,
  `com.pt` and the like), not the public suffix list.
- **Encrypted mail** (S/MIME, PGP/MIME or an inline PGP block) shows "This
  message is encrypted (S/MIME). Komorebi cannot decrypt it" instead of
  ciphertext, and the encrypted part is offered as one file to save
  (`smime.p7m`, `encrypted.asc` or `message.asc`) for a program that has
  the key. The list preview reads "Encrypted message".
- **Unsubscribe:** mail with a `List-Unsubscribe` header shows
  "Unsubscribe" beside the recipients. Its `mailto:` address (preferred)
  opens the composer filled in with the address, subject and text the list
  asks for, for you to send; otherwise its `https:` address goes through
  the same confirm prompt as a web link. Nothing is sent or fetched until
  you do so; plain `http:` addresses and one-click unsubscribe are not
  used.
- **Meeting invitations:** a message with a calendar invite (a
  `text/calendar` part or an `.ics` file) shows a card above the body:
  the title, when (in your time zone), the organizer, where, a short
  repeat line ("Repeats weekly on Mon, Wed until 2027-12-31"; anything
  more involved just says "Repeats"), and a line for a cancelled
  (struck-through title), updated or past event. Join opens the meeting
  link through the usual link prompt, Details shows the description, and
  Save .ics saves the calendar file as `invite.ics`. A request you can
  answer offers Accept, Tentative and Decline; a prompt names the one
  recipient first, with a warning when the organizer is not in the
  sender's domain or you are not listed. The reply goes **only to the
  organizer** (never Cc or the other attendees) through the Outbox, sent
  at once (no undo window; it can be retried or discarded there, not
  edited), and "Don't send a reply" keeps the answer on this computer
  only. The card then reads "You accepted" with Change. Times in UTC
  convert to your zone; a named zone is converted only with the
  definition the invite carries (there is no built-in zone table), else
  the time shows with the zone's name ("09:30 (Europe/Lisbon)"); dates
  are all-day. Only the first event in a calendar is read, nothing is
  added to a calendar, and an invite that can't be read (or is over 256
  KB) shows as an ordinary message.
- **Writing:** compose, reply, reply all and forward, with attachments:
  attach from the dialog, drop files from Explorer on the window while the
  composer shows (it gets an accent border), or copy them in Explorer and
  press Ctrl+V in the composer (text still pastes as text). Files are read
  off the UI thread, folders are refused, and the 100 MB limit holds. A line
  above the attachments counts their size as sent ("Attachments 18.4 MB of
  25 MB"); it turns amber from 80% and red over 25 MB, when Send is off.
  The account's signature (plain text) starts every new message, reply and
  forward, below the standard `-- ` line and above the quoted text;
  picking another From account swaps it while it is unchanged, and the
  signature of a message replied to is left out of the quote.
  Drafts are saved on this computer two seconds after you stop typing, and
  at once when the composer closes (Esc), another message takes its place,
  the From account changes or the window closes; attachments are copied
  when added. Starting another message (a reply, a `mailto:` link, or New
  mail while the composer shows) keeps the draft in progress, and New mail
  brings it back once the composer is closed. "Drafts" in the navigation
  (shown while drafts are saved, with their count) lists them; a click
  opens one. Reply on a message that has a draft reopens that draft, and
  the message shows a "Draft" tag in the list. At start, saved drafts are
  offered once ("Resume draft"); the composer never opens by itself.
  Discard asks first when the draft has anything you wrote. Saved drafts
  are also copied to the account's Drafts folder on the server (flagged as
  a draft, not as read): at most every 30 seconds while you type, when the
  composer closes, and after the next sync for drafts written offline. Each
  new copy replaces the previous one, which is removed like a delete (by
  UID only on servers with UIDPLUS, otherwise only flagged as deleted);
  sending or discarding the draft removes its copy too. No folder is
  created: without a Drafts folder, or with attachments over 10 MB, drafts
  stay on this computer (said once per session). Opening the server's
  drafts in the composer is not supported yet. A wrong address shows a note under
  its field when you leave it, and a line under the toolbar says why Send
  is off. Before sending, a bar asks once ("Send anyway") when the subject
  is empty or your text mentions an attachment (attach, attached,
  attachment, anexo, em anexo, anexado...) but nothing is attached; quoted
  and forwarded text doesn't count.
- **Recipient suggestions:** after two characters of an address in To, Cc
  or Bcc, up to six addresses whose address or name starts with them show
  under the address fields: people who wrote to the From account and
  people you sent mail to from it (those rank higher, then the most
  recent). Up/Down pick one, Enter or Tab take it (as `Name <address>, `),
  Esc closes the list; a click takes one too. The addresses come from an
  index of the account's cached mail (not Junk or Drafts, and never your
  own address), built as mail arrives; for mail cached before this
  version, the first sync after the update fills it in the background.
  There is no address book to edit.
- **Formatting and HTML replies:** the composer is plain text, and a few
  Markdown-style markers format the sent message: `**bold**`, `*italic*`
  or `_italic_`, `- ` and `1. ` lists (two spaces nest one level; a
  numbered list starts at 1), `> ` quotes and `[label](https://...)`
  links (bare http(s) addresses become
  links too; only http, https and mailto links are made). A message with
  bold, italic, a list or such a link gets an HTML version beside the
  plain text, whose links read `label (url)`; anything else goes out as
  plain text only, exactly as typed. A reply to or forward of an HTML
  message keeps the `> ` quote in the composer and sends the original's
  HTML below your text, with its images (inline ones too), remote styles,
  scripts and forms removed; a forward shows the original's From, Sent,
  To and Subject above it. If you change the quoted text (or delete the
  "On ... wrote:" line), the text is sent as you edited it instead.
  A small toolbar above the body types the markers for you: Bold
  (Ctrl+B) and Italic (Ctrl+I, written `_italic_`) wrap the selection
  (whole words), or put an empty pair at the caret, and take the markers off again when
  it is already wrapped; Bulleted list (Ctrl+Shift+8), Numbered list
  (Ctrl+Shift+7) and Quote act on the lines the selection touches, and
  off again when they all have it; Link (Ctrl+K) asks for an address
  (`example.org` becomes `https://example.org`, `name@example.org` a
  mailto link) and makes the selection `[label](address)`. Ctrl+Z right
  after one of these undoes it. The text stays plain: there is no preview
  and no underline, headings, fonts or colours.
- **Sending:** Send closes the composer at once and the message waits in
  the Outbox for 5 seconds (the delay is set in Settings → General; "Sending
  in 5 s", with Undo, which brings the draft back unchanged). It is then sent, and a copy of exactly the bytes
  sent is saved to Sent. A message that surely did not reach the server
  (no connection, or the server said "try later") stays in the Outbox and
  is tried again by itself after 30 s, doubling (the Outbox is checked
  at each check for new mail), at most 6 times; one the
  server refused, or that ran out of tries, is marked failed. If the
  connection breaks during a send, the message may or may not have gone
  out: it is marked as such and never sent again by itself. "Outbox" in
  the navigation (shown while it holds messages) lists them with their
  state; failed and unsure ones offer Edit (back to the composer), Retry
  (asks first when it may have been sent) and Discard. Messages in the
  Outbox survive a restart; a send cut off by closing the app is never
  resent automatically. Removing an account that still has drafts or
  Outbox messages asks first and says how many will be lost.
- **One window:** starting Komorebi again brings the running window to
  the front (one app per cache file; a throwaway `KOMOREBI_DB` runs
  beside it).
- **Organizing:** read/unread, flag, archive, delete and move, all
  two-way with the server. Several messages can be selected at once:
  Ctrl+click adds or removes one, Shift+click or Shift+Up/Down selects a
  range, Ctrl+A in the list selects the whole view, and Esc clears. The
  reader pane then shows how many are selected, with Archive, Delete, Move
  (when they are all in one account), Mark read / unread and Flag /
  Unflag; Delete and Ctrl+Q act on the selection too. The list changes at
  once, and the server gets one command per folder for the whole batch
  (in UID sets of up to 4,000 characters), never one per message.
  Right-clicking a message opens a menu (Reply, Reply all, Forward, Mark
  as read / unread, Flag, Archive, Delete, Move to, Report junk) that acts
  on the selection; a row outside the selection is selected first,
  without opening it or marking it read. The same menu is under More
  (…) in the reader.
- **Search:** the search box (Ctrl+F) matches the subject, sender, To, Cc
  and preview of the cached messages, ignoring case and accents ("joao"
  finds "João", "strasse" finds "Straße"), so Sent can be searched by
  recipient. While text is typed, an "All folders" toggle searches every
  cached folder of every account instead of the view shown; each result
  then shows its folder. The scope goes back to the view when another
  folder is opened. Drafts and the Outbox search the same way. Only
  cached headers and previews are searched, never the server or full
  message bodies.
- **Filters and read state:** Unread and Flagged toggles under the search
  box narrow the list (with the search, too); a message read while Unread
  is on stays in the list until it reloads. More (…) beside Refresh offers
  "Mark all as read" for the folder shown (in All inboxes, every inbox):
  the list changes at once and the server gets one command per folder,
  covering mail up to the newest message listed, so mail that arrived
  since the last check stays unread. An opened message is marked read
  after it has been open for a second, so arrowing through the list
  leaves the messages passed on the way unread (Settings → General:
  right away, after 1 or 3 seconds, or never). An empty list says why: no matches (with Clear search),
  no unread or flagged messages, or that the last check for mail failed.
- **Junk and Undo:** Report junk moves messages to the account's Junk
  folder (with none, nothing moves and a line says so); in Junk, Not junk
  marks them read and moves them to the Inbox. Archive, Delete, Move and
  Junk show "Moved to Archive" with Undo (or Ctrl+Z) for 5 seconds: the
  list changes at once, but the server is only told when that time is up,
  when the next such action starts, or when the window closes (which then
  waits up to 3 seconds for it to reach the server). Undo puts the
  messages back as they were; the server never saw a thing.
- **Keyboard and screen readers:** every button and toggle is reached
  with Tab and pressed with Space or Enter, and the focused one shows a
  2 px ring. The folders, the message list, the reader's toolbar and the
  composer's formatting buttons are one Tab stop each, moved along with
  the arrows (the list and folders
  show their current row once a key has moved it); F6 moves between the
  folders, the list and the reader. Dialogs start on a safe button (a
  prompt on Cancel, so a stray Enter never confirms), keep Tab inside
  them, close with Esc and give the focus back where it was (the list,
  the composer, or Settings under a prompt). Controls carry names and states for screen
  readers (the list and folders by name, each message with its unread,
  flagged, attachment and replied state, toggles as checked, accounts as
  expanded or collapsed, the reader's header fields), and the status line
  is announced as it appears, at once when it is an error. Accent-coloured
  text is darkened or lightened from the Windows accent until it reads at
  4.5:1 on the surfaces it sits on, and unread subjects use the text
  colour.
- **Private:** no telemetry, no ads, no AI. Passwords stay in Windows
  Credential Manager.
- **Connection trouble:** an error says what happened in one sentence
  with a next step (offline, sign-in rejected, untrusted certificate,
  message too large, mailbox full, too many connections), once per
  account until a sync goes through again (Refresh shows it again). A
  server that is busy or at its connection limit never counts as a wrong
  password. An archive, delete, move or flag change that couldn't reach
  the server stays done on screen and is tried once more as soon as a
  connection works (for up to 5 minutes); only if that fails too does the
  list go back to the server's state. A message another app already
  removed counts as done ("Already gone from the server" for a move).
  After the computer sleeps (a gap in the clock of over 2 minutes),
  every connection is replaced and all accounts sync 3 seconds later.
  A first sync of a large folder shows "downloading 12,400 of 103,000"
  on the status line and in Settings. If a folder disappears from the
  server while it is open, the view moves to the account's inbox.
- **Damaged cache:** if SQLite reports the cache file as damaged, it is
  renamed aside with the date (`cache.damaged-20261009-101500.sqlite3`,
  with its drafts folder) and a new cache is created. The accounts were
  stored in it, so they must be added again (with their passwords); the
  status line says so and names the old file. A cache made by a newer
  Komorebi is never touched: the app says so and does not start.
- **Diagnostics log** (Settings → Privacy, off by default):
  `logs\komorebi.log` in the data folder, at most 1 MB plus two older
  files. A line holds only the time, an account number, a command name,
  its status or response code and how long it took (`account 1 FETCH OK
  12 ms`): never mail, addresses, subjects, folder names, server text or
  passwords.
- **Crash note:** if the app ever stops on an internal error, it appends one
  line (time, version, thread and source location, never mail text) to
  `crash.log` next to the cache, and says so once on the next start.

See [`Project Komorebi - Email Client Plan.md`](Project%20Komorebi%20-%20Email%20Client%20Plan.md)
for scope, non-goals and the performance budgets.

## Install

No release has been published yet (see TODO.md); until then, build from
source below. Releases (GitHub Releases, Windows 10/11 x64) come in two
forms:

- `komorebi-<version>-setup.exe` installs for the current user only, with
  no administrator rights, into `%LOCALAPPDATA%\Programs\Komorebi`, with a
  Start menu shortcut and an uninstaller (Settings → Apps). `/SILENT` or
  `/VERYSILENT` installs without questions; a running Komorebi is closed
  first. Uninstalling asks, with No as the default, whether to delete the
  mail cache folder `%LOCALAPPDATA%\komorebi` too (a silent uninstall keeps
  it). It never touches the saved passwords: use Settings → Privacy →
  Remove all data before uninstalling, or remove the
  `account:<n>:password.komorebi` entries in Credential Manager.
- `komorebi-<version>-portable-x64.zip`: `komorebi.exe` with the license,
  notices, privacy statement, README and changelog. Unzip it anywhere and
  run it; it keeps its data in `%LOCALAPPDATA%\komorebi` as the installed
  app does.

**Unsigned builds.** Until a code-signing certificate is set up, the
releases are not signed, so Windows SmartScreen may say "Windows protected
your PC" the first time: check the checksum below, then choose More info →
Run anyway.

**Checksums.** Each release lists the SHA-256 of every file in
`SHA256SUMS`. In PowerShell, compare:

```powershell
(Get-FileHash .\komorebi-0.2.0-setup.exe -Algorithm SHA256).Hash.ToLower()
Select-String komorebi-0.2.0-setup.exe .\SHA256SUMS
```

Releases are built by `.github/workflows/release.yml` from a `vX.Y.Z` tag
that must equal the Cargo version: tests, the release build, its size
check and the offline boot snapshot, an optional signing step (only when
the `SIGN_CERT_PFX` and `SIGN_CERT_PASSWORD` secrets exist; see
`scripts/sign.ps1`), the Inno Setup installer (`installer/komorebi.iss`),
the portable zip, `SHA256SUMS`, and a draft release with this version's
CHANGELOG section and the debug symbols (`.pdb`) as a separate file. Run by
hand (Actions → Release → Run workflow), it is a dry run that keeps the
packages as a workflow artifact and publishes nothing.

## Install from source

To build Komorebi yourself on Windows 10/11 (x64):

1. Install [Rust](https://rustup.rs) (rustup picks the toolchain pinned in
   `rust-toolchain.toml`) and the Visual Studio Build Tools with the
   "Desktop development with C++" workload (the MSVC linker, the Windows
   SDK and its `rc.exe`, which embeds the icon, version information and
   manifest).
2. Build and run:

   ```powershell
   git clone https://github.com/RusticStack/komorebi
   cd komorebi
   cargo build --release --locked   # target\release\komorebi.exe
   .\target\release\komorebi.exe
   ```

`komorebi.exe` is a single file (the C runtime is linked in); copy it
anywhere. Its data lives in `%LOCALAPPDATA%\komorebi` (see PRIVACY.md).

Stack: Rust + [Slint](https://slint.dev) (Fluent style, software renderer),
`rusqlite`, `imap`, `lettre`, `mail-parser`, `keyring-core` (with the Windows Credential Manager store), `rfd`.

## Checks

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
pwsh scripts/check-pins.ps1        # exact-pinned crates linked once
cargo deny --locked check licenses advisories   # cargo install cargo-deny
```

CI (`.github/workflows/ci.yml`) runs these, then the release build, its
size check and the boot snapshot below.

## Verifying a build

After `cargo build --release --locked`:

- **Size:** `target\release\komorebi.exe` must be at most 15,000,000
  bytes (`(Get-Item target\release\komorebi.exe).Length`).
- **Version:** `(Get-Item target\release\komorebi.exe).VersionInfo`
  shows the Cargo version, and Explorer shows the icon.
- **Boot:** run it offline against a new seeded cache; it renders the
  window to a file, checks that the list and the reader are not blank and
  exits non-zero if either is (this touches no mail and no network):

  ```powershell
  cargo run --release --locked --example seed -- $env:TEMP\seed.sqlite3 2000
  $env:KOMOREBI_OFFLINE = 1; $env:KOMOREBI_DB = "$env:TEMP\seed.sqlite3"
  $env:KOMOREBI_SNAPSHOT = "$env:TEMP\boot.bmp"; $env:KOMOREBI_SNAPSHOT_SELECT = 0
  $env:KOMOREBI_SNAPSHOT_CHECK = 1; $env:KOMOREBI_TIMING = 1
  (Start-Process target\release\komorebi.exe -NoNewWindow -Wait -PassThru).ExitCode   # 0
  ```

  `KOMOREBI_TIMING` prints the launch timings (the budget is 200 ms to the
  first frame).

## Keyboard

| Key | Action |
|---|---|
| Ctrl+N | New mail |
| Ctrl+, | Settings (Esc closes them; from the account form, Esc goes back to the list first) |
| Ctrl+Enter or Alt+S | Send (in the composer) |
| Ctrl+Shift+C / Ctrl+Shift+B | Show Cc and Bcc, and go to Cc / Bcc (in the composer) |
| Ctrl+Shift+A | Attach files (in the composer) |
| Ctrl+V | Attach the files copied in Explorer (in the composer; text pastes as usual) |
| Ctrl+B / Ctrl+I | Bold / italic markers on the selection (in the message body) |
| Ctrl+K | Link the selection to an address (in the message body; Enter inserts, Esc cancels) |
| Ctrl+Shift+8 / Ctrl+Shift+7 | Bulleted / numbered list on the selected lines (in the message body; the key that types `8` / `7`) |
| Up / Down, Enter or Tab, Esc | Pick, take or close the recipient suggestions (while they show under an address field) |
| Esc | Close the composer, saving the draft |
| Tab / Shift+Tab | Next / previous control; the folders, the list, the reader's toolbar and the composer's formatting buttons are one stop each |
| F6 / Shift+F6 | Next / previous pane: folders, message list, reader (toolbar, composer or the selection's buttons) |
| Space or Enter | Press the focused button or toggle |
| Left / Right | Move along the reader's toolbar or the formatting buttons (while they have the focus) |
| Up / Down, Home / End, Enter or Space | Move through the folders and open one (while they have the focus; Left / Right fold an account) |
| Ctrl+R / Ctrl+Shift+R | Reply / Reply all |
| Ctrl+F | Search |
| Ctrl+Q or Ctrl+Shift+U | Toggle read (on a selection: all read if any is unread) |
| Ctrl+S | Save the message as a .eml file |
| Ctrl+U | Show the original (header fields and source; Esc closes it) |
| Insert | Toggle the flag |
| Ctrl+E | Archive (not while typing in the search box) |
| Up / Down | Previous / next message (with none open, the first) |
| Home / End | First / last message |
| PageUp / PageDown | Up / down by the rows the list shows |
| Enter | Open the selected message (with none, the first) and move to the page, where PageUp/PageDown, Space and Tab work |
| Esc | Clear the search, else the selection and the open message, else the status line (not while it offers Undo or Resume draft; Esc on its buttons always closes it) |
| Ctrl+click / Shift+click | Add or remove a message from the selection / select a range |
| Shift+Up / Shift+Down | Extend the selection |
| Ctrl+A | Select every message in the list (with the list focused) |
| Delete | Delete (move to Trash) |
| Shift+F10 or Menu | The row menu for what is selected, under the current row |
| Ctrl+Z | Undo the last archive, delete, move or junk report (while it is offered) |
| F5 | Refresh (also retries a rejected sign-in) |

The keys that act on the selected message (reply, toggle read, flag,
archive, Delete, and the list keys above) work only while the reader
shows, never in the composer or Settings. The list has the keys from the
start, before any click.

## Development switches

| Variable | Effect |
|---|---|
| `KOMOREBI_DB=path` | Use this cache file instead of `%LOCALAPPDATA%\komorebi\cache.sqlite3` (`crash.log` and the diagnostics log's `logs` folder then go beside it too, draft attachments into `<path without extension>.drafts` and opened attachments into `<path without extension>.open`; each cache file has its own single-instance lock) |
| `KOMOREBI_PASSWORD=secret` | With `KOMOREBI_DB`, use this password instead of Credential Manager (debug builds, or release built with `--features dev-env-password`) |
| `KOMOREBI_FLOW=dir` | Debug builds only: script a full walk through the app (open, flag, read/unread, reply, a formatted message with an attachment whose draft must be saved and whose HTML part must arrive, move to Junk and back) on `[komorebi-test]` messages only, saving a log and snapshots to `dir` |
| `KOMOREBI_NO_SERVER_DRAFTS=1` | Debug builds only: keep drafts on this PC instead of copying them to the server's Drafts folder (used with `KOMOREBI_FLOW`, so a live test adds and removes no server messages) |
| `KOMOREBI_OFFLINE=1` | No network and no credential access (demo caches, timing runs) |
| `KOMOREBI_TIMING=1` | Print startup timings to stderr (up to the first frame drawn), and how long each opened message took to its first tile |
| `KOMOREBI_SNAPSHOT=file.bmp` | Render the window to a file and exit (always at the default place and size; the saved window place is ignored). Options: `KOMOREBI_SIZE=WxH`, `KOMOREBI_THEME=light` or `dark` (over the theme chosen in Settings), `KOMOREBI_SNAPSHOT_SELECT=n` (or `a-b`: open message a, then Shift+Down to b, showing the "N selected" panel), `KOMOREBI_SNAPSHOT_WALK=n` (then open the next n messages, 250 ms apart), `KOMOREBI_SNAPSHOT_SCROLL=px` (scroll the reader), `KOMOREBI_SNAPSHOT_ACTION=reply`, `compose`, `compose-invalid` (a new message with a wrong To address, after leaving the field), `compose-format` (a new message with formatting markers, and the link field open with an address typed), `compose-suggest` (a new message with `al` typed into To: the recipient suggestions), `compose-attach` (a new message with three made-up attachments, the counter at 85%), `drafts` (the Drafts view), `discard` (the newest saved draft, then the Discard prompt), `guard` (the newest saved draft, then Send: the send guards' bar), `outbox` (the Outbox view with its newest message), `settings`, `settings:general`, `settings:privacy`, `settings:about` (or `settings:accounts`: Settings on that section), `settings-edit` (Settings with the first account's edit form), `remove-data` (Settings → Privacy with the Remove all data prompt; nothing is removed until it is confirmed), `plain` (plain-text view), `original` (Show original colours, in the dark theme), `source` (Show original on the selected message), `rsvp` (Accept on the selected invitation: the reply prompt; nothing is sent until it is confirmed), `select-all` (select and copy the page), `scroll-right` (scroll a page wider than the reader to its right edge), `archive` (archive what is selected: the Undo offer; the move is held, so the cache is not changed), `junk` (Report junk on what is selected: on the seeded cache, which has no Junk folder, the line saying so), `offline` (the first account's status line as when its server can't be reached), `offline-settings` (the same, then Settings with that account's line), `unread` (the Unread filter) or `read-all` (Mark all as read, then the Unread filter: its empty state; this changes the cache, so use a copy). `KOMOREBI_SNAPSHOT_SEARCH=text` types `text` into the search box first (with `KOMOREBI_SNAPSHOT_ACTION=search-all`, in the "All folders" scope; `joao` on the seeded cache). `KOMOREBI_SNAPSHOT_FOCUS` then moves the keyboard focus with real key presses from the list: `toolbar` (Tab: the reader's first button; `toolbar:n` then Right n times), `nav` (Shift+F6 to the folders; `nav:n` then Down n times), `list` (the list's current row) or `tab:n` (n Tabs). `KOMOREBI_SNAPSHOT_CHECK=1` then exits with code 1 (and says which on stderr) if the message list or the reader came out blank (less than 1% of their pixels unlike the background); CI uses it with `KOMOREBI_SNAPSHOT_SELECT=0` |

Measurement tools against an account you authorize (credentials from an
env file with `EMAIL` and `PASSWORD_EMAIL`; output is numbers only):
`examples/probe` (read-only; `--send` mails the account itself once),
`examples/idle_check` (IDLE wake test), `examples/delivery_delay` and
`examples/modseq_check` (read-only), `examples/parse_cache` (runs every
cached body through the reader's parser).

`cargo run --release --example seed -- new.sqlite3 50000` fills a new cache
with a fake `.invalid` account and deterministic messages for timing runs
(open it with `KOMOREBI_OFFLINE=1` and `KOMOREBI_DB`). One sender has
accents ("João Gonçalves": an inbox message, two in Archive and one sent to
him), for checking the search. `KOMOREBI_TIMING=1` also prints how long
each search took.
`seed -- new.sqlite3 0 tests/fixtures/html` holds just the 14 synthetic HTML
messages, in file order (`KOMOREBI_SNAPSHOT_SELECT=n` opens file n+1), for
checking the reader; `tests/fixtures/notice` holds ten synthetic messages for
the sender notice, encrypted mail and Unsubscribe, and `tests/fixtures/invite` eight meeting invitations, and `tests/fixtures/reader` an emoji message and three heavy ones (a long newsletter, a 120-message digest, a 1,200-row statement) for timing the reader (see their READMEs). Add `--drafts` for two saved drafts (a new message and
a reply to the newest message), and `--outbox` for a message whose send failed
(in the Outbox) plus a newer draft with no subject that mentions an
attachment, and `--signature` for an account signature. With an eml-dir,
`--html-reply` adds, newest, a reply to the newest HTML message as Komorebi
sends one (formatted text, signature and the sanitized HTML quote), as its
sender would receive it (`KOMOREBI_SNAPSHOT_SELECT=0` opens it). `examples/peak_memory` runs a program (the app, with
those variables set) and prints its peak working set and private memory.

## License

Apache-2.0 (LICENSE). Third-party notices are in NOTICE; Slint is used under
its royalty-free license, and the attribution appears in Settings → About.
Changes are listed in CHANGELOG.md.
