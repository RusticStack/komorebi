//! The app-drawn title bar (Skua's titlebar.rs, cookbook_window): 48 px on
//! canvas, the ink mark and the app name at the left, an optional centre
//! slot (the search pill, phase 2), and the three 46 px caption buttons.
//!
//! The caption buttons are `window_control_area` regions, so Windows
//! hit-tests them as its own (Aero Snap, the Snap Layouts flyout on
//! Maximize). A screen reader's Invoke never reaches that hit test, so each
//! button also answers AccessKit's Click action by posting the window the
//! same `WM_SYSCOMMAND` the system caption sends (Minimize, Maximize or
//! Restore, Close). Inactive windows dim the title and glyphs to ink-3;
//! Close's hover is Windows' caption red.

use gpui::{
    AccessibleAction, AnyElement, App, Hsla, IntoElement, ParentElement, RenderOnce, Role,
    SharedString, Styled, Window, WindowControlArea, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use komorebi_i18n::tr;

use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, space};

/// A caption button's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caption {
    Minimize,
    /// Maximize, or Restore while maximized.
    Maximize,
    Close,
}

/// The caption buttons' accessible names, in the language shown (the
/// Slint title bar's messages in `ui/app.slint`).
pub fn caption_names(maximized: bool) -> [String; 3] {
    let max = if maximized {
        tr!("Restore")
    } else {
        tr!("Maximize")
    };
    [tr!("Minimize"), max, tr!("Close")]
}

/// The `WM_SYSCOMMAND` code a caption button sends.
pub fn system_command(caption: Caption, maximized: bool) -> u32 {
    // winuser.h SC_MINIMIZE, SC_MAXIMIZE, SC_RESTORE, SC_CLOSE.
    match (caption, maximized) {
        (Caption::Minimize, _) => 0xF020,
        (Caption::Maximize, false) => 0xF030,
        (Caption::Maximize, true) => 0xF120,
        (Caption::Close, _) => 0xF060,
    }
}

/// Posts a system command to the window, as its system caption would.
#[cfg(windows)]
fn post_system_command(window: &Window, command: u32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_SYSCOMMAND};
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return;
    };
    // SAFETY: GPUI's live window; posting returns at once, and the window
    // handles the command on its own thread like a caption click.
    let posted = unsafe {
        PostMessageW(
            Some(HWND(h.hwnd.get() as _)),
            WM_SYSCOMMAND,
            WPARAM(command as usize),
            LPARAM(0),
        )
    };
    if let Err(e) = posted {
        eprintln!("komorebi-gpui: caption command: {e}");
    }
}

#[cfg(not(windows))]
fn post_system_command(_: &Window, _: u32) {}

/// Whether the windows draw their own title bar (the Slint app's
/// `chrome::custom_wanted`): on Windows 11, unless a contrast theme is on
/// or `KOMOREBI_SYSTEM_TITLEBAR` asks for the system caption; a snapshot
/// draws its own bar unless `KOMOREBI_SNAPSHOT_TITLEBAR` says `system`.
/// On Windows 10 the system caption is used: its frame and buttons are the
/// ones that work there. Read once.
pub fn app_drawn() -> bool {
    static DRAWN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // The tests draw the app's bar whatever Windows runs them.
    if cfg!(test) {
        return true;
    }
    *DRAWN.get_or_init(|| {
        let forced =
            komorebi::dev_var("KOMOREBI_SNAPSHOT_TITLEBAR").and_then(|v| v.into_string().ok());
        komorebi::chrome::custom_wanted(
            komorebi::theme::windows_build(),
            komorebi::theme::read().contrast.is_some(),
            komorebi::dev_var("KOMOREBI_SYSTEM_TITLEBAR").is_some(),
            komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some(),
            forced.as_deref(),
        )
    })
}

/// The title bar of a Komorebi window.
#[derive(IntoElement)]
pub struct TitleBar {
    title: SharedString,
    center: Option<AnyElement>,
    always_active: bool,
    /// Under the system caption (Windows 10): only the centre slot, no
    /// mark, title, drag area or caption buttons.
    system: bool,
}

