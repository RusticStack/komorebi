//! The tab keys of the main window (docs/WINUI3.md "TabView"; the same keys
//! as the Slint app): actions bound once at start-up in the window's key
//! context (Skua cookbook_actions), so they win over a focused text field.
//! The tabs themselves are `komorebi::tabs::Tabs`, shared with the Slint
//! app, drawn and driven by `views::tabs` and `window.rs` (phase 5).

use gpui::{App, KeyBinding};

gpui::actions!(
    komorebi,
    [
        NextTab,
        PreviousTab,
        FirstTab,
        LastTab,
        CloseTab,
        ReopenTab,
        MoveTabLeft,
        MoveTabRight,
        OpenInTab,
        OpenInWindow,
        ToggleSettings,
        ShowShortcuts,
        NextPane,
        PreviousPane,
    ]
);

/// Ctrl+2 to Ctrl+8: the tab of that number, when there is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = komorebi, no_json)]
pub struct GoToTab(pub usize);

/// The key context on the main window's root: its bindings match wherever
/// the focus is inside the window.
pub const CONTEXT: &str = "MainWindow";

/// The one binding table, installed once in `main` (and by the tests).
pub fn bind_keys(cx: &mut App) {
    let main = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", NextTab, main),
        KeyBinding::new("ctrl-pagedown", NextTab, main),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, main),
        KeyBinding::new("ctrl-pageup", PreviousTab, main),
        KeyBinding::new("ctrl-1", FirstTab, main),
        KeyBinding::new("ctrl-9", LastTab, main),
        KeyBinding::new("ctrl-w", CloseTab, main),
        KeyBinding::new("ctrl-f4", CloseTab, main),
        KeyBinding::new("ctrl-shift-t", ReopenTab, main),
        KeyBinding::new("ctrl-shift-pageup", MoveTabLeft, main),
        KeyBinding::new("ctrl-shift-pagedown", MoveTabRight, main),
        KeyBinding::new("ctrl-,", ToggleSettings, main),
        // The Keyboard shortcuts dialog from anywhere (Ctrl+/ not in the
        // composer, as the Slint app), and F6 round the panes.
        KeyBinding::new("f1", ShowShortcuts, main),
        KeyBinding::new("ctrl-/", ShowShortcuts, main),
        KeyBinding::new(
            "ctrl-/",
            gpui::NoAction,
            Some(crate::views::composer::CONTEXT),
        ),
        KeyBinding::new("f6", NextPane, main),
        KeyBinding::new("shift-f6", PreviousPane, main),
    ]);
    for n in 2..=8 {
        cx.bind_keys([KeyBinding::new(&format!("ctrl-{n}"), GoToTab(n), main)]);
    }
    // On the list and the reading pane (never in the composer).
    let messages = Some(crate::views::mail::commands::MESSAGES);
    cx.bind_keys([
        KeyBinding::new("shift-enter", OpenInTab, messages),
        KeyBinding::new("ctrl-o", OpenInWindow, messages),
    ]);
    // A message window: Esc closes it (Ctrl+W too).
    let popout = Some(crate::views::tabs::message::WINDOW_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", crate::views::tabs::message::CloseMessage, popout),
        KeyBinding::new("ctrl-w", crate::views::tabs::message::CloseMessage, popout),
    ]);
}
