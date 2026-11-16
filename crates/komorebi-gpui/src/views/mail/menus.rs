//! The mail view's menus (the Slint app's row menu, More menu, list menu
//! and folder pane menu; `src/app/triage.rs`, `nav_menu` in `src/app.rs`):
//!
//! - Right-click menus are Kit popup menus in the brand's surface
//!   (`widgets::menu::brand`, cookbook_menu). Commands with a key are the
//!   key's action, sent to the list (`action_context`), so the menu shows
//!   the key beside them; the others run on click.
//! - The keyboard opens the same entries as a [`Flyout`] (Shift+F10 or the
//!   Menu key: a Kit context menu opens on a mouse press only), and Move
//!   (Ctrl+Shift+V, or Move to in a menu) is a flyout with a filter and the
//!   recent folders first: the palette pattern (cookbook_palette: its own
//!   field, its own list, capture_action, an overlay that closes on a click
//!   outside).

use gpui::{
    AnyElement, App, Context, ElementId, Focusable as _, InteractiveElement, IntoElement,
    ParentElement, Role, ScrollHandle, Styled, WeakEntity, Window, deferred, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;

use super::commands;
use super::folder_edit::{self, Target};
use super::nav::NavKind;
use super::text::{self, Command};
use super::triage::Flag;
use super::{MailEvent, MailView, Place};
use crate::access::{self, Key};
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::reader::actions::{self, MenuCmd, SourceUse};
use crate::widgets::field::{FieldLook, Nav, TextField};
use crate::widgets::menu;

/// What a menu entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Cmd(Command),
    /// Move the open message or the selection to this folder.
    MoveTo(i64),
    /// A folder pane line's command.
    Nav(NavKind, NavRun),
    /// A view toggle of the reader's More menu.
    Reader(MenuCmd),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavRun {
    /// Go to this folder (a rail tile's Inbox, Sent, Archive).
    Open(i64),
    MarkRead,
    Collapse,
    Refresh,
    AccountSettings,
    Fold,
    /// New folder (an account line, the pane's empty room) or New
    /// subfolder (a folder line): the name is typed in the pane.
    NewFolder,
    /// Rename the folder in place (F2).
    RenameFolder,
    /// Ask to delete the folder (Delete).
    DeleteFolder,
}

/// One entry of a menu or a flyout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    /// The key, shown at the entry's end.
    pub keys: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    /// The Move flyout's indent.
    pub level: usize,
    pub run: Run,
}

fn entry(cmd: Command, enabled: bool) -> Entry {
    Entry {
        label: text::command(cmd),
        keys: keys_of(cmd),
        enabled,
        checked: None,
        level: 0,
        run: Run::Cmd(cmd),
    }
}

/// The key a command has (the Slint menu's accelerator text).
fn keys_of(cmd: Command) -> Option<String> {
    Some(String::from(match cmd {
        Command::OpenTab => "Shift+Enter",
        Command::OpenWindow => "Ctrl+O",
        Command::Reply => "Ctrl+R",
        Command::ReplyAll => "Ctrl+Shift+R",
        Command::Forward => "Ctrl+Shift+F",
        Command::MarkRead => "Ctrl+Q",
        Command::MarkUnread => "Ctrl+U",
        Command::Flag | Command::Unflag => "Ctrl+Shift+G",
        // Backspace has a name in each language (the Slint menus' too).
        Command::Archive => return Some(komorebi_i18n::tr!("key" => "Backspace")),
        Command::Delete => "Del",
        Command::MoveTo => "Ctrl+Shift+V",
        Command::SaveEml => "Ctrl+S",
        _ => return None,
    }))
}

/// The action a command's key sends (the menu dispatches it to the list).
fn action_of(cmd: Command) -> Option<Box<dyn gpui::Action>> {
    Some(match cmd {
        Command::Reply => Box::new(commands::Reply),
        Command::ReplyAll => Box::new(commands::ReplyAll),
        Command::Forward => Box::new(commands::Forward),
        Command::MarkRead => Box::new(commands::MarkRead),
        Command::MarkUnread => Box::new(commands::MarkUnread),
        Command::Flag | Command::Unflag => Box::new(commands::ToggleImportant),
        Command::Archive => Box::new(commands::Archive),
        Command::Delete => Box::new(commands::Delete),
        Command::MoveTo => Box::new(commands::MoveTo),
        Command::SaveEml => Box::new(commands::SaveEml),
        _ => return None,
    })
}

