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
4. **Multiple accounts**, each with its own folders.
5. **Fluent 2 look.** Light and dark themes follow Windows, with Segoe UI
   Variable, Segoe Fluent Icons, a 4 px spacing grid and quiet surfaces
   (see `FLUENT2.md` and `WINUI3.md`).
6. **Write mail.** Compose, reply, reply all and forward, with attachments,
   basic text formatting and named signatures. Formatted mail goes out as
   text/plain plus a generated text/html alternative; any message can be
   switched to plain text.
7. **Rules and contacts** (added 2026-10-15). Local mail rules (move, mark
   as read, mark as important, delete to Trash) run by the sync worker on
   new Inbox mail and on demand. Contacts with groups, stored locally and
   synced over CardDAV. Both use the account's own credentials and load
   lazily (never on the launch path). The calendar that 0.7 added with them
   is not in 1.0 (see the non-goals).

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
- Rendering HTML mail through a web engine. Komorebi has its own small
  layout engine; remote content is never loaded.
- A general HTML editor, or a web engine in the composer. The composer
  edits Komorebi's own small document model and generates the HTML itself.
  Out of scope: web or custom fonts, a free colour picker, highlight,
  headings, merged table cells, images in the message body (a signature may
  carry one small logo), stationery, templates, HTML source editing, signature
  files and server-synced signatures.
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
