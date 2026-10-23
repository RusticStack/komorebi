# Komorebi Privacy Statement

Last updated: 2026-10-23

Komorebi is a local-first email client with zero telemetry. It does not
collect analytics, run ads, upload crash reports, or send mailbox content
through any Komorebi service. There is no Komorebi service.

## Data inventory

Everything Komorebi keeps on your computer, all in
`%LOCALAPPDATA%\komorebi` (beside the cache file named by `KOMOREBI_DB`
when that development switch is set), except the passwords:

| Where | What | Removed |
|---|---|---|
| `cache.sqlite3` (with `cache.sqlite3-wal` and `-shm`) | Accounts (no passwords), settings, folders, message headers and previews, downloaded messages, drafts, the Outbox, invitation answers, the address index, the image-sender list, the calendar (events of the local "On this PC" calendar, see below) | With the file; per account when it is removed |
| `cache.sqlite3`, rules tables | Your rules (names, conditions, actions) and the activity of automatic runs: time, rule name, the message's Message-ID, subject and sender, what was done and where to, at most 500 lines per account. Nothing of it leaves this computer; the diagnostics log gets counts only | With the file; per account when it is removed; Clear activity removes the activity |
| `cache.drafts\<draft number>\` | Copies of a draft's attachments | With the draft |
| `cache.sqlite3.key` (only for an encrypted cache) | The cache's key, protected by Windows (DPAPI) for your Windows user | With the cache, by Remove all data |
| `cache.open\<random>\` | Attachments you opened | At the next start |
| `crash.log`, `crash.seen` | One line per crash: time, version, thread, source line (no mail) | By hand (capped at 64 KB) |
| `logs\komorebi.log` (`.1`, `.2`) | Only if you turn on the diagnostics log: server command names, codes, timings (no mail) | By hand (capped at 1 MB each) |
| `cache.damaged-<date>.sqlite3` (and its `-wal`, `-shm`, `.drafts`) | A damaged cache, renamed aside as it was | By hand |
| `cache.sqlite3` | Contacts and address books: names, email addresses, phone numbers, organization, job title, notes, groups, the vCards as last synced, sync tokens and ETags, a contact's local version while a sync conflict waits, and each account's CardDAV address and user name (plaintext, like the rest of the cache) | With the file; synced books when sync is turned off and removed, or with their account |
| Windows Credential Manager, generic credential `account:<n>:password.komorebi` (`<n>` is the account's number in the cache) | The account's password | When the account is removed |
| Windows Credential Manager, generic credential `account:<n>:dav.komorebi` | Only if "Same as the mail password" is off: the CardDAV password | When the account is removed |

Settings → Privacy → **Remove all data** removes every item above at
once (see "Removing everything" below).

To look like the rest of Windows, Komorebi reads Windows' appearance
settings when it starts and when Windows says one changed: the accent
colour, light or dark apps, contrast theme colours, text size, animation
effects and whether title bars show the accent (from your user's
registry, `HKCU`, and Windows' system settings). It only reads them,
keeps them in memory while it runs, and sends them nowhere.

Updates: when you press Check for updates in Settings > About, or once a
day at most when "Check for updates automatically" is on, Komorebi makes
one HTTPS request to `api.github.com` for the project's latest release.
It carries the User-Agent `Komorebi/<version>` and nothing else about
you or your mail. Only when you press Download and install does it
download the installer and its `.sha256` and `.sig` files from
`github.com` (GitHub's download servers), into a new
`komorebi-update-*` folder in your temporary folder, and runs it after
checking its hash and signature. Nothing is sent with `KOMOREBI_OFFLINE`.

Komorebi writes nothing else: no registry keys, no files elsewhere, no
temporary files outside `cache.open` (Windows itself may remember the
folder a file dialog last showed). Files you save (attachments, `.eml`,
`.ics`) go only where you choose. The sections below describe each item.

The installer (if you use it rather than the portable zip) adds only the
app itself: the program, its license, notices and this statement in
`%LOCALAPPDATA%\Programs\Komorebi` (installed for you only, the default) or
in `Program Files\Komorebi` (installed for all users, which asks for
administrator rights), a Start menu shortcut, the mail app registration and
the uninstall entry Windows lists under Apps (in your user's registry,
`HKCU`, or for all users in `HKLM`). It collects and sends nothing.

## Removing everything

Settings → Privacy → Remove all data asks first, then:

1. deletes the saved password of every account in the cache from
   Credential Manager (if one can't be deleted, it stops there: the
   passwords already deleted stay deleted, and no file is removed);
2. stops all syncing and sending (a message still in its undo-send
   window is not sent, and an archive, delete or move still in its undo
   window is not done);
3. closes the window and deletes the cache with its `-wal` and `-shm`
   files, `cache.sqlite3.key`, `cache.drafts`, `cache.open`, `crash.log`, `crash.seen`, the
   diagnostics log files (and the `logs` folder once it is empty) and
   any damaged caches set aside, then the
   `%LOCALAPPDATA%\komorebi` folder itself if nothing else is in it. A
   file still in use is tried again for a few seconds; whatever can't be
   removed is named on screen so you can delete it by hand.

Mail on your servers is not touched: nothing is deleted or moved there,
and unsent drafts and Outbox messages are lost. Passwords left behind by
a damaged cache (accounts it no longer lists) stay in Credential Manager
until you remove them there.

Uninstalling (installer only) removes the program, the shortcut and the
uninstall entry, then asks, with No as the default, whether to delete
`%LOCALAPPDATA%\komorebi` too; a silent uninstall keeps it. The
uninstaller never touches Credential Manager: use Remove all data first,
or remove the `account:<n>:password.komorebi` entries there by hand.

## What stays on your computer

The cache lives at `%LOCALAPPDATA%\komorebi\cache.sqlite3` and holds:

- Account settings: email address, display name, IMAP/SMTP host, port and
  security mode, the "save sent mail" choice, and the account's two signature choices
  (which named signature starts new mail, and which starts replies and
  forwards). The named signatures themselves (text, formatting and an
  optional small logo image) are stored in the cache too, and so is the
  formatted form of each draft (`drafts.body_rich`). All of it is plaintext.
- App settings, as named values: the theme, the display language (System,
  English or Portuguese; Windows' display language is only read, never
  stored or sent), the check interval, the
  undo-send and mark-as-read delays, the taskbar flash choice, where
  the window was when it last closed (position and size on screen), the
  folder pane's folded lines and whether it is folded to the rail, the
  view last open, the tabs open when the app last closed (message and
  draft ids and which tab was active: never a subject, an address or any
  text; cleared when "Reopen tabs at start" is off), the size of the last
  message window, the last folders
  Move went to (folder ids), which teaching tips were seen, and the
  version the What's new bar last told about. Each
  account also records whether it is switched on and its place in the
  account order.
- Folder names and roles.
- SQLite's query-planner statistics (row counts per index, refreshed when
  the app closes; no mail text). Search keeps no copy or history: it reads
  the cached headers and previews as you type.
- Message headers (subject, sender, recipients, date, size, flags), a short
  preview line, and the full raw message (including attachments) for each
  message you opened or that was prefetched.
- Drafts you are writing: recipients, subject, text, reply details and the
  Message-ID they will be sent with. A reply to or forward of an HTML
  message also keeps the original's HTML as it will be quoted (with images,
  remote styles, scripts and forms removed). They are saved two seconds after you
  stop typing and when the composer closes (Close, Esc, or a click on a
  message or an Outbox row). A draft is deleted from the cache when you
  discard it (with its copy in your server's Drafts folder) or when its
  account is removed, and it moves to the Outbox when you send it. A copy of each saved draft is also kept
  in the account's Drafts folder on your own mail server (see Network
  calls), so the cache also records where that copy is (folder, UID). The
  copy carries an `X-Komorebi-Draft-Id` header with the draft's local
  number, and its Bcc recipients. Removing an account leaves its drafts'
  server copies in place.
- The Outbox: messages you sent that have not gone out yet (during the
  5-second undo window, while offline, or after a failed send). Besides
  the draft's fields, each holds the message as it will be sent, with its
  attachments encoded in it, and the last error. A message leaves the
  cache as soon as the server accepts it, when you discard it, or when its
  account is removed; Edit (and Undo) turn it back into a draft (except
  a reply to an invitation, which can only be retried or discarded).
- Your answers to meeting invitations: for each one, the invite's UID
  (and which occurrence), its version number when you answered, the
  answer (accepted, tentative or declined) and when. Nothing else from the
  invite (title, attendees, description) is copied; it is deleted with
  the account.
- The calendar: when you accept (or tentatively accept) an invitation,
  its calendar part (title, time, place, description, organizer,
  attendees, its repeat rule) is stored as written, in plain text, in the
  local calendar "On this PC", with the times of its occurrences for
  about a year back and two ahead; Decline removes it. This calendar
  exists only in the cache: it is not synced anywhere, it is not removed
  with an account, and "Remove all data" deletes it with the cache.
- CalDAV calendars, when you press Find calendars on the Calendar page's
  Calendars pane: their names and addresses, and each event as the
  server sent it (title, time, place, description, organizer, attendees,
  reminders, its repeat rule), in plain text in the cache, with the
  server's version tags for the next sync. Events you create, change or
  delete in them are written to the cache at once and kept in a queue of
  changes not yet sent (the event as you saved it) until the next sync
  sends them to that calendar's server (and to no other host); a change
  the server refused because the event changed elsewhere stays in that
  queue until you apply it again or discard it. Events in the local
  calendar "On this PC" exist only in the cache. They are deleted with the account, when the
  calendar is no longer found, and by "Remove all data". The sign-in is
  the one Contacts uses: the mail password, or the separate
  `account:{id}:dav` password, both in Windows Credential Manager, sent
  only over TLS (HTTP Basic) and only to the calendar server address: the
  one you typed, or one looked up for your mail domain (or a name under
  it). A server found elsewhere is shown in the pane first, and Komorebi
  signs in there only after you press Find calendars again; a redirect
  to another server is refused, not followed with credentials. The
  address that worked is kept for calendars, apart from the Contacts
  one. Komorebi contacts that server only on Find calendars, when
  the Calendar page opens, on Sync now and at most every 15 minutes while
  a CalDAV calendar is on and Komorebi is open (never under
  `KOMOREBI_OFFLINE`); with the address empty, Find calendars first asks
  DNS (SRV `_caldavs._tcp`) and `https://<your domain>/.well-known/caldav`
  without credentials.
