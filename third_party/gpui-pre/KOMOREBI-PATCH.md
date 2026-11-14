# Komorebi patch: gpui-pre 0.3.9

On top of the Skua copy (`SKUA-PATCH.md`: published `gpui-pre 0.3.9` plus Skua's G3-F27
splice), exported from Skua at 7e0fdcf. Every Komorebi line is marked `KOMOREBI PATCH`.

## Manifest provided by the host (`windows-manifest-provided-by-host`)

**Problem.** On Windows, `gpui-pre-platform` turns on gpui-pre's `windows-manifest` feature
unconditionally, and gpui-pre's build script then embeds its own RT_MANIFEST id 1 through
`embed-resource` into every binary that links it. Komorebi links its own resources
(`assets/komorebi.rc`: icon, version information and RT_MANIFEST id 1), so the link fails with
`CVTRES : fatal error CVT1100: duplicate resource. type:MANIFEST, name:1` and then `LNK1123`
(reproduced in the feasibility spike, REPORT.md S6).

**Change.**
- `Cargo.toml`: a new feature `windows-manifest-provided-by-host = []` (the name of gpui-fast's
  equivalent).
- `build.rs`: with that feature on, `embed_resource()` is skipped (the build script reads
  `CARGO_FEATURE_WINDOWS_MANIFEST_PROVIDED_BY_HOST`). The spike's environment variable
  `GPUI_MANIFEST_PROVIDED_BY_HOST` is gone.

**What the host must carry instead.** GPUI's `resources/windows/gpui.manifest.xml` declares
asInvoker, Windows 10 support, Common Controls 6, `dpiAware true/pm`, `dpiAwareness
PerMonitorV2` and `heapType SegmentHeap`. GPUI sets no DPI awareness at run time, so a host
manifest without PerMonitorV2 makes the window DPI-unaware (bitmap-scaled). Komorebi's
`assets/komorebi.manifest` carries all of them except SegmentHeap (to be measured).

**Feature unification.** Features unify per build, so every binary built together with a crate
that enables this feature also loses GPUI's manifest. In Komorebi only `crates/komorebi-gpui`
links GPUI.

**Upstream.** Not reported. gpui-fast 0.1.10 has the same switch as a feature; suggested
upstream change: exactly this one.

## `RenderImage::from_shared_bgra`: an image over a shared pixel buffer (`src/assets.rs`)

**Problem.** `RenderImage::new` takes `image::Frame`s, which own their pixels
(`RgbaImage = ImageBuffer<Rgba<u8>, Vec<u8>>`). An app that already keeps its decoded pixels in an
`Arc<[u8]>` (Komorebi's reader keeps its rasterized tiles in one) has to copy them into a `Vec` for
the image, so every tile exists twice on the CPU side, besides its copy in the GPU atlas.

**Change.** `RenderImage::from_shared_bgra(width, height, Arc<[u8]>) -> Option<RenderImage>` makes a
one-frame image whose `as_bytes(0)` is the caller's buffer, with no copy (`None` unless the length is
`width * height * 4`). `size`, `delay`, `frame_count`, `Debug` and `as_bytes` understand it; the
painting code (`Window::paint_image`) is unchanged because it only calls `as_bytes`. `RenderImage::new`
is as before. The bytes are BGRA premultiplied, as the frames of `new` are.

**Use in Komorebi** (not done in this patch; the reader belongs to another lane):
`crates/komorebi-gpui/src/views/reader/worker.rs`, `image()`:
`RenderImage::from_shared_bgra(pixels.width, pixels.height, pixels.data.clone()).map(Arc::new)`
instead of `from_raw(.., pixels.data.to_vec())` and `RenderImage::new(vec![Frame::new(buf)])`.

**Gain.** One CPU copy less per tile and no `to_vec` burst when a tile arrives: private bytes 2 s
after the heavy message opened, 100.66 to 97.33 MB (9 rounds, valid batch).

## Image codecs trimmed to what Komorebi decodes (`Cargo.toml`)

**Problem.** `gpui-pre` (and `gpui-pre-windows`) enabled all fifteen codecs of the `image` crate (`bmp`,
`dds`, `exr`, `ff`, `gif`, `hdr`, `ico`, `jpeg`, `png`, `pnm`, `qoi`, `rayon`, `tga`, `tiff`, `webp`).
Komorebi decodes no image through GPUI: its reader and composer hand GPUI finished pixels
(`RenderImage`), its icons are SVG, and mail images use the library's own decoders.

**Change.** The `image` dependency keeps `bmp`, `gif`, `ico`, `jpeg`, `png` and `webp` (the formats
`gpui::ImageFormat` and the Windows clipboard code name that a desktop app plausibly meets:
`GifDecoder` is used directly in `platform.rs`, and the clipboard hands back DIB/BMP, PNG, JPEG,
GIF and WebP) and drops `dds`, `exr`, `ff`, `hdr`, `pnm`, `qoi`, `rayon`, `tga` and `tiff`.
`ImageFormat::Tiff` and `::Pnm` still compile; decoding one now returns the `image` crate's
unsupported-format error. Nothing in Komorebi asks GPUI to decode either. The same change is in
`gpui-pre-windows/Cargo.toml`; the exe links one copy of `image` with the union.

