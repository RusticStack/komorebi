# Using Komorebi

Adding an account, installing, and the keyboard.

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
   server settings (hosts, ports, encryption, username, and "This server
   does not need a sign-in for sending" for a relay on your own network:
   no password goes to it, and TLS or STARTTLS is still required). Sign in checks
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
x64), or build from source ([BUILDING.md](BUILDING.md)). Releases come in two forms:

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
(Get-FileHash .\komorebi-0.7.0-setup.exe -Algorithm SHA256).Hash.ToLower()
Select-String komorebi-0.7.0-setup.exe .\SHA256SUMS
```

How releases are made, signed and published is in
[RELEASING.md](RELEASING.md).


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
| Up / Down, Enter or Tab, Esc | Pick, take or close the recipient suggestions (while they show under an address field; a group adds each of its members) |
| Enter (in the contact editor's Name box) | Save the contact |
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

