# Environment variables and development switches

These are read by `komorebi.exe` (Komorebi 1.0) and the library. The app's own switches
(`KOMOREBI_EXIT_AFTER_FIRST_FRAME`, `KOMOREBI_GPUI_FORCE_WARP`, `KOMOREBI_GPUI_PREFLIGHT_FAIL`,
the gallery) are in [GPUI-BUILD.md](GPUI-BUILD.md) "Development switches"; the snapshot ones in
[GPUI-TESTING.md](GPUI-TESTING.md) "Pixel snapshots" (a snapshot is refused without
`KOMOREBI_OFFLINE` and a throwaway `KOMOREBI_DB`).

| Variable | Effect |
|---|---|
| `KOMOREBI_LANG=system`, `en` or `pt-PT` | The display language for this run, over the one saved in Settings (nothing is saved). |
| `KOMOREBI_DB=path` | Use this cache file instead of `%LOCALAPPDATA%\komorebi\cache.sqlite3` (`crash.log` and the diagnostics log's `logs` folder then go beside it too, draft attachments into `<path without extension>.drafts` and opened attachments into `<path without extension>.open`; each cache file has its own single-instance lock) |
| `KOMOREBI_PASSWORD=secret` | With `KOMOREBI_DB`, use this password instead of Credential Manager (debug builds, or release built with `--features dev-env-password`) |
| `KOMOREBI_NO_SERVER_DRAFTS=1` | Debug builds only: keep drafts on this PC instead of copying them to the server's Drafts folder |
| `KOMOREBI_OFFLINE=1` | No network and no credential access (demo caches, timing runs) |
| `KOMOREBI_TIMING=1` | Print startup timings to stderr (up to the first frame drawn), and how long each opened message took to its first tile |
| `KOMOREBI_BENCH=steps` | Development only, for the A/B harness (`scripts/opt-ab.ps1`, docs/PERF.md section 8): after the first frame, run these comma-separated steps on the live window and print `[bench]` lines to stderr: `wait:ms`, `mark:name`, `open:row` (a row of the list, to its tiles, then to complete), `read:frames:px` (the open message scrolled, counting blank tiles), `scroll:frames:px`, `compose`, `type:n`, `quit`; `sample:name` and `sample-end` in a build with `--features sampler`. Refused (exit 2) unless `KOMOREBI_OFFLINE=1` and `KOMOREBI_DB` names a throwaway cache. Off by default. |
| `KOMOREBI_ACCENT=#rrggbb` | Use this accent colour instead of Windows' (an approximate ramp of shades around it), or 7 comma-separated colours, Dark3 to Light3, for an exact ramp (green: `#0c1c14,#173525,#1e4732,#26593f,#2e6a4b,#357c58,#40956a`; WinUI's blue: `#002642,#004275,#005a9e,#0078d7,#429ce3,#76b9ed,#a6d8ff`) |
| `KOMOREBI_THEME=contrast` | Show a contrast theme (any run, not only snapshots; `light` and `dark` are snapshot options, below): `KOMOREBI_CONTRAST=hc1`, `hc2`, `hcblack` (the default) or `hcwhite` (the four themes in `C:\Windows\Resources\Ease of Access Themes`), or 8 comma-separated colours: Window, WindowText, Highlight, HighlightText, Hotlight, GrayText, ButtonFace, ButtonText. hc1 `#2d3236,#ffffff,#a1bfde,#212d3b,#70ebde,#a6a6a6,#2d3236,#b6f6f0`; hc2 `#000000,#ffffff,#d6b4fd,#2b2b2b,#8080ff,#a6a6a6,#000000,#ffee32`; hcblack `#202020,#ffffff,#8ee3f0,#263b50,#75e9fc,#a6a6a6,#202020,#ffffff`; hcwhite `#fffaef,#3d3d3d,#903909,#fff5e3,#1c5e75,#676767,#fffaef,#202020` |
| `KOMOREBI_TEXT_SCALE=1.5` | Use this text size (1.0 to 2.25) instead of Windows' |
| `KOMOREBI_ANIMATE=0` | Act as if Windows' animation effects were off |
| `KOMOREBI_SYSTEM_TITLEBAR=1` | Keep the Windows title bar, as on Windows 10 |
| `KOMOREBI_SNAPSHOT=file.png` | Render the first frame to a file and exit, at the default place and size (the saved window place is ignored). Refused (exit 2) unless `KOMOREBI_OFFLINE=1` and `KOMOREBI_DB` names a throwaway cache. Its options (`KOMOREBI_SIZE=WxH`, `KOMOREBI_THEME=light` or `dark`, `KOMOREBI_SNAPSHOT_SELECT`, `KOMOREBI_SNAPSHOT_ACTION` for each screen, `KOMOREBI_SNAPSHOT_FOCUS`, `KOMOREBI_SNAPSHOT_HOVER`, `KOMOREBI_SNAPSHOT_CHECK` and the rest) are listed in [GPUI-TESTING.md](GPUI-TESTING.md) "Pixel snapshots". |

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
him), for checking the search. `KOMOREBI_TIMING=1` also prints the launch timings.
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
ids are printed). `examples/demo_mailbox` makes the fictional demo mailbox of the README screenshots (invented people on example.com and example.org; the path must contain `komorebi-shots`). `examples/peak_memory` runs a program (the app, with
those variables set) and prints its peak working set and private memory.

