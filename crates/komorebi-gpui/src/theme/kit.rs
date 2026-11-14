//! GPUI Kit's theme from the tokens (IMPLEMENTATION.md section 2), so the
//! Kit pieces Komorebi composes (inputs, menus, dialogs, scrollbars) draw
//! with the brand's paper and ink.
//!
//! Skua's contract (AGENTS.md "Kit theme writes", cookbook_theme): one
//! `Theme::change(mode)`, then one `Theme::update` that assigns every
//! field; nothing else ever writes Kit's theme (`sync_system_appearance`,
//! a bare `Theme::global_mut` or a `ThemeRegistry` write would reload
//! Kit's defaults). Komorebi has two schemes, so a scheme or text size
//! change runs the same two calls again, re-assigning every field
//! (`assign_colors` sets all of them; the test proves it).
//!
//! | Kit field(s) | Token |
//! |---|---|
//! | background, title_bar, sidebar | canvas |
//! | list*, table*, group_box, accordion, popover, tab_bar, status_bar | paper |
//! | foreground, caret, *_foreground on light fills, link | ink |
//! | muted_foreground, tab_foreground, description_list_label_foreground | ink-2 |
//! | accent, list_hover/active, table_hover/active, button_hover, secondary, sidebar_accent, tab_bar_segmented, skeleton | wash |
//! | primary, button_primary, slider_bar, progress_bar, switch | ink; hover and active at 0.9 |
//! | primary_foreground, button_primary_foreground, switch_thumb, slider_thumb | on-ink |
//! | border, sidebar/title_bar/status_bar/window border, table_row_border, list_active_border | clear (panes are split by gaps) |
//! | input | line (the one 1 px edge, under composer fields) |
//! | ring, drag_border | ink |
//! | selection | wash (text stays ink) |
//! | danger, warning, success, info (+ buttons) | ink: no status hues |
//! | overlay | black at 0.32 |
//! | scrollbar | clear; thumb ink-3 (spot is 1.9:1, below 3:1 for a control) |
//! | red ... cyan, chart_* | ink-2, so nothing colourful can leak in |

use gpui::{App, Hsla, px};
use gpui_kit::component::{Theme, ThemeColor, ThemeMode};

use super::{Scheme, Tokens, radius};

fn hover(c: Hsla) -> Hsla {
    c.opacity(super::motion::INK_HOVER)
}

