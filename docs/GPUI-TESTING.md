# Testing the app (Komorebi 1.0)

How the app (`crates/komorebi-gpui`) is tested, and the patterns each phase copies. The
APIs are the ones Skua's GPUI cookbook shows (`D:\Projects\skua\docs\cookbook\gpui.md`:
cookbook_test, cookbook_a11y, cookbook_actions, cookbook_window_state, "real host I/O"), plus
`Window::dispatch_action` for actions without a key. The plan and its progress:
[GPUI-MIGRATION.md](GPUI-MIGRATION.md); where every test of the deleted Slint app went:
[GPUI-TEST-PORT.md](GPUI-TEST-PORT.md).

## The gate

One command before every commit:

```powershell
cargo xtask check            # fmt, clippy -D warnings, tests, lints, exe size
cargo xtask check --release  # the same, after building the release exe (fat LTO, long)
cargo xtask check --all      # also the library's integration tests (cargo test --locked, long)
```

`check` runs, and stops at the first failure: `cargo fmt --all --check`; `cargo clippy
--workspace --all-targets --locked -- -D warnings`; `cargo test -p komorebi-gpui --locked`;
`cargo test -p komorebi --lib --locked` (the shared library); `cargo test -p xtask --locked`;
the lints below; and the size of `target\release\komorebi.exe` (under `CARGO_TARGET_DIR`
when it is set) against 21,500,000 bytes. Without `--release` the size is checked only when a
release build exists; otherwise the step prints `SKIPPED` and the last line says so (it never
reports a size it did not measure). Its parts on their own:

```powershell
cargo xtask lint                   # token lint and no-timers lint
cargo xtask size                   # the release exe against its ceiling
```

The xtask crate (`xtask/`) uses the standard library only, so it adds nothing to the
dependency graph or to `cargo deny`.

## Kinds of test

| Kind | Where | Attribute | Use for |
|---|---|---|---|
| State | the library, or a plain struct in the app | `#[test]` | Everything that is not a view: drafts, tabs, rows, rules, dates. |
| View | `crates/komorebi-gpui/src/**` test modules | `#[gpui::test]` | A view's state through its methods and GPUI's test window: `cx.new`, `entity.update`, `read_with`, `add_window_view`, `simulate_resize`, the test clock. |
| Window | the same | `#[gpui_kit::test]` | Real Kit components in a headless window: find by accessible name and role, click, type, press keys, dispatch actions, assert state and names. |
| Pixels | the release exe | `KOMOREBI_SNAPSHOT` | What the window looks like (below); never in `cargo test`. |

All three test kinds run on any machine: GPUI's test platform is headless (no HWND, no GPU, no
window on screen), so `cargo test` opens nothing a person could see or that could be left
running.

### The test support module

`crates/komorebi-gpui/src/test_support.rs` (`#[cfg(test)]`) is the shared harness:

