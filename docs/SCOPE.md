# Scope and non-goals

Komorebi is a free, open-source Windows 10/11 email client. Its job is to
read, organize and send mail without getting in the way. It is not a
platform. Scope is deliberately small: check this page before proposing a
feature.

## Goals

1. **Fast.** Window on screen in under 200 ms. All cached mail for the
   selected folder listed in under 600 ms. Scrolling, switching folders and
   opening messages never stutter.
2. **Light.** One native binary, no background services, no web engine.
   Komorebi 1.0 draws with GPUI, which holds more memory at idle than the
   0.7 line did; that is accepted for 1.0 and optimizing it is the work of
   1.1 (docs/PERF.md, docs/GPUI-MIGRATION.md).
3. **Correct.** Mail stays organized the way the server has it: folders,
   read and starred state, deletions, all two-way.
4. **Multiple accounts**, each with its own folders (made, renamed and
   deleted on the server from the folder pane, added 2026-11-16) and an
   optional icon: an image the user chooses, else the mail domain's own
   website icon (asked of that domain only, on by default, one switch in
   Settings > Privacy), else the bundled provider icon, else none.
5. **Fluent 2 look.** Light and dark themes follow Windows, with Segoe UI
   Variable, Segoe Fluent Icons, a 4 px spacing grid and quiet surfaces
   (see `FLUENT2.md` and `WINUI3.md`). From 1.1.2 (owner decision,
   2026-11-16, reversing the neutral-only look) the colours are a theme:
   Komorebi (the default, unchanged), Outlook or GitHub, each light and
   dark, or the user's own, edited in Settings > Appearance with contrast
   checks and shared as a small JSON file of colours. Optionally a
   background picture behind translucent panes (blurred, redrawn as pixel
   art or ASCII, dimmed, faded at the edges, or blurred behind each pane)
   and Windows' Mica or Acrylic, all off by default and
   costing nothing when off. High contrast themes are still not in 1.x.
6. **Write mail.** Compose, reply, reply all and forward, with attachments,
   basic text formatting and named signatures. Formatted mail goes out as
   text/plain plus a generated text/html alternative; any message can be
   switched to plain text. Komorebi sets no limit on attachment size (owner
   decision, 2026-11-16): the account's server is the only judge, and its
   refusal of a message too large for it is shown in the Outbox.
7. **Rules and contacts** (added 2026-10-15). Local mail rules (move, mark
   as read, mark as important, delete to Trash) run by the sync worker on
   new Inbox mail and on demand. Contacts with groups, stored locally and
   synced over CardDAV. Both use the account's own credentials and load
   lazily (never on the launch path). The calendar that 0.7 added with them
   is not in 1.0 (see the non-goals).
8. **Import from Outlook** (added 2026-11-16). Classic Outlook's accounts
   (their settings, never their passwords) and signatures, read on this PC
   only and read-only; each account is still signed in to by the user. Mail
   is not imported: it downloads again from the server. Other apps' data
   (Thunderbird, Windows Mail), PST files and the new Outlook's cloud
   settings are out of scope.

## Non-goals

These features are out of scope. They are not deferred: they are not
planned.

- AI features of any kind, and telemetry.
- OAuth, Gmail and Microsoft 365 sign-in. Nothing is discovered in the
  background: looking an account's domain up (DNS SRV, the domain's
  autoconfig page) happens only when the user presses "Find settings" (or
  Continue on the first-run email step for a domain that is not a preset),
  never sends the password, and is refused in offline mode.
- Labels, snooze, a command palette, notes and a notifications center.
  New mail is noticed through the unread count in the window title, an
  optional taskbar flash and optional native Windows notifications with a
  tray icon. Notifications are local: they come from the existing sync and
  IDLE threads, there is no push service, and nothing leaves the machine.
  Background mode (close to the tray, start with Windows) is opt-in.
- Server-side filters: rules are client-side only (no ManageSieve), and no
  rule ever sends mail (no forward, redirect or auto-reply).
- Decrypting, signing or verifying mail (S/MIME, OpenPGP, DKIM, SPF).
  Encrypted mail is shown as encrypted, with its ciphertext offered as a
  file to save. The sender notice only reads the `Authentication-Results`
  header the account's own server added.
- Icon services: an account's icon is never asked of a third party (no
  favicon service, no Gravatar or brand-logo lookup), never in SVG, and
  never for a sender.
- Rendering HTML mail through a web engine. Komorebi has its own small
  layout engine; remote content is never loaded.
- A general HTML editor, or a web engine in the composer. The composer
  edits Komorebi's own small document model and generates the HTML itself.
  Out of scope: web or custom fonts, a free colour picker, highlight,
  headings, merged table cells, stationery, templates, HTML source editing,
  signature files (beyond the one-time Import from Outlook) and server-synced
  signatures. Pictures in the message body are in scope (the owner reversed
  the old non-goal): PNG, JPEG, GIF and WebP pictures carried in the message
  itself (`multipart/related`, `cid:`), never SVG and never a picture
  fetched from the network.
- A calendar: no agenda, day, week or month views, no event editing, no
  CalDAV sync, no event reminders and no adding an invitation to a calendar.
  The calendar of 0.7 is not in 1.0 (owner decision, 2026-11-06); it is
  planned to return in 1.1.0, and its library code (`src/cal`,
  `src/dav/caldav`, the calendar tables and their tests) stays in the tree,
  compiling and tested, for that. A
  meeting invitation in a message is still shown on a card, and Save .ics
  hands the event to the calendar app you use.
- Windows beyond the main one and its read-only message windows. Closing the
  main window closes every window and ends the app. No dragging tabs out into
  windows, no split view inside one window.

## Supported providers

Built-in presets exist for [Purelymail](https://purelymail.com) and
[Postale.io](https://postale.io). Any other IMAP/SMTP server can be added
with manual settings (TLS or STARTTLS only; plaintext connections are
refused; an SMTP relay without sign-in is allowed, over TLS only). The
local cache can be encrypted on request (Settings > Privacy, off by
default). The username is the full email address; with two-factor sign-in
use an app password.

## Performance budgets

- **Launch under 200 ms:** from process start to the first presented frame
  on a warm start. Startup work is limited to opening SQLite (WAL mode),
  reading accounts, settings and folders, and reading the header rows of
  the selected folder. Nothing on the network runs before the first frame.
- **Cached list under 600 ms** for the selected folder.
- **Release binary under 21.5 MB** (21,500,000 bytes), for `komorebi.exe`, the Komorebi 1.0 release exe. The ceiling was 20.0 MB, then 20.5 MB; it was raised to 21.5 MB on 2026-10-25 for Komorebi 1.0 on GPUI (docs/GPUI-MIGRATION.md). Higher idle memory and CPU than the 0.7 Slint app are accepted for 1.0 (optimization is planned for 1.1); launch and cached list keep their budgets.

See `PERF.md` for how to measure.
