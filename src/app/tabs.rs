//! The main window's tabs (docs/WINUI3.md "TabView"): the fixed Mail tab
//! (the folders, the list and the reading pane), message tabs (one message
//! at full width) and composer tabs (one draft each). One reader and one
//! composer serve them all: a tab switch saves what the leaving tab showed
//! and shows the entering one's message or draft.
//!
//! The `tabs` setting holds ids only (message ids, draft rows and the
//! active tab), never a subject, an address or any text.

use super::composer::DraftId;
use std::collections::{HashSet, VecDeque};

/// At most this many tabs come back at a start.
pub(super) const RESTORE_MAX: usize = 20;
/// Ctrl+Shift+T reopens up to this many closed tabs (this session only).
const CLOSED_MAX: usize = 10;

/// What a tab shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabKind {
    /// The folders, the list and the reading pane; always the first tab.
    Mail,
    /// One message, by its cache id.
    Message(i64),
    /// One draft held in memory.
    Draft(DraftId),
    /// A saved draft row not loaded yet (a restored tab): it loads when
    /// the tab is first shown.
    Saved(i64),
}

impl TabKind {
    /// The kind the window shows: 0 Mail, 1 a message, 2 a draft.
    pub(super) fn code(self) -> i32 {
        match self {
            TabKind::Mail => 0,
            TabKind::Message(_) => 1,
            TabKind::Draft(_) | TabKind::Saved(_) => 2,
        }
    }
}

/// A closed tab, as Ctrl+Shift+T opens it again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reopen {
    Message(i64),
    /// A saved draft row.
    Draft(i64),
}

/// How a message showed in a tab, put back when the tab is shown again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ViewState {
    /// The reader's scroll (`r-content-y`, 0 or less).
    pub scroll_y: f32,
    /// "View as plain text".
    pub plain: bool,
    /// "Show original colours".
    pub original: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Tab {
    pub kind: TabKind,
    /// The subject (or "New message"); only in the window's model, never
    /// in the setting or a log.
    pub title: String,
    pub unread: bool,
    /// The message was moved or deleted on the server: it stays readable
    /// from the cache until the tab closes, its actions off.
    pub gone: bool,
    pub view: ViewState,
}

impl Tab {
    pub(super) fn new(kind: TabKind, title: String) -> Self {
        Tab {
            kind,
            title,
            unread: false,
            gone: false,
            view: ViewState::default(),
        }
    }
}

/// What the Mail tab's reading pane held when another tab took over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct MailPane {
    pub open: Option<i64>,
    pub draft: Option<DraftId>,
    pub view: ViewState,
}

pub(super) struct Tabs {
    list: Vec<Tab>,
    active: usize,
    closed: VecDeque<Reopen>,
    /// The Mail tab's pane while another tab shows.
    pub mail: MailPane,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs {
            list: vec![Tab::new(TabKind::Mail, String::new())],
            active: 0,
            closed: VecDeque::new(),
            mail: MailPane::default(),
        }
    }
}

impl Tabs {
    pub(super) fn len(&self) -> usize {
        self.list.len()
    }

    pub(super) fn active(&self) -> usize {
        self.active
    }

    pub(super) fn active_kind(&self) -> TabKind {
        self.list[self.active].kind
    }

    pub(super) fn active_tab(&self) -> &Tab {
        &self.list[self.active]
    }

