//! Geometry and motion tokens (IMPLEMENTATION.md section 1: `radius.rs`,
//! `space.rs`, `layout.rs`, `motion.rs`; sizes from the mockups' `k.css`
//! and `app.html`). Logical px on the 4 px grid. Text sizes are not here:
//! they grow with Windows' text size (`text.rs`).

/// Corner radii.
pub mod radius {
    /// Every control and spot: a pill or a circle (GPUI clamps to half the
    /// shorter side).
    pub const FULL: f32 = 9999.;
    /// List rows.
    pub const ROW: f32 = 16.;
    /// Panes.
    pub const SURFACE: f32 = 12.;
    /// Cards, notices and dialogs inside or over paper.
    pub const CARD: f32 = 12.;
    /// The seal's corner, as a fraction of its side (drawn by its SVG path,
    /// never by a styled div: the seal is the only square).
    pub const SEAL_CORNER: f32 = 0.16;
}

/// Spacing on the 4 px grid (Skua's `SP_*` names).
pub mod space {
    pub const SP_0_5: f32 = 2.;
    pub const SP_1: f32 = 4.;
    pub const SP_2: f32 = 8.;
    pub const SP_2_5: f32 = 10.;
    pub const SP_3: f32 = 12.;
    pub const SP_3_5: f32 = 14.;
    pub const SP_4: f32 = 16.;
    pub const SP_5: f32 = 20.;
    pub const SP_6: f32 = 24.;
    pub const SP_7: f32 = 28.;
    pub const SP_8: f32 = 32.;
    pub const SP_10: f32 = 40.;
    /// The gap between panes, and between panes and the window edge.
    pub const PANE_GAP: f32 = SP_2;
}

/// Fixed sizes of the shell and controls.
pub mod layout {
    pub const TITLEBAR_H: f32 = 48.;
    pub const CAPTION_BTN_W: f32 = 46.;
    /// The caption glyphs (Windows draws 10 px glyphs; the Fluent 16 px
    /// icons carry a 2 px margin, so 12 px gives the same ink).
    pub const CAPTION_GLYPH: f32 = 12.;
    /// The ink mark in the title bar.
    pub const TITLE_MARK: f32 = 16.;
    pub const CMD_BAR_H: f32 = 56.;
    /// The folder pane: 256 px, so a line keeps room for its label beside a 20 px
    /// icon, a count and the fold column (pt-PT "Caixa de entrada", "Rascunhos
    /// neste PC" whole).
    pub const NAV_W: f32 = 256.;
    pub const RAIL_W: f32 = 56.;
    pub const LIST_W: f32 = 384.;
    pub const LIST_W_COMPACT: f32 = 320.;
    pub const AGENDA_W: f32 = 300.;
    pub const SETNAV_W: f32 = 220.;
    pub const ROW_H: f32 = 76.;
    /// A message row's tag (its account, or "Account · Folder") at most,
    /// before the preview on line 3.
    pub const ROW_TAG_MAX_W: f32 = 160.;
    pub const CONTACT_ROW_H: f32 = 56.;
    /// The search pill in the title bar.
    pub const SEARCH_W: f32 = 440.;
    pub const SEARCH_H: f32 = 32.;
    /// Spots and avatars.
    pub const SPOT: f32 = 32.;
    pub const AVATAR_SM: f32 = 22.;
    pub const AVATAR_MD: f32 = 40.;
    pub const AVATAR_LG: f32 = 72.;
    /// The current-place spot of nav items and settings pages.
    pub const PLACE_SPOT: f32 = 6.;
    /// The current-place spot under a tab or a filter pill.
    pub const TAB_SPOT: f32 = 4.;
    /// The selected row's marker: a 4 x 20 ink pill at its left edge.
    pub const MARKER_W: f32 = 4.;
    pub const MARKER_H: f32 = 20.;
    /// Icons in dense places: menus, small (32 px) buttons, row marks,
    /// notices (Fluent's 16 px set).
    pub const ICON: f32 = 16.;
    /// The standard icon beside 14 px text: navigation lines, command bar
    /// and icon buttons, 36 and 40 px buttons, settings cards (Fluent: "20 px
    /// standard"; its 20 px set, regular at rest and filled when selected).
    pub const ICON_LG: f32 = 20.;
    /// The glyph inside a select's or pill's trailing chevron, a fold
    /// chevron and a tab's close glyph (Fluent's 12 px set: information
    /// only, never a target of its own).
    pub const ICON_SM: f32 = 12.;
    /// Round icon buttons.
    pub const ICON_BTN: f32 = 36.;
    /// Icon buttons of the composer's format bar.
    pub const ICON_BTN_SM: f32 = 32.;
    /// Pills: buttons, selects, segments.
    pub const CONTROL_H: f32 = 32.;
    /// Large pills: New mail, nav items, attachments.
    pub const CONTROL_H_LG: f32 = 40.;
    /// Send and Reply.
    pub const CONTROL_H_MD: f32 = 36.;
    /// Recipient chips.
    pub const CHIP_H: f32 = 28.;
    pub const FIELD_H: f32 = 48.;
    /// A settings card's least height.
    pub const CARD_H: f32 = 64.;
    /// The label column of a composer field ("To", "Subject").
    pub const FIELD_LABEL_W: f32 = 64.;
    pub const TOGGLE_W: f32 = 40.;
    pub const TOGGLE_H: f32 = 20.;
    pub const TOGGLE_KNOB: f32 = 12.;
    /// The knob's inset from the track's edge.
    pub const TOGGLE_INSET: f32 = 4.;
    /// The round check (multi-select, checkbox).
    pub const CHECK: f32 = 20.;
    /// The Sent notice: an ink pill.
    pub const TOAST_H: f32 = 44.;
    /// The seal beside the Sent notice's text.
    pub const TOAST_SEAL: f32 = 18.;
    /// The notice's distance from the bottom of the reader.
    pub const TOAST_BOTTOM: f32 = 24.;
    /// The three loader spots and their gap.
    pub const LOADER_SPOTS: [f32; 3] = [7., 4., 5.];
    pub const LOADER_GAP: f32 = 5.;
    pub const PROGRESS_RING: f32 = 20.;
    pub const DIALOG_W: f32 = 440.;
    pub const MENU_MIN_W: f32 = 200.;
    pub const TOOLTIP_MAX_W: f32 = 320.;
    /// Below this window width the nav becomes the rail.
    pub const COMPACT_BELOW_W: f32 = 1000.;
    pub const WINDOW_MIN: (f32, f32) = (640., 480.);
    pub const WINDOW_DEFAULT: (f32, f32) = (1280., 800.);
    /// Stroke widths: the one 1 px edge, rings and the focus ring.
    pub const HAIRLINE: f32 = 1.;
    pub const FOCUS_RING: f32 = 2.;
    pub const FOCUS_GAP: f32 = 2.;
}

