# Skua patch: gpui-pre 0.3.9 (D-108, G3-F27)

Base: the published crate `gpui-pre 0.3.9` from crates.io, byte-for-byte (gpui-pre snapshot of zed
`gpui`), license Apache-2.0 (`LICENSE-APACHE`). Only the registry marker `.cargo-ok` was dropped.
Wired from the workspace root with:

```toml
[patch.crates-io]
gpui-pre = { path = "third_party/gpui-pre" }
```

The pin stays `gpui = { package = "gpui-pre", version = "=0.3.9" }`; `Cargo.lock` changed only in
the `gpui-pre` entry's `source`/`checksum` lines. `third_party` is in the workspace `exclude` list,
so Skua's lints, fmt, clippy and tests do not run on this crate. Every changed line is marked
`SKUA PATCH (G3-F27)`.

## Problems

1. **Ancestors of a notified view always re-render.** `Window::mark_view_dirty` marks every
   ancestor of a notified view dirty, and `Window::draw` rebuilds the whole tree from the root:
   every uncached ancestor renders, lays out and prepaints again. A cached view only saves its
   own subtree. In Skua a cursor blink (the terminal pane, inside the cached center content) or a
   status sample (the cached status bar) re-rendered GPUI Kit's `Root`/`WindowBorder` (≈ 0.3
   Mcyc) and the shell with its Kit split (≈ 0.45 Mcyc) on every frame (G3-F24).
2. **Reused cached views lost their window control areas.** `Window::reuse_paint` replays the
   scene, listeners, cursor styles, input handlers and tab stops of a reused range, but not
   `Frame::window_control_hitboxes`. After any frame that reused a cached view holding
   `.window_control_area(..)` (Skua's titlebar drag regions and caption buttons, cached since
   G3-F24), `WM_NCHITTEST` found no caption/min/max/close area until the next full frame.

## Change

- `window.rs`: `PaintIndex::window_control_hitboxes_index`; `reuse_paint` replays that range
  (problem 2). `Frame::{view_slots, root_prepaint, root_paint, root_size}`;
  `Window::{cached_view_depth, recording_view_slots}`; `Window::draw` asks `plan_splice` before
  the notified views are marked dirty and `draw_roots` either draws the root in full (recording
  slots) or calls `prepaint_spliced` / `paint_spliced`.
- `window/splice.rs` (new): a **view slot** is a top-level cached view of the last frame (all its
  ancestors uncached) with the context it was drawn in (element id stack, text style stack,
  content mask, rem size, opacity, parent view, its dispatch node) and its prepaint/paint ranges.
  `plan_splice` returns the slots to re-render when every notified view resolves, through the
  last frame's view path, to exactly one slot, and nothing else asks for a full draw: not
  `refreshing` (any `Window::refresh`, hover/focus/activation/bounds change, `prepare_frame`),
  same viewport and scale, no a11y tree this frame, no inspector, prompt or drag, all slots
  complete and in order. A notified model or uncached view, or a cached view nested in another
  cached view's subtree only, falls back to the full draw.
  `prepaint_spliced` copies the root's last prepaint range segment by segment around the holes
  (`reuse_prepaint_segment`: hitboxes, tooltips, accessed element states, line layouts, dispatch
  nodes, deferred draws) and re-renders each hole with `prepaint_view` at its last bounds in its
  restored context; `paint_spliced` does the same for the paint range (`reuse_paint` + the hole's
  `paint_view`) and moves the other slots (and their `ViewElementState` ranges) to their new
  indices. The entities the replayed ancestors accessed last frame stay tracked.
  `Window::window_control_area_under_mouse()` (test-support) reads the non-client hit test for
  tests (`cookbook_cached_view`).
- `view.rs`: `ViewElement::{replay, slot}` (`Entity::cached` / `AnyView::cached` keep an
  `AnyView` to re-render); a top-level cached view records its slot in prepaint and completes it
  in paint; `prepaint_view` / `paint_view` report their ranges; `prepaint_view_slot`,
  `paint_view_slot`, `set_view_slot_ranges` for `splice.rs`.
- `key_dispatch.rs`: `DispatchNodeId::{index, from_index}`; `DispatchTree::reuse_span` (copy a
  pre-order span that need not be a whole subtree, parents remapped) and `reset_stacks`.
- `text_system/line_layout.rs`: `LineLayoutIndex::shifted`.

Semantics kept: a notify still re-renders the notified view's whole cached slot (uncached views
inside it, and its nested cached views, as upstream does on a cache miss); every redraw that is
not a pure notify of cached views is the upstream full draw.

## Result (owner machine, release, temporary per-phase trace, not committed)

Same traced binary with the splice on or off (env switch in the trace build), interleaved,
`skua --panes 1`, median per frame (`QueryThreadCycleTime`, 3410 MHz):

| Frames | Splice off (upstream) | Splice on |
| --- | --- | --- |
| idle blink/status, 2 runs | 0.98 / 0.70 ms (gpui draw 2.26 / 1.57 Mcyc) | 0.43 / 0.57 ms (gpui draw 0.75 / 1.02 Mcyc) |
| 60-key echo (WM_CHAR posted to the window), 2 runs | 0.69 / 0.87 ms (gpui 1.52 / 1.89), 114 / 118 frames | 0.51 / 0.61 ms (gpui 0.95 / 1.12), 117 / 124 frames |

A spliced frame: hole prepaint 0.20–0.34 Mcyc (the terminal's `PaneGrid`), segment copies
0.04 (prepaint) + 0.10–0.20 (paint, the scene replay), hole paint 0.05–0.10; the renderer's
`Present` (0.6 Mcyc) is now the largest item. Output: client-area captures (PrintWindow) after
typing a command, splice vs upstream draw, differ only in the cursor blink phase and the status
digits. Goldens unchanged (`golden check --all` ×2). More in `docs/gates/G3.md` (G3-F27).

## Upstream

Not reported upstream. Problem 2 is a plain upstream bug (any app with a cached view holding a
window control area). Problem 1 is a design limit of gpui's view cache; the splice is
upstream-able as an opt-in. Drop this patch when a `gpui-pre` release does the same (D-108 rule:
`gpui-pre` and `gpui-kit` move together); re-measure with the G3-F27 frame trace and
`cargo xtask bench --scenario one-pane`.
