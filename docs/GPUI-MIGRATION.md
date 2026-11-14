# Komorebi 1.0: the move to GPUI

The living plan for moving Komorebi's user interface from Slint to GPUI with GPUI Kit, on the
owner's Skua-style stack (vendored, patched gpui-pre 0.3.9, gpui-pre-windows 0.3.9 and GPUI Kit
0.8.0). Every phase updates this page: decisions, what is done, what remains.

Sources: the feasibility report (spike branch `spike/gpui`, `spike-gpui/REPORT.md`, S1 to S7),
the approved brand (komorebi-brand `final/BRAND.md`, `IMPLEMENTATION.md`, `REVIEW.md`,
`LAUNCH.md`, mockups), and Skua as the reference for GPUI on Windows (its `docs/cookbook/gpui.md`
is the API surface to use, `docs/gotchas.md`, `docs/DECISIONS.md`).

How to build: [GPUI-BUILD.md](GPUI-BUILD.md). How to test, and the one gate before each commit
(`cargo xtask check`): [GPUI-TESTING.md](GPUI-TESTING.md). Where each Slint-side test went:
[GPUI-TEST-PORT.md](GPUI-TEST-PORT.md).

**State: the cut-over is done** (see "Cut-over" near the end of this page). The Slint app is
deleted; the sections below are the history of the move and still say "the Slint app" where it
then existed.

## Owner decisions (binding)

- This release is **Komorebi 1.0**, on GPUI with GPUI Kit, built on the vendored stack as Skua is.
- **Brand:** the hanko K seal in persimmon #B3503A is the only square and the only colour, never
  in text; the dapple layout (pills, paper panes with 8 px gaps, solid sender spots for unread);
  Segoe UI Variable text; tagline "Read. Reply. Rest.". (After 1.0.0 the seal became neutral
  everywhere and Komorebi has no accent colour: [BRANDING.md](BRANDING.md) "Neutral brand".)
- **High contrast is not in 1.0.** No work for it, but the theme token layer (phase 1) is built
  so a high contrast palette is one more branch of the palette function, with no refactor.
- **Budgets:** launch under 200 ms and cached list under 600 ms stay; the exe ceiling is
  **21.5 MB** (raised from 20.5 MB), the one gate now that the Slint app is gone; higher idle
  memory and CPU than the Slint app are accepted for 1.0 (a later optimization release on the
  forks).
- Languages: English and European Portuguese (pt-PT, AO90, never Brazilian). No AI features, no
  telemetry. Pico Reforged is not used.
- The Slint app on `main` stayed the shipping 0.7.x line until the cut-over; the GPUI app was a
  new binary next to it, and the library (`src/lib.rs` and everything under it) is shared and
  keeps its tests green.
- **Cut-over decisions (2026-11-06):** the Slint app is deleted and nothing is ported back to
  it. The calendar UI is not in 1.0 but all calendar library code (`src/cal`, `src/dav/caldav`,
  the calendar tables and their tests) stays, compiling and tested: the calendar returns in
  1.1.0. The version is 1.0.0. 1.0 does not read 0.7's saved window place. The release exe is
  `komorebi.exe`, as in 0.7. The exe ceiling stays 21,500,000 bytes; the size cut is the next
  (optimization) run.

## Layout

| Path | What |
|---|---|
| `src/lib.rs` and below | The library both apps link: cache, mail, settings, theme, Win32 shell code. |
| `crates/komorebi-gpui` | The app (`komorebi.exe`, package `komorebi-gpui`): a workspace member (a library, `src/lib.rs`, shared by the binary, the gallery and the tests), its own `build.rs` linking `assets/komorebi.rc` (the icons and the manifest). |
| `xtask` | `cargo xtask check`, `lint` and `size` (phase 14, [GPUI-TESTING.md](GPUI-TESTING.md)). |
| `assets/` (`komorebi.ico`, `komorebi.rc`, `tray/`, `brand-src/`, `brand/`) | Komorebi 1.0's icons and brand assets ([BRANDING.md](BRANDING.md)). |
| `third_party/` | The vendored GPUI stack ([third_party/README.md](../third_party/README.md)), excluded from the workspace. |
| `crates/komorebi-i18n` | `Lang`, the current language, `tr!`/`trn!` over the gettext catalog (phase 15); used by the library and the GPUI app. |

A separate package (not `src/bin/`) because GPUI must not become a dependency of the library:
its build script, its resources and its features would reach it. The root package (the library,
no binary since the cut-over) stays the default member, so plain `cargo build` and `cargo test`
mean the library; the app is `-p komorebi-gpui`.

## Phases

From REPORT.md S7 (agent-days, one agent, reviews included).

| # | Phase and work | Agent-days | Status |
|---|---|---|---|
| 0 | Foundations: vendored stack in the app repo (third_party + [patch], as Skua), manifest feature patch, PerMonitorV2 in komorebi.manifest, fxc in CI, app bootstrap, DirectX pre-flight with error dialog | 3 | **Done** (below) |
| 1 | Shell and theme layer: token module replacing ui/theme.slint (WinUI 3 resources from theme.rs), Fluent widgets replacing ui/widgets.slint (3,870 lines; Kit components where they fit, custom divs elsewhere), title bar (chrome.rs logic onto window_control_area), text-scale and reduce-motion feeds | 10 | **Done** on `gpui/shell`, with the gaps listed (below) |
| 2 | Inbox and folder views: folder tree with folding and unread counts, lazy list (rows.rs model), multi-select, filters, search box, context menus, drag to folder, keyboard model | 9 | **Done** on `gpui/list`, merged in stage B, with the gaps listed (below) |
| 3 | Reader and tile element: port src/app/reader.rs (worker, byte-bounded tile cache with drop_image, selection overlay, copy, link hover and confirm, find, anchors, remote-image banner), invite card, attachments, a11y text nodes with bounds | 8 | **Done** on `gpui/reader`, merged in stage B, with the gaps listed (below) |
| 4 | Composer and painted rich editor (S5): input handler, View pieces, toolbar, recipient chips, signatures, attachments, controller rewire | 16 | **Done** on `gpui/composer`, merged in stage B, with the gaps listed (below) |
| 5 | Tabs and secondary windows (tabs.rs, windows.rs) | 5 | **Done** on `gpui/c-shell`, merged in stage C, with the gaps listed (below) |
| 6 | Settings (settings.rs, about, accounts, appearance, shortcuts page) | 6 | **Done** on `gpui/c-settings`, merged and mounted in stage C, with the gaps listed (below) |
| 7 | Onboarding (onboarding.slint 1,399 lines + onboarding.rs) | 4 | **Done** on `gpui/c-settings`, merged and mounted in stage C (first run and Add account), with the gaps listed (below) |
| 8 | Rules (rules.slint, src/app/rules.rs) | 3 | **Done** on `gpui/c-data`, merged in stage C and mounted in Settings, with the gaps listed (below) |
| 9 | Contacts | 3 | **Done** on `gpui/c-data`, merged in stage C and mounted beside the pane, with the gaps listed (below) |
| 10 | Calendar (calendar.slint 960 lines + edit dialogs) | 6 | **Dropped from 1.0 by owner decision (2026-11-06)**: no calendar UI in 1.0; the library code stays and the calendar returns in 1.1.0 |
| 11 | Notifications and tray (win_shell.rs and notify.rs are Win32 and stay; work is hide-to-tray with GPUI and toast activation) | 2 | **Done** on `gpui/c-shell`, merged in stage C, with the gaps listed (below); close to tray is untested by hand |
| 12 | Updater UI (updates.rs view only; update.rs stays) | 1.5 | **Done**: the bar and the What's new bar on `gpui/c-shell`, the Settings > About card in stage C |
| 13 | Accessibility: focus model, active descendant in lists, Invoke on caption buttons, names on every control, text nodes for reader and composer, Narrator fix pass | 6 | Partly done: caption Invoke and widget names and roles (phase 1); stage D added dialog regions, live notices, disabled states, tree levels, the reader and composer text nodes, access keys (Alt and a letter) and radio-group keys. The Narrator pass, roving tab stops and the remaining gaps are listed in the stage D sections |
| 14 | Tests: port the about 310 binary-side tests | 10 | **Harness done** (below); ports done with each lane through stage D (status per row in [GPUI-TEST-PORT.md](GPUI-TEST-PORT.md)); rows for the unbuilt pieces stay open |
| 15 | i18n bridge (REPORT.md S7) | 4 | **Done** (below; [GPUI-I18N.md](GPUI-I18N.md)) |
| 16 | Installer and manifest: komorebi.iss unchanged apart from the exe; manifest gains PerMonitorV2 (done in phase 0); check the VC runtime with crt-static | 1 | **Done**: manifest, and the 1.0 setup icon and wizard images are the installer's only branch (cut-over) |
| 17 | CI: fxc on runners, WARP for GPU tests, size gate, boot snapshot replaced, cargo deny | 2 | **Done** (below), to verify on the first CI run |
| 18 | Size and performance pass to the ceiling (S1 cuts 1 to 4), launch timing, memory baseline | 4 | Moved to 1.1 by owner decision (the 21.5 MB CI gate stays) |
| 19 | Parity bug-fix buffer (about 10 percent) | 10 | In progress: the stage D fix lanes are merged on main (stage D integration section); what is left is its Not done list |
| | Total | about 113 | |

Order: 0, 1, 15 and 14's harness first (everything depends on tokens, widgets, strings and
tests); then 2 and 3 (a readable inbox); then 4; then 5 to 12 in parallel lanes; 13 alongside
each view plus a final pass; 17 early; 18 and 19 last.

## Phase 0: foundations (done)

### What was done

1. **Vendored stack.** `third_party/gpui-pre` and `third_party/gpui-pre-windows` (0.3.9) and
   `third_party/gpui-component` (0.8.0), copied from the spike (which exported them from Skua
   7e0fdcf), each with its `SKUA-PATCH.md` and `LICENSE-APACHE`. Skua's patches: demand-driven
   frames and no frames while the display is off (gpui-pre-windows), view splice and cached
   window control areas (gpui-pre), Kit style hooks (gpui-component). Skua patches no other
   crates.io crate that Komorebi links (its fourth, alacritty_terminal, is Skua's terminal).
   Root `Cargo.toml`: `[patch.crates-io]` for the three, `[workspace.dependencies]`
   `gpui = { package = "gpui-pre", version = "=0.3.9" }` and `gpui-kit = "=0.8.0"`, `exclude =
   ["third_party"]`. `[profile.release]` unchanged: the GPUI build needs nothing added (the
   spike measured with this profile).
2. **Manifest.** The spike's `GPUI_MANIFEST_PROVIDED_BY_HOST` environment variable is now the
   Cargo feature `windows-manifest-provided-by-host` of the vendored gpui-pre (KOMOREBI PATCH;
   gpui-pre-platform needs no change, as the feature only skips the embed in gpui-pre's build
   script). `assets/komorebi.manifest` gained `dpiAware true/pm` and `dpiAwareness
   PerMonitorV2`; it already had what else GPUI's carried (asInvoker, Windows 10 and 11,
   Common Controls 6) plus the UTF-8 code page. `heapType SegmentHeap` is set (measured:
   optimization release, docs/PERF.md 8). Neither manifest declared long paths.
   `scripts/check-manifest.ps1` reads the RT_MANIFEST resources of a built exe and checks all
   of it; CI runs it on both exes.
3. **The GPUI app** (`crates/komorebi-gpui`): an empty Komorebi window with the app-drawn title
   bar (48 px, `window_control_area` Drag, Min, Max, Close as in Skua's titlebar.rs; caption
   buttons 46 px with accessible names in English or Portuguese, Restore while maximized, Close
   hover #C42B1C; title and glyphs dim while inactive), a paper pane 8 px in, light or dark from
   the theme setting and Windows; the window icon from `assets/komorebi.ico` (resource 1 of
   `assets/komorebi.rc`, which GPUI's window class loads); the saved place through the
   existing settings (`gpui_window` key, below); reduce motion from Windows' animation effects;
   the demand-driven frames patch active (it is in the vendored crate; nothing to switch on).
   Start-up reuses the Slint app's library code instead of copying it: the per-cache
   single-instance mutex, the hand-off of a `mailto:` link or a bring-forward to the running
   app, the window mark, the installer's app mutex and the wait for a restart moved from
   `src/main.rs` into the library as `komorebi::instance`, which both apps call (their tests
   moved with them).
4. **DirectX pre-flight** (`crates/komorebi-gpui/src/preflight.rs`): on a thread beside GPUI's
   platform start, the same Direct3D 11 device GPUI would create is created and dropped; on
   failure a native message box (English or European Portuguese, picked by the Windows display
   language through `komorebi::i18n::startup_lang`, the reason under the text) and exit code 3
   instead of the silent abort. `KOMOREBI_GPUI_FORCE_WARP` (a KOMOREBI PATCH in
   gpui-pre-windows, read by the pre-flight too) draws with WARP; `KOMOREBI_GPUI_PREFLIGHT_FAIL`
   shows the dialog on purpose; GPUI's `GPUI_DISABLE_DIRECT_COMPOSITION` is documented.
5. **fxc.exe discovery** (KOMOREBI PATCH in gpui-pre-windows' build script): `GPUI_FXC_PATH`,
   then `PATH`, then the newest Windows SDK; a wrong `GPUI_FXC_PATH` or a miss is a
   `cargo::error` saying what was tried and how to fix it.
6. **Docs:** this page, [GPUI-BUILD.md](GPUI-BUILD.md), [third_party/README.md](../third_party/README.md),
   the `KOMOREBI-PATCH.md` files; the 21.5 MB ceiling in README, CONTRIBUTING, BUILDING, PERF,
   SCOPE, RELEASING and TODO; NOTICE gains GPUI and GPUI Kit (Apache-2.0) and the Lucide icons
   (ISC, with the Feather MIT part), with their notice texts.
7. **Checks and CI:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -D warnings`
   (the vendored crates are outside the workspace, so not linted), `cargo test -p
   komorebi-gpui`, `scripts/check-pins.ps1` extended to the GPUI app's graph, `cargo deny
   --workspace` (before, `cargo deny` without `--workspace` checked only the root package). CI
   builds both apps in release (so fxc.exe is exercised), checks each exe's size against
   21,500,000 bytes and each manifest.

### Decisions

- **Separate settings key for the window place.** The Slint app saves `window` as physical px
  of its outer position and inner size; GPUI reports logical px of the client area. The GPUI app
  saves `gpui_window` (the same `x y w h max` format, logical px, `Settings::gpui_window`) and
  restores it when it still meets a connected display (the cookbook's check), else 1280 x 800
  centred. At the cut-over, the 1.0 app reads the 0.7 `window` key once and converts it.
- **Exact pins for the stack's own copies:** `windows =0.62.2` and `raw-window-handle =0.6.2`
  in komorebi-gpui are the copies gpui-pre-windows and GPUI link (no new code). The DirectX
  probe uses `windows` (COM interfaces) rather than the library's `windows-sys`.
- **One API beyond the cookbook:** `raw_window_handle::HasWindowHandle` on `gpui::Window`, to
  get the HWND for the window mark (`komorebi::instance::mark_window`, now `unsafe` with its
  safety contract). Everything else the window does is in Skua's cookbook (cookbook_window,
  cookbook_window_state, cookbook_async, cookbook_a11y).
- **Known second copies in the GPUI app's graph** (check-pins lists them as known, not
  failures): tiny-skia 0.11 with png 0.17 (resvg 0.45 in gpui-component and resvg 0.46 in
  gpui-pre), gif 0.13, roxmltree 0.20 and zune-jpeg 0.4 with zune-core 0.4 (resvg 0.45),
  windows 0.58 (gpui-base). They are S1 size cut 2, phase 18.
- **cargo deny advisories:** four unmaintained (not vulnerable) crates only the GPUI stack links
  are ignored with reasons in `deny.toml` (instant, paste, ttf-parser, rustybuzz); no release
  of the pinned stack avoids them. Look again when the pins move.
- **Start-up errors** (a locked or newer cache) are not dialogs yet in the GPUI app: it logs to
  stderr and opens the empty window without the cache. They come with the shell (phase 1).
- **Crash log:** the Slint app's panic hook (`src/crash.rs`) is binary-side; the GPUI app has
  none yet (phase 1 moves it into the library or copies the hook's one-line contract).

### Measurements

The baseline every later phase is compared with. Machine: i7-13700KF, Radeon RX 9070 XT,
2560 x 1440 at 180 Hz, 100 percent scaling, Windows 11 Pro 26300, rustc 1.95.0; other agents'
cargo jobs were running on the machine (load 7 percent at the start of the batch).

**Size** (Komorebi's release profile, stripped, `cargo build --release --locked --workspace`,
13 min with warm dependencies):

| Exe | Bytes |
|---|---|
| komorebi.exe (Slint 0.7, this tree) | 20,023,808 |
| komorebi-gpui.exe (phase 0: empty window, title bar, library start-up code) | 10,667,520 |

Ceiling 21,500,000 bytes. The spike's projection for the full GPUI app is 19.8 to 21.3 MB
before the size cuts (REPORT.md S1); every phase records its size here.

**First frame** (Komorebi's clock: process start to the first event-loop pass after the first
frame was drawn and presented; `scripts/gpui-bench.ps1 -Mode timing -Runs 9 -Config
slint,gpui`, 9 interleaved runs after one warm-up each, each app on its own copy of a seeded
throwaway cache: 2000 messages, the 14 `.eml` files of `tests/fixtures/html`, `--accounts`;
Slint with `KOMOREBI_SNAPSHOT`, GPUI with `KOMOREBI_EXIT_AFTER_FIRST_FRAME`):

| App | In-app median / min / max (ms) | Wall median / min / max (ms) |
|---|---|---|
| Slint 0.7 | 33.0 / 31.7 / 35.9 | 42.5 / 40.6 / 53.7 |
| GPUI phase 0 (empty window) | 94.3 / 89.6 / 106.2 | 105.9 / 100.0 / 118.4 |

GPUI phase 0 breakdown (3 runs, release, `KOMOREBI_TIMING`): platform created 33.9 to 35.1 ms,
DirectX probed 45.6 to 47.4 (the pre-flight runs beside the platform start and finishes about
11 to 13 ms after it: its cost on the launch path), cache loaded at the same moment (its own
thread), window created 90.4 to 92.4 (GPUI's own device and vsync set-up in `run` is most of
the gap), first frame 90.5 to 92.5. Well under the 200 ms budget; the fixed DirectX cost is the
lever the optimization release targets (REPORT.md backlog 3).

**Run-time checks of phase 0** (debug build, offline, throwaway cache): first frame and exit 0;
the place saved as `gpui_window = 640 296 1280 800 0`; WARP (`KOMOREBI_GPUI_FORCE_WARP`) draws
and exits 0; `KOMOREBI_GPUI_PREFLIGHT_FAIL` shows the message box in English and, with
`KOMOREBI_LANG=pt-PT`, in European Portuguese (read back through UI Automation), exit 3; a
second launch on the same cache exits 0 in 20 ms and leaves the first running, whose window
carries the `Komorebi.Instance` mark. Both release exes pass `scripts/check-manifest.ps1`. A
wrong `GPUI_FXC_PATH` stops a release build with the new error; without it, fxc.exe was found in
the Windows SDK (it is not on `PATH` on this machine).

### Not done in phase 0, or left for later phases

- A GPUI boot check in CI ("launch on WARP, first frame, exit 0" plus a UIA dump assert,
  REPORT.md S7) is phase 17 (done on gpui/harness, below; its first Blacksmith run still has to
  prove that a GPUI window opens in the runner's session).
- The title bar shows the word "Komorebi", not yet the ink mark; the caption glyphs are Kit's
  Lucide icons, not yet the Fluent shapes (phase 1 icons). Caption buttons have no UIA Invoke
  (phase 13). *Done in phase 1.*
- Text size (Windows' text scale) is not fed yet (phase 1). *Done in phase 1.*
- Known flaky test, not caused by phase 0: the Slint app's
  `app::composer::tests::opened_messages_are_marked_read_after_the_delay` depends on a 1 s timer
  and fails in a full parallel `cargo test` on a loaded machine. It failed the same way at the
  base commit d8b9461; it passes alone and in the full Slint suite with `--test-threads=4`
  (307 passed, 1 ignored).

## Phase 14 harness and phase 17 CI (done on gpui/harness)

### What was done

1. **Test harness** (`crates/komorebi-gpui/src/test_support.rs`, [GPUI-TESTING.md](GPUI-TESTING.md)):
   an app context set up as `main` sets it up (Kit, key bindings, reduce motion), headless Kit
   windows at 1280 x 800, the main window with any palette and cache (the language through
   `set_lang`, crate::i18n, since the stage A merge), a throwaway
   cache (`TempCache`) seeded by the library (`seeded`), and queries by accessible name and role
   (`names`, `assert_named`, `click_named`, `press`, `type_text`, `dispatch`). The harness tests
   itself (a Kit `Input` and `Button` inside the window's key context: typing, a click by name,
   a window key that wins over the focused field).
2. **Two library moves the harness needed:** the tabs model (`src/app/tabs.rs` to
   `komorebi::tabs`, with `DraftId`) so both apps drive one model, and the seed data
   (`examples/seed.rs` to `komorebi::seed`, the same data in the same order) so the tests seed
   what the example seeds. The Slint app now calls both; its tests are green (301 passed, 1
   ignored, after its 6 tabs tests moved to the library).
3. **Ported, as proof** ([GPUI-TEST-PORT.md](GPUI-TEST-PORT.md) "Status"): the model half of
   `close_tab_activates_left_neighbour_and_ctrl_shift_t_reopens` to a plain `#[test]` in
   `komorebi::tabs`; `ctrl_tab_moves_between_tabs_even_from_a_text_field` to a
   `#[gpui_kit::test]` on the main window (the tab keys are now actions in the window's
   `MainWindow` key context: Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+PageDown/PageUp, Ctrl+1, Ctrl+9;
   `crates/komorebi-gpui/src/tabs.rs`); `ink_counts_pixels_unlike_the_background` to the
   snapshot module. The caption buttons are registered for Kit queries, and a test reads their
   names in European Portuguese.
4. **Inventory** of all 315 Slint-side tests (308 in the binary, 7 that read Slint sources)
   with kind, target and owner phase: [GPUI-TEST-PORT.md](GPUI-TEST-PORT.md).
5. **Pixel snapshots** (`crates/komorebi-gpui/src/snapshot.rs`): `KOMOREBI_SNAPSHOT` with the
   Slint app's switches (`KOMOREBI_THEME`, `KOMOREBI_SIZE`, `KOMOREBI_LANG`,
   `KOMOREBI_SNAPSHOT_ACTION=shell`, `KOMOREBI_SNAPSHOT_CHECK`), the safety rules in code (exit
   2, nothing opened, without `KOMOREBI_OFFLINE` and a throwaway `KOMOREBI_DB`), PNG or BMP.
6. **Lints** (`cargo xtask lint`): token literals outside the token layer, and no timers or
   sleeps anywhere in the app (tests included).
7. **One gate** (`cargo xtask check`, `xtask/`, standard library only): fmt, clippy, the GPUI
   app's, the library's and xtask's tests, the lints, the GPUI exe's size; `cargo xtask size`
   checks each exe against its own ceiling.
8. **CI** (`ci.yml`, `release.yml`): fxc.exe located (`scripts/find-fxc.ps1`, sets
   `GPUI_FXC_PATH`); `cargo check -p komorebi-gpui --all-targets`; the lints; the GPUI app's and
   xtask's tests; the size per exe (komorebi-gpui.exe 21,500,000, komorebi.exe 20,500,000 until
   the cut-over); both manifests; a GPUI boot check on WARP (`scripts/gpui-boot-check.ps1`: first
   frame captured, checked, exit 0) as a gate; a UI Automation dump with asserts
   (`scripts/gpui-uia-check.ps1`) as an informational step (`continue-on-error`); every
   existing step kept; `cargo deny --workspace` covers the new graph (xtask adds no crate, the
   GPUI app adds the library's own `png`).

### Decisions

- **Snapshots capture a real window, not an offscreen render.** GPUI 0.3.9 has no headless
  renderer in the cookbook's API, and the test platform draws nothing. The capture is Skua's:
  `PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)` from a background thread. The app's first
  frame (its first present) reaches DWM a composition later, and later still on WARP: the
  capture waits for DWM's next composition (`DwmFlush`, no timer) while the frame is blank, at
  most 30 times. On WARP the first capture was black every time without it, and one composition
  later with it.
- **Pixels are not asserted in `cargo test`.** View and window tests assert state, names and
  roles; the snapshot is a release-exe check (CI's boot check), and golden comparisons are a
  later choice per view (REPORT.md S7).
- **The GPUI app enforces the snapshot safety in code** (the Slint app leaves it to its callers).
- **`Window::dispatch_action`** is the one API the harness uses beyond the cookbook (actions
  without a key, as a menu sends them).
- **The token lint skips test modules and `test_support.rs`** (tests may name sizes), and treats
  `palette.rs`, `theme.rs`, `tokens.rs` and a `theme/` folder as the token layer, so phase 1 can
  put its tokens in any of them. The no-timers lint has no such exception.
- **Size gates per exe until the cut-over**: the Slint app keeps 20,500,000 bytes (phase 0 had
  set 21.5 MB for both); the GPUI app has the owner's 21,500,000.
- **`cargo xtask check` runs the library's tests** (shared code) but not the Slint app's by
  default (`--all` adds them): those take minutes and do not change with GPUI-only work.

### Checks and measurements

On this machine (as in phase 0), CARGO_TARGET_DIR `F:\dev-cache\cargo-target-gpui-harness`:

- `cargo xtask check`: fmt, clippy, 18 GPUI tests, 679 library tests (1 ignored), 9 xtask tests,
  both lints: ok. `cargo test --locked`: 679 library, 301 Slint app (1 ignored), the
  integration tests: ok.
- Snapshot, debug exe, offline, throwaway caches in the session scratchpad: the GPU at 1280 x
  800 light (passed, first try); WARP at 1280 x 800 light, at 900 x 700 dark in European
  Portuguese (BMP), and WARP with `GPUI_DISABLE_DIRECT_COMPOSITION=1` (passed, each after one
  more DWM composition). Refused with exit 2 and no cache file made: no `KOMOREBI_OFFLINE`; an
  unknown `KOMOREBI_SNAPSHOT_ACTION`.
- `scripts/gpui-uia-check.ps1 -Warp`: Window 'Komorebi', Buttons 'Minimize', 'Maximize',
  'Close' found; closed through UI Automation, exit 0.
- `scripts/find-fxc.ps1`: `C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\fxc.exe`
  (10.0.26100.8249). Both workflow files parse (PyYAML 6.0.2).
- `scripts/check-pins.ps1` and `cargo deny --workspace --locked check licenses advisories`: ok.
- **Not measured: the release build.** `cargo build --release --locked --workspace` stopped
  after 7 minutes with "os error 112" (not enough space on the disk): drive F: was full (0 GB
  free, about 95 GB in the target folders of the GPUI tasks running in parallel). So the size of
  the new `komorebi-gpui.exe` (the snapshot code and the `png` encoder are new in it), `cargo
  xtask size`, the release manifests and the boot and UIA checks on a release exe are still to
  run: `cargo xtask check --release`, `cargo xtask size`, `scripts/check-manifest.ps1` on both
  exes and `scripts/gpui-boot-check.ps1 -Warp` on the release exe, once there is room. (Done
  for the GPUI exe at the stage A integration, below; the Slint exe was not rebuilt there.)

### To verify on the first CI run

That `find-fxc.ps1` finds the image's SDK; that the WARP boot check gets a composed window in
the runner's session (a black capture means no DWM composition there: try
`GPUI_DISABLE_DIRECT_COMPOSITION=1`, or fall back to "first frame, exit 0" with
`KOMOREBI_EXIT_AFTER_FIRST_FRAME`); what the UIA dump lists; the release job's run time with
both apps.

## Phase 15: the i18n bridge (done)

The developer guide for the view phases: [GPUI-I18N.md](GPUI-I18N.md).

### What was done

1. **`crates/komorebi-i18n`** (workspace member, no dependencies): `Lang` moved here from the
   library (`komorebi::i18n::Lang` re-exports it, so nothing else changed); the current
   language as a process-wide value with a per-thread override for tests (`with_lang`); the
   build script parses every `translations/<tag>/LC_MESSAGES/komorebi.po` with `po.rs` into a
   static table sorted by (msgctxt, msgid), with the header's plural rule (fuzzy and empty
   entries left out, so English shows); `tr!(ctx => "text {0}", args)` and `trn!(one, many, n,
   args)` with and without a context fill Slint's placeholders (`{}`, `{0}`, `{n}`, `{{`).
   `komorebi::i18n::select` (the Slint switch) also sets the value, so library code sees the
   language in both apps.
2. **The catalog test** (`tests/i18n_catalog.rs`) reads `.po` files with the same `po.rs`; a
   Rust extractor (a small lexer: comments, raw and escaped strings, char literals, nested
   calls, `#[cfg(test)]` items skipped) reads `tr!`/`trn!` from `src/` and
   `crates/komorebi-gpui/src`; both extractors feed one template (Slint first, then Rust).
   Every earlier rule now covers the Rust messages too; new: each call's argument count must
   match its placeholders, and shown literals in the GPUI app outside `tr!` fail (the shared
   allowlist). `KOMOREBI_UPDATE_POT=1 cargo test --test i18n_catalog template` regenerates.
3. **The GPUI app:** `src/i18n.rs` keeps the language in the GPUI global `CurrentLang` and in
   the macros' value; `set_language` sets both, notifies observers and calls
   `cx.refresh_windows()` (a refresh re-renders cached views too), so the live switch works as
   in the Slint app. Start-up uses the Windows display language for the pre-flight dialog, then
   the saved choice (or `KOMOREBI_LANG`) once the cache is read (phase 0 used only the Windows
   language). The OS window title follows a switch through an observer.
4. **Sentences:** `src/text.rs` is the GPUI counterpart of `ui/tr.slint`, `src/tr.rs` and
   `src/app/text.rs`: views keep typed values and call it in `render`. Ported as proof:
   `window_title`, `start_failed`, `tray_open`, `tray_tooltip` and `toast_burst` (plurals),
   `sent` (context), and the whole `say(&Say)` mapping (27 sentences), each the same catalog
   entry as its `Tr` function (the template diff added only `#:` file names). Tests: the Slint
   render test's English list word for word, every `Say` translated with its names and
   numbers kept, exact pt-PT for the others. The caption names (`window.rs`) and the DirectX
   pre-flight text (`preflight.rs`, the one new message, translated) use `tr!` now.
5. **Format helpers** confirmed toolkit-independent (dates, numbers, sizes take a `Lang`; only
   chrono and Win32), with two caveats written down in GPUI-I18N.md: they are in the Slint
   binary (`src/format.rs`), so phase 2 moves them into the library; `outbox_status`,
   `send_sentence` (`t!`) and `avatar_color` (`slint::Color`) are rewritten, not moved.
6. **Docs and CI:** GPUI-I18N.md (new), TRANSLATIONS.md, CONTRIBUTING.md, GPUI-BUILD.md; CI
   runs `cargo test -p komorebi-gpui -p komorebi-i18n --locked`.

### Decisions

- **The macros read a plain value, not the GPUI global.** `tr!` needs no `cx`, works in the
  library, on worker threads and before GPUI starts (the pre-flight dialog). The GPUI global is
  the notification point and always set together with the value (`crate::i18n` is the only
  writer). Under `cfg(test)` the GPUI app writes the test thread's override instead, so
  parallel `#[gpui::test]`s never share a language.
- **One `.po` reader** (`crates/komorebi-i18n/po.rs`) for the build script and the test, so the
  table and the rules cannot disagree on parsing. The build does not enforce the shipping rules
  (fuzzy, completeness): the test does, as before.
- **Every pt-PT entry is in the table**, also those only the Slint app uses so far (1,265
  entries, about 140 KB of generated source): no second filter to keep in step; all of them are
  used at the cut-over.
- **Shown-literal check for Rust is a heuristic** (sinks plus sentence shape; GPUI-I18N.md);
  Slint's check (every literal) stays as it was.
- **GPUI APIs beyond the cookbook** (Skua's cookbook has no globals): `gpui::Global`,
  `App::set_global`, `App::try_global`, `App::observe_global`, `Context::observe_global_in`,
  `App::refresh_windows` and `Window::set_window_title`, all public gpui-pre 0.3.9 APIs used
  as documented; the report's S7 design calls for a global and a refresh.
- **GPUI Kit stays on its English locale.** Kit's built-in strings come from its own
  `rust-i18n` table (English and Chinese, no pt-PT); see GPUI-I18N.md "Known gaps" for the
  choice the owner makes when a view needs one.

### Checks

`cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`;
`cargo test -p komorebi-i18n` (8 unit tests and the doc test); `cargo test --test
i18n_catalog` (9 tests); `cargo test -p komorebi-gpui` (13 tests, among them a switch
re-rendering a view in Portuguese and observers hearing it); `cargo test --locked` for the
library and the Slint app: exit 0 (library 670 passed, 1 ignored; Slint app 307 passed, 1
ignored; every integration test and doc test passed; the flaky composer timer test of phase 0
passed this time).

Size: `cargo build --release --locked -p komorebi-gpui` gives komorebi-gpui.exe of
10,850,304 bytes, 182,784 more than phase 0 (10,667,520): the pt-PT table with all 1,265
entries and the lookup. Ceiling 21,500,000. Run-time (debug, offline, throwaway cache,
`KOMOREBI_EXIT_AFTER_FIRST_FRAME`): first frame and exit 0 with `KOMOREBI_LANG=pt-PT` and
`en`.

### Not done in phase 15

- Kit's own strings in pt-PT (above).
- The remaining `ui/tr.slint` sentences: each view phase ports the ones it shows.
- Moving the format helpers into the library (phase 2).
- At the cut-over: the Slint extractor, `ui/tr.slint`, `src/tr.rs` and the Slint call in
  `select` go.

## Brand assets (done)

The new icon and brand files, from the approved brand's logo set (komorebi-brand `final/logo`).
Every file, its source and its use: [BRANDING.md](BRANDING.md).

### What was done

1. **App icon** `assets/brand-1.0/komorebi.ico`: 16, 20, 24, 32, 40, 48, 64, 128 (32-bit BMP)
   and 256 (PNG); 16 and 24 are the hand-drawn pixel files, the rest the vector icon with the
   seal's edge snapped to whole pixels. Resource 1 of `komorebi-gpui.exe` through the new
   `assets/brand-1.0/komorebi.rc` (which takes the manifest from `assets/komorebi.manifest`).
2. **Tray icons** `assets/brand-1.0/tray/tray-{light,dark,unread}.ico` (16, 20, 24, 32),
   resources 101 to 103. `src/win_shell.rs` picks by `SystemUsesLightTheme` (ink seal on a light
   taskbar, white on a dark one) and turns persimmon while unread mail waits (new
   `Shell::set_unread`); it re-reads the theme on `WM_SETTINGCHANGE` "ImmersiveColorSet" and
   loads the icon at the system small icon size. An exe without resource 101 (the Slint app)
   keeps icon 1 in every state: the 0.7.x behaviour is unchanged.
3. **Notifications:** they are the tray icon's `NIF_INFO` notifications, attributed by Windows
   to the exe (its icon 1 is now the seal in the GPUI app); with the brand icons the toast also
   carries icon 1 at the large size as its image (`NIIF_USER | NIIF_LARGE_ICON`). No
   AppUserModelID is used.
4. **Installer:** `installer/wizard-image.bmp` (164 x 314), `wizard-small.bmp` (55 x 58) and 2x
   copies, paper ground with the seal and the wordmark; `komorebi.iss` uses them and the new
   icon only with `/DBrand10` (`scripts/build-installer.ps1 -Brand10`), so the 0.7.x setup is
   unchanged. Both variants compile with ISCC.
5. **GPUI view assets** `assets/brand/`: minified `mark.svg`, `mark-mono.svg`, `fall.svg`,
   `wordmark.svg`, `wordmark-dark.svg`, `logo.svg`, `logo-dark.svg`, PNGs at 1x and 2x, and
   `index.json` (about 55 KB in all). Not wired into a view yet (phases 1, 6 and 7).
6. **Repository images:** `.github/assets/logo.png` and `logo-light.png` (the new lockup, light
   and dark) and both social previews; screenshots untouched. README's logo alt text updated.
7. **Generator and checks:** `scripts/brand-assets.js` rebuilds everything from
   `assets/brand-1.0/src`; `crates/komorebi-gpui/tests/brand_assets.rs` checks the ICO sizes
   and formats, loads every size through `LoadImageW`, and keeps `assets/brand` under 150 KB;
   `win_shell` unit tests cover the tray selection logic, the theme value, the settings-change
   area and the rc's resource ids.

### Decisions

- **Separate files until the cut-over.** The Slint app keeps `assets/komorebi.ico` and
  `assets/komorebi.rc`; only the GPUI app links `assets/brand-1.0/komorebi.rc`. At the cut-over
  `assets/brand-1.0/` moves up and replaces them, and `Brand10` becomes the installer's only
  branch.
- **Brand detection by resource, not by a flag.** The shared shell code switches to the brand
  tray icons when the exe has resource 101, so `Shell::new` keeps its signature and the Slint app
  needs no change.
- **Size:** the icon resources grow from 11 KB (the old ICO) to about 142 KB (the new ICO and the
  three tray ICOs), so `komorebi-gpui.exe` grows by about 131 KB. Computed from the files, not
  measured on a release build.

### Not done

- The GPUI app does not create the tray (`Shell`) yet, so the tray and toast states were checked
  by unit tests and by loading the resources from the built exe, not on a taskbar (phase 11).
- The views do not use `assets/brand` yet (title bar ink mark in phase 1, welcome in phase 7,
  About in phase 6).
- The release workflow still builds the 0.7.x setup; it passes `-Brand10` from the cut-over.

## Phase 1: shell, theme and widgets (done, with gaps)

Branch `gpui/shell` (from `gpui/main` fdd12cc). All in `crates/komorebi-gpui`, now a library
(`src/lib.rs`: theme, icons, widgets, title bar, window, pre-flight) with the binary
(`src/main.rs`) on top, so the gallery and the tests share the modules and unused widgets are not
dead code.

### What was done

1. **Token layer** (`src/theme/`). `Tokens` holds the semantic colours (`Palette`: canvas,
   paper, inset, wash, fall, line, spot, ink, ink-2, ink-3, on-ink, seal, segment-on,
   pill-on-inset, close-hover, on-close, overlay, clear), the type ramp (`TypeRamp` of
   `TextToken`s: family, size, line height, weight), Windows' text size and the reduce-motion
   flag. One function, `theme::tokens(&ThemeInput)`, builds it from the theme setting and
   `komorebi::theme::SystemTheme`. Colours are BRAND.md and IMPLEMENTATION.md section 1 exactly;
   geometry and motion tokens are consts (`metrics.rs`: `radius`, `space` on the 4 px grid,
   `layout`, `motion` with the enter easing and the loader cycle) from IMPLEMENTATION.md and
   the mockups' CSS. `theme::install` is the one writer: the tokens as a GPUI global
   (`cx.tokens()`), Kit's theme (`theme/kit.rs`: one `Theme::change`, one `Theme::update`
   assigning every `ThemeColor` field from the table in that file, as Skua does; a test proves
   no field is left at a Kit default), `cx.set_reduce_motion`, the windows' DWM frame, and
   `refresh_windows`; an unchanged input does nothing. `set_choice` (the Settings override,
   live) and `system_changed` are its two entry points.
2. **Token lint** (`tests/token_lint.rs`, runs with `cargo test -p komorebi-gpui`): no
   `rgb(`, `rgba(0x`, `hsla(`, `Rgba {`, six or eight digit hex literals, `px(<digit>`,
   `.size(<digit>`, `opacity(<digit>`, `from_millis(<digit>` or `rems(<digit>` outside
   `src/theme/`, test modules excepted; `// token-exempt: <reason>` opts a line out (none is
   used).
3. **Window shell.** `src/titlebar.rs`: 48 px on canvas, the ink seal (16 px) and the name, a
   centre slot (the search pill), the 46 px caption buttons with Fluent glyphs. Each caption
   button is a named AccessKit button that also answers the Click action (UIA Invoke) by
   posting the window the system caption's `WM_SYSCOMMAND` (SC_MINIMIZE, SC_MAXIMIZE or
   SC_RESTORE, SC_CLOSE), so Narrator can press them and Close goes through the normal close
   path. Checked live through UI Automation (Invoke Maximize, Restore, Minimize, Close: zoomed,
   restored, iconic, exit 0). Live feeds (`src/theme/feeds.rs`): Windows' light and dark from
   `observe_window_appearance`; the text size from `komorebi::theme::watch_text_scale` (a
   registry wait on its own thread, handed to the UI thread over a futures channel); animation
   effects re-read when a window becomes active (Windows sends GPUI no message for it); the DWM
   immersive dark frame follows the app's scheme when the setting forces one; the window is
   opaque (no Mica). The window place is saved as in phase 0.