- Event reminders: which reminders were already shown (an event id and
  the occurrence's start, no text) are kept in the cache for two days so
  none shows twice. Reminder notifications are off unless you turn them
  on; they are made on this computer by the running app (title, start
  time and place; never the event's notes), and nothing is scheduled with
  Windows.

Komorebi keeps an index of addresses from your cached mail to suggest
recipients; it is rebuilt from messages and deleted with the account. Each
entry holds an address, the newest display name seen with it, when it was
last seen and how often (as a sender, or as a recipient of mail you sent).
Junk, Drafts and your own address are left out. Deleting a message does not
remove its addresses from the index.

A draft's attachments are copied, when you add them, into the folder
`%LOCALAPPDATA%\komorebi\cache.drafts\<draft number>\` beside the cache, and
those copies are deleted together with the draft.

An attachment you open is written to
`%LOCALAPPDATA%\komorebi\cache.open\<random>\` beside the cache, so the app
for its type can read it; these copies are deleted at the next start
(a copy still open in another app stays until the start after that).

Download again (on the account's page in Settings → Accounts) deletes an account's cached
messages, their raw copies and its address index from the cache (in one
step) and downloads the mail again; nothing on the server changes, and
the account, its password, drafts, Outbox and the settings stay.

The cache is **plaintext unless you turn on** Settings → Privacy → Encrypt
the mail cache on this computer (off by default). Then every page of the
cache and its `-wal` is sealed with AES-256-GCM, and a changed, swapped or
zeroed page is refused rather than read; the key is in `cache.sqlite3.key`,
protected by Windows (DPAPI) for your Windows user. The choice waits in
`cache.sqlite3.encrypt` until the next start converts the cache (crash-safe,
both ways). A cache whose key is missing or belongs to another Windows user
cannot be opened: Komorebi says so and offers to set it aside (dated, not
deleted) and start a fresh cache, or to quit. The draft copies
(`cache.drafts`) and the opened copies (`cache.open`) stay plaintext even
then, and the setting says so. Deleting the file (and `cache.drafts`) resets
the app; your mail stays on the server, but unsent drafts and Outbox
messages are lost.

If Komorebi stops on an internal error, it appends one line to
`crash.log` in the same folder: the time (UTC), the app version, the
thread name and the source file and line where it stopped. The error
message itself is never written, because it can quote mail text. The file
is capped at 64 KB, never leaves your computer, and can be deleted at any
time. The next start shows a one-time note pointing to it (`crash.seen`
beside it records that the note was shown).

The **diagnostics log** is off unless you turn it on in Settings →
Privacy (Open logs folder there shows the files). Then each server
command adds one line to
`logs\komorebi.log` in the same folder: the time (UTC), the account's
number in the cache, the command's name (`FETCH`, `STORE`, `SMTP SEND`,
...), its status or the server's response code (`OK`, `NO [OVERQUOTA]`)
and how long it took. Never the command's arguments, folder names,
addresses, subjects, message text, the server's own words or passwords.
At 1 MB the log moves to `komorebi.log.1` (and that one to `.2`; older
lines are deleted). It never leaves your computer; turn it off and delete
the `logs` folder to remove it.

If SQLite reports the cache as damaged, the file (with its `-wal` and
`-shm` files and `cache.drafts`) is renamed aside with the date, such as
`cache.damaged-20261009-101500.sqlite3`, and a new cache is created. The
old file stays as it was, still plaintext, until you delete it. Its
accounts are not copied: add them again. The passwords saved for them
stay in Credential Manager (as `account:<n>:password.komorebi`) until an account
added later takes their place or you remove them there. A cache made by
a newer Komorebi is never renamed, changed or deleted.

## Passwords

Passwords (or app passwords) are stored only in Windows Credential Manager.
They are never written to the cache, logs or the UI state, and the account
form in Settings never shows a saved one. A new password typed there
replaces the saved one only after it signs in to both servers. Removing an
account deletes its saved password first; if Credential Manager can't be
reached, the account is not removed.

## Certificates of your own server

- Komorebi validates server certificates with Windows. For a self-signed or
  private-CA server you can pin that server's certificate: "Review
  certificate" (offered after a check failed on an untrusted certificate of
  a manual server) connects to the server without sending any credentials,
  only to read its certificate, and shows its SHA-256 fingerprint. Nothing is
  saved until you confirm. Compare the fingerprint with the one your server
  reports; the first trust decision is yours.
- The certificate (public data) is stored in the local cache, per server
  (host, port and role), and only used to recognise that server. It goes
  with the account or with "Remove all data". A pinned server presenting
  another certificate is refused before any password is sent, and a pin
  never makes Windows' own trusted roots acceptable for that server.
  Changing the host, port or security setting clears that server's pin.
- A password is sent only after the TLS session is up and the certificate is
  validated by Windows or is the pinned one. STARTTLS is never optional.
- Windows checks revocation from its cache only; building a certificate chain
  may still fetch a missing intermediate certificate from the address named in
  the certificate (an unpinned public server or a CA-issued pinned one).
- The prompt offers what to trust: this exact certificate (reviewed again
  when the server renews it), or this server key (the SHA-256 of its public
  key; a renewal that keeps the key passes, checked after the handshake and
  before anything else is sent, on IMAP and SMTP alike), or the issuing CA
  (offered only when the chain the server sent reaches past its own
  certificate; its subject and SHA-256 are shown; Windows then checks every
  certificate against that CA alone, with the host name). Reading the
  chain uses the same accept-everything handshake as the review, which
  sends nothing.
- An account can say its SMTP server needs no sign-in (a relay on your own
  network): no password is read or sent for sending, and TLS or STARTTLS
  is still required.
- Before TLS, Komorebi asks an IMAP server only for its capabilities and
  for STARTTLS; nothing the server lists then is trusted (they are asked for
  again under TLS). A server that does not list STARTTLS, answers it with
  anything but OK, greets with PREAUTH or BYE, or disables `LOGIN` under TLS
  never receives the password.
- The SMTP greeting is `EHLO [127.0.0.1]` on every connection, so a server
  never learns the name of this computer from Komorebi.
- When an account cannot connect for a reason only you can fix (a rejected
  password, a changed or untrusted certificate, no STARTTLS), its sync,
  IDLE and sending threads make one attempt and then stop asking until you
  press Retry or Refresh or save the account, so no server sees repeated
  sign-in attempts.

## Network calls

Komorebi connects only to the servers you configure:

- CardDAV, only for an account where you turned on address book sync:
  HTTPS to the server address you entered, with HTTP Basic sign-in (your
  mail password, or the separate one), only to that address's server and
  never across a redirect to another one. "Find" sends your email domain
  to your DNS resolver (`_carddavs._tcp`) and asks
  `https://<domain>/.well-known/carddav` where the address books are,
  without credentials. "Find address books" is the first signed-in
  request. A pass runs when Contacts opens, on Sync now, after an edit,
  and at most every 30 minutes with the mail check, never before the
  window shows and never under `KOMOREBI_OFFLINE`. Read only never writes
  to the server; read and write sends the contacts you add, change or
  delete. The server is never changed when you turn sync off.

- IMAP (TLS or STARTTLS) to sync folders, headers, flags and message bodies,
  and to apply your actions (read, flag, move, delete, save to Sent). An
  action that couldn't reach the server is tried once more when a
  connection next works (within 5 minutes), never later and never after
  a restart. After the computer wakes from sleep, the connections are
  opened again and every account syncs. Your
  saved drafts are copied to your Drafts folder there (at most every 30
  seconds while you type, and when the composer closes); the copy a new one
  replaces, and the copy of a draft you send or discard, is removed.
  While the app runs, one read-only IMAP connection waits on the inbox
  (IDLE) so new mail shows up without waiting for the next sync.
  Your rules (Settings > Rules) act on new Inbox mail by themselves, with
  no press per message: on the same IMAP connection to your own server,
  one command per flag (read, important) and one move per destination
  folder. They never send mail, and the rules themselves stay on this
  computer. Copy as Sieve only puts text on your clipboard; nothing is
  sent.
- Looking up your server settings, only when you press Continue on the
  "Your email" step of adding an account (and only for a domain Komorebi
  has no built-in settings for), or "Find settings" in manual setup or on
  an account's page; never while you type or when a field loses focus,
  and never under `KOMOREBI_OFFLINE`: your
  email domain (the part after the @, never the address or the password) goes
  to your system's DNS resolver as SRV queries (`_imaps._tcp`,
  `_submissions._tcp`, and their STARTTLS forms), in the clear unless your
  system uses DNS over HTTPS, so your network and resolver can see it; and,
  if those find nothing, over HTTPS to `autoconfig.<your domain>` and to
  `<your domain>` (`/.well-known/autoconfig/mail/config-v1.1.xml`), your
  domain's own web servers, which see your IP address and the domain. At most
  four DNS queries and two web requests, never repeated by itself. Nothing
  goes to Mozilla or any other third party. The answer only fills the form's
  empty server fields: a server on another domain than your address is shown
  but not filled in, and nothing is saved or signed in to until you press Add
  or Test connection. Hosts that resolve to private addresses are refused, and
  with `KOMOREBI_OFFLINE` nothing is looked up.
- IMAP and SMTP sign-in checks when you add an account, or save a new
  password or other servers for one: the app signs in to each server and
  logs out (SMTP: greeting, sign-in, quit); nothing is sent.
- SMTP (TLS or STARTTLS) to send mail, only after you press Send (and its
  5-second undo window ends). A message that surely did not reach your
  server is tried again by itself, a few times; one that may have reached
  it is only sent again when you choose Retry.
- SMTP to send a reply to a meeting invitation, only after you choose
  Accept, Tentative or Decline and confirm the prompt that names its one
  recipient: the reply goes only to the invite's organizer (no Cc, no
  other attendees) and holds a line of text and the standard calendar
  reply (the event's UID, version and occurrence, the organizer, your
  attendee address and answer, the title and start, and the time you
  answered). "Don't send a reply" sends nothing.
