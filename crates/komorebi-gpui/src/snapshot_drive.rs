//! What a snapshot does to the window between its first frame and the
//! capture (`KOMOREBI_SNAPSHOT_FOCUS`, `_TYPE`, `_WALK`, `_SCROLL`, `_HOVER`;
//! docs/GPUI-TESTING.md "Pixel snapshots"). Everything is input GPUI would
//! get from the platform: keystrokes and mouse events dispatched to the
//! window (`Window::dispatch_keystroke`, `dispatch_event`), so a key reaches
//! the same handlers a person's key does, and the focus ring and hover
//! styles are the real ones. The order and the waits are the 0.7 app's:
//! the message gets 600 ms to open, the focus moves, the walk opens the next
//! messages 250 ms apart, the reading pane scrolls, the pointer points, and
//! the capture follows 700 ms later (a tooltip's 500 ms delay) when it did.

use gpui::{
    AnyWindowHandle, AsyncApp, FocusHandle, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PlatformInput, Point, ScrollDelta, ScrollWheelEvent, TouchPhase,
    Window, point, px,
};
use std::time::Duration;

use crate::snapshot::{Drive, Hover, Pointer};
use crate::theme::layout;
use crate::titlebar::Caption;

/// The message gets this long to open and its body to arrive.
const OPEN_WAIT: Duration = Duration::from_millis(600); // token-exempt: a wait of a development tool, not a size
/// One message of a walk renders before the next replaces it.
const WALK_STEP: Duration = Duration::from_millis(250); // token-exempt: a wait of a development tool, not a size
/// The reading pane lays out before it is scrolled.
const SCROLL_WAIT: Duration = Duration::from_millis(400); // token-exempt: a wait of a development tool, not a size
/// After a hover: a tooltip's delay (500 ms) and a frame.
const HOVER_WAIT: Duration = Duration::from_millis(700); // token-exempt: a wait of a development tool, not a size
/// After the other steps: the last frame.
const SETTLE: Duration = Duration::from_millis(250); // token-exempt: a wait of a development tool, not a size

/// The steps of `drive` on the window, then the wait that lets the last
/// frame reach the screen. Errors are the window going away.
pub async fn run(
    handle: AnyWindowHandle,
    list: Option<FocusHandle>,
    drive: &Drive,
    cx: &mut AsyncApp,
) {
    wait(cx, OPEN_WAIT).await;
    if let Some(focus) = &drive.focus {
        // A key press first: the focus ring shows for the keyboard only.
        let mut keys = vec!["shift"];
        keys.extend(focus.keystrokes());
        let list = list.clone();
        step(handle, cx, move |window, cx| {
            if let Some(list) = &list {
                window.focus(list, cx);
            }
            for key in &keys {
                press(window, key, cx);
            }
        });
        wait(cx, SETTLE).await;
    }
    if !drive.typed.is_empty() {
        // Where the focus is now: the list is not focused first.
        let keys = drive.typed.clone();
        step(handle, cx, move |window, cx| {
            for key in &keys {
                press(window, key, cx);
            }
        });
        wait(cx, SETTLE).await;
    }
    for _ in 0..drive.walk {
        let list = list.clone();
        step(handle, cx, move |window, cx| {
            if let Some(list) = &list {
                window.focus(list, cx);
            }
            press(window, "down", cx);
        });
        wait(cx, WALK_STEP).await;
    }
    if let Some(y) = drive.scroll {
        wait(cx, SCROLL_WAIT).await;
        step(handle, cx, move |window, cx| scroll(window, y, cx));
        wait(cx, SETTLE).await;
    }
    if let Some(hover) = drive.hover.clone() {
        step(handle, cx, move |window, cx| point_at(window, &hover, cx));
        wait(cx, HOVER_WAIT).await;
    }
}

/// Runs `f` in the window, on the UI thread.
fn step(
    handle: AnyWindowHandle,
    cx: &mut AsyncApp,
    f: impl FnOnce(&mut Window, &mut gpui::App) + 'static,
) {
    let _ = handle.update(cx, |_, window, cx| f(window, cx));
}

