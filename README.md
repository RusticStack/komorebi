# Komorebi

A small, fast email client for Windows 10/11. It is free and open source
(GPL-3.0-or-later), uses a Fluent 2 look, and does one job: reading and
writing mail without getting in the way. No AI, no telemetry, no web engine.

## Highlights

- Fast and light: a window in under 200 ms, a cached folder listed in under
  600 ms, one small native binary (under 20 MB), low memory.
- IMAP and SMTP with a local SQLite cache; several accounts; presets for
  Purelymail and Postale.io, manual settings for any other server.
- Reads HTML mail with its own layout engine (remote content is never
  loaded) and writes formatted mail with signatures, attachments and drafts.
- Tabs and message windows, keyboard shortcuts, meeting invitations,
  optional Windows notifications and tray icon.
- English and European Portuguese, following the Windows display language.
- Private by design: passwords live in Windows Credential Manager, nothing
  is sent anywhere except to your own mail servers (see PRIVACY.md).

## Install

Download the latest `komorebi-<version>-setup.exe` from the
[Releases page](https://github.com/RusticStack/komorebi/releases), or build
from source (see "Install from source" below). More about the installer,
the portable zip and checksums is in "Install" further down. Releases are
not code-signed yet, so Windows SmartScreen may warn on first run.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/SCOPE.md](docs/SCOPE.md)
first. Open items are in [TODO.md](TODO.md); changes are in
[CHANGELOG.md](CHANGELOG.md).

## Details

Features in more detail:

- **Languages:** English and European Portuguese (pt-PT, never Brazilian).
  The language follows the Windows display language (any Portuguese except
  pt-BR is shown in pt-PT; everything else, pt-BR included, in English) and
  Settings → General → Language can pin System, English or Português
  (Portugal). It switches at once, with no restart: the whole window, the
  list's dates and sizes, the status line, the window title and the dialogs.
  Dates, numbers and plurals follow the language (`12 out.`, `1,2 MB`,
  "0 mensagens"). Text in the mail itself, server messages and the
  technical detail after an error sentence stay as they came. The choice
  is the only thing stored; nothing is looked up or sent. To add a
  language, see "Translations" below.
