//! The mail view's keys (the Slint app's keyboard map, README "Keyboard" and
//! `ui/shortcuts.slint`): actions bound once in three key contexts
//! (cookbook_actions).
//!
//! - The window's (`crate::tabs::CONTEXT`): New mail, Search, Go to the
//!   Inbox, Refresh, Show or hide the folders. They work from anywhere in
//!   the window, a text field included.
//! - [`MESSAGES`], on the list and the reading pane (never the composer):
//!   the commands on the open message or the selection (Ctrl+Q, Ctrl+U,
//!   Insert, Delete, Backspace, Ctrl+Shift+V, Ctrl+Z, Ctrl+R).
//! - [`LIST`] and [`NAV`]: moving through the rows and the folders.

use gpui::{App, Global, KeyBinding};

use crate::tabs;

gpui::actions!(
    komorebi_mail,
    [
        NewMail,
        FocusSearch,
        GoInbox,
        Refresh,
        ToggleFolders,
        NextRow,
        PreviousRow,
        FirstRow,
        LastRow,
        PageDownRows,
        PageUpRows,
        OpenRow,
        ExtendDown,
        ExtendUp,
        SelectAllRows,
        EscapeList,
        MarkRead,
        MarkUnread,
        ToggleImportant,
        Archive,
        Delete,
        MoveTo,
        SaveEml,
        Undo,
        RowMenu,
        Reply,
        ReplyAll,
        Forward,
        NavDown,
        NavUp,
        NavFirst,
        NavLast,
        NavOpen,
        NavExpand,
        NavCollapse,
        NavMenu,
        ClosePane,
        FlyoutNext,
        FlyoutPrevious,
        FlyoutFirst,
        FlyoutLast,
        FlyoutRun,
        FlyoutClose,
        NextPane,
        PreviousPane,
    ]
);

/// A keyboard menu's keys (`menus::Flyout` without a field).
pub const FLYOUT: &str = "MailFlyout";

/// The list's and the reading pane's context: the message commands.
pub const MESSAGES: &str = "MailMessages";
/// The message list's own keys.
pub const LIST: &str = "MailList";
/// The folder pane's keys.
pub const NAV: &str = "MailNav";

/// The bindings are installed once per app.
struct Bound;
impl Global for Bound {}

/// The mail view's binding table, installed once (the first window).
pub fn bind_keys(cx: &mut App) {
    if cx.has_global::<Bound>() {
        return;
    }
    cx.set_global(Bound);
    let window = Some(tabs::CONTEXT);
    let messages = Some(MESSAGES);
    let list = Some(LIST);
    let nav = Some(NAV);
    cx.bind_keys([
        KeyBinding::new("ctrl-n", NewMail, window),
        KeyBinding::new("ctrl-e", FocusSearch, window),
        KeyBinding::new("ctrl-f", FocusSearch, window),
        KeyBinding::new("f3", FocusSearch, window),
        // Kit binds Ctrl+Shift+I to its inspector with no context, which
        // GPUI ranks as the deepest: Go to the Inbox takes it the same way,
        // and wins as the later binding (the window handles it).
        KeyBinding::new("ctrl-shift-i", GoInbox, None),
        KeyBinding::new("f5", Refresh, window),
        KeyBinding::new("ctrl-shift-e", ToggleFolders, window),
        // F6 and Shift+F6: round the panes (search, folders, list, reader).
        KeyBinding::new("f6", NextPane, window),
        KeyBinding::new("shift-f6", PreviousPane, window),
        KeyBinding::new("ctrl-q", MarkRead, messages),
        KeyBinding::new("ctrl-u", MarkUnread, messages),
        KeyBinding::new("insert", ToggleImportant, messages),
        KeyBinding::new("ctrl-shift-g", ToggleImportant, messages),
        KeyBinding::new("delete", Delete, messages),
        KeyBinding::new("ctrl-shift-v", MoveTo, messages),
        KeyBinding::new("ctrl-s", SaveEml, messages),
        KeyBinding::new("ctrl-z", Undo, messages),
        KeyBinding::new("ctrl-r", Reply, messages),
        KeyBinding::new("ctrl-shift-r", ReplyAll, messages),
        KeyBinding::new("ctrl-shift-f", Forward, messages),
        KeyBinding::new("down", NextRow, list),
        KeyBinding::new("up", PreviousRow, list),
        KeyBinding::new("home", FirstRow, list),
        KeyBinding::new("end", LastRow, list),
        KeyBinding::new("pagedown", PageDownRows, list),
        KeyBinding::new("pageup", PageUpRows, list),
        KeyBinding::new("enter", OpenRow, list),
        KeyBinding::new("shift-down", ExtendDown, list),
        KeyBinding::new("shift-up", ExtendUp, list),
        KeyBinding::new("ctrl-a", SelectAllRows, list),
        // The arrows from the reading pane (a click in the message, the
        // command bar) go through the list, as in the Slint app, whose
        // page leaves only them to the window. Home, End, Page Up and Page
        // Down stay the page's (they scroll the message there), so they
        // are not bound here. The reader keeps its own Ctrl+A, Enter and
        // Esc (its context is deeper).
        KeyBinding::new("down", NextRow, messages),
        KeyBinding::new("up", PreviousRow, messages),
        KeyBinding::new("shift-down", ExtendDown, messages),
        KeyBinding::new("shift-up", ExtendUp, messages),
        KeyBinding::new("ctrl-a", SelectAllRows, messages),
        KeyBinding::new("escape", EscapeList, messages),
        // Backspace archives from the rows or the reading pane only (never
        // the filters; in Drafts and the Outbox the handler does nothing).
        KeyBinding::new("backspace", Archive, messages),
        KeyBinding::new("shift-f10", RowMenu, list),
        KeyBinding::new("menu", RowMenu, list),
        KeyBinding::new("down", NavDown, nav),
        KeyBinding::new("up", NavUp, nav),
        KeyBinding::new("home", NavFirst, nav),
        KeyBinding::new("end", NavLast, nav),
        KeyBinding::new("enter", NavOpen, nav),
        KeyBinding::new("space", NavOpen, nav),
        KeyBinding::new("right", NavExpand, nav),
        KeyBinding::new("left", NavCollapse, nav),
        KeyBinding::new("shift-f10", NavMenu, nav),
        KeyBinding::new("menu", NavMenu, nav),
        // Esc closes the pane opened over the content from the rail.
        KeyBinding::new("escape", ClosePane, nav),
        KeyBinding::new("down", FlyoutNext, Some(FLYOUT)),
        KeyBinding::new("up", FlyoutPrevious, Some(FLYOUT)),
        KeyBinding::new("home", FlyoutFirst, Some(FLYOUT)),
        KeyBinding::new("end", FlyoutLast, Some(FLYOUT)),
        KeyBinding::new("enter", FlyoutRun, Some(FLYOUT)),
        KeyBinding::new("space", FlyoutRun, Some(FLYOUT)),
        KeyBinding::new("escape", FlyoutClose, Some(FLYOUT)),
    ]);
}
