# Komorebi patch: gpui-pre-windows 0.3.9

On top of the Skua copy (`SKUA-PATCH.md`: published `gpui-pre-windows 0.3.9` plus Skua's
demand-driven frames, G1-F10, and no frames while the display is off, G3-F26), exported from
Skua at 7e0fdcf. Every Komorebi line is marked `KOMOREBI PATCH`.

## fxc.exe discovery with a clear error (`build.rs`)

**Problem.** Release builds compile the HLSL shaders with `fxc.exe` (debug builds compile them
at run time). Upstream looks in `GPUI_FXC_PATH`, then `where.exe fxc.exe`, then the newest
Windows SDK, and fails with a bare `panic!("Failed to find fxc.exe")`. A `GPUI_FXC_PATH` naming
a missing file was skipped silently; `where.exe` printing several lines produced a path with a
line break in it; the SDK path was returned without checking that fxc.exe is there.

**Change.** `find_fxc_compiler`: `cargo:rerun-if-env-changed=GPUI_FXC_PATH`; a set but missing
`GPUI_FXC_PATH` is a `cargo::error` naming the value and an example path; the first `where.exe`
line that is a file is taken; the SDK candidate must be a file; a miss is a `cargo::error` that
says what was tried and how to fix it (install the Windows SDK or set `GPUI_FXC_PATH`;
docs/GPUI-BUILD.md).

## Force WARP (`src/directx_devices.rs`)

**Problem.** GPUI takes the first DXGI adapter that gives a suitable D3D11 device. There is no
way to choose WARP (the software rasterizer in every Windows 10 and 11) to test a machine
without a GPU, to work around a broken driver, or to run on CI runners that have no GPU.

**Change.** `get_adapter`: when `KOMOREBI_GPUI_FORCE_WARP` is set and not empty, the adapter is
`IDXGIFactory4::EnumWarpAdapter` and the usual device checks (feature level 10.1 or better,
BGRA, structured buffers) apply. Komorebi's DirectX pre-flight
(`crates/komorebi-gpui/src/preflight.rs`) reads the same variable, so it probes what GPUI will
use.

**Upstream.** Not reported. The REPORT.md optimization backlog item 7 (explicit WARP fallback
and a software-adapter switch) builds on this.

## One DirectX device at start-up: `prepare_devices` (`src/directx_devices.rs`, `src/gpui_windows.rs`)

**Problem.** An app that wants to say "no usable GPU" in its own dialog (Komorebi's DirectX
pre-flight) has to create a Direct3D 11 device before GPUI does, and then GPUI creates a second
one in `WindowsPlatform::connect_initially`. A device costs tens of milliseconds in the driver
(`D3D11CreateDevice` is 47 percent of the start-up samples in docs/PERF.md's profile, two callers),
and a second device brings its own driver threads and allocations.

**Change.** `gpui_windows::prepare_devices(force_warp: bool) -> Result<(), String>` creates the
`DirectXDevices` (factory, adapter, device, immediate context) on the calling thread, which can be
a worker beside the rest of start-up, and keeps them in a process-wide slot. The next
`DirectXDevices::new()` (the platform's first `attach_gpu`) takes them; a device-lost recovery
finds the slot empty and creates fresh devices as before. `get_adapter` takes `force_warp` as a
parameter (`new()` still reads `KOMOREBI_GPUI_FORCE_WARP`), and when no adapter gives a device it
now returns an error naming each adapter's reason instead of the `EnumAdapters` NOT_FOUND error
(it was `unreachable!()` after the loop). The D3D11 device is free-threaded; the immediate context
is only ever used by one thread at a time (the creating thread stops using it when it returns).

**Komorebi.** `crates/komorebi-gpui/src/preflight.rs` calls it from its `dx-probe` thread and
joins before `app.run`; on `Err` the existing dialog and exit code 3 apply, and the WARP switch and
`KOMOREBI_GPUI_PREFLIGHT_FAIL` work as before.

**Gain.** See `docs/PERF.md` / the lane report: the second device creation leaves the critical path.

## Path intermediate textures are made when a frame first draws a path (`src/directx_renderer.rs`)

**Problem.** `DirectXResources` made the path rasterization targets with the swap chain and again on
every resize: a 4x multisampled B8G8R8A8 texture (`PATH_MULTISAMPLE_COUNT`) and its single-sample
resolve texture, both at the window size. The AMD driver backs them with write-combined system
mappings that count as private bytes: three mappings of 30, 29 and 22 MB (81 MB) at 1280 x 800 in the
first frame of Komorebi's main view, which draws no path at all.

**Change.** `DirectXResources::path_targets: Option<PathTargets>` (texture, shader view, MSAA texture,
MSAA view). It starts `None`; `draw_paths_to_intermediate` calls `ensure_path_targets`, which makes
the targets at the current viewport size; a resize (`recreate_resources`) drops them so the next
path frame makes them at the new size, and a device loss builds new resources with none. Drawing and
the resolve are unchanged, so pixels are the same (onboarding's brand mark, the one path Komorebi
draws, is byte-identical in a window capture before and after).

**Gain.** Private bytes 2 s after the first frame: 111 MB to 50 MB in Komorebi's main view
(`docs/PERF.md` section on the fork patches); the first frame that draws a path pays the creation
(a few ms) instead of every launch.

## Image codecs trimmed (`Cargo.toml`)

The `image` dependency's features are cut to `bmp`, `gif`, `ico`, `jpeg`, `png` and `webp`, as in
`gpui-pre`: see "Image codecs trimmed to what Komorebi decodes" in
`third_party/gpui-pre/KOMOREBI-PATCH.md` (724,480 bytes of the exe together). `clipboard.rs` names
`image::ImageFormat::Tiff` when writing a TIFF clipboard image; that write now reports the
unsupported-format error. Komorebi never writes one.
