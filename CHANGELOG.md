# Changelog

All notable changes to Komorebi are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/). Each entry says what it
changes in the local data (the cache, its folders and Credential Manager).

## [Unreleased]

### Added

- Sender-set importance: the `Importance`, `X-Priority` and
  `X-MSMail-Priority` headers are read in the same header FETCH as the
  rest of a list row (no extra round trip). A high-importance message
  shows a bold "!" in the text colour before its date ("High importance"
  to screen readers), apart from the user's amber Important flag; a
  low-importance one a small Down arrow in the secondary text colour
  ("Low importance"). The reader says "Sent with high importance" (after
  the same "!") or "Sent with low importance" under the To line.
- The composer's formatting bar has High importance and Low importance
  toggles; the message is sent with `Importance` and `X-Priority`
  headers (none for normal mail), and the choice is saved with the draft
  and its server copy. Changing it, or the From account, on an empty new
  message keeps nothing (Esc still drops it silently).
- Local data: the cache moves to schema v13, adding `messages.priority`
  and `drafts.importance` (both default to normal). Headers already
  cached are not fetched again and the upgrade does not scan the cache
  (so the first start stays fast). Instead a message cached before v13
  gets its importance from its body: when the body is downloaded (the
  background prefetch or opening it), or, if it was already downloaded,
  when it is opened (the reader reads it from the cached copy and the row
  keeps it). Until then its row shows none. No extra round trip. The step
  is one way: 0.3.0 leaves a v13 cache as it is and does not open it.

### Changed

- Read and unread rows look more different: an unread row has its
  sender, subject and date semibold in the text colour and an 8 px accent
  dot before the sender; a read row is regular in the secondary text
  colour (still 4.5:1 on every plate; the window text colour under high
  contrast). The old dot at the end of the subject line is gone, and the
  row's text sits 12 px further right so read and unread rows line up.
  Local data: none.
- Flags are now "important" messages: Mark as important / Remove
  importance (row menu, quick actions, reader and selection buttons) and
  an Important filter tab ("No important messages" when empty). Still the
  IMAP `\Flagged` flag, so other clients keep showing them as flagged or
  starred. The row marker is a 14 px amber flag (Fluent 2 marigold, apart
  from the red of drafts and failed sends) at the end of the subject line;
  hovering a row shows an outline one there, and clicking it marks or
  unmarks that row only. Ctrl+Shift+G does what Insert does, for
  keyboards without an Insert key. Local data: none (no schema change).
  On a draft or an Outbox row the flag is only shown: screen readers hear
  "Important" once, in the row's name, not again as a disabled button.

## [0.3.0] - 2026-10-13

The rewrite: a small Slint app in place of the GPUI/AI prototype. This
section lists everything since the start of the rewrite, including what
the 0.2.0 pre-release (2026-10-07) already had. It is still a
pre-release: the binary is not code-signed, and self-hosted servers have
been tested only against local fakes (see TODO.md).

### Added

- Folder tree in the folder pane: subfolders sit under their parent
  (including under Inbox, Sent or Archive), each parent folder has an
  expander, and folded accounts and folders stay folded after a restart.
  A folded folder shows the unread mail inside it; Left and Right fold,
  open and move through the tree as in Windows. The pane is 16 px wider
  (264 px). Local data: one new setting, `nav_folded`, in the cache's
  existing settings table (no schema change).
- Folder pane accounts read as groups: space above each account, a round
  initial in the account's colour, the account's name (or its address) in
  bold, a leading chevron, and a tooltip with the name and address. Unread
  counts are neutral instead of accent-coloured and stop at 999+; the
  server's Drafts folder shows how many drafts it holds; long folder names
  show in full as a tooltip. Local data: none.
- Narrow windows (UX final, Stage 8): the window goes down to 500 px wide
  (was 720), so Snap Layouts' halves and thirds fit common screens. Below
  720 px one pane shows at a time: the list, or the open message, composer
  or Outbox entry with a back arrow ("Back to list (Esc)"); the status line
  and the Undo offer then float at the foot of the reader; while the list
  shows there, the arrow keys only move through it and Enter opens (the
  back button alone takes the focus when the list hides). Below 1040 px
  the list stretches over the reader's place while nothing is open. In
  English, dates with a year (the list's older rows, the reader's date)
  follow Windows' short-date format, read once at start, with ISO 8601 as
  the fallback; quote headers in sent replies keep ISO, so what is sent
  does not depend on the PC. Local data: none.