- HTTPS to the image hosts of a message, only after you choose to load its
  remote images (see below).

Plaintext connections are refused. Komorebi makes no other network
requests: no analytics and no remote fonts. The only other contact with
the internet is the update check described above (an HTTPS request to
GitHub, when you press Check for updates or once a day while "Check for
updates automatically" is on).

## Remote content and attachments

HTML mail is drawn by Komorebi's own renderer, which has no script engine
and no network code: scripts never run, and remote stylesheets, fonts and
tracking pixels are never loaded. Images carried inside the message itself
are shown. Remote images are blocked until you choose otherwise: a bar
above the message counts them and warns that loading them tells the sender
when you opened it and your IP address. "Load once" loads them for that
message while it stays open. "Always load from" adds the sender's exact
From address (never its display name or domain) to a list kept in the
local cache; a bar then says so with Undo (which takes the address off the
list again), and later messages from that sender carry the caption "Images
load from this sender · Change". Settings → Privacy shows that list, where
each sender can be removed or the whole list forgotten. Loading uses HTTPS only: an `http://` (or
scheme-relative `//host`) image address is requested over HTTPS, a redirect
to plain HTTP is refused, and plain HTTP is never used. It sends
no cookies, Referer or credentials and a generic User-Agent, refuses
hosts on loopback, private or link-local addresses, and caps time, size and
redirects. Fetched images stay in memory, for the message shown only: they
are freed when another message opens or the reader closes. Images that
could not be fetched are only counted ("2 images couldn't be loaded"), never
retried on their own. Nothing it renders is written to
disk. Tracking pixels (tiny or hidden images and backgrounds) are never
fetched, even after you choose to load images. Nothing opens on its own: a web link opens in your default browser only after you
confirm it in a prompt that shows the full address, and only `http`/`https`
links can be opened that way. `mailto:` links open Komorebi's composer;
`javascript:`, `file:` and other links do nothing. Messages it cannot draw
are shown as plain text.
Selected text goes to the clipboard only when you copy it.
"Unsubscribe" (shown for mail with a `List-Unsubscribe` header) does nothing
on its own: its `mailto:` address opens the composer, and nothing is sent
until you press Send; its `https:` address opens in your browser only after
the same confirm prompt as a link. Nothing is fetched when a message opens,
and one-click unsubscribe (a request made by the app) is not used.
The sender notice and the encrypted-mail state are worked out on this
computer from the message already in the cache (its headers, including the
`Authentication-Results` your own server added); nothing is looked up.
Encrypted mail is never decrypted, and no keys are stored.
The meeting-invitation card is read on this computer from the calendar
part already in the message; nothing is fetched or looked up (a named
time zone is read from the invite, else from Windows' own time zone
data on this computer). Accepting adds the event to the local calendar
only; nothing about it leaves the computer except the reply you choose
to send. Its Join button
opens the meeting link only through the same confirm prompt as a link,
and Save .ics saves the calendar file where you choose.
Attachments are never opened automatically; you can save them to a folder
you choose, or open one yourself (programs and scripts are never opened,
only saved). Attachment file names are sanitized (reserved device names,
trailing dots and spaces, length). Saved and opened attachments get the
Mark of the Web (a `Zone.Identifier` stream, internet zone), as browser
downloads do, so Windows SmartScreen and Office's Protected View apply.
Files you drop on the window or paste are read only to attach them.
Save as .eml writes the message exactly as cached (every header field,
the body and its attachments) to a file you choose, with the same Mark of
the Web; Show original only shows it, and Copy Message-ID, Copy headers
and Copy all put text on the clipboard only when you press them. If the
message is not downloaded yet, these fetch it from your IMAP server first,
as opening it does. Forward as attachment attaches the original whole,
with all its header fields (its recipients and the servers it passed
through, for example), to a new draft; nothing is sent until you press
Send.

## Notifications, tray and starting with Windows

New-mail notifications are local. They are shown by Windows from the
tray icon when the IMAP sync or IDLE connection you already have sees new
mail; there is no push service, no cloud relay and no extra connection,
and nothing about them is logged or sent anywhere. In Settings you choose,
per account, whether it notifies, and whether a notification shows the
sender and subject or only "New mail"; Windows keeps its own notification
history and shows it on the lock screen according to your Windows
settings, so pick "Only New mail" if that matters to you. Notifications
can be muted from the tray menu. Komorebi remembers, in memory only, which
messages it already notified about.

Closing to the tray and starting with Windows are off by default. Turning
"Start with Windows" on writes one value, `Komorebi`, under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (the path of the
program and `--minimized`); turning it off removes it. Nothing else is
written to the registry.

## Not collected

Komorebi never collects or transmits to a third party: usage data,
identifiers, crash reports, mailbox contents, drafts, address books or
credentials.