| Helper | What it does |
|---|---|
| `init_app(cx)` | `gpui_kit::init`, the app's key bindings (`tabs::bind_keys`), reduce motion on: what `main` sets up before a window. |
| `init(theme, lang, db)`, `plain_init()` | The main window's `Init` with the Komorebi palette for a theme, a language and an optional cache. |
| `open(cx, build)`, `open_main(cx, init)` | A headless Kit window (with Kit's Root) at 1280 x 800, returning its handle and view. |
| `in_window(cx, window, f)` | Runs `f(window, cx)` after a fresh frame (queries read the last completed frame). |
| `names(window)` | Every `(role, accessible name)` in the frame, top to bottom: what UI Automation and Narrator get. |
| `assert_named(window, name, role)` | Exactly one visible element with that name, and its role. |
| `click_named(window, name, cx)` | Click by accessible name (for popup items and other elements with no stable id). By id: `window.click(id, cx)`. |
| `press(window, &["ctrl-tab"], cx)` | Key presses; bindings match first, as on Windows. |
| `type_text(window, "text", cx)` | Text into the focused element. |
| `dispatch(window, &Action, cx)` | An action from the focused element, as a menu item would send it. |
| `TempCache::new(name)`, `seeded(name, count)` | A throwaway cache in its own temp folder, removed on drop (`name` only labels the folder: the process id and a counter make it unique, `test_support::unique_dir`, so two tests may use the same name); `seeded` fills it with `komorebi::seed` (the same account and messages as `cargo run --example seed`). |

A window test, in full (from `window.rs`):

```rust
#[gpui_kit::test]
fn the_caption_buttons_are_named_buttons(cx: &mut TestAppContext) {
    test_support::init_app(cx);
    let init = test_support::init(ThemeChoice::Dark, Lang::PtPt, None);
    let (window, _) = test_support::open_main(cx, init);
    in_window(cx, window, |window, _| {
        for name in ["Minimizar", "Maximizar", "Fechar"] {
            assert_named(window, name, Role::Button);
        }
    });
}
```

### Rules

- **Query by accessible name and role, never by rendered text.** An element is visible to the
  queries only with an `.id(..)`, a `.role(..)`, an `.aria_label(..)` and, last in the chain
  (before `track_focus`), `gpui_kit::TestSupportExt::test_support()` (cookbook_a11y). It is
  inert outside tests. What the queries see is what UI Automation reads, so a test that finds
  a control by its name also proves Narrator can.
- **No timers or sleeps.** Tests move GPUI's test clock: `cx.executor().advance_clock(d)`, then
  `cx.run_until_parked()` (cookbook_window_state). The no-timers lint fails a `thread::sleep`,
  `block_on`, `.timer(` or `Timer::after` anywhere in the app, tests included.
- **Real I/O** (the fake IMAP/SMTP servers of `src/mail/fake.rs`, a worker thread) that wakes
  GPUI tasks from another thread needs `cx.executor().allow_parking()` at the start of the
  test (Skua's "real host I/O in `#[gpui::test]`"); fake data alone stays deterministic.
- **TLS fakes** (`src/mail/fake_tls.rs`) make their server identity with Schannel in a key container named for the process and a counter, deleted at exit; never `native_tls::Identity::from_pkcs8`, whose `native-tls-<n>` containers are shared by every test process of the user and made the fakes present the wrong certificate when two runs overlapped.
- **Caches are throwaway:** `TempCache` or `seeded`, never `komorebi::db::default_path()`, never
  the network (the app's workers do not run in tests).
- **Key bindings** are installed once (`init_app`); views put `.key_context(..)` on an ancestor
  of the focused element and handle actions with `.on_action(cx.listener(..))`
  (cookbook_actions). A window key then wins over a focused Kit `Input`
  (`test_support::tests::names_clicks_typing_and_keys_reach_the_view` proves it).
- `.expect` and `unwrap` are for tests only.

## Tests ported from the Slint app

The Slint app (0.7) is deleted; its tests went with it, each ported to one of the forms below
or marked obsolete in [GPUI-TEST-PORT.md](GPUI-TEST-PORT.md) (the record of where every one
went). A ported test names its source in its doc comment. The forms:

1. **Controller logic moves to a state struct, and its test becomes a plain `#[test]`.** Example:
   the tabs model moved into the library as `komorebi::tabs`; the model half of the old view
   test `close_tab_activates_left_neighbour_and_ctrl_shift_t_reopens` is
   `komorebi::tabs::tests::close_then_reopen_and_move_as_the_tab_keys_do`.
2. **What the user does in the window becomes a `#[gpui_kit::test]`.** Example: the old
   `ctrl_tab_moves_between_tabs_even_from_a_text_field` is
   `window::tests::ctrl_tab_moves_between_tabs`: the same keys pressed in the window, the
   same tabs asserted.
3. **Pixel helpers stay `#[test]`s.** `ink_counts_pixels_unlike_the_background` is
   `snapshot::tests::ink_counts_pixels_unlike_the_background`.

## Pixel snapshots

`KOMOREBI_SNAPSHOT=<file>` on `komorebi.exe` opens the window as usual, waits for its first
frame to be presented, captures the client area, writes it, and quits. The switches are the
ones in docs/ENVIRONMENT.md where they make sense:

| Switch | GPUI app |
|---|---|
| `KOMOREBI_SNAPSHOT=<file>` | Output: PNG when the name ends in `.png`, else a 32-bit BMP. |
| `KOMOREBI_THEME=light\|dark` | Over the saved theme. (No `contrast`: high contrast is not in 1.0.) |
| `KOMOREBI_SIZE=WxH` | Logical px, at least 640 x 480; else 1280 x 800. A snapshot never uses the saved place, and does not save it. |
| `KOMOREBI_LANG=en\|pt-PT` | As for the app. |
| `KOMOREBI_SNAPSHOT_ACTION` | `shell` (the default: the window; the welcome flow on an empty cache, the mail view on a seeded one), `compose` (a new message in the Mail tab's reading pane, as Ctrl+N), `compose-pictures` and `compose-picture-selected` (that message holding a made-up two-step how-to with two generated pictures, the first selected), `signature-pictures` (a new signature's page holding a made-up signature pasted from Outlook: a logo beside the name, partner logos under it), `settings-accounts`, `settings-account`, `settings-signatures`, `settings-rules`, `settings-general`, `settings-privacy`, `settings-about`, `rule-edit`, `contacts`, `contact-edit`, `contacts-sync`, `ob-welcome`, `ob-email`, `ob-sign-in`, `ob-manual`, `ob-providers` (the provider list, offline), `ob-import` (Import from Outlook with made-up accounts and signatures, never this PC's), `ob-sign-in-gmail` (Sign in for a Gmail address: the app password field and its button), `ob-sign-in-gmail-failed` (the same after a made-up refused sign-in) (`snapshot::ACTIONS`). Every other action of 0.7 is refused with exit 2 until its view exists. |
| `KOMOREBI_SNAPSHOT_CHECK` | Exit 1 unless the title bar has ink on the canvas colour and the paper pane is the paper colour (`snapshot::blank_parts`; no title bar check under `KOMOREBI_SNAPSHOT_TITLEBAR=system`, whose caption is outside the client area). With `KOMOREBI_SNAPSHOT_SELECT=<one row>` on the default action it also names a blank list or reading pane (`snapshot::blank_panes`). |
| `KOMOREBI_SNAPSHOT_SELECT`, `_SEARCH` | The mail view's: `_SEARCH=<text>` types a search, `_SELECT=<row>` opens that row (`a,b,c` selects several). |
| `KOMOREBI_SNAPSHOT_FOCUS` | Moves the keyboard focus with real key presses (`Window::dispatch_keystroke`) from the list: `list` (the list's current row), `nav` (Shift+F6 to the folders; `nav:n` then Down n times; `nav:n:keys` then the keys), `toolbar` (one Tab; `toolbar:n` then Right n times: the toolbar has no roving stop yet, so Right does not move along its buttons and `tab:n` is how to reach the nth one), `tab:n` (n Tabs) or `press:keys` (the keys where the focus already is, such as a dialog's). Keys: `l` Left, `r` Right, `u` Up, `d` Down, `e` Enter, `n` PageDown, `p` PageUp, `h` Home, `z` End, `f` F6, `t` Tab, `2` F2, `x` Delete, `m` Shift+F10, `w` Ctrl+Shift+N. |
| `KOMOREBI_SNAPSHOT_TYPE=<text>` | Then types the text with real key presses where the focus is (after `_FOCUS`, or where the view put it; the list is not focused first): a-z, A-Z, 0-9 and space, `\|` a Tab, at most 100. With `compose`, `\|\|\|\|aaasdasdAaaa` types into the To field (four Tabs from where the composer opens: attach, discard, send, To), five Tabs reach "Cc Bcc" and six the subject. The composer's first focus can lose a race with the window's activation (the Tabs then start in the list), so look at the capture. |
| `KOMOREBI_SNAPSHOT_WALK=n` | Then opens the next n messages (at most 1000), 250 ms apart, with Down in the list: for measuring memory after a reading session. |
| `KOMOREBI_SNAPSHOT_SCROLL=px` | Then scrolls the reading pane down this many logical px with a mouse wheel event over its middle. |
| `KOMOREBI_SNAPSHOT_HOVER` | Last, points the pointer at `x,y` (logical px in the client area); `x,y:press` also presses the left button there, `x,y:click` clicks. `caption-min`, `caption-max` or `caption-close` (`:press` held down) point at a caption button. The capture follows 700 ms later, after a tooltip's delay. |
| `KOMOREBI_SNAPSHOT_CLOCK=HH:MM[+S]` | Pins the Rest scene (the calm panel with nothing unread) to a local time, and with `+S` to S seconds into its motion (0 to 3600, such as `21:00+7.5`), with a fixed sky seed (a snapshot otherwise holds motion still; with `+S` over 0 it draws exactly that moment of it, held: the motion does not run on): the same scene can be captured at several moments, such as `+7`, `+7.05` and `+7.1` (one frame apart at the scene's 20 a second), to see what moves and by how much. |
| `KOMOREBI_SNAPSHOT_TITLEBAR=custom\|system` | The title bar (`custom`, the app's own, is what a snapshot draws unless the build is below Windows 11 and the switch says otherwise; `system` keeps the caption: `snapshot::Request::system_title_bar`, `titlebar::app_drawn`). |

The order is the 0.7 app's: the message gets 600 ms to open, then the focus moves, the walk, the scroll and the pointer follow, and the capture is taken after the last wait (`snapshot_drive`). The pointer and the keys are events GPUI gets from the platform, dispatched to the window, so the focus ring, the hover and the tooltip are the real ones. A bad value of any switch is refused (exit 2) with the form it expects, as `KOMOREBI_SIZE` is.

**Safety, in code** (`snapshot::parse`, before the cache or any window opens): when any
`KOMOREBI_SNAPSHOT*` switch is set, the app refuses (exit 2, nothing opened) unless
`KOMOREBI_OFFLINE` is set and `KOMOREBI_DB` names a cache outside the app's own folder
(`%LOCALAPPDATA%\komorebi`, `komorebi::seed::in_real_cache_dir`).

Exit codes: 0 captured (and checked), 1 the capture or the check failed, 2 refused, 3 no
DirectX 11 device (the pre-flight).

### How the capture works, and why not offscreen

The capture is `PrintWindow(hwnd, PW_CLIENTONLY | PW_RENDERFULLCONTENT)` into a GDI bitmap,
read back with `GetDIBits` (Skua's skua-shot capture). With `PW_RENDERFULLCONTENT`, DWM renders
the window's DirectComposition content into the bitmap, so the window may be covered or
behind others; it must not be minimized. It runs on a background thread, because
`PrintWindow` sends the window a message the UI thread must stay free to answer.

An offscreen render (GPUI drawing into a texture with no window) is not possible with the
cookbook's API: GPUI 0.3.9 has no headless renderer, and its test platform draws nothing. A
real window is also what the user sees (title bar, DWM composition, DPI), so the snapshot
checks the real path.

### On a machine without a GPU

`KOMOREBI_GPUI_FORCE_WARP=1` makes GPUI and the DirectX pre-flight use WARP, the software
rasterizer every Windows has. The frame is the same as on a GPU apart from antialiasing at
edges (the check allows 3 per channel). What a GPU-less runner needs:

- An interactive desktop session with DWM (Windows Server with Desktop Experience; the runner
  agent in a logged-on session, not session 0 as a service), so that a window can be created
  and composed. GitHub's hosted Windows images are known to run desktop UI tests in such a
  session (a 1024 x 768 desktop); that the local machine does the same is checked by
  `scripts/ci-local.ps1`. A 1280 x 800 window on a 1024 x 768 desktop reaches past the screen edge;
  `PW_RENDERFULLCONTENT` still renders the whole client area.
- The Windows SDK is not needed to run, only to build (fxc.exe).
- If DirectComposition is not available, `GPUI_DISABLE_DIRECT_COMPOSITION=1` puts the swap chain
  on the window directly.

`scripts/gpui-boot-check.ps1` wraps all of it (the safety switches set and asserted first, a
fresh throwaway cache, a timeout that stops the app):

```powershell
pwsh scripts/gpui-boot-check.ps1 -Exe target\release\komorebi.exe `
  -Cache $env:TEMP\gpui-boot.sqlite3 -Out $env:TEMP\gpui-boot.png -Warp
```

## UI Automation dump

`scripts/gpui-uia-check.ps1` starts the app (offline, throwaway cache, English, optionally WARP),
waits for its window, prints its UI Automation tree (control type, name, automation id,
selection, Invoke, focusable, bounds; adapted from the spike's `tools/uia-dump.ps1`), checks
that each `-Expect` entry (`ControlType:Name`, by default the window "Komorebi" and the three
caption buttons) is there, then closes the window through UI Automation's Window pattern and
waits for the app to exit (it stops the app if it does not). Exit 0 all found, 1 missing or
misbehaved, 2 refused.

## CI

Before pushing, run `pwsh -File scripts/ci-local.ps1` (`cargo xtask ci` wraps it): it runs the steps below, natively on this PC with the pinned toolchain, stops at the first failure and prints each step's time (`-Fast` skips the release build and everything after it that needs the exe, `-From <step>` resumes; throwaway caches and snapshots go under `$env:TEMPkomorebi-ci`). GitHub Actions does not run on every push: `.github/workflows/ci.yml` runs on demand and for pull requests from forks, on GitHub's own `windows-2025` runner (no GPU), with the same steps:
`scripts/find-fxc.ps1` (fxc.exe located and `GPUI_FXC_PATH` set), clippy over the
workspace, `cargo check -p komorebi-gpui --all-targets`, `cargo xtask lint`, `cargo test -p
komorebi-gpui` and `-p xtask`, the release build, `cargo xtask size` (the exe against its
ceiling), its manifest, `scripts/gpui-boot-check.ps1 -Warp -Fallback` (a gate), the
same on a seeded cache with the newest message open (`-Select 0`: the list and the reading pane
must have content, `snapshot::blank_panes`), and `scripts/gpui-uia-check.ps1 -Warp`
(`continue-on-error` until the first run shows what the runner's UI Automation lists; then it
becomes a gate). `-Fallback` runs a blank first try again with `GPUI_DISABLE_DIRECT_COMPOSITION=1`
and says which mode passed. The PNGs, their logs and the UIA dump are uploaded as the `boot-snapshot` artifact.
`release.yml` runs the same tests, size, manifest and both boot checks, and builds the installer
around the exe (`unsigned-installer`, never signed or attached to a release by CI without the
maintainer).

To verify on a GitHub-hosted run (the workflow's boot checks need the runner's desktop session): that `find-fxc.ps1`
finds the image's SDK (`C:\Program Files (x86)\Windows Kits\10\bin\10.0.<build>.0\x64\fxc.exe`;
the windows-2025 image ships the Windows 11 SDK 10.0.26100); that the boot check opens a window
on WARP and the PNG is the shell, not black (a black capture means no DWM composition in the
runner's session: then try `GPUI_DISABLE_DIRECT_COMPOSITION=1`, or keep the boot check to "first
frame, exit 0" with `KOMOREBI_EXIT_AFTER_FIRST_FRAME`); and what the UIA dump lists.

## Lints

`cargo xtask lint` (`xtask/src/lint.rs`, Skua's xtask lint adapted) scans
`crates/komorebi-gpui/src`:

- **Tokens:** `rgb(0x`, `rgba(0x`, `hsla(`, `px(<digit>`, `.size(<digit>` and a token times a
  number (`layout::GAP * 2.0`) fail outside the token layer (`palette.rs` until phase 1;
  `theme.rs`, `tokens.rs` and a `theme/` folder after it) and `test_support.rs`. Test modules (a
  `#[cfg(test)]` at the start of a line, to the end of the file) are not scanned. A line opts
  out with a trailing `// token-exempt: <why>`.
- **No timers or sleeps:** `thread::sleep`, `block_on`, `.timer(`, `Timer::after`, `tokio::`
  fail everywhere in the app, tests included, unless the line ends with
  `// timer-exempt: <why>` (a debounce or delay the design asks for, as Skua allows its two).

`xtask`'s own test `lint::tests::the_app_passes` runs the same scan, so `cargo test` catches a
finding too.