/// The row menu's groups for what is selected or open (the Slint
/// `row_menu`): open, respond; read, important; Archive, Delete, Move;
/// junk; the message itself (with the reader, phase 3); Show in folder.
pub fn row_entries(view: &MailView) -> Vec<Vec<Entry>> {
    // A Drafts row (the Slint draft menu): Open, Discard.
    if view.mail.view() == super::model::View::Drafts {
        return vec![vec![
            entry(Command::OpenDraft, view.mail.current_row().is_some()),
            entry(Command::DiscardDraft, view.mail.current_row().is_some()),
        ]];
    }
    let m = view.mail.row_menu();
    let read = if m.unread {
        entry(Command::MarkRead, true)
    } else {
        entry(Command::MarkUnread, true)
    };
    let flag = if m.unflagged {
        entry(Command::Flag, true)
    } else {
        entry(Command::Unflag, true)
    };
    let junk = if m.junk {
        entry(Command::NotJunk, true)
    } else {
        entry(Command::Junk, true)
    };
    let mut groups = vec![
        vec![
            entry(Command::OpenTab, m.one),
            entry(Command::OpenWindow, m.one),
            entry(Command::Reply, m.one && !m.drafts),
            entry(Command::ReplyAll, m.one && !m.drafts),
            entry(Command::Forward, m.one),
            entry(Command::ForwardAttached, m.one),
        ],
        vec![read, flag],
        vec![
            entry(Command::Archive, true),
            entry(Command::Delete, true),
            entry(Command::MoveTo, m.can_move),
        ],
        vec![junk],
        source_entries(m.one),
    ];
    if m.show_in_folder {
        groups.push(vec![entry(Command::ShowInFolder, m.one)]);
    }
    groups
}

/// The message's source: Save as .eml, Show original, Copy Message-ID (the
/// reader runs them, `views::reader::actions`).
fn source_entries(one: bool) -> Vec<Entry> {
    vec![
        entry(Command::SaveEml, one),
        entry(Command::ShowOriginal, one),
        entry(Command::CopyId, one),
    ]
}

/// The reading pane's More menu (the Slint `more_menu`): Open in new
/// window, Forward as attachment; Report junk or Not junk; the reader's
/// view toggles (View as plain text, Show original colours, Load images or
/// Stop loading images from this sender); the source; Show in folder.
pub fn more_entries(view: &MailView, cx: &App) -> Vec<Vec<Entry>> {
    let m = view.mail.row_menu();
    let junk = if m.junk {
        entry(Command::NotJunk, true)
    } else {
        entry(Command::Junk, true)
    };
    let v = view.reader.read(cx).view_menu();
    let toggle = |cmd: MenuCmd, (enabled, checked): (bool, bool)| Entry {
        label: actions::menu_label(cmd),
        keys: None,
        enabled,
        checked: Some(checked),
        level: 0,
        run: Run::Reader(cmd),
    };
    let images = if v.stop_images {
        MenuCmd::StopImages
    } else {
        MenuCmd::LoadImages
    };
    let mut groups = vec![
        vec![
            entry(Command::OpenWindow, m.one),
            entry(Command::ForwardAttached, m.one),
        ],
        vec![junk],
        vec![
            toggle(MenuCmd::Plain, v.plain),
            toggle(MenuCmd::Colours, v.colours),
            Entry {
                checked: None,
                ..toggle(images, (v.stop_images || v.load_images, false))
            },
        ],
        source_entries(m.one),
    ];
    if m.show_in_folder {
        groups.push(vec![entry(Command::ShowInFolder, m.one)]);
    }
    groups
}

/// The command bar's buttons that went to More (`ids`, in the bar's
/// order) as its first entries: the same commands, named as their buttons.
pub fn bar_entries(view: &MailView, ids: &[&str]) -> Vec<Entry> {
    let m = view.mail.row_menu();
    ids.iter()
        .filter_map(|id| {
            Some(match *id {
                "cmd-reply-all" => entry(Command::ReplyAll, true),
                "cmd-forward" if m.drafts => Entry {
                    label: komorebi_i18n::tr!("Forward as new draft"),
                    ..entry(Command::Forward, true)
                },
                "cmd-forward" => entry(Command::Forward, true),
                "cmd-archive" => entry(Command::Archive, true),
                "cmd-delete" => entry(Command::Delete, true),
                "cmd-move" => entry(Command::MoveTo, m.can_move),
                "cmd-read" if m.unread => entry(Command::MarkRead, true),
                "cmd-read" => entry(Command::MarkUnread, true),
                "cmd-flag" if m.unflagged => entry(Command::Flag, true),
                "cmd-flag" => entry(Command::Unflag, true),
                _ => return None,
            })
        })
        .collect()
}