    pub(super) fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.list[self.active]
    }

    pub(super) fn get(&self, i: usize) -> Option<&Tab> {
        self.list.get(i)
    }

    #[cfg(test)]
    pub(super) fn get_mut(&mut self, i: usize) -> Option<&mut Tab> {
        self.list.get_mut(i)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.list.iter()
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.list.iter_mut()
    }

    pub(super) fn find(&self, kind: TabKind) -> Option<usize> {
        self.list.iter().position(|t| t.kind == kind)
    }

    /// The tab of message `id`, if one is open.
    pub(super) fn find_message(&self, id: i64) -> Option<usize> {
        self.find(TabKind::Message(id))
    }

    /// Make tab `i` the active one (the caller saves and shows).
    pub(super) fn set_active(&mut self, i: usize) {
        if i < self.list.len() {
            self.active = i;
        }
    }

    /// Put `tab` at `at` (after the Mail tab at least, at the end at
    /// most); returns its index. The active tab stays the same tab.
    pub(super) fn insert(&mut self, at: usize, tab: Tab) -> usize {
        let at = at.clamp(1, self.list.len());
        self.list.insert(at, tab);
        if at <= self.active {
            self.active += 1;
        }
        at
    }

    /// Remove tab `i` (never the Mail tab). When it was the active one, the
    /// tab to its left becomes active (the caller shows it).
    pub(super) fn remove(&mut self, i: usize) -> Option<Tab> {
        if i == 0 || i >= self.list.len() {
            return None;
        }
        let tab = self.list.remove(i);
        if i < self.active || (i == self.active && self.active > 0) {
            self.active -= 1;
        }
        Some(tab)
    }

    /// Remember a closed tab for Ctrl+Shift+T.
    pub(super) fn remember(&mut self, reopen: Reopen) {
        self.closed.retain(|r| *r != reopen);
        self.closed.push_back(reopen);
        while self.closed.len() > CLOSED_MAX {
            self.closed.pop_front();
        }
    }

    /// The last closed tab, taken off the stack.
    pub(super) fn take_closed(&mut self) -> Option<Reopen> {
        self.closed.pop_back()
    }

    pub(super) fn has_closed(&self) -> bool {
        !self.closed.is_empty()
    }

    /// Move tab `from` to place `to`; the Mail tab stays first. True if it
    /// moved. The active tab stays the same tab.
    pub(super) fn move_tab(&mut self, from: usize, to: usize) -> bool {
        let n = self.list.len();
        if from == 0 || from >= n {
            return false;
        }
        let to = to.clamp(1, n - 1);
        if to == from {
            return false;
        }
        let active = self.list[self.active].kind;
        let tab = self.list.remove(from);
        self.list.insert(to, tab);
        self.active = self.find(active).unwrap_or(0);
        true
    }

    /// The tab `delta` places from the active one, round the strip.
    pub(super) fn step(&self, delta: isize) -> usize {
        let n = self.list.len() as isize;
        (self.active as isize + delta).rem_euclid(n) as usize
    }

    /// Ctrl+1..8 go to tab N (when there is one), Ctrl+9 to the last.
    pub(super) fn number(&self, n: usize) -> Option<usize> {
        match n {
            9 => Some(self.list.len() - 1),
            1..=8 => (n - 1 < self.list.len()).then_some(n - 1),
            _ => None,
        }
    }

    /// The `tabs` setting: `m<message id>` and `d<draft row>` tokens in
    /// order, comma separated, then `;a=<index>` of the active one among
    /// them (the Mail tab is implied, 0). A draft without a saved row is
    /// left out (there is nothing to open again).
    pub(super) fn setting(&self, draft_row: impl Fn(DraftId) -> Option<i64>) -> String {
        let mut tokens = Vec::new();
        let mut active = 0;
        for (i, tab) in self.list.iter().enumerate().skip(1) {
            let token = match tab.kind {
                TabKind::Message(id) => Some(format!("m{id}")),
                TabKind::Draft(d) => draft_row(d).map(|row| format!("d{row}")),
                TabKind::Saved(row) => Some(format!("d{row}")),
                TabKind::Mail => None,
            };
            if let Some(token) = token {
                tokens.push(token);
                if i == self.active {
                    active = tokens.len();
                }
            }
        }
        if tokens.is_empty() {
            return String::new();
        }
        format!("{};a={active}", tokens.join(","))
    }
}

/// The `tabs` setting back ([`Tabs::setting`]): the tabs in order, at most
/// [`RESTORE_MAX`], and the active one's index (0: the Mail tab). Unknown
/// or broken tokens are skipped.
pub(super) fn parse_setting(value: &str) -> (Vec<Reopen>, usize) {
    let (list, active) = value.split_once(";a=").unwrap_or((value, "0"));
    let mut tabs: Vec<Reopen> = Vec::new();
    let mut map = Vec::new();
    for token in list.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let number = || token.get(1..)?.parse::<i64>().ok().filter(|&n| n > 0);
        let parsed = match token.get(..1) {
            Some("m") => number().map(Reopen::Message),
            Some("d") => number().map(Reopen::Draft),
            _ => None,
        };
        map.push(parsed.map(|p| {
            if let Some(i) = tabs.iter().position(|t| *t == p) {
                i + 1
            } else if tabs.len() < RESTORE_MAX {
                tabs.push(p);
                tabs.len()
            } else {
                0
            }
        }));
    }
    let active = active
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|a| a.checked_sub(1))
        .and_then(|a| map.get(a).copied().flatten())
        .unwrap_or(0);
    (tabs, active)
}

// ------------------------------------------------------------ the app side

use super::composer::{self, Draft};
use super::{App, View, t, with_app};
use crate::TabItem;
use slint::{ModelRc, Timer, TimerMode, VecModel};
use std::time::Duration;

/// The `tabs` setting is written this long after the last change.
const SAVE_AFTER: Duration = Duration::from_secs(1);

impl App {
    /// The Mail tab shows (the folders, the list and the reading pane).
    pub(super) fn in_mail_tab(&self) -> bool {
        self.tabs.active_kind() == TabKind::Mail
    }