**Measured (release, one worktree, the same profile).** Exe 17,499,136 bytes before (after the
`gpui-base` `text-formats` patch), 16,774,656 after: 724,480 bytes smaller (4.14 percent).
`Cargo.lock` is unchanged (it lists a crate's optional dependencies).

**Moving to a newer GPUI.** Re-apply the feature list; check `image::` use first.

# Per-frame cost patches (opt/fork-core, Komorebi 1.0 optimization pass)

Six pixel-identical changes that cut per-frame CPU and allocations. Each is a separate commit
and a separate Skua patch (`skua-patches/core-NN-*.patch`). Baseline (scroll): median 3.41 ms,
p95 4.01 ms, 5,301 allocations and 3.59 MB per frame; (typing) median 2.95 ms, p95 3.63 ms,
6,778 allocations and 4.45 MB per key. Measured gains are recorded in the commit that closes the
pass.

## 01 scene-sort (`scene.rs`)

**Problem.** `Scene::finish` ran `sort_by_key` (driftsort, stable) on six primitive vectors every
frame: 6.2% of scroll CPU samples, plus a scratch buffer per sort.
**Change.** Sort `(key, index)` pairs with `sort_unstable`, return early when already sorted, then
apply the permutation in place. Same result as the stable sort (the index breaks ties).
**Test.** Unit tests compare against the stable sort for many shapes. Microbench (300 / 1500 /
6000 items): 43.8 / 198.1 / 815.2 us down to 7.5 / 24.0 / 188.0 us.
**Upstream.** Applicable as is.

## 02 wrapped-line-decorations (`text_system.rs`, `text_system/line.rs`)

**Problem.** `shape_text` allocated `Vec::with_capacity(32)` of `DecorationRun` for every wrapped
line (about 493 allocations and 1.4 MB per frame).
**Change.** `WrappedLine.decoration_runs` is a `SmallVec<[DecorationRun; 1]>`.

## 03 font-features-default (`text_system/font_features.rs`)

**Problem.** `FontFeatures::default()` allocated a fresh `Arc<Vec>` on every `Window::text_style`
call (about 148 allocations per frame).
**Change.** `Default` clones one shared static empty `Arc`.

## 04 tabstop-lazy-index (`tab_stop.rs`)

**Problem.** `TabStopMap` rebuilt a hash map and a SumTree while the tree was painted, every
frame, though focus navigation reads them rarely.
**Change.** `insert` and group calls only append to a history; the map and tree are built on the
first query (`OnceCell`). `clear` keeps capacities.

## 05 uniform-list-single-measure (`elements/uniform_list.rs`)

**Problem.** In `Auto` sizing, `request_layout` and `prepaint` each called `measure_item`
(render and lay out item 0): 9.6% of scroll CPU.
**Change.** `request_layout` measures only for `Infer`; `prepaint` measures once and reuses the
size. Layout output is unchanged.

## 06 text-runs-inline (`elements/text.rs`)

**Problem.** `TextLayout::layout` allocated a `Vec<TextRun>` for the common single-run case
(about 150 allocations per frame).
**Change.** `SmallVec<[TextRun; 1]>`.

## Measured gains (release, 2,000-message cache, harness `scripts/opt-ab.ps1`, 15 rounds)

Allocations (counting allocator, `scripts/opt-prof.ps1`, median of 3 runs), baseline -> all six:

| | allocations | bytes |
|---|---|---|
| scroll, per frame | 5,301 -> 4,254 (-20%) | 3.59 -> 1.72 MB (-52%) |
| typing, per key | 6,778 -> 5,634 (-17%) | 4.45 -> 2.43 MB (-45%) |

Per patch, scroll per frame: 01 5,298 / 3.47 MB (the sort scratch buffers only), 02 4,814 / 2.05 MB,
03 4,665 / 2.04 MB, 04 4,553 / 1.85 MB, 05 4,324 / 1.72 MB, 06 4,254 / 1.72 MB.

Process cycles per scroll frame: 7.50 -> 6.69 M (-10.8%) against the harness exe, 7.55 -> 6.53 M
(-13.6%) against a base built in the same target directory. Each single patch is within the spread
(-1.4% to -3.7%).

Frame time (wheel or key to frame rendered) did NOT improve: no single patch is outside the noise
floor, and the six together read +0.23 ms on the scroll median (3.44 -> 3.67, floor 0.18) and +0.28
ms on the typing median (3.02 -> 3.30, floor 0.27). The same sign came back with A and B swapped
and against a base built in the same target directory, so it may be real. The harness metric
includes waiting for the next display frame, which can hide or shift CPU savings. The patches stay
as separate commits so a verdict that rejects the frame-time shift can revert them one by one.
Launch (first frame) and memory are within spread (working set -2.3 MB).