- **Providers:** built-in presets for [Purelymail](https://purelymail.com) and
  [Postale.io](https://postale.io), plus manual settings for any IMAP/SMTP
  server (TLS or STARTTLS only).
- **Servers that do less:** a self-hosted server need not offer IDLE
  (Komorebi checks on the timer in Settings → General instead),
  CONDSTORE (flag changes are found by comparing the flags of the cached
  mail), UIDPLUS (a delete or move leaves the message flagged as deleted
  and hidden, and says so, because a bare EXPUNGE would also remove what
  another client flagged; a sent copy is found again by its Message-ID),
  MOVE (the message is copied first, and flagged only after the copy
  succeeded) or special-use folders (Sent, Drafts, Trash, Archive and Junk
  are recognised by their names, with or without the `INBOX.` prefix and
  with any delimiter). Folder names in modified UTF-7 are shown decoded and
  sent as the server wrote them; a name that is not valid is shown as it
  is. Where nothing is marked, Settings → Accounts → Edit → Folders picks
  the folder for each role (nothing is created on the server); without a
  Sent folder, mail still goes and is not copied (said once), and without
  a Trash folder nothing is deleted. The sign-in is IMAP `LOGIN` after TLS,
  or after STARTTLS with the capabilities asked for again; a server that
  does not list STARTTLS, answers it with anything but OK, greets with
  PREAUTH or BYE, or disables `LOGIN` never gets the password. The SMTP
  greeting is `EHLO [127.0.0.1]`, never the computer's name, and Komorebi
  always signs in to the sending server. A failure only you can fix (a
  rejected password, a changed or untrusted certificate, no STARTTLS)
  makes one attempt and then the account's sync, IDLE and sending wait:
  the account is marked, the status line offers Update password, Review
  certificate (a changed certificate: its page opens and the new one is
  read into the prompt, with the saved one's fingerprint beside it) or
  Retry, and Refresh or Save on the account's page tries once more. A
  server out of reach puts a caution badge on the account's row and rail
  tile until a sync goes through (its tooltip says "Can't reach
  {server}"; the row's caution button opens the account's page), and the
  status line offers Try now.
- **Your own server (Other server):** adding an account asks for your
  email first. Continue (for a domain without built-in settings) or "Find
  settings" in manual setup (only when pressed) looks your email domain
  up in DNS (SRV records) and at its own autoconfig page over HTTPS and
  fills the empty server fields, never one on another domain and never
  with the password; the "Server settings" expander opens the fields by
  hand (it opens by itself after a lookup, or when a check fails). Each
  server has its own host, port and SSL/TLS or STARTTLS (changing the
  security moves the port only if it was still the default); the incoming
  server can have its own username, and sending can sign in as another
  ("Same sign-in as incoming"). "Test connection" names each step (find the
  server, connect, secure, sign in, folder list, sending server, sending
  sign-in) and, when one fails, says in a sentence what to check; it saves
  nothing, and Add runs the same steps first. For a self-signed or
  private-CA certificate, "Review certificate" shows the whole SHA-256
  fingerprint and, once you trust it, saves it for that server only; a
  changed certificate is refused before any password is sent, and it has
  to be reviewed again when the server renews it. Enter in the password
  field adds the account, Esc goes back. Not yet validated against a real
  self-hosted server.
- **Multiple accounts**, each with its own folders, plus a combined
  "All inboxes" view. With no account yet, the window says what is needed
  and its Add account opens the form at once (New mail shows once there is
  an account). Adding an account signs in to both servers first
  (IMAP, then SMTP; nothing is sent). Settings → Edit changes an account in
  place, keeping its downloaded mail: name, password (left empty, the saved
  one stays; it is never shown), servers with SSL/TLS or STARTTLS for IMAP
  and SMTP, the Sent copy and the two signature choices. A new password or other
  servers are checked the same way before anything is saved, and the error
  says which server refused. Each account in Settings shows its last sync
  ("Up to date, checked 12:41", "Syncing…", "Can't reach …"). A rejected
  sign-in marks the account in the navigation (a red dot; in the narrow
  rail, a red divider) and offers
  "Update password" (on the status line and in Settings). Each account is
  a card in Settings › Accounts that opens its page: "Sync this account"
  (switched off, it keeps its downloaded mail, shown greyed in the
  navigation, but opens no connections, and mail sent from it waits in
  the Outbox until it is switched on again), Name, Password, Signature
  (one for new messages, one for replies and forwards, picked from the
  named signatures or None), Folders, Server settings (folded), Download again and Remove (it asks
  first; mail on the server is not affected), with Save, Test connection
  and Cancel pinned at the bottom. Its "…" menu has Move up and Move down
  (the account order); with more than one account, each has a colour dot
  that marks its rows in All inboxes and its name in the navigation.
- **Window sizes:** from 1040 px wide, the folder pane, the list and the
  reader side by side; narrower, the folders fold into a labelled rail and
  the list takes the reader's place until a message opens; below 720 px
  (down to 500 px, so Snap Layouts' halves and thirds fit) one pane shows
  at a time: the list, or the open message, composer or Outbox entry with
  a back arrow (Esc does the same); there the arrow keys only move
  through the list and Enter opens. The status line and Undo then show at
  the foot of the reader.
- **Settings** (Ctrl+, or the Settings item at the foot of the navigation; Esc closes,
  or goes back from the account form) is a page in the window, as in
  Windows 11's own apps: choosing a folder or New mail leaves it, and
  closing it gives the focus back to what opened it. It has four tabs of
  setting cards (switches apply at once). Accounts: the list above (a
  bar shows a first download's progress), each opening its account page,
  and Add account, which opens the same three steps as the first run
  (below) in the content area.
  General: the theme (System follows Windows, or always Light or Dark;
  an open HTML message is laid out again), how often to check for new
  mail (every 1, 2, 5 or 15 minutes; every folder is still synced in
  full at least hourly), the undo-send delay (off, 5, 10, 20 or 30
  seconds), when an opened message is marked read (right away, after 1 or
  3 seconds, or never), and an opt-in taskbar flash when new mail arrives
  while the window is in the background, and the language (the window
  changes at once; a line already shown changes after a restart).
  Privacy: the senders whose images always load, an opt-in diagnostics
  log (off by default; see Connection trouble) with Open logs folder, and
  under Reset, Remove all data: after a prompt it deletes every account's saved
  password from Credential Manager, then everything in the data folder
  (downloaded mail, drafts, the Outbox, settings, logs, crash notes and
  damaged caches set aside), and closes Komorebi; mail on the server is
  not affected. If a password can't be deleted, it stops there and no
  file is removed.
  About: the version, the license, the data folder (Copy, or Open in
  File Explorer) and the repository.
- **Title and window:** the window title shows the inboxes' unread count
  ("Inbox (7) - Komorebi"). The window opens where it was closed, at the
  same size (maximized if it was), moved onto a screen that still exists
  if its monitor is gone. The title bar is dark or light with the theme
  chosen in Settings, coloured as the window's own background so the two
  read as one surface (Windows' accent colour when "Show accent colour on
  title bars" is on, and Windows' own colours under a contrast theme).
- **Follows Windows' appearance**, as apps that come with Windows 11 do:
  the accent colour (the New mail button, selection and accent text use
  the shades Windows derives from it), light or dark apps (under the
  System theme), contrast themes (light or dark as the theme's window
  colour; the accent, selection and focus colours take the theme's own),
  the text size (Settings > Accessibility > Text size: text grows, icons
  don't) and animation effects. Read when Komorebi starts and again
  whenever Windows says one changed; nothing is stored.
- **UI polish round (WinUI 3 / Fluent 2):** the look follows WinUI 3 as
  Windows 11's own apps draw it (`docs/WINUI3.md`, `docs/FLUENT2.md`):
  the folder pane on the window's background, the list and reader on one
  card, WinUI buttons, text fields, check boxes, toggle switches, tabs,
  info bars and dialogs with their states, the double focus ring only
  after a key, and every colour taken from the Windows theme (accent
  ramp, light / dark, contrast themes). Every field of the account form,
  the signature included, is a WinUI text box or a drop-down with a
  Button's edge (a ButtonText border under a contrast theme).
- **Reading:** HTML mail is drawn by a small built-in renderer (no web
  engine, no scripts); plain-text mail and anything the renderer can't
  handle show as text. Fixed-width mail (a 600 px newsletter or statement)
  in a narrower reader is shown scaled down to fit, sharp, down to 60%;
  below that it scrolls sideways (Shift+wheel). Emoji are drawn in colour
  (Segoe UI Emoji). In the dark theme, mail that has no dark design of its
  own is dimmed: light backgrounds turn dark and dark text light, keeping
  text at least as readable as the sender made it (images are left as they
  are); "Show original colours" in the reader's More (…) menu turns that off for the
  message shown. A paragraph longer than 64 KB is cut, with a note, and
  "View as plain text" shows all of it. Remote images are blocked: a bar above the message
  says how many and from which address, and loads them once or always for that exact sender
  address (Settings lists those senders, with Remove and Forget all); it
  can be closed for the message shown.
  Images that still could not be loaded (no HTTPS on the host, not found,
  too large, too slow) are counted in the bar ("2 images couldn't be
  loaded") once the rest are in.
  Images load over HTTPS only: `http` image addresses are requested over
  HTTPS, and plain HTTP is never used. Remote stylesheets and fonts and tracking pixels never load. Text on the page
  can be selected (drag, double-click for a word, triple-click for a
  paragraph, Ctrl+A) and copied (Ctrl+C or right-click), and "View as
  plain text" in the More menu shows any HTML message as text. Links show
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
  downloaded), Show original (every header field unfolded and
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
  Signatures are named and kept in one list (Settings > Signatures; the
  text is formatted, with at most one small logo sent as an embedded
  `cid:` part). Each account has two defaults, one for new messages and one
  for replies and forwards, either of which can be None (the account's
  page has a box for each, and Manage signatures). The default goes under
  your text after a blank line (the plain-text part has the standard `-- `
  line) and above the quoted text; the composer's **Signature** menu
  replaces it (Ctrl+Z puts the old one back), and picking another From
  account swaps it while it is unchanged. The signature of a message
  replied to is left out of the quote.
  Drafts are saved on this computer two seconds after you stop typing, and
  at once when the composer closes (Esc), another message takes its place,
  the From account changes or the window closes; attachments are copied
  when added. Starting another message (a reply, a `mailto:` link, or New
  mail) keeps the draft in progress; New mail always starts a blank
  message. "Drafts", always in the navigation with its count, lists the
  saved drafts ("No drafts. Select New mail to start one." when there are
  none); a click or Enter opens one, Delete discards it (asking first, as
  Discard does), and right-click or Shift+F10 offers Open and Discard.
  Reply on a message that has a draft reopens that draft, and the message
  shows a "Draft" tag in the list. The composer never opens by itself. The
  server's Drafts folder leaves out the copies of these drafts, so a draft
  never shows twice; a draft that is only there (saved on another device or
  in webmail) opens read-only, with Forward as new draft instead of Reply.
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
  its field when you leave it, and Send's tooltip (and its description
  for screen readers) says why it is off. Cc and Bcc are links at the end
  of the To field. Before sending, a bar asks once ("Send anyway") when the subject
  is empty or your text mentions an attachment (attach, attached,
  attachment, anexo, em anexo, anexado...) but nothing is attached; quoted
  and forwarded text doesn't count.
- **Recipient suggestions:** after two characters of an address in To, Cc
  or Bcc, up to six addresses whose address or name starts with them show
  just under the field, over the fields below it (nothing moves): people who wrote to the From account and
  people you sent mail to from it (those rank higher, then the most
  recent). Up/Down pick one, Enter or Tab take it (as `Name <address>, `),
  Esc closes the list; a click takes one too. The addresses come from an
  index of the account's cached mail (not Junk or Drafts, and never your
  own address), built as mail arrives; for mail cached before this
  version, the first sync after the update fills it in the background.
  There is no address book to edit.
- **Formatting and HTML replies:** the composer formats text with its own
  document model: a short list of email-safe fonts and sizes, bold, italic,
  underline, strikethrough, ten named colours (or Automatic, which sends no
  colour so the recipient's dark mode still works), bulleted and numbered
  lists, indent, alignment, links (http, https and mailto), quotes and
  simple tables (up to 10 by 10, text only). A formatted message goes out as
  text/plain plus a generated text/html alternative (the plain part reads
  `label (url)` for links, `- ` and `1. ` for lists, `> ` for quotes, and
  aligned columns for tables); a message with no formatting goes out as
  plain text only, exactly as typed, and any message can be switched to
  plain text. A plain-text message is sent as text/plain only (marks such
  as `**` or `- ` stay text, and a reply quotes with `> ` lines). A
  formatted reply to or forward of an HTML message sends the original's
  HTML below your text, with its images (inline ones too), remote styles,
  scripts and forms removed; a forward shows the original's From, Sent, To
  and Subject above it. Nothing is ever loaded from the network. Web fonts,
  a free colour picker, highlight, headings, images in the body and HTML
  source editing do not exist (see docs/SCOPE.md). The body is
  Komorebi's own painted page (the reader's text engine): it shows the
  formatting as you type, with only the paragraph you change laid out
  again, while a hidden text field underneath keeps the caret, the keyboard
  layouts and IME, the clipboard's text and what screen readers read.
  Pasted HTML (from a browser, Word or Outlook) keeps only what the
  document can hold; Ctrl+Shift+V pastes plain text. The toolbar's font,
  size, indent and alignment move into **More** below 1040 px (and Quote,
  Clear formatting and the importance toggles when the composer is
  narrower still), so it never clips. **More** also holds **Preview** (the
  message as the recipient gets it) and **Switch to plain text** /
  **Switch to formatted text**; switching to plain text asks first, and
  Ctrl+Z (or switching back before typing) brings the formatting back.
  Settings > General > Compose sets whether new messages start formatted
  or as plain text, and their font, size and colour. **High importance**
  and **Low importance**, after the Signature menu, send the message with
  `Importance: high` / `low` and `X-Priority: 1` / `5` (one at most;
  neither for normal mail); the choice is saved with the draft.
- **Replies and forwards:** the quote is folded behind **Show quoted
  text** under the body (it is still saved and sent); the line above it
  ("On ..., ... wrote:", or a forward's heading and fields) is written in
  the language Komorebi shows. The body says "Write your message" while
  it is empty.
- **Closing and drafts:** Close (the X in the composer's header), Esc and
  a click on a message or an Outbox row all keep the draft the same way:
  it is saved on this PC and copied to the server's Drafts folder at
  once, and "Draft saved" shows; the header says "Draft saved 14:05",
  refreshed by every save (two seconds after you stop typing). A message
  with a reply in progress shows "You started a reply to this message."
  with **Resume**. Discard asks first: "It is deleted here and from your
  Drafts folder on the server, with its attachments." Files dragged over
  the window show where they go: "Drop to attach", or with no composer
  open "Drop to attach to a new message" (a new message opens for them).
- **Send** is always on: pressed with no recipient or a wrong address,
  the field says why under it (read out by screen readers) and takes the
  focus; nothing is sent.
- **Sending:** Send closes the composer at once and the message waits in
  the Outbox for 5 seconds (the delay is set in Settings → General;
  "Sending "Plans" in 5 s" at the bottom of the list, counting down, with
  Undo, which brings the draft back unchanged; "Sent" once it went). It is then sent, and a copy of exactly the bytes
  sent is saved to Sent. A message that surely did not reach the server
  (no connection, or the server said "try later") stays in the Outbox and
  is tried again by itself after 30 s, doubling (the Outbox is checked
  at each check for new mail), at most 6 times; one the
  server refused, or that ran out of tries, is marked failed. If the
  connection breaks during a send, the message may or may not have gone
  out: it is marked as such and never sent again by itself. "Outbox" in
  the navigation (shown while it holds messages) lists them with their
  state (a failed or unsure one with a red mark), said as a plain
  sentence for the kind of refusal (for example "ana@example.org doesn't
  exist. Check the address and send again." for a 5.1.1 reply), with the
  server's own reply under **Details** and the first lines of the text.
  Failed and unsure ones offer Edit (back to the composer), Retry (asks
  first when it may have been sent) and Discard; Edit is the main button
  when sending again unchanged can't help (an unknown address, a message
  too large, a refused sender or sign-in), Retry when it can (the server
  was busy or out of reach). A failed send's status line stays until it
  is closed. Messages in the
  Outbox survive a restart; a send cut off by closing the app is never
  resent automatically. Removing an account that still has drafts or
  Outbox messages asks first and says how many will be lost.
- **One app per cache:** starting Komorebi again brings the main window
  to the front (one app per cache file; a throwaway `KOMOREBI_DB` runs
  beside it). Pop-out message windows are secondary windows of that one
  app: closing the main window closes them all and ends the app.
- **Tabs and windows:** the folder pane folds to the rail at any width
  (the pane toggle at its top, Ctrl+Shift+E, or Hide folders in its menu),
  and stays as it was left. A double-click, Shift+Enter or "Open in new
  tab" opens a message in its own tab at full width (a middle-click opens
  it in the background, unread until it is shown); each draft beyond the
  one in the reading pane gets a composer tab, and a draft with something
  written moves to a tab of its own when another message is selected. The
  tab strip under the title bar shows only while more than the fixed Mail
  tab is open; tabs close (×, a middle-click, Ctrl+W), reorder (drag, or
  Ctrl+Shift+PageUp/PageDown) and reopen (Ctrl+Shift+T, this session's last
  ten), and a menu at the strip's end lists them all. Open tabs come back
  at the next start, as message and draft ids only ("Reopen tabs at
  start" in Settings › General). Ctrl+O or "Open in new window" shows a
  message in a slim read-only window of its own (several can sit side by
  side to compare); Reply, Reply all and Forward there open a composer tab
  in the main window. Archiving, deleting or moving a message closes its
  tab or window, and Undo brings a tab back at its place; a message removed
  on the server stays readable, with a warning, until its tab is closed.
  A message window has the reader's commands but not Show original, Save
  as .eml, Copy message ID or the invitation card (open the message in the
  main window for those); the composer has no window of its own yet.
- **Organizing:** read/unread, important, archive, delete and move, all
  two-way with the server (important is the standard IMAP `\Flagged`
  flag, which other clients show as a flag or a star). Several messages can be selected at once:
  Ctrl+click adds or removes one, Shift+click or Shift+Up/Down selects a
  range, Ctrl+A in the list selects the whole view, and Esc clears. The
  reader pane then shows how many are selected, with Archive, Delete, Move
  (when they are all in one account), Mark read / unread and Mark as
  important / Remove importance; Delete and Ctrl+Q act on the selection too. The list changes at
  once, and the server gets one command per folder for the whole batch
  (in UID sets of up to 4,000 characters), never one per message.
  Right-clicking a message opens a menu (Reply, Reply all, Forward, Mark
  as read / unread, Mark as important, Archive, Delete, Move to, Report junk) that acts
  on the selection; a row outside the selection is selected first,
  without opening it or marking it read. The same menu is under More
  (…) in the reader.
- **Search:** the search box in the title bar (Ctrl+E, F3 or Ctrl+F;
  "Search Inbox", "Search Newsletters" ...) matches the subject, sender,
  To, Cc and preview of the cached messages, ignoring case and accents
  ("joao" finds "João", "strasse" finds "Straße"), so Sent can be searched
  by recipient. The scope beside it is "This folder" (in All inboxes
  "These inboxes") or "All folders", which searches every cached folder
  of every account; each result then shows its account and folder
  ("Seed · Inbox"), and Show in folder in its menu opens it in its own
  folder. The list says "Search results" with a line such as "4 results
  in Inbox", and links to search all folders or clear the search. The
  query stays when another folder is opened and runs there; the scope goes
  back to the view. Drafts and the Outbox search the same way. Only cached
  headers and previews are searched, never the server or full message
  bodies.
- **Title bar:** on Windows 11 Komorebi draws its own 48 px title bar, as
  Windows 11 Mail and the new Outlook do: the icon and name, the search
  box in the middle and the minimize, maximize and close buttons (Snap
  Layouts on the maximize button, drag, double-click and the system menu
  work as usual). On Windows 10 and under high contrast the Windows title
  bar stays, with the same search strip under it; turning high contrast
  on or off switches it while the app runs.
- **Filters and read state:** All, Unread and Important tabs under the search
  box narrow the list (with the search, too); a message read while Unread
  is on stays in the list until it reloads. More (…) beside Refresh offers
  "Mark all as read" for the folder shown (in All inboxes, every inbox;
  not while a search is showing, as it would cover the whole folder: the
  folder pane's row menu still has it):
  the list changes at once and the server gets one command per folder,
  covering mail up to the newest message listed, so mail that arrived
  since the last check stays unread. An opened message is marked read
  after it has been open for a second, so arrowing through the list
  leaves the messages passed on the way unread (Settings → General:
  right away, after 1 or 3 seconds, or never). An empty list says why, in
  its middle, with one way out: no matches (Search all folders, or Clear
  search), no unread or important messages (Show all), or that the last
  check for mail failed.
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
  folders, the list and the reader (in Settings, the folders and the
  page). Dialogs start on a safe button (a prompt that removes, discards
  or sends again on Cancel with no default, so a stray Enter never
  confirms it; any other on its accent answer, which Enter gives), keep Tab inside
  them, close with Esc and give the focus back where it was (the list, the
  reader's toolbar or page, the folders, the composer, or the Settings
  button that opened them). Controls carry names and states for screen
  readers (the list and folders by name, each message with its unread,
  important, attachment and replied state, toggles as checked, accounts as
  expanded or collapsed, the reader's header fields), and the status line
  is announced as it appears, at once when it is an error. Accent-coloured
  text is darkened or lightened from the Windows accent until it reads at
  4.5:1 on the surfaces it sits on. Unread messages are semibold in the
  text colour with an accent dot before the sender; read ones are regular
  in the secondary text colour, still at 4.5:1. An important message
  has an amber flag at the end of its subject line; hovering a row shows
  an outline one there, and a click on it marks or unmarks that row only.
  A message its sender marked as high importance (the `Importance`,
  `X-Priority` or `X-MSMail-Priority` header) shows a bold "!" in the
  text colour before the date, and screen readers hear "High importance";
  low importance is a small Down arrow in the secondary text colour. The
  reader says "Sent with high importance" or "Sent with low importance"
  under the To line. Mail cached before this version shows it once its
  body is downloaded, or opened if it already was.
- **Private:** no telemetry, no ads, no AI. Passwords stay in Windows
  Credential Manager.
- **Connection trouble:** an error says what happened in one sentence
  with a next step (offline, sign-in rejected, untrusted certificate,
  message too large, mailbox full, too many connections), once per
  account until a sync goes through again (Refresh shows it again). The
  status line is an info bar at the top of the message list (or of
  Settings), never over a row; a server that can't be reached (or is at
  its connection limit) is a caution that stays until that account syncs
  again or the bar is closed, and comes back after a shorter message or
  an Undo offer ends; other errors go after 12 seconds, other messages
  after 5. A
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

See [`docs/SCOPE.md`](docs/SCOPE.md) for scope, non-goals and the performance budgets.

## Add an account

The first start shows only the welcome: "Add your email account", three
lines on what Komorebi is, and the links "Need help?" (this section),
"Privacy" (PRIVACY.md), a Language box (it switches at once) and Settings.
Adding an account takes three short steps, with the steps remaining shown
above them (Settings → Accounts → Add account opens the same steps):

1. **Your email.** Continue checks the address. For purelymail.com and
   postale.io the settings are built in; for any other domain Komorebi
   looks them up from your domain (DNS, then its autoconfig page; only the
   domain name is sent, never the password), and only because you pressed
   Continue. Offline, it goes straight to the server settings.
2. **Sign in.** What was found shows above the Password box (an app
   password if your provider uses two-step sign-in; it is saved in
   Windows Credential Manager, never in Komorebi's files). If nothing was
   found, pick Purelymail, Postale.io or Other server; "Change" opens the
   server settings (hosts, ports, encryption, username). Sign in checks
   each step (connecting securely, checking your password, getting ready
   to send); Back or Esc stop a check that takes too long. A failure says
   what went wrong in one sentence with the next step ("Set username",
   "Review certificate", "Try again" or "Set up manually"); "Show details"
   lists the seven steps and the server names.
3. **Make it yours** (optional): your name as people see it, and a
   signature, with a preview. Skip keeps them empty; both stay in Settings
   → Accounts → your account.

"You're all set" then shows the first download (newest mail first) until
you press "Open your inbox", which opens the new Inbox.

## Install

Download the latest release from the GitHub Releases page (Windows 10/11
x64), or build from source below. Releases come in two forms:

- `komorebi-<version>-setup.exe` installs for the current user by default,
  with no administrator rights, into `%LOCALAPPDATA%\Programs\Komorebi`, or
  for all users (its first page offers it, or `/ALLUSERS`), which asks for
  administrator rights and installs into `Program Files\Komorebi`; either
  way with a Start menu shortcut and an uninstaller (Settings → Apps). `/SILENT` or
  `/VERYSILENT` installs without questions; a running Komorebi is closed
  first and started again afterwards (the self-update runs it this way). Uninstalling asks, with No as the default, whether to delete the
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
(Get-FileHash .\komorebi-0.4.1-setup.exe -Algorithm SHA256).Hash.ToLower()
Select-String komorebi-0.4.1-setup.exe .\SHA256SUMS
```

Releases are made by hand (see "Signing a release" below), because the
in-app update accepts only an installer signed with the maintainer's private
key. `.github/workflows/release.yml` never publishes: run by hand (Actions →
Release → Run workflow) it is a dry run of tests, the release build, its size
check and the offline boot snapshot, an optional signing step (only when the
`SIGN_CERT_PFX` and `SIGN_CERT_PASSWORD` secrets exist; see
`scripts/sign.ps1`), the Inno Setup installer (`installer/komorebi.iss`),
the portable zip and `SHA256SUMS`, kept as a workflow artifact. A tag does
not start it.

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

## Signing a release

The in-app update (Settings > About) installs only an installer whose
SHA-256 matches the published `.sha256` and whose Ed25519 signature
verifies against the public key compiled into `src/update.rs`. The
signature covers the version and the hash, so an older installer cannot be
served as a newer one.

The steps, on the maintainer's machine, from a clean checkout of the
commit to release (the Cargo version and the top `CHANGELOG.md` section
already say the new version, here `0.4.1`):

```powershell
cargo build --release --locked
pwsh scripts/build-installer.ps1 -Exe target\release\komorebi.exe -Version 0.4.1
cargo run --example sign_update -- sign dist\komorebi-0.4.1-setup.exe 0.4.1
gh release create v0.4.1 --title "Komorebi 0.4.1" --notes-file notes.md `
    dist\komorebi-0.4.1-setup.exe dist\komorebi-0.4.1-setup.exe.sha256 dist\komorebi-0.4.1-setup.exe.sig
```

1. The build writes `target\release\komorebi.exe`; the installer script
   writes `dist\komorebi-0.4.1-setup.exe` and prints its SHA-256. If an
   Authenticode certificate is available, sign `komorebi.exe` before the
   installer and the installer after it with `scripts/sign.ps1`, and only
   then sign for the update (the Ed25519 signature covers the final bytes).
2. `sign_update -- sign` writes `komorebi-0.4.1-setup.exe.sha256` and
   `.sig` beside the installer. The private key is read from
   `%USERPROFILE%\.komorebi-release\update-signing.key` (made once with
   `sign_update -- keygen`, which refuses to overwrite it and prints only
   the public key). Keep it out of the repository and back it up offline:
   losing it means shipping a new public key by hand.
3. `gh release create` makes the release and its tag `v0.4.1` and uploads
   the three files the in-app update needs, under exactly these names; the
   notes are this version's `CHANGELOG.md` section. Optionally add the
   portable zip, the `.pdb` and `SHA256SUMS` from the workflow's dry run.
4. Check it: `cargo run --example sign_update -- verify
   dist\komorebi-0.4.1-setup.exe 0.4.1`, then Settings → About → Check for
   updates on an older install.

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

- **Size:** `target\release\komorebi.exe` must be at most 20,000,000
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
| F1 or Ctrl+/ | Keyboard shortcuts: every key on this list, by group (also in Settings › About) |
| Ctrl+, | Settings, and back to the mail (as the gear and Settings' back arrow do; Esc closes them too; from the account form, Esc goes back to the list first) |
| Ctrl+Enter or Alt+S | Send (in the composer) |
| Ctrl+Shift+C / Ctrl+Shift+B | Show Cc and Bcc, and go to Cc / Bcc (in the composer) |
| Ctrl+Shift+A | Attach files (in the composer) |
| Ctrl+V | Attach the files copied in Explorer (in the composer; text pastes as usual) |
| Up / Down, Enter or Tab, Esc | Pick, take or close the recipient suggestions (while they show under an address field) |
| Ctrl+B / Ctrl+I / Ctrl+U | Bold / italic / underline (in a formatted message's body, where Ctrl+U does not mark as unread) |
| Ctrl+Shift+X | Strikethrough (in a formatted message's body) |
| Ctrl+K | Link the selection to an address, or insert one at the caret (in a formatted message's body; Enter inserts, Esc cancels) |
| Ctrl+Shift+8 / Ctrl+Shift+7 | Bulleted / numbered list on the selected paragraphs (the key that types `8` / `7`); typing `- `, `* ` or `1. ` at a line start does the same |
| Ctrl+Shift+Q | Quote the selected paragraphs (again takes it off) |
| Ctrl+L / Ctrl+E / Ctrl+R / Ctrl+J | Align left / center / right / justify (in the body these come before Search and Reply) |
| Ctrl+] / Ctrl+[ | One font size larger / smaller |
| Ctrl+M / Ctrl+Shift+M | Increase / decrease the indent |
| Tab / Shift+Tab (in a list or table) | A list item's level, or the next / previous table cell (Tab after the last cell leaves the table); elsewhere Tab moves the focus |
| Ctrl+Space | Clear formatting |
| Ctrl+Z / Ctrl+Y | Undo / redo text and formatting together (200 steps) |
| Ctrl+Shift+V | Paste as plain text (in the body, where it does not move messages) |
| Esc | Close the composer, saving the draft (in a composer tab, close the tab) |
| Tab / Shift+Tab | Next / previous control; the folders, the list, the reader's toolbar and the composer's formatting buttons are one stop each (a row's quick actions, shown on hover and on the keyboard's row, are never Tab stops: the keys below do the same) |
| F6 / Shift+F6 | Next / previous pane: the title bar's search, folders, message list, reader (toolbar, composer or the selection's buttons) |
| Space or Enter | Press the focused button or toggle |
| Left / Right | Move along the reader's toolbar or the formatting buttons (while they have the focus) |
| Up / Down, Home / End, Enter or Space | Move through the folders and open one (while they have the focus; Left / Right fold an account) |
| Shift+F10 or Menu | In the folders: the row's menu (Mark all as read and Collapse all; on an account also Refresh and Account settings; on a rail tile its Inbox, Sent and Archive) |
| Ctrl+R / Ctrl+Shift+R | Reply / Reply all |
| Ctrl+Shift+F | Forward |
| Ctrl+Shift+V | Move to: the Move list with its filter box focused (type to narrow it, Enter moves to the first folder listed) |
| Ctrl+Shift+I | Go to the Inbox (All inboxes with two or more accounts; from Settings too) |
| Ctrl+E, F3 or Ctrl+F | Search (the title bar's box; from Settings too, which then close) |
| Ctrl+Q / Ctrl+U | Mark as read / Mark as unread (Outlook's keys; on a selection, every message) |
| Ctrl+S | Save the message as a .eml file |
| Insert or Ctrl+Shift+G | Mark as important / Remove importance (Ctrl+Shift+G for keyboards without Insert) |
| Backspace | Archive (with the focus on a message row or the reader's page only; never in a text field, Drafts or the Outbox) |
| Up / Down | Previous / next message (with none open, the first); in Drafts, move to a draft (Enter opens it); in the Outbox, show the entry |
| Home / End | First / last message |
| PageUp / PageDown | Up / down by the rows the list shows |
| Enter | Open the selected message (with none, the first) and move to the page, where PageUp/PageDown, Space and Tab work |
| Esc | Close the folder pane opened over a narrow window (the toggle at the top of the narrow rail opens it), else clear the search, else the selection and the open message or Outbox entry (below 720 px, back to the list, as the reader's back arrow does), else "Select messages", else the status line (Esc on its buttons always closes it); never the Undo offer at the bottom of the list |
| Ctrl+click / Shift+click | Add or remove a message from the selection / select a range |
| Shift+Up / Shift+Down | Extend the selection |
| Ctrl+A | Select every message in the list (with the list focused) |
| Delete | Delete (move to Trash); in Trash (deleted on the server) or Junk, after asking (the prompt has no default button); in Drafts or the Outbox, discard the row (asking first) |
| Shift+F10 or Menu | The row menu for what is selected, under the current row (a draft or Outbox row: Open, Discard) |
| Ctrl+Z | Undo the last archive, delete, move, junk report or send (while the offer at the bottom of the list shows: at least 10 s, or until the next one; its time stops while the pointer is over it) |
| F5 | Refresh (also retries a rejected sign-in) |
| Ctrl+Shift+E | Show / hide the folders (the pane folds to the rail) |
| Double-click, Shift+Enter | Open the message in a new tab (a tab it already has comes forward) |
| Middle-click | On a message row, open it in a background tab; on a tab, close it |
| Ctrl+O | Open the message in a new window (a tab moved this way closes) |
| Ctrl+W or Ctrl+F4 | Close the tab (not the Mail tab); in a message window, close the window |
| Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+PageDown / Ctrl+PageUp | Next / previous tab |
| Ctrl+1 to Ctrl+8, Ctrl+9 | Go to tab 1 to 8, the last tab |
| Ctrl+Shift+PageUp / Ctrl+Shift+PageDown | Move the tab left / right (the Mail tab stays first) |
| Ctrl+Shift+T | Reopen the last closed tab |
| Esc (in a message window) | Close the window; in a message tab Esc never closes the tab |

The keys that act on the selected message (reply, forward, read and
unread, important, archive, Delete, Move, and the list keys above) work only
while the reader shows, never in the composer or Settings. The list has
the keys from the start, before any click. Show original has no key: it
is in the reader's More menu (and the row menu), whose items show their
keys.

On a message row, the quick actions (hover, or the keyboard's row) are
Archive, Delete, Mark as important and Mark as read / unread: they act on that row
only, never open it and never mark it read. "Select messages" in the
list's "..." menu shows check boxes (a click then checks a row instead of
opening it). A message opened from the Unread filter stays in the list,
read, until the filter or the view changes (as in Outlook).

## Translations

The text lives in the `.slint` files, each string in `@tr("...")` (whole
sentences with `{}` placeholders, `"one" | "many" % n` for counts, a context
`@tr("ctx" => "...")` where one English word needs two translations). Rust
reads its sentences back through the `Tr` global in `ui/tr.slint`
(`src/i18n.rs`, `src/tr.rs`), so there is one catalog. Slint bundles
`translations/<tag>/LC_MESSAGES/komorebi.po` at build time (no gettext at
run time); English is the source text.

To add a language: add `translations/<tag>/LC_MESSAGES/komorebi.po` (copy
`translations/komorebi.pot`, set the `Plural-Forms` header), add the `Lang`
variant and the date, number and size tables in `src/format.rs`, and the
Settings choice. After changing any string, regenerate the template with
`KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template` and fill the
new entries; `cargo test --test i18n_catalog` fails on a missing or fuzzy
translation, a changed placeholder, a stale entry, Brazilian wording in
pt-PT, and any visible literal outside `@tr` that is not in
`translations/untranslated-allowlist.txt`. Check the wide layout and the
compact one (below 1040 px) and one pane (below 720 px, e.g.
`KOMOREBI_SIZE=560x700`) in each language with `KOMOREBI_LANG=pt-PT`
(or `en`) and `KOMOREBI_SNAPSHOT`. English dates with a year follow
Windows' short-date format (Settings › Time & language › Region), read
once at start; ISO 8601 when it can't be read.

## Development switches

| Variable | Effect |
|---|---|
| `KOMOREBI_LANG=system`, `en` or `pt-PT` | The display language for this run, over the one saved in Settings (nothing is saved). |
| `KOMOREBI_DB=path` | Use this cache file instead of `%LOCALAPPDATA%\komorebi\cache.sqlite3` (`crash.log` and the diagnostics log's `logs` folder then go beside it too, draft attachments into `<path without extension>.drafts` and opened attachments into `<path without extension>.open`; each cache file has its own single-instance lock) |
| `KOMOREBI_PASSWORD=secret` | With `KOMOREBI_DB`, use this password instead of Credential Manager (debug builds, or release built with `--features dev-env-password`) |
| `KOMOREBI_FLOW=dir` | Debug builds only: script a full walk through the app (open, flag, read/unread, reply, a formatted message with an attachment whose draft must be saved and whose HTML part must arrive, move to Junk and back) on `[komorebi-test]` messages only, saving a log and snapshots to `dir` |
| `KOMOREBI_NO_SERVER_DRAFTS=1` | Debug builds only: keep drafts on this PC instead of copying them to the server's Drafts folder (used with `KOMOREBI_FLOW`, so a live test adds and removes no server messages) |
| `KOMOREBI_OFFLINE=1` | No network and no credential access (demo caches, timing runs) |
| `KOMOREBI_TIMING=1` | Print startup timings to stderr (up to the first frame drawn), and how long each opened message took to its first tile |
| `KOMOREBI_ACCENT=#rrggbb` | Use this accent colour instead of Windows' (an approximate ramp of shades around it), or 7 comma-separated colours, Dark3 to Light3, for an exact ramp (green: `#0c1c14,#173525,#1e4732,#26593f,#2e6a4b,#357c58,#40956a`; WinUI's blue: `#002642,#004275,#005a9e,#0078d7,#429ce3,#76b9ed,#a6d8ff`) |
| `KOMOREBI_THEME=contrast` | Show a contrast theme (any run, not only snapshots; `light` and `dark` are snapshot options, below): `KOMOREBI_CONTRAST=hc1`, `hc2`, `hcblack` (the default) or `hcwhite` (the four themes in `C:\Windows\Resources\Ease of Access Themes`), or 8 comma-separated colours: Window, WindowText, Highlight, HighlightText, Hotlight, GrayText, ButtonFace, ButtonText. hc1 `#2d3236,#ffffff,#a1bfde,#212d3b,#70ebde,#a6a6a6,#2d3236,#b6f6f0`; hc2 `#000000,#ffffff,#d6b4fd,#2b2b2b,#8080ff,#a6a6a6,#000000,#ffee32`; hcblack `#202020,#ffffff,#8ee3f0,#263b50,#75e9fc,#a6a6a6,#202020,#ffffff`; hcwhite `#fffaef,#3d3d3d,#903909,#fff5e3,#1c5e75,#676767,#fffaef,#202020` |
| `KOMOREBI_TEXT_SCALE=1.5` | Use this text size (1.0 to 2.25) instead of Windows' |
| `KOMOREBI_ANIMATE=0` | Act as if Windows' animation effects were off |
| `KOMOREBI_SYSTEM_TITLEBAR=1` | Keep the Windows title bar (with the search strip under it), as on Windows 10 and under high contrast |
| `KOMOREBI_TITLEBAR_BORDER=1` | Draw a 1 px line along the top of the app-drawn title bar, for a Windows that draws no top border over it |
| `KOMOREBI_SNAPSHOT=file.bmp` | Render the window to a file and exit (always at the default place and size; the saved window place is ignored). Options: `KOMOREBI_SIZE=WxH`, `KOMOREBI_THEME=light` or `dark` (over the theme chosen in Settings), `KOMOREBI_SNAPSHOT_SELECT=n` (or `a-b`: open message a, then Shift+Down to b, showing the "N selected" panel), `KOMOREBI_SNAPSHOT_WALK=n` (then open the next n messages, 250 ms apart), `KOMOREBI_SNAPSHOT_SCROLL=px` (scroll the reader), `KOMOREBI_SNAPSHOT_ACTION=reply`, `compose`, `compose-invalid` (a new message with a wrong To address, then Send: the error under To, which has the focus; nothing is sent), `compose-no-to` (a new message with no recipient, then Send: "Add a recipient to send." under To), `compose-rich` (a new formatted message: bold, italic, underline, a colour, a link, both lists and a quote; `compose-rich-compact` is the same, for a narrow `KOMOREBI_SIZE`), `compose-table` (the same with a 3 x 3 table), `compose-format` (the same, and the link field open with an address typed), `compose-preview` (the same with Preview on: the HTML part drawn in the body's place), `compose-plain` (the same switched to plain text) and `compose-plain-ask` (the question before that), `compose-signature`, `settings-signatures` and `signature-edit` (only with `KOMOREBI_OFFLINE=1` on a throwaway cache, as they write two made-up signatures to it, one with a logo and set as the first account's defaults: a formatted message with it, Settings > Signatures, and the first signature's page), `drop-hover` and `drop-hover-compose` (a file dragged over the window with no composer or with one open: "Drop to attach to a new message" / "Drop to attach"), `draft-reply` (a reply to the selected message with a line typed, then Close: the reader's "You started a reply to this message." bar with Resume; offline only, as the draft is saved in the throwaway cache), `compose-suggest` (a new message with `al` typed into To: the recipient suggestions), `compose-attach` (a new message with three made-up attachments, the counter at 85%), `drafts` (the Drafts view), `discard` (the newest saved draft, or on a cache with none a made-up message typed into a new one, then the Discard prompt), `guard` (the newest saved draft, then Send: the send guards' bar), `menu:list`, `menu:pane` or `menu:rail` (print the entries of the list's "..." menu, of the folder pane's row menu on All inboxes, each account line and the first folder shown of each account, or of each rail tile's menu, with their enabled state, to stderr and exit; no native menu is opened, which in a snapshot run would wait for a click), `menu:row`, `menu:more` or `menu:move` (the same for the row menu on what is selected, the reader's More menu, each item with its accelerator text and check mark, or the Move list: recent folders first, then the tree by depth), `select-mode` ("Select messages" on: the rows' check boxes), `move` (the Move list on what is selected, its filter focused), `delete-confirm` (the Trash question on what is selected; nothing is deleted), `not-downloaded` (the open message as if not downloaded with its server out of reach, in memory only: after 2 s the sentence and Retry; the snapshot waits for them), `outbox` (the Outbox view with its newest message), `outbox:permanent` or `outbox:transient` (only with `KOMOREBI_OFFLINE=1`, on a throwaway cache, as it writes it: a made-up message whose send failed, with a 550 5.1.1 "user unknown" reply (Edit leads) or a 421 "try again later" (Retry leads), opened in the Outbox with its sentence, Details and first lines; nothing is sent), `pane` (the narrow window's folder pane, opened over the content: use a width below 1040), `nav:collapsed` (the folder pane folded to the rail, as Ctrl+Shift+E does; at 1040 px or wider, and saved in the cache's settings, so use a throwaway copy), `tabs` or `tabs:n` (the list's first n messages, 3 by default, in tabs of their own, the first one shown: the tab strip and a message at full width), `windows:n` (the list's first n messages in message windows of their own; the snapshot is of the main window, for measuring memory with `examples/peak_memory`), `settings`, `settings:signatures`, `settings:general`, `settings:privacy`, `settings:about` (or `settings:accounts`: Settings on that section), `update:checking`, `update:uptodate`, `update:available`, `update:downloading`, `update:failed`, `update:failed-verify` (Settings > About > Updates in a made-up state; nothing is looked up or downloaded) and `update:banner` (the "Version X is available" bar over the list), `settings-edit` (Settings with the first account's edit form), `settings-edit-logins` (the same, with made-up IMAP and SMTP usernames), `settings-edit-folders` (the same, scrolled to its Folders: the roles Sent, Drafts, Trash, Archive and Junk, each header above its box), `menu:account` (prints the account page's "…" menu: Move up / Move down, with two accounts or more), `notice-why` (the sender notice's "Why?" dialog over the selected message), `remote-always-bar` (Always load from this sender on the selected HTML message, staying in the reader: the bar with Undo and Privacy settings), `cert-changed` (the first account's status line as when its saved certificate changed: the short sentence and Review certificate; made-up, nothing is sent), `cert-changed-settings` (the same, then Settings with that account's line), the onboarding flow (made up, nothing is looked up, checked or sent): `ob-email`, `ob-email-error` (a wrong address, its error under the field), `ob-finding` (Sign in while the lookup runs), `ob-found-preset` (a Purelymail address: settings built in), `ob-found` (a lookup's answer from a provider's autoconfig), `ob-notfound` (nothing found: the provider choice), `ob-password`, `ob-wait` and `ob-wait-slow` (the check's three stages, then the 10 s sentence), `ob-error-rejected`, `ob-error-unreachable`, `ob-error-cert` (one title, one sentence, one next step; "Show details" folded), `ob-manual` (the server settings), and, only with `KOMOREBI_OFFLINE=1` on a throwaway cache as they add a made-up account to it, `ob-personalise`, `ob-done` and `ob-done-progress` (Make it yours, the finish, and the finish at 1,234 of 5,000); `ob-embedded` opens the flow from Settings > Add account (a cache with an account). The rings show after a second, so add `KOMOREBI_SNAPSHOT_HOVER` to take the snapshot later. `settings-add`, `signin-wait`, `account-test-failed` and `account-find*` are kept as aliases of `ob-email`, `ob-wait`, `ob-error-rejected` and `ob-found` / `ob-notfound` / `ob-manual`, `fresh-inbox` (only with `KOMOREBI_OFFLINE=1`, on a throwaway cache, as it writes it: a made-up account added to the cache as just added, its Inbox in its first download at 1,234 of 5,000 and the list's progress; use an empty cache to see it alone), `account-cert-failed` (the first account's edit form after the secure connection failed on an untrusted certificate of a made-up server: the sentence and Review certificate), `account-cert-prompt` (the same with the Trust this certificate? prompt open: the whole SHA-256 fingerprint and the certificate's details; nothing is saved), `account-cert-again` (the prompt as when a forgotten certificate is reviewed again: the old fingerprint is shown too), `account-cert-pinned` (the edit form with a certificate saved for each server: the lines and Forget certificate), `remove-data` (Settings → Privacy with the Remove all data prompt; nothing is removed until it is confirmed), `plain` (plain-text view), `original` (Show original colours, in the dark theme), `source` (Show original on the selected message), `rsvp` (Accept on the selected invitation: the reply prompt; nothing is sent until it is confirmed), `select-all` (select and copy the page), `scroll-right` (scroll a page wider than the reader to its right edge), `archive` (archive what is selected: the Undo offer; the move is held, so the cache is not changed), `undo-send` (a reply to the selected message, sent: the undo-send offer; only with `KOMOREBI_OFFLINE=1`, as the reply stays queued in the cache and the next start sends it, so use a throwaway copy), `junk` (Report junk on what is selected: on the seeded cache, which has no Junk folder, the line saying so), `offline` (the first account's status line as when its server can't be reached, with Try now, and the caution badge on its pane row and rail tile), `offline-settings` (the same, then Settings with that account's line), `settings-download` (Settings with the first account's mail downloading, made-up numbers; its bar shows after a second, so add `KOMOREBI_SNAPSHOT_HOVER` to take the snapshot later), `unread` (the Unread filter), `read-all` (Mark all as read, then the Unread filter: its empty state; this changes the cache, so use a copy), `tips` (the teaching tips of this layout from the first one), `tips:folders`, `tips:search` or `tips:help` (the series at that tip, its place counted as in the series: `tips:search` is "1 of 2" at 1280 and "2 of 3" at 900; `tips:folders` at 1040 px or wider prints "tip not applicable at this width" to stderr and exits 0 without a snapshot; nothing is saved as seen), `whats-new` (the What's new bar, on a cache with an account and no or another `whats_new_version`; otherwise it says so on stderr) or `shortcuts` (the Keyboard shortcuts dialog). Snapshot runs never start the tips or the What's new bar by themselves. `KOMOREBI_SNAPSHOT_SEARCH=text` types `text` into the search box first (with `KOMOREBI_SNAPSHOT_ACTION=search-all`, in the "All folders" scope; `joao` on the seeded cache). `KOMOREBI_SNAPSHOT_ACTION=menu:scope` prints the search scope menu, `inactive` draws the window as inactive (the title bar dims). `KOMOREBI_SNAPSHOT_TITLEBAR=custom` or `system` picks the title bar (snapshots draw the app's own unless `KOMOREBI_THEME=contrast`); every snapshot prints the bar's drawn and pushed height and its passthrough rectangles to stderr (`[titlebar]`), and the layout in effect (`[layout] width=… mode=wide|compact|one-pane list=… reader=…`; `KOMOREBI_SIZE=560x700` shows one pane). `KOMOREBI_SNAPSHOT_FOCUS` then moves the keyboard focus with real key presses from the list: `toolbar` (Tab: the reader's first button; `toolbar:n` then Right n times), `nav` (Shift+F6 to the folders; `nav:n` then Down n times; `nav:n:keys` then presses `l` Left, `r` Right, `u` Up, `d` Down or `e` Enter for each letter), `list` (the list's current row), `tab:n` (n Tabs) or `press:keys` (the letters above plus `n` PageDown, `p` PageUp, `h` Home, `z` End, `f` F6 and `t` Tab, pressed where the focus already is, such as a dialog's or a Settings page's: `press:ttt` walks three Tab stops into Settings, which `tab:n` would leave; in one pane with a message open the list is hidden, so `toolbar` and `tab:n`, which start from it, show no focus: use `press:t` there, or `press:de` to open a row from the list). `KOMOREBI_SNAPSHOT_HOVER=x,y` then points at x,y (logical px) to show a hover, `x,y:press` also holding the left button down there and `x,y:click` clicking there (the snapshot is taken 700 ms later, so a tooltip shows); `caption-min`, `caption-max` or `caption-close` (`:press` held down) show a caption button's state. `KOMOREBI_SNAPSHOT_CHECK=1` then exits with code 1 (and says which on stderr) if the message list or the reader came out blank (less than 1% of their pixels unlike the background); CI uses it with `KOMOREBI_SNAPSHOT_SELECT=0` |

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
attachment, and `--signature` for two named signatures set as the account's defaults. With an eml-dir,
`--html-reply` adds, newest, a reply to the newest HTML message as Komorebi
sends one (formatted text, signature and the sanitized HTML quote), as its
sender would receive it (`KOMOREBI_SNAPSHOT_SELECT=0` opens it). For the folder
pane, `--accounts` adds nested folders and two more `.invalid` accounts,
`--folders N` N more folders, and `--fold a2,f5` saves those lines folded (the
ids are printed). `examples/peak_memory` runs a program (the app, with
those variables set) and prints its peak working set and private memory.

## License

GPL-3.0-or-later (LICENSE). Third-party notices are in NOTICE. Slint is used
under its GPL-3.0 option; its attribution still appears in Settings → About.
Changes are listed in CHANGELOG.md.