/// The list's "..." menu: Mark all as read, Select messages (checked
/// while the boxes show). Not while a search shows: it would mark the
/// whole view read, not the results.
pub fn list_entries(view: &MailView) -> Vec<Entry> {
    let mail = &view.mail;
    let unread = mail.shown_unread();
    let has_unread = mail
        .view_folders()
        .iter()
        .any(|f| unread.get(f).is_some_and(|&n| n > 0));
    vec![
        entry(
            Command::MarkAllRead,
            has_unread && mail.lists_messages() && !mail.list_drafts() && !mail.searching(),
        ),
        Entry {
            checked: Some(mail.select_mode()),
            ..entry(Command::SelectMessages, mail.lists_messages())
        },
    ]
}

/// A folder pane line's menu (none for Drafts and the Outbox).
pub fn nav_entries(view: &MailView, kind: NavKind, rail: bool) -> Vec<Vec<Entry>> {
    let Some(menu) = view.mail.nav_menu(kind, rail) else {
        return Vec::new();
    };
    let nav = |label: String, enabled: bool, run: NavRun| Entry {
        label,
        keys: None,
        enabled,
        checked: None,
        level: 0,
        run: Run::Nav(kind, run),
    };
    let mut main = Vec::new();
    for (role, folder) in &menu.open {
        let label = folder
            .and_then(|id| view.mail.folder(id))
            .map(text::folder_label)
            .or_else(|| text::role_name(*role))
            .unwrap_or_default();
        main.push(nav(
            label,
            folder.is_some(),
            NavRun::Open(folder.unwrap_or(0)),
        ));
    }
    if let Some(enabled) = menu.mark_read {
        main.push(nav(
            text::command(Command::MarkAllRead),
            enabled,
            NavRun::MarkRead,
        ));
    }
    if let Some((account, enabled)) = menu.collapse {
        let label = if account {
            text::collapse_all(true)
        } else {
            text::collapse_all(false)
        };
        main.push(nav(label, enabled, NavRun::Collapse));
    }
    // Make, rename, delete: never a special folder's Rename or Delete.
    let mut change = Vec::new();
    if !rail && let Some(f) = view.mail.folder_menu(kind) {
        let keyed = |label: String, keys: &str, run: NavRun| Entry {
            keys: Some(keys.to_string()),
            ..nav(label, true, run)
        };
        match kind {
            NavKind::Account(_) => {
                change.push(keyed(
                    folder_edit::new_folder(),
                    "Ctrl+Shift+N",
                    NavRun::NewFolder,
                ));
            }
            _ => change.push(nav(folder_edit::new_subfolder(), f.new, NavRun::NewFolder)),
        }
        if f.change {
            change.push(keyed(folder_edit::rename(), "F2", NavRun::RenameFolder));
            change.push(keyed(folder_edit::delete(), "Del", NavRun::DeleteFolder));
        }
    }
    let mut extra = Vec::new();
    if let Some(on) = menu.account {
        extra.push(nav(text::refresh(), on, NavRun::Refresh));
        extra.push(nav(
            text::command(Command::AccountSettings),
            true,
            NavRun::AccountSettings,
        ));
    }
    // Hide folders / Show folders: from the compact width up (below it the
    // pane is always the rail).
    if !view.compact_now {
        let label = if view.mail.nav_collapsed() {
            text::command(Command::ShowFolders)
        } else {
            text::command(Command::HideFolders)
        };
        extra.push(nav(label, true, NavRun::Fold));
    }
    vec![main, change, extra]
}

/// The access key of an entry's command (`crate::access`): the same letter
/// as Alt and that letter, here the item's own while its menu is open.
pub(super) fn access_of(run: Run) -> Option<Key> {
    let Run::Cmd(cmd) = run else {
        return None;
    };
    Some(match cmd {
        Command::Reply => Key::Reply,
        Command::ReplyAll => Key::ReplyAll,
        Command::Forward => Key::Forward,
        Command::Archive => Key::Archive,
        Command::Delete => Key::Delete,
        Command::MoveTo => Key::MoveTo,
        Command::Flag | Command::Unflag => Key::Important,
        Command::MarkRead => Key::MarkRead,
        Command::MarkUnread => Key::MarkUnread,
        Command::SaveEml => Key::SaveEml,
        _ => return None,
    })
}