- Onboarding tips, help and shortcuts (UX final, Stage 5): short
  teaching tips, one at a time and never in the way (no smoke, no focus
  taken, Esc is Got it): Search and Help on a wide window, and Folders
  first when the folder pane is folded into the narrow rail; "1 of 2",
  Got it and Skip tips. Each is shown once, in either layout. After an
  update, a "Komorebi was updated to …" bar with What's new (the
  changelog, through the link prompt), never together with a tip.
  Settings › About gains What's new, Show tips again, Keyboard shortcuts,
  Get help, Report a problem (the issues page; no data is attached) and
  Privacy statement. F1 or Ctrl+/ opens a Keyboard shortcuts dialog with
  every key of the README's table (Up / Down, PageUp / PageDown, Home and
  End scroll it; F6 stays inside it, as in every dialog). A tip hides
  while the Move list is open. Local data: two new settings,
  `tips_seen` and `whats_new_version`, in the existing settings table (no
  schema change); the version is saved silently when the first account
  is added.
- Settings, accounts and trust surfaces (UX final, Stage 7): each account
  is a card that opens its page ("Accounts › name"): "Sync this account"
  with what Off does, Name, Email (with why it can't change), Password,
  Signature, Folders (each role's header above its box, so no label is
  cut in Portuguese), Server settings folded in an Expander (it opens on
  a failed check or a saved certificate; Find settings only for a server
  that isn't a preset), Download again and Remove; Save, Test connection
  and Cancel stay pinned under the page; Move up / down sit in its "…"
  menu. A changed certificate says so in one short sentence with Review
  certificate (it opens the page and reads the new certificate into the
  prompt); the prompt shows fingerprints as 4 lines of 8 bytes, the old
  one beside the new with a caution line, a Copy button and "How do I
  check this?". A server out of reach offers Try now and puts a caution
  badge on the account's row and rail tile until a sync goes through. The
  sender notice reads "This message may not be from who it says" with the
  reason under it and Report junk, Delete and Why?. "Always load from this
  sender" is followed by a bar with Undo; later messages say "Images load
  from this sender · Change". Privacy holds remote images, the
  diagnostics log with Open logs folder, and Reset (Remove all data);
  About has Help and About groups, the data path cut from its start with
  the whole path in its tooltip, and a Made with Slint card. A language
  change says that some text changes after a restart. Local data: none
  changed (no new settings, no schema change).
- Composer, drafts and Outbox (UX final, Stage 6): a Close button in the
  composer's header, and every way out of it (Close, Esc, a click on a
  message or an Outbox row) saves the draft on this PC and copies it to
  the server's Drafts at once, saying "Draft saved"; the header shows
  "Draft saved 14:05", refreshed by each save. Send is always on: pressed
  with no recipient or a wrong address, the field says why and takes the
  focus. The undo-send offer names the message ("Sending "Plans" in 5 s").
  The Outbox says why a send failed in a plain sentence for the kind of
  refusal (for example "ana@example.org doesn't exist."), with the
  server's reply under Details and the first lines of the text; Edit or
  Retry is the main button depending on whether sending again can help;
  a failed send's status line stays until closed. One discard sentence
  everywhere: deleted here and from the server's Drafts, with the
  attachments. Preview (a toggle on the formatting row) shows the message
  as sent; the plain-text part and the Drafts and Outbox previews no
  longer show `**` and `_` marks; Quote has Ctrl+Shift+Q; a one-time
  caption explains the marks. Replies and forwards fold the quote behind
  "Show quoted text" and write its header line in the language shown;
  an empty body says "Write your message", and text typed over the whole
  body keeps a blank line before the quote; a Drafts row previews the
  text above the quote. Dragging files over the window
  shows "Drop to attach" (or "...to a new message", which opens one). In a
  narrow window Discard and Send keep their labels. A message with a
  reply in progress says so in the reader, with Resume. Local data: one
  new setting, `format_hint_seen`, in the existing settings table (no
  schema change). Behaviour: a click on a message or an Outbox row while
  writing now also copies the draft to the server, as Esc did; nothing
  else changes in what is sent, saved or deleted.
- First run and account setup (UX final, Stage 4): the welcome has the
  whole window (no folder pane, rail or search until there is an account)
  with a line on what stays private and the links "Need help?", "Privacy"
  and "Settings". The add form asks Email, Password (saved in Windows
  Credential Manager, said under it), Provider as three choices with a line
  each (picked from the address for purelymail.com and postale.io) and an
  optional name; the signature and the Sent copy moved to the account's
  Edit page. Add account and Test connection stay enabled and say what is
  missing under each field. Other server's fields are in a "Server settings"
  expander, and with no server typed Add or Test looks them up first (only
  on that press). Cancel, Back and Esc stop a running check; after 3 s the
  wait says what to check. A failure keeps the steps inside its message,
  scrolls it into view (its top under the page header, never cut by it;
  final UX audit) and offers Set username (a username now works with
  Purelymail and Postale.io too) or Review certificate; a rejected password
  marks the Password box, and the sentence fits the provider. A new account
  opens on its Inbox with its download's progress in the empty list ("Newest
  messages arrive first"), the status line stays until the first pass ends,
  and an empty folder says "This folder is empty" once synced. The window no
  longer opens at its minimum height on a first run. Local data: none (no
  schema or setting change; a username typed for a preset is stored in the
  existing columns).
- Easier navigation (UX final, Stage 1): Settings has a back arrow ("Back
  to mail", Esc), and the gear and Ctrl+, open and close it. With one
  account there is no All inboxes: Komorebi opens on its Inbox, and the
  view you left is opened again at the next start. Long folder trees start
  folded (parent folders without unread mail of their own; with several
  accounts, every account after the first), so all accounts show at once.
  The narrow window's rail is 64 px with a name under every icon (New
  mail, All inboxes, Drafts, the accounts, Folders, Settings) and unread
  counts as badges; the folder pane opened over it dims the rest of the
  window. Drafts is always in the folder pane: New mail always starts a
  blank message, the start-up "saved drafts / Resume draft" line is gone,
  and drafts take Enter, Delete (asking first) and a right-click menu
  (Open, Discard). The server's Drafts folder no longer repeats the drafts
  saved here, shows no filters or bold, and its drafts open with Forward
  as new draft instead of Reply. The list shows a folder's account under
  its name. The folder pane menu is Mark all as read and Collapse all (on
  an account also Refresh and Account settings; "Expand all" and the
  per-folder Expand / Collapse entries are gone: the chevrons and
  Left/Right do that); a rail tile's menu goes to its Inbox, Sent or
  Archive. Local data: two new settings in the existing settings table,
  `last_view` and `nav_folds_init` (no schema change); the first start
  after the update folds each account's parent folders without unread
  mail once.
- A Windows 11 title bar (UX final, Stage 2): Komorebi draws its own 48 px
  title bar as Windows 11 Mail and the new Outlook do, with the icon and
  name, the search box in the middle and the minimize, maximize and close
  buttons (Snap Layouts, drag, double-click and the system menu work as
  before; it dims while another window is in front). On Windows 10 and
  under high contrast the Windows title bar stays, with the same search
  strip under it, and turning high contrast on or off switches it while
  the app runs. The search moved there: Ctrl+E, F3 or Ctrl+F reach it
  (from Settings too), it says what it searches ("Search Newsletters"),
  its scope is "This folder" or "All folders", the query stays when you
  open another folder, the list says "Search results" with how many were
  found where, results from all folders show their account and folder,
  and Show in folder opens one in its folder (the list's Mark all as
  read is off while results show: it would cover the whole folder). Archive is now Backspace
  (on a message or the reader), as in the new Outlook. The window keeps
  its size across restarts. Local data: none.
- Easier triage (UX final, Stage 3): rows say unread with bold text and a
  small accent dot at the end of the subject line, and in All inboxes
  name their account in a text tag (no more coloured dots). Pointing at a
  row (or reaching it with the keyboard) shows Archive, Delete, Flag and
  Mark as read / unread buttons that act on that message only: they never
  open it or mark it read. "Select messages" in the list's "..." menu
  shows check boxes; the selection panel has Select all, Clear selection
  and says why Move is off. The reader's bar keeps its most used labels
  longest (Reply, Archive and Delete also in Portuguese at 1280 x 800);
  Mark as read / unread and Flag / Remove flag are plain buttons that say
  what they do, and every button and menu item shows its key. New keys:
  Ctrl+Q marks read and Ctrl+U unread (as in Outlook; Show original keeps
  its menu item), Ctrl+Shift+F forwards, Ctrl+Shift+V opens Move and
  Ctrl+Shift+I goes to the Inbox. Move has a filter box, your last three
  folders first, and subfolders indented (Drafts folders left out). Undo
  shows at the bottom of the list, never moves the rows or gets replaced
  by another message, lasts at least 10 seconds and waits while you point
  at it. Deleting from Trash (which deletes on the server) or Junk asks
  first. A message not downloaded while offline says so with Retry, and
  one opened from the Unread filter stays in the list until you change
  the filter or the folder. Local data: one new setting, `move_recent`
  (the last folders moved to), in the existing settings table (no schema
  change).
- Folder pane menus: right-click a folder, All inboxes or an account
  (or use its "..." button, Shift+F10 or the Menu key) to mark all as
  read there, refresh one account, expand or collapse all its folders, or
  open its settings. The pane shows a thin scroll bar and edges while
  folders are scrolled away. In a narrow window the rail shows one round
  initial per account (it opens that account's Inbox) instead of every
  folder, and the pane opened over the content scrolls to the open
  folder. Local data: none.
- Languages: English and European Portuguese (pt-PT; Brazilian Portuguese
  is not included, and Windows in pt-BR shows English). The language
  follows the Windows display language, and Settings → General → Language
  (System, English, Português (Portugal)) overrides it and switches at once
  without a restart. Every screen, dialog, error sentence, status line and
  window title is translated; dates, numbers, sizes and plurals follow the
  language (`12 out.`, `1,2 MB`, "0 mensagens"). Text inside the mail,
  server replies and the technical detail after an error sentence are shown
  as received. The catalog is `translations/pt-PT/LC_MESSAGES/komorebi.po`,
  bundled by Slint at build time, and a test keeps it complete. Local data:
  one new setting, `language`, in the cache's existing settings table (no
  schema change); nothing is sent anywhere.
- Find settings: a button in the manual account form that looks up your
  email domain (DNS SRV records, then the provider's autoconfig file over
  HTTPS) and fills the server fields you have not typed in. Only when
  pressed; only the domain is sent (never the address or password); nothing
  goes to a third party; a server on another domain is shown but not filled
  in. Local data: none changed.
- Servers that do less: IDLE, CONDSTORE, UIDPLUS, MOVE and special-use
  folders are optional; folders are recognised by name under `INBOX.` or at
  the top with any delimiter, odd and invalid modified UTF-7 names are kept
  as the server wrote them, and Settings → Accounts → Edit → Folders picks
  the folder for Sent, Drafts, Trash, Archive and Junk. Without a Sent
  folder mail still goes (said once); without a Trash folder nothing is
  deleted. A bare EXPUNGE is never sent.
- An account that needs you (rejected password, changed or untrusted
  certificate, no STARTTLS) makes one attempt, then waits: the status line
  offers Retry or Update password. Local data: none changed (the folder
  roles you pick are stored in the cache's existing `role_override` column).
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
- Windows appearance: the accent colour and the shades Windows derives
  from it (New mail, selection, accent text), light or dark apps under the
  System theme, contrast themes (light or dark as the theme's window
  colour; accent, selection and focus in its colours), the text size and
  animation effects, read at start and whenever Windows says one changed
  (read only, never stored); a title bar coloured as the window, or
  Windows' own with accent title bars or a contrast theme. Colours,
  strokes and text follow WinUI 3's theme resources (contrast themes
  included; every control keeps a 1 px edge, links in the theme's
  hyperlink colour); no drop shadows. No local data change.
- UI polish round (WinUI 3 / Fluent 2): every field of the account form
  is a WinUI text box (the signature too) or a drop-down with a
  Button's edge, with contrast-theme borders. No local data change.
- WinUI-style controls: standard, accent, subtle, toggle and link buttons
  that get lighter when hovered; text fields with a strong bottom edge
  (accent while focused) and a clear or show-password button; check boxes,
  info bars, dialogs with a button bar, a delayed progress ring, tabs with
  a selection pill (one Tab stop, arrows select) and a flyout for Move;
  tooltips on icon buttons and the narrow rail; the double focus ring shows
  only after a key moves the focus. Status colours mark only status (a
  flagged row's red flag, a failed send), never a command: the selection
  panel's Flag button is no longer red, and its buttons go two to a row
  so they fit a narrow window at a large text size.
- Windows 11 shell: the list and the reader on one card with a rounded
  top-left corner beside the folder pane, as in Settings and File
  Explorer; WinUI navigation items (hover, pressed and selected states,
  the accent pill, folding account lines with a chevron); Settings at the
  foot of the folder pane; in a narrow window a 48 px rail whose toggle
  opens the folder pane over the content (Esc or a click outside closes
  it).
- Message list as a Windows 11 list: rounded hover and selection plates
  with the accent pill, no lines between rows, unread mail in semibold
  with an accent date; All / Unread / Flagged tabs; a search box with a
  search button and an "All folders" toggle beside it while searching;
  empty lists explain themselves in the middle with one action; a thin
  scroll thumb over the rows that widens under the pointer; a small ring
  while a sync lasts more than a second.
- Reader as a Windows 11 command bar and info bars: labels by priority as
  the reader narrows (the same in both themes and for every message),
  Unread and Flag as toggles, a Move chevron, and View as plain text and
  Show original colours (checked while on) in the reader's More menu; the
  sender check, encryption and remote-image notices as info bars (the
  image bar names the sender's address and can be closed); attachments as
  a wrapping grid with type icons; an outlined invitation card with
  answer buttons; the date moves under the sender in a narrow reader. No
  local data change.
- Composer on WinUI controls: the labels line up in a column that grows
  with the text size; Cc and Bcc are links at the end of the To field;
  why Send is off is its tooltip and description instead of a line that
  moved the fields; recipient suggestions drop down over the fields below
  as a ComboBox list; a quote icon and a divider in the formatting row;
  Insert is the link bar's accent button; the size counter gets a caution
  or error icon; the Discard prompt has no default button and Cancel has
  the focus. The undo offer counts down ("Sending in 9 s") for the whole
  undo window (before, it vanished after 5 seconds), and a delivered
  message says "Sent". The Outbox entry shows its commands on top (Edit,
  Retry, Discard) and where its send is as an info bar; failed rows get a
  red mark. No local data change.
- Settings as a page in the window, as in Windows 11's own apps, instead
  of a dialog: the Settings item is selected, the folders stay usable
  (choosing one or New mail leaves Settings), tabs for the sections and
  a column of setting cards; switches for settings that apply at once
  (Enabled, taskbar flash, diagnostics log); the account form with
  headers above its fields and server and port side by side, and while
  it signs in a ring with "Signing in …" after a second; a download bar
  with "Downloading 1,234 of 5,000 …" in the account's card; About as
  cards with Copy and Open for the data folder. Every prompt has the
  Windows dialog's button bar on one smoke layer; prompts that remove,
  discard or send again have no default button and start on Cancel, the
  others have the accent answer that Enter gives; Show original gets the
  button bar too. Closing Settings or a prompt gives the focus back to
  what opened it. No local data change.
- States and first run: the status line is an info bar at the top of the
  message list (and of Settings) instead of a toast over the rows, with a
  severity: a server that can't be reached is a caution that stays until
  the account syncs again (before, it went after 12 seconds), errors are
  critical, the rest informational; Undo offers keep their time. Waits
  show nothing for the first second ("Downloading message …" and
  "Fetching message …" too), and every ellipsis has a no-break space
  before it. Drafts and the Outbox say "No drafts" and "Outbox is empty.
  Messages wait here while they are sent." when empty, the reader under
  Drafts "Select a draft to keep writing"; opening Drafts answers the
  "Resume draft" offer. With no account, the welcome says what is needed
  and how long it takes, its Add account opens the form directly, and
  New mail is hidden. No local data change.
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
- Speed: the HTML reader opens image-heavy mail up to about 40 percent
  faster (the tile raster is built at full optimization, 50 KB smaller) and
  list, filter and search queries run 10 to 25 percent faster, for 0.3 MB
  more in the exe (17.07 MB). No local data change.
- Speed: search matches the six columns in one pass without copying them, about 38 percent
  faster at 20000 messages (14.5 to 9.0 ms), for 1.5 KB more in the exe. No local data change.
- Speed: the cache opens and the title bar icon loads while the window is created, and a first launch writes its default folds after the first frame; the cache is ready about 2 ms sooner (9.8 to 7.9 ms) and the first frame about 1.5 ms sooner, for 7 KB more in the exe. No local data change.
- Speed: background body downloads are written one transaction per fetched batch of up to 25 instead of one per message, so 300 bodies store in 5.9 ms instead of 25.9 ms (on a worker thread; a failed batch stores nothing and is retried), for 3 KB more in the exe. No local data change.

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
- v12: each account's IMAP and SMTP sign-in names (empty: the email
  address), room for a pinned certificate per server, and a folder role the
  user chose. An older Komorebi refuses a v12 cache.
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

[0.3.0]: https://github.com/RusticStack/komorebi/releases/tag/v0.3.0
[0.2.0]: https://github.com/RusticStack/komorebi/releases/tag/v0.2.0
