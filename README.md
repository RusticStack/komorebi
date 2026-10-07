# Komorebi

A small, fast email client for Windows 10/11. It is free and open source,
uses a Fluent 2 look, and does one job: reading and writing mail without
getting in the way.

- **Providers:** built-in presets for [Purelymail](https://purelymail.com) and
  [Postale.io](https://postale.io), plus manual settings for any IMAP/SMTP
  server (TLS or STARTTLS only).
- **Multiple accounts**, each with its own folders, plus a combined
  "All inboxes" view.
- **Reading:** a native text reader. HTML mail is converted to clean text,
  and remote content is never loaded. Attachments can be saved.
- **Writing:** compose, reply, reply all and forward, with attachments.
- **Organizing:** read/unread, flag, archive, delete and move, all
  two-way with the server.
- **Private:** no telemetry, no ads, no AI. Passwords stay in Windows
  Credential Manager.

See [`Project Komorebi - Email Client Plan.md`](Project%20Komorebi%20-%20Email%20Client%20Plan.md)
for scope, non-goals and the performance budgets.

## Build

```powershell
cargo build --release   # target\release\komorebi.exe
```

Stack: Rust + [Slint](https://slint.dev) (Fluent style, software renderer),
`rusqlite`, `imap`, `lettre`, `mail-parser`, `keyring`, `rfd`.

## Checks

```powershell
cargo check --all-targets
cargo test
cargo clippy --all-targets
cargo fmt --check
```

## Keyboard

| Key | Action |
|---|---|
| Ctrl+N | New mail |
| Ctrl+R / Ctrl+Shift+R | Reply / Reply all |
| Ctrl+F | Search |
| Ctrl+Q | Toggle read |
| Up / Down | Previous / next message |
| Delete | Delete (move to Trash) |
| F5 | Refresh |

## Development switches

| Variable | Effect |
|---|---|
| `KOMOREBI_DB=path` | Use this cache file instead of `%LOCALAPPDATA%\komorebi\cache.sqlite3` |
| `KOMOREBI_OFFLINE=1` | No network and no credential access (demo caches, timing runs) |
| `KOMOREBI_TIMING=1` | Print startup timings to stderr |
| `KOMOREBI_SNAPSHOT=file.bmp` | Render the window to a file and exit. Options: `KOMOREBI_SIZE=WxH`, `KOMOREBI_THEME=light` or `dark`, `KOMOREBI_SNAPSHOT_SELECT=n`, `KOMOREBI_SNAPSHOT_ACTION=reply`, `compose` or `settings` |

## License

Apache-2.0. Slint is used under its royalty-free license; the attribution
appears in Settings → About.