4. **Widgets** (`src/widgets/`, each file under 400 lines with headless tests of behaviour,
   names and roles): `Button` (pill: primary ink, secondary wash, subtle, on-inset, inset,
   destructive in ink; 32, 36 and 40 px; leading icon, trailing chevron), `IconButton` (round
   36 or 32 px, tooltip, name, toggled state), the brand tooltip (ink pill, 500 ms, the text
   also as the target's AccessKit description), `FilterPills` (All, Unread, Important with the
   4 px spot), `Chip` (recipient), `SenderSpot` (initials; unread solid ink; read on wash or
   paper; multi-select round check as a named checkbox), `Toggle` and round `Checkbox`,
   `TextField` (Kit `Input` with `appearance(false)` in a pill, the composer's underline row or
   the title bar's search pill; `navigation` hands Up, Down, Page Up, Page Down, Home, End,
   Enter, Ctrl+Enter and Escape to the owner through `capture_action`), `Dropdown` (own pill +
   brand menu, name "<title>: <choice>"), menus (`menu::brand` over Kit `PopupMenu`),
   `dialog::confirm` (Kit `Dialog`, brand pills, Enter and Escape), `Notice` (InfoBar: inset,
   glyph, words; Status or Alert), the Sent notice `Toast` (ink pill with the persimmon seal,
   fade 250 ms, a 2 s timer hold, fade out; a newer one cancels the older timer), `Loader`
   (three spots stepped at 8 fps on its own entity only while active and motion is allowed),
   `ProgressRing` (Kit's circle in ink), `Pane` (paper, 12 px), `ScrollArea` (Kit scrollbar),
   `ListRow` (16 px, wash and the 4 x 20 marker when selected, position in set),
   `TabStrip` (pill tabs, 4 px spot, closable) and `Segmented`, `Kbd` and `Badge`. Every
   focusable widget draws the brand focus ring (2 px ink, 2 px paper gap) on keyboard focus only.
5. **Gallery and shots** (`src/gallery/`, the `gallery` feature, development only):
   `KOMOREBI_GALLERY=light|dark` shows every widget; `KOMOREBI_GALLERY_SHOT=<png>` renders the
   frame to a PNG with `Window::render_to_image` and quits; `KOMOREBI_GALLERY_OPEN=dialog`
   opens the confirm dialog. Compared with the mockups (`inbox`, `settings`, `compose`, `sent`
   in light and dark) and fixed: caption glyphs were not drawn, Kit's input text was smaller
   than the body step and indented, notices did not wrap, the segmented control stretched, the
   dialog had Kit's close glyph. `scripts/gpui-shot.ps1` captures a running window with
   `PrintWindow` on an offline throwaway cache (any build).
6. **Icons** (`src/icons.rs`): 43 Fluent System Icons (MIT, `@fluentui/svg-icons` 1.1.292,
   regular, 16 px; 20 px for Power, which has no 16 px regular) and the seal from komorebi-brand
   `logo/mark-mono.svg`, `include_bytes!`d and served by `AppAssets` ahead of Kit's default
   `Assets` (not `AllAssets`). NOTICE lists them.

### Decisions

- **Icons are bundled SVGs, not the Segoe Fluent Icons font.** BRAND.md and IMPLEMENTATION.md
  name Fluent System Icons as SVG: the same shapes as Segoe Fluent Icons, which Windows 10 does
  not have. With the SVGs there is no fallback to define: Windows 10 and 11 draw the same
  glyphs (the Slint app fell back to Segoe MDL2 Assets on Windows 10). Lucide stays only inside
  Kit's own components (Kit's default `Assets`, about 100 icons). Cost: about 25 KB of SVG.
- **Type:** Segoe UI Variable Text below 20 px and Display from 20 px on Windows 11 (build
  22000 and up), Segoe UI on Windows 10; no font bundled. Windows' text size scales each step
  with WinUI's formula (`komorebi::theme::scaled`: small text grows most), line heights in
  proportion; Kit's font size follows the body step. Controls keep their heights at larger
  text for now (see gaps).
- **Modules, not crates.** IMPLEMENTATION.md suggests `komorebi-theme` and `komorebi-ui`
  crates as in Skua; the name `komorebi-ui` is the Slint UI crate until the cut-over, and one
  library crate builds faster here. The split can come with the cut-over.
- **The focus ring is two crisp spread shadows** (paper 2 px, then ink to 4 px, no blur) set
  through GPUI's `focus_visible` style, rather than the critic's border wrapper: a wrapper
  cannot know that its child has keyboard focus, and `focus_visible` draws only for keyboard
  focus. A zero-blur shadow takes GPUI's solid quad path, one quad each. Kit's own translucent
  ring is off (`Theme::focus_ring = false`).
- **Menus have the 1 px `line` edge.** A paper menu over a paper pane disappears without an
  edge or a shadow, and the brand allows no shadow; BRAND.md's one edge colour is used. Owner
  review: the brand says "at most one 1 px edge in a view".
- **Tooltips are ink pills** (inverted), for the same reason.
- **Destructive is ink.** No red in the brand; the word and the confirm dialog carry the
  meaning. `Variant::Destructive` exists so a later look needs no call-site change.
- **Caption buttons keep `window_control_area`** (Snap Layouts) and add the AccessKit Click
  listener; the command is posted, not called, so it runs as the system caption's would.
- **The gallery uses GPUI's `test-support`** for `render_to_image` (feature `gallery` only).
- **Pinned additions** (copies already in the graph, `check-pins.ps1` passes): `futures
  =0.3.34`, `image =0.25.10` (gallery only, png), `serde_json =1.0.151` (dev, the Kit theme
  test).

### Gotchas (Kit and GPUI looks the widgets work around)

- Kit `Button` draws its own radius, padding, focus ring and Lucide tooltip: buttons are GPUI
  divs (GPUI gives a focusable div with a click listener Enter, Space and AccessKit Click).
- Kit popovers and menus carry a shadow and a ring: `PopupMenu::menu_style` (vendored hook)
  replaces them per menu; `menu::brand` must start every menu builder.
- Kit `Input` sizes its text and padding by its `Size` (`text_sm`, `input_px`), not the theme:
  `TextField` refines them after Kit (`typo`, `px(0)`). Its placeholder is Kit's
  `muted_foreground` (ink-2); the mockups use ink-3 (open, small).
- Kit `Dialog` draws a close glyph without a tooltip: `close_button(false)`.
- Kit `Kbd` draws a keycap box and `Badge` a coloured pill: both are plain text here.
- Kit `Select`'s trigger is a bordered box: `Dropdown` is our own pill with a Kit menu.
- Kit `Notification` is not used for the Sent notice (its card, width, shadow and spring do not
  fit); `Toast` is our own entity, placed by the host view.
- A GPUI `svg()` paints in its own `text_color`, which is not inherited: set it on the svg, and
  reach hover through `group` / `group_hover`.
- This gpui-pre has no `Window::activate_a11y_for_test`, so headless tests cannot read the
  AccessKit actions; the caption Invoke was checked live through UI Automation.
- `PrintWindow` returns a black frame while GPUI draws nothing (the display off, a remote
  session not shown): use the gallery's `render_to_image` shots.
- `Window::render_to_image` is behind `test-support` in gpui-pre and gpui-pre-windows.

### Measurements

- Tests: `cargo test -p komorebi-gpui` 43 unit and headless UI tests plus the 2 token lint
  tests, all passing; `cargo clippy -p komorebi-gpui --all-targets -D warnings` clean with and
  without `--features gallery`; `scripts/check-pins.ps1` passes. The library and the Slint app
  are unchanged in this phase.
- Gallery shots (debug build, 1280 x 900 at 100 percent display scale): light, dark, both with
  the confirm dialog, and light at 150 and 225 percent text size. At larger text sizes the text
  grows as WinUI's does; fixed heights (list rows, spots, settings cards) do not yet, so rows
  overlap at 150 percent (gap below).