/// Assigns every `ThemeColor` field from the palette.
pub fn assign_colors(c: &mut ThemeColor, t: &Tokens) {
    let p = &t.color;
    let (canvas, paper, wash, ink, ink_2, ink_3, on_ink, clear) = (
        p.canvas, p.paper, p.wash, p.ink, p.ink_2, p.ink_3, p.on_ink, p.clear,
    );

    c.background = canvas;
    c.foreground = ink;
    c.accent = wash;
    c.accent_foreground = ink;
    c.accordion = paper;
    c.border = clear;
    c.input = p.line;
    c.ring = ink;
    c.caret = ink;
    c.selection = wash;
    c.overlay = p.overlay;
    c.window_border = clear;

    c.button = paper;
    c.button_hover = wash;
    c.button_active = wash;
    c.button_foreground = ink;

    (
        c.primary,
        c.primary_hover,
        c.primary_active,
        c.primary_foreground,
    ) = (ink, hover(ink), hover(ink), on_ink);
    (
        c.secondary,
        c.secondary_hover,
        c.secondary_active,
        c.secondary_foreground,
    ) = (wash, wash, wash, ink);
    // No status hues: errors are words and an icon (BRAND.md persimmon rule).
    for (base, h, a, fg) in [
        (
            &mut c.danger,
            &mut c.danger_hover,
            &mut c.danger_active,
            &mut c.danger_foreground,
        ),
        (
            &mut c.success,
            &mut c.success_hover,
            &mut c.success_active,
            &mut c.success_foreground,
        ),
        (
            &mut c.warning,
            &mut c.warning_hover,
            &mut c.warning_active,
            &mut c.warning_foreground,
        ),
        (
            &mut c.info,
            &mut c.info_hover,
            &mut c.info_active,
            &mut c.info_foreground,
        ),
    ] {
        (*base, *h, *a, *fg) = (ink, hover(ink), hover(ink), on_ink);
    }

    (
        c.button_primary,
        c.button_primary_hover,
        c.button_primary_active,
        c.button_primary_foreground,
    ) = (
        c.primary,
        c.primary_hover,
        c.primary_active,
        c.primary_foreground,
    );
    (
        c.button_secondary,
        c.button_secondary_hover,
        c.button_secondary_active,
        c.button_secondary_foreground,
    ) = (c.secondary, c.secondary_hover, c.secondary_active, ink);
    (
        c.button_danger,
        c.button_danger_hover,
        c.button_danger_active,
        c.button_danger_foreground,
    ) = (
        c.danger,
        c.danger_hover,
        c.danger_active,
        c.danger_foreground,
    );
    (
        c.button_success,
        c.button_success_hover,
        c.button_success_active,
        c.button_success_foreground,
    ) = (
        c.success,
        c.success_hover,
        c.success_active,
        c.success_foreground,
    );
    (
        c.button_warning,
        c.button_warning_hover,
        c.button_warning_active,
        c.button_warning_foreground,
    ) = (
        c.warning,
        c.warning_hover,
        c.warning_active,
        c.warning_foreground,
    );
    (
        c.button_info,
        c.button_info_hover,
        c.button_info_active,
        c.button_info_foreground,
    ) = (c.info, c.info_hover, c.info_active, c.info_foreground);

    c.group_box = paper;
    c.group_box_foreground = ink;
    c.description_list_label = wash;
    c.description_list_label_foreground = ink_2;
    c.drag_border = ink;
    c.drop_target = wash;
    (c.link, c.link_hover, c.link_active) = (ink, ink, ink);

    c.list = paper;
    c.list_active = wash;
    c.list_active_border = clear;
    c.list_even = paper;
    c.list_head = paper;
    c.list_hover = wash;

    c.muted = wash;
    c.muted_foreground = ink_2;
    c.popover = paper;
    c.popover_foreground = ink;
    c.progress_bar = ink;
    c.scrollbar = clear;
    c.scrollbar_thumb = ink_3;
    c.scrollbar_thumb_hover = ink_2;
    c.skeleton = wash;
    c.slider_bar = ink;
    c.slider_thumb = on_ink;
    c.switch = ink;
    c.switch_thumb = on_ink;

    c.sidebar = canvas;
    c.sidebar_accent = wash;
    c.sidebar_accent_foreground = ink;
    c.sidebar_border = clear;
    c.sidebar_foreground = ink;
    c.sidebar_primary = ink;
    c.sidebar_primary_foreground = on_ink;

    c.tab = clear;
    c.tab_active = paper;
    c.tab_active_foreground = ink;
    c.tab_bar = paper;
    c.tab_bar_segmented = wash;
    c.tab_foreground = ink_2;

    c.table = paper;
    c.table_active = wash;
    c.table_active_border = clear;
    c.table_even = paper;
    c.table_head = paper;
    c.table_head_foreground = ink_2;
    c.table_foot = paper;
    c.table_foot_foreground = ink_2;
    c.table_hover = wash;
    c.table_row_border = clear;

    c.title_bar = canvas;
    c.title_bar_border = clear;
    c.status_bar = paper;
    c.status_bar_border = clear;

    for hue in [
        &mut c.chart_1,
        &mut c.chart_2,
        &mut c.chart_3,
        &mut c.chart_4,
        &mut c.chart_5,
        &mut c.chart_bullish,
        &mut c.chart_bearish,
        &mut c.chart_grid,
        &mut c.red,
        &mut c.red_light,
        &mut c.green,
        &mut c.green_light,
        &mut c.yellow,
        &mut c.yellow_light,
        &mut c.blue,
        &mut c.blue_light,
        &mut c.magenta,
        &mut c.magenta_light,
        &mut c.cyan,
        &mut c.cyan_light,
    ] {
        *hue = ink_2;
    }
}

/// Writes Kit's theme: one `Theme::change`, one `Theme::update`.
pub fn apply(t: &Tokens, cx: &mut App) {
    let mode = match t.scheme {
        Scheme::Light => ThemeMode::Light,
        Scheme::Dark => ThemeMode::Dark,
    };
    let t = t.clone();
    Theme::change(mode, None, cx);
    Theme::update(cx, move |theme| {
        assign_colors(&mut theme.colors, &t);
        theme.font_family = t.text.ui_family.clone();
        theme.font_size = t.text.body.size;
        theme.radius = px(radius::FULL);
        theme.radius_lg = px(radius::CARD);
        // No shadow, gradient, blur or glow anywhere (BRAND.md "Shape").
        theme.shadow = false;
        // Komorebi draws its own focus ring (2 px ink, 2 px paper gap); Kit's
        // translucent ring is off, so a focused Kit field shows a 1 px ink
        // edge instead.
        theme.focus_ring = false;
        theme.transparent = t.color.clear;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ThemeInput, tokens};
    use komorebi::db::ThemeChoice;
    use komorebi::theme::SystemTheme;

    /// Two passes from two different fills: every field ends up equal, so
    /// no field is left at a Kit default (Skua's proof).
    #[test]
    fn every_kit_colour_is_assigned() {
        let t = tokens(&ThemeInput::new(ThemeChoice::Dark, SystemTheme::default()));
        let mut a = ThemeColor::default();
        let mut b = ThemeColor::default();
        let red: Hsla = gpui::red();
        let json = |c: &ThemeColor| serde_json::to_value(c).expect("serialize");
        // Make `b` differ everywhere first.
        let mut v = json(&b);
        if let Some(map) = v.as_object_mut() {
            for value in map.values_mut() {
                *value = serde_json::to_value(red).expect("serialize red");
            }
        }
        b = serde_json::from_value(v).expect("deserialize");
        assign_colors(&mut a, &t);
        assign_colors(&mut b, &t);
        assert_eq!(json(&a), json(&b));
    }
}