/// Fills a Kit popup menu with entry groups (separators between).
fn fill(
    mut menu: PopupMenu,
    groups: Vec<Vec<Entry>>,
    view: WeakEntity<MailView>,
    action_context: Option<gpui::FocusHandle>,
    cx: &App,
) -> PopupMenu {
    menu = menu::brand(menu, cx);
    if let Some(focus) = action_context {
        menu = menu.action_context(focus);
    }
    let lang = crate::i18n::lang(cx);
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        menu = menu.separator();
        for e in group {
            let run = e.run;
            let mut item = PopupMenuItem::new(e.label.clone()).disabled(!e.enabled);
            if let Some(checked) = e.checked {
                item = item.checked(checked);
            }
            if let Some(key) = access_of(run) {
                item = item.access_key(key.letter(lang));
            }
            item = match run {
                Run::Cmd(cmd) if action_of(cmd).is_some() => {
                    item.action(action_of(cmd).unwrap_or_else(|| Box::new(commands::Undo)))
                }
                _ => {
                    let view = view.clone();
                    item.on_click(move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| this.run(run, window, cx));
                    })
                }
            };
            menu = menu.item(item);
        }
    }
    menu
}

/// The row menu on a right-click: the row is selected first unless it
/// already is (never opened, so reporting junk never marks it read).
pub fn row_menu(
    view: WeakEntity<MailView>,
    id: i64,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let built = view.update(cx, |this, cx| {
        this.act_later(cx, |m| {
            m.row_context(id);
        });
        if !this.mail.lists_messages() && this.mail.view() != super::model::View::Drafts {
            return None;
        }
        Some((row_entries(this), this.list_focus.clone()))
    });
    match built {
        Ok(Some((groups, focus))) => fill(menu, groups, view, Some(focus), cx),
        _ => menu,
    }
}

/// The reading pane's More menu ([`more_entries`]), after the command
/// bar's buttons that did not fit it (`over`, [`bar_entries`]).
pub fn more_menu(
    view: WeakEntity<MailView>,
    over: &[&str],
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    // One pane hides the list behind the open message: the commands go
    // from the reading pane then (a focus outside the frame reaches no
    // handler).
    let one_pane = window.viewport_size().width.as_f32() < super::ONE_PANE_BELOW_W;
    let built = view
        .read_with(cx, |this, cx| {
            let mut groups = more_entries(this, cx);
            let bar = bar_entries(this, over);
            if !bar.is_empty() {
                groups.insert(0, bar);
            }
            let focus = if one_pane {
                this.reading_focus.clone()
            } else {
                this.list_focus.clone()
            };
            (groups, focus)
        })
        .ok();
    match built {
        Some((groups, focus)) => fill(menu, groups, view, Some(focus), cx),
        None => menu,
    }
}

/// The list's "..." menu.
pub fn list_menu(
    view: WeakEntity<MailView>,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let entries = view.read_with(cx, |this, _| list_entries(this)).ok();
    match entries {
        Some(entries) => fill(menu, vec![entries], view, None, cx),
        None => menu,
    }
}

/// A folder pane line's menu on a right-click.
pub fn nav_menu(
    view: WeakEntity<MailView>,
    kind: NavKind,
    rail: bool,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let groups = view
        .read_with(cx, |this, _| nav_entries(this, kind, rail))
        .unwrap_or_default();
    fill(menu, groups, view, None, cx)
}

/// The menu of the pane's room below its lines: New folder (in the open
/// folder's account, else the first), while an account is on.
pub fn pane_entries(view: &MailView) -> Vec<Vec<Entry>> {
    if view.mail.new_folder_account(None).is_none() {
        return Vec::new();
    }
    vec![vec![Entry {
        label: folder_edit::new_folder(),
        keys: Some("Ctrl+Shift+N".into()),
        enabled: true,
        checked: None,
        level: 0,
        run: Run::Nav(NavKind::AllInboxes, NavRun::NewFolder),
    }]]
}

/// The pane's room below its lines, on a right-click.
pub fn pane_menu(
    view: WeakEntity<MailView>,
    menu: PopupMenu,
    _window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let groups = view
        .read_with(cx, |this, _| pane_entries(this))
        .unwrap_or_default();
    fill(menu, groups, view, None, cx)
}

// ------------------------------------------------------------ flyouts

/// What a flyout lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutKind {
    /// Move to: the folders, recent first, with a filter.
    Move,
    /// The row menu, from the keyboard.
    Row,
    /// A folder pane line's menu, from the keyboard.
    Nav(NavKind, bool),
}

