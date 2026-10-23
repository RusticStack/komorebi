# Features in detail

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
  server (TLS or STARTTLS only). What is known about each, and what is
  not (nothing below is assumed):
  - *Sent copy:* SMTP never copies to Sent by itself, so "save a copy to
    Sent" is on for new accounts. Postale.io is known from live tests not to
    save sent mail on the server (on). Purelymail has no documentation
    either way and keeps the general default (on; unverified). Accounts that
    exist keep their stored choice.
  - *IDLE:* Postale.io (Dovecot) reports only flag changes over IDLE, never
    newly delivered mail, and an open selection sees new mail only after a
    new SELECT; the timer in Settings finds it. After five delivered
    messages in a row that a pass found while IDLE was connected and
    silent, with none announced between, Komorebi gives that IDLE connection
    back for the rest of the session (a restart tries again). Unverified for
    Purelymail, Stalwart, Cyrus and Proton Bridge.
  - *`Authentication-Results`:* the sender notice trusts the topmost header
    whose authserv-id has the IMAP server's registrable domain (the last two
    labels, three under `co.uk` and the like), because RFC 8601 servers
    remove such headers that arrive from outside and write their own on top.
    A header from another domain is ignored wherever it is, one above it
    included. Which authserv-id Purelymail, Postale.io, Dovecot, Stalwart,
    Cyrus and Proton Bridge write is unverified, and Dovecot itself writes
    none (the MTA does, with its own configured name, often the MX host's):
    a provider whose id is in another domain shows no DMARC notice. Nothing
    is learned or guessed per account.
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
  convert to your zone; a named zone is converted with the definition
  the invite carries, else with Windows' own rules for that zone (IANA
  names such as `Europe/Lisbon` and Outlook's Windows names), else the
  time shows with the zone's name ("09:30 (Mars/Base)"); dates are
  all-day. The card reads only the first event; an invite that can't be
  read (or is over 256 KB) shows as an ordinary message. Accept and
  Tentative also add the event to the local calendar (a newer version of
  the same invitation replaces it, an older one is ignored, one changed
  occurrence joins its series), Decline removes it (declining one
  occurrence leaves just that one out), the card says "In your calendar
  on this PC." once it is there, and a cancelled event offers Remove from
  calendar.
- **Calendar:** the Calendar item above Settings (in the folder pane and
  the rail) opens the calendar over the mail, in the view last chosen:
  **Month** (the default: six weeks from the first day of the week Windows
  uses, each day with the events that fit and "+N more", the selected
  day's agenda beside it, or under it on a narrow window), **Week** and
  **Day** (a time grid with an all-day strip, overlapping events side by
  side and a line at the current time), and **Agenda** (30 days: a header
  for each day, each event's time or "All day", title, place, and a mark
  for a repeating event or one with a reminder; a cancelled event says
  so). Today, Previous and Next (Page Up, Page Down; Home for today) move
  by the period shown; the arrow keys move the selected day in Month.
  **New event** and a click on an event open the editor: title,
  calendar, all day, start and end (typed, or picked from a month
  flyout; Up and Down step a time by 15 minutes), repeat (every day,
  week, month on the day or on the weekday, or year; a rule set in
  another app is kept and shown as it is), reminder, place and notes.
  For a repeating event you choose "Only this event" or "The whole
  series" (Delete then leaves out one occurrence or deletes the
  series). An event someone else organized is read only, apart from
  **Remove from calendar**.
  Events live in one local calendar, "On this PC", stored in the cache;
  repeating events are expanded on this computer (daily, weekly, monthly
  and yearly rules with their exceptions; a rule beyond that shows its
  first occurrence). Nothing about the calendar runs at start: the agenda
  is read when the page opens.
  **Calendars** on the command bar lists each account's CalDAV calendars.
  Changes to their events show at once and are sent on the next sync,
  each only over the version you changed (If-Match); when the event
  changed on another device meanwhile, the server's version stays and a
  bar offers **Apply my changes again** or **Discard**, and a calendar
  that refuses changes becomes read only. Komorebi looks for them only when you press **Find
  calendars**: with the server address empty it first looks it up from
  your mail address (DNS SRV and `/.well-known/caldav`, without
  credentials; a server outside your mail domain is shown first and
  used only on a second press), then signs in with the user name and
  password set in
  Contacts > Sync (by default your mail address and mail password, from
  Windows Credential Manager). Task lists are left out; each calendar can
  be turned off. Calendars sync when the page opens, on **Sync now**, and
  at most every 15 minutes while one is on and Komorebi is open
  (sync-collection when the server offers it, else the calendar's ctag
  and ETags). An event under way says **Now**, and one whose reminder is
  due says **In 10 minutes**. With Settings > General > **Notifications
  for event reminders** on (off by default), Windows also shows a
  notification with the event's title, start time and place (never its
  notes) when the reminder is due, but only while Komorebi is open or in
  the notification area: one timer in the app, armed to the next
  reminder, no scheduled task or background service. A reminder missed
  by more than five minutes (Komorebi was closed) stays a marker; Pause
  notifications in the tray menu pauses these too, and clicking one
  opens the Calendar.
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
  in webmail) opens read-only, with Edit and Forward as new draft instead of Reply.
  Discard asks first when the draft has anything you wrote. Saved drafts
  are also copied to the account's Drafts folder on the server (flagged as
  a draft, not as read): at most every 30 seconds while you type, when the
  composer closes, and after the next sync for drafts written offline. Each
  new copy replaces the previous one, which is removed like a delete (by
  UID only on servers with UIDPLUS, otherwise only flagged as deleted);
  sending or discarding the draft removes its copy too. No folder is
  created: without a Drafts folder, or with attachments over 10 MB, drafts
  stay on this computer (said once per session). A draft that is only on
  the server opens read-only in the reader with an Edit button: it makes a
  draft of this computer from its text and attachments, linked to the
  server's message, so the next save replaces that message (the folder
  keeps one copy) and Discard removes it; opening it again finds the same
  draft. A wrong address shows a note under
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
  Contacts in the Contacts view (and their groups) come first in the list;
  see the Contacts entry above.
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
- **Rules** (Settings > Rules): each account has an ordered list of
  simple local rules for new mail in its Inbox. A rule looks at the
  sender, To or Cc, the subject or whether there is an attachment
  (contains, does not contain, is, starts with, ends with; case and
  accents ignored), with all or any of its conditions, and can move the
  message to a folder, delete it (to Trash), mark it read or important,
  and stop the rules after it. Rules run in the sync worker before a new
  message is stored, so a filed message never shows in the Inbox or
  raises a notification, and one marked read raises none. They run while
  Komorebi is open; mail that arrived while it was closed is sorted at the
  next start (each Inbox keeps a watermark, so older mail is never swept
  up). Nothing is sent: no forward, redirect or auto-reply, and the rules
  stay on this computer (no ManageSieve). A rule whose folder is gone is
  paused until you pick another. The page lists what automatic runs did
  (Activity) and can copy the rules as a Sieve script for a provider's
  filter box (Copy as Sieve). Run now on a folder is not there yet.
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
- **Encrypted cache** (Settings → Privacy → Encrypt the mail cache on this
  computer, off by default): after a question that explains the risk, the
  next start (or Restart now) converts the cache, crash-safe, so every page
  of it and its log is sealed with AES-256-GCM under a key that only your
  Windows user can unlock (`cache.sqlite3.key`, DPAPI). Turning it off
  converts it back. If the key is lost (another Windows user, a reset
  Windows password, a copied data folder) the cache cannot be opened: a
  dialog says so and offers Start a fresh cache (the locked files are set
  aside, dated, never deleted; accounts are added again and mail is
  downloaded again; drafts and the Outbox in it are lost) or Quit.
  Attachment copies (`cache.drafts`, `cache.open`) are not encrypted.
  In-memory caches (tests) stay plain on purpose.
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