impl Default for TitleBar {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleBar {
    /// The caption names follow the language shown (`tr!`).
    pub fn new() -> Self {
        Self {
            title: "Komorebi".into(),
            center: None,
            always_active: false,
            system: !app_drawn(),
        }
    }

    /// Under the system caption (`true`) or app-drawn (the default follows
    /// [`app_drawn`]).
    pub fn system_caption(mut self, system: bool) -> Self {
        self.system = system;
        self
    }

    /// The text beside the mark (secondary windows name themselves, through
    /// `tr!`).
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }

    /// Draws as the active window whatever has the focus (snapshots).
    pub fn always_active(mut self, always: bool) -> Self {
        self.always_active = always;
        self
    }

    /// The centre slot (the search pill).
    pub fn center(mut self, el: impl IntoElement) -> Self {
        self.center = Some(el.into_any_element());
        self
    }
}

fn caption_button(
    caption: Caption,
    name: String,
    maximized: bool,
    glyph: Hsla,
    hover: (Hsla, Hsla),
) -> impl IntoElement {
    let (id, icon, area) = match caption {
        Caption::Minimize => ("caption-min", Icon::Minimize, WindowControlArea::Min),
        Caption::Maximize if maximized => ("caption-max", Icon::Restore, WindowControlArea::Max),
        Caption::Maximize => ("caption-max", Icon::Maximize, WindowControlArea::Max),
        Caption::Close => ("caption-close", Icon::Dismiss, WindowControlArea::Close),
    };
    let command = system_command(caption, maximized);
    // An svg paints in its own text colour, so the glyph follows the
    // button's hover through a group.
    div()
        .id(id)
        .group(id)
        .role(Role::Button)
        .aria_label(name)
        .on_a11y_action(AccessibleAction::Click, move |_, window, _| {
            post_system_command(window, command);
        })
        .w(px(layout::CAPTION_BTN_W))
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .hover(move |s| s.bg(hover.0))
        .window_control_area(area)
        .child(
            svg()
                .path(icon.path())
                .size(px(layout::CAPTION_GLYPH))
                .text_color(glyph)
                .group_hover(id, move |s| s.text_color(hover.1)),
        )
        .test_support()
}

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let maximized = window.is_maximized();
        let [min, max, close] = caption_names(maximized);
        let ink = if window.is_window_active() || self.always_active {
            c.ink
        } else {
            c.ink_3
        };
        let normal = (c.wash, ink);
        let red = (c.close_hover, c.on_close);
        if self.system {
            // The system caption above has the title and the buttons; with
            // nothing in the centre (a message window, no account yet) no
            // strip of our own is drawn under it.
            if self.center.is_none() {
                return div().id("titlebar").test_support().into_any_element();
            }
            return div()
                .id("titlebar")
                .test_support()
                .h(px(layout::TITLEBAR_H))
                .w_full()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .bg(c.canvas)
                .text_color(ink)
                .children(self.center)
                .into_any_element();
        }
        div()
            .id("titlebar")
            .relative()
            .h(px(layout::TITLEBAR_H))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .bg(c.canvas)
            .text_color(ink)
            .child(
                // Everything not covered by a button drags the window.
                div()
                    .id("titlebar-drag")
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(space::SP_3))
                    .pl(px(space::SP_4))
                    .typo(&t.text.caption)
                    .window_control_area(WindowControlArea::Drag)
                    // The ink mark: the window keeps its colour for the seal
                    // moments (BRAND.md persimmon rule).
                    .child(
                        svg()
                            .path(Icon::Seal.path())
                            .size(px(layout::TITLE_MARK))
                            .flex_none()
                            .text_color(ink),
                    )
                    .child(self.title),
            )
            .when_some(self.center, |bar, center| {
                bar.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(center),
                )
            })
            .child(caption_button(
                Caption::Minimize,
                min,
                maximized,
                ink,
                normal,
            ))
            .child(caption_button(
                Caption::Maximize,
                max,
                maximized,
                ink,
                normal,
            ))
            .child(caption_button(Caption::Close, close, maximized, ink, red))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{Context, Render, TestAppContext};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    #[test]
    fn caption_names_follow_the_language_and_the_state() {
        use komorebi::i18n::Lang;
        use komorebi_i18n::with_lang;
        assert_eq!(
            with_lang(Lang::En, || caption_names(true)),
            ["Minimize", "Restore", "Close"]
        );
        assert_eq!(
            with_lang(Lang::PtPt, || caption_names(false)),
            ["Minimizar", "Maximizar", "Fechar"]
        );
        assert_eq!(
            with_lang(Lang::PtPt, || caption_names(true))[1],
            "Restaurar"
        );
    }

    #[test]
    fn caption_buttons_send_the_system_caption_commands() {
        assert_eq!(system_command(Caption::Minimize, false), 0xF020);
        assert_eq!(system_command(Caption::Maximize, false), 0xF030);
        assert_eq!(system_command(Caption::Maximize, true), 0xF120);
        assert_eq!(system_command(Caption::Close, true), 0xF060);
    }

    struct Demo;
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(TitleBar::new())
        }
    }

    /// The caption buttons are named buttons, in the language shown. (Their
    /// Click action, UIA Invoke, has no headless test hook in this GPUI; the
    /// live UIA check is in docs/GPUI-MIGRATION.md.)
    #[gpui_kit::test]
    fn caption_buttons_are_named_buttons(cx: &mut TestAppContext) {
        cx.update(|cx| crate::i18n::init(komorebi::i18n::Lang::PtPt, cx));
        let (window, _) = test_support::open(cx, |_, _| Demo);
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            for name in ["Minimizar", "Maximizar", "Fechar"] {
                assert_eq!(
                    window.find_by_label(name).role(),
                    Some(Role::Button),
                    "{name}"
                );
            }
        })
        .expect("draw");
    }

    /// A bar under the system caption, with a button in its centre (the
    /// main window's search) or with nothing (a message window).
    struct SystemDemo {
        center: bool,
    }
    impl Render for SystemDemo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let bar = TitleBar::new().system_caption(true);
            let bar = if self.center {
                bar.center(
                    div()
                        .id("bar-search")
                        .role(Role::Button)
                        .aria_label("Search")
                        .test_support()
                        .w(px(200.))
                        .h(px(24.)),
                )
            } else {
                bar
            };
            div().size_full().child(bar)
        }
    }

    /// Under the system caption (Windows 10, `KOMOREBI_SYSTEM_TITLEBAR`) the
    /// bar draws no caption buttons of its own, keeps its centre content,
    /// and with no centre content draws no strip at all. (`app_drawn` is
    /// true under test, so the Windows 10 detection itself is not covered
    /// here: `komorebi::chrome::custom_wanted` has its own tests.)
    #[gpui_kit::test]
    fn the_system_caption_bar_has_no_caption_buttons(cx: &mut TestAppContext) {
        let (window, _) = test_support::open(cx, |_, _| SystemDemo { center: true });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            let buttons: Vec<_> = window
                .elements()
                .into_iter()
                .filter(|e| e.role() == Some(Role::Button))
                .filter_map(|e| e.label().map(str::to_owned))
                .collect();
            assert_eq!(buttons, ["Search"], "only the centre's button");
            assert_eq!(
                window.find("titlebar").bounds().size.height,
                px(layout::TITLEBAR_H)
            );
        })
        .expect("draw");
        let (window, _) = test_support::open(cx, |_, _| SystemDemo { center: false });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window.find("titlebar").bounds().size.height,
                px(0.),
                "no empty strip under the system caption"
            );
        })
        .expect("draw");
        assert!(app_drawn(), "the tests draw the app's bar");
    }
}