- Release size and launch time were not measured in this phase: the release build stopped
  with "no space on device" (drive F: had 0 bytes free, mostly other work); the partial output
  of this phase was removed. The additions are small (43 SVGs of about 25 KB, the token and
  widget code; `futures` and `image` were already linked), so the phase 0 baseline
  (10,667,520 bytes, first frame 94 ms) should move little; measure at the start of phase 2.
  (Measured at the stage A integration, below: 11,074,048 bytes, first frame as phase 0's.)

### Not done in phase 1, or left for later phases

- Start-up error dialogs (a locked or newer cache) and the crash log hook, carried from phase 0:
  not started (the confirm dialog and notice they need exist now).
- Controls do not grow with Windows' text size yet (text does); rows and fixed heights move to
  text-scaled tokens with the lists (phase 2). Visual checks at 150 and 225 percent were done on
  the gallery only (below), not on real screens.
- The nav item, rail, command bar and message row are phase 2 views built on these widgets (the
  gallery's rows are samples; the mockup's spot sits at the row's top, the sample centres it).
- Menus and tooltips were not captured in a shot (they open on pointer events); they are
  covered by tests only.
- Owner review: the menu's 1 px edge; the dark dialog (paper on dimmed paper has little
  separation); the Kit placeholder tone.
- Access keys on commands, list keyboard model and active descendant: phases 2 and 13.

## Stage A integration (done on gpui/main)

The four layer branches merged into `gpui/main` with `git merge --no-ff`, in this order:
`gpui/harness` (61b1d35), `gpui/i18n` (5168b1f), `gpui/assets` (b06b7f7), `gpui/shell`
(b7d0a48). All four branches existed and started from phase 0's tip (fdd12cc).

### Conflicts and how they were resolved

- **harness + i18n:** `Cargo.toml` members, `Cargo.lock` (the GPUI package's dependency list),
  `ci.yml`, CONTRIBUTING.md and GPUI-BUILD.md (their check lists), this file: unions. `main.rs`
  and `window.rs`: i18n's design wins for the language (no `lang` in `Init`; the
  `CurrentLang` global set by `i18n::init` before the window opens, the OS title observer,
  captions through `tr!`), the harness's snapshot, exit code, focus and tab keys are kept. The
  harness's test helpers lost their `lang` argument: `test_support::init_app` sets English and
  `test_support::set_lang` switches (as start-up does).
- **assets:** only this file (the phase table and the log).
- **shell:** the shell turned the GPUI package into a library plus a binary (`src/lib.rs`) and
  replaced `palette.rs` with the token layer (`src/theme/`). The merge moved the harness's and
  i18n's modules (`i18n`, `snapshot`, `tabs`, `test_support`, `text`) into `lib.rs`, and:
  - `window.rs`: the shell's title bar, tokens and live feeds, plus the CurrentLang observer, the
    tab keys and focus, the snapshot capture (its blank check now reads `cx.tokens().color`).
  - `titlebar.rs`: the caption names come from `tr!` (the shell's hard-coded English and
    Portuguese table is gone), `TitleBar::new()` takes no language, the title is a
    `SharedString` (secondary windows pass a `tr!` text), and `always_active` keeps a snapshot
    drawn as the active window.
  - `main.rs`: the theme is installed through `theme::install` with the snapshot's theme and,
    in a snapshot, animation effects off (the tokens then set reduce motion); `i18n::init` and
    the tab keys before it.
  - `test_support.rs` installs the light tokens (animation effects off) and gains `set_theme`;
    `init` takes only the cache.
  - The two lints met the shell's widgets: the loader's 8 fps step, the Sent notice's fade and
    hold and the gallery's shot delay carry `// timer-exempt:` reasons (design timers, as
    Skua's); `TOGGLE_INSET * 2.` became a sum. The shell's `tests/token_lint.rs` skips
    `test_support.rs` as `cargo xtask lint` does.
  - The catalog test skips `src/gallery/` (development only, sample mail) in its shown-literal
    check; `translations/komorebi.pot` regenerated (only the caption names' `#:` file changed,
    window.rs to titlebar.rs).
  - `crates/komorebi-gpui/Cargo.toml`: `png` (snapshots) and `futures` and `image` (shell) all
    kept; the `windows` features are the union.

### Checks (CARGO_TARGET_DIR `F:\dev-cache\cargo-target-gpui-harness`, reused for disk space)

- `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`
  and `cargo clippy -p komorebi-gpui --all-targets --features gallery --locked -- -D warnings`:
  clean.
- `cargo test --locked --workspace`: exit 0. Library 685 passed (1 ignored), Slint app 301
  passed (1 ignored), every integration test (`i18n_catalog` 9), GPUI app 62 unit and headless
  tests plus `brand_assets` 4 and `token_lint` 2, `komorebi-i18n` 8 and its doc test, xtask 9.
  Nothing failed, so no reruns were needed.
- `cargo xtask lint`: ok (tokens, timers). `scripts/check-pins.ps1`: ok. `cargo deny
  --workspace --locked check licenses advisories` online (deny.toml has `yanked = "deny"`, so
  the index was asked about yanked crates): advisories ok, licenses ok.
- Release build: `cargo build --release --locked -p komorebi-gpui -j 24`, 8 min 38 s.
  `komorebi-gpui.exe` **11,074,048 bytes** stripped (`cargo xtask size --gpui`: ceiling
  21,500,000, 10,425,952 left): phase 0 10,667,520 (+406,528), phase 15 10,850,304 (+223,744:
  the shell's widgets, tokens and 43 SVGs, the snapshot code and `png`, the 1.0 icon
  resources). `scripts/check-manifest.ps1` on it: every item ok.
- Boot check on the release exe (`scripts/gpui-boot-check.ps1`, offline, throwaway caches in
  the session scratchpad): GPU 1280 x 800 light, WARP 1280 x 800 light, WARP 900 x 700 dark in
  European Portuguese: each drew, passed `KOMOREBI_SNAPSHOT_CHECK` and exited 0 (WARP after one
  more DWM composition). `scripts/gpui-uia-check.ps1 -Warp`: Window 'Komorebi' with the
  Minimize, Maximize and Close buttons (Invoke pattern), closed through UI Automation, exit 0.
  No app window was left running.
- First frame (`KOMOREBI_EXIT_AFTER_FIRST_FRAME`, `KOMOREBI_TIMING`, offline, each exe on its
  own copy of a throwaway cache, phase 0's release exe and this one interleaved, 1 warm-up and 7
  runs each): phase 0 median 115.5 ms (108.5 to 148.9), stage A median 118.5 ms (104.2 to
  129.5); the same within noise, under the 200 ms budget. This session's machine was slower
  than phase 0's measurement (94.3 ms median there, the bench script on a seeded cache), so the
  interleaved pair is the comparison. Stage A breakdown (one run): platform 33.7 ms, DirectX
  41.6, cache 52.4, window 106.0, first frame 112.8. A boot check on a cache that does not exist
  yet loads it later (118 to 131 ms: the cache is created and migrated), first frame 159 to 207.
- Gallery (`--features gallery`, `KOMOREBI_GALLERY=light|dark`, `KOMOREBI_GALLERY_SHOT`):
  rendered and looked at, light and dark: title bar with the ink mark, the search pill,
  buttons, pills, tabs, rows with spots, multi-select, chips, settings cards, toggles, fields,
  notices, loader and the Sent notice with the seal; nothing missing or broken.

### Remaining problems after stage A

- CI has not run the new jobs yet (fxc on the Blacksmith image, the WARP boot check in the
  runner's session, the UIA dump): first push.
- Two token lints overlap (`crates/komorebi-gpui/tests/token_lint.rs` and `cargo xtask
  lint`); keep both until one owner folds the test into xtask (phase 18 cleanup).
- The gallery's shots use `image` and the snapshots `png`: one encoder is enough (phase 18).
- A snapshot re-reads Windows' animation setting when its window activates
  (`feeds::refresh_from_windows`), so reduce motion is only forced until then; harmless for the
  empty shell, to revisit when views animate on start.
- Carried: start-up error dialogs and the crash log; controls that grow with text size; Kit's
  built-in strings in pt-PT (GPUI-I18N.md); the tray and toasts on a real taskbar (phase 11);
  the 1.0 setup's pages not opened; `release.yml` passes `-Brand10` at the cut-over; the owner
  review points of phase 1.

## Phase 3: reader, tile element, links, banners, invite card (done on gpui/reader, with gaps)

Branch `gpui/reader` (from `gpui/main` d94a86a). The reader is a view of its own,
`views::reader::ReaderView`, for the list lane's reader slot (`ReaderView::new(db, window, cx)`:
the view owns its own connection to the cache).

### What was done

1. **The reader's engine moved into the library, toolkit-free** (first commit of the lane):
   `komorebi::reader` holds the render worker (layout, 256 px tiles on demand, selection on the
   retained `Doc`, consented remote images on a pool of four, generations), its laid-out page
   cache and byte-bounded tile cache (24 MB, 8 MB for a message window), the page geometry
   (`page_width`, `tile_span`, `wanted_tiles`, fit-to-column scaling), `PageLink`, `Page`,
   `Shell`/`SystemShell`/`shell_execute`, and the read delay (`ReadWait`, from `src/app.rs`).
   Tiles come as `Pixels` (premultiplied, `Arc<[u8]>`) in RGBA (the Slint app) or BGRA (GPUI:
   `Worker::with_order`), converted once on the worker; `Worker::with_text_lines` also sends each
   page's lines of text in view coordinates (`Reply::Text`, for screen readers).
   `komorebi::reader::session::Session<T>` is the UI-side state machine the Slint `State` had
   (which page and key, tiles near the view, multi-click, link hover/Tab focus/anchors/mailto/
   confirm prompt, remote images, original colours, the text fallback after a second, the resize
   debounce), with no toolkit: the view feeds it a `Frame` and takes `Effect`s (scroll to, copy,
   compose, the two timers) and the tile images to release. The Slint app calls the moved worker
   and keeps its own `State` until the cut-over.
2. **`komorebi::find`** (Find settings' form merge, from `src/app/find.rs`; the lookup thread and
   the note's words stay there) and **`komorebi::invite`** (the invite card's facts as typed
   values, the prompt's warnings, the reply queued for the organizer, the answer kept and the
   local calendar in step, from `src/app/invite.rs`). The Slint files call them.
3. **The view** (`crates/komorebi-gpui/src/views/reader/`): `mod.rs` (the view, its keys in the
   `Reader` context: Ctrl+A, Ctrl+C, Escape, Tab and Shift+Tab over the links then on out of the
   page, Enter; `bind_keys` installs them), `worker.rs` (the GPUI adapter: BGRA tiles made into
   `RenderImage`s on the worker thread, replies over a futures channel), `tile.rs` (the tile
   element: a canvas that paints the page colour, the tiles with `paint_image`, the selection and
   the focused link's ring, each edge snapped to device pixels, and measures itself in prepaint),
   `links.rs` (the confirm prompt: host apart, whole address, one line per warning, Enter opens
   only without warnings; the target strip), `banner.rs` (the bars: sender notice with Report
   junk, Delete, Why?; encrypted; server draft with Edit; remote images blocked or failed with
   Load images and Always load from this sender; "will always load" with Undo and Privacy
   settings; the allowed caption), `invite.rs` (the card), `attachments.rs` (40 px inset pills
   that wrap, 180 to 280 px wide), `a11y.rs` (the page's lines as AccessKit `Label` nodes with
   window device-pixel bounds), `format.rs` (temporary, below). New tokens in `theme/reader.rs`.
   Evicted, replaced and closed tiles leave every window's sprite atlas through
   `App::drop_image`, deferred past the update.
4. **What the view hands its owner** (`ReaderEvent`): mark read (after the delay, or at once),
   compose (`mailto:`), download a body (and Retry when the server is out of reach), open or
   save an attachment, save the invitation, the queued invite reply's dispatch, a status
   sentence, Privacy settings, Report junk / Not junk, Delete, Why?, Edit a server draft, and
   `AskRsvp` (the owner asks the RSVP question, then calls `answer_invite`).
5. **Snapshots:** `crates/komorebi-gpui/examples/reader_shot.rs` puts one message (an `.eml`)
   into a throwaway cache with the seed account and renders the reader pane to a PNG once the
   page and its visible tiles are in (`Window::render_to_image`, as the gallery's shots: it works
   with the display off), with the snapshot module's safety rules (`KOMOREBI_OFFLINE`, a
   throwaway `KOMOREBI_DB`, `KOMOREBI_SNAPSHOT`); `build.rs` links the manifest into the
   examples (without Common Controls 6 the example failed to start, 0xC0000139).

### Decisions

- **One state machine for the GPUI reader, in the library.** The Slint `State` mixed the logic
  with Slint models and timers; `Session` keeps the logic and leaves drawing, timers and the
  clipboard to the view, so the Slint view tests became plain `#[test]`s (on the session) and
  short `#[gpui_kit::test]`s (on the wiring). The Slint app keeps its copy: replacing it before
  the cut-over would change shipping code for no user gain.
- **Tile images are made on the worker thread** (the notify closure runs there), so the UI
  thread only places them; the worker's cache holds the `Arc<[u8]>` and each reply copies once.
- **The view measures itself in prepaint** (cookbook_measure) and defers the update: the band in
  view, the column width, the scale and the theme go to the session as one `Frame`; a theme or
  scale change is a relayout with the resize debounce, as a width change.
- **Links for screen readers** are real elements (role Link, the text as name, the address as
  description, AccessKit Click activates) for the links within a screen of the view only, so a
  long newsletter does not build thousands of elements a frame; the page's lines are synthetic
  children (no element cost). Narrator's scan mode reads lines; Tab reaches every link.
- **The selection highlight** is ink at 16 percent over the page (`theme::reader::selection`):
  the brand's wash would hide the text on a page of pixels.
- **Attachments follow the mockup** (after the body), not the Slint reader (above it). Owner
  review.
- **Find in a message does not exist in the Slint app** (Ctrl+F reaches the search box); the
  plan's `find.rs` is Find settings, whose merge moved to the library. No find bar was built.
- **Remote images** use `komorebi::fetch`, which refuses every request under
  `KOMOREBI_OFFLINE`.
- **APIs beyond the cookbook:** `Window::paint_image`, `App::drop_image` and `RenderImage::new`
  (the tiles, as the spike measured them, S4); `Element` synthetic children through
  `InteractiveElement::a11y_synthetic_children` and `on_a11y_action`; `Window::focus_next` /
  `focus_prev` (Tab off the last link); `Context::observe_in`, `Window::on_next_frame` and
  `Window::render_to_image` (the example only). In tests: `TestAppContext::simulate_window_resize`,
  `read_from_clipboard`, Kit's `TestWindowExt::drag`, `click_with_options`, `scroll`.
- **Dependencies:** `image =0.25.10` is no longer optional in komorebi-gpui (the tiles need
  `image::Frame`; the copy GPUI links, so the exe gains nothing); `chrono` (the library's copy)
  for dates. The `gallery` feature no longer names `dep:image`.

### Tests

GPUI-TEST-PORT.md rows of phase 3 are filled in. Library: 18 worker and geometry tests moved
from `src/app/reader.rs` and the worker half of `links_are_hit_under_scale`, 3 new (whole pixels, BGRA tiles, text lines), the read delay test
moved from `src/app.rs`, 7 session tests (the Slint view tests without the window, plus a theme change), the read delay test, 5
Find settings tests moved from `src/app/find.rs`, 3 new invite tests. GPUI: 19 reader tests
(the view tests over a fake worker, plus the unit tests of the tile snapping, the a11y nodes,
the warnings, the invite card's lines, the image bar's sentence, the formats). The Slint app
keeps 6 reader view tests and `invite_card_answers_and_replies` (all pass).

### Checks

CARGO_TARGET_DIR `F:\dev-cache\cargo-target-gpui-reader`. `cargo xtask check`: fmt, clippy
(workspace, all targets, `-D warnings`), komorebi-gpui 83 unit and view tests plus
`brand_assets` 4 and `token_lint` 2, the library 723 (1 ignored), xtask 9, lints ok; size
skipped (no release build in a lane, by the plan). The Slint app's tests (`cargo test -p komorebi --bins --locked -- --test-threads=4`): 277 passed, 1 ignored (301 at stage A: 24 moved to the library with the reader worker, Find settings and the read delay).
`tests/i18n_catalog.rs`: every rule passes except `template_is_current`, whose only difference
is `#:` references (no new or changed msgid, so the `.po` needs nothing); the template is the
integrator's to regenerate, as the stage B plan says.

Snapshots (debug `reader_shot`, offline, throwaway caches in the session scratchpad, 640 x
784, looked at against `inbox-light-wide.png` and `inbox-dark-wide.png`): a mockup-like HTML
message light in English and European Portuguese, dark, on WARP; the corpus bank statement
(`05-bank-statement.eml`, a fixed-width page drawn from many tiles) light and dark. Header,
spacing, attachment pills, the dimmed dark page and the tiles match the mockup's reader; the
page's own margins and link colours are the HTML renderer's, as in the Slint app.

### Not done in phase 3, or left for later

- **Integration (stage B integrator):** mount `ReaderView` in the list lane's reader slot,
  call `views::reader::bind_keys` in `main.rs` and `test_support::init_app`, handle the
  `ReaderEvent`s (mark read through the sync workers, the composer, body downloads, the RSVP
  question, attachments, dispatch of the queued reply); `views/mod.rs` and `lib.rs` each carry
  one line of this lane (`pub mod reader;`, `pub mod views;`), and `theme/mod.rs` one
  (`pub mod reader;`).
- **`views/reader/format.rs` is a temporary copy** of the Slint app's `sender`, `recipients`,
  `size`, `file_kind` and the date: replace it with `komorebi::format` from the list lane's
  move (English dates then follow Windows' short-date format, as in the Slint app).
- **`src/app/files.rs`** (open and save attachments, Mark of the Web, the clipboard's HTML) is
  toolkit-free but shared with the composer and owned by no lane: move it into the library at
  integration; until then the reader hands attachments to its owner.
- **Selectable plain text:** the text view is plain text, not yet selectable (the Slint app used
  a read-only text input; a multi-line read-only Kit field is the candidate).
- **Source view and More menu commands** (Show original, Save as .eml, Copy Message-ID,
  Forward as attachment, rows 140, 141, 173), the command bar (row 160) and the Unread filter
  (row 168) belong to the list lane's commands and filters.
- **The draft-reply bar** ("You started a reply", Resume) needs the composer's drafts (phase 4).
- **The "Why?" dialog** of the sender notice and the RSVP question are the owner's dialogs.
- A save glyph for attachments: the icon set has none; the document glyph stands in.
- By hand: Narrator over the page's lines and links, 125 and 150 percent scaling on a real
  display, touchpad scrolling, a page wider than the column (sideways scroll).
## Stage B plan: inbox and folders, reader and tile element, composer

Three agents in parallel, one branch each from `gpui/main` after stage A (`gpui/list`,
`gpui/reader`, `gpui/composer`), then one integrator. Each lane writes only the files below;
anything else it needs goes in its report for the integrator.

| Lane | Phase | Owns (new or changed) | Library work it owns |
|---|---|---|---|
| list | 2 | `crates/komorebi-gpui/src/views/mod.rs` (the module list; the other lanes add one line each, merged as a union), `src/views/mail/` (the mail view's layout: rail, folder tree, list pane, the reader and composer slots), `views/mail/{folders,list,rows,filters,search,commands,menus}.rs`, `window.rs` (mounting the mail view), `widgets/pane.rs`, `widgets/pill.rs`, `widgets/spot.rs`, `widgets/tabs.rs` (row and nav changes), `theme/metrics.rs` (text-scaled row and control heights for every lane) | Move `src/format.rs`, `src/rows.rs` and `src/tree.rs` from the Slint binary into the library (`komorebi::format`, `komorebi::rows`, `komorebi::tree`), rewriting `outbox_status`, `send_sentence` and `avatar_color` free of Slint; the Slint app calls the moved code. First commit of the lane, so the others can rebase onto it. |
| reader | 3 | `src/views/reader/` (`mod.rs`, `tile.rs` the tile element, `worker.rs`, `find.rs`, `links.rs`, `banner.rs`, `invite.rs`, `attachments.rs`, `a11y.rs` text nodes) and new `theme/reader.rs` tokens if needed | Move the reader worker and tile cache out of `src/app/reader.rs` (and `src/app/find.rs`, `src/app/invite.rs`) into the library behind a toolkit-free API; only this lane touches those Slint files. |
| composer | 4 | `src/views/composer/` (`mod.rs`, `editor.rs` the painted rich editor and input handler, `toolbar.rs`, `recipients.rs`, `signatures.rs`, `attachments.rs`, `controller.rs`), `widgets/field.rs` (recipient chips in fields), new `theme/composer.rs` if needed | Move the controller, markup and rich text model out of `src/app/composer.rs` and `src/app/composer/` (and `src/app/signatures.rs`) into the library; only this lane touches those Slint files. |

Rules for all three:

- The mail view's slots are typed: the list lane mounts `reader::ReaderView` and
  `composer::ComposerView` through one constructor each (`new(db: Db, window, cx)`), agreed in
  the list lane's first commit as stubs; the reader and composer lanes fill them in and never
  edit `views/mail/`.
- Shared files are append-only and merged as unions: `Cargo.toml`/`Cargo.lock` (regenerate the
  lock at the merge), `crates/komorebi-gpui/src/lib.rs` (none should need it: `views` is the
  list lane's one new line), `translations/pt-PT/LC_MESSAGES/komorebi.po` (new entries only),
  `docs/GPUI-TEST-PORT.md` (each lane marks only its phase's rows: list 51, reader 34, composer
  93), and this file (each lane writes only its phase section and one phase log row). The
  template `translations/komorebi.pot` is regenerated by the integrator, not the lanes.
- Existing widgets outside a lane's list are not edited; a lane that needs a change describes it
  in its report (or adds a new widget file named for the lane).
- Each lane ports its phase's tests from GPUI-TEST-PORT.md, keeps `cargo xtask check` green,
  uses its own `CARGO_TARGET_DIR` under `F:\dev-cache` (or reuses one when the disk is short),
  and skips the release build: the integrator builds once.
- Integration order: list, reader, composer (the reader and composer rebase onto the list
  lane's library moves). Then the full checks, one release build, the boot check, and the
  budgets: launch under 200 ms and a cached list under 600 ms on a seeded throwaway cache
  (`scripts/gpui-bench.ps1`), and the size against 21,500,000 bytes.

## Phase 2: inbox and folders (lane `gpui/list`)

Branch `gpui/list` from `gpui/main` d94a86a. The mail view: the folder pane (or rail), the
message list and the reading pane around the reader and composer slots, in the main window
under the title bar's search.

### What was done

1. **Library moves (the lane's first commit, 307490a).** `src/format.rs` and `src/rows.rs`
   are library modules now (`komorebi::format`, `komorebi::rows`; `komorebi::tree` already
   was). `outbox_status` and `send_sentence` use `tr!` with the Slint texts (the template gains
   only file names), `avatar_color` became `avatar_rgb` (`0xRRGGBB`), and the rows report what
   changed (`Refresh`, `Removed`, the indices `select` returns) instead of calling Slint's
   `ModelNotify`; each row is a plain `Row` (with `who`, the person for the spot's initials).
   The Slint app keeps thin wrappers: `format::avatar_color` in `src/main.rs` and the `Model`
   in `src/app/rows.rs` (which passes the composer's `above_quote` for draft previews). Their
   19 tests moved with the code; the Slint app's tests pass (282, 1 ignored).
2. **Typed slots.** `views::reader::ReaderView` (`new(db, window, cx)`, `show(id, cx)`,
   `message()`) and `views::composer::ComposerView` (`new(db, window, cx)`, `open(draft,
   window, cx)`, `is_open()`, `draft()`; `Draft::New`, `Saved(id)`, `Reply { message, all }`,
   `Forward { message, attached }`). `views::Db` is the window's `Rc<komorebi::db::Db>`. The
   mail view mounts both and draws the pane around them.
3. **The model** (`views/mail/model.rs`, `triage.rs`, `nav.rs`; no GPUI, plain tests), ported
   rule by rule from the Slint controller (`src/app.rs`, `src/app/triage.rs`): the view (All
   inboxes, a folder, Drafts, the Outbox) and the start view, the filters, the search with its
   pause on large folders and the all-folders scope, the selection (click, Ctrl, Shift, Shift
   and the arrows, Ctrl+A, the right press), the open message and its read delay, read and
   important flags, Archive, Delete (Trash and Junk ask first), Move (recent folders, filter,
   depth), junk and not junk, the held move with its Undo window and clock (paused while
   hovered), Show in folder, Go to the Inbox, Mark all as read (pane and list), the folder tree
   with its folds, the default folds written after the first frame, the pane's and the rail's
   lines and menus, the empty list's sentences. Actions ask the view for timers, the composer,
   a question or the focus (`Effect`).
4. **The views** (`views/mail/`): `folders.rs` (canvas pane: New mail, the lines with the 6 px
   place spot, the account's quiet label, chevrons, counts, Calendar, Contacts, Settings at the
   foot; the rail below the compact width or when folded by the user, with its Folders button
   that opens the full pane over the content), `list.rs` and `rows.rs` (paper pane: title and
   account, the "..." menu, the pills, a search's results line with Search all folders and Clear
   search, the status line, a `uniform_list` of rows read from the cache in view only, the Undo
   slot; rows of the mockup's `.row`: the spot at the top left, three lines, the marks, the
   date; quick actions on hover or on the keyboard's row; round checks while rows can be
   checked), `reading.rs` (the command bar over the open message: Reply the only labelled pill;
   the selection panel with Archive, Mark as read, Delete, Select all, Clear selection),
   `menus.rs` (Kit popup menus for the row, the list, the pane lines and the More menu; the
   keyboard's menus and Move as a flyout), `filters.rs`, `search.rs`, `commands.rs` (the keys)
   and `text.rs` (every word, the Slint catalog entries).
5. **Keys** (`commands.rs`, the README keyboard map): Ctrl+N, Ctrl+E, F3, Ctrl+F, Ctrl+Shift+I,
   F5, Ctrl+Shift+E in the window; Ctrl+Q, Ctrl+U, Insert, Ctrl+Shift+G, Delete, Backspace,
   Ctrl+Shift+V, Ctrl+Z, Ctrl+R, Ctrl+Shift+R, Ctrl+Shift+F, Esc on the list and the reading
   pane (never the composer); Up, Down, Home, End, Page Up, Page Down, Enter, Shift+Up,
   Shift+Down, Ctrl+A, Shift+F10 and the Menu key in the list; Up, Down, Home, End, Enter,
   Space, Right, Left (TreeView), Shift+F10, Esc in the pane. The search box hands Down, Enter
   and Esc back to the list.
6. **The window** (`window.rs`) mounts the view, draws the search in the title bar's centre
   once there is an account, routes the window keys, and sets the OS title from the inboxes'
   unread count (`MailEvent::Unread`).
7. **Text-scaled heights for every lane** (`theme/metrics.rs`, `scaled`): `message_row`,
   `message_slot`, `control(base)`, `nav_item`, `indent(level)`; a row is 83 px (plus a 2 px
   gap) at 100 percent and grows with its lines. `widgets/pane.rs`: `ListRow` gains
   `focusable`, `ring`, `current` (the active descendant) and `marker`.
8. **Snapshots** (`window.rs`): `KOMOREBI_SNAPSHOT_SEARCH=<text>` and
   `KOMOREBI_SNAPSHOT_SELECT=<row>[,<row>...]` (one opens, several are Ctrl+clicked) run before
   the first frame; a development build (`--features gallery`) with
   `KOMOREBI_SNAPSHOT_OFFSCREEN=1` draws the frame with `Window::render_to_image` instead of
   `PrintWindow`, for a display that is off (PrintWindow returned black frames on this machine,
   GPU and WARP alike). Looked at against the mockups (`inbox`, `multi`, `empty`, light and
   dark, wide and compact, pt-PT, 150 percent text) and fixed: the search's placeholder, the
   marker on checked rows (the mockup's checked rows are wash only), the results line's
   alignment.

### Decisions

- **The model is in the GPUI app, not the library**: the Slint app keeps its controller until
  the cut-over; moving it would rewrite the shipping app for no gain now.
- **No account workers yet.** The GPUI app starts none, so changes for the server are kept as
  jobs (`Mail::take_jobs`) and dropped by the view; a held move's end still deletes its rows
  from the cache as in the Slint app, and Refresh re-reads the cache. Whoever starts the
  workers sends the jobs there (`MailView::settle`).
- **Behaviour is the Slint app's, the look the mockups'.** Where they differ: the local Drafts
  line stays above the accounts (the Slint pane), the folder's account is the list's subtitle
  (not "2 unread"), the selection panel is the Slint one in the reading pane (the mockup's
  right side), the placeholder says "Search {view}". With one account the rail lists its
  top-level folders (the compact mockup); with several, one tile per account (the Slint rail).
- **No date groups** ("Today", "Earlier this week" in the mockups): the Slint list has none,
  and the virtual list needs one row height. Owner review.
- **Menus from the keyboard are flyouts.** Kit's context menu opens on a mouse press only, and
  synthetic input is for fixtures and tests (cookbook_synthetic_click), so Shift+F10, the Menu
  key and Move use the palette pattern (cookbook_palette): the same entries, keyboard-driven.
- **Ctrl+Shift+I** is bound without a context: Kit binds its inspector there, and GPUI ranks
  context-free bindings as the deepest; the later binding wins.
- **GPUI APIs beyond the cookbook**: `UniformListScrollHandle` with `ScrollStrategy::Nearest`
  (the cookbook names the handle, without an example), `on_hover` (the rows' quick actions,
  the Undo slot's pause), `aria_active_descendant` and `aria_expanded`,
  `InputState::set_placeholder`, `Window::window_handle` and `Window::render_to_image`
  (development builds only); Kit's `WindowExt::has_active_dialog` in tests.
- **Gotcha:** a `ScrollHandle::scroll_to_item` asked before the first frame is dropped (the
  first layout finds the line but does not scroll), so the pane scrolls to the open view's line
  after the first frame (`MailView::follow_pill`).
- **One new catalog entry**, "Filter" (the pills' group name; pt-PT "Filtro"); every other word
  is a Slint entry. The template is left for the integrator: regenerated, it gains "Filter" and
  file names only, and the catalog test then passes (checked).

### Checks

CARGO_TARGET_DIR `F:\dev-cache\cargo-target-gpui-list`. `cargo xtask check`: fmt, clippy
(workspace, all targets), the GPUI app's 102 unit, view and window tests, `brand_assets` 4,
`token_lint` 2, library 707 (1 ignored), xtask 9, both lints: ok (no release build: the
integrator builds once). `cargo clippy -p komorebi-gpui --all-targets --features gallery`: ok.
`cargo test -p komorebi --bins`: the Slint app 282 passed, 1 ignored. `cargo test --test
i18n_catalog`: 9 passed with the template regenerated (then reverted). Snapshots of the debug
exe, offline, on throwaway seeded caches in the session scratchpad.

### Not done in phase 2

- Drag to folder.
- The Outbox entry's pane (its status, Retry, Discard), a draft's Discard from the list: the
  composer and reader lanes' panes; a saved draft opens the composer slot.
- What needs the account workers: the loader and the sync line, offline and problem badges and
  status lines with actions, the first download's progress, Refresh as a sync.
- Open in new tab or window, Save as .eml, Show original, Copy Message-ID (phases 3 and 5;
  their menu entries are greyed).
- F6 round the panes and the Tab order (phase 13); access keys on the commands.
- `snapshot.rs` still lists `KOMOREBI_SNAPSHOT_SELECT` and `_SEARCH` as ignored (they now
  work): its owner drops them from `NOT_YET`; `blank_parts` was not run against the views.
- Release size and launch timing (the integrator's build).

## Phase 4: the composer (done on gpui/composer, with gaps)

Branch `gpui/composer` (from `gpui/main` d94a86a), the composer lane of stage B. The library
moved first (one commit, so the other lanes can rebase onto it), then the GPUI view.

### What was done

1. **The composer's model in the library** (`komorebi::compose`, `src/compose/`): `Draft`,
   `Drafts`, `Kind`, `Field` (the library's own field enum; the Slint app maps its generated
   `ComposeField` with `composer::field`/`ui_field`), saving and loading (`store`, `load`,
   `Stored`), the autosave and server-copy rules, signatures (`sign`, `resign`,
   `replace_signature`, `pick_signature`, `switch_account`, `signature_for`, `prepare`),
   quotes and subjects (`compose::quote`), addresses and suggestions (`compose::address`,
   `suggest`, `accept_suggestion`), attachments and their limits (`compose::attach`), the send
   guards and blocks (`check_send` returns `SendCheck`), importance and the switch between
   formatted and plain text (`set_importance`, `set_rich`, `can_unplain`), the send build and
   the undo window's state (`compose::send`: `PendingSend`, `BuildJob`, `outgoing`,
   `short_subject`), the Preview raster (`preview_html`, `preview_page`, a tiny-skia pixmap),
   the plain-text markup edits (`compose::markup`, moved as it was), the recipient chips'
   model (`compose::recipients`, moved, its Slint wiring left in the app), and the formatted
   editor's commands (`compose::rich`: `Action` and `apply`, the Slint toolbar's `ed_act`, plus
   `doc_from_text`, `is_blank`, `logos`, `link`, `straight_bgra`). `Heads` (the quote heads in
   the language shown) takes its words and its date function from each app. The Slint app's
   `src/app/files.rs` moved too, as `komorebi::files` (CF_HTML, CF_HDROP, opened attachments):
   the composer's clipboard needs it and the reader's attachments will. Its controller tests
   (rows 53 to 73, 85 to 90, 180 to 187, 260 to 272) moved with the code; `Db::file` is new (a
   worker opens its own connection to it). The Slint app calls all of it and keeps its view
   tests (253 pass, 1 ignored).
2. **The view** (`crates/komorebi-gpui/src/views/composer/`): `ComposerView::new(db, window,
   cx)`, the slot the mail view mounts, with `compose_new`, `compose_mailto`, `reply`,
   `forward`, `forward_attached`, `open_saved`, `send`, `undo_send`, `close`, `park_composer`,
   `discard`, `attach_paths`, `save_all`, and `ComposerEvent` for the shell (status lines,
   Drafts changed, server copies, the undo offer, `Queued` for the send worker, Manage
   signatures). The header (title, Attach, Discard, Send), From (a select with several
   accounts), To, Cc and Bcc as chips with suggestions under the field, Subject, the link row,
   the format bar (Bold, Italic, Underline, the lists, Link, the Signature menu and More:
   strikethrough, indent, quote, alignment, a table, clear formatting, importance, Preview,
   plain or formatted text; Font, Font size, Font colour and Table as submenus, Edit quoted text), the body, the attachments with their counter, the send guards'
   notice, and the caption ("Draft saved 14:05"). Keys: Ctrl+Enter and Alt+S send, Esc
   closes (or closes the suggestions or the link row first), Ctrl+Shift+C and B show Cc and
   Bcc, Ctrl+Shift+A attaches, Ctrl+K links; files dropped on the composer are attached.
3. **The painted rich editor** (`editor.rs`, REPORT.md S5): the library's `Editor` laid out by
   `mail::rich::view::View` and painted in pieces; each piece is a `RenderImage` (straight
   BGRA from the premultiplied pixmap), kept by key, painted with `paint_image` and released
   with `drop_image` once it changes or leaves the view (pieces one view above and below are
   kept). Selection and caret are quads (wash, ink); `EntityInputHandler` takes typing and
   IME; the composition is kept beside the model, drawn over the text at the caret in a paper
   box and underlined, and committed as one edit (one undo step). Keys as the Slint editor's
   (`format-key`, `nav-key`): arrows, words, lines, the document's ends, pages, Tab between
   table cells, undo and redo, select all, Ctrl+B, I, U, Shift+X, Shift+8, Shift+7, M,
   Shift+M, L, E, R, J, ], [, Space, Shift+Q; clicks, Shift+clicks, double clicks and drags;
   Ctrl+C and X put the model's HTML and text on the clipboard (`komorebi::files`), Ctrl+V
   pastes formatted text through the sanitizer and the lossy import, files are attached;
   stored quotes are laid out on a thread of their own (the reader's stack); signature logos
   come from the cache. A plain-text draft uses Kit's `Textarea`.
4. **Widgets and tokens:** `ChipField` in `widgets/field.rs` (label, wrapping chips and the
   Kit field, a quiet trailing button, an inline error), `theme/composer.rs` (the mockup's
   composer geometry and `editor_theme`, the page colours from the tokens), 16 Fluent icons
   (bold to eye) in `icons.rs` and NOTICE, `examples/composer_shot.rs` (below).
5. **Tests** (rows 53 to 130 and 180 to 290 of [GPUI-TEST-PORT.md](GPUI-TEST-PORT.md), all 93
   with a status): 54 library tests in `komorebi::compose` (with the moved `markup` and
   `recipients` tests) and `komorebi::files`; 22 window tests (`views::composer::tests::port`), 9 editor tests (with
   the spike's IME tests on the real editor, and the typing latency measurement, ignored), 2
   text tests (the Slint app's sentences in English and pt-PT).

### Decisions

- **The document lives in the editor while its draft shows.** `RichEditor` owns the `Editor`
  (it implements the input handler); `ComposerView::lend` gives the document back to the draft
  for whatever reads the whole draft (saving, sending, the guards, signatures, importance) and
  takes it again; `stow` puts it back before another draft takes the composer or it hides.
- **The composition is drawn over the text, not in it** (a paper box at the caret, as many
  editors' over-the-spot IME), so the model never holds uncommitted text and a commit is one
  undo step. The caret does not blink (the app has no timers but design delays).
- **A plain-text draft has no format bar**, only the Signature and More menus (the Slint app's
  markup edits stay in the library for its toolbar until the cut-over); plain text is sent as
  typed.
- **Suggestions sit under their own field** (a `deferred` list in the field's row), Up, Down
  and Enter work them, Esc closes them first.
- **Leaving an address field is seen at render** (`follow_focus`: the focused field compared
  with the last render's), not through Kit's `InputEvent::Blur` or `cx.on_blur`: neither
  reached the view in the headless tests.
- **Sends:** the composer saves, moves the row to the outbox (`outbox_send`), builds the
  message on GPUI's background executor while the undo window runs on the executor's clock
  (`// timer-exempt:`, the delay the user set), then queues it (`outbox_queue`) and emits
  `Queued { row, account }`: the GPUI app has no send workers yet, so the shell hands it on.
  The autosave is the same kind of design delay (two seconds after the last edit), measured on
  the executor's clock so tests move it.
- **Words:** every sentence is a `tr!` with the Slint app's exact text and context, so all are
  translated already (a regenerated template adds only `#:` lines; the integrator regenerates
  it). The quote heads' date and the attachment sizes use local helpers until phase 2 moves
  `format.rs` into the library (`text::heads`, `text::size`, `compose::clock`).
- **Pinned additions:** `image` is no longer optional in the GPUI app (the editor's frames are
  `image::Frame`s; the copy GPUI links); `unicode-segmentation =1.13.3` (grapheme boundaries;
  the library's copy). The build script also links the manifest into the examples.
- **GPUI APIs beyond the cookbook:** `RenderImage::new`, `Window::paint_image`,
  `App::drop_image`,
  `ScrollHandle::set_offset`, `Window::defer`, `ExternalPaths` with `on_drop`,
  `InputEvent::PressEnter`, `BackgroundExecutor::now`, and in the example
  `Window::render_to_image` and `Window::draw` (test-support, a dev-dependency).

### Gotchas

- Kit's `Textarea` with zero padding hangs its layout in a narrow pane; keep Kit's padding (the
  container's left padding is shortened by `PLAIN_PAD` instead).
- In Rust 2024 a `render_*` helper returning `impl IntoElement` captures `&self` and `cx`; add
  `+ use<>`.
- A test module in its own file (`tests.rs`) is not seen as a test module by the token lint or
  the catalog's literal check; wrap it in one `#[cfg(test)] mod port { .. }`.
- With the display off (a locked or remote session), GPUI draws no frames: `PrintWindow`
  returns black and a deferred relayout never shows. The example renders with
  `render_to_image` after drawing twice by hand.

### Checks

`cargo xtask check` (CARGO_TARGET_DIR `F:\dev-cache\cargo-target-gpui-composer`): fmt, clippy
`-D warnings` over the workspace, the GPUI app's tests (94 lib tests and the integration tests;
1 ignored, the measurement), the library's 737 (1 ignored), xtask's 9, both lints: ok; the size
check skipped (no release build, as the plan says). `cargo test --locked --lib --bin komorebi
--test i18n_catalog -- --test-threads=4` after the library move: library 736, Slint app 253 (1
ignored), catalog 9: ok. On this branch the catalog's `template_is_current` fails until the
integrator regenerates the template (only `#:` lines change; no new message).

Typing latency, the ignored measurement (debug test build, the library's dependencies at
opt-level 2): 2,000 lines with 20 tables, the caret in the middle, a key through the input
handler and one drawn frame on the test platform: median 6.1 ms, p95 9.0 ms over 200 keys.

Snapshots (`examples/composer_shot.rs`, offline, throwaway caches in the session scratchpad,
`render_to_image`): the mockup's message formatted, light and dark, at 1280 x 800; a reply in
European Portuguese; a plain-text draft at 1000 x 700; attachments with the counter; Preview.
Looked at against `compose-light-wide.png` and `compose-dark-wide.png`: the header, From, the
chips with their avatars, Cc Bcc, Subject, the format bar's glyphs, the body's bold, italic,
link underline and bullets, the paper on canvas with 8 px gaps match; the mockup's list has
more space around it than the reader's layout gives (shared with the Slint app's editor).

### Not done in phase 4, or left for others

- **Mounting:** the mail view (list lane) mounts `ComposerView` in its slot and wires
  `ComposerEvent` (status line, undo offer, Drafts count and rows, the Draft tag, workers);
  `views::composer::bind_keys` must be called where `tabs::bind_keys` is (`main.rs` and
  `test_support::init_app`), and `save_all` on quit (`window.rs`). A snapshot action
  `compose` (`snapshot.rs`) can replace the example then.
- Tabs and windows of their own for drafts (phase 5, `Drafts::set_tabbed`, `swap_view` are in
  the library already); Settings > Signatures and Compose defaults (phase 6, rows 284, 285);
  the composer field half of `ctrl_tab_moves_between_tabs` (needs the mounted composer).
- Accessibility of the painted body (phase 13): a text node per line with bounds and the
  value and selection on the `MultilineTextInput` node (Narrator reads the name only now).
- The first-format caption and the drop overlay ("Drop to attach").
- By hand: a real Japanese and Chinese IME (candidate window position, conversion, cancel),
  Narrator, the clipboard's HTML with Word and a browser.

## Stage B integration (done on gpui/main)

`gpui/list`, `gpui/reader` and `gpui/composer` are merged with `--no-ff` in that order (merge
commits 3e8f306, 276b8d5, 02082d0). Checks on the merged tree: `cargo fmt --check`, clippy
with `-D warnings`, the full `cargo test --locked` (library 797 passed, 1 ignored), `cargo xtask
check` (lint ok, xtask tests 9 passed), `scripts/check-pins.ps1` and `cargo deny --workspace`
(licences and advisories) all pass.

Owner decision (2026-10-25): the stage B release build, boot check and benchmark are skipped.
Using GPUI is final whatever the numbers say. A larger exe and a slower launch than Slint are
accepted for 1.0 and become the work of the 1.1 optimisation release (phase 18 moves to 1.1,
apart from keeping the 21.5 MB gate in CI). The launch and cached-list budgets are measured
again at the 1.0 release candidate, for the record, not as a gate.

## Stage C plan: the remaining screens

Four lanes in parallel from `gpui/main`, then one integrator, under the stage B rules (own files
only, shared files append-only, one `CARGO_TARGET_DIR` per lane, no release build, check F: for
15 GB free first).

| Lane | Phases | Owns |
|---|---|---|
| shell | 5 tabs and secondary windows, 11 tray and toast activation, 12 updater UI, and the phase 2 sync parts (loader, badges, status actions, Refresh, Outbox pane) | `views/tabs/`, `views/windows/`, `views/updates/`, `tray.rs`, `window.rs`, `views/mail/` sync files |
| settings | 6 settings, accounts, appearance, shortcuts, about; 7 onboarding | `views/settings/`, `views/onboarding/` |
| data | 8 rules, 9 contacts and CardDAV | `views/rules/`, `views/contacts/` |
| calendar | 10 calendar, edit dialogs, reminders | Dropped by owner decision (2026-11-06): no calendar in 1.0, no lane |

Stage D (after stage C): 13 accessibility pass with Narrator by hand, the gaps listed under
phases 1 to 4, 14 remaining test ports, 19 parity fixes against the Slint app, the cut-over
(`release.yml` with `-Brand10`, docs, README and PERF wording for memory) and Komorebi 1.0.

## Phases 5, 11, 12 and the phase 2 sync parts: tabs, windows, tray, updater, sync (done on gpui/c-shell, with gaps)

The shell lane of stage C. No calendar anywhere (owner decision): no calendar tab, window,
setting, sync or reminder; a reminder toast target is not handled.

### What was done

- Mount points (first commit): `window.rs` mounts Settings, Onboarding, Rules and Contacts over
  the mail through `views/windows/mounts.rs` stubs (`MountEvent::Close`, `EditAccount`), so the
  other lanes replace the stub bodies without touching the window.
- Sync parts (phase 2): `views/mail/hub.rs` owns the account workers (`komorebi::mail::Workers`,
  started after the first frame, never in a snapshot or a test; `KOMOREBI_OFFLINE` starts none),
  the status line (`sync.rs`: what is said, its severity, its action, 5 s news and 12 s errors,
  standing lines), the Undo send countdown and the outbox drain. `status.rs` draws the line as a
  notice under the list pills (mockup `error`), with its action and a Close pill. The account
  caution badge on the folder pane and the rail, the Loader and Refresh (F5) in the list header,
  and the Outbox entry pane (Edit, Retry, Discard, Details) in `outbox.rs`.
- Tabs (phase 5): `views/tabs/strip.rs` (pill tabs from two tabs on, close button, middle click,
  the tab menu), `views/tabs/message.rs` (a message at full width: the command bar over its own
  reader, the "moved or deleted" notice), `views/tabs/mod.rs` (the `tabs` setting restored, titles),
  and `window/tab_host.rs`: open in a tab (reused when open, background on request), draft tabs
  with their own composer (Reply and Forward from a tab or a window), Ctrl+W and Ctrl+F4,
  Ctrl+Shift+T, Ctrl+2 to Ctrl+8, Ctrl+Shift+PageUp and PageDown, Shift+Enter and Ctrl+O on the
  list, the setting saved on every change when `reopen_tabs` is on and restored after the first
  frame, tabs marked gone on a cache change or a sync.
- Message windows (phase 5): `views/windows/popout.rs`, the app-drawn title bar over a message
  view; its actions go to the main window through its handle (Archive is the list's, with its
  Undo, and closes the window; Reply opens a draft tab in front); Esc closes it; the main window
  closes them as it closes.
- Tray (phase 11): `tray.rs`. The icon after the first frame (never in a snapshot or a test),
  its menu (Open Komorebi, Check mail, Pause notifications, Quit) in the language shown, the
  unread tooltip and dot, new-mail toasts through `komorebi::notify` when the unread count rises
  and the window is not in front, toast clicks open the message in a tab, close to tray
  (`on_window_should_close` hides the HWND), Quit really exits. The shell's events reach GPUI
  through a channel (the handler runs inside the tray window's procedure).
- Updater (phase 12): `views/updates/mod.rs`, the `Updater` state (the Slint app's `Updates`),
  the once-a-day automatic check 8 s after start (never offline, in a snapshot or a test), the
  check and the verified download on short-lived threads, the installer then Quit, and the quiet
  "Version X is available" bar (Download and install, Later, Release notes, hide until
  tomorrow). `whats_new.rs`: the What's new bar on the status line 2 s after start, saved once
  closed or acted on, back next start when replaced.

### Decisions

- The tab host is a child module of `window.rs` (`window/tab_host.rs`) so it keeps the window's
  fields private.
- A message window acts on the main window after its event, through `AsyncApp::update_window`:
  the main window may be drawing when the event comes.
- Hide to tray uses `ShowWindow(SW_HIDE)` on GPUI's HWND: GPUI has no window hide on Windows.
- The tray, notification and update settings are read again each time from the cache, so the
  Settings lane only writes them.
- The new strings reuse the Slint catalog entries word for word (`ui/tr.slint`, `ui/app.slint`);
  no new catalog entry was needed.

### Checks

`cargo xtask check` (GPUI tests, library tests, lint for tokens and timers; the size check is
skipped without a release build, which this lane does not make), `cargo fmt --all -- --check`,
`cargo clippy -p komorebi-gpui --all-targets -- -D warnings`. The app was not launched.

### Not done

- The list's double click and middle click on a row, and the row menu's Open in new tab and Open
  in new window, are still off: rows.rs and menus.rs are the list lane's (integrator request).
  Shift+Enter and Ctrl+O work.
- Undo of an Archive from a tab does not reopen the tab at its index; a draft tab's title does not
  follow its subject; composer windows (a draft in a window of its own), a pane draft moving to a
  tab when another message is selected, the scroll and view flags per tab.
- Ports left: 77, 101 to 104, 295, 302, 304, 305, 308 to 310, 313 to 315; the teaching tips the
  What's new bar waits for and the first account saving the version are the onboarding lane's.
- The `mailto:` handed over by a second launch only shows the window (no prefilled composer
  yet); the tray menu is not relabelled on a theme change (it has no theme).
- Settings > About (the update state, Check for updates, the automatic switch) is the settings
  lane's: it reads `MainWindow::updater()`.

## Phases 6 and 7: settings and onboarding (done on gpui/c-settings, with gaps)

Branch `gpui/c-settings` (from `main` 5b8bc50), the settings lane of stage C. The first commit
(the plain rules: choices, form, hosts, pins, roles, text, wipe) came from an earlier run that
stopped; this run reviewed it, kept it, and finished both phases.

### What was done

1. **Settings** (`views::settings::SettingsView::new(db, window, cx)`, mockup `settings`): the
   section list (a tab list, the mockup's pill with its dot; a wrapping row under 1000 px), the
   page column (20 px above, 32 px at the sides, 16 in a compact window), the page heading and
   lede, group headings (`.sg`), cards (`card::Card`, inset, 12 px, 64 px at least, 760 px at
   most) and the segmented control (`card::Segmented`, a radio group). Sections: Accounts,
   Signatures, Rules (shown once the data lane's view is mounted, `set_rules`), General,
   Privacy, About. Esc steps back: an open signature, an account's page, Add account, then
   `SettingsEvent::Close`.
2. **Accounts:** one card per account (its sender spot, its name, address and the line the
   shell reports through `set_account_state`: syncing, checked at, unreachable, failed, the
   sign-in or security latch, the first download), with the fix as a button (Update password,
   Review certificate) and Add account. **An account's page** (`editor::AccountEditor`): Sync
   this account, its notifications, Name, the address (read only), Password (empty keeps the
   saved one), the two default signatures and Manage signatures, Folders (the Sent copy and the
   five role overrides with Automatic), Server settings (`servers::ServerForm`: host, port,
   security that moves only a default port, sign-in names, Same sign-in as incoming), the
   certificate lines with Forget certificate, Download again and Remove (both asked first;
   Remove names the unsent mail), Save, Test connection and Cancel. Save checks the servers
   first only when how they are reached changed or a password was typed; the steps list and
   the answer (an error notice with Set username or Review certificate) scroll into view. The
   certificate question (`trust::open`) is shared with the setup flow.
3. **Signatures** (`signatures::SignaturesPage`): the list (Edit, Delete asked first with the
   accounts that lose it), Add signature, and a signature's page: its name, its text in the
   composer's own `RichEditor` with Bold, Italic, Underline and the two lists, and one logo
   (PNG or JPEG, 200 KB at most, through the file dialog).
4. **General:** Theme (segmented, applied to every window at once), Language (applied live),
   Check for new mail, Undo send, Mark as read, the notification switches, Keep running in the
   notification area, Start with Windows (the Run key), Reopen tabs, Message format.
   **Privacy:** the senders whose images load (Remove, Forget all), the diagnostics log and its
   folder, the mail cache's encryption (asked first; Restart now when it changes at the next
   start, `SettingsEvent::RestartNow`), Remove all data (every password first; then
   `SettingsEvent::WipeAndQuit`). **About:** the version, the shell's updater view in its slot
   (`set_updates`) with the automatic check's switch, What's new, Show tips again, Keyboard
   shortcuts (the dialog, `shortcuts::open`), Get help, Report a problem, the privacy
   statement, the data folder (Copy, Open) and the source. Every web page goes through the
   shell's link prompt (`SettingsEvent::OpenLink`).
5. **Onboarding** (`views::onboarding::OnboardingView::new(db, window, cx)` for the first run,
   `adding` for Settings > Accounts > Add account; mockup `welcome`): the welcome (the seal,
   the 40/48 display heading, the lede, the ink call to action, "It takes about a minute.", the
   mark's K falling off the right edge drawn with `PathBuilder` in the fall colour, Need help?,
   Privacy and the language at the foot), then Your email (refuses what isn't an address and an
   account already here), Sign in (the chip: a preset's settings, the lookup running, found,
   not found with the provider choice; manual setup; offline goes straight to manual setup
   with its note; the password and, after a rejected one, the username), the wait in three
   stages with the 3 s and 10 s reassurance, a failure as one title, one sentence and one next
   step (Set username, Review certificate, Try again, Set up manually) with Show details, Make
   it yours (the name started from the address, the signature, the From preview; Skip writes
   nothing) and You're all set (the first download as the shell reports it: `set_syncing`,
   `set_progress`, `set_has_mail`; Open your inbox, Add another account).
6. **Services** (`settings::services::Services`, a GPUI global): the check, the saved
   password's read, the password store, the certificate read and the lookup. The app uses the
   real ones (offline under `KOMOREBI_OFFLINE`); test builds default to fakes that refuse, so
   no test can reach the network or Windows Credential Manager.
7. **Tests:** 58 lib tests in the two folders (the plain rules, and view tests in
   `views::settings::tests` and `views::onboarding::tests`); docs/GPUI-TEST-PORT.md has a status
   on every phase 6 and 7 row (44 and 16).
8. **A development snapshot:** `examples/settings_shot.rs` (as `composer_shot.rs`: refuses
   without `KOMOREBI_OFFLINE`, a throwaway `KOMOREBI_DB` and `KOMOREBI_SNAPSHOT`), with
   `KOMOREBI_SETTINGS_STATE=accounts|account|signatures|general|privacy|about|welcome|email|sign-in|manual`.

### Decisions

- **No calendar in 1.0:** no calendar section, field, reminder choice or onboarding step was
  ported (General drops "event reminders"; the Slint app's calendar settings stay there).
- **Add account is the setup flow,** inside Settings, from Your email; the account page edits
  existing accounts only (the Slint app's add form is not ported).
- **The welcome follows the mockup's layout with the Slint app's words:** the mockup's "A quiet
  place for your mail." has no translation yet, so the heading, lede and button are the
  translated Slint strings; the three value rows of the Slint welcome are left out, as the
  mockup has none. The call to action is the 40 px button height (the mockup's 44 px is not a
  button token).
- **Theme is the segmented control** of the mockup, not a select.
- **The cache's encryption** follows the Slint app (`cachecrypt::set_intent`, applied at the
  next start); the switch on is asked first with the Slint wording.
- **Typed lines:** every error or result line holds a typed value (`editor::Line`,
  `Message`, `SigError`) and is worded at render, so a language switch rewords it.
- Two Slint rows are not applicable (229 accent contrast: the brand has no accent; 230 account
  dots: accounts show their sender spot) and one (150) is Slint-only.

### Checks

`cargo xtask check` (CARGO_TARGET_DIR `C:\dev-overflow\cargo-targets\komorebi-settings`): fmt, clippy `-D warnings` over the workspace, the GPUI app's 213 lib tests (1 ignored) and its integration tests, the library's 797 (1 ignored), xtask's 9, both lints: ok; the size check skipped (no release build). `cargo test --test i18n_catalog`: 8 of 9; `template_is_current` fails until the integrator regenerates the template (every new msgid has its pt-PT entry, `catalogs_are_complete` passes).

Snapshots (`examples/settings_shot.rs`, offline, throwaway caches in the session scratchpad, 1280 x 800, European Portuguese from the system): the welcome, Accounts, an account's page, General and Sign in, looked at against the `settings` and `welcome` mockups: the section list's pill and dot, the 760 px cards, the segmented Theme, the welcome's seal, heading, lede, ink button and the falling K match; cards without an icon start their title at the card's edge where the mockup has none either, and the foot's links carry the subtle button's padding.

### Not done in phases 6 and 7, or left for others

- **Mounting (integrator, `window.rs`):** show `SettingsView` in the content area (Ctrl+, and
  the folder pane's Account settings, `edit_account`), and `OnboardingView::new` on a first run
  with no account; wire `SettingsEvent` (workers on AccountAdded, AccountSaved's `Change`,
  AccountEnabled, Redownload, AccountRemoved; OpenLink to the link prompt; CheckForUpdates;
  ShowTips; RestartNow; WipeAndQuit then `wipe::remove_files` after the cache is closed;
  `Changed` for the timer, undo, notifications and the tray) and `OnboardingEvent`; forward
  account states (`set_account_state`) and the first download (`OnboardingView::set_syncing`,
  `set_progress`, `set_has_mail`); mount the updater (`set_updates`) and rules (`set_rules`)
  views; call `views::settings::bind_keys` in `main.rs` and `test_support::init_app`.
- Snapshot actions (`snapshot.rs`) for the settings sections and the flow's states; the
  example stands in until then.
- Rows 175 and 176 (the teaching tips, the shell's); the account page's compose defaults
  (font, size, colour) are not on a settings page yet.
- The template (`translations/komorebi.pot`) is regenerated by the integrator: the catalog's
  `template_is_current` fails on this branch until then (new msgids, all with their pt-PT
  entries).
- By hand: Narrator over the forms, the trust dialog and the flow.

## Phases 8 and 9: rules, contacts and CardDAV sync (done on gpui/c-data, with gaps)

Branch `gpui/c-data` from `main` at 5b8bc50. The views are built and tested on their own; the
window does not mount them yet (integration items below).

### What was done

- Library moves, with the Slint app calling the moved code (its tests stay green):
  `komorebi::rules_page` (the Rules page: accounts, cards with their summary parts, the
  activity log, folder choices, toggle, reorder, delete, Copy as Sieve, and the rule `Editor`
  with its `Problem`s), `komorebi::contacts_page` (the list, groups, search, the contact shown,
  the contact `Form` with its checks, Write to, keep mine or the server's after a conflict) and
  `komorebi::contacts_sync` (the CardDAV settings form, the books list and its status, Find,
  Find address books, the sync plan and pass, `due`). `src/app/rules.rs`, `contacts.rs` and
  `contacts_sync.rs` now hold only the Slint wiring.
- `views::rules::RulesView::new(db, window, cx)`: the title in the large step with its sentence,
  Copy as Sieve and New rule; the account picker with two accounts or more; "N rules, in this
  order" (the mockup's line); a card per rule on inset (name, summary, the paused line with the
  warning glyph, its On/Off switch named by the rule, and a "..." menu with Edit, Move up, Move
  down and Delete); the activity of automatic runs with Clear activity. The editor page: Back,
  name, All/Any, condition rows (field, operator, value, remove; Has attachment needs no
  value), Add condition up to the limit, the actions (move to a folder from an indented folder
  list, delete, mark read, important, stop), the problem notice, Save and Cancel; Esc cancels.
  `RulesEvent::{SieveCopied, Failed}` for the status line.
- `views::contacts::ContactsView::new(db, window, cx)`, with `opened` and `closed` for the
  shell: a list pane (title and count, New contact, a "..." menu with Address book sync, the
  search field that hands Down and Enter to the list, the group picker, a virtual list of rows
  with their spots, one tab stop with Up, Down, Home, End, Page Up, Page Down and Enter) and a
  side pane: the contact (spot, name, organisation, New message, read-only and conflict
  notices with Keep mine and Keep the server's, email lines with Write to and Copy, phones with
  Copy, groups, notes, address book), the editor (name, given and family, organisation, job
  title, emails and phones added and removed, groups, notes; Save and Cancel stay in view below
  the scrolling form; the field at fault takes the focus; Cancel asks before throwing changes
  away) and the sync pane (account, mode with the questions the Slint app asks when turning it
  off, server address with Find, user name, same password or its own, Find address books, Sync
  now, the books with their switches, the status). Below `RAIL_W + 2 * LIST_W_COMPACT` the
  panes show one at a time with Back. Network work runs on a thread and comes back over a
  oneshot; `timer_tick` runs a due pass. `ContactsEvent::{Compose, NoAccount, Copied, Failed}`.
- Icons `Edit`, `Copy` and `PersonAdd` (Fluent 20 regular, drawn at 16); NOTICE counts 62.
- New strings: one, "{n} rule, in this order" (plural), in pt-PT as "{n} regra, por esta
  ordem". Everything else uses the Slint app's msgids and contexts.
- `examples/data_shot.rs`: a development snapshot of the five screens
  (`KOMOREBI_DATA_STATE=rules|rule-edit|contacts|contact-edit|contacts-sync`) with the app's
  snapshot safety (offline, throwaway cache), until the window has snapshot actions for them.

### Decisions

- The rule editor is a page in place of the list, as the Slint app's, not the mockup's inline
  condition row: a rule has up to the condition limit and five actions, which do not fit one
  row. The mockup's drag handles are left out until drag reorder exists (Move up and Move down
  are in the menu); a handle that does not drag would mislead.
- The contact's "Invite" action in the mockup is left out (no calendar in 1.0). Its "Write" is
  "New message", the Slint app's word.
- Keys are `on_key_down` handlers on the view, not actions, so the window needs no
  `bind_keys` for these views.
- The two-answer question (turn sync off: keep or remove the synced contacts) is a Kit dialog
  local to the contacts view, where Esc is the second answer, because `widgets/dialog.rs` is
  not this lane's.
- Text is kept typed in state (problems, notes, log kinds) and worded at render, so a language
  change rewords what is on screen.

### Checks

`CARGO_TARGET_DIR` `C:\dev-overflow\cargo-targets\komorebi-data`. `cargo xtask check` passes
(fmt, clippy over the workspace on all targets with `-D warnings`, komorebi-gpui 164 passed and
1 ignored, library 816 passed and 1 ignored, xtask 9 passed, the token and timer lints; the
size check skipped without a release build). The Slint app's tests (`cargo test --bin
komorebi`) passed 210, 1 ignored, after the library move. The new view
tests: rules 2 (one is the row 291 port) and contacts 3, plus the text tests for both
languages. Snapshots of the five screens in light and dark, at 1280 x 800 and 900 x 700, in
English and pt-PT, compared with the brand's `rules-*` and `contacts-*` mockups (only in a
scratch folder, not committed). `tests/i18n_catalog.rs` `template_is_current` fails on this
branch until the integrator regenerates `komorebi.pot` (the one new msgid); the pt-PT entry is
in the catalog already.

### Not done in phases 8 and 9

- Mounting: the Rules view in Settings > Rules, the Contacts view on the Contacts place, the
  events in the status line, Compose opening the composer, `timer_tick` on the mail timer,
  snapshot actions in the window (integration items).
- The mockup's letter headers in the contacts list (`uniform_list` needs one row height; a
  sticky header or a list with two heights is the work).
- Drag reorder of rules, access keys, a Narrator pass by hand (phase 13).
- The bundled icons' `@fluentui/svg-icons` version is not checked against 1.1.292; they came
  from a local Fluent copy.
- The spot's initials at the large size are the shared widget's size, smaller than the
  mockup's.

## Stage C integration (done on main)

`gpui/c-shell`, `gpui/c-settings` and `gpui/c-data` are merged with `--no-ff` in that order (the
calendar lane does not exist: phase 10 is dropped). The merges conflicted only in the shared
append-only files (`views/mod.rs`, this page, `GPUI-TEST-PORT.md`, `komorebi.po`), resolved as
unions; where two lanes marked the same test row, the more complete status was kept.

### What was done

1. **Mounting.** `window/places.rs` replaces the stubs of `views/windows/mounts.rs` (deleted).
   Settings and Contacts show beside the folder pane or rail in place of the list and the reading
   pane (the mockup's `main two`) through `MailView::set_place`; choosing a folder, the Inbox or
   New mail takes them away (`MailEvent::ClosePlace`), the foot's button toggles them, Esc leaves
   Settings, and Ctrl+, toggles it. The first-run onboarding flow (no account in the cache) takes
   the whole body; Settings > Add account runs the same flow inside Settings. Rules is Settings'
   own section (`SettingsView::set_rules` now takes the `RulesView` and reloads it as the section
   opens). Settings > About gets the update card (`views/updates/card.rs`: state, Check for
   updates, progress, Release notes, Download and install or Try again, Later) over the window's
   `Updater`.
2. **Settings events.** Account added (workers and first download start; the first account saves
   the What's new version silently), saved (servers changed: new workers; only the password
   changed: the sign-in latch lifts), switched on or off, Download again (the cache resets, new
   workers, a status line), removed, accounts changed (the pane, lists, composer, reader and tabs
   read the cache again), links (a confirm first, https only), check for updates, Show tips again
   (the setting is reset; the tips themselves are not in the GPUI app yet), Restart now (a new
   copy with `KOMOREBI_RESTART_AFTER`, then quit), Remove all data (the app stops; `main` removes
   the files once the cache is closed and says in a native box what is left). The changed
   settings reach their users: the check interval (the hub now reads it each round), undo send,
   mark as read and compose format (`MailView::reload_settings`), automatic updates, the
   diagnostics log (also switched on at start); the tray and toasts read theirs from the cache.
3. **Worker state into the views.** The hub's per-account state feeds Settings' account cards, and
   the first download of the account the flow added feeds the flow (syncing, count, mail in the
   Inbox). The finish screen lands on that account's Inbox.
4. **Integrator requests of the list lane.** A double click on a row opens it in a tab, a middle
   click in a background tab, and the row menu's Open in new tab and Open in new window work;
   "Account settings" (the pane, the row menu) and the status line's fixes open that account's
   page; the What's new bar's button opens the changelog behind the link prompt; a `mailto:` link
   in the reader and Write to in Contacts open the composer with the fields filled
   (`MailView::compose_mailto`).
5. **Contacts and rules.** The window owns one `ContactsView` (empty until it shows); the hub's
   check timer emits `HubEvent::Tick`, which runs the CardDAV passes that are due. Contacts'
   Compose, NoAccount, Copied and Failed, and Rules' SieveCopied and Failed, go to the status
   line.
6. **Calendar entry points.** Nothing of the data, settings or shell lanes touched the calendar.
   In the reader's invite card (stage B) the "In your calendar on this PC." line and "Remove from
   calendar" are gone; the card stays a display of the invitation with Save .ics, Details and
   Join.
7. **Snapshot actions** for the new screens (`KOMOREBI_SNAPSHOT_ACTION=settings-accounts`,
   `settings-account`, `settings-signatures`, `settings-rules`, `settings-general`,
   `settings-privacy`, `settings-about`, `rule-edit`, `contacts`, `contact-edit`,
   `contacts-sync`, `ob-welcome`, `ob-email`, `ob-sign-in`, `ob-manual`), replacing the lanes'
   `settings_shot` and `data_shot` examples.
8. **Shared files.** `Cargo.lock` merged without conflict (checked with `--locked`);
   `translations/komorebi.pot` regenerated once, here, for the new msgids of all three lanes and
   the update card.

### Checks

On the merged tree (`CARGO_TARGET_DIR` `komorebi-integ`): `cargo fmt --all -- --check`, `cargo clippy
--workspace --all-targets --locked -- -D warnings` and `cargo xtask check` pass (komorebi-gpui 260
passed, 1 ignored; library 816 passed, 1 ignored; xtask 9; lint ok; size skipped, no release
build). The full `cargo test --locked` passes too (Slint bin 210 passed, 1 ignored); the Slint
test `the_outbox_offers_edit_retry_and_discard` failed once under load and passed alone and in the
next full run. `cargo deny --workspace` passes licences, bans and sources but fails advisories on
three RUSTSEC entries that were already in the lock file before the merge (pin-utils unmaintained,
pulldown-cmark 0.13.4 twice); the lock file gained only `idna` and `native-tls` lines.
`scripts/check-pins.ps1` could not run: this machine's execution policy blocks scripts and the
rules forbid changing it.

### Not done (stage C)

- The RSVP question for Accept, Tentative and Decline on the invite card: `ReaderEvent::AskRsvp`
  has no owner yet (nothing answers it), and `komorebi::invite::record` still stores an accepted
  invitation in the library's local calendar tables, which nothing shows. Both go with the stage D
  parity fixes (the library part can drop the calendar store once the Slint app is gone).
- The teaching tips (rows 175 and 176 of the test port) and the call to hold the What's new bar
  back while one shows.
- A second launch's `mailto:` only shows the window; no prefilled composer yet.
- Composer windows, a dirty pane draft moving to a tab, per-tab scroll position, the draft tab's
  title following its subject, undo of an Archive from a tab (the shell lane's list).
- Close to tray, the tray and the toasts, Restart now and Remove all data, and the real network
  paths of the onboarding flow and the updater have not been run by hand (the rules forbid
  launching the app without the offline snapshot switches).
- Letter group headers in the Contacts list, drag reorder of rules, the Narrator pass and access
  keys (phase 13), and the other gaps in the lanes' own sections above.
- No release build, so the 21,500,000-byte exe ceiling was not measured on the merged tree.

## Stage D: library (done on gpui/d-library)

The stage D fix lane for the library and the shared test harness. Calendar is dropped, so the
library part of its entry points is cut down here; nothing was dropped from the cache.

### What was fixed

1. **cargo deny.** `pulldown-cmark` 0.13.4 to 0.13.6 (RUSTSEC-2026-0414 and -0433; it comes from
   `i-slint-common`, not from Komorebi's code, so `cargo update -p pulldown-cmark` was the whole
   fix) and `async-executor` 1.14.0 to 1.14.1 (yanked since the list above was made).
   `pin-utils` (RUSTSEC-2026-0441, unmaintained, no vulnerability) is pulled in by `spin_on`,
   which `slint-build` and `slint-macros` 1.18.1 use; nothing of ours depends on it, so it is an
   `ignore` entry in `deny.toml` with that reason, and it goes with the Slint app at the cut-over.
   `cargo deny --workspace --locked check licenses advisories bans sources` passes.
2. **Flaky tests under load, both with their cause found.**
   - GPUI `deleting_from_trash_or_junk_asks_first` failed in about half of the full runs under
     load (three different messages: two `dialog-confirm` buttons, and `held()` empty after the
     confirm). The cause is not in the mail view: the main window reads Windows' appearance
     settings again whenever it becomes active (`theme::feeds::refresh_from_windows`, correct in
     the app), and in a test that was this machine's: animation effects on, so `reduce_motion`
     turned off right after `init_app` set it, and Kit's dialog slid in over real time. A click
     then hit the button where the last frame had it, not where the animation put it, so Cancel
     sometimes did not close the dialog (the second question stacked on the first) and a confirm
     sometimes missed. `komorebi::theme::pin` fixes what `read()` answers for the process;
     `test_support::init_app` pins the tests' settings (animation off), and
     `test_support::tests::a_window_reading_windows_again_keeps_the_tests_settings` proves a
     re-read keeps them. Eight parallel full runs of the suite, four times each, failed 7 of 32
     before (all in this test) and 0 of 32 after.
   - Slint `the_outbox_offers_edit_retry_and_discard` (the "Komorebi stopped trying" sentence)
     read the wrong language: `komorebi::i18n::select` set the process-wide language, and the
     Slint tests that switch to Portuguese and back (`a_rejected_password_reads_by_provider`,
     `the_language_switches_live_and_is_saved`, `language_and_theme_switch_reach_popouts` and
     others) moved every other test's `tr!` while it ran. `select` now also keeps the language as
     the calling thread's own (`komorebi_i18n::set_thread_lang`); in the app only the UI thread
     selects, so both agree, and a test thread that has selected is no longer moved by another.
     Six parallel loops of the 11 language-related Slint tests, five runs each: 6 of 30 failed
     without the change (the "Komorebi stopped trying" assertion, and the Portuguese "Para:" line
     for the same reason), 0 of 30 with it.
3. **Invitations no longer reach the calendar tables on the GPUI path.**
   `komorebi::invite::record_answer` keeps the answer in the `rsvp` table and nothing else; the
   GPUI app's RSVP owner calls it. `record` (answer plus local calendar) stays for the Slint app
   and calls `record_answer` first. No table or data was dropped. A new library test proves the
   card shows the answer and `event_by_uid` stays empty.
4. **The attachment helpers are in the library.** `src/files.rs` had already moved
   (`komorebi::files`, phase 4); what was left was the sequence both apps need around it.
   `files::open_attachment` (refuse what runs code, write the marked copy, start the app; returns
   `Opened::{Started, Refused, NoApp, Failed}`) and `files::save_attachment` (write, Mark of the
   Web) are shared; the Slint app now calls them instead of its own copies. Tests use a fake
   launcher, so no app starts.
5. **Fluent icons checked against `@fluentui/svg-icons` 1.1.292** (the package was fetched with
   `npm pack` and each bundled file compared with its `16_regular` glyph, white space aside).
   59 of 62 were identical; `copy`, `edit` and `person_add` were 20 px glyphs of a different
   design (from a local copy, not from 1.1.292) and now are the package's 16 px regular ones.
   `icons::tests::notice_counts_the_bundled_fluent_icons` keeps NOTICE's count and version in
   step with the folder.
6. **Crash log moved to the library** (`komorebi::crash`; rows 15 and 16 are its tests, one made
   robust against other threads panicking). The Slint binary uses it from there.
7. **Test port rows.** 15, 16, 37 to 42 (names corrected: row 38 is
   `every_shown_string_is_translatable`), 240, 241 (covered by the catalog tests: the GPUI app has
   no `CALLS` table), 245 to 248 (in `crates/komorebi-gpui/src/text.rs`, over the sentence
   functions of the views). `views::reader::trust_notice` is now `pub(crate)` for row 246.

### Checks

`CARGO_TARGET_DIR` `komorebi-d-library`: `cargo xtask check` passes (fmt, clippy `-D warnings`,
komorebi-gpui 267 passed and 1 ignored, library 822 passed and 1 ignored, xtask 9, lint ok; size
skipped, no release build). The full `cargo test --locked` passes too (Slint bin 208 passed and 1
ignored: the two crash tests moved to the library). `cargo deny --workspace --locked check
licenses advisories bans sources` passes. One test of this lane's own (the attachment tests in
`files.rs`) shared a folder name with an older test and failed once in the first gate run; it uses
the module's `temp` helper with its own names now.

### Not done in stage D (library)

- Nothing in the GPUI app calls the new helpers yet (the files they need are other lanes'):
  `ReaderEvent::OpenAttachment`, `SaveAttachment` and `SaveInvite` have no owner in
  `views/mail/mod.rs` (`on_reader` ignores them; the save dialog needs `rfd`, which the GPUI
  package does not depend on yet), and `AskRsvp` has none. The GPUI binary does not install the
  crash log yet (`komorebi::crash::install` and the launch notice).
- `invite::card` still marks a stored copy cancelled (an `UPDATE` of an existing calendar row, for
  both apps) when a cancellation arrives; with no stored copy it writes nothing.
- `pin-utils` is ignored, not removed: it is Slint's.

## Stage D: shell (done on gpui/d-shell, with gaps)

The shell lane's fixes after stage C: start-up, tray and toasts, keys, tabs and windows, the title
bar, and the widget layer's accessibility. No calendar anywhere (owner decision).

### What was fixed

- **Start-up** (`main.rs`, `window/startup.rs`): the crash log hook (the Slint
  app's `src/crash.rs`, now `komorebi::crash` in the library; the shell lane's own copy in the GPUI crate was dropped at integration) and its "closed unexpectedly" notice once on the
  status line, standing until closed or replaced (the Slint app's `app::notice`, so a
  `--minimized` start still shows it); the log is marked seen only when a cache opened (the
  locked-cache window has no status line); a damaged cache set aside is said there too, after the
  crash notice, as in the Slint app; a cache this Windows user cannot
  unlock shows the locked-cache dialog over the empty window (Start a fresh cache, asked again,
  sets it aside and restarts; Quit and Escape quit); any other failure (a newer cache, a full
  disk) is said in a native error box with `text::start_failed` and the app exits 1 (no box in a
  snapshot). `widgets::dialog::confirm_or` runs its own Cancel and Escape.
- **Command line:** a `mailto:` argument opens the composer, filled (never sent), in the Mail
  tab; `--minimized` (Start with Windows) keeps the window hidden (`WindowOptions::show` off, not
  activated), starts the workers, the tray and the timers without a first frame, and shows the
  window when no tray icon could be made. A second launch's `mailto:` (`ShellEvent::Mailto`) now
  reaches `TrayEvent::Mailto`: the window comes back with the filled composer.
- **Self-update:** the app mutex is let go just before the installer starts (and taken back if it
  did not), as the Slint app does. The update bar's and the About card's Release notes go through
  the window's link prompt (`UpdateEvent::OpenLink`).
- **Tray and toasts:** new mail is looked for on every cache change and recount (by message id), so
  a message read in the same pass no longer hides a new one; the taskbar flashes
  (`FlashWindowEx`, `FLASHW_TRAY | FLASHW_TIMERNOFG`) when the setting is on, the window is behind
  and the count rose; native menus (the tray's) follow the app's light or dark
  (`komorebi::theme::set_menu_theme` in `theme::install`). The tray menu's labels already followed
  the language and have no theme of their own, so nothing else was needed there.
- **Keys:** F1 and Ctrl+/ open the Keyboard shortcuts dialog from anywhere (Ctrl+/ not in the
  composer); F6 and Shift+F6 go round the folders, the list and the reading pane (the composer or
  the open message) in the Mail tab; Ctrl+N from a message or draft tab shows the Mail tab first,
  so the composer is not hidden behind the tab (a mail link from a tab too).
- **Tabs and windows:** a draft tab's title follows its subject; a move from the list closes the
  messages' tabs and windows, and Undo (of a move from the list, or of Archive or Delete in a tab)
  brings the tabs back at their places (`tab_host::follow_held`, from the list's held move). As
  the Slint app's `reopen_message_tabs`, only the tab a move was made from comes back in front; a
  move from the list leaves the shown tab (the Mail tab) where it is. Message windows are not
  reopened: Undo says "Undo brought the message back; open it from the list." (`Status::
  UndoWindow`, the Slint app's `invoke_undo_window`); a
  `mailto:` link in a tab's or window's message opens the filled composer in the Mail tab.
- **Title bar:** the app-drawn bar only where the Slint app has it (`titlebar::app_drawn`:
  Windows 11, no contrast theme, no `KOMOREBI_SYSTEM_TITLEBAR`); elsewhere (Windows 10) the system
  caption, with our bar keeping only the search, and no strip of its own when there is nothing in
  its centre (no account yet, a message window). Message windows follow `app_drawn` too: that is
  the GPUI app's own choice (phase 5), not Slint parity, since the Slint app's message windows
  always use the system caption (`apply_popout_theme`).
- **First show after `--minimized`:** GPUI keeps a hidden window's saved place until its first
  activation, so the first show (tray, toast, mail link, the no-icon fallback) is the activation
  alone (`MainWindow::show_window`), not `ShowWindow` first, which would show the window at its
  creation place and then move it. Not checked on screen.
- **New-mail check cost:** the check on every hub change reads `MAX(id)` first and runs the
  unread-inbox query only when a message was stored since the last look (`tray::Notes`).
- **Reduce motion** in a snapshot now holds when the window's activation re-reads Windows'
  settings (`feeds::hold_still`).
- **Widgets (a11y and text size):** a dialog's content is a Group named by the title with the
  sentence as its description (`widgets::dialog::region`, also for dialogs built on Kit's
  directly), inside Kit's own host node, which is the one Dialog (gpui-base 0.8.0 sets
  `Role::Dialog` and takes the focus) but cannot be named through Kit's API; a confirm dialog does
  not close on a press outside it (its Cancel can be Quit, the locked cache); notices (the status line) and the Sent toast are live regions (polite, an
  error assertive); disabled buttons and icon buttons are exposed as disabled; the update
  progress has its value (0 to 100); buttons, text fields, chips, tabs, segments, the tab strip and
  the toast grow with Windows' text size (`scaled::control`).

### Tests

New: `window::tests::{f6_cycles_the_panes, f1_and_ctrl_slash_open_the_shortcuts,
a_mail_link_at_start_opens_the_composer, the_notices_of_the_last_run_are_said (now past 30 s),
the_damaged_cache_notice_stays_after_the_crash_notice,
a_hidden_start_starts_the_work_and_shows_without_an_icon,
the_title_bar_has_no_search_until_the_first_account}`, `window::startup::tests` (the locked
dialog, the error kinds, setting aside, the words, a minimized start), `window::tab_tests::
{archive_from_a_tab_closes_it (now with Undo, the tab in front),
a_move_from_the_list_closes_its_tab_and_window (Undo leaves the Mail tab in front and says the
window is not reopened),
a_draft_tab_follows_its_subject, a_tabs_mail_link_and_new_mail_go_to_the_mail_tab}`,
`tray::tests::{the_taskbar_flashes_for_a_rise_in_the_background,
a_read_and_a_new_message_in_one_pass_still_notify}`, `crash::tests` (rows 15 and 16),
`theme::feeds::tests`, `titlebar::tests::the_system_caption_bar_has_no_caption_buttons` (the
centre's button only, no strip without content; `app_drawn` is true under test, so the Windows 10
detection is covered only by `komorebi::chrome`'s tests), and the dialog test asserts the content
Group and at most one Dialog node. Rows marked in GPUI-TEST-PORT.md: 14 (obsolete), 15,
16, 112, 156 and 172 (their F6 halves), 298, 306. The AccessKit properties (live, disabled,
numeric value) cannot be read by the headless queries; they want the Narrator pass.

### Not done (stage D shell)

- Kit's dialog host node (the Dialog with the focus) has no name: the title is on the Group
  inside it. Naming the host needs a Kit change (gpui-base `Dialog` has no label setter); whether
  Narrator says the title as a dialog opens is unverified until the Narrator pass. A destructive
  confirm is not an AlertDialog node (Kit's `open_alert_dialog` has its own fixed layout).
- The first show after `--minimized` (saved place, maximized) and the system caption on Windows 10
  have not been looked at on screen.

- Tab cannot reach the message list, the folders, the reader page, the composer body or the
  contacts list: their focus handles are made without `.tab_stop(true)` in files of the mail,
  reader, composer and data lanes (integrator request). F6 now reaches the panes. The reader's
  handle is also focused while its page is not drawn (a body still downloading), and then no key
  reaches the window.
- Composer windows (rows 101 to 104; `PopoutKind::Draft` is still unused, and the tab menu's Open
  in new window does nothing on a draft tab), a dirty pane draft moving to a tab when another
  message is selected (rows 77 and 302: the mail view has to hand the draft over), the scroll and
  view flags per tab (295), the autosave flush on a tab switch (304), Undo send reopening in a new
  tab (305), `save_all` on quit (100), rows 303, 308 to 315, 107's countdown, 124 (outbox
  recovery at start), 157, 171, 174 and 7 (the wall-clock wake check is not in the hub yet).
- The teaching tips (rows 175 and 176) and holding the What's new bar back while one shows.
- The message tab and window command set is still Reply, Reply all, Forward, Archive, Delete and
  Open in new window: Move, Junk, Flag, Read, Forward as attachment and More need `Quick` and
  `MessageEvent` variants in the list lane's `triage.rs`. A tab's reader events other than the
  hub's, Mark read, Delete, Privacy and mail links (Junk, Why, attachments, the invite file, a
  server draft's Edit) are still dropped, as they are in the mail view itself.
- Message windows do not remember their size or cascade; tabs cannot be dragged to reorder.
- Roving tab stops and arrow keys in tab lists, toolbar names with Left and Right, tab close
  buttons and the tab menu from the keyboard, the Dropdown as a combo box with its expanded state,
  UIA accelerator keys on commands, and the onboarding progress value (the settings lane's file).
- Visual checks at 150 and 225 percent text size, menus and tooltips in a shot, and the owner
  review points of phase 1 (the menu's edge, the dark dialog, the placeholder tone) need the app
  on screen, which this lane does not launch.

## Stage D: mail (done on gpui/d-mail, with gaps)

The mail lane's fix list from the stage C review, in `crates/komorebi-gpui/src/views/mail/`.

### What was fixed

- **Keys.** The arrows, Shift+arrows and Ctrl+A also move through the list from the reading
  pane, which now has its own focus (`reading_focus`); Home, End, Page Up and Page Down are left
  to the page, as in the Slint app, where they scroll the message. F6 and Shift+F6 go round
  the panes that show (search, folders, list, reading; `mod.rs` `next_stop`), skipping a pane
  that is hidden and doing nothing while a dialog is open. Ctrl+S saves the open message as
  .eml.
- **Rows.** Double and middle clicks open tabs only from message rows (never Drafts or the
  Outbox, tested by double and middle click); high and low importance marks; a row's name says its tag once,
  "(no subject)" for an empty subject and "Needs attention" for a problem. Rows drag onto a
  folder line or rail tile to move there (`MailView::drop_on_folder`); files dropped on the view
  start a new message with them attached (`drop_files`, with a drop overlay).
- **Folder pane.** Tree items carry their level, position and set size; the footer's place
  buttons are toggles; an account's caution mark is a button named Account settings that opens
  the account (from the keyboard the line's menu has the same command); Refresh on an account
  line syncs that account only (`refresh_account`, tested with two accounts); an account whose
  sign-in was rejected says so. A row dropped on a folder it cannot move to keeps the selection.
- **Reading pane.** The selection panel has the Slint commands (Archive, Delete, Move with the
  reason it is off, Read or Unread, Flag or Unflag, Select all, Clear). One-pane mode leads the
  command bar with Back to list (Esc) and keeps the status line and the undo offer.
- **List.** The empty state names the filter or search, shows the first download's progress
  (a bar with its value once the count is known, none before), the search hint and Show all; "Updated 14:05" beside Refresh when there is room; the results
  line offers "This folder" back from an all-folders search; `search::scope_box` is the Slint
  ScopeBox (the window places it).
- **Source commands** (merged at integration into the reader lane's `views/reader/actions.rs`; the mail lane's own `original.rs` was dropped, its test now goes through the reader): Save as .eml, Show original (header fields and source,
  Copy Message-ID, Copy headers, Copy all) and Copy Message-ID from the row and More menus; a body
  not downloaded yet is fetched first and the command runs when it arrives. The .eml is written
  off the UI thread, and a command reads the message once.
- **Drafts and the Outbox.** The Drafts menu is Open and Discard, Delete discards the draft.
  Sends cut short by the last exit go out at start (`resume_outbox`, the Slint port). The undo
  offer names the subject and reads "Sending ..." at 0 s; its seconds round up as the Slint
  `count_down` does, so a late timer neither skips a second nor ends the offer early.
- **Tests.** 14 window tests in `views/mail/tests.rs` and 5 plain tests; `deleting_from_trash_or_junk_asks_first`
  clicks in the newest dialog (a closing one stays in the layer and made it flaky). Test rows
  4, 5, 75, 92, 96, 124, 128, 130, 140, 141, 168, 293 and 294 done; 109 and 170 partly.

### What remains

- Enter does not move the focus into the reader's page: the reader's Enter (FollowLink) and Esc
  (ClearSelection) do not propagate when they have nothing to do (reader lane).
- F6 from the title bar's search and the search scope box need the window (`window.rs`):
  routing NextPane and PreviousPane to `MailView::cycle_pane`, placing `search_scope_box`.
- The reader bar's labels by priority (row 160), Tab order and the quick actions as no Tab stops
  (row 164, phase 13), the filters' Backspace scope (row 163), Outbox Edit (row 109) and the
  focus back from Show original (row 173) are not ported or tested.
- The reader's page does not scroll on Home, End, Page Up or Page Down yet (it binds none of
  them; reader lane). The caution mark has no tab stop of its own (the tree is one stop with an
  active descendant). Copy Message-ID still reads the whole message, raw bytes included, as the
  library has no Message-ID accessor. The rail tile's caution mark is still passive; drag and drop was tested through
  `drop_on_folder` only, not by hand.
- The new msgid "Needs attention" is in `komorebi.po`; the .pot is not regenerated here.

## Stage D: reader (fix lane `gpui/d-reader`)

Branch `gpui/d-reader` from `main` adee939: the reader's gaps from phase 3 and stage C, and the
reader items of the parity audit.

### What was fixed

1. **The reader acts for itself** (`views/reader/actions.rs`), so the reading pane, a message
   tab and a message window behave alike with no owner code: an attachment pill opens a copy in
   its default app (beside the cache, Mark of the Web; types that run code are refused with the
   Slint sentence), its save button saves it through the system's save dialog (offered in the
   Downloads Known Folder, `dirs::download_dir`, as the Slint app; marked as downloaded); Save .ics on the invite card saves `invite.ics`; the
   sender notice's Why? opens "Why am I seeing this?". `ReaderEvent::OpenAttachment`,
   `SaveAttachment`, `SaveInvite`, `WhySender` and `AskRsvp` are gone.
2. **RSVP:** Accept, Tentative and Decline ask the Slint question (who the reply goes to,
   exactly; the domain, unlisted and alias warnings) with Send reply (Enter), Don't send a
   reply and Cancel. Send reply queues the iTIP reply through the normal outbox path
   (`invite::queue_reply`, then `ReaderEvent::ReplyQueued`, which the hub dispatches); the
   answer is kept for the card only with `komorebi::invite::record_answer` from the library lane (the `rsvp`
   table, no calendar store). The card's Remove from calendar press does nothing (never shown).
   The Slint app keeps `invite::record` with its calendar.
3. **Source commands:** Save as .eml (also Ctrl+S anywhere in the reader), Show original (a
   dialog with the Message-ID, the header fields and the source in read-only fields, Copy
   Message-ID, Copy headers, Copy all and Close; the focus goes back on close) and Copy
   Message-ID, over `komorebi::mail::source` (the Slint `original.rs` logic, which was already in
   the library). A body that is not downloaded yet is asked for, "Fetching message" shows after
   a second, and the command runs when it arrives (`ReaderView::body_failed` says why it did
   not). The wait belongs to its message: opening another or closing the reader drops it. The row menu's entries are enabled for one message.
4. **More menu** (`menus::more_entries`, the Slint `more_menu`): Open in new window, Forward as
   attachment; Report junk or Not junk; View as plain text and Show original colours
   (checkable, both ways), Load images or Stop loading images from this sender; the source
   commands; Show in folder while searching.
5. **Command bar** (`views/mail/reading.rs`): Reply, Reply all, Forward; Archive, Delete, Move
   (a menu button with its chevron, expanded for UI Automation while the Move flyout is open); Mark as read or unread, Mark as important or Remove importance (toggled);
   More. On a server draft Reply and Reply all go and Forward reads "Forward as new draft". The
   tooltips carry the keys, as the Slint bar's.
6. **Keyboard:** Up and Down (three lines), PageDown and PageUp (90 percent of the view), Home
   and End scroll the reader wherever the focus is in it (`PANE_CONTEXT`), the text view's field
   too (its own Move actions are captured on the pane); Space and Shift+Space page on the page
   itself (not on a button, which Space presses).
7. **Text view** is a read-only Kit `Textarea` (auto-grow): a Tab stop, selectable with the mouse
   and the keyboard, Ctrl+A and Ctrl+C; the reader's focus handle is the field while the text
   view shows. The field is set again when the message or the text (hashed) changes.
8. **Accessibility:** the link Tab reached is the page's active descendant for UI Automation;
   the page draws the focus ring while no link carries one.
9. **"You started a reply to this message"** with Resume shows while a reply to the open message
   is saved in Drafts (`Db::draft_for_message`), not on a server draft (as the Slint bar), and
   only once the owner calls `ReaderView::handle_resume` (no Resume that does nothing); Resume
   emits `ReaderEvent::ResumeReply`. No owner calls it yet (Not done).
10. `views/reader/format.rs` already re-exported `komorebi::format` (the list lane's move); only
    the `FileKind` type is left in it. Nothing to replace.

Tests: 9 new reader view tests (attachments, Why?, rows 140, 141 and 173, the page keys, the
text view, the view toggles, the draft-reply bar), the invite test now through the dialog, a
command bar and More menu test in `views::mail::reading`.
GPUI-TEST-PORT.md rows 139, 140, 141 and 173 updated, 154 noted.

### Not done (stage D reader), with why

- **Integrator wiring** (files of the mail and shell lanes): `ReaderEvent::ResumeReply` needs a
  handler in `MailView::on_reader` and `TabHost::tab_reader` (open `Draft::Saved(draft)`), and
  then `reader.handle_resume(cx)` where each reader is made, so the bar shows; `EditServerDraft`
  has no handler (the composer opens a saved draft with `Draft::Saved`; a
  server draft's Edit needs a composer path that copies the server draft, which does not exist
  yet); `Reaction::BodyFailed` should call `ReaderView::body_failed` (views/mail/mod.rs `react`);
  `TabHost::tab_reader` drops `Compose` (`mailto:`) and `Junk`; `OpenPrivacy` opens Settings on
  its default section, not Privacy (`window/places.rs`); Ctrl+S from the list (the
  `MESSAGES` context) needs a mail action that calls `source_command`.
- **Command bar:** one Tab stop with Left and Right along it (row 154) needs a roving tab stop on
  the shared `widgets::button` buttons; labels dropped as the width shrinks are moot in the
  mockup's bar (Reply is the only label).
- **Text view:** Backspace does not archive from the read-only field (Kit's input takes the key).
- **A save glyph** for attachments, and the Slint move glyph (MDL2 E8DE) for Move: the icon set
  (`icons.rs`, shared) has neither; the document and folder glyphs stay.
- **The save dialog's title** ("Save attachment"): GPUI's `prompt_for_new_path` takes no title.
- **UIA text selection** of the page's lines (TextRun nodes with character lengths and
  `set_text_selection`) is not built; the page's selection is on the worker's laid-out page.
- Row 155 (the composer's formatting row) is the composer's.
- The Slint app's `invite::record` still writes the local calendar; it goes with the Slint app.

## Stage D: composer (done on gpui/d-composer, with gaps)

Branch `gpui/d-composer` (from `main` adee939), the composer fix lane of stage D. Only composer
files were changed (`views/composer/`, `widgets/field.rs`, the library's `compose::send`); what
the shell, mail and reader views must call is listed under "Not done" as integrator work.

### What was fixed

1. **A send from a draft tab goes out (blocker).** The sends in their undo window are the app's,
   not a composer's (`views/composer/sends.rs`, one desk in a GPUI global shared by every
   composer, as the Slint app's App-level `pending_sends`): the build, the undo window and
   `outbox_queue` run on the app, so a draft tab's composer may go with its tab. What they say
   later (`Queued`, a failed build's error, `Unsent`, `DraftsChanged`) is told through the *home*
   composer, the first one made that is still there (the mail view's, made with the window,
   before any tab). `send` now tells `DraftsChanged` and `Sending` before `Closed`, so the tab
   host still listens when the Undo offer and the Drafts count go out.
2. **Undo of a draft tab's send** works from the status line: any composer undoes any pending
   send (`pending_sends` lists the app's), so the mail view's composer, which `HubEvent::UndoSend`
   asks, takes it back and shows it in the pane. `sent_by(row)` names the composer that started
   a send, for the shell to reopen it in a tab instead, as the Slint app does.
3. **`ComposerEvent` wiring, confirmed in code:** the status line (`Hub::on_composer`, `Status`),
   the Undo offer and its count-down (`Sending`, `Queued`, `Unsent`), the server copies
   (`SaveToServer`, `RemoveServerCopy`) and the send worker (`Queued` to `Job::Send`), from the
   mail view and from draft tabs alike; the Drafts count, rows and the row's Draft tag
   (`DraftsChanged` and `Unsent` reload the folders and the rows); `Closed` gives the list its
   focus back; `bind_keys` in `main.rs` and `test_support::init_app`; every composer saves its
   drafts on quit through its own `on_app_quit` (not `window.rs`).
4. **The painted body for screen readers** (`views/composer/a11y.rs`): the whole text as the
   value of the `MultilineTextInput` node, a `TextRun` node per document line (paragraph, list
   item, table cell) with its bounds in window device pixels and its character lengths, and the
   caret and selection as the node's text selection. Only the lines in the editor's paint margin
   are given, with the caret's and the anchor's lines always among them. Settings > Signatures
   uses the same editor and gets the same nodes.
5. **Preview for screen readers:** `compose::send::PreviewPage` carries the page's text lines
   (`Doc::text_lines`, the ones drawn) and the Preview's `Document` node gets them through the
   reader's `a11y::text_nodes`.
6. **Recipient suggestions:** a `ListBox` named "Suggested recipients" of `ListBoxOption`s, the
   highlighted one selected (see Decisions for why not the active descendant).
7. **Focus ring:** the composer's underline rows (Subject; To, Cc and Bcc in `ChipField`) draw
   the brand ring on keyboard focus as well as the ink line.
8. **Ctrl+V with files** copied in Explorer attaches them from any composer field (the address
   rows, Subject, the link row, a plain-text body), not only from the formatted body.
9. **"Drop to attach"** shows over the composer while files from Explorer are dragged over it
   (`on_drag_move::<ExternalPaths>`, cleared when GPUI drops the drag), as a `Status` node.
10. **The composer's side of the parity items** (the shell wires them, below):
    `edit_server_draft(message)` (the reader's Edit on a server draft: adopted once as a saved
    draft linked to the server's copy; the Slint app's `edit_server_draft`),
    `reply_started(message)` (the reader's "You started a reply to this message" bar; Resume is
    `open(Draft::Reply { .. })`, which brings the draft back), and `set_aside()` (the pane's draft
    with writing is saved here and on the server and handed over by row for a background tab,
    status "Draft kept in a tab"; the Slint app's `pane_draft_aside`).
11. **Tests:** `a_send_from_a_draft_tab_goes_out_after_the_tab_goes`,
    `files_dragged_over_the_composer_say_drop_to_attach`,
    `a_server_draft_is_edited_as_a_saved_draft`, `a_dirty_pane_draft_is_set_aside_for_a_tab`,
    `a_saved_reply_tags_its_row_in_the_list` (row 95, its tag half),
    `ctrl_tab_moves_between_tabs_from_the_composer` (row 296, its composer half), the editor's
    `screen_readers_get_the_text_its_lines_and_the_caret`, the preview's lines in the library's
    `the_preview_draws_what_is_sent_within_its_caps`, and the suggestions' roles and name in
    `recipient_suggestions_from_the_address_index`.

### Decisions

- **The home composer** is the oldest live one rather than a flag the mail view sets, so no file
  outside the composer had to change; a composer made before the mail view's would become home
  (none is today).
- **Suggestions without the active descendant:** GPUI honours `aria_active_descendant` only on a
  descendant of the focused node, and the focus is in Kit's `Input`, whose node cannot hold the
  list (it is a sibling, and a deferred one). A `ListBox` is a selection container, so
  AccessKit's UIA adapter raises `SelectionItem_ElementSelected` for the highlighted option as
  Up and Down move it, which Narrator reads with the focus in the field.
- **No first-format caption:** the Slint app's `c-format-hint` is set only by the plain-text
  markup edits, and no element in `ui/*.slint` shows it; the GPUI app's plain-text body has no
  markup edits (phase 4 decision), so there is nothing to port.
- The new sentences ("Drop to attach", "Suggested recipients", "Draft kept in a tab") are the
  Slint app's catalog entries word for word; no new msgid (the regenerated template gains only
  `#:` lines).

### Checks

`cargo xtask check` (CARGO_TARGET_DIR `C:\dev-overflow\cargo-targets\komorebi-d-composer`):
fmt, clippy `-D warnings` over the workspace, komorebi-gpui 267 passed (1 ignored, the typing
measurement), the library 816 passed (1 ignored), xtask 9, lint ok; the size check skipped (no
release build, as the stage rules say). The first run failed once in the mail lane's
`views::mail::tests::deleting_from_trash_or_junk_asks_first` ("ambiguous ElementId
dialog-confirm": the cancelled dialog still closing); alone it failed once in three runs, and the
second full run passed. It opens no composer; it is reported for the mail lane. The app was not
launched.

### Not done (stage D composer), and integrator requests

- **Tab host, mail and reader wiring** (not this lane's files): on `HubEvent::UndoSend` for a row
  whose `sent_by` is not the mail view's composer, reopen it in a new draft tab
  (`open_draft_tab(Draft::Saved(row))`) instead of the pane (row 305); call `set_aside()` before
  a new draft starts in the pane and when another message is selected, and open the row it
  returns in a background draft tab (rows 77, 302); handle `ReaderEvent::EditServerDraft` with
  `ComposerView::edit_server_draft`; show the reader's "You started a reply" bar from
  `reply_started`; pass the current folder's or the open message's account to `compose_new` and
  `compose_mailto` (the mail view's `Draft::New` and `ReaderEvent::Compose` pass `None`, so the
  first account is used); `ManageSignatures` should open Settings on its Signatures section and
  not close Settings when it is already the place (`window/places.rs` `show_place` toggles).
- The address fields are not marked expanded while their suggestions show: Kit's `Input` has no
  `aria_expanded`.
- Composer windows (a draft in a window of its own) and the draft tab's title following its
  subject (shell); the compose defaults (font, size, colour) on a settings page (settings); a
  snapshot action `compose` in `snapshot.rs` (its test still asserts it is refused).
- By hand: Narrator over the body's lines, the caret and the suggestions; a real IME; the drop
  overlay and a paste of files from Explorer (the tests cannot reach the system clipboard).

## Stage D: settings (done on gpui/d-settings, with gaps)

The settings lane's parity fixes after stage C (`views/settings/`, `views/onboarding/`).

### What was fixed

- **Closed, checked:** Settings > About has its update card (mounted in stage C,
  `set_updates(UpdatesCard)`), and the settings and onboarding mounting and event wiring in
  `window/places.rs` is complete (every `SettingsEvent` and `OnboardingEvent` has an owner;
  `bind_keys` is called from `main.rs` and `test_support::init_app`). The phase 5 and phase 6/7
  notes that still list them are out of date.
- **Compose defaults:** General has a Default font card (font, size, colour, with a sample in the
  chosen colour in every theme, as the Slint card); it saves `compose_font`, `compose_size` and `compose_color` and tells the
  shell. The defaults are global, as in the Slint app, so one card covers the three items.
- **Account page:** Find settings (the lookup for the address, filled in through `find::merge`
  only when the form did not change meanwhile), the SMTP relay without a sign-in
  (`smtp_no_auth`), Move up and Move down in the page's More options menu (`move_account`, the
  shell told through `AccountsChanged`), Enter in a field saves, and Server settings folded for
  any account without a pin or a relay (`editor::starts_open`; a failed check opens them); Find
  settings is hidden for a preset's servers, and offline it says so in its own note (the Slint
  app's `invoke_err_offline_find`), each new lookup clearing the last note. The account card shows
  its download as a progress bar and a warning line with the warning glyph, named "Warning: ..."
  to UI Automation (the brand has no danger colour; the text stays ink).
- **Signatures:** the full toolbar (bold, italic, underline, strike, colour, lists, indent, link,
  clear, alignment) and Link with its address row (Ctrl+K, Enter inserts, a bad address is
  refused).
- **Onboarding:** Find settings and the certificate lines with Forget certificate in manual
  setup, the SMTP relay, the Slint flow's server labels, provider radio buttons with their
  descriptions, the welcome's three value rows, and the first account saving the What's new
  version (row 179).
- **Accessibility:** radio groups (Segmented, the provider list, the trust dialog) are one Tab stop
  and move with the arrows, Home and End; on the provider list the arrows only move the focus onto
  Other server (Space, Enter or a click picks it and opens manual setup), so the list is not left
  by stepping; F1 and Ctrl+/ open Keyboard shortcuts from any window (deferred until the key's
  window update is over, and not over another open dialog); the step indicator is a named list whose
  items say done and current step.
- Tests: 16 new view tests and 3 plain ones (Ctrl+K from the signature text, F1 over an open
  dialog and the provider list's arrows among them); rows 116, 120, 121 and 179 done, the rest marked.

### Not done (stage D, settings)

- Show tips again: it resets `tips_seen`, but the teaching tips are not built (rows 175 and 176,
  the shell's).
- The welcome's Settings button needs an `OnboardingEvent` variant and its arm in the exhaustive
  match in `window/places.rs` (integrator request).
- Rows 142, 143, 153, 158 and 159 stay in part: their remaining halves (the workers, the
  redownload reset, the Ctrl+, binding, the shell's account line, the file removal) are shell
  code. Row 119's changed-certificate half stays the rule test.
- Three new msgids in pt-PT; `translations/komorebi.pot` is regenerated by the integrator.
- By hand: Narrator over the radio groups and the step list.

## Stage D: data (done on gpui/d-data, with gaps)

Branch `gpui/d-data` from `main` at adee939: the data lane's stage D items (rules and contacts).

### What was fixed

- **Mounting items of phases 8 and 9:** checked in the code, all done in stage C (the "Not done"
  list above predates it): Settings > Rules (`window/places.rs`), the Contacts place, Rules' and
  Contacts' events in the status line (`window/places.rs` `RulesEvent`, `ContactsEvent`: Sieve
  copied, Copied, No account, Failed; Compose opens the composer through `compose_mailto`),
  `timer_tick` on the hub's `HubEvent::Tick`, and the snapshot actions `settings-rules`,
  `rule-edit`, `contacts`, `contact-edit`, `contacts-sync` (`snapshot.rs`).
- **Letter headers in the Contacts list** (the mockup's `.letter`): the list is a Kit
  `v_virtual_list` with two heights (a header, a row) in place of `uniform_list`. The letter is
  the library's (`komorebi::contacts::letter`, a new `ContactRow::letter`): the first letter of
  the sort key (family name, then given, then shown name, folded), so a letter's rows are
  together; `#` for anything else. Headers are headings (Narrator's heading keys jump by
  letter); the keys and a click scroll a letter's first row into view with its header.
- **Contact rows grow with Windows' text size:** 56 px at 100 percent plus what the name (body)
  and address (caption) lines grew, as `scaled::message_row` does; the header grows too.
- **Drag reorder of rules:** a card drags onto another to take its place (`DraggedRule`, the card
  under the pointer washes). Move up and Move down stay in the menu for the keyboard. After review:
  the handle is a grip of six dots at the card's start (`rule-grip`, the open hand), so a press on
  Edit, the switch or "..." never starts a drag; the drag and the drop name the rules by id
  (`RulesView::drop_rule(id, onto)`), so a list read again during the drag cannot move another
  rule; the move is one transaction that numbers the account's rules again
  (`Db::move_rule_to`, `rules_page::Page::move_rule_to` returns the cache's error, shown as
  `RulesEvent::Failed`).
- **Review fixes (contacts):** the list order and the letter now come from the same text. The
  stored `sort_key` is unchanged (no migration); `contacts::list_order` orders `Db::contact_ids`
  by `#` first, then letter, then the key without the separators of empty name parts, and
  `ContactRow::letter` is `contacts::key_letter` of the stored key. Before, a contact with no
  family name (FN-only vCards, contacts typed in by hand) sorted before every family name, so
  letters repeated and came out of order. The virtual list's entries, scroll targets and sizes
  are built once per read of the list (`list::Entries`, cleared on every change of the rows), not
  on every frame and key press.
- **The large spot's initials:** the contact's 72 px avatar has its initials in the title step
  (24 px semibold, display face) in ink, as the mockup's `.cd .av` (a local element in
  `views/contacts/detail.rs`; the shared `SenderSpot` keeps body text from 40 px up).
- **Sync is a visible button** beside New contact, named "Sync" with the tooltip "Address book
  sync" (the Slint app's), in place of a "..." menu with one item.
- **Names for repeated controls:** condition rows are "Condition {n}" (field), "Condition {n}
  comparison", "Condition {n} value" and "Remove condition {n}"; a rule card's buttons are
  "Edit {rule}" (described by its summary) and "More actions for {rule}"; the contact editor's
  lines are "Email {n}", "Remove email {n}", "Phone {n}", "Remove phone {n}". Nine new msgids,
  each in pt-PT.

### Checks

`CARGO_TARGET_DIR` `C:\dev-overflow\cargo-targets\komorebi-d-data`. `cargo xtask check` passes
(fmt, clippy over the workspace on all targets with `-D warnings`, komorebi-gpui 265 passed and 1
ignored, library 818 passed and 1 ignored, xtask 9, the token and timer lints; size skipped, no
release build). The first run failed `views::mail::sync_tests::port::the_outbox_pane_retries_and_discards`
once (the Discard step left the outbox row); it passed three times alone and in the next two full
runs, and it is not this lane's code. `cargo test --test i18n_catalog` fails only
`template_is_current` (the new msgids, below).
Tests added or changed: `views::contacts::tests::port::rows_sit_under_their_letter`,
`views::contacts::tests::rows::{a_header_starts_each_letter, rows_grow_with_the_text_size}`, the
names in `contacts_are_added_edited_and_deleted` and the Sync button in
`the_sync_pane_saves_the_mode_and_waits_for_an_address`; the drag and the row names in the row
291 port; `komorebi::contacts::tests::letter_follows_the_sort_key`,
`komorebi::rules_page::tests::a_rule_moves_to_the_place_it_is_dropped_on`; the text tests in both
languages. No release build, so no snapshot was taken of the new list and cards.

After the review fixes `cargo xtask check` passes again (komorebi-gpui 267 passed and 1 ignored,
library 820 passed and 1 ignored, xtask 9; size skipped). New tests:
`komorebi::contacts::tests::list_order_keeps_each_letter_together`,
`komorebi::db::contacts::tests::list_rows_keep_each_letter_together`,
`views::contacts::tests::port::{mixed_contacts_keep_one_header_per_letter,
rows_hold_both_lines_at_large_text}` (it reads the laid-out row, name and address; with the
growth taken out of `row_h` it fails at 225 percent, the name above the 56 px row, but passes at
150 percent, so only the 225 percent clipping is shown by a test); the rules drag test drags by
the grip, checks that a drag from Edit or the switch moves nothing, and that a drop of a rule
that is gone moves nothing; `a_rule_moves_to_the_place_it_is_dropped_on` moves by id and over a
gap in the positions.

### Not done (stage D, data)

- The composer's recipient suggestion rows (`views/composer/recipients.rs`, `SUGGEST_ROW_H`) still
  have a fixed 40 px height; they are the composer's files (an integrator request: grow them
  with `scaled::control`).
- `translations/komorebi.pot` is not regenerated (the integrator's), so `tests/i18n_catalog.rs`
  `template_is_current` fails on this branch until then; the pt-PT entries are in the catalog.
- A sticky letter header (the header scrolls away with its rows), a drop indicator line between
  cards, and a Narrator pass by hand (phase 13). The grip is drawn dots, not a Fluent icon (the
  bundled icons have no re-order glyph), and it is not named for Narrator (the menu's Move up
  and Move down are the accessible way).
- The Slint app also lists contacts in the new order (it reads `Db::contact_ids`); its own list
  has no letter headers, so nothing else changes there.
- The gallery's large `SenderSpot` still shows body-size initials (the shared widget is not this
  lane's; moving the title step into it would cover both).

## Stage D: other (access keys, CI, Kit in pt-PT, snapshot checks; on gpui/d-other)

The fix lane for what belongs to no screen. Items are the ones the earlier sections list as open.

### What was fixed

1. **Kit's built-in strings in pt-PT.** `third_party/gpui-component/locales/ui.yml` has a `pt-PT`
   line on all 119 keys (KOMOREBI PATCH, `KOMOREBI-PATCH.md` there: the words Kit shares with the
   catalog are the catalog's; the calendar's names are the library's), and `crate::i18n` switches
   Kit's locale (`gpui_kit::component::set_locale`) wherever it sets the language, so a language switch
   redraws an input's Cut, Copy, Paste, Select all, Clear, the password buttons, a dialog's Cancel
   and the rest in Portuguese. Two traps found and closed: Kit's build script told cargo to
   watch only `build.rs` and the icons (an edit of `ui.yml` did not rebuild the crate), and rustc's
   incremental cache keys the `i18n!` proc macro on its input tokens, so a `target` folder that
   had built Kit before kept the old table; the call now reads `fallback = ["en"]` (same fallback,
   new tokens) and the script watches `locales`. Tests: `i18n::tests::kit_follows_the_language`,
   `every_kit_string_is_in_portuguese` (a key without Portuguese fails, so does a stale table),
   `tests/i18n_catalog.rs` `kit_strings_agree_with_the_catalog`.
2. **Access keys** (`crate::access`). Hold Alt: every control with an access key draws its letter
   in a small ink chip under it (the tooltip's look, no new colour); Alt and the letter runs the
   command. One `Press(char)` action is bound to Alt+A to Alt+Z; its handler maps the letter to a
   `Key` in the current language and sends the command's own action (the one its shortcut sends:
   `NewMail`, `Reply`, `Archive`, composer `Send`, ...) to the focused window, to the first
   that is available where the focus is, the composer's first. So an access key does exactly what the shortcut
   does, and English Alt+S still sends (the composer's own binding is deeper). The letters are
   per language, in the name the person reads (English Folders is Alt+O, Pastas Alt+T; Reply is R
   in both); keys that can be on screen together differ, which a test checks for both languages,
   with another test that the letter is in the command's name. `Button::access_key` and
   `IconButton::access_key` draw the chip and tell UI Automation the letter (`aria-keyshortcuts`
   "Alt+R"); `AccessHost::access_keys()` on the window's root element shows the chips while Alt
   (alone: Ctrl+Alt is AltGr) is held. Tests: `access::tests` (8, window tests among them; the language test uses Search, E in English
   and P in Portuguese, so the chip and the key both move). Wired in the app (review fix):
   `.access_keys()` is on the roots of the main window, the message view and the composer; the
   bindings are installed at start-up; the controls that carry a key are New mail (the button and
   the rail's pill), Folders, Settings (the folder pane's foot), Refresh, Reply, Forward, Archive,
   Delete, and the composer's Attach, Send and Link. Alt held while the window is left (Alt+Tab)
   lets go: the activation observers call `access::on_activation`, so no chip stays drawn.
3. **Snapshot checks.** `snapshot::blank_panes` (the Slint app's, on the GPUI layout: folder pane
   or rail, list, reading pane) names a blank list or reading pane; it runs with the default
   action when `KOMOREBI_SNAPSHOT_SELECT` names one row (a seeded cache, as the Slint boot check
   does), through `Request::panes`. Row 2 of the test port is ported
   (`snapshot::tests::blank_panes_names_the_empty_ones`, plus
   `the_pane_check_is_for_one_opened_row`). `KOMOREBI_SNAPSHOT_SELECT` and `_SEARCH` were already
   out of `NOT_YET` (stage C); GPUI-TESTING.md now says so, and lists the actions.
4. **CI.** The boot check script gained `-Select <row>` and `-Fallback` (a blank first run is
   tried again with `GPUI_DISABLE_DIRECT_COMPOSITION=1` only when the app's log names a blank
   title bar or pane, not for a timeout, a missing file or a blank list or reader; the output says
   which mode passed; the script returns the app's real exit code, which an earlier version lost to
   the log lines in the function's output); CI
   runs the GPUI boot check with the fallback and a second one on a seeded cache with the pane
   check, and uploads both PNGs; `release.yml` uses the fallback and builds a preview of the 1.0
   setup (`-Brand10`, `-OutDir dist-1.0` around `komorebi-gpui.exe`, artifact
   `preview-installer-1.0`, never signed or attached to the draft). Nothing of this ran on the
   runner: no push from a lane.
5. **The 1.0 setup compiles.** `ISCC /DBrand10` on `installer/komorebi.iss` compiled to a setup
   (the icon and the four wizard images accepted); it was not run (installing changes the
   machine), so its pages are still unopened.
6. **Test port.** Row 2 done; rows 82, 150, 152, 229, 230 and 273 are closed without a port and
   GPUI-TEST-PORT.md now has a "Status words" paragraph that says how to count them.

### Measured on the merged tree (release, this machine, desktop session, WARP)

- **Snapshot checks on the views.** The GPUI boot check passed on an empty cache (the welcome flow)
  and on a seeded 2,000-message cache with row 0 open, English light and Portuguese dark: the pane
  check (`blank_panes`: list and reading pane both have content) passed, and the screenshot shows
  the folder pane, the list and the reader filled. Every snapshot action ran with
  `KOMOREBI_SNAPSHOT_CHECK=1`: the 7 settings sections (accounts, account, signatures, rules,
  general, privacy, about), `rule-edit`, `contacts`, `contact-edit`, `contacts-sync` and the four
  onboarding states (`ob-welcome`, `ob-email`, `ob-sign-in`, `ob-manual`) draw, and the pictures
  were looked at. This found a fault in the check itself: `blank_parts` judged the "pane" by one
  pixel in the middle of the window, which on `settings-rules`, `settings-general`,
  `settings-privacy` and `ob-manual` falls inside a card or a field, so four drawn pages were
  reported "blank pane" (exit 1). It now takes the colour most of the middle band under the title
  bar has, and a test (`the_shell_check_names_what_is_missing`) paints a card under the middle; all
  15 actions pass after the fix.
- **Size: over the ceiling, and not by this lane.** `cargo xtask size --gpui`:
  `komorebi-gpui.exe` is **22,051,328 bytes**, ceiling 21,500,000, **OVER by 551,328**. A release
  build of `main` itself (a separate worktree, same toolchain and profile) is 22,036,480 bytes, so
  this lane adds 14,848 bytes (the Kit's Portuguese table and the access keys) and the rest came in
  with stage B and C (stage A measured 11,074,048). The CI "Binary size" step will fail on the
  merged tree until the exe is cut by about 0.6 MB; that is the S1 size pass (phase 18, moved to
  1.1 by owner decision, the 21.5 MB CI gate stays), so the integrator or the owner decides: cut,
  or move the ceiling.
- **Launch.** First frame drawn, `KOMOREBI_TIMING=1`, seeded 2,000-message cache, offline, WARP,
  1280x800, median of 9: **77.0 ms** (this lane; 78.4 ms for `main`; range 75.0 to 80.7 ms), against
  the 200 ms budget; stage A measured 118.5 ms on the empty skeleton. The cache loads in about
  40 ms. The cached-list budget (600 ms) was not measured on its own: the list is drawn in that
  first frame.


### Not done in stage D (other)

- **CI did not run.** The new GPUI jobs (fxc step, WARP boot check with the fallback, the seeded
  check, the UI Automation dump, the 1.0 setup preview) are written and their scripts run on this
  machine, but a lane cannot push, so nothing ran on a Blacksmith runner. Whether the runner's DWM
  composes the window, and whether `fxc.exe` is found, shows on the first push.
- **The 1.0 setup's pages are unopened.** `ISCC /DBrand10` compiles it; running a setup changes the
  machine, so the wizard pages (icon, the four images, the texts) still need an eye on a clean
  machine or VM.
- **Access keys on the rest of the controls** (later work: "Gaps: other" below). The main commands
  carry a key now (see item 2);
  Search (Alt+E, Alt+P) works as a key but draws no chip, because the search box is not a button;
  the message actions in the more menu, the list rows and the Settings pages had none then. The chips
  on a real screen and a Narrator pass over `aria-keyshortcuts` were not done (the rules forbid
  launching the app outside the snapshot switches).
- **`translations/komorebi.pot` was not regenerated** (the integrator does it); the template still
  passes `template_is_current`, because this lane added no new msgid.
- **Size is over the ceiling** (above), not fixable inside this lane.

## Stage D integration (done on main)

The eight stage D lanes are merged with `--no-ff` in this order: `gpui/d-library`, `d-shell`,
`d-mail`, `d-reader`, `d-composer`, `d-settings`, `d-data`, `d-other`. The merges conflicted only
in the shared append-only files (this page, `GPUI-TEST-PORT.md`, `komorebi.po`), resolved as
unions, and in a few code places where two lanes had built the same thing.

### Where two lanes built the same thing

1. **Crash log.** The library lane moved `src/crash.rs` into `komorebi::crash`; the shell lane
   had made its own copy in the GPUI crate. The GPUI copy is deleted and `main.rs` calls
   `komorebi::crash` (rows 15 and 16 are the library's tests).
2. **Source commands** (Save as .eml, Show original, Copy Message-ID). The mail lane's
   `views/mail/original.rs` and the reader lane's `views/reader/actions.rs` both had them. The
   reader's is kept (it also serves tabs and windows) and the mail view runs it through
   `MailView::source_command` (the row menu, the More menu and Ctrl+S from the list). `original.rs`
   and its `raw_wait` are gone; `Reaction::BodyFailed` now calls `ReaderView::body_failed`; the mail
   test `the_source_commands_copy_show_and_save` goes through the reader (the "Fetching message"
   line shows after one second, as the reader says it).
3. **RSVP.** The reader lane's `invite::keep_answer` and the library lane's
   `invite::record_answer` did the same; `record_answer` stays, the reader calls it, and
   `keep_answer` and its test are removed.
4. **F6 and F1.** The shell lane's window-level F6 (three panes) and the mail lane's `cycle_pane`
   (search, folders, list, reading) both existed: the mail view's is the one, and the window's
   F6 and Shift+F6 call it (that also covers F6 pressed in the title bar's search); the window test
   now expects the search between the reading pane and the folders. The settings lane bound F1 and
   Ctrl+/ in every context, which opened the dialog from the composer too; those bindings are now
   Settings' own (the main window binds the rest, and Ctrl+/ is off in the composer), and the
   window's F1 does not stack a second dialog.

### Integrator requests done

- Reader: `ResumeReply` opens the saved reply (in the pane, or in a draft tab from a tab or window)
  and `handle_resume` is called for the pane's reader and every tab's and window's, so the "You
  started a reply" bar shows; `EditServerDraft` edits the server draft (in the pane, or in a tab);
  `OpenPrivacy` opens Settings on Privacy; `Reaction::BodyFailed` reaches the reader.
- Composer: `ManageSignatures` opens Settings on Signatures and does not toggle Settings
  (`MainWindow::show_section`); New mail and `mailto:` use the open folder's (or message's)
  account; a send undone from a draft tab comes back in a tab (`MailEvent::OpenDraftTab`); a pane
  draft with writing in it goes to a background tab ("Draft kept in a tab") when New mail,
  a `mailto:` or another message needs the pane (`MailView::make_room`); the recipient suggestion
  rows grow with the text size.
- Settings: the welcome has a Settings button (`OnboardingEvent::OpenSettings`).
- Mail: the title bar's search has its scope box beside it (`search_scope_box`).
- Library: the GPUI binary installs `komorebi::crash` and shows the launch notice (the shell
  lane's start-up code over the library's module); `trust_notice` is `pub(crate)`.
- `translations/komorebi.pot` regenerated once (the 3 settings msgids and `Needs attention`);
  the two `.po` additions were appended as a union.

### Checks

On the merged tree (`CARGO_TARGET_DIR` `komorebi-integ`): `cargo xtask check` passes (fmt,
clippy `-D warnings`, komorebi-gpui 362 passed and 1 ignored plus its 4 and 2 integration tests,
library 826 passed and 1 ignored, xtask 9, lint ok; size skipped, no release build). The full
`cargo test --locked` passes (Slint bin 208 passed and 1 ignored, `i18n_catalog` with
`template_is_current`, the corpus tests). `cargo deny --workspace --locked check licenses
advisories bans sources` is all ok. `scripts/check-pins.ps1` could not run (the execution policy
blocks scripts and may not be changed); its checks were done by hand with the same logic over
`cargo tree --locked -p <package> --duplicates` and `cargo metadata`: no exactly pinned crate is
linked twice in the Slint app, in the GPUI app only the known pairs (`tiny-skia`, `png`, `gif`,
`roxmltree`, `zune-jpeg`, `zune-core`, `windows`) and the exempt `windows-sys`, and Slint still
declares the three `TextInput` properties.

### Not done (stage D integration)

- **The exe ceiling.** No release build was made, so the 21,500,000 bytes were not measured. The
  last release exe on a lane's disk was 22,051,328 bytes (main alone was already 22,036,480):
  about 0.55 MB over. Cutting it is the size pass (phase 18, moved to 1.1) or a decision on the
  ceiling; the CI Binary size step fails until one of them is done.
- Untested by hand or by test: an undone send from a draft tab coming back in a tab, a pane draft
  going to a background tab on New mail or `mailto:`, the reader's Resume and Edit in tabs and
  windows, the welcome's Settings button (closing Settings does not return to the welcome), the
  scope box's layout in a narrow window. Rows 77, 302 and 305 of the test port stayed open here; "Gaps: other" closes 77 and 305 and
  gives 302 a test that waits for a patch.
- The reader's Junk event in a tab or window (`Quick` has no Junk), the command bar as one Tab stop
  (row 154), Backspace in the read-only text view, a save glyph and a move-to-folder glyph in the
  shared icons, the Save attachment title (GPUI's prompt takes none), UI Automation text
  selection in the reader page, and the Search chip for Alt+E and Alt+P.
- Kit's dialog host has no accessible name (the title is on the Group inside it); the Narrator
  pass over dialogs, radio groups, suggestions, the composer text and the access keys is not done.
- `set_aside` hides a draft whose save failed without a tab; Copy Message-ID and Show original
  still load the whole message (no Message-ID accessor in the cache); `invite::card` still marks a
  stored copy cancelled; `pin-utils` was ignored, not removed (it was Slint's; the Cut-over removed it).
- Teaching tips (rows 175 and 176), composer windows (rows 101 to 104), the remaining tab rows,
  popout size memory, tab drag, roving tab stops, and the other gaps in the lanes' own stage D
  sections above.

## Cut-over preparation (done on main, local only)

What the Stage D note, phases 16 and 17 and the owner decisions list for the cut-over, done in the
tree and not published: nothing was pushed, tagged, signed or released. The Slint app was still in
the tree at this step, which changed what is released and what the docs say; the Cut-over section
below deleted it, bumped the version and settled the items under "Not done" here (each is marked).

### What was done

1. **`release.yml` builds and packages the GPUI app.** It builds `komorebi-gpui` only
   (`cargo build --release --locked -p komorebi-gpui`), checks it with `cargo xtask size --gpui`
   (21,500,000 bytes) and its manifest, runs the GPUI boot checks on WARP (empty cache, and the
   seeded one with the pane check; the Slint boot snapshot step is gone), uploads
   `komorebi-gpui.exe` as `unsigned-exe`, builds the installer around it with `-Brand10`
   (`unsigned-installer`; the setup still installs the exe as `komorebi.exe`), and packages the
   portable zip with the exe named `komorebi.exe` and the symbols from `komorebi_gpui.pdb`. The
   `preview-installer-1.0` job steps are removed (the release installer is the 1.0 one). The
   SignPath comments point at `komorebi-gpui.exe`. The Slint app is still tested with the library
   (`cargo test --locked`) but is neither built nor packaged by the release.
2. **The ceiling.** `ci.yml`, `xtask/src/size.rs`, README, CONTRIBUTING, `docs/BUILDING.md`,
   `docs/PERF.md`, `docs/SCOPE.md`, `docs/RELEASING.md`, `docs/GPUI-BUILD.md` and TODO name
   21,500,000 bytes as the GPUI app's, the release exe's ceiling; the Slint exe keeps its old
   20,500,000 gate "for as long as it is built" (the code of `xtask` is unchanged: only the comment).
3. **Honest memory wording.** "Light on memory", "low idle memory", "about 10 MB idle" and the
   8.9 MB installer figure are gone from the README, SCOPE, PERF and TODO; they say that 1.0 holds
   more memory at idle than 0.7, that this is accepted for 1.0 and that reducing it is 1.1 work.
   `docs/PERF.md` has a new section 0 with what is measured and what is not: the 22,051,328-byte
   exe (over the ceiling), the 77.0 ms first frame, and, from the spike report (S2, S3; a skeleton,
   not the shipped app), private bytes 111.1 MB against 11.6 MB, private working set 22.1 MB
   against 10.7 MB, 35 threads against 11, and idle CPU. No number of the shipped app's memory
   exists.
4. **Calendar out of the user docs.** README, `docs/FEATURES.md` (the Calendar entry and the
   calendar sentences of the invitation entry), `docs/SCOPE.md` (a non-goal now), PRIVACY.md (the
   calendar, CalDAV and event reminder data and requests; one paragraph says what a 0.7 cache may
   still hold and that 1.0 does not read or sync it) and TODO (the Calendar section) no longer
   describe a calendar. The invitation card stays, with its reply to the organizer and Save .ics.
5. **CHANGELOG** has `## [1.0.0] - Unreleased` (so that the release notes step finds it once the
   version is 1.0.0): the move to GPUI, the new brand, higher idle memory, access keys, Kit in
   Portuguese, the calendar removed, high contrast not in 1.0, the known gaps, and the local data
   note (no schema change; a 0.7 cache keeps its calendar rows unseen).

### Checks

`cargo xtask check` (fmt, clippy `-D warnings`, komorebi-gpui 362 passed and 1 ignored plus its
4 and 2 integration tests, library 826 passed and 1 ignored, xtask 9, lints) passes; the size step
is skipped (no release build). The full `cargo test --locked` passes too (library 826 passed and 1 ignored, Slint bin 208 passed
and 1 ignored, the rest of the integration tests).
`actionlint` is not installed on this machine, so the two workflow files were checked by reading
(the changed steps and their indentation), not by a linter; the first run on a runner is the check.

### Not done (cut-over preparation)

- **The version was not 1.0.0 (done at the Cut-over, below).** This page does not say the cut-over bumps it, so `Cargo.toml`
  (`komorebi` 0.7.0, which `release.yml`'s Version step, the installer name and the library's
  update check read) and `crates/komorebi-gpui` (1.0.0-alpha.0, which the exe's version resource
  and About read) are unchanged. Until they are bumped a `v1.0.0` tag fails the Version step, and
  an installer built now is named `0.7.0`. The comment in `crates/komorebi-gpui/Cargo.toml` says
  the GPUI app "takes the package version" at the cut-over; the owner confirms how, then the root
  package, the GPUI crate, `Cargo.lock` and the test that names the version move together.
- **The Slint app was not deleted (done at the Cut-over, below).** The page lists what goes at the cut-over (the Slint extractor,
  `ui/`, `ui/tr.slint`, `src/tr.rs`, `pin-utils` in `deny.toml`, `assets/brand-1.0` moving up
  over `assets/komorebi.ico` and `komorebi.rc`, the `Brand10` branch becoming the installer's only
  one, `invite::record` and the calendar library code) but does not say to delete the app, so
  none of it was done. NOTICE still says "Slint ... linked into komorebi.exe" and is unchanged.
- **The exe is over the ceiling (still true; the latest figure is in the Cut-over's "Not done").**
  22,051,328 bytes at this measurement against 21,500,000 (551,328 over): `cargo xtask size`
  in `ci.yml` and `release.yml` fails until the exe is cut or the owner moves the ceiling. No
  release build was made here.
- **The 0.7 `window` key is not read (decided: it stays so).** The Layout decisions said the 1.0
  app converts it once at the cut-over; the owner decided the 1.0 app resets the saved window
  position instead, so `crates/komorebi-gpui` reads `gpui_window` only and the first 1.0 start opens
  at the default place. The CHANGELOG says so.
- The calendar code stays in the library (`src/cal/`, `src/dav/caldav`, the calendar tables). The
  Slint calendar page went with the Slint app at the Cut-over; the library code stays on purpose,
  because the calendar returns in 1.1.0.
- The user docs still described the 0.7 screens in places (`docs/USAGE.md`, the screenshots in the
  README, which say so). The pass over USAGE is done ("Gaps: other" below); new screenshots are not.
- Not run: the 1.0 installer pages on a clean machine, the update from 0.7 through the signed
  updater, the first `release.yml` run on a runner.

## 1.0 release candidate checks (run on main, 2026-11-06)

Run in `D:\Projects\komorebi` on `main` at the cut-over commit `5062e98`, with
`CARGO_TARGET_DIR=C:\dev-overflow\cargo-targets\komorebi-integ`. Nothing was pushed, tagged,
signed or published. The machine was busy with other work (the bench header says load 54 to
100 percent), so the timings below are noisy.

### Results

| Check | Result |
|---|---|
| `cargo xtask check` (fmt, clippy `-D warnings`, tests, lints; no release exe yet at that point, so no size step) | **Pass.** komorebi-gpui 362 passed and 1 ignored, plus 4 `brand_assets` and 2 `token_lint`; library 826 passed and 1 ignored; xtask 9; GPUI lints ok. |
| Full `cargo test --locked --workspace` | **Pass.** 1,490 passed, 0 failed, 3 ignored over 23 test binaries (library 826, Slint bin 208, komorebi-gpui 362, i18n 8, xtask 9, the corpus and render integration tests the rest). |
| Release build `cargo build --release --locked -p komorebi-gpui` | Built. |
| Exe size, `cargo xtask size --gpui` | **FAIL: 22,387,712 bytes against the ceiling of 21,500,000, over by 887,712** (stage D measured 22,051,328, so the merged tree grew by 336,384). `release.yml` and `ci.yml` run this step and will fail until the exe is cut or the owner moves the ceiling. Plain `cargo xtask size` also reports the missing Slint `komorebi.exe`, which this run did not build. |
| Boot check, `scripts/gpui-boot-check.ps1 -Fallback` (sets `KOMOREBI_OFFLINE=1`, a throwaway `KOMOREBI_DB` under `C:\dev-overflow\claude-tmp\rc`, `KOMOREBI_SNAPSHOT` and `KOMOREBI_SNAPSHOT_CHECK`) | **Pass** on an empty cache (the welcome flow), and on a seeded 2,000-message cache with `-Select 0` (list and reading pane have content). Also pass with `-Warp` (seeded, `-Select 0`) and with `-Lang pt-PT -Theme dark` (seeded, `-Select 0`). The first run on a GPU did not need the DirectComposition fallback. |
| One snapshot per action in `snapshot::ACTIONS` (15 besides `shell`: `settings-accounts`, `settings-account`, `settings-signatures`, `settings-rules`, `settings-general`, `settings-privacy`, `settings-about`, `rule-edit`, `contacts`, `contact-edit`, `contacts-sync`, `ob-welcome`, `ob-email`, `ob-sign-in`, `ob-manual`) | **Pass: all 15 exit 0 with the check passed, on both a seeded and an empty throwaway cache (30 runs).** The main view (`shell`) is the two runs above. The seeded main view was looked at: no calendar tab, the rail has Drafts, the folders, Contacts and Settings. |

### Budgets, for the record (not a gate)

`scripts/gpui-bench.ps1 -Mode timing -Runs 9 -Config gpui` on a copy of the seeded 2,000-message
cache (`cargo run --release --example seed`), offline, 1280x800. "Launch" is the first frame
drawn (`KOMOREBI_TIMING=1`, in-app); the list is drawn in that same first frame, so the cached
list has no separate figure (as in stage D).

| Run | In-app first frame, median (min to max) | Wall, median |
|---|---|---|
| GPU, run 1 | 240.3 ms (192.7 to 406.9) | 264.1 ms |
| GPU, run 2 | 613.9 ms (235.5 to 1146.5) | 724.8 ms |
| WARP, run 1 | 176.9 ms (152.3 to 369.5) | 260.9 ms |
| WARP, run 2 | 147.7 ms (134.0 to 216.6) | 166.8 ms |

Stage D measured 77.0 ms on WARP on a quieter machine. These runs are 2 to 8 times slower and
vary 4-fold between runs of the same exe, which is the other processes on this machine (a game,
browsers and several agents were running), not a change in the app; a single boot-check run
printed 141 ms first frame on the same machine. The 200 ms launch budget is met in the best
run and not in the others; a calm-machine measurement is still needed before the number goes
into any release note. The cached-list budget (600 ms) is bounded by the same first-frame figure
(met in 3 of 4 medians).

### By hand, for the owner (nothing here was done)

- Narrator over the reader's lines and links, the composer (fields, forms, the trust dialog and
  its flow), and the onboarding and settings forms.
- A real Japanese and a real Chinese IME in the composer and the search box (candidate window
  position, conversion, cancel).
- The clipboard HTML pasted into Word and into a browser.
- 125 and 150 percent scaling on a real display, and 150 and 225 percent beyond the gallery.
- Touchpad scrolling; a page wider than the column (sideways scroll).
- The tray, toasts and close to tray on a real taskbar; Restart now and Remove all data.
- The real network paths of onboarding (autoconfig, sign in) and of the updater.
- Windows 10 (the system caption path) and a machine with no GPU (the DirectX pre-flight, exit 3,
  and WARP in real use); neither is covered by any section of this page, so both are new gates.
- The 1.0 installer pages on a clean machine, and the 0.7 to 1.0 update through the signed
  updater.
- Dropped, not gated: the calendar (Phase 10), its one test (row 43, `times_step_by_quarters`),
  and high contrast.

### Remaining before release (nothing was started)

- Decide the exe size: cut 887,712 bytes or move the 21,500,000 ceiling (CI and `release.yml`
  fail until then).
- (Done at the Cut-over: the version is 1.0.0 in the root package, the GPUI crate and `Cargo.lock`;
  the 0.7 `window` key is ignored; the Slint app is deleted. The pass over `docs/USAGE.md` is done in
  "Gaps: other".) New screenshots remain.
- The first runner run of `ci.yml` and `release.yml` (Blacksmith, fxc, WARP boot checks, the UIA
  check, `actionlint`); none of it has run.
- Code signing through SignPath, the `v1.0.0` tag, the GitHub release, and pushing `main`.

### Not done (release candidate checks)

- No calm-machine timing, no memory or idle-CPU run, and no UI Automation check
  (`scripts/gpui-uia-check.ps1`) were made here.
- The exe was not stripped or otherwise cut; the size result is the plain `release` profile build.

### Re-run on main after the cut-over and the gaps integration (2026-11-06)

Run in `D:\Projects\komorebi` on `main` at `3f604ab` (plus the one test fix below), with
`CARGO_TARGET_DIR=C:\dev-overflow\cargo-targets\komorebi-integ`. The exe is `komorebi.exe`
(the `komorebi-gpui.exe` of the table above is a stale file in the same folder). The machine was
busy with other builds, so no timing was taken here. Nothing was pushed, tagged, signed or released.

| Check | Result |
|---|---|
| `cargo xtask check` (fmt, clippy `-D warnings`, tests, lints; no release exe at that point) | **Pass, after one fix.** The first run failed: `window::tab_tests::a_message_tab_has_the_full_command_set` panicked in `test_support::seeded` ("UNIQUE constraint failed: accounts.email"). Two tests of different lanes opened the same throwaway cache name, `tab-commands` (`views::tabs::message::tests` and `window::tab_tests`), so under the parallel run they shared one folder. The merge put them together; the window test now uses `tab-full-command-set`. Second run: komorebi-gpui 453 passed and 1 ignored, plus 4 `brand_assets` and 2 `token_lint`; library 812 passed and 1 ignored; xtask 9; GPUI lints ok. |
| Full `cargo test --workspace --locked --no-fail-fast` | **Pass.** 1,359 passed, 0 failed, 2 ignored over 19 test binaries. |
| Release build `cargo build --release --locked -p komorebi-gpui` | Built (4 min 37 s). |
| Exe size, `cargo xtask size` | **`komorebi.exe` is 18,759,168 bytes, under the 21,500,000 ceiling (2,740,832 left).** Down from 22,387,712 at the release candidate checks: the Slint tree is gone from the build. The size cut proper is the next run; the ceiling is unchanged. |
| Boot checks (offline, throwaway caches under `C:\dev-overflow\claude-tmp\integ\boot`, `KOMOREBI_SNAPSHOT_CHECK=1`, run by hand because `scripts/*.ps1` may be blocked by the execution policy) | **Pass, 6 of 6 exit 0:** the empty cache on GPU (light, English), on WARP and in pt-PT dark, and the seeded 2,000-message cache (newest message open) on GPU, on WARP and in pt-PT dark. The seeded pt-PT dark capture was looked at: no calendar tab, the rail has Rascunhos, the folders, Contactos and Definicoes, and the list and the reading pane have content. |
| One snapshot per action in `snapshot::ACTIONS` (16 besides `shell`, with `compose`) | **Pass: all 16 exit 0 with the check passed on a seeded, an empty and a 0.7-style cache (48 runs).** A removed action (`calendar`) is refused with exit 2 and the list of what the app knows. |
| A cache in the 0.7 schema | **Pass: the main view on GPU, WARP and pt-PT dark (3 runs) and the 16 actions above all exit 0.** No 0.7 fixture exists and the schema did not change (v18, pinned by `db::tests::a_07_cache_opens_in_1_0_with_everything_in_it`), so the cache was made from the seed (`PRAGMA user_version` 18) and given what 0.7 wrote and 1.0 must ignore or keep: the `window` setting `10 20 1280 820 0`, `theme` dark, and a calendar and an event row. It starts, shows the cached messages and does not crash. A real 0.7 cache from a real install was not available and was not touched. |

What the 1.0 start does on a 0.7 cache, as built: it keeps the accounts, the cached messages, the
rules, the contacts and the settings (no conversion and no re-download: the schema is the same);
it does not read 0.7's `window` key, so the first start opens at the default place; the calendar
tables stay in the file unseen. The owner allowed a fresh message cache and a new download; the
code does not need it, so none was written.

Not done: no real 0.7 install was opened, no timing, memory or UI Automation run, and the by-hand
list above is unchanged.

## Cut-over (done on main, local only)

The step the owner decisions of 2026-11-06 asked for: the Slint app is deleted, the app is
`komorebi.exe` at version 1.0.0, and the cleanup that the cut-over preparation listed is done,
except the calendar library code, which stays. Nothing was pushed, tagged, signed or released.

### What was done

1. **The Slint app is gone.** Deleted: `src/main.rs`, `src/app.rs` and `src/app/` (the whole
   Slint controller, 6,210 + 1,985 lines and its modules), `src/tr.rs`, `ui/` (13 `.slint` files,
   `ui/tr.slint` included), `crates/komorebi-ui`, the root `build.rs`, `tests/shortcuts_table.rs`
   (its GPUI form is test row 36), the Slint scanner of `tests/i18n_catalog.rs`, the `slint`,
   `komorebi-ui`, `rfd` and `embed-resource` dependencies of the root package (`Cargo.lock` lost
   about 1,860 lines: the whole Slint, winit and accesskit trees), the release profile entries of
   Slint's crates, winit, accesskit and muda, `komorebi::i18n::select` (its only job was Slint's
   catalog), the Slint size, boot-snapshot and manifest steps of `ci.yml`, `xtask size --slint`,
   the Slint pin check of `scripts/check-pins.ps1`, the Slint config of `scripts/gpui-bench.ps1`,
   the `pin-utils` ignore of `deny.toml` (`pin-utils` is not in the lock any more) and NOTICE's
   Slint paragraph. `deny.toml` also lost the `GPL-3.0-only` and Slint royalty-free allowances,
   which no crate needs now.
2. **Kept and moved.** `src/chrome.rs` was mostly the Slint title bar's window procedure; the
   GPUI app uses only `custom_wanted`, so the file is that function, `WINDOWS_11` and the
   test. Everything else the app and its tests use was already in the library (`format`, `rows`,
   `tabs`, `compose`, `reader`, `files`, `rules_page`, `contacts_page`, `crash`, ...). The root
   package now has no binary: it is the library, and `cargo build` at the root builds it.
3. **Pot and catalog.** The template comes from the Rust sources only (1,094 messages); the 191
   pt-PT entries only the Slint app used were removed from the `.po`; the untranslated-literal
   allowlist is empty (it listed Slint literals only). `invite::record` and the calendar library
   stay.
4. **Brand assets moved up.** `assets/brand-1.0/komorebi.ico` and `komorebi.rc` replace
   `assets/komorebi.ico` and `komorebi.rc` (the rc finds `komorebi.manifest` beside it), `tray/`
   moved to `assets/tray/` and the master SVGs to `assets/brand-src/`; `scripts/brand-assets.js`,
   the brand tests, `win_shell`'s rc test and `crates/komorebi-gpui/build.rs` follow.
   `installer/komorebi.iss` has the 1.0 icon and wizard images unconditionally (`/DBrand10` and
   `build-installer.ps1 -Brand10` are gone).
5. **Continuity for 0.7 users.** The release exe is named `komorebi.exe`: the `[[bin]]` of
   `komorebi-gpui` is now `komorebi` (its package keeps the name), so the build, the CI steps,
   the installer, the portable zip and `komorebi.pdb` all use that one name; nothing is renamed
   at packaging any more. Checked against the 0.7 code (`git diff 8d3e009`, the `Version 0.7.0`
   commit): the single-instance mutex `Local\Komorebi-<cache>`, the window property
   `Komorebi.Instance`, the installer's `Komorebi.App` mutex (`AppMutex`, held by the app at
   start and released before an update), the Run value `Komorebi` with `--minimized`, the
   `Software\Clients\Mail\Komorebi` and `Komorebi.Url.mailto` registrations, the Restart Manager
   filter `komorebi.exe`, the installer's `AppId` and install path, and the notification path
   (the tray balloon: there is no AppUserModelID, 0.7 had none either) are library code or
   installer text that did not change, and `installer/komorebi.iss` itself is unchanged apart from
   the icon branch. The 0.7 updater downloads the signed 1.0 installer and runs it as it ran any
   other; that path is untested end to end (see "Not done").
6. **Version 1.0.0** together: the root package, `komorebi-gpui`, `komorebi-i18n`, `Cargo.lock`
   (the exe's version resource, About, the update check's `current()` and the installer name
   read the package version). A new test (`about::tests::the_app_and_the_library_have_one_release_version`)
   fails if the app's and the library's versions differ or if the version is a pre-release the
   update check cannot parse (`1.0.0-alpha.0` read as 0.0.0). The CHANGELOG heading stays
   `[1.0.0] - Unreleased`.
7. **The window place.** `Settings` has `gpui_window` only: the `window` field and its parser arm
   are gone, so 0.7's key is never read and there is no converter. The key keeps the name
   `gpui_window` on purpose (a plain `window` would read 0.7's physical pixels as logical ones).
   Test: `db::tests::general_settings` sets `window` and sees `None`.
8. **The 0.7 cache.** Decided with the code: 1.0 **keeps** it. `git diff 8d3e009` shows the cache
   schema is unchanged (v18; the only `db.rs` change is the `gpui_window` setting), and the
   library that reads it is the same, so a 0.7 cache opens as it is: accounts, cached messages,
   rules, contacts and settings carry over, nothing is converted and nothing is downloaded again,
   and the passwords stay in Credential Manager. Dropping the message tables to force a download
   would delete the user's data for no gain, so no reset was written. The test
   `db::tests::a_07_cache_opens_in_1_0_with_everything_in_it` pins schema v18, opens a throwaway
   file the way the app does, and checks that the accounts, the messages and the settings are
   there, that 0.7's `window` is not read, and that the calendar tables are untouched. If the
   schema is ever raised, that test fails first and the 0.7 cache needs a decision. The CHANGELOG
   and PRIVACY say the cache carries over (they say so instead of "mail is downloaded again",
   which would be false).
9. **Docs.** README, CONTRIBUTING, NOTICE, `docs/BUILDING.md`, `GPUI-BUILD.md`, `GPUI-TESTING.md`,
   `GPUI-TEST-PORT.md`, `GPUI-I18N.md`, `TRANSLATIONS.md`, `ENVIRONMENT.md`, `RELEASING.md`,
   `SCOPE.md`, `PERF.md`, `BRANDING.md`, TODO and the CHANGELOG no longer describe a Slint app or
   two apps. The calendar is described as left out of 1.0 and planned for 1.1.0.
10. **CI and release.** `ci.yml` builds `-p komorebi-gpui` in release, checks the size and
    manifest of `komorebi.exe` and runs the two WARP boot checks and the UIA dump on it;
    `release.yml` packages that exe under its own name and installs without `-Brand10`.
    `cargo xtask size` has one gate (21,500,000 bytes); `cargo xtask check` skips the size step
    when no `komorebi.exe` is in the release folder (the old `komorebi-gpui.exe` is not looked at).

### Checks

All green on the cut-over tree: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets --locked -- -D warnings`, `cargo xtask check` (its size step skipped: no release
build), `cargo test --locked --workspace --no-fail-fast` (19 test binaries, 1,265 passed, 0
failed, 2 ignored), `cargo deny --workspace --locked check licenses advisories bans sources`, and
`scripts/check-pins.ps1` (run as `pwsh -File`; it passes).

### Not done (cut-over)

- **The exe is not rebuilt in release and its size is not measured**: the last measurement
  (22,387,712 bytes, taken while the library still depended on Slint through
  `i18n::select`; the app never called it, so the linker most likely dropped it, but that is not
  checked) is over the 21,500,000 ceiling by 887,712, and the size cut is the next run. The
  removal of the Slint dependency and of 191 unused pt-PT entries can only shrink the exe; by
  how much is not measured.
- Not run: the 0.7 to 1.0 update through the signed updater, the 1.0 installer over a 0.7 install
  (the installer text is unchanged, but nothing was installed), `release.yml` and `ci.yml` on a
  runner, `actionlint` (not installed; the workflow edits were checked by reading).
- `scripts/*.ps1` were edited; only `check-pins.ps1` was run (it passes). The boot, UIA,
  snapshot, bench, manifest and installer scripts were not run (they launch the app or need a
  release build).
- The library still holds code only the Slint app called (parts of `win_shell`, `theme`,
  `launch`, `instance`, `contacts_page` ...). It is `pub`, so nothing warns; pruning it is
  optimization-pass work, not done here.
- Comments in the library and the GPUI crate that say "the Slint app" (history of where a
  function came from) are left as they are.
- The calendar library code has no user of its own now (`invite::record` is not called by the
  app); it is built and tested, which is the point.

## Gaps: library (done on gpui/e-library)

One gap was open for the library after the cut-over: opening a cancelled invitation still wrote
the calendar tables.

### What was fixed

- **`invite::card` no longer writes.** It called `db.cancel_event_uid` every time the card for a
  cancellation was built, so opening a message in the 1.0 app (which has no calendar screen)
  changed `events` rows nothing shows. The card now only reads. The write moved to
  `invite::mark_cancelled(db, ev)`, which keeps the same rules (a cancellation of the whole
  series only, a lower SEQUENCE than the stored copy changes nothing) and returns whether it
  marked a copy. No app calls it in 1.0; the 1.1.0 calendar calls it when the open message is a
  cancellation, next to `record` and `remove`.
- **What the 1.0 app writes to the calendar tables is now nothing.** `queue_reply` writes the
  outbox, `record_answer` writes `rsvp`; `record`, `remove` and `mark_cancelled` are the calendar
  half and are not called. A search of the library and the GPUI crate found no other caller of
  the calendar writes (`store_invite`, `save_event`, `remove_event_uid`, `decline_occurrence`,
  `cancel_event_uid`, `set_found_calendars`, `rebuild_occurrences`).
- **Kept on purpose:** `card` still reads `event_by_uid` for `Card::in_calendar` and the
  `Tool::Remove` entry. On a fresh 1.0 cache the table is empty, so both are false; on a 0.7
  cache that is opened as it is, an old copy can make `Remove` appear in the card's tool list.
  The GPUI card filters `Remove` out (`views/reader/invite.rs`), so no dead button shows. The
  calendar returns in 1.1.0 with these fields as they are.

### Tests

- New: `invite::tests::opening_a_cancellation_does_not_write_the_calendar_tables` (a copy stored
  earlier is not marked cancelled by building the card of its cancellation; `mark_cancelled`
  marks it once, and does nothing for a request).
- Unchanged and green: the 0.7-era card test, the GPUI-answer test (no calendar rows), the
  `cal::store` cancel test, `tests/invite_corpus.rs`.

### Not done (gaps: library)

- Nothing from the gap. The reader view's `Tool::Remove => {}` arm (`views/reader/mod.rs`) is
  reader-lane code and stays an empty arm until the calendar returns.

## Gaps: other (done on gpui/e-other, with gaps)

The fix lane for what belongs to no screen, after the cut-over: the snapshot switches, the open test
port rows, the stale user docs and the access keys. Nothing was pushed, tagged or released. Files of
other lanes were not edited: what they need is listed as integrator requests.

### What was done

1. **Snapshot switches** (`snapshot.rs`, new `snapshot_drive.rs`). `KOMOREBI_SNAPSHOT_FOCUS`
   (`list`, `nav[:n[:keys]]`, `toolbar[:n]`, `tab:n`, `press:keys`), `_HOVER` (`x,y[:press|:click]`,
   `caption-min|max|close[:press]`), `_SCROLL`, `_WALK` (Down n times in the list, one capture) and
   `_TITLEBAR=system` are read, parsed and driven on the UI thread between the first frame and the capture, with the
   window's own input path (`Window::dispatch_keystroke` and `dispatch_event`); the `compose` action
   opens the composer. Table in `docs/GPUI-TESTING.md`. 4 window tests and the parser tests; no real
   windowed snapshot was run in this lane.
2. **Test port rows** (`window/port_tests.rs`, 20 tests, none ignored, and `wake.rs`, 3 tests).
   Done: 7, 77, 100, 109 (Edit), 142, 143, 153, 156, 171, 174, 242 to 244, 284, 285, 295, 302 to 305,
   308 to 315, 160 (closed as not applicable); partly: 107, 163, 170, 172. Open with a reason: 101 to
   104 (composer windows are not built), 154, 155 and 164 (roving tab stop), 157 (dialog Enter),
   175 to 177 (teaching tips). The table in `docs/GPUI-TEST-PORT.md` has the exact status of each
   row. Row 174 is ported as two tests (the Back button and Undo offer; both panes on a desktop,
   one at a time below 720 px, Enter and Esc); the Slint cases "a selection alone keeps the list"
   and "arrows in one-pane mode" have no GPUI counterpart (the arrows open the message they reach).
3. **Wake from sleep** (`wake.rs`, `views/mail/hub.rs`, row 7). `woke` (a gap above 120 s between
   two 60 s checks; a clock set back is no wake) and `watch`, a task that tells its owner
   `Wake::Woke` at once and `Wake::Settled` 3 s later. `Hub::start` runs it: `Woke` calls each
   worker's `resumed()` (dead connections are dropped), `Settled` syncs every account (in full when
   one is due) and hands over the Outbox. Tested on a clock the test moves, and the hub's side with
   captured workers; a real sleep and wake was not tried.
4. **Access keys** (`access.rs`). Mark as read, Mark as unread and Save as .eml are keys now (English
   M, U, S; Portuguese C, D, E), so Alt and the letter run the More menu's and the row menu's actions
   wherever the message commands are available. To make room, Portuguese Settings moved from Alt+D to
   Alt+F and Refresh from Alt+U to Alt+Z (the letter is in "Definições" and "Atualizar"); the tests
   that the letters of keys that meet differ and that each letter is in its name pass for all 18.
5. **User docs.** `docs/USAGE.md` describes the 1.0 screens and what is built (no promise of one Tab
   stop for the reader's toolbar, the access keys, the Esc order as the app does it). The stale
   statements of the cut-over preparation, the release candidate checks and stage D above are marked.
   Comments that say "the Slint app" are history of where a function came from and were left (the
   Cut-over said so).

### Decisions

- A test that needed a patch to a file of another lane was first written against the right
  behavior and `#[ignore]`d. The fix round applied the four patches (below) and removed the
  `#[ignore]`s: no test of this lane is ignored.
- Row 160 is closed as not applicable: the GPUI command bar does not drop labels by width.
- Row 157 is not a test gap but a behavior gap: Enter runs the action of a destructive dialog.

### Integrator requests

Done in the fix round: 1 to 4 (the patches below were applied, the tests un-ignored; request 4 is
the hub's wake watch, tested in `views::mail::sync_tests`). Still open: 5 to 8.

1. (done) `window/tab_host.rs` (rows 303 reopen and 304; two edits). Before `c.open(None, window, cx)` in
   the close path keep the row (`c.save_all(cx); let row = saved_row(c); c.open(None, ...)`), make
   `saved_row` read `c.drafts.active().and_then(|d| d.row)` (it read `c.draft()`, which is `None`
   for a new draft), and in `select_tab`, when the tab changes and the active tab is a draft, call
   `view.update(cx, |c, cx| c.save_all(cx))` before `set_active`. Then remove the `#[ignore]` of
   `a_closed_draft_tab_comes_back_with_ctrl_shift_t` and
   `a_tab_switch_flushes_the_autosave_of_the_leaving_draft`.
2. (done) `views/mail/model.rs` (row 302). In `select_message`, replace
   `if self.draft_open.take().is_some() { self.effects.push(Effect::CloseComposer); }` with
   `self.draft_open = None; self.effects.push(Effect::CloseComposer);`, because a New mail has no
   `draft_open`, so the pane's dirty draft was never moved to a tab. Remove the `#[ignore]` of
   `selecting_a_message_moves_a_dirty_pane_draft_to_a_tab`.
3. (done) `views/mail/mod.rs` (row 171). At the end of `MailView::escape`: if `done == Escaped::Nothing`,
   `self.hub.update(cx, |hub, cx| hub.dismiss(cx))`. Remove the `#[ignore]` of
   `escape_dismisses_the_status_line_last`.
4. (done) `views/mail/hub.rs` and `wake.rs` (row 7). In `Hub::start`, keep
   `wake::watch(SystemTime::now, |hub, wake, cx| ..., cx)` in a field; on `Wake::Woke` call
   `resumed()` on each worker (`src/mail/mod.rs`), on `Wake::Settled` call `sync_all(full)` with
   `full = last_full.is_none_or(|t| t.elapsed() >= FULL_SYNC_EVERY)` and `drain_outbox(cx)`.
5. `widgets/dialog.rs` (row 157). Put the focus on Cancel when a destructive dialog opens and do not
   bind Enter to the action of a destructive one.
6. `widgets/button.rs` (rows 154, 155, 164). A roving tab stop (one Tab stop for the reader's
   command bar and the composer's format row, Left and Right inside).
7. Access keys: the More menu and the row menu items, the list rows and the Settings sections show no
   letter yet. The keys work as Alt and the letter; drawing the letter beside a menu item is a Kit
   menu change (a mnemonic on `PopupMenuItem`), and Settings' sections need `access_key` on their
   buttons (`views/settings`).
8. Teaching tips (rows 175 to 177) and composer windows (rows 101 to 104) are features, not tests:
   the lanes that own them write their tests with them.

### Checks

`cargo xtask check` (fmt, clippy `-D warnings`, komorebi-gpui, library, xtask, lints) passes; the
size step is skipped (no release build). No windowed snapshot was run here.

### Fix round (review of gpui/e-other)

The four regressions against the Slint app that the lane had only `#[ignore]`d are fixed: Select
a message with a dirty New mail in the pane (the draft goes to a tab), Ctrl+Shift+T on a closed draft
tab, a tab switch saving the leaving draft, and Esc dismissing the status line. The hub starts the
wake watch (row 7). `docs/USAGE.md` again says Esc ends with the status line; it no longer promises
Left/Right along a toolbar (not built) and says the menu items' keys tell no screen reader their
letter. The Shortcuts dialog lost its "Left / Right: Move along a toolbar" row, which its own test
(`the_dialog_lists_the_readme_keys`) found out of step with USAGE.md; the catalog entries went
with it. `FOCUS=toolbar` is documented as one Tab and n Rights with no roving stop behind it, and
the hover/wheel test says what it does not check. Row 174's second Slint test is ported (see above).

### Not done (Gaps: other)

- Rows 101 to 104, 154, 155, 157, 164 and 175 to 177 (above); the roving tab stop is also why
  `FOCUS=toolbar` cannot move along the buttons.
- Row 107's "Sent" after a delivery, the arrow keys in the filters.
- Access key chips beside menu items and on the Settings sections (request 7): Mark as read, Mark
  as unread and Save as .eml have a key and no chip, and tell UI Automation no letter.
- New screenshots for the README.
- A real snapshot with the focus, hover, scroll and walk switches (the switches are tested; no
  window was opened). The caption hover and `:press` are platform hit-tested and the scroll offset
  is not asserted anywhere.
- The Portuguese letters Settings Alt+F and Refresh Alt+Z moved in this lane against the first
  table, but access keys are new in 1.0 (CHANGELOG, Added), so no released version has the old
  letters.

## Gaps: shell (on gpui/e-shell, with gaps)

The shell gaps left by stage D and the cut-over, closed where the shell's own files allow it. Each
fix has a test; the rows of docs/GPUI-TEST-PORT.md it ports say so. Nothing here launches the app.

### What was fixed

- **Message tabs and windows have the reader's command set.** Reply, Reply all, Forward,
  Archive, Delete, a Move to menu (the reading pane's entries, `Mail::message_move_targets`:
  translated role names, tree order and levels, the recent folders first, never its own folder or
  Drafts; a move from a tab or window is a recent folder too), Mark read or unread, Important,
  and More (Forward as attachment, Report junk or Not junk, Open in new window). The bar's state
  is pushed by the owner (`MainWindow::message_state`) when the view opens, on a read or flag
  change and on every cache change, never read on a redraw. Every command acts on that message
  through the list's own paths (Undo, server job)
  whatever the list shows, and a move or a junk report closes the tab or window once the message
  left (`a_message_tab_has_the_full_command_set`, `report_junk_from_a_tab_moves_the_message`).
  Report junk from a tab's or window's reader no longer does nothing.
- **Save and Move glyphs**: Fluent `save` and `folder_arrow_right` (1.1.292, NOTICE updated),
  `Icon::Save` and `Icon::MoveTo`; the message view uses them (the mail and reader uses are
  integrator requests).
- **Draft windows** (`PopoutKind::Draft`): a draft tab's menu on the tab strip (right click) has
  Open in new window (the composer's own More does not offer it yet, and with the tab strip
  keyboard model not started a draft window needs the mouse); the draft moves to a window of its
  own with its OS title following the subject (set only when it changes); closing that window, a
  tab switch, a tab close and quitting save the draft (an empty one goes); Undo of a send from a
  window reopens the draft in a window, from a tab in a tab; closing the main window saves and
  closes the draft windows first. Rows 101, 102, 104 (partly), 303 to 305, 308. Tested: the
  quit and main-close saves; the draft window's own close button (`on_window_should_close`,
  `park_composer`) is not driven by a test (the test platform does not send an OS close).
- **Window size and cascade**: a new message or draft window opens 28 px down and right of the
  last one (8 steps, then from the start again) at the size the last one was closed at (for the
  session; see below), `views::windows::popout::tests::windows_of_their_own_cascade`. Every close
  keeps the size: the OS close button, Esc and Close, a move, Send and Discard
  (`popout::close_keeping_size`); only the cascade arithmetic is tested.
- **Per-tab view** (row 295, partly): each tab keeps its own view entity across tab switches
  (`tab_switch_keeps_each_tabs_own_view`), which is where its scroll and view flags live; the
  test checks the entities, not a set scroll offset or find bar.
- **Tabs follow the cache**: a vanished folder or a removed account marks its message tabs gone,
  open or in the background (rows 311 to 314); Delete in a message tab acts on that message, never
  on a selected draft row (row 310); a language switch reaches message windows (row 309, the
  language half); Settings from a message tab keep the pane (row 315, the pane half).
- **Teaching tips** (rows 175 to 177): the three tips of the 0.7 app (Your folders, Search, Keys
  and help) over the main window, 1.5 s after start, one at a time, following the layout (the
  compact series skips the folders tip while the rail shows); Got it moves on, Skip tips ends the
  series for both layouts, Esc closes only the tip; Settings > Show tips again restarts them;
  What's new waits until no tip shows. On a first run (no account at start) the tips arm again on
  the first frame with an account, as the 0.7 app started them when the list first showed rows
  (`a_first_run_starts_the_tips_once_it_has_an_account`). English and pt-PT strings; the template
  is regenerated.
- **Dialogs are named, destructive confirms are alert dialogs**: the vendored Kit gets
  `Dialog::a11y_label` and `Dialog::alert`, and gpui-base is vendored (exact 0.8.0 source) for
  the host's label (`third_party/*/KOMOREBI-PATCH.md`); `widgets::dialog::confirm` names its host
  by the title and makes a destructive confirm an AlertDialog. `Cargo.lock` loses gpui-base's
  registry source and checksum (the path patch).
- **The select says whether it is open**: `widgets::select::Dropdown` is collapsed, and expanded
  while its menu is open (`Button::expandable`, through Kit's `Selectable::open`; UIA's
  ExpandCollapse pattern). It stays a Button node: the ComboBox role is an integrator request,
  as the settings lane's test asserts the Button role.
- The message tab's view is a tab stop.
- **The snapshot tool's `compose` action** (refused before): New mail, the composer in the
  reading pane (`the_compose_snapshot_action_opens_the_composer`; not run as a snapshot here).

### Not done (gaps: shell), with why

- Tab stops on the mail, reader, composer, editor, contacts and rules focus handles, the reader's
  and mail's Save and Move glyphs, a dirty New-mail pane draft moving to a tab when a message is
  selected (row 302, the mail half), F6 beside Settings (row 315, the F6 half), the placeholder for
  a popped-out draft in the pane (row 103), the Drafts row's Open in new window (row 104, the row
  half), and the fixed pixel heights in `views/mail/rows.rs`: other lanes' files, sent as exact
  integrator requests.
- The window size is remembered for the session only: the library's settings table has no
  generic key the shell can write without a library change.
- Tab strip keyboard model (arrow keys between tabs, the close buttons and the tab menu reachable
  without a mouse) and tab drag reorder: not started in this run.
- The snapshot tool's FOCUS, HOVER, SCROLL, WALK and TITLEBAR switches stay ignored (with the
  note on stderr): not started in this run.
- The theme half of row 309 is not tested (every window reads the same global tokens).
- By hand, not checkable in code: Narrator saying the dialog title and "alert dialog", the tips'
  placement and look, the owner's visual review points, the cascade on a multi-monitor desktop.

## Gaps: mail (gap lane `gpui/e-mail`)

The three mail gaps left after the cut-over, in `views/mail/`, `views/tabs/message.rs` and
`window/tab_host.rs`.

### What was done

- **Message tabs and windows have the list's commands** (Stage D shell "Not done"). The command
  bar of a message tab or window now has the reading pane's groups: Reply, Reply all, Forward;
  Archive, Delete and a Move menu button (the account's other folders, the recent ones first, a
  separator, then all of them, flat); Mark as read or unread and Mark as important (toggled);
  Open in new window (tabs only); More: Forward as attachment, Report junk or Not junk (by the
  message's folder), the reader's view toggles (plain text, original colours, load or stop
  images) and the source commands (Save as .eml, Show original, Copy Message-ID), which the tab's
  own reader runs. The bar's state (read, important, in Junk, has a folder to move to) is read
  from the cache when the view is made and on `MessageView::refresh`, not on every frame; the
  Move menu's folder list is read only when it opens. The window refreshes every message tab and
  window on the cache's changes (`mark_gone_tabs`: syncs, the mail view's `Changed`) and when the
  mail view sets read or important marks (`Mail::flags_seq`, followed by `follow_flags` from the
  mail view's observer), as the Slint app's `popouts_flags_changed` did.
  - `triage::Quick` gained `Move(folder)`, `Junk` and `NotJunk`; `Mail::act_on_message` runs a
    `Quick` on one message whatever the Mail tab lists (a tab's Archive used to do nothing while
    the Mail tab showed Drafts or the Outbox); `quick_action` keeps the row-only guard. A move
    from a tab is held with its Undo as the list's own; outside a message list (Drafts, the
    Outbox, whose rows are draft-table ids) `hold` and `undo` leave the rows and their selection
    alone, so a draft that shares the message's number is never hidden, opened or selected. Move
    notes its folder as recent. `triage::move_options_in` is the flyout's folder list over any
    folder slice (the tab reads the folders from the cache).
  - `MessageEvent` gained `ForwardAttached`, `Move { message, folder }`, `Junk`, `NotJunk`,
    `Flag` and `Read`. `MainWindow::message_action` routes them; a tab or window closes only when
    its message is held after the command (a Junk with no Junk folder, or a Trash delete waiting
    for its question, leaves it open; once confirmed, `follow_held` closes it).
- **The wake check (row 7)** is in the hub: every `WAKE_TICK` (60 s) the wall clock is compared
  with the last check; a gap over `WAKE_GAP` (120 s) calls `Workers::resumed` on every account
  (pooled connections dropped, IDLE cut and reopened) and, after `WAKE_SETTLE` (3 s), syncs every
  account (full when the last full sync is an hour old) and drains the Outbox. A clock set back is
  no wake. Not run under `Mode::Off`.
- **Dead `Command` arms.** `Command::CollapseAll`, `CollapseAllFolders` and `Refresh` were labels
  only (the folder line's menu runs them as `NavRun`s); they are gone from `Command`, the labels
  are `text::collapse_all(account)` and `text::refresh()` (the same msgids), and the empty arm in
  `menus.rs` is removed.

### Tests

- `views::mail::hub::tests::a_wall_clock_gap_means_a_wake` (row 7, the Slint test) and
  `a_wake_syncs_every_account_after_a_moment` (captured workers, the test clock).
- `views::tabs::message::tests::a_message_tab_has_the_list_commands` (names and roles, the clicks
  ask for their message, the bar follows the cache) and `a_message_tabs_move_and_more_menus`.
- `views::mail::tests::a_message_tabs_move_and_junk_act_on_its_message_only` (Move, Report junk,
  Not junk from Junk with the Mail tab on Drafts, the row guard) and
  `a_message_tabs_move_leaves_the_drafts_list_alone` (a draft with the message's id keeps its row,
  selection and composer through the move and its Undo).
- `window::tab_tests::a_message_tabs_list_commands_reach_the_mail_view` (Flag, Read, Junk with no
  Junk folder, Move with Undo, Forward as attachment), `a_message_windows_move_closes_it`,
  `a_message_window_closes_only_for_a_held_move` (asked from the window: Junk with no Junk folder
  and a Trash delete waiting for its question leave it open, Move closes it) and
  `a_message_windows_bar_follows_the_lists_marks`.

### Not done

- Row 160 (the reader bar's labels by priority) stays open: neither bar has labels to drop yet
  besides Reply; it needs the measured-width layout of the Slint bar.
- The tab's Move menu is a Kit popup menu without the flyout's filter or indents, and the tab's
  commands have no keys of their own (the list's keys act on the list, not on a tab).
- A tab's reader events other than the hub's, Mark read, Delete, Privacy, Resume reply, server
  draft Edit and mail links (Junk from the reader's banner, Why, attachments) are still dropped,
  as before.
- No new msgids: every label reuses an existing one. The .pot is not regenerated.
## Gaps: reader (lane `gpui/e-reader`)

The reader's open gaps after the cut-over. What needs a file of another lane is written down as
an exact change for the integrator.

### What was done

1. **The command bar is one Tab stop** (test row 154, `views/mail/reading.rs`): each button that
   shows and is enabled has a focus handle kept under its id; one of them is a Tab stop, the
   button last focused (a click or a key), else the first. Left and Right go along the bar and
   wrap, Home and End go to its ends; Tab leaves it, Shift+Tab comes back to the button last
   used. The bar is a toolbar named "Message commands" ("Comandos da mensagem"). Nothing in
   `widgets::button` changed: the bar gives each button its handle with `track_focus`.
2. **The bar's labels by priority** (row 160, in part): the mockup's bar has one label, Reply's.
   The bar measures itself and its buttons with that label each frame (a canvas, as the reader's
   column); while they fit, Reply keeps it, narrower it is an icon still named Reply, and it comes
   back when the bar widens (measured per label text, so a language change measures again). The
   Slint bar also labelled Archive, Delete and Move by priority; the GPUI bar does not, and that
   change has no owner decision yet (see Not done).
3. **The bar's overflow:** narrower than its icons (420 px with Back, a narrow two-pane reader),
   the last buttons (Flag, then Mark as read, Move, Delete, Archive, Forward, Reply all; never
   Back or Reply) leave the bar, its arrow order and Tab, and are the first group of More
   (`menus::bar_entries`, the same commands and names), so More stays in sight. Each button that
   can go notes where it ends with Reply as an icon (kept per set of buttons), so they come back
   as the bar widens without flicker.
4. **Enter and Escape pass the page** when it has nothing to do (`views/reader/mod.rs`): Enter is
   the page's only on a focused link, Escape only with a selection or a focused link; otherwise the
   action propagates, so Escape reaches the owner (Back to the list in the mail view, closing a
   message window).
5. **UI Automation text of the page:** the lines are `TextRun` nodes (they were labels) with
   their character lengths, each character's left edge and width (`TextLine::chars`, from the
   layout's clusters), where their words start (after a space, as AccessKit's examples) and the
   page's direction, so a text range has a rectangle (what Narrator highlights; AccessKit gives
   none without the character positions) and moving by word goes word by word. A line that wraps
   ends with a space and a paragraph's last line with a line break (`TextLine::ends_paragraph`),
   so text read across lines keeps its words apart; the page's value is what AccessKit builds from
   the runs. The worker maps the selection onto those lines as (line, character) with a
   `LineMap` built with the lines (`Doc::text_lines_upto`, capped at `MAX_TEXT_LINES`) and kept
   for the page, so a drag step is a binary search, not a walk of the layout; an end past the
   cap goes to the last line given, a selection starting past it has none. It sends it after the
   highlight as `Reply::TextSelection` (only when it sends the lines), the session keeps it
   (`Session::text_selection`, dropped with the page) and the page node carries it as its text
   selection (`views/reader/a11y.rs`). The composer's Preview uses the same nodes, without a
   selection. `the_text_pattern_reads_words_lines_and_rectangles` checks the nodes through
   `accesskit_consumer` 0.38 (what the Windows adapter reads; a dev-dependency only): the text
   across a wrapped line, word by word, line by line, a word's rectangle, a point to a position.
6. **Glyphs:** `Icon::Save` (Fluent `save_16_regular`) on an attachment's save button and the
   invite card's Save .ics, `Icon::MoveTo` (`folder_arrow_right_16_regular`) on Move in the
   command bar and the selection panel; both files are from `@fluentui/svg-icons` 1.1.292 as the
   others, and NOTICE counts 64.
7. **Copy Message-ID without the message:** `Db::message_id` reads the one column. The reader's
   Copy Message-ID no longer waits for (or downloads) a body: the Message-ID is in the cached
   header. `ReaderView::copy_message_id_of(id)` copies it for any message, open or not, for the row
   menu (integrator request below). Show original needs the source, so it still loads the message
   with its body; that is the data it shows, not a gap.

Tests: `tab_reaches_the_reader_toolbar_as_one_stop` and `reader_bar_labels_go_by_priority`
(`views::mail::reading`), `enter_and_escape_pass_the_page_when_it_has_nothing_to_do` and
`the_text_selection_reaches_the_session` (`views::reader`), `source_actions_wait_for_the_body`
changed (Copy Message-ID copies at once; Show original waits) with `copy_message_id_of`, two
a11y unit tests, `reader::worker::tests::the_selection_maps_onto_the_text_lines` and
`db::tests::message_id_reads_the_header_without_the_message` in the library. Test port rows 154
and 160 done, 141 noted.

### Not done (gaps: reader), with the exact change asked for

- **Junk from a tab or a window** needs the mail lane's `views/mail/triage.rs` (`Quick::Junk` and
  `Quick::NotJunk`, `quick_action` arms `Quick::Junk => self.move_out(Dest::Junk, targets)` and
  `Quick::NotJunk => self.move_out(Dest::NotJunk, targets)`) and the shell lane's
  `window/tab_host.rs` (`tab_reader`: `ReaderEvent::Junk { message, junk } => self.quick(*message,
  if *junk { Quick::Junk } else { Quick::NotJunk }, window, cx)`).
- **The save dialog's title** ("Save attachment", "Guardar anexo") needs the vendored GPUI (shell
  lane only): a `title: Option<&str>` through `Platform::prompt_for_new_path` (or a defaulted
  `prompt_for_new_path_titled`), `App::prompt_for_new_path`, the Windows platform's
  `file_save_dialog` (`IFileDialog::SetTitle`) and the test platform; then `save_file` in
  `views/reader/actions.rs` passes it.
- **Enter from the reading pane into the page** is the mail lane's: the reader now lets Enter and
  Escape pass, but `reading_focus` has no Enter of its own; a `MESSAGES` binding for "enter" that
  focuses `self.reader.focus_handle(cx)` while a message is open (and propagates otherwise) does it
  (the list's `OpenRow` is deeper and keeps Enter on the rows).
- **The row menu's Copy Message-ID** still opens the message first (`views/mail/menus.rs`
  `source_command`): for `SourceUse::CopyId` it can call
  `self.reader.update(cx, |r, cx| r.copy_message_id_of(id, cx))` and return.
- **Owner decision asked (row 160):** the Slint bar labelled Reply, Archive, Delete and Move
  by priority, measured per language; the GPUI bar follows the mockup and labels Reply only.
  Either that is the 1.0 look (record it under Owner decisions) or the bar gets the other labels
  back by priority.
- Not tested by hand: the roving stop with Narrator, the text pattern (highlight, words) and the
  text selection with Narrator itself; the AccessKit consumer test is what checks them.

## Gaps: composer (done on gpui/e-composer)

Branch `gpui/e-composer` (from `main` 2a5b5c0), the composer lane of the gap run after the
cut-over. Files: `views/composer/` (`controller.rs`, `mod.rs`, `sends.rs`, `toolbar.rs`,
`tests.rs`), the draft parts of `window/tab_host.rs`, `docs/GPUI-TEST-PORT.md`, TODO and this
section. No new sentence (no msgid): the windows use the tab and window titles that exist.

### What was done

1. **Composer windows (major; rows 101 to 104).** `MainWindow::open_draft_window` opens a draft
   in a window of its own (`PopoutKind::Draft`, the message window's `Popout` with its own
   `ComposerView`): the app-drawn title bar, the OS title following the subject ("Subject -
   Komorebi"), Esc or the composer's Close saving it as in the pane, and the title bar's Close or
   Alt+F4 doing the same through `on_window_should_close` (the window goes when its composer says
   `Closed`). Ways in: **Ctrl+O** in a composer (bound in the composer's key context; the pane's
   draft or a draft tab's), and the tab strip's "Open in new window" on a draft tab
   (`TabCommand::OpenWindow`, see the integrator request for the menu item). A draft leaves by
   its saved row (`ComposerView::pop_out`: a reply or forward not yet changed is kept too); an
   empty new message opens empty. What the window's composer says goes to the hub (status line,
   Undo offer, server copies, sends) and reloads the Drafts count and rows, as a draft tab's.
   The main window closing saves every composer window's draft first (`close_popouts`).
2. **Undo of a send from a window** brings the draft back in a window, not a tab or the pane
   (row 102): the tab host remembers the rows sent from composer windows while they wait
   (`window_sends`), and the mail view's "reopen in a tab" for a send from elsewhere opens a
   window for those.
3. **A draft is never open twice** (row 103, "comes forward once"): a saved draft asked for while
   a draft tab or a composer window shows it (Drafts list, Reopen, Undo, the reader's Resume)
   brings that tab or window forward (`raise_draft` in the tab host; `ComposerEvent::Raise` from
   the pane's composer, `Sends::holder` finds the composer). The Slint app's placeholder in the
   pane ("draft popped out") has no GPUI counterpart: the pane closes as on Esc.
4. **Tab switch saves the leaving draft (row 304).** `select_tab` flushes the autosave of the
   draft tab left, and of the pane's draft when the Mail tab is left
   (`ComposerView::flush_autosave`), instead of waiting for the two-second timer.
5. **A draft whose save failed stays in sight (major).** `set_aside` (and the new `hand_over`)
   keep such a draft shown in the pane with "Could not save the draft" and return `None`; Esc and
   Close (`park_composer`) keep it too; closing its draft tab keeps the tab. Before, the pane hid
   it with no saved copy and no tab. The mail view's `make_room` already reads `is_open()`, so
   New mail, `mailto:` and selecting another message see it stay.
6. **The formatting row is one Tab stop (row 155).** Bold, Italic, Underline, Bullets, Numbers
   and Link have focus handles of the composer's (`track_focus`) and only the one used last is a
   Tab stop; Left and Right move along the row (they wrap), Home and End go to its ends; a click
   makes the clicked one the stop. Signature and More stay their own stops, as in the Slint app.
   No shared widget changed (`IconButton` takes a tracked focus handle as any element).
7. **Recipient chips wrap.** Checked: `ChipField`'s chips already wrap (`flex_wrap`) and the row
   grows with them; the TODO line was left over from the Slint app. A test pins it.

Tests (`views::composer::tests::port`): `a_composer_window_keeps_its_own_draft_and_close_saves_it`,
`send_from_a_composer_window_once_and_undo_reopens_it_there`,
`a_popped_out_draft_comes_forward_once`, `a_draft_tab_opens_in_a_window_and_closing_saves_it`,
`tab_switch_flushes_autosave_of_the_leaving_draft`, `a_draft_that_could_not_be_saved_stays_shown`,
`the_formatting_row_is_one_tab_stop`, `recipient_chips_wrap_onto_more_lines`. The tab stop and
the flush tests were checked against the code without the change (both fail there).

### Not done (gaps: composer), with why

- **`aria_expanded` on the address fields.** Kit's `Input` has no `aria_expanded`, and only the
  shell lane may change `third_party/`. The exact Kit change and the composer side are in the
  lane's integrator requests; until then the suggestions are a named list box beside the field
  (stage D composer), but the field does not say it is expanded.
- **"Open in new window" on a draft tab's menu** is still disabled: `views/tabs/strip.rs` (shell)
  enables it for message tabs only. The command itself works (Ctrl+O, `TabCommand::OpenWindow`).
- **Ctrl+O on a row of the Drafts list** (the other half of row 104) is the mail view's (its
  `OpenInWindow` binding is on the list); `MainWindow::open_draft_window(Draft::Saved(row))` is
  what it would call.
- The Undo from a window path goes through the mail view's `set_aside`, which says "Draft kept in
  a tab" while the draft opens in a window; `hand_over` (no status) is the call for that path
  (integrator request).
- By hand: a composer window on a real display (its title bar, Alt+F4, Narrator over it), and the
  roving row with Narrator.

### Checks

`cargo xtask check` (CARGO_TARGET_DIR `C:\dev-overflow\cargo-targets\komorebi-e-composer`):
fmt, clippy `-D warnings` over the workspace, komorebi-gpui 371 passed (1 ignored, the typing
measurement), the library 809 passed (1 ignored), xtask 9, lint ok; the size check skipped (no
release build, as the run's rules say). The app was not launched.

## Gaps: settings (done on gpui/e-settings)

Three settings gaps left after the cut-over.

- **`SettingsEvent::CheckForUpdates` was a dead variant**: nothing emitted it. About's Check for
  updates goes through the updates card (`views/updates/card.rs`), which calls the updater
  directly, so the variant (`views/settings/mod.rs`) and its arm in `window/places.rs` are
  deleted. The compiler proves nothing else used it.
- **ManageSignatures opens Settings on Signatures and does not toggle it**: this was already done at
  the stage D integration. The composer's event goes to `MailEvent::OpenSettings` (pane composer)
  or `TabHost::show_section` (tab composer), and `show_section` opens Settings only if it does
  not show. The account page's own Manage signatures (`EditorEvent::ManageSignatures`) moves to
  the section inside Settings. The window test
  `privacy_and_signatures_open_their_settings_section` now also asks a second time while
  Signatures shows and checks that Settings stays open there.
- **Kit dialog host name and the address fields' expanded state**: not done in this app. Both need
  a change to the stack, which only the shell lane may make:
  - The dialog host is `gpui_base::Dialog` (gpui-base 0.8.0, from crates.io, not vendored). It
    builds its node with `role: Role::Dialog` and has no setter for a name. The fix is a
    `Dialog::aria_label(impl Into<SharedString>)` (and the same on `AlertDialog`) in gpui-base,
    passed through by `third_party/gpui-component/src/dialog/dialog.rs` (`BaseDialogRoot`), and
    then `widgets/dialog.rs` names the host with the title. That is Skua fork work.
  - Kit's `Input` (`third_party/gpui-component/src/input/input.rs`) sets its role
    (`accessibility_role`, `Input::role`) but has no `aria_expanded`. gpui-pre's `div` already
    has `aria_expanded`, so the fix is an `Input::aria_expanded(bool)` that sets it on the input's
    node, after which the composer's address fields pass `true` while suggestions show.

### Checks

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo xtask check` on gpui/e-settings (see the lane report for the result).

## Gaps: data (done on gpui/e-data)

Branch `gpui/e-data` from `main` at 2a5b5c0: the three minor gaps of "Not done (stage D, data)".
No blocker or major was open in this area.

### What was done

- **Sticky letter header in Contacts** (`views/contacts/list.rs`). Once the list scrolls, the
  letter of the rows at the top sits over the list on paper (`contacts-sticky-letter`), with the
  same look as the in-list header. The next letter's header pushes it up as it arrives, like
  Fluent's sticky list headers. `list::Entries` keeps each entry's top and the header positions
  with the sizes. `Entries::sticky` finds the letter by binary search on every frame and does no
  per-frame allocation. The overlay is not a heading: the header in the list is the one Narrator
  reads, so the letter is not said twice. It blocks the pointer, so a click on it does not reach
  the row hidden under it, but it lets the wheel through. The list area clips it.
- **Rules drop indicator, Fluent grip, grip name** (`views/rules/list.rs`, `mod.rs`).
  - While a card is dragged, a 2 px ink line in the gap shows where the rule lands. The line
    is above the card when the rule moves up onto it, below it when the rule moves down onto
    it, and absent over the rule's own card (`list::drop_edge`, the same rule as
    `RulesView::drop_rule`).
  - Each card's `on_drag_move` records the dragged rule (by id) and the card under the pointer
    (`RulesView::drop_at`). The line is drawn absolutely, so nothing moves during the drag.
  - The drop clears the line, and so does a drag that ends anywhere else (render drops it when
    no drag is active).
  - The grip is Fluent's `re_order_dots_vertical` 16 px regular glyph, copied byte for byte
    from the checked `@fluentui/svg-icons` 1.1.292 package (`Icon::ReOrder`). NOTICE now
    counts 63 bundled icons.
  - The grip is named for Narrator as an image, "Drag to reorder {rule}" ("Arraste para
    reordenar {0}"), with the same text as its brand tooltip. It is still not a tab stop: Move
    up and Move down in the "..." menu stay the keyboard's way.
- **Large SenderSpot initials** (`widgets/spot.rs`). The title step moved into the shared
  widget (`spot::initials_step`: title from `AVATAR_LG`, body from `AVATAR_MD`, caption below).
  The large read spot's initials are ink, matching the mockup's `.cd .av`. The contact page's
  local avatar is gone: `views/contacts/detail.rs` draws `SenderSpot::new(..).size(AVATAR_LG)`.
  The gallery's large spot now has title-size initials too.

### Checks

`CARGO_TARGET_DIR` `C:\dev-overflow\cargo-targets\komorebi-e-data`. `cargo xtask check` passes:
fmt, clippy over the workspace on all targets with `-D warnings`, komorebi-gpui 367 passed and 1
ignored, library 809 passed and 1 ignored, xtask 9, and the token and timer lints. The size check
was skipped (no release build). `cargo test --test i18n_catalog` fails only `template_is_current`
(see below).

New tests:
- `views::contacts::tests::port::the_letter_sticks_over_the_scrolled_list`: wheel-scrolls a
  42-contact list, checks that the overlay is at the list's top and is not a heading, that the
  wheel over it still scrolls, and that it is gone back at the top.
- `views::contacts::tests::rows::the_sticky_letter_follows_the_top_row`: the letter and the push
  at each edge.
- `views::rules::tests::port::a_drag_shows_where_the_rule_lands`: real pointer events, with the
  line above, below and absent, the drop clearing it, the order after each drop, and the grip's
  name and role.
- `widgets::spot::tests::the_large_spot_has_title_initials`: at 100, 150 and 225 percent.
- The pt-PT string in `views::rules::text::tests::repeated_controls_are_named_by_their_row_or_rule`.

### Review fixes (gaps: data)

- **Sticky letter one frame late (major).** The virtual list applies a pending
  `scroll_to_item` and keeps the offset inside the list only when it lays out, after the
  render that drew the sticky header, and nothing rendered again. Home from 'R' left 'R' over
  'A'. Now `ContactsView::scroll_to_row` sets the offset itself (`Entries::scroll_for_row`,
  from the entry tops and the viewport of the last frame), so the render after a key already
  has the new offset. The render keeps the offset inside the list too (`Entries::clamp`), for a
  search that shrinks it. Should the list still move during layout (the first frame, a new
  viewport), a canvas after the list asks for one more render (`request_animation_frame`).
- **Current row under the sticky header (major, WCAG 2.4.11).** `scroll_for_row` scrolls
  as Nearest, but a row that is not the first of its letter stops a header's height below the
  top edge, clear of the sticky header. The first row of a letter comes with its own header,
  as before.
- **Drag released off every card (minor).** `a_drag_shows_where_the_rule_lands` now also
  releases a drag over the "in order" heading with no move out first: no drop, no line, and
  `drop_at` is cleared. GPUI ends the drag and redraws the window on that mouse up
  (`third_party/gpui-pre/src/window.rs`, `cx.active_drag = None; self.refresh()`), which is
  what the render-time reset relies on.
- **Large spot ink and the contact page's avatar (minor).** `spot::read_ink` holds the ink /
  ink-2 choice and `the_large_spot_has_title_initials` checks it. `SenderSpot::id` names an
  initials spot; the contact page's is `contact-avatar`, and
  `the_list_searches_shows_and_writes_to_a_contact` checks it is 72 px and above the name.

New test: `views::contacts::tests::port::keys_and_search_keep_the_sticky_letter_true`. It
checks the first frame after Home from the bottom (no sticky letter), every Up from the bottom
(the current row whole in the list and below the sticky letter), and the first frame after a
search to one row (no sticky letter). With the old code it fails at each of the three steps.

### Not done (gaps: data)

- `translations/komorebi.pot` is not regenerated (that is the integrator's job). Until it is,
  `cargo test --test i18n_catalog` fails only `template_is_current`, on the one new msgid
  "Drag to reorder {0}". The pt-PT entry is in the catalog.
- No release build, so there is no snapshot of the sticky header, the drop line or the large
  spot, and no Narrator pass by hand (phase 13).
- `NOTICE` (the icon count, 62 to 63) is a shared file. If another lane adds an icon too, the
  merged count must be the folder's (`icons::tests::notice_counts_the_bundled_fluent_icons`
  checks it).

## Gaps integration (main)

The eight gap lanes (`gpui/e-library`, `e-other`, `e-shell`, `e-mail`, `e-reader`, `e-composer`,
`e-settings`, `e-data`) merged into main with `--no-ff`, in that order. Nothing was pushed or
tagged.

### What the merges had to decide

Three lanes built the same thing twice, from the same base. Each pair was reduced to one:

- **The message tab's command set** (shell and mail). The mail lane's `MessageView` stays: a bar
  cached in the view (`Bar`, `refresh()`), the More menu with the reader's view toggles and source
  commands, `MessageEvent::Move { folder }`, `Junk(id)` and `NotJunk(id)`, and the owner's
  `Mail::act_on_message` (the hold and Undo leave Drafts and Outbox rows alone; a move notes the
  recent folder). `follow_flags` refreshes every open bar when the list marks a message. The shell
  lane's push of a `MessageState` from the owner is gone (`set_state`, `message_state`); its
  `Mail::message_move_targets` stays as the reading pane's own list, and the test
  `a_message_tab_has_the_full_command_set` checks that the tab's Move entries equal it. The shell
  lane's `tab_stop(true)` on the view stays.
- **Composer windows** (shell and composer). The composer lane's are kept (`Raise`, Ctrl+O in the
  pane, a draft opened twice comes forward, `window_sends` keyed by the sending composer, `hand_over`
  on Undo, a window whose save failed is kept and the quit stops). From the shell lane they get the
  cascade and the size kept from the last window (`popout_options`, `close_keeping_size`), and its
  window tests run against them (`draft_window_numbers()` replaced `draft_windows()` in the test
  accessors). The pane draft saved at the main window's close is saved by `on_app_quit`, which the
  close leads to; `main_close_closes_popouts_then_flushes` now quits before it asserts.
- **The wake check** (other and mail). `wake::watch` (the other lane) stays; the mail lane's second
  wall-clock timer in the hub and its tests are removed. Row 7 is the other lane's.

Smaller unions: one `Save` and `MoveTo` icon, the `compose` snapshot action once (the Mail tab is shown
first), `SettingsEvent::CheckForUpdates` gone with `ShowTips` kept as the shell lane's
`show_tips_again`, the NOTICE icon count recounted (65), the `.pot` regenerated, the doc rows of
`GPUI-TEST-PORT.md` kept from the lane whose test or code survived.

The reader and mail lanes had review fixes in their worktrees that were not committed; they were gated
there (`cargo xtask check`, green) and committed on their branches before the merge.

### Integrator requests, done

- Open in new window in the composer's More menu (not in a composer window's own): `ComposerEvent::OpenInWindow`,
  mapped by the draft tab to `draft_tab_to_window` and by the pane to `MailEvent::OpenDraftWindow`.
  Test `window::tab_tests::the_composers_more_opens_it_in_a_window`.
- The expanded state of the address fields: `Input::aria_expanded` in the vendored Kit (patch noted in
  `third_party/gpui-component/KOMOREBI-PATCH.md`), passed by `ChipField::expanded` while that field's
  suggestions show. The suggestions test asserts it.
- The dialog host named by its title: already done by the shell lane (`Dialog::a11y_label`, the
  AlertDialog for destructive confirms); the settings lane's request is closed by it.
- Enter in a destructive confirm never runs its action (row 157, in part).
- `.pot` regenerated after the merge; the full `cargo test --workspace --locked` run.
- `USAGE.md` says the reader's toolbar and the composer's formatting row are one Tab stop with Left
  and Right, as the reader and composer lanes built them (the other lane had written the opposite).

### Still open

- Done in "Last gaps: UI": row 160: the reader bar labels Reply only. Either record "the reader bar labels Reply only (mockup)"
  under Owner decisions, or bring back the Archive, Delete and Move labels by priority. This is the
  owner's call.
- Done in "Last gaps: UI": row 164 (Tab from a row skips its quick actions): the test is not written; row 154 it depended on is done.
- Rows 155 and 157: the formatting row is done; row 157's focus start on Cancel is not asserted (done in "Last gaps: UI").
- Done in "Last gaps: UI": access key chips and screen reader letters for menu items and the Settings sections (needs a Kit
  mnemonic and `access_key` on the Settings buttons). Mark read, Mark unread and Save as .eml have no chip.
- Done in "Last gaps: UI": Ctrl+O on a row of the Drafts list (a composer window from the list) is open in the mail view.
- A draft window's own close button and the size kept on app-side closes: done, see "Last gaps: tests".
- Scroll offset and view flags of a message tab across tab switches (row 295): done, see "Last gaps: tests".
- Narrator was not run on the reader's text pattern, the sticky letter header, the drop line or the dialogs.
- No real windowed snapshot was run for the new FOCUS, HOVER, SCROLL and WALK switches, and no real
  sleep and wake. README screenshots are not redone. No release build, so the exe size (ceiling
  21,500,000 bytes) is not measured; the size cut is the next run.
- `src/mail/pin.rs` TLS fake tests were reported flaky under the parallel library run by the data lane
  (they passed in the full run here): cause found and fixed, see "Last gaps: tests".
- The commits of three lanes (other: `5ebacef`, `a01c67c`, `5647253`) carry `Co-Authored-By: Claude Sonnet 5.5`.

### Checks

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked`, `cargo xtask check` (the size check skipped: no release build),
`cargo deny --workspace --locked check licenses advisories` (ok), and the `check-pins.ps1` rules by
hand (`cargo tree --duplicates` per package: only the listed known pairs, `windows-sys` exempt).

## Last gaps: UI (gpui/f-ui)

The UI items of "Gaps integration", "Still open", on `gpui/f-ui` from main `8d6b256`. The test-only
items of that list (a draft window's own close, row 295, the UndoSend save failure, the TLS fakes)
are another lane's.

- **Row 160, reader bar labels.** The Slint bar's priority on the GPUI bar: Archive, Delete and
  Reply strictly in that order, then Mark as read, Mark as important, Move, Reply all and Forward
  while each still fits, each label measured on its own (`reading::labels_by_priority`; the text is
  shaped in the pill's type, and the bar's width with no label is its measured width less what
  the labels shown add). With no
  label left the last buttons go to More as before. The GPUI reading pane is narrower than the
  Slint one at the same window size and its buttons larger, so 1280 x 800 labels fewer than the
  Slint bar (Archive and Delete in the test font; 1920 x 800 has Reply, Archive and Delete and
  more). Replaces the mockup's "Reply the one label"; no owner decision needed now.
- **Row 164.** The list row's quick actions were icon buttons and so Tab stops; they are not now
  (`IconButton::no_tab_stop`), so Tab from the ringed row reaches the reader's bar (Reply), as in
  the Slint app.
- **Row 157.** A destructive confirm opens with the focus on Cancel; Enter presses the focused
  button (Cancel, or the action once Tab has moved there); Tab and Shift+Tab go round the two
  buttons only (Kit's focus trap). A plain confirm keeps Enter for its action.
- **Access keys on menu items and the Settings sections.** The vendored Kit's
  `PopupMenuItem::access_key` (gpui-component `KOMOREBI-PATCH.md`, general for Skua's fork): the
  item tells UI Automation its letter, draws a chip while Alt is held, and the letter alone runs it
  in the open menu. The More menu and the row menu give every command with a key its letter
  (Mark as read, Mark as unread, Save as .eml and the others); the keyboard's row menu (Shift+F10)
  does the same. The Settings sections have keys of their own (`Key::Section`, `Group::Settings`;
  English A, S, R, G, P, B; pt-PT C, A, R, G, I, E), drawn on the section list and run by
  `settings::ShowSection`; in Settings a section's letter wins over a message command's. The bar's
  Mark as read or unread button has its chip. gpui-base's test snapshot reads the keyboard
  shortcut (`ElementSnapshot::keyshortcuts`) so tests can assert the letters.
- **Ctrl+O on a Drafts list row** opens that draft in a composer window: the pane's draft moves
  there when the pane shows that row (`pane_draft_out`), otherwise `open_draft_window`, which
  brings an open window forward.
- Found on the way: the More menu sent its commands to the list, which one pane hides behind the
  open message, so its items did nothing there; it sends them from the reading pane in one pane.

Tests: `widgets::dialog::tests::a_destructive_confirm_starts_on_cancel_and_keeps_tab`,
`views::mail::reading::tests::reader_bar_labels_go_by_priority`,
`labels_go_by_priority_each_measured_on_its_own`, `tab_from_a_row_skips_its_quick_actions`,
`window::access_tests` (three), `window::drafts_list_tests::ctrl_o_on_a_drafts_row_opens_it_in_a_composer_window`.
No new strings; the `.pot` regenerated (an entry moved). Not done here: Narrator on the new chips
and letters (the UIA keyboard shortcut is set and tested, not heard), and no windowed snapshot of the
labelled bar.

## Last gaps: tests (gpui/f-tests)

The last open test gaps and the flaky tests, in `D:\Projects\kw-f-tests` (branch `gpui/f-tests`, from main
`8d6b256`).

- **A composer window's own close path** (`window::tab_tests::the_os_close_of_a_composer_window_parks_the_draft_and_keeps_its_size`).
  The test platform sends no OS close by itself, so the test calls the handler the platform would call
  (`gpui::VisualTestContext::simulate_close`, which runs the `on_window_should_close` closure). A window
  with a draft to keep: the handler asks the composer to close (it returns false, the platform leaves the
  window to the composer), `park_composer` saves the draft, the composer says Closed, the window goes,
  and the size it had (resized first with `simulate_window_resize`) is the next window's; the next
  window opens at that size. A draft whose save fails keeps its window, its text and the old size, with
  "Could not save the draft"; once it saves, the close goes through and the size changes.
  `closes_by_the_os_and_by_the_app_keep_the_size` asserts the same for a message window's close button
  (the handler returns true), for the app's close of a message window (a held Move) and for Esc/Close in a
  composer window.
- **Row 295, scroll and view flags across tab switches.** Nothing was missing in the tab or reader
  state: each message tab owns its reader (the scroll handle, the session's plain text and image
  choices), so a switch resets nothing. What was missing was the proof; the old test only compared the
  reader's entity id. `tab_switch_restores_scroll_and_view_flags` plants a fake render worker for the
  tab's reader (`views::reader::testing::inject`, used by `ReaderView::new` in tests only), scrolls the
  page with a mouse wheel event, turns on the plain text view and the images, goes through the Mail tab
  and the other message tab and back, and asserts the scroll place, the flags and that no layout was asked
  for again; the other tab's are its own. The 1.0 reader has no find bar, so there is no find state.
- **Undo of a send when the pane's draft cannot be saved: a bug, fixed.** Undo of a send from a draft tab
  or a composer window opened the undone message in the reading pane's composer first and then handed it
  to a tab or window. Opening it parked the pane's own draft; if that draft could not be saved it stayed
  in memory, and the hand-over then closed the pane: the text was out of sight, with no tab and no saved
  copy. `ComposerView::unsend_for_elsewhere` takes the message back to Drafts without opening it in the
  pane, and the mail view hands the saved row to the shell (`MailEvent::OpenDraftTab`, which the shell
  turns into a tab, or a window for a send from a window). The pane's draft is untouched. Test:
  `undo_of_a_send_leaves_a_pane_draft_that_could_not_be_saved_shown`. A send from the pane itself still
  comes back in the pane (`undo_send`).
- **The TLS fake tests were flaky because of Windows key containers.** The fakes' servers (`src/mail/fake_tls.rs`)
  made their identity with `native_tls::Identity::from_pkcs8`, which on Windows puts the private key in a
  named key container of the user's profile, `native-tls-<n>`, with `n` counted from 0 in each process.
  Two test processes that run at the same time (two worktrees, two `cargo test` runs, which is how the
  lanes ran) share that profile and import different keys into each other's `native-tls-0`: the server
  then presented a certificate that was not the saved one ("the certificate of localhost is not the
  saved one"; up to 10 of 58 tests failed in a process). Measured with four test processes started at
  once: 9 of 12 runs failed before the fix. After it, 92 runs (four to eight at once, the pin, tolerance, fetch,
  DAV and updater tests and then all of `mail::`) had 2 failures, both in the first batch of 12, run
  just after the fix was built (the form check saw a plain answer on a TLS port; an SMTP fake had no
  listener), and none in the 80 runs after it. I did not find the cause of those two (both
  would follow from a stray connection landing on a fake's port, as a fake takes one connection; nothing
  shows one did). Fix: `fake_tls::Acceptor` makes the identity
  itself with Schannel, with a container named `komorebi-test-<pid>-<n>` and deletes the containers when the
  process exits (`atexit`; the count of key files in the profile is the same before and after a run).
  An in-memory key (`CRYPT_VERIFYCONTEXT`, the key handle on the certificate) was tried first and Schannel refused it
  ("the credentials supplied to the package were not recognized"). The server tests of fetch, DAV, the updater and
  the pin tests all use it; `chain_server` in `pin.rs` had its own container named by port.
  No retry and no sleep was added.
- **Throwaway cache names are unique by construction.** `test_support::TempCache::new` (so `seeded` and
  every fixture) puts the process id and a counter that only goes up in the folder name
  (`test_support::unique_dir`); the name only labels the folder. The same for the library's
  helpers with a name (`testdir::unique`: the cache encryption tests, `files`, the compose tests) and the
  Settings wipe test's folder. The audit found one duplicate left in the tree: `"tab-flush"` (in
  `window::tab_tests` and `views::composer::tests`, two lanes merged); the "tab-commands" one was fixed
  on main. The fixed names (`komorebi-crash-<pid>`, `komorebi-diag-<pid>`, the pin end-to-end caches and
  the like) each have one test in the process. Test: `test_support::tests::a_name_shared_by_two_tests_is_two_caches`.

## Optimization pass

Branch `opt/integ` merges the lanes `opt/fork-win`, `opt/fork-core`, `opt/speed` (with `opt/fork-kit`), `opt/app`, `opt/render` (with `opt/lib`) and `opt/overlap`; `opt/fast` was reverted to empty and skipped. Conflicts were documentation only (third_party README and KOMOREBI-PATCH files, PERF.md section numbers): resolved as unions, the cached-views section becoming PERF.md section 12. Beyond the merges: the reader hands tile pixels to GPUI without a copy, `closed_port()` in the mail tests now hands out ports from 20000..32000 so a parallel fake server cannot be given the same port, and `render_reuse` was fixed (see PERF.md section 13).

Per-lane numbers, and what was not measured, are in docs/PERF.md section 13. The combined re-measurement against the baseline and the mimalloc leave-one-out are deferred to a later update by the owner's decision; nothing was reverted on timing.

Skua patch files (`C:\dev-overflow\claude-tmp\komorebi-opt\skua-patches\`) that apply to the final tree: core-01-scene-sort, core-02-wrapped-line-decorations, core-03-font-features-default, core-04-tabstop-lazy-index, core-05-uniform-list-single-measure, core-06-text-runs-inline, fork-kit-01-native-menu-svg, fork-kit-02-text-formats, fork-kit-03-image-codecs-gpui-pre, fork-kit-03-image-codecs-gpui-pre-windows, fork-win-01-device-once, fork-win-03-lazy-msaa, fork-win-06-shared-pixels. Not applicable: fast-01-text-style-stack (its commit was reverted).

## Phase log

| Date | Phase | Branch | Notes |
|---|---|---|---|
| 2026-10-25 | 0 | gpui/main | Foundations, as above. |
| 2026-10-25 | 14 (harness), 17 | gpui/harness | Test harness, snapshots on WARP, lints, `cargo xtask check`, CI for both apps, the test inventory. |
| 2026-10-25 | 15 | gpui/i18n | The i18n bridge, as above. |
| 2026-10-25 | Brand assets (11, 16) | gpui/assets | New icon, tray icons, installer images, GPUI brand assets, repository images, as above. |
| 2026-10-25 | 1 | gpui/shell | Token layer, Kit theme, window shell with caption Invoke, live feeds, widgets, gallery; gaps listed in phase 1. |
| 2026-10-25 | Stage A | gpui/main | The four branches merged; checks, release size 11,074,048 bytes, first frame 118.5 ms median; stage B plan. |
| 2026-10-25 | 2 | gpui/list | Library moves (format, rows), the typed reader and composer slots, the mail view (folder pane and rail, list, menus, keys, flyouts), 28 tests ported and 19 moved; gaps listed in phase 2. |
| 2026-10-25 | 3 | gpui/reader | Reader engine, session, find merge and invite logic in the library; the reader view, tile element, links, banners, invite card, attachments, a11y text nodes; 19 view tests. |
| 2026-10-25 | 4 | gpui/composer | Composer model into the library (`komorebi::compose`, `komorebi::files`); the GPUI composer with the painted rich editor, chips, format bar, signatures, attachments, Preview; 93 test rows with a status; gaps listed in phase 4. |
| 2026-10-25 | Stage B | gpui/main | Lanes 2, 3 and 4 merged; fmt, clippy, tests, xtask, pins and deny pass; release build and benchmark skipped by owner decision; stage C plan. |
| 2026-11-06 | 5, 11, 12, 2 (sync) | gpui/c-shell | Mount points, the hub and status line, badges, Refresh, Outbox pane, tabs, message windows, tray and toasts, updater and What's new bars; gaps listed above. |
| 2026-11-06 | 6, 7 | gpui/c-settings | Settings (accounts and the account page, signatures, general, privacy with the cache encryption, about, shortcuts) and onboarding (welcome, email, sign in, make it yours, finish); calendar settings dropped; 60 test rows with a status; gaps listed in phases 6 and 7. |
| 2026-11-06 | 8, 9 | gpui/c-data | Rules and contacts pages and CardDAV sync into the library; the Rules and Contacts views with the sync pane, 5 view tests (row 291 ported, row 227 moved), a snapshot example; not mounted yet; gaps listed in phases 8 and 9. |
| 2026-11-06 | Stage C | main | The shell, settings and data lanes merged; Settings, Contacts, Rules and the onboarding flow mounted; the integrator requests done; phase 10 dropped by owner decision; checks as in the section above. |
| 2026-11-06 | Stage D (library) | gpui/d-library | cargo deny green (pulldown-cmark, async-executor, a documented pin-utils ignore); both flaky tests fixed at their cause (tests read this machine's Windows settings; one test moved every test's language); `invite::record_answer` for the GPUI path; shared attachment open/save and the crash log in the library; Fluent icons checked against 1.1.292; test port rows 15, 16, 37 to 42, 240, 241, 245 to 248. |
| 2026-11-06 | D (shell) | gpui/d-shell | Start-up dialogs and crash log, mailto and Start with Windows, the installer mutex, toasts by id and the taskbar flash, F1 and F6, tabs following moves and Undo, the system caption on Windows 10, dialog roles, live notices, scaled controls; gaps listed in stage D: shell. |
| 2026-11-06 | Stage D mail | gpui/d-mail | Mail lane fixes: keys from the reading pane, F6, drag to folder and file drop, source commands, Drafts menu, outbox resume, selection panel, one-pane Back, empty state; 14 window tests and 5 plain tests; gaps listed in Stage D: mail. |
| 2026-11-06 | D (reader) | gpui/d-reader | Attachments, Save .ics, the RSVP question (reply through the outbox, no calendar), Why?, Save as .eml, Show original, Copy Message-ID, the More menu and command bar of the Slint app, page keys, selectable text view, active descendant; review fixes (source wait per message, text view keys, Resume bar gated on its owner, Downloads Known Folder, Move menu button); gaps listed in stage D: reader. |
| 2026-11-06 | Stage D: composer (4, 13, 19) | gpui/d-composer | Sends shared by every composer (a draft tab's send goes out, Undo from any composer), body and Preview text nodes for screen readers, suggestions as a named list box, focus ring on the underline rows, files pasted in any field, Drop to attach, the composer's side of the server-draft Edit, the draft-reply bar and the pane draft to a tab; rows 95 and 296 done; wiring left to the integrator. |
| 2026-11-06 | Stage D (settings) | gpui/d-settings | Compose defaults card, the account page's Find settings, relay, Move up and down, Enter saves and folded servers; the signature toolbar and Link; the onboarding fixes; radio groups, F1 and the step list; rows 116, 120, 121 and 179 done; gaps listed in stage D: settings. |
| 2026-11-06 | Stage D: data (8, 9, 13 in part) | gpui/d-data | Letter headers and text-size rows in Contacts, the Sync button, the large initials, drag reorder of rules, names for repeated controls; the mounting items confirmed done; gaps listed in "Stage D: data". |
| 2026-11-06 | Stage D (other) | gpui/d-other | Kit in pt-PT, access keys (wired into the main window, message view and composer), the pane snapshot check (row 2) and a fix to the shell check, CI boot check fallback and seeded check, the 1.0 setup preview; measured: 22,051,328 bytes (OVER the ceiling by 551,328, main alone 22,036,480), first frame 77 ms, all 15 snapshot actions pass. |
| 2026-11-06 | Stage D integration | main | The eight stage D lanes merged (library, shell, mail, reader, composer, settings, data, other); the duplicate crash log, source commands, RSVP and F6/F1 handling reduced to one each; the integrator requests wired; pot regenerated; fmt, clippy, tests, xtask check and cargo deny pass; the exe size is not measured. |
| 2026-11-06 | Cut-over preparation | main | `release.yml` builds and packages the GPUI app with `-Brand10`; the 21.5 MB ceiling and honest memory wording in the docs; the calendar out of the user docs; CHANGELOG 1.0.0 (Unreleased); fmt, clippy, tests and xtask check pass. Not done: the version bump, deleting the Slint app, the exe over the ceiling (22,051,328 bytes), the 0.7 window key. |
| 2026-11-06 | 1.0 release candidate checks | main | fmt, clippy, `cargo xtask check` and the full tests pass (1,490 passed); boot check and all 16 snapshot actions pass; the release exe is 22,387,712 bytes, OVER the 21,500,000 ceiling by 887,712; launch 134 to 240 ms on a busy machine (budget 200 ms met in the best run); the by-hand gates and what remains are listed. |
| 2026-11-06 | Cut-over | main | The Slint app deleted (`src/main.rs`, `src/app*`, `ui/`, `crates/komorebi-ui`, `src/tr.rs`, the Slint dependencies and CI steps); the calendar library code kept; `assets/brand-1.0` moved up, `Brand10` gone; the exe is `komorebi.exe`; version 1.0.0; 0.7's `window` key ignored; a 0.7 cache opens as it is (schema v18, tested); docs updated. The size cut and the real-install checks are not done. |
| 2026-11-06 | Gaps: library | gpui/e-library | `invite::card` only reads (the cancel write moved to `invite::mark_cancelled`, called by no app in 1.0, for the calendar in 1.1.0); one new test; the calendar code and tests kept. |
| 2026-11-06 | Gaps: other | gpui/e-other | Snapshot switches (focus, hover, scroll, walk, title bar, compose), 20 window tests and the wake detector for the open test port rows, three more access keys, the user docs for the 1.0 screens, stale statements marked; open rows are listed in "Gaps: other"; a fix round applied the four patches the lane had left ignored (draft to a tab, reopen, flush on tab switch, Esc and the status line) and started the wake watch in the hub. |
| 2026-11-06 | Gaps: shell | gpui/e-shell | The message tab's full command set and Junk, Save and Move glyphs, draft windows and draft saves on switch, close and quit, cascading windows, teaching tips with What's new waiting, named dialogs and AlertDialog confirms (Kit and gpui-base patches), the select's expanded state, the snapshot `compose` action; rows 101, 102, 175 to 177, 295, 303 to 305, 308, 310 to 314 done; gaps listed in "Gaps: shell". |
| 2026-11-06 | Gaps: mail | gpui/e-mail | Message tabs and windows get Move, Report junk / Not junk, Mark as read, Mark as important, Forward as attachment and More (`Quick` and `MessageEvent` variants, `Mail::act_on_message`); the hub's wall-clock wake check (row 7); the dead collapse and refresh `Command` arms removed; 7 tests; row 160 stays open. |
| 2026-11-06 | Gaps: reader | gpui/e-reader | The command bar as one Tab stop (Left, Right, Home, End) and Reply's label by measured width (rows 154, 160); Enter and Escape pass the page with nothing to do; TextRun nodes and the UIA text selection of the page; Save and Move glyphs; `Db::message_id`, Copy Message-ID without a body; Junk in tabs, the save dialog title, Enter into the page and the row menu's Copy Message-ID left to the integrator. |
| 2026-11-06 | Gaps: composer | gpui/e-composer | Composer windows (Ctrl+O and the tab command; Undo brings a window's send back in a window; a draft open in a tab or window comes forward instead of opening twice; closing saves); tab switch saves the leaving draft; a draft whose save failed stays shown; the formatting row is one Tab stop; chips wrap (tested); rows 101 to 103, 155, 304 done, 104 in part; `aria_expanded` and the strip's menu item left to the shell lane. |
| 2026-11-06 | Gaps: settings | gpui/e-settings | The dead `SettingsEvent::CheckForUpdates` removed; ManageSignatures confirmed (opens Signatures, no toggle) with the test extended; the dialog host name and the address fields' expanded state need gpui-base and Kit changes (Skua fork work, not done). |
| 2026-11-06 | Gaps: data | gpui/e-data | Sticky letter header in Contacts, the rules drop indicator line with the Fluent re-order grip named for Narrator, the title-size initials in the shared large SenderSpot; 4 new tests; one new msgid (pot not regenerated). |
| 2026-11-06 | Gaps integration | main | The eight gap lanes merged; duplicate message-tab, composer-window and wake code reduced to one each; Open in new window in the composer's More, the address fields' expanded state, Enter in a destructive confirm; the `.pot` regenerated; open items listed in "Gaps integration". |
| 2026-11-06 | Re-run on main after the gaps integration | main | `cargo xtask check` and the full tests pass (1,359 passed) after a test-name collision of two merged lanes was fixed; `komorebi.exe` is 18,759,168 bytes, under the 21,500,000 ceiling; boot check and all 16 snapshot actions pass on an empty, a seeded and a 0.7-style cache. |
| 2026-11-06 | Last gaps: UI | gpui/f-ui | Row 160 labels by priority, row 164 quick actions no Tab stops, row 157 destructive confirms start on Cancel, access keys on menu items (Kit patch) and the Settings sections, Ctrl+O on a Drafts row; see "Last gaps: UI". |
| 2026-11-06 | Last gaps: tests | gpui/f-tests | The OS close path of composer windows and the size kept by every close, scroll and view flags per message tab (row 295), a bug in Undo of a send with an unsaved pane draft, the flaky TLS fakes (shared Windows key containers), unique throwaway cache names; see "Last gaps: tests". |
| 2026-11-14 | Optimization pass | opt/integ | Six optimization lanes and the overlap fixes merged; reader tile pixels shared; flaky `closed_port()` fixed; `render_reuse` fixed; gate and snapshot checks pass on an empty, a seeded and a 0.7-style cache; combined timing and leave-one-out deferred. |
