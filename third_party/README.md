# Vendored crates

The GPUI stack of Komorebi 1.0, vendored as Skua does (Skua D-108, D-116): each folder is the
exact published crates.io source plus the owner-approved patches its `SKUA-PATCH.md` (Skua's)
and `KOMOREBI-PATCH.md` (Komorebi's) describe. Every changed line is marked `SKUA PATCH` or
`KOMOREBI PATCH`. Each crate keeps its own licence file (Apache-2.0).

| Crate | Version | Source | Patches |
|---|---|---|---|
| gpui-pre | 0.3.9 | crates.io, via Skua 7e0fdcf | Skua: view splice and cached window control areas (G3-F27). Komorebi: `windows-manifest-provided-by-host` feature, `RenderImage::from_shared_bgra` (an image over a shared pixel buffer); per-frame cost patches 01-06 (scene sort, decoration runs, font features, tab stop index, uniform list measure, text runs; see KOMOREBI-PATCH.md); `image` codecs trimmed to bmp, gif, ico, jpeg, png, webp (724,480 bytes) |
| gpui-pre-windows | 0.3.9 | crates.io, via Skua 7e0fdcf | Skua: demand-driven frames (G1-F10), no frames while the display is off (G3-F26). Komorebi: fxc.exe discovery with a clear error, `KOMOREBI_GPUI_FORCE_WARP`, `prepare_devices` (one DirectX device at start-up), path intermediate textures made on first use, `image` codecs trimmed as in gpui-pre |
| gpui-component | 0.8.0 | crates.io, via Skua 7e0fdcf | Skua: Kit style hooks (G3-F20). Komorebi: `pt-PT` strings for Kit's built-in text, a dialog host named by its title and an alert host, `Input::aria_expanded`, access keys on menu items, `native-menu-svg` feature off by default (drops a second resvg stack, 428,032 bytes) (`KOMOREBI-PATCH.md`) |
| gpui-base | 0.8.0 | crates.io | Komorebi: the dialog host's accessible name, the keyboard shortcut in a test snapshot, `text-formats` feature off by default (drops the Markdown and HTML parsers, 889,856 bytes) (`KOMOREBI-PATCH.md`) |

They are wired from the root `Cargo.toml` with `[patch.crates-io]`; the pins stay exact
(`gpui = { package = "gpui-pre", version = "=0.3.9" }`, `gpui-kit = "=0.8.0"`), and gpui-pre
and gpui-kit move together. `third_party` is in the workspace `exclude` list, so Komorebi's
`cargo fmt`, `cargo clippy` and `cargo test` do not run on these crates.

To move to a newer release: take the published crate source (the registry copy, without
`.cargo-ok`), re-apply each patch from its description, re-run the checks in
docs/GPUI-BUILD.md, and keep Skua and Komorebi on the same base where possible. Build and
patch details: docs/GPUI-BUILD.md.
