//! A window of its own for one message or one draft (docs/GPUI-MIGRATION.md
//! phase 5; the Slint app's `src/app/windows.rs`): the app-drawn title bar
//! (seal, name, caption buttons) over the message view or the composer, on
//! canvas with the paper pane 8 px in, as the main window.
//!
//! The main window owns what shows here (`window.rs`): it opens the window,
//! listens to its view, and closes it (a message archived from the main
//! window, the main window closing). A language or theme switch redraws
//! every window by itself (`crate::i18n`, `crate::theme`).

use gpui::{
    AnyView, App, Bounds, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, div,
    prelude::*, px,
};

use crate::theme::{ActiveTokens as _, space};
use crate::titlebar::TitleBar;
use crate::window::MIN_SIZE;

/// What a secondary window holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoutKind {
    /// Message `id` (a `messages` row).
    Message(i64),
    /// A draft, numbered by the main window for this session.
    Draft(u64),
}

/// The size a message or composer window first opens at (logical px).
pub const POPOUT_SIZE: (f32, f32) = (880., 720.);

/// Each window of its own opens this far down and right of the one before
/// (logical px), and the cascade starts over after [`CASCADE_STEPS`].
pub const CASCADE: f32 = 28.;
pub const CASCADE_STEPS: usize = 8;

/// The offset of the `n`th window of its own from the centre (the cascade).
pub fn cascade(n: usize) -> f32 {
    (n % CASCADE_STEPS) as f32 * CASCADE
}

/// A window's size to remember as it closes: its windowed size, also while
/// maximized (logical px).
pub fn size_of(window: &Window) -> (f32, f32) {
    let s = window.window_bounds().get_bounds().size;
    (
        s.width.as_f32().max(MIN_SIZE.0),
        s.height.as_f32().max(MIN_SIZE.1),
    )
}

/// Close a window of its own from the app (Esc, Close, a move, Send or
/// Discard), keeping its size for the next one, as its own close button
/// does.
pub fn close_keeping_size(window: &mut Window, size: &std::cell::Cell<Option<(f32, f32)>>) {
    size.set(Some(size_of(window)));
    window.remove_window();
}

/// The window options of a secondary window at the first size, centred.
pub fn options(title: &str, cx: &App) -> WindowOptions {
    options_at(title, POPOUT_SIZE, 0, cx)
}

/// The window options of the `n`th secondary window, `size` big: centred on
/// the display, then [`cascade`]d down and right so it does not hide the
/// one before. The app-drawn title bar where the main window has one
/// (`titlebar::app_drawn`), else the system caption; the OS title
/// (taskbar, Alt+Tab, UI Automation), opaque paper. The Slint app's message
/// windows always use the system caption (`apply_popout_theme`); the
/// app-drawn bar on Windows 11 is the GPUI app's own choice (phase 5), so
/// this is not Slint parity.
pub fn options_at(title: &str, size: (f32, f32), n: usize, cx: &App) -> WindowOptions {
    let mut bounds = Bounds::centered(None, gpui::size(px(size.0), px(size.1)), cx);
    let step = px(cascade(n));
    bounds.origin.x += step;
    bounds.origin.y += step;
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            appears_transparent: crate::titlebar::app_drawn(),
            traffic_light_position: None,
        }),
        window_min_size: Some(gpui::size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        window_background: WindowBackgroundAppearance::Opaque,
        ..Default::default()
    }
}

pub struct Popout {
    kind: PopoutKind,
    body: AnyView,
    focus: FocusHandle,
}

impl Popout {
    /// The window around `body` (a `MessageView` or a `ComposerView`, made
    /// by the main window), which takes the keyboard focus.
    pub fn new(
        kind: PopoutKind,
        body: AnyView,
        body_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.focus(&body_focus, cx);
        Self {
            kind,
            body,
            focus: body_focus,
        }
    }

    pub fn kind(&self) -> PopoutKind {
        self.kind
    }
}

impl Focusable for Popout {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Popout {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = cx.tokens().color;
        div()
            .id("popout")
            .size_full()
            .flex()
            .flex_col()
            .bg(c.canvas)
            .text_color(c.ink)
            .child(TitleBar::new())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(space::PANE_GAP))
                    .pb(px(space::PANE_GAP))
                    .child(self.body.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows of their own cascade down and right, then start over.
    #[test]
    fn windows_of_their_own_cascade() {
        assert_eq!(cascade(0), 0.);
        assert_eq!(cascade(1), CASCADE);
        assert_eq!(cascade(3), 3. * CASCADE);
        assert_eq!(cascade(CASCADE_STEPS), 0.);
    }
}