    /// The active tab's message was moved or deleted on the server: its
    /// actions are off.
    pub(super) fn tab_gone(&self) -> bool {
        self.tabs.active_tab().gone
    }

    /// Push the tabs to the window, and save them shortly.
    pub(super) fn show_tabs(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let items: Vec<TabItem> = self
            .tabs
            .iter()
            .map(|t| TabItem {
                title: t.title.as_str().into(),
                kind: t.kind.code(),
                unread: t.unread,
                gone: t.gone,
            })
            .collect();
        ui.set_tabs(ModelRc::new(VecModel::from(items)));
        ui.set_active_tab(self.tabs.active() as i32);
        ui.set_tab_kind(self.tabs.active_kind().code());
        ui.set_tabs_reopenable(self.tabs.has_closed());
        ui.set_r_gone(self.tabs.active_tab().gone);
        if !self.tabs_timer.running() {
            self.tabs_timer
                .start(TimerMode::SingleShot, SAVE_AFTER, || {
                    with_app(|a| a.save_tabs());
                });
        }
    }

    /// Write the `tabs` setting (ids only), or clear it when "Reopen tabs
    /// at start" is off.
    pub(super) fn save_tabs(&self) {
        self.tabs_timer.stop();
        let value = if self.reopen_tabs {
            self.tabs
                .setting(|d| self.drafts.get(d).and_then(|draft| draft.row))
        } else {
            String::new()
        };
        self.db.set_setting("tabs", &value).ok();
    }

    /// A message's tab title and read state (none: not cached).
    fn message_title(&self, id: i64) -> Option<(String, bool)> {
        let (_, subject, seen) = self.db.subjects(&[id]).ok()?.into_iter().next()?;
        Some((title_or_none(subject), !seen))
    }

    /// A draft's tab title: its subject, or "New message".
    fn draft_title(&self, d: &Draft) -> String {
        match d.subject.trim() {
            "" => t!(invoke_tab_new_message),
            subject => subject.to_string(),
        }
    }

    /// The active draft tab's title follows its subject as it is typed.
    pub(super) fn retitle_draft_tab(&mut self) {
        let TabKind::Draft(d) = self.tabs.active_kind() else {
            return;
        };
        let Some(title) = self.drafts.get(d).map(|x| self.draft_title(x)) else {
            return;
        };
        if self.tabs.active_tab().title != title {
            self.tabs.active_tab_mut().title = title;
            self.show_tabs();
        }
    }

    /// The language changed: titles Rust made say so in it.
    pub(super) fn retitle_tabs(&mut self) {
        let ids: Vec<i64> = self
            .tabs
            .iter()
            .filter_map(|t| match t.kind {
                TabKind::Message(id) => Some(id),
                _ => None,
            })
            .collect();
        let found = self.db.subjects(&ids).unwrap_or_default();
        let drafts: Vec<(DraftId, String)> = self
            .tabs
            .iter()
            .filter_map(|t| match t.kind {
                TabKind::Draft(d) => Some((d, self.drafts.get(d).map(|x| self.draft_title(x))?)),
                _ => None,
            })
            .collect();
        for tab in self.tabs.iter_mut() {
            match tab.kind {
                TabKind::Message(id) => {
                    if let Some((_, subject, _)) = found.iter().find(|(m, _, _)| *m == id) {
                        tab.title = title_or_none(subject.clone());
                    }
                }
                TabKind::Draft(d) => {
                    if let Some((_, title)) = drafts.iter().find(|(x, _)| *x == d) {
                        tab.title = title.clone();
                    }
                }
                _ => {}
            }
        }
        self.show_tabs();
    }

    /// Open message `id` in a tab of its own, right of the active one; a
    /// tab it already has comes forward instead. `background` (a
    /// middle-click) leaves the tab shown as it is, and the message unread.
    pub(super) fn open_in_tab(&mut self, id: i64, background: bool) {
        if let Some(i) = self.tabs.find_message(id) {
            if !background {
                self.activate_tab(i);
            }
            return;
        }
        let Some((title, unread)) = self.message_title(id) else {
            return;
        };
        let mut tab = Tab::new(TabKind::Message(id), title);
        tab.unread = unread;
        let at = self.tabs.active() + 1;
        let i = self.tabs.insert(at, tab);
        if background {
            self.show_tabs();
        } else {
            self.activate_tab(i);
        }
    }

    /// The message an "Open in new tab / window" acts on: the open one, or
    /// the one row selected.
    pub(super) fn current_message(&self) -> Option<i64> {
        if let Some(open) = &self.open {
            return Some(open.msg.id);
        }
        if !self.in_mail_tab() || !self.lists_messages() {
            return None;
        }
        match self.rows.selection().ids()[..] {
            [id] => Some(id),
            _ => None,
        }
    }

