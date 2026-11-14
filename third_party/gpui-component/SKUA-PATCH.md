# Skua patch: gpui-component 0.8.0 (D-116, G3-F20)

Base: the published crate `gpui-component 0.8.0` from crates.io, byte-for-byte (the component
layer of GPUI Kit; `gpui-kit 0.8.0` is a facade that re-exports it as `gpui_kit::component`),
license Apache-2.0 (`LICENSE-APACHE`). Only the registry marker `.cargo-ok` was dropped. Wired
from the workspace root with:

```toml
[patch.crates-io]
gpui-component = { path = "third_party/gpui-component" }
```

D-116 names `third_party/gpui-kit`; the facade crate has no code to patch (`src/lib.rs` only
re-exports), so the vendored crate is the one that holds the hard-coded looks. The pins stay
`gpui-kit =0.8.0` (which requires `gpui-component` `0.8.0`, resolved to 0.8.0). `Cargo.lock` changed only in the
`gpui-component` entry's `source`/`checksum` lines.

`third_party` is in the workspace `exclude` list, so Skua's lints, fmt, clippy and tests do not
run on this crate. Every changed line is marked `SKUA PATCH (G3-F20)`. All patches are additive:
with no caller opt-in, Kit renders exactly as upstream.

## Patches

| # | File | What | Why (Skua) | Upstream-able |
| --- | --- | --- | --- | --- |
| 1 | `src/dialog/dialog.rs` | `surface_shadow`: a shadow set on the `Dialog` through `Styled::shadow(..)` replaces `dialog_shadow` (resting and slide-in animation; the animation scales its alpha by the same delta) | C13 `SHADOW_DIALOG`: Kit applied its own `shadow-xl` after `refine_style`, so the caller's shadow never showed (escalation G3-F2 item 2) | yes: "caller style wins" is how every other refinable field already behaves |
| 2 | `src/notification.rs` | `card_shadow`: same rule for `Notification` (entrance and exit animations keep the caller's shadow, alpha × opacity³ as Kit does) | C12 `SHADOW_FLOATING`: Kit's animator re-applied `toast_shadow` on every frame (gotcha M3-19) | yes, same reason |
| 3 | `src/menu/popup_menu.rs`, `src/menu/menu_item.rs`, `src/menu/mod.rs` | `PopupMenuStyle { border, radius, shadow, item_hover, separator }` + `PopupMenu::menu_style(..)`; `MenuItemElement::hover_bg` (crate-private); `PopupMenuStyle` re-exported | C15 surface (1 px `BORDER_STRONG`, `R_MENU`, `SHADOW_MENU`), item hover `MENU_ITEM_HOVER`, separator `MENU_SEPARATOR` per menu. The only Kit inputs were the global `radius`, `accent` and `border`, shared with buttons, lists and tables (escalation G3-F2 items 3-5) | yes: a per-menu style is a common request; field names follow Kit's theme fields |
| 4 | `src/input/input.rs` | `Input::caret(bool)` (default `true`); `false` projects a caret color of alpha `f32::EPSILON` | the create-dialog fixture still has no caret (`dialogs.md`) while `ui-text-input`/`palette` keep theirs; the theme `caret` is global (escalation G3-F16 item 2) | yes, but upstream would better add a `show_caret` flag in `gpui-base`'s `InputEditorStyle`: gpui-base reads an alpha of exactly 0 as "unset" and falls back to the foreground (`InputEditorStyle::resolved`), hence the epsilon (below one 8-bit step: nothing is painted) |

Not patched: the 2 px separator width (`popup_menu.rs` `border_b(px(2.))`; accepted at G3-F2), item
radius (`theme.radius.min(8)` = `R_MD`, as specified), keycaps (`Kbd` is not used: C02 draws Orca's
`ButtonKeyHint` from tokens), backdrop blur (GPUI 0.3.9 has no backdrop filter; see G3.md).

## Upstream

Not reported upstream yet (longbridge/gpui-component). Drop a patch row when a `gpui-kit` release
ships an equivalent hook and the pins move (D-108 rule: `gpui-pre` and `gpui-kit` move together).
Re-check with `cargo xtask golden check --all` (goldens `dialog-worktree-create`, `ui-modal`,
`ui-menu`, `shell-new-tab-menu`, `ui-toast`).
