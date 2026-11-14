# Building the app (Komorebi 1.0)

Komorebi's user interface is GPUI with GPUI Kit (the plan, its progress and the cut-over from the
Slint app of 0.7: [GPUI-MIGRATION.md](GPUI-MIGRATION.md)). The repository builds one app over one
library:

| Binary | Package | UI |
|---|---|---|
| `komorebi.exe` | `komorebi-gpui` (`crates/komorebi-gpui`) | GPUI + GPUI Kit |

It links the library (`src/lib.rs`, the root package `komorebi`: cache, mail, settings, theme,
Win32 shell code), which has no UI toolkit in it. The binary is named `komorebi` so that the
installed file is `komorebi.exe`, the name 0.7 installed: the installer, the Start menu shortcut,
the Run value and the `mailto:` registration all point at it. The root package is the workspace's
default, so plain `cargo build` and `cargo test` at the root mean the library; pass
`-p komorebi-gpui` or `--workspace` for the app.

## Requirements

What [BUILDING.md](BUILDING.md) lists (Rust from `rust-toolchain.toml`, the Visual Studio Build
Tools with the Windows SDK), plus:

- **fxc.exe**, the HLSL shader compiler of the Windows SDK, for release builds (debug builds
  compile GPUI's shaders at run time and need no fxc). The build script of the vendored
  `gpui-pre-windows` looks, in order, at `GPUI_FXC_PATH`, `fxc.exe` on `PATH`, and the newest
  Windows 10/11 SDK (`HKLM\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0`,
  `InstallationFolder\bin\<version>\x64\fxc.exe`). When none is found the build stops with an
  error that says what was tried. To choose one:

  ```powershell
  $env:GPUI_FXC_PATH = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\fxc.exe'
  ```

  A `GPUI_FXC_PATH` that names no file is an error (it is not skipped). GitHub's and
  Blacksmith's Windows images ship the SDK, so CI needs no setting.
- **A Direct3D 11 device** to run it: any GPU with feature level 10.1 or better, or WARP (the
  software rasterizer every Windows has; see `KOMOREBI_GPUI_FORCE_WARP`). Tests need none: GPUI's
  test platform is headless.

## Build, run, test

```powershell
cargo build -p komorebi-gpui                     # target\debug\komorebi.exe
cargo build -p komorebi-gpui --release --locked  # target\release\komorebi.exe (fat LTO, long)
cargo test -p komorebi-gpui -p komorebi-i18n --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Run it only against a throwaway cache in development, never your real one:

```powershell
$env:KOMOREBI_OFFLINE = 1; $env:KOMOREBI_DB = "$env:TEMP\gpui-test.sqlite3"
.\target\debug\komorebi.exe
```

The release profile is Komorebi's (`opt-level = 3`, fat LTO, one codegen unit, stripped,
`panic = "abort"`); the GPUI build needs nothing added to it.

## Development switches

All are read only when set and not empty.

| Variable | Effect |
|---|---|
| `KOMOREBI_DB`, `KOMOREBI_OFFLINE`, `KOMOREBI_LANG` | As in docs/ENVIRONMENT.md. |
| `KOMOREBI_TIMING` | Prints `[timing]` lines to stderr: platform created, DirectX probed, cache loaded, window created, and `first frame drawn: <ms>` (process start to the first event-loop pass after the first frame was drawn and presented). |
| `KOMOREBI_EXIT_AFTER_FIRST_FRAME` | Quits right after the first frame (launch measurements; the window's place is saved as on any quit). |
| `KOMOREBI_SNAPSHOT`, `KOMOREBI_THEME`, `KOMOREBI_SIZE`, `KOMOREBI_SNAPSHOT_ACTION`, `KOMOREBI_SNAPSHOT_CHECK` | Capture the first frame to a PNG or BMP and quit; refused (exit 2) without `KOMOREBI_OFFLINE` and a throwaway `KOMOREBI_DB`. See [GPUI-TESTING.md](GPUI-TESTING.md) "Pixel snapshots". |
| `KOMOREBI_GPUI_FORCE_WARP` | Draws with WARP instead of the first hardware adapter (a broken driver, a machine or VM without a GPU, CI). Read by the vendored gpui-pre-windows and by the DirectX pre-flight. |
| `KOMOREBI_GPUI_PREFLIGHT_FAIL` | Acts as if no DirectX 11 device could be made: shows the error message box and exits with code 3. For checking the dialog. |
| `GPUI_DISABLE_DIRECT_COMPOSITION` | GPUI's own switch: the swap chain goes on the window directly instead of through DirectComposition (no memory difference measured, REPORT.md S2). |
| `GPUI_FXC_PATH` | Build time: fxc.exe to compile the shaders with (above). |
| `KOMOREBI_GALLERY`, `KOMOREBI_GALLERY_SHOT`, `KOMOREBI_GALLERY_OPEN` | The component gallery, builds with the `gallery` feature only (below). |
| `KOMOREBI_TEXT_SCALE`, `KOMOREBI_ANIMATE=0` | Windows' text size (1.0 to 2.25) and animation effects off, for checking the token feeds. |

## The component gallery

Development builds can show every widget of the brand in one window instead of the app (no
cache, no accounts), to compare with the mockups in komorebi-brand `final/app/png`:

```powershell
cargo build -p komorebi-gpui --features gallery
$env:KOMOREBI_OFFLINE = 1; $env:KOMOREBI_DB = "$env:TEMP\gpui-gallery.sqlite3"
$env:KOMOREBI_GALLERY = 'light'          # or 'dark'
$env:KOMOREBI_GALLERY_SHOT = "$env:TEMP\gallery-light.png"   # optional: render to a PNG and quit
$env:KOMOREBI_GALLERY_OPEN = 'dialog'    # optional: with the confirm dialog open
.\target\debug\komorebi.exe
```

The shot is GPUI's `Window::render_to_image` (the Direct3D scene drawn to a texture), so it works
with the window off screen, the display off or over a remote session; motion is off in shots.
The `gallery` feature turns on GPUI's `test-support` (for that API) and links `image` for the
PNG; release builds never enable it. `scripts/gpui-shot.ps1` captures a running window of any
build with `PrintWindow` instead (as Skua's skua-shot), on its own offline throwaway cache; it
gives a black frame when GPUI draws nothing (display off, a remote session that is not shown).

## DirectX pre-flight

GPUI panics when it cannot create its Direct3D 11 device, and with `panic = "abort"` the app
would vanish silently. `crates/komorebi-gpui/src/preflight.rs` therefore creates the same device
first, on a thread beside GPUI's platform start (DXGI factory, the first adapter that gives a
D3D11 device at feature level 11.1, 11.0 or 10.1 with BGRA support and structured buffers, or
WARP under `KOMOREBI_GPUI_FORCE_WARP`). When that fails it shows a native message box, in
English or European Portuguese after the Windows display language (a catalog message, `tr!`),
with the technical reason under the text, and exits with code **3**.

## Text and languages

The app's text comes from the gettext catalog, through `tr!` and `trn!`
(`crates/komorebi-i18n`): see [GPUI-I18N.md](GPUI-I18N.md) for the macros, the language
switch and the workflow after changing a string.

## Vendored crates and patches

The GPUI stack is vendored as Skua does: `third_party/gpui-pre`, `third_party/gpui-pre-windows`
(0.3.9) and `third_party/gpui-component` (0.8.0), each the exact published source plus the
patches in its `SKUA-PATCH.md` and `KOMOREBI-PATCH.md`, wired with `[patch.crates-io]` in the
root `Cargo.toml` and pinned with `=` in `[workspace.dependencies]` (`gpui` is `gpui-pre`,
`gpui-kit` is the facade over the patched `gpui-component`). `third_party` is excluded from the
workspace, so fmt, clippy and tests skip it. See [third_party/README.md](../third_party/README.md).

| Patch | Crate | Why |
|---|---|---|
| Demand-driven frames (Skua G1-F10) | gpui-pre-windows | Frames only when something changed: an idle window draws nothing (0.01 percent of a core instead of about 3.5). |
| No frames while the display is off (Skua G3-F26) | gpui-pre-windows | The driver's present path costs about 2.5 ms of CPU per frame with the display off. |
| View splice, cached window control areas (Skua G3-F27) | gpui-pre | Cheaper redraws of cached views; title bar hit regions survive cached frames. |
| Kit style hooks (Skua G3-F20) | gpui-component | Caller shadows, per-menu styles, caret switch. |
| `windows-manifest-provided-by-host` feature (Komorebi) | gpui-pre | GPUI's RT_MANIFEST id 1 collided with Komorebi's (CVT1100); komorebi-gpui turns the feature on and links `assets/komorebi.rc` instead. |
| fxc.exe discovery with a clear error (Komorebi) | gpui-pre-windows | A missing or wrong fxc stopped the build with a bare panic. |
| `KOMOREBI_GPUI_FORCE_WARP` (Komorebi) | gpui-pre-windows | Choose WARP without a GPU, or around a broken driver. |

## The manifest

`assets/komorebi.manifest` is RT_MANIFEST id 1 of komorebi.exe: `assets/komorebi.rc`, which also
carries the icons ([BRANDING.md](BRANDING.md)), embeds it (`crates/komorebi-gpui/build.rs`). It
carries everything GPUI's own manifest did: asInvoker, Windows 10 and 11, Common Controls 6, and
per-monitor DPI awareness (`dpiAware true/pm`, `dpiAwareness PerMonitorV2`; GPUI sets none at
run time), plus Komorebi's UTF-8 code page. `heapType SegmentHeap` (GPUI's, measured in the
optimization release). To check a built exe without running it:

```powershell
pwsh scripts/check-manifest.ps1 target\release\komorebi.exe -Show
```

It fails unless there is exactly one RT_MANIFEST (id 1) with all of the above.

## Checks

The gate before a commit is one command ([GPUI-TESTING.md](GPUI-TESTING.md) "The gate"):

```powershell
cargo xtask check            # fmt, clippy, app + library + xtask tests, lints, exe size
cargo xtask check --release  # build the release exe first, so the size is measured
```

As in [BUILDING.md](BUILDING.md), over the whole workspace:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked                       # the library and its integration tests, catalog (tests/i18n_catalog.rs)
cargo test -p komorebi-gpui -p komorebi-i18n --locked   # the app (with tests/token_lint.rs) and its catalog crate
cargo clippy -p komorebi-gpui --all-targets --features gallery --locked -- -D warnings
cargo xtask lint                          # the app's token and no-timers lints
pwsh scripts/check-pins.ps1               # exact pins linked once
cargo deny --workspace --locked check licenses advisories
pwsh scripts/find-fxc.ps1                 # which fxc.exe a release build uses
```

Size: `cargo xtask size` checks `target\release\komorebi.exe` against 21,500,000 bytes (the 1.0
ceiling).
Boot on WARP: `pwsh scripts/gpui-boot-check.ps1 -Exe target\release\komorebi.exe -Cache <throwaway> -Out <png> -Warp`.
Launch measurements: `scripts/gpui-bench.ps1 -Mode timing -Runs 9 -Config gpui -Cache <copy> -Scratch <dir> -GpuiExe <exe>` (offline, a throwaway cache inside `-Scratch` only; GPUI-MIGRATION.md "Measurements"; another exe's path as a `-Config` compares an older build).