/// An open flyout: its kind and the keyboard's entry.
#[derive(Debug)]
pub struct Flyout {
    pub kind: FlyoutKind,
    pub selected: usize,
    pub scroll: ScrollHandle,
}

impl MailView {
    /// The open flyout's entries (enabled ones only: the keyboard skips
    /// the others).
    pub(super) fn flyout_entries(&self, kind: FlyoutKind, cx: &App) -> Vec<Entry> {
        let entries: Vec<Entry> = match kind {
            FlyoutKind::Move => {
                let filter = self.move_filter.read(cx).value().to_string();
                self.mail
                    .move_targets(&filter)
                    .into_iter()
                    .map(|o| Entry {
                        label: o.label,
                        keys: None,
                        enabled: true,
                        checked: None,
                        // The recent copies are drawn flush.
                        level: if o.recent { 0 } else { o.level },
                        run: Run::MoveTo(o.id),
                    })
                    .collect()
            }
            FlyoutKind::Row => row_entries(self).into_iter().flatten().collect(),
            FlyoutKind::Nav(kind, rail) => nav_entries(self, kind, rail)
                .into_iter()
                .flatten()
                .collect(),
        };
        entries.into_iter().filter(|e| e.enabled).collect()
    }

    pub(super) fn open_flyout(
        &mut self,
        kind: FlyoutKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.flyout_entries(kind, cx).is_empty() && kind != FlyoutKind::Move {
            return;
        }
        self.flyout = Some(Flyout {
            kind,
            selected: 0,
            scroll: ScrollHandle::new(),
        });
        if kind == FlyoutKind::Move {
            self.move_filter
                .update(cx, |s, cx| s.set_value("", window, cx));
            let focus = self.move_filter.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        } else {
            window.focus(&self.flyout_focus, cx);
        }
        cx.notify();
    }

