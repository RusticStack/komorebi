# Skua patch: gpui-pre-windows 0.3.9 (D-108, G1-F10, G3-F26)

Base: the published crate `gpui-pre-windows 0.3.9` from crates.io, byte-for-byte (gpui-pre
snapshot of zed `gpui_windows` at zed@93d745668805029476e6d7fbb19652116fed51de), license
Apache-2.0 (`LICENSE-APACHE`). Only the registry marker `.cargo-ok` was dropped. Wired from the
workspace root with:

```toml
[patch.crates-io]
gpui-pre-windows = { path = "third_party/gpui-pre-windows" }
```

`third_party` is in the workspace `exclude` list, so Skua's lints, fmt, clippy and tests do not
run on this crate. Every changed line is marked `SKUA PATCH (G1-F10)` or `SKUA PATCH (G3-F26)`.

## Problem

`WindowsPlatform::begin_vsync_thread` waits on `DwmFlush` and then calls
`RedrawWindow(RDW_INVALIDATE)` on every window at every vblank, whether or not anything changed.
The main thread then runs the full `request_frame` path (Direct Manipulation update, IME state,
gpui's frame callback) at the display rate: 180×/s on a 180 Hz monitor. A bare, idle GPUI Kit
window used 2.3–3.8 % of one core with ~350–550 main-thread and ~270–350 `VSyncProvider`
context switches per second. Minimized, `DwmFlush` returns early and the thread falls back to
`sleep(interval)`, still invalidating every window: no saving at all.

gpui core already supports demand-driven platforms: `PlatformWindow::frame_waker` is called
when a window becomes dirty (`WindowInvalidator::set_dirty` / `invalidate_view`), when
next-frame callbacks are queued (`Window::on_next_frame`, `request_animation_frame`,
animations), and after a frame that left demand behind (`wake_platform`). The web backend
implements it; the Windows backend did not.

## Change

- `vsync.rs`: `FrameDemand` (mutex + condvar shared by the platform, every window and the
  `VSyncProvider` thread) and `WindowFrameRequest` (one window's `AtomicBool` request);
  `VSyncProvider::interval()`.
- `window.rs`: `WindowsWindowState::frame_request`; `PlatformWindow::frame_waker` sets it and
  wakes the vsync thread. `WindowCreationInfo`/`WindowCreateContext` carry the shared demand.
- `platform.rs`: the `VSyncProvider` loop parks on `FrameDemand` (1 s timeout, only so a stop
  request or a lost device is still noticed while idle) and invalidates **only windows whose
  request it takes**. The first frame after an idle period is invalidated immediately (no wait
  for a vblank, so keystroke → frame is not delayed); a frame requested within one refresh
  interval of the previous one waits for `DwmFlush` as before, so animations stay vsync-paced.
  `detach_gpu` and `Drop` notify the thread so it exits promptly.
- `events.rs`: requests a frame where upstream relied on the unconditional re-invalidation:
  a re-entrant `draw_window` that had to defer; restore from minimized (requests made while
  minimized are dropped, since gpui's frame callback is parked then); a touchpad pointer hit
  test (Direct Manipulation starts), and after each frame while a gesture or its inertia runs.
- `direct_manipulation.rs`: `needs_frames()` (`IDirectManipulationViewport::GetStatus` is
  `RUNNING` or `INERTIA`).

## Result (owner machine, 180 Hz, release)

Probe: `QueryProcessCycleTime` over 10 s (3418 MHz nominal) and per-thread context switches.

| Window | Before | After |
| --- | --- | --- |
| bare `cookbook_view`, visible | 3.76 %; main 556, VSyncProvider 348 wakeups/s | 0.01 %; VSyncProvider 1.5/s |
| bare `cookbook_view`, minimized | 3.36 %; main 472, VSyncProvider 283 /s | 0.00 %; VSyncProvider 1.3/s |
| `skua.exe` live pwsh pane, visible (cursor blinking) | 2.78 %; main 357, VSyncProvider 270 /s | 0.25 %; main 16, VSyncProvider 4 /s |
| `skua.exe` live pane, minimized | 2.27 %; main 362, VSyncProvider 260 /s | 0.04 % |
| `cargo xtask bench --scenario one-pane` idle visible / minimized | 2.50 / 2.81 % (G1 run) | 0.00 / 0.00 % |

Goldens unchanged (ratio 0.000000 ×2). Full table: `docs/gates/G1.md` (G1-F10).

## G3-F26: no frames while the display is off

**Problem.** G3-F24 saw the renderer's draw + `Present` cost ≈ 8.6 Mcyc (≈ 2.5 ms CPU) instead of
≈ 0.6 Mcyc on many frames of some runs. Reproduced with a temporary per-phase trace
(`QueryThreadCycleTime` around `pre_draw`/uploads/batches/`Present`, the CPU number, and the
owner's input idle time from `GetLastInputInfo` every 5 s) over a 43-min idle `skua --panes 1`:
the extra cost is entirely in `Present` (render 0.14 Mcyc either way; Present 0.86 → 8.6 Mcyc,
3.1–3.2 ms wall per frame, thread busy the whole time). It appears exactly when the owner has
been idle for the power plan's display timeout (Balanced, `VIDEOIDLE` = 300 s): 0.5 % of frames
slow while the owner was active (4439 frames), 1.5 % at 30–200 s idle, 75 % from 300 s on
(17:00:20–17:02:10: 9/24, 19/25, 23/27, 23/27, 20/26 … per 10 s), back to 0 within seconds of
input. In G3-F24's slow runs the owner was away too (the window could take the foreground, which
Windows allows after the foreground lock timeout, and its cursor blinked). Every slow frame ran on
an E-core (all frames do once the machine idles), so it is not the core type. A temporary sampler
(suspend the UI thread when a `Present` passes 0.6 ms, read RIP and the stack) put the long
`Present`s in `amdxx64.dll` waiting (`WaitForSingleObjectEx`) and in
`NtDxgkSubmitPresentToHwQueue`: the driver/kernel present path, not gpui code. A CPU load on all
24 threads (no display-off) did not reproduce it; minimize/restore, resize and typing did not.

**Cause.** With the display off nothing is shown, but GPUI kept drawing every blink and status
frame and each `Present` became ~2.5 ms of CPU in the AMD driver (RX 9070 XT, driver
32.0.32015.2008). gpui's `WindowVisibility` only covered minimized/hidden windows.

**Change.**
- `display_power.rs` (new): `GUID_SESSION_DISPLAY_STATUS` subscription
  (`RegisterPowerSettingNotification` on the platform window; Windows sends the current state at
  once); `is_off()` ("dimmed" counts as on); `update()` reads `PBT_POWERSETTINGCHANGE`.
- `platform.rs`: registers at startup (not headless), unregisters on drop; on a change posts
  `WM_GPUI_DISPLAY_POWER_CHANGED` to every window.
- `window.rs`: `visibility()` is `Hidden` while the display is off (Skua's status sampler stops,
  cookbook `cookbook_window_state`).
- `events.rs`: `draw_window` skips the frame while the display is off (validates the region, keeps
  a forced render pending; the window stays dirty, so no further invalidations follow);
  `WM_GPUI_DISPLAY_POWER_CHANGED` re-reports visibility and, when the display is back, requests
  the frame.

**Result.** A traced build that injects a real `PBT_POWERSETTINGCHANGE` (off, then on 30 s later)
into its own platform window: one skipped frame, then no frame and no `Present` for the 30 s; the
next frame is drawn 29 ms after "on" and blinking resumes (3 runs, no frame drawn while off in
any). Process CPU while off 0.012 / 0.081 / 0.254 % (main thread 0 cycles in the first; woken by
window messages, not frames, in the others) vs 0.051 / 0.171 / 0.937 % on, same runs. Before (43-min run above),
display off cost ≈ 2.5 frames/s × ≈ 11 Mcyc ≈ 0.8 % of a core for frames nobody saw. Details:
`docs/gates/G3.md` (G3-F26).

**Upstream.** Not reported. Upstream gpui_windows has the same gap (no display-state
notification). Suggested upstream change: exactly this one.

## Upstream

Upstream zed `main` (raw `crates/gpui_windows/src/platform.rs`, fetched 2026-10-21) still
invalidates every window on every vblank and `crates/gpui_windows/src/window.rs` has no
`frame_waker`; crates.io has no `gpui-pre` newer than 0.3.9 (2026-10-12). No upstream issue or
PR was searched for. Suggested upstream change: exactly this one — implement `frame_waker` on Windows and
gate the vsync invalidation on it. Drop this patch when a `gpui-pre` release does the same
(re-measure with `cargo xtask bench --scenario one-pane`).