/// Heights that grow with Windows' text size (phase 2, for every lane): a
/// row or control keeps its 100 percent size and grows by what its text
/// grew, so nothing overlaps at 150 or 225 percent. The 100 percent sizes
/// are the mockups' (app.html `.row`, `.ni`).
pub mod scaled {
    use super::{layout, space};
    use crate::theme::Tokens;

    /// The space under each message row (app.html `.row` margin).
    pub const ROW_GAP: f32 = 2.;
    /// A message row's padding above and below its lines.
    pub const ROW_PAD: f32 = space::SP_3;
    /// The space between a row's subject and its preview.
    pub const PREVIEW_GAP: f32 = layout::HAIRLINE;

    /// A message row: the sender and subject lines (body), the preview line,
    /// 12 px above and below. 83 px at 100 percent; the list's slot adds
    /// [`ROW_GAP`].
    pub fn message_row(t: &Tokens) -> f32 {
        ROW_PAD * 2. + t.text.body.line.as_f32() * 2. + PREVIEW_GAP + t.text.preview.line.as_f32()
    }

    /// The list's slot for a message row (the row and its gap).
    pub fn message_slot(t: &Tokens) -> f32 {
        message_row(t) + ROW_GAP
    }

    /// A control of `base` px (at 100 percent) holding one line of body
    /// text: it grows by what the line grew.
    pub fn control(t: &Tokens, base: f32) -> f32 {
        let grown = t.text.body.line.as_f32() - crate::theme::text::BODY.1;
        base + grown.max(0.)
    }

    /// A folder pane line (40 px at 100 percent).
    pub fn nav_item(t: &Tokens) -> f32 {
        control(t, layout::CONTROL_H_LG)
    }