    pub(super) fn close_flyout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(flyout) = self.flyout.take() else {
            return;
        };
        let back = match flyout.kind {
            FlyoutKind::Nav(..) => &self.nav_focus,
            _ => &self.list_focus,
        };
        window.focus(back, cx);
        cx.notify();
    }

    fn flyout_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(kind) = self.flyout.as_ref().map(|f| f.kind) else {
            return;
        };
        let n = self.flyout_entries(kind, cx).len();
        if let Some(f) = self.flyout.as_mut()
            && n > 0
        {
            f.selected = (f.selected as isize + delta).rem_euclid(n as isize) as usize;
            f.scroll.scroll_to_item(f.selected);
            cx.notify();
        }
    }

    fn flyout_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((kind, selected)) = self.flyout.as_ref().map(|f| (f.kind, f.selected)) else {
            return;
        };
        let run = self.flyout_entries(kind, cx).get(selected).map(|e| e.run);
        self.close_flyout(window, cx);
        if let Some(run) = run {
            self.run(run, window, cx);
        }
    }

    /// The flyout's field or list keys.
    pub(super) fn flyout_key(&mut self, key: Nav, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            Nav::Up => self.flyout_step(-1, cx),
            Nav::Down => self.flyout_step(1, cx),
            Nav::Enter | Nav::SecondaryEnter => self.flyout_confirm(window, cx),
            Nav::Escape => self.close_flyout(window, cx),
            Nav::Home | Nav::PageUp => {
                if let Some(f) = self.flyout.as_mut() {
                    f.selected = 0;
                    cx.notify();
                }
            }
            Nav::End | Nav::PageDown => {
                let kind = self.flyout.as_ref().map(|f| f.kind);
                let n = kind.map_or(0, |k| self.flyout_entries(k, cx).len());
                if let Some(f) = self.flyout.as_mut() {
                    f.selected = n.saturating_sub(1);
                    cx.notify();
                }
            }
        }
    }

    /// Run a menu entry.
    pub(super) fn run(&mut self, run: Run, window: &mut Window, cx: &mut Context<Self>) {
        match run {
            Run::Cmd(cmd) => self.command(cmd, window, cx),
            Run::MoveTo(id) => self.act(window, cx, |m| m.move_to(id)),
            Run::Nav(kind, nav) => self.nav_run(kind, nav, window, cx),
            Run::Reader(cmd) => self.reader.update(cx, |r, cx| r.menu(cmd, window, cx)),
        }
    }

    /// Save as .eml, Show original, Copy Message-ID: on the open message,
    /// or the one selected row, which opens first (the Slint
    /// `source_action`); the reader waits for a body that is not here yet.
    pub(super) fn source_command(
        &mut self,
        what: SourceUse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::model::Mail;
        let ids = self.mail.selection().ids();
        if self.mail.open_id().is_none()
            && let [id] = ids[..]
        {
            self.act(window, cx, |m: &mut Mail| {
                m.select_message(id);
            });
        }
        let Some(id) = self.mail.open_id() else {
            return;
        };
        self.reader.update(cx, |r, cx| {
            if r.message() != Some(id) {
                r.open(id, window, cx);
            }
            r.source(what, window, cx);
        });
    }

    fn command(&mut self, cmd: Command, window: &mut Window, cx: &mut Context<Self>) {
        use super::model::Mail;
        use crate::views::composer::Draft;
        match cmd {
            Command::Reply => self.respond(
                |message| Draft::Reply {
                    message,
                    all: false,
                },
                window,
                cx,
            ),
            Command::ReplyAll => {
                self.respond(|message| Draft::Reply { message, all: true }, window, cx)
            }
            Command::Forward => self.respond(
                |message| Draft::Forward {
                    message,
                    attached: false,
                },
                window,
                cx,
            ),
            Command::ForwardAttached => self.respond(
                |message| Draft::Forward {
                    message,
                    attached: true,
                },
                window,
                cx,
            ),
            Command::MarkRead => self.act(window, cx, |m| m.mark(Flag::Seen, true)),
            Command::MarkUnread => self.act(window, cx, |m| m.mark(Flag::Seen, false)),
            Command::Flag => self.act(window, cx, |m| m.mark(Flag::Flagged, true)),
            Command::Unflag => self.act(window, cx, |m| m.mark(Flag::Flagged, false)),
            Command::Archive => self.act(window, cx, Mail::archive),
            Command::Delete => self.act(window, cx, Mail::delete),
            Command::MoveTo => self.open_flyout(FlyoutKind::Move, window, cx),
            Command::Junk => self.act(window, cx, Mail::report_junk),
            Command::NotJunk => self.act(window, cx, Mail::not_junk),
            Command::ShowInFolder => {
                self.act(window, cx, Mail::show_in_folder);
                self.search.update(cx, |s, cx| s.set_value("", window, cx));
            }
            Command::MarkAllRead => self.act(window, cx, Mail::mark_all_read),
            Command::SelectMessages => {
                let on = !self.mail.select_mode();
                self.act(window, cx, |m| m.set_select_mode(on));
            }
            Command::HideFolders | Command::ShowFolders => self.toggle_folders(window, cx),
            Command::AccountSettings => match self.mail.view_account().map(|a| a.id) {
                Some(id) => cx.emit(MailEvent::EditAccount(id)),
                None => cx.emit(MailEvent::Show(Place::Settings)),
            },
            Command::OpenTab => {
                if let Some(id) = self.mail.current_row() {
                    cx.emit(MailEvent::OpenTab {
                        id,
                        background: false,
                    });
                }
            }
            Command::OpenWindow => {
                if let Some(id) = self.mail.current_row() {
                    cx.emit(MailEvent::OpenWindow(id));
                }
            }
            Command::SaveEml => self.source_command(SourceUse::SaveEml, window, cx),
            Command::ShowOriginal => self.source_command(SourceUse::ShowOriginal, window, cx),
            Command::CopyId => self.source_command(SourceUse::CopyId, window, cx),
            Command::OpenDraft => self.act(window, cx, |m| {
                m.open_current();
            }),
            Command::DiscardDraft => self.discard_draft(window, cx),
        }
    }

    fn nav_run(&mut self, kind: NavKind, run: NavRun, window: &mut Window, cx: &mut Context<Self>) {
        match run {
            NavRun::Open(id) if id > 0 => self.nav_clicked(NavKind::Folder(id), false, window, cx),
            NavRun::Open(_) => {}
            NavRun::MarkRead => self.act(window, cx, |m| m.nav_mark_read(kind)),
            NavRun::Collapse => self.act(window, cx, |m| m.collapse_all(kind)),
            // An account line's Refresh syncs that account only (the
            // Slint `retry_account`).
            NavRun::Refresh => match kind {
                NavKind::Account(id) => self.refresh_account(id, window, cx),
                _ => self.refresh(window, cx),
            },
            NavRun::AccountSettings => match kind {
                NavKind::Account(id) => cx.emit(MailEvent::EditAccount(id)),
                _ => cx.emit(MailEvent::Show(Place::Settings)),
            },
            NavRun::Fold => self.toggle_folders(window, cx),
            NavRun::NewFolder => {
                let target = match kind {
                    NavKind::Folder(id) => self.mail.folder(id).map(|f| Target::New {
                        account: f.account_id,
                        parent: Some(id),
                    }),
                    NavKind::Account(account) => Some(Target::New {
                        account,
                        parent: None,
                    }),
                    _ => self
                        .mail
                        .new_folder_account(None)
                        .map(|account| Target::New {
                            account,
                            parent: None,
                        }),
                };
                if let Some(target) = target {
                    self.start_folder_edit(target, window, cx);
                }
            }
            NavRun::RenameFolder => {
                if let NavKind::Folder(id) = kind {
                    self.start_folder_edit(Target::Rename(id), window, cx);
                }
            }
            NavRun::DeleteFolder => {
                if let NavKind::Folder(id) = kind {
                    self.ask_delete_folder(id, window, cx);
                }
            }
        }
    }
}

