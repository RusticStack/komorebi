//! Komorebi Japandi design system: design tokens, palette, and theme application.
//!
//! Design principles (our own Japandi aesthetic, inspired by desktop design
//! guidance but expressed through warm, calm, uncluttered surfaces):
//!
//!   Effortless - do what I want, with focus and precision
//!   Calm       - soft, decluttered, fades into the background
//!   Personal   - adapts to how I work
//!   Familiar   - no learning curve; pick it up and go
//!   Coherent   - consistent across every surface
//!
//! Japandi values applied throughout:
//!   • Ma (間) - negative space / breathing room; never crowd a surface
//!   • Warm, natural materials - muted charcoal, sand, beige, soft greens
//!   • Functional beauty - every element earns its place
//!   • Understated refinement - subtle, not flashy

use gpui::{Hsla, Pixels, Window, px};
use gpui_component::Theme;

// ---------------------------------------------------------------------------
// Design tokens
// ---------------------------------------------------------------------------

// Corner radius (tiered for visual hierarchy, softened for warmth):
//   • In-page controls → 6px (buttons, inputs, list rows, pills)
//   • Top-level containers & cards → 8px (panels, cards, overlays)
//
// Spacing (4px grid):
//   • Nav items & list rows: px_2 py_1 (8/4)
//   • Cards / containers: p_3 (12)
//   • Reader body: p_4 (16)
//   • Section gaps: gap_1 (4) → gap_2 (8) → gap_3 (12)

/// In-page element corner radius (buttons, inputs, list rows, pills).
pub const RADIUS_SM: Pixels = px(6.0);
/// Top-level container / card / overlay corner radius.
pub const RADIUS_LG: Pixels = px(8.0);

// ---------------------------------------------------------------------------
// Color helpers
// ---------------------------------------------------------------------------

pub fn komorebi_hex(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Apply the Komorebi Japandi palette (warm charcoal + sand/beige accents).
pub fn apply_komorebi_theme(window: &mut Window, cx: &mut gpui::App) {
    let theme = Theme::global_mut(cx);
    if theme.mode.is_dark() {
        theme.colors.background = komorebi_hex(0x1b1816);
        theme.colors.foreground = komorebi_hex(0xefe9dc);
        theme.colors.muted = komorebi_hex(0x262220);
        theme.colors.muted_foreground = komorebi_hex(0x9a9284);
        theme.colors.border = komorebi_hex(0x2e2a26);
        theme.colors.primary = komorebi_hex(0xe6d9b8);
        theme.colors.primary_foreground = komorebi_hex(0x1f1a14);
        theme.colors.primary_hover = komorebi_hex(0xefe4c8);
        theme.colors.primary_active = komorebi_hex(0xdccda6);
        theme.colors.accent = komorebi_hex(0xe6d9b8);
        theme.colors.accent_foreground = komorebi_hex(0x1f1a14);
        theme.colors.link = komorebi_hex(0xc7b78f);
        theme.colors.sidebar = komorebi_hex(0x161311);
        theme.colors.sidebar_foreground = komorebi_hex(0xc9c2b4);
        theme.colors.sidebar_border = komorebi_hex(0x2e2a26);
        theme.colors.sidebar_accent = komorebi_hex(0x2a2520);
        theme.colors.sidebar_accent_foreground = komorebi_hex(0xf1ebdd);
        theme.colors.sidebar_primary = komorebi_hex(0xe6d9b8);
        theme.colors.sidebar_primary_foreground = komorebi_hex(0x1f1a14);
        theme.colors.list = komorebi_hex(0x1b1816);
        theme.colors.list_active = komorebi_hex(0x26221e);
        theme.colors.list_hover = komorebi_hex(0x221e1a);
        theme.colors.popover = komorebi_hex(0x26221e);
        theme.colors.popover_foreground = komorebi_hex(0xefe9dc);
        theme.colors.input = komorebi_hex(0x1e1a17);
        theme.colors.title_bar = komorebi_hex(0x161311);
        theme.colors.title_bar_border = komorebi_hex(0x2e2a26);
        theme.colors.window_border = komorebi_hex(0x2e2a26);
        theme.colors.danger = komorebi_hex(0xc9684e);
        theme.colors.warning = komorebi_hex(0xc99a4a);
        theme.colors.success = komorebi_hex(0x7e9a6b);
        theme.colors.info = komorebi_hex(0x6b7fa0);
        theme.colors.drop_target = komorebi_hex(0x33291f);
    } else {
        theme.colors.background = komorebi_hex(0xf6f1e7);
        theme.colors.foreground = komorebi_hex(0x2c2620);
        theme.colors.muted = komorebi_hex(0xece4d4);
        theme.colors.muted_foreground = komorebi_hex(0x7d756a);
        theme.colors.border = komorebi_hex(0xddd3c2);
        theme.colors.primary = komorebi_hex(0xb9a67e);
        theme.colors.primary_foreground = komorebi_hex(0x241f18);
        theme.colors.primary_hover = komorebi_hex(0xae9a70);
        theme.colors.primary_active = komorebi_hex(0xa28d61);
        theme.colors.accent = komorebi_hex(0xb9a67e);
        theme.colors.accent_foreground = komorebi_hex(0x241f18);
        theme.colors.link = komorebi_hex(0x8d7a52);
        theme.colors.sidebar = komorebi_hex(0xefe8da);
        theme.colors.sidebar_foreground = komorebi_hex(0x4a4238);
        theme.colors.sidebar_border = komorebi_hex(0xddd3c2);
        theme.colors.sidebar_accent = komorebi_hex(0xe2d8c2);
        theme.colors.sidebar_accent_foreground = komorebi_hex(0x2c2620);
        theme.colors.sidebar_primary = komorebi_hex(0xb9a67e);
        theme.colors.sidebar_primary_foreground = komorebi_hex(0x241f18);
        theme.colors.list = komorebi_hex(0xf6f1e7);
        theme.colors.list_active = komorebi_hex(0xece3d1);
        theme.colors.list_hover = komorebi_hex(0xf0e9db);
        theme.colors.popover = komorebi_hex(0xfbf7ee);
        theme.colors.popover_foreground = komorebi_hex(0x2c2620);
        theme.colors.input = komorebi_hex(0xfbf7ee);
        theme.colors.title_bar = komorebi_hex(0xefe8da);
        theme.colors.title_bar_border = komorebi_hex(0xddd3c2);
        theme.colors.window_border = komorebi_hex(0xddd3c2);
        theme.colors.danger = komorebi_hex(0xb2533c);
        theme.colors.warning = komorebi_hex(0xa87f2e);
        theme.colors.success = komorebi_hex(0x5f7d4e);
        theme.colors.info = komorebi_hex(0x4f668a);
        theme.colors.drop_target = komorebi_hex(0xe2d8c2);
    }
    // Komorebi geometry: tiered corner radius.
    //   radius    = in-page controls (6px, Japandi-softened)
    //   radius_lg = top-level containers / dialogs (8px)
    theme.radius = RADIUS_SM;
    theme.radius_lg = RADIUS_LG;
    window.refresh();
}