async fn wait(cx: &mut AsyncApp, d: Duration) {
    cx.background_executor().timer(d).await; // timer-exempt: a snapshot waits for the app's own timers (tooltips, the body of a message) as the 0.7 app's did
}

/// A key as the keyboard sends it.
fn press(window: &mut Window, key: &str, cx: &mut gpui::App) {
    match Keystroke::parse(key) {
        Ok(keystroke) => {
            window.dispatch_keystroke(keystroke, cx);
        }
        Err(e) => eprintln!("snapshot: key {key}: {e}"),
    }
}

/// Where a scroll of the reading pane points: its middle (logical px) in a
/// window `width` by `height`, the layout of `views::mail` (the folder pane
/// or rail, then the list, then the pane). The whole window below the
/// one-pane width, where the open message is all there is.
pub fn reading_pane_centre(width: f32, height: f32) -> (f32, f32) {
    if width < crate::views::mail::ONE_PANE_BELOW_W {
        return (width / 2., height / 2.);
    }
    let (nav, list) = if width < layout::COMPACT_BELOW_W {
        (layout::RAIL_W, layout::LIST_W_COMPACT)
    } else {
        (layout::NAV_W, layout::LIST_W)
    };
    let left = nav + list + 2. * crate::theme::space::PANE_GAP;
    ((left + width) / 2., height / 2.)
}

/// Scrolls the reading pane down by `y` logical px with the wheel (a
/// negative delta is down in GPUI).
fn scroll(window: &mut Window, y: f32, cx: &mut gpui::App) {
    let size = window.viewport_size();
    let (x0, y0) = reading_pane_centre(size.width.as_f32(), size.height.as_f32());
    let position = point(px(x0), px(y0));
    // The pointer comes first, as a wheel is turned over something.
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Modifiers::default(),
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(-y))), // token-exempt: the requested scroll, not a size
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        }),
        cx,
    );
}

/// The middle of a caption button (logical px) in a window `width` wide:
/// the buttons are 46 px wide in the 48 px bar, Close at the right edge.
pub fn caption_centre(which: Caption, width: f32) -> (f32, f32) {
    let from_right = match which {
        Caption::Close => 0,
        Caption::Maximize => 1,
        Caption::Minimize => 2,
    };
    (
        width - layout::CAPTION_BTN_W * (from_right as f32 + 0.5),
        layout::TITLEBAR_H / 2.,
    )
}