/// Ctrl+Shift+V: the Move flyout, when the open message or the selection
/// can move.
pub fn open_move(view: &mut MailView, window: &mut Window, cx: &mut Context<MailView>) {
    if view.mail.lists_messages() && !view.mail.move_choices().is_empty() {
        view.open_flyout(FlyoutKind::Move, window, cx);
    }
}

/// Shift+F10 or the Menu key in the list: the row menu of the current row.
pub fn open_row_menu_at_cursor(
    view: &mut MailView,
    window: &mut Window,
    cx: &mut Context<MailView>,
) {
    if !view.mail.lists_messages() {
        return;
    }
    if let Some(id) = view.mail.current_row() {
        view.act(window, cx, |m| {
            m.row_context(id);
        });
        view.open_flyout(FlyoutKind::Row, window, cx);
    }
}

/// Shift+F10 or the Menu key in the folder pane: the line's menu.
pub fn open_nav_menu_at_cursor(
    view: &mut MailView,
    window: &mut Window,
    cx: &mut Context<MailView>,
) {
    let rail = view.keys_on_rail(window);
    if let Some(item) = view.nav_items(rail).items.get(view.nav_cursor) {
        let kind = item.kind;
        view.open_flyout(FlyoutKind::Nav(kind, rail), window, cx);
    }
}