    /// Shift+Enter and "Open in new tab": the list's current message.
    pub(super) fn open_current_in_tab(&mut self) {
        if !self.in_mail_tab() || self.ui().get_composing() || !self.lists_messages() {
            return;
        }
        let id = self
            .current_message()
            .or_else(|| self.rows.selection().cursor());
        if let Some(id) = id {
            self.open_in_tab(id, false);
        }
    }

    /// Show tab `i`: what the active tab shows is kept for when it comes
    /// back (a draft saved at once), and tab `i`'s message or draft shows.
    pub(super) fn activate_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.tabs.active() {
            return;
        }
        self.leave_tab();
        self.tabs.set_active(i);
        self.enter_tab();
        self.show_tabs();
    }

    /// The active tab hands over: its draft saved (as the autosave would),
    /// its view remembered, the read delay stopped.
    pub(super) fn leave_tab(&mut self) {
        let ui = self.ui();
        let composing = ui.get_composing();
        if composing {
            if let Some(id) = self.drafts.active_id()
                && self.save_draft(id)
            {
                self.autosave_timer.stop();
            }
            ui.set_c_suggestions(ModelRc::default());
            ui.set_c_confirm_discard(false);
            ui.set_composing(false);
        }
        let view = ViewState {
            scroll_y: ui.get_r_content_y(),
            plain: ui.get_r_plain(),
            original: ui.get_r_original(),
        };
        match self.tabs.active_kind() {
            TabKind::Mail => {
                self.tabs.mail = MailPane {
                    open: self.open.as_ref().map(|o| o.msg.id),
                    draft: if composing {
                        self.drafts.active_id()
                    } else {
                        None
                    },
                    view,
                };
            }
            TabKind::Message(_) => self.tabs.active_tab_mut().view = view,
            TabKind::Draft(_) | TabKind::Saved(_) => {}
        }
        self.read_wait.cancel();
        self.read_timer.stop();
    }

    /// Show what the active tab holds.
    pub(super) fn enter_tab(&mut self) {
        let ui = self.ui();
        match self.tabs.active_kind() {
            TabKind::Mail => {
                let pane = self.tabs.mail;
                match pane.open {
                    Some(id) => self.show_message(id, pane.view),
                    None => self.clear_open(),
                }
                match pane.draft.filter(|d| self.drafts.contains(*d)) {
                    Some(d) => {
                        self.drafts.show(d);
                        self.show_draft();
                    }
                    None => {
                        self.drafts.hide();
                        self.sync_selection_index();
                        ui.invoke_focus_list();
                    }
                }
            }
            TabKind::Message(id) => {
                let view = self.tabs.active_tab().view;
                self.show_message(id, view);
                // No longer cached (removed account, server removal): the
                // tab shows the moved-or-deleted warning, actions off.
                if self.open.as_ref().is_none_or(|o| o.msg.id != id) {
                    self.tabs.active_tab_mut().gone = true;
                    self.show_tabs();
                }
                ui.set_focus_page(true);
            }
            TabKind::Draft(d) => {
                self.drafts.show(d);
                self.show_draft();
            }
            TabKind::Saved(row) => self.load_saved_tab(row),
        }
    }

    /// No message is open (an empty reading pane).
    fn clear_open(&mut self) {
        self.open = None;
        self.reader.close();
        self.ui().set_has_message(false);
    }

    /// Open message `id` in the reader as a tab showed it (its scroll, the
    /// plain-text view and original colours). A message no longer cached
    /// leaves the reader empty.
    fn show_message(&mut self, id: i64, view: ViewState) {
        if !self.load_message(id) {
            return self.clear_open();
        }
        let ui = self.ui();
        ui.set_r_plain(view.plain);
        if view.original {
            ui.set_r_original(true);
            ui.invoke_r_show_original(true);
        }
        self.show_open();
        self.sync_selection_index();
        let y = view.scroll_y;
        ui.set_r_content_y(y);
        // Again once the page has its height.
        if y < 0.0 {
            Timer::single_shot(Duration::from_millis(50), move || {
                with_app(|a| {
                    if let Some(ui) = a.ui.upgrade() {
                        ui.set_r_content_y(y.max(ui.get_r_scroll_min()));
                    }
                });
            });
        }
    }

    /// A restored draft tab shown for the first time: its saved row loads.
    fn load_saved_tab(&mut self, row: i64) {
        match composer::load(&self.db, row) {
            Ok(Some((mut draft, missing))) => {
                self.prepare_draft(&mut draft);
                let id = self.drafts.add(draft);
                self.drafts.set_tabbed(id, true);
                self.tabs.active_tab_mut().kind = TabKind::Draft(id);
                self.drafts.show(id);
                self.show_draft();
                if !missing.is_empty() {
                    let names = missing.join(", ");
                    self.set_status(&t!(invoke_draft_left_out, names.into()), true);
                }
            }
            _ => {
                // Sent or discarded elsewhere since: the tab goes.
                self.set_status(&t!(invoke_draft_gone), true);
                self.drop_active_tab();
            }
        }
    }

    /// Draft `id`, in memory, in a tab of its own right of the active one;
    /// shown unless `background`.
    pub(super) fn open_draft_tab(&mut self, id: DraftId, background: bool) {
        // In a composer window already: that window comes forward.
        if self.composer_window(id).is_some() {
            return self.open_draft_window(id);
        }
        if let Some(i) = self.tabs.find(TabKind::Draft(id)) {
            if !background {
                self.activate_tab(i);
            }
            return;
        }
        let Some(title) = self.drafts.get(id).map(|d| self.draft_title(d)) else {
            return;
        };
        self.drafts.set_tabbed(id, true);
        let at = self.tabs.active() + 1;
        let i = self.tabs.insert(at, Tab::new(TabKind::Draft(id), title));
        if background {
            self.show_tabs();
        } else {
            self.activate_tab(i);
        }
    }

    /// Saved draft `row` in a new tab beside what shows (Undo send).
    pub(super) fn open_saved_in_tab(&mut self, row: i64) {
        if let Some(id) = self.drafts.find_row(row) {
            return self.open_draft_tab(id, false);
        }
        match composer::load(&self.db, row) {
            Ok(Some((mut draft, _))) => {
                self.prepare_draft(&mut draft);
                let id = self.drafts.add(draft);
                self.open_draft_tab(id, false);
            }
            Ok(None) => self.set_status(&t!(invoke_draft_gone), true),
            Err(e) => self.set_status(
                &t!(invoke_could_not_open_draft, format!("{e:#}").into()),
                true,
            ),
        }
    }

    /// The reading pane's draft makes room: one with something written is
    /// saved (here and on the server) and moves to a background tab of its
    /// own; an empty one goes. True if it moved to a tab.
    pub(super) fn pane_draft_aside(&mut self) -> bool {
        if !self.in_mail_tab() || !self.ui().get_composing() {
            return false;
        }
        let Some(id) = self.drafts.active_id() else {
            return false;
        };
        let saved = self.save_draft(id);
        if !self.drafts.get(id).is_some_and(Draft::keep) {
            self.drafts.remove(id);
            return false;
        }
        if saved {
            self.save_to_server_now(id);
        }
        self.drafts.hide();
        self.open_draft_tab(id, true);
        true
    }

    /// Close tab `i` (never the Mail tab). A draft tab's draft is saved,
    /// as Esc saves it, or dropped when nothing is written.
    pub(super) fn close_tab(&mut self, i: usize) {
        let Some(kind) = self.tabs.get(i).map(|t| t.kind) else {
            return;
        };
        match kind {
            TabKind::Mail => return,
            TabKind::Message(id) => self.tabs.remember(Reopen::Message(id)),
            TabKind::Saved(row) => self.tabs.remember(Reopen::Draft(row)),
            TabKind::Draft(d) => {
                let saved = self.save_draft(d);
                if self.drafts.get(d).is_some_and(Draft::keep) {
                    if saved {
                        self.save_to_server_now(d);
                        self.set_status(&t!(invoke_draft_saved), false);
                    }
                    if let Some(row) = self.drafts.get(d).and_then(|x| x.row) {
                        self.tabs.remember(Reopen::Draft(row));
                    }
                    self.drafts.set_tabbed(d, false);
                } else {
                    self.drafts.remove(d);
                }
            }
        }
        if i == self.tabs.active() {
            self.drop_active_tab();
        } else {
            self.tabs.remove(i);
            self.show_tabs();
        }
    }

    /// The active tab goes (closed, sent, discarded or its message moved):
    /// the tab to its left shows.
    pub(super) fn drop_active_tab(&mut self) {
        let ui = self.ui();
        ui.set_composing(false);
        ui.set_c_suggestions(ModelRc::default());
        if let TabKind::Draft(d) = self.tabs.active_kind() {
            self.drafts.set_tabbed(d, false);
            if self.drafts.active_id() == Some(d) {
                self.drafts.hide();
            }
        }
        self.read_wait.cancel();
        self.read_timer.stop();
        let i = self.tabs.active();
        self.tabs.remove(i);
        self.enter_tab();
        self.show_tabs();
    }

    /// Tabs of drafts that are gone (sent, discarded, their account
    /// removed) close.
    pub(super) fn prune_draft_tabs(&mut self) {
        let gone: Vec<usize> = (1..self.tabs.len())
            .rev()
            .filter(|&i| {
                matches!(self.tabs.get(i).map(|t| t.kind),
                    Some(TabKind::Draft(d)) if !self.drafts.contains(d))
            })
            .collect();
        if gone.is_empty() {
            return;
        }
        for i in gone {
            if i == self.tabs.active() {
                self.drop_active_tab();
            } else {
                self.tabs.remove(i);
            }
        }
        self.show_tabs();
    }

    /// The tab keys (`tab-key`): 1 next, 2 previous, 3 / 4 move the tab
    /// left / right, 5 close, 6 reopen, 10 + n tab n.
    pub(super) fn tab_key(&mut self, key: i32) {
        match key {
            1 => self.activate_tab(self.tabs.step(1)),
            2 => self.activate_tab(self.tabs.step(-1)),
            3 | 4 => {
                let i = self.tabs.active();
                let to = if key == 3 { i.saturating_sub(1) } else { i + 1 };
                if self.tabs.move_tab(i, to) {
                    self.show_tabs();
                }
            }
            5 => {
                let i = self.tabs.active();
                self.close_tab(i);
            }
            6 => self.reopen_closed(),
            n if (11..=19).contains(&n) => {
                if let Some(i) = self.tabs.number((n - 10) as usize) {
                    self.activate_tab(i);
                }
            }
            _ => {}
        }
    }

    /// Drag a tab from `from` to `to` in the strip (the Mail tab stays).
    pub(super) fn move_tab(&mut self, from: usize, to: usize) {
        if self.tabs.move_tab(from, to) {
            self.show_tabs();
        }
    }

    /// A tab menu command (`tab-command`) on tab `i`.
    pub(super) fn tab_command(&mut self, i: usize, command: i32) {
        match command {
            0 => self.close_tab(i),
            1 | 2 => {
                // From the end, so the places before stay put.
                let keep = self.tabs.get(i).map(|t| t.kind);
                for j in (1..self.tabs.len()).rev() {
                    if j != i && (command == 1 || j > i) {
                        self.close_tab(j);
                    }
                }
                if let Some(k) = keep.and_then(|k| self.tabs.find(k)) {
                    self.activate_tab(k);
                }
            }
            3 => match self.tabs.get(i).map(|t| t.kind) {
                Some(TabKind::Message(id)) => {
                    self.open_window(id);
                    if let Some(j) = self.tabs.find_message(id) {
                        self.close_tab(j);
                    }
                }
                // The draft leaves its tab for a composer window.
                Some(TabKind::Draft(id)) => self.open_draft_window(id),
                Some(TabKind::Saved(row)) => {
                    self.open_saved_in_window(row);
                    if let Some(j) = self.tabs.find(TabKind::Saved(row)) {
                        if j == self.tabs.active() {
                            self.drop_active_tab();
                        } else {
                            self.tabs.remove(j);
                            self.show_tabs();
                        }
                    }
                }
                _ => {}
            },
            4 => self.reopen_closed(),
            _ => {}
        }
    }

    /// Ctrl+Shift+T: the last closed tab opens again (one whose message or
    /// draft is gone is skipped).
    pub(super) fn reopen_closed(&mut self) {
        while let Some(reopen) = self.tabs.take_closed() {
            match reopen {
                Reopen::Message(id) => {
                    if self.message_title(id).is_some() {
                        return self.open_in_tab(id, false);
                    }
                }
                Reopen::Draft(row) => {
                    if self.db.is_saved_draft(row).unwrap_or(false) {
                        return self.open_saved_in_tab(row);
                    }
                }
            }
        }
        self.show_tabs();
    }

    /// Ctrl+O and "Open in new window": the current message in a window
    /// of its own; from its tab, the tab closes.
    pub(super) fn open_current_window(&mut self) {
        if self.ui().get_composing() {
            // The draft showing, in a composer window of its own.
            if let Some(id) = self.drafts.active_id() {
                self.open_draft_window(id);
            }
            return;
        }
        // A draft row in the Drafts folder, in a composer window.
        if self.view == View::Drafts && self.in_mail_tab() {
            if let Some(row) = self.current_row() {
                self.open_saved_in_window(row);
            }
            return;
        }
        let Some(id) = self.current_message() else {
            return;
        };
        self.open_window(id);
        if let TabKind::Message(m) = self.tabs.active_kind()
            && m == id
        {
            let i = self.tabs.active();
            self.close_tab(i);
        }
    }

    /// To the Mail tab (search, the folders, the Inbox).
    pub(super) fn go_to_mail_tab(&mut self) {
        if !self.in_mail_tab() {
            self.activate_tab(0);
        }
    }

    /// Messages left the list (moved, deleted, junk): their tabs close.
    /// Returns the closed tabs' places and messages, for Undo.
    pub(super) fn close_message_tabs(&mut self, ids: &HashSet<i64>) -> Vec<(usize, i64)> {
        let mut closed = Vec::new();
        for i in (1..self.tabs.len()).rev() {
            if let Some(TabKind::Message(id)) = self.tabs.get(i).map(|t| t.kind)
                && ids.contains(&id)
            {
                closed.push((i, id));
                if i == self.tabs.active() {
                    // The reader's message went with it.
                    self.open = None;
                    self.drop_active_tab();
                } else {
                    self.tabs.remove(i);
                }
            }
        }
        if self.tabs.mail.open.is_some_and(|id| ids.contains(&id)) {
            self.tabs.mail.open = None;
        }
        if !closed.is_empty() {
            closed.reverse();
            self.show_tabs();
        }
        closed
    }

    /// Undo of a move: its tabs come back at their places (the one it was
    /// done from shows again).
    pub(super) fn reopen_message_tabs(&mut self, tabs: &[(usize, i64)], activate: Option<i64>) {
        for &(at, id) in tabs {
            if self.tabs.find_message(id).is_some() {
                continue;
            }
            if let Some((title, unread)) = self.message_title(id) {
                let mut tab = Tab::new(TabKind::Message(id), title);
                tab.unread = unread;
                self.tabs.insert(at, tab);
            }
        }
        match activate.and_then(|id| self.tabs.find_message(id)) {
            Some(i) => self.activate_tab(i),
            None => self.show_tabs(),
        }
    }

    /// Messages changed in the cache (a sync, a flag): tab titles follow
    /// the read state, and a tab whose message is no longer cached is
    /// marked as moved or deleted on the server.
    pub(super) fn refresh_message_tabs(&mut self) {
        let ids: Vec<i64> = self
            .tabs
            .iter()
            .filter_map(|t| match t.kind {
                TabKind::Message(id) => Some(id),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let Ok(found) = self.db.subjects(&ids) else {
            return;
        };
        let mut changed = false;
        for tab in self.tabs.iter_mut() {
            let TabKind::Message(id) = tab.kind else {
                continue;
            };
            match found.iter().find(|(m, _, _)| *m == id) {
                Some((_, _, seen)) => {
                    if tab.unread == *seen || tab.gone {
                        tab.unread = !*seen;
                        tab.gone = false;
                        changed = true;
                    }
                }
                None => {
                    if !tab.gone {
                        tab.gone = true;
                        changed = true;
                    }
                }
            }
        }
        // The active tab's message came back (re-download) or its reader
        // was emptied: show it again, or keep the warning.
        if let TabKind::Message(id) = self.tabs.active_kind()
            && !self.tabs.active_tab().gone
            && self.open.as_ref().is_none_or(|o| o.msg.id != id)
        {
            let view = self.tabs.active_tab().view;
            self.show_message(id, view);
        }
        if changed {
            self.show_tabs();
        }
    }

    /// At start, after the first frame: the tabs saved at the last close
    /// come back (titles in one query; bodies load as a tab is shown). The
    /// saved active tab shows only if it is a message or draft tab.
    pub(super) fn restore_tabs(&mut self, saved: &str) {
        let (list, active) = parse_setting(saved);
        if list.is_empty() {
            return;
        }
        let messages: Vec<i64> = list
            .iter()
            .filter_map(|r| match r {
                Reopen::Message(id) => Some(*id),
                Reopen::Draft(_) => None,
            })
            .collect();
        let found = self.db.subjects(&messages).unwrap_or_default();
        let mut wanted = None;
        for (n, reopen) in list.iter().enumerate() {
            let tab = match *reopen {
                Reopen::Message(id) => {
                    found
                        .iter()
                        .find(|(m, _, _)| *m == id)
                        .map(|(_, subject, seen)| {
                            let mut tab =
                                Tab::new(TabKind::Message(id), title_or_none(subject.clone()));
                            tab.unread = !seen;
                            tab
                        })
                }
                Reopen::Draft(row) => match self.db.draft_header(row) {
                    Ok(Some(h)) if self.drafts.find_row(row).is_none() => {
                        let title = match h.subject.trim() {
                            "" => t!(invoke_tab_new_message),
                            s => s.to_string(),
                        };
                        Some(Tab::new(TabKind::Saved(row), title))
                    }
                    _ => None,
                },
            };
            if let Some(tab) = tab {
                let at = self.tabs.len();
                let i = self.tabs.insert(at, tab);
                if n + 1 == active {
                    wanted = Some(i);
                }
            }
        }
        match wanted {
            Some(i) => self.activate_tab(i),
            None => self.show_tabs(),
        }
    }
}

/// A subject as a tab title ("(no subject)" for none).
fn title_or_none(subject: String) -> String {
    if subject.trim().is_empty() {
        t!(invoke_no_subject)
    } else {
        subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(kinds: &[TabKind]) -> Tabs {
        let mut t = Tabs::default();
        for &k in kinds {
            let n = t.len();
            t.insert(n, Tab::new(k, String::new()));
        }
        t
    }

    #[test]
    fn close_tab_activates_left_neighbour() {
        let mut t = tabs(&[
            TabKind::Message(1),
            TabKind::Message(2),
            TabKind::Message(3),
        ]);
        t.set_active(2);
        assert_eq!(t.remove(2).unwrap().kind, TabKind::Message(2));
        assert_eq!(t.active_kind(), TabKind::Message(1));
        // A tab left of the active one: the active tab stays.
        t.set_active(2);
        t.remove(1);
        assert_eq!(t.active_kind(), TabKind::Message(3));
        // The Mail tab never closes.
        assert!(t.remove(0).is_none());
        t.remove(1);
        assert_eq!(t.active_kind(), TabKind::Mail);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn reorder_keeps_mail_tab_first() {
        let mut t = tabs(&[
            TabKind::Message(1),
            TabKind::Message(2),
            TabKind::Message(3),
        ]);
        t.set_active(1);
        assert!(!t.move_tab(0, 2));
        assert!(!t.move_tab(1, 0), "never before the Mail tab");
        assert_eq!(t.get(0).unwrap().kind, TabKind::Mail);
        assert_eq!(t.get(1).unwrap().kind, TabKind::Message(1));
        assert!(t.move_tab(1, 3));
        let order: Vec<TabKind> = t.iter().map(|x| x.kind).collect();
        assert_eq!(
            order,
            [
                TabKind::Mail,
                TabKind::Message(2),
                TabKind::Message(3),
                TabKind::Message(1)
            ]
        );
        // The active tab is still message 1.
        assert_eq!(t.active_kind(), TabKind::Message(1));
        // Inserting before the active tab keeps it active.
        t.insert(1, Tab::new(TabKind::Message(9), String::new()));
        assert_eq!(t.active_kind(), TabKind::Message(1));
    }

    #[test]
    fn steps_and_numbers() {
        let t = tabs(&[TabKind::Message(1), TabKind::Message(2)]);
        assert_eq!(t.step(1), 1);
        assert_eq!(t.step(-1), 2);
        assert_eq!(t.number(1), Some(0));
        assert_eq!(t.number(3), Some(2));
        assert_eq!(t.number(4), None);
        assert_eq!(t.number(9), Some(2));
    }

    #[test]
    fn closed_tabs_stack_up_to_ten() {
        let mut t = Tabs::default();
        for i in 1..=12 {
            t.remember(Reopen::Message(i));
        }
        t.remember(Reopen::Message(5));
        assert_eq!(t.take_closed(), Some(Reopen::Message(5)));
        assert_eq!(t.take_closed(), Some(Reopen::Message(12)));
        let mut n = 2;
        while t.take_closed().is_some() {
            n += 1;
        }
        assert_eq!(n, 10);
    }

    #[test]
    fn tabs_setting_holds_ids_only() {
        let mut t = tabs(&[
            TabKind::Message(123),
            TabKind::Draft(DraftId::for_test(1)),
            TabKind::Draft(DraftId::for_test(2)),
            TabKind::Saved(7),
        ]);
        t.get_mut(1).unwrap().title = "Lunch with Ana <ana@example.org>".into();
        t.set_active(4);
        // Draft 2 has no saved row yet: left out, and the index follows.
        let s = t.setting(|d| (d == DraftId::for_test(1)).then_some(45));
        assert_eq!(s, "m123,d45,d7;a=3");
        assert!(!s.contains("Lunch") && !s.contains('@'));
        assert_eq!(
            parse_setting(&s),
            (
                vec![Reopen::Message(123), Reopen::Draft(45), Reopen::Draft(7)],
                3
            )
        );
        // Only the Mail tab: nothing to keep.
        assert_eq!(Tabs::default().setting(|_| None), "");
    }

    #[test]
    fn restore_skips_broken_tokens_and_caps_at_20() {
        let many: Vec<String> = (1..=30).map(|i| format!("m{i}")).collect();
        let (list, active) = parse_setting(&format!("{},x9,m-2,d,m3;a=30", many.join(",")));
        assert_eq!(list.len(), RESTORE_MAX);
        assert_eq!(list[0], Reopen::Message(1));
        // The active one was cut: the Mail tab.
        assert_eq!(active, 0);
        assert_eq!(
            parse_setting("m5,junk,d6;a=3"),
            (vec![Reopen::Message(5), Reopen::Draft(6)], 2)
        );
        assert_eq!(parse_setting(""), (Vec::new(), 0));
        assert_eq!(parse_setting("m1;a=zz"), (vec![Reopen::Message(1)], 0));
        assert_eq!(parse_setting("é5,m2"), (vec![Reopen::Message(2)], 0));
    }
}
