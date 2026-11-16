//! The brand's widgets (docs/GPUI-MIGRATION.md phase 1): pills, round
//! buttons, spots, fields, menus and dialogs on paper and ink, built from
//! the tokens only (`crate::theme`; `tests/token_lint.rs`). GPUI Kit pieces
//! are used where they fit (the text field, popup menus, dialogs, the
//! progress circle, the scrollbar) and restyled through the tokens; where
//! Kit hard-codes a look the widget is a plain GPUI div (the GPUI-MIGRATION
//! gotchas list them).
//!
//! Every widget gives AccessKit a role and a name; every focusable one
//! shows the brand's focus ring (2 px ring, 2 px gap) only for
//! keyboard focus ([`focus_ring`]).

pub mod button;
pub mod dialog;
pub mod field;
pub mod icon_button;
pub mod loader;
pub mod marks;
pub mod menu;
pub mod notice;
pub mod pane;
pub mod pill;
pub mod select;
pub mod spot;
pub mod tabs;
pub mod toast;
pub mod toggle;
pub mod tooltip;

use gpui::{BoxShadow, Pixels, StyleRefinement, Styled, px};

use crate::theme::{Tokens, layout};

/// The keyboard focus ring: a 2 px gap, then a 2 px ring, in the theme's
/// ring and gap colours (Komorebi's: ink over paper),
/// following the element's corners. Two crisp spreads (no blur), because
/// the ring must sit
/// outside the element without moving it, and `focus_visible` styles are
/// the only way to draw on keyboard focus alone. GPUI paints the shadows in
/// order, the last on top (CSS puts the first on top), so the 4 px ink comes
/// first and the 2 px paper gap over it; a crisp shadow paints outside the
/// element only (KOMOREBI PATCH hollow-crisp-shadow in gpui-pre).
pub fn focus_ring(t: &Tokens) -> Vec<BoxShadow> {
    let zero = Pixels::ZERO;
    vec![
        BoxShadow::new(zero, zero, t.color.ring)
            .spread_radius(px(layout::FOCUS_GAP + layout::FOCUS_RING)),
        BoxShadow::new(zero, zero, t.color.ring_gap).spread_radius(px(layout::FOCUS_GAP)),
    ]
}

/// The width `text` takes on one line in a step of the type ramp: for
/// deciding whether a label is cut short (it then gets a tooltip with the
/// whole of it).
pub fn text_width(text: &str, token: &crate::theme::TextToken, window: &mut gpui::Window) -> f32 {
    let mut style = window.text_style();
    style.font_family = token.family.clone();
    style.font_weight = token.weight;
    let run = gpui::TextRun {
        len: text.len(),
        font: style.font(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        gpui::SharedString::from(text.to_string()),
        token.size,
        &[run],
        None,
    );
    f32::from(line.width)
}

/// `focus_visible` style with the ring.
pub fn ring_style(t: &Tokens) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let ring = focus_ring(t);
    move |s| s.shadow(ring)
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared set-up for the widget tests: Kit, the tokens, and a headless
    //! window holding one view.

    use gpui::{
        AppContext as _, Bounds, Entity, Render, TestAppContext, Window, WindowBounds,
        WindowHandle, WindowOptions, point, px, size,
    };
    use gpui_kit::component::Root;
    use komorebi::db::ThemeChoice;
    use komorebi::theme::SystemTheme;

    use crate::theme::{ThemeInput, install};

    pub fn init(cx: &mut TestAppContext, dark: bool) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let choice = if dark {
                ThemeChoice::Dark
            } else {
                ThemeChoice::Light
            };
            install(ThemeInput::new(choice, SystemTheme::default()), cx);
            // Kit's springs snap under reduce motion; test time does not
            // move them.
            cx.set_reduce_motion(true);
        });
    }

    /// Opens a 800 x 600 headless window with the view `build` makes.
    pub fn open<V: Render + 'static>(
        cx: &mut TestAppContext,
        build: impl FnOnce(&mut Window, &mut gpui::Context<V>) -> V + 'static,
    ) -> (WindowHandle<Root>, Entity<V>) {
        init(cx, false);
        let (window, view) = cx
            .update(|cx| {
                let bounds = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.)));
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    cx,
                    move |window, cx| cx.new(|cx| build(window, cx)),
                )
            })
            .expect("open test window");
        let window = window.downcast::<Root>().expect("Base Root");
        (window, view)
    }
}