/// The open flyout, over the mail view (deferred, closes on a click
/// outside).
pub fn render_flyout(view: &mut MailView, cx: &mut Context<MailView>) -> Option<AnyElement> {
    let flyout = view.flyout.as_ref()?;
    let (kind, selected, scroll) = (flyout.kind, flyout.selected, flyout.scroll.clone());
    let entries = view.flyout_entries(kind, cx);
    let t = cx.tokens().clone();
    let c = t.color;
    let name = match kind {
        FlyoutKind::Move => text::command(Command::MoveTo),
        FlyoutKind::Row | FlyoutKind::Nav(..) => text::more_actions(),
    };
    // The row menu's access keys: the letter runs its entry, as in Kit's
    // menus (the Move list's letters go to its filter).
    let lang = crate::i18n::lang(cx);
    let chips = access::shown(cx);
    let letters: Vec<Option<char>> = entries
        .iter()
        .map(|e| {
            (kind == FlyoutKind::Row)
                .then(|| access_of(e.run))
                .flatten()
                .map(|k| k.letter(lang))
        })
        .collect();
    let rows: Vec<AnyElement> = entries
        .iter()
        .enumerate()
        .map(|(ix, e)| {
            let on = ix == selected;
            let run = e.run;
            let letter = letters[ix];
            div()
                .id(ElementId::Name(format!("flyout-{ix}").into()))
                .role(Role::MenuItem)
                .aria_label(e.label.clone())
                .when_some(letter, |el, k| el.aria_keyshortcuts(k.to_string()))
                .aria_selected(on)
                .when(on, |el| el.aria_active_descendant())
                .h(px(crate::theme::metrics::scaled::control(
                    &t,
                    layout::CONTROL_H,
                )))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::SP_3))
                .pl(px(
                    space::SP_3 + crate::theme::metrics::scaled::indent(e.level)
                ))
                .pr(px(space::SP_3))
                .rounded(px(radius::FULL))
                .typo(&t.text.body)
                .text_color(c.ink)
                .when(on, |el| el.bg(c.wash))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if let Some(f) = this.flyout.as_mut()
                        && f.selected != ix
                    {
                        f.selected = ix;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_flyout(window, cx);
                    this.run(run, window, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(e.label.clone()),
                )
                .when_some(e.keys.clone(), |el, keys| {
                    el.child(
                        div()
                            .flex_none()
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child(keys),
                    )
                })
                .when_some(letter.filter(|_| chips), |el, k| {
                    el.child(access::inline_chip(k, &t))
                })
                .test_support()
                .into_any_element()
        })
        .collect();
    let view_handle = cx.entity().downgrade();
    let field = (kind == FlyoutKind::Move).then(|| {
        TextField::new(&view.move_filter, text::filter_folders())
            .look(FieldLook::Pill)
            .navigation(move |key, window, cx| {
                let _ = view_handle.update(cx, |this, cx| this.flyout_key(key, window, cx));
            })
    });
    let card = div()
        .id("flyout")
        .role(Role::Menu)
        .aria_label(name)
        .key_context(commands::FLYOUT)
        .track_focus(&view.flyout_focus)
        .on_action(cx.listener(|this, _: &commands::FlyoutNext, window, cx| {
            this.flyout_key(Nav::Down, window, cx);
        }))
        .on_action(
            cx.listener(|this, _: &commands::FlyoutPrevious, window, cx| {
                this.flyout_key(Nav::Up, window, cx);
            }),
        )
        .on_action(cx.listener(|this, _: &commands::FlyoutFirst, window, cx| {
            this.flyout_key(Nav::Home, window, cx);
        }))
        .on_action(cx.listener(|this, _: &commands::FlyoutLast, window, cx| {
            this.flyout_key(Nav::End, window, cx);
        }))
        .on_action(cx.listener(|this, _: &commands::FlyoutRun, window, cx| {
            this.flyout_key(Nav::Enter, window, cx);
        }))
        .on_action(cx.listener(|this, _: &commands::FlyoutClose, window, cx| {
            this.flyout_key(Nav::Escape, window, cx);
        }))
        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_flyout(window, cx)))
        .on_key_down(
            cx.listener(move |this, e: &gpui::KeyDownEvent, window, cx| {
                let m = &e.keystroke.modifiers;
                if m.control || m.platform || m.function {
                    return;
                }
                let mut key = e.keystroke.key.chars();
                let (Some(k), None) = (key.next(), key.next()) else {
                    return;
                };
                let Some(ix) = letters
                    .iter()
                    .position(|l| l.is_some_and(|l| l.eq_ignore_ascii_case(&k)))
                else {
                    return;
                };
                cx.stop_propagation();
                if let Some(f) = this.flyout.as_mut() {
                    f.selected = ix;
                }
                this.flyout_key(Nav::Enter, window, cx);
            }),
        )
        .w(px(layout::DIALOG_W))
        .max_h(px(layout::DIALOG_W))
        .flex()
        .flex_col()
        .gap(px(space::SP_2))
        .p(px(space::SP_2))
        .rounded(px(radius::CARD))
        .bg(c.paper)
        .border(px(layout::HAIRLINE))
        .border_color(c.line)
        .children(field)
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .min_h_0()
                .child(
                    div()
                        .id("flyout-list")
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_0_5))
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .children(rows),
                )
                .vertical_scrollbar(&scroll),
        )
        .test_support();
    Some(
        deferred(
            div()
                .absolute()
                .size_full()
                .occlude()
                .flex()
                .justify_center()
                .pt(px(layout::CMD_BAR_H))
                .child(card),
        )
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    /// Port of `menu_entries_carry_their_accelerator_after_a_tab`
    /// (src/app/triage.rs): each entry with a key shows it at its end
    /// (the Slint menus put it after a tab), Backspace by its name in the
    /// language shown.
    #[test]
    fn menu_entries_carry_their_keys() {
        let keys = |cmd| entry(cmd, true).keys;
        with_lang(Lang::En, || {
            for (cmd, want) in [
                (Command::Reply, "Ctrl+R"),
                (Command::ReplyAll, "Ctrl+Shift+R"),
                (Command::Forward, "Ctrl+Shift+F"),
                (Command::MarkUnread, "Ctrl+U"),
                (Command::Flag, "Ctrl+Shift+G"),
                (Command::Archive, "Backspace"),
                (Command::Delete, "Del"),
                (Command::SaveEml, "Ctrl+S"),
            ] {
                assert_eq!(keys(cmd).as_deref(), Some(want), "{cmd:?}");
            }
            assert_eq!(keys(Command::ShowOriginal), None);
            let e = entry(Command::Reply, false);
            assert_eq!((e.label.as_str(), e.enabled), ("Reply", false));
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(keys(Command::Archive).as_deref(), Some("Retrocesso"));
            assert_eq!(entry(Command::Reply, true).label, "Responder");
        });
    }
}