    /// The indent of a tree level (folder lines, the Move flyout): 16 px a
    /// level.
    pub fn indent(level: usize) -> f32 {
        level as f32 * space::SP_4
    }
}

/// Durations and the loader's cycle (BRAND.md "Motion").
pub mod motion {
    pub const PRESS_MS: u64 = 100;
    pub const SELECT_MS: u64 = 167;
    pub const PANE_MS: u64 = 250;
    pub const LOADER_PERIOD_MS: u64 = 2400;
    pub const LOADER_OFFSETS_MS: [u64; 3] = [0, 800, 1600];
    /// The loader steps at 8 frames a second (125 ms), only while active.
    pub const LOADER_STEP_MS: u64 = 125;
    /// The loader's opacities at rest (reduce motion, or before the first
    /// step).
    pub const LOADER_REST: [f32; 3] = [0.9, 0.35, 0.6];
    pub const LOADER_MIN: f32 = 0.3;
    pub const LOADER_MAX: f32 = 1.0;
    pub const SENT_HOLD_MS: u64 = 2000;
    /// A search of a large folder waits for this pause in typing (the
    /// Slint app's `SEARCH_DEBOUNCE`).
    pub const SEARCH_PAUSE_MS: u64 = 100;
    pub const TOOLTIP_DELAY_MS: u64 = 500;
    /// Opacity of a pressed or hovered ink fill (IMPLEMENTATION.md: hover
    /// `opacity(0.9)`).
    pub const INK_HOVER: f32 = 0.9;
    /// Opacity of disabled controls.
    pub const DISABLED: f32 = 0.4;
    /// A draft being sent.
    pub const DIM: f32 = 0.55;

    /// `cubic-bezier(0, 0, 0, 1)` (enter): an exponential ease out.
    pub fn ease_enter(t: f32) -> f32 {
        if t >= 1. {
            1.
        } else {
            1. - 2f32.powf(-10. * t.max(0.))
        }
    }

    /// Exit is linear.
    pub fn ease_exit(t: f32) -> f32 {
        t.clamp(0., 1.)
    }

    /// A linear fade out: opacity from 1 to 0 as `t` goes from 0 to 1.
    pub fn fade_out(t: f32) -> f32 {
        1. - ease_exit(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_is_on_the_4_px_grid() {
        for v in [
            space::SP_1,
            space::SP_2,
            space::SP_3,
            space::SP_4,
            space::SP_5,
            space::SP_6,
            space::SP_7,
            space::SP_8,
            space::SP_10,
            layout::TITLEBAR_H,
            layout::ROW_H,
            layout::CONTROL_H,
            layout::CONTROL_H_LG,
            layout::CONTROL_H_MD,
            layout::ICON_BTN,
        ] {
            assert_eq!(v % 4., 0., "{v}");
        }
    }

    #[test]
    fn rows_and_controls_grow_with_the_text_size() {
        use crate::theme::{ThemeInput, tokens};
        use komorebi::db::ThemeChoice;
        use komorebi::theme::SystemTheme;
        let at = |scale| {
            tokens(&ThemeInput::new(
                ThemeChoice::Light,
                SystemTheme {
                    text_scale: scale,
                    ..SystemTheme::default()
                },
            ))
        };
        let normal = at(1.0);
        // The mockup's row: 12 + 20 + 20 + 1 + 18 + 12.
        assert_eq!(scaled::message_row(&normal), 83.);
        assert_eq!(scaled::message_slot(&normal), 85.);
        assert_eq!(scaled::nav_item(&normal), layout::CONTROL_H_LG);
        let large = at(2.25);
        assert!(scaled::message_row(&large) > 1.5 * scaled::message_row(&normal));
        assert!(scaled::nav_item(&large) > layout::CONTROL_H_LG);
        // Each line still fits: the row holds its three lines and padding.
        let lines = large.text.body.line.as_f32() * 2. + large.text.preview.line.as_f32();
        assert!(scaled::message_row(&large) >= lines + 2. * scaled::ROW_PAD);
    }

    #[test]
    fn easing_starts_at_zero_and_ends_at_one() {
        assert_eq!(motion::ease_enter(0.), 0.);
        assert_eq!(motion::ease_enter(1.), 1.);
        assert!(motion::ease_enter(0.25) > 0.8, "fast start");
        assert_eq!(motion::ease_exit(0.5), 0.5);
    }
}