/// The pointer at the hover's place, pressed or clicked there when asked.
fn point_at(window: &mut Window, hover: &Hover, cx: &mut gpui::App) {
    let (position, how): (Point<gpui::Pixels>, Pointer) = match hover {
        Hover::At { x, y, how } => (point(px(*x), px(*y)), *how),
        Hover::Caption { which, press } => {
            let (x, y) = caption_centre(*which, window.viewport_size().width.as_f32());
            (
                point(px(x), px(y)),
                if *press {
                    Pointer::Press
                } else {
                    Pointer::Move
                },
            )
        }
    };
    let modifiers = Modifiers::default();
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        cx,
    );
    if how != Pointer::Move {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
    }
    if how == Pointer::Click {
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Focus, Key};
    use crate::test_support;
    use gpui::{AppContext as _, TestAppContext};

    /// The reading pane's middle is right of the list at every layout, and
    /// the caption buttons sit at the right edge of the bar.
    #[test]
    fn the_points_are_in_the_right_panes() {
        // Wide: folder pane, list 384, two 8 px gaps.
        let (x, y) = reading_pane_centre(1280., 800.);
        assert_eq!((x, y), ((layout::NAV_W + 384. + 16. + 1280.) / 2., 400.));
        // Compact: the rail and the narrower list.
        let (x, _) = reading_pane_centre(900., 700.);
        assert!(x > layout::RAIL_W + layout::LIST_W_COMPACT && x < 900.);
        // One pane: the window's middle.
        assert_eq!(reading_pane_centre(600., 500.), (300., 250.));
        assert_eq!(caption_centre(Caption::Close, 1280.), (1257., 24.));
        assert_eq!(caption_centre(Caption::Maximize, 1280.), (1211., 24.));
        assert_eq!(caption_centre(Caption::Minimize, 1280.), (1165., 24.));
    }

    /// The keys a focus switch names reach the panes as a person's keys do:
    /// `nav` (Shift+F6) takes the keys to the folders from the list, `tab:1`
    /// does not leave them there, and the walk's Down moves the list's row.
    #[gpui_kit::test]
    fn the_focus_keys_move_the_focus(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("drive-focus", 5);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        let mail = cx.update_window(window, |_, _, cx| {
            view.read(cx).mail().cloned().expect("the mail view")
        });
        let mail = mail.expect("update");
        let focused = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                let m = mail.read(cx);
                (
                    m.nav_focus().contains_focused(window, cx),
                    m.list_focus().contains_focused(window, cx),
                )
            })
            .expect("update")
        };
        let go = |cx: &mut TestAppContext, focus: &Focus| {
            let list = mail.read_with(cx, |m, _| m.list_focus().clone());
            test_support::in_window(cx, window, |window, cx| {
                window.focus(&list, cx);
                for key in focus.keystrokes() {
                    press(window, key, cx);
                }
            });
            cx.run_until_parked();
        };
        go(cx, &Focus::List);
        assert_eq!(focused(cx), (false, true), "the list");
        go(
            cx,
            &Focus::Nav {
                downs: 1,
                keys: Vec::new(),
            },
        );
        assert_eq!(focused(cx), (true, false), "the folders");
        // `press:` keys are the same keys, where the focus is.
        go(cx, &Focus::Press(vec![Key::Down]));
        assert_eq!(focused(cx), (false, true), "back in the list");
    }

    /// `KOMOREBI_SNAPSHOT_ACTION=compose` opens a new message in the Mail
    /// tab's reading pane, as Ctrl+N does.
    #[gpui_kit::test]
    fn the_compose_action_opens_the_composer(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("drive-compose", 3);
        test_support::init_app(cx);
        let get = |k: &str| match k {
            "KOMOREBI_SNAPSHOT" => Some("out.png".to_owned()),
            "KOMOREBI_OFFLINE" => Some("1".to_owned()),
            "KOMOREBI_DB" => Some("scratch.sqlite3".to_owned()),
            "KOMOREBI_SNAPSHOT_ACTION" => Some("compose".to_owned()),
            _ => None,
        };
        let request = crate::snapshot::parse(get, true, |_| false)
            .expect("a safe request")
            .expect("a snapshot");
        let mut init = test_support::init(Some(db));
        init.snapshot = Some(request);
        let (window, view) = test_support::open_main(cx, init);
        cx.run_until_parked();
        test_support::in_window(cx, window, |_, cx| {
            let mail = view.read(cx).mail().cloned().expect("the mail view");
            assert!(mail.read(cx).composer().read(cx).is_open());
        });
    }

    /// The pointer is moved to where the caption button and the reading pane
    /// are (`mouse_position` follows) and a wheel is dispatched without
    /// failing. It does not assert a hover style or that the pane scrolled:
    /// caption buttons are platform hit-tested regions, so the hover and
    /// the caption `:press` are checked only in a real snapshot (not run
    /// yet, docs/GPUI-MIGRATION.md "Gaps: other").
    #[gpui_kit::test]
    fn a_hover_and_a_wheel_are_dispatched(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("drive-hover", 3);
        test_support::init_app(cx);
        let (window, _view) = test_support::open_main(cx, test_support::init(Some(db)));
        test_support::in_window(cx, window, |window, cx| {
            point_at(
                window,
                &Hover::Caption {
                    which: Caption::Close,
                    press: false,
                },
                cx,
            );
            let at = caption_centre(Caption::Close, window.viewport_size().width.as_f32());
            assert_eq!(window.mouse_position(), point(px(at.0), px(at.1)));
            scroll(window, 300., cx);
            let (x, y) = reading_pane_centre(
                window.viewport_size().width.as_f32(),
                window.viewport_size().height.as_f32(),
            );
            assert_eq!(window.mouse_position(), point(px(x), px(y)));
        });
    }
}
