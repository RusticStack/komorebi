//! The main window's tabs and message windows (docs/GPUI-MIGRATION.md phase
//! 5; the Slint app's `src/app/tabs.rs` and `src/app/windows.rs`): what a
//! tab shows, opening, closing, reopening and moving tabs, the `tabs`
//! setting, the "moved or deleted" mark, and the windows of their own.
//!
//! - A message tab has its own [`MessageView`] (its own reader); a draft tab
//!   its own [`ComposerView`]. Both are made when the tab opens (a restored
//!   tab: when first shown) and dropped when it closes.
//! - What a tab or a message window asks (Reply, Archive, a download) goes
//!   to the mail view and the hub, so Archive in a tab is the list's Archive
//!   with its Undo; the tab closes and Ctrl+Shift+T brings it back.
//! - A message window acts on the main window through its handle, after the
//!   event (the main window may be busy drawing when the event comes).
//! - A draft can have a window of its own too (a draft tab's Open in new
//!   window): its composer saves as the window closes, as the app quits,
//!   and an Undo of its send opens it in a window again.
//! - Every draft is saved when the main window closes or the app quits
//!   (the pane's, the tabs', the windows'), and a tab being left saves its
//!   draft at once (the autosave would wait).

use std::collections::HashMap;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Styled as _, Subscription, Window, WindowHandle, div,
};
use komorebi::tabs::{DraftId, Reopen, Tab, TabKind};

use super::MainWindow;
use crate::tabs::{
    CloseTab, GoToTab, MoveTabLeft, MoveTabRight, OpenInTab, OpenInWindow, ReopenTab,
};
use crate::views::composer::{ComposerEvent, ComposerView, Draft};
use crate::views::mail::triage::Quick;
use crate::views::reader::ReaderEvent;
use crate::views::settings::Section;
use crate::views::tabs::message::{MessageEvent, MessageView};
use crate::views::tabs::strip::{self, Handlers, TabCommand};
use crate::views::tabs::{self as tab_views, text};
use crate::views::windows::popout::{self, Popout, PopoutKind};

/// The views behind the tabs, by tab kind.
#[derive(Default)]
pub(super) struct TabViews {
    messages: HashMap<i64, (Entity<MessageView>, Subscription)>,
    /// A draft tab's composer, what it asks, and its title following the
    /// subject.
    drafts: HashMap<DraftId, (Entity<ComposerView>, [Subscription; 2])>,
    /// The windows of their own: message id, the window, its view.
    popouts: Vec<(i64, WindowHandle<Popout>, Entity<MessageView>, Subscription)>,
    /// The next draft tab's number (this session).
    next_draft: u64,
    /// Message tabs closed by a move in its undo window. Undo reopens them
    /// at their places.
    moved: Vec<Moved>,
    /// Message windows closed by a move from the list in its undo window:
    /// the move's number, the message. Undo does not reopen them; the
    /// status line says so (the Slint app's `invoke_undo_window`).
    moved_windows: Vec<(u64, i64)>,
    /// The composer windows (a draft in a window of its own).
    draft_windows: Vec<DraftWindow>,
    /// Saved drafts sent from a composer window, while their send can be
    /// undone, with the composer that sent them: Undo of that send brings
    /// such a draft back in a window, not a tab (the Slint app's
    /// `send_from_a_composer_window_once_and_undo_reopens_it_there`). Only
    /// an Undo whose send came from that very composer matches, so an entry
    /// left by a send that went out never sends a later draft of the same
    /// row (a reused rowid, a send from a tab) to a window.
    window_sends: HashMap<i64, gpui::EntityId>,
    /// How many windows of their own were opened: each opens a step down
    /// and right of the last (the cascade).
    opened_windows: usize,
    /// The size the last window of its own had when it closed (this
    /// session): the next one opens at it.
    popout_size: std::rc::Rc<std::cell::Cell<Option<(f32, f32)>>>,
    /// The mail view's flag count the message views last followed
    /// (`follow_flags`).
    flags_seen: u64,
}

/// A composer window: its number (counted with the draft tabs), the
/// window, its composer, what it asks and its title following the subject.
struct DraftWindow {
    n: u64,
    handle: WindowHandle<Popout>,
    view: Entity<ComposerView>,
    title: String,
    _subscriptions: [Subscription; 2],
}

/// A message tab a move closed.
#[derive(Clone, Copy, Debug)]
struct Moved {
    /// The move's number (`Held::seq`).
    seq: u64,
    message: i64,
    /// The tab's place.
    at: usize,
    /// The move was made from this tab (its toolbar): Undo shows it again.
    /// A move from the list leaves the shown tab as it is.
    from_tab: bool,
}

impl MainWindow {
    // ------------------------------------------------------------ opening

    /// Message `id` in a tab: the one it already has, or a new one after
    /// the current tab (shown unless `background`).
    pub fn open_in_tab(
        &mut self,
        id: i64,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let i = match self.tabs.find_message(id) {
            Some(i) => i,
            None => {
                let Some((title, unread)) = tab_views::message_title(&db, id) else {
                    return;
                };
                let mut tab = Tab::new(TabKind::Message(id), title);
                tab.unread = unread;
                self.tabs.insert(self.tabs.active() + 1, tab)
            }
        };
        if !background {
            self.select_tab(i, window, cx);
        }
        self.tabs_changed(cx);
    }

    /// A draft in a tab of its own, shown: a reply, a forward, a saved draft.
    pub fn open_draft_tab(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        self.open_draft_tab_at(draft, false, window, cx);
    }

    /// A draft in a tab of its own; in `background` the shown tab stays (a
    /// pane draft set aside, "Draft kept in a tab").
    pub fn open_draft_tab_at(
        &mut self,
        draft: Draft,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Draft::Saved(row) = draft {
            // Open already in a tab or a window: that one comes forward.
            if self.raise_draft(row, window, cx) {
                return;
            }
        }
        let n = self.tab_views.next_draft;
        self.tab_views.next_draft += 1;
        let title = match &draft {
            Draft::Saved(row) => self
                .db
                .as_ref()
                .and_then(|db| db.draft(*row).ok().flatten())
                .map(|d| text::draft_title(&d.subject))
                .unwrap_or_else(|| text::draft_title("")),
            _ => text::draft_title(""),
        };
        let kind = TabKind::Draft(DraftId::new(n));
        let i = self
            .tabs
            .insert(self.tabs.active() + 1, Tab::new(kind, title));
        if let Some(composer) = self.draft_view(DraftId::new(n), window, cx) {
            composer.update(cx, |c, cx| c.open(Some(draft), window, cx));
        }
        if !background {
            self.select_tab(i, window, cx);
        }
        self.tabs_changed(cx);
    }

    /// Saved draft `row` from the mail view: a pane draft set aside, or an
    /// undone send (`sent_by`: the composer that sent it). Undo of a send
    /// from a composer window brings it back in a window; anything else
    /// opens in a tab.
    pub fn open_undone_or_aside(
        &mut self,
        row: i64,
        background: bool,
        sent_by: Option<gpui::EntityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = Draft::Saved(row);
        let from_window =
            sent_by.is_some() && self.tab_views.window_sends.get(&row).copied() == sent_by;
        if from_window {
            self.tab_views.window_sends.remove(&row);
            if self.raise_draft(row, window, cx) {
                return;
            }
            return self.open_draft_window(draft, window, cx);
        }
        self.open_draft_tab_at(draft, background, window, cx);
    }

    /// A server draft (Drafts folder) opened for editing in a tab of its
    /// own: saving it replaces that message.
    fn edit_server_draft_tab(&mut self, message: i64, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tab_views.next_draft;
        self.tab_views.next_draft += 1;
        let kind = TabKind::Draft(DraftId::new(n));
        let i = self.tabs.insert(
            self.tabs.active() + 1,
            Tab::new(kind, text::draft_title("")),
        );
        if let Some(composer) = self.draft_view(DraftId::new(n), window, cx) {
            composer.update(cx, |c, cx| c.edit_server_draft(message, window, cx));
        }
        self.select_tab(i, window, cx);
        self.tabs_changed(cx);
    }

    /// Message `id` in a window of its own (the one it already has, brought
    /// to the front).
    pub fn open_in_window(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        if let Some((_, handle, _, _)) = self.tab_views.popouts.iter().find(|p| p.0 == id) {
            let _ = handle.update(cx, |_, w, _| w.activate_window());
            return;
        }
        let Some((title, _)) = tab_views::message_title(&db, id) else {
            return;
        };
        let options = self.popout_options(&text::window_title(&title), cx);
        let mut made = None;
        let this = cx.entity().downgrade();
        let size = self.tab_views.popout_size.clone();
        let opened = cx.open_window(options, |w, cx| {
            let view = cx.new(|cx| MessageView::new(db, id, true, w, cx));
            let focus = view.focus_handle(cx);
            made = Some(view.clone());
            // Closed by its own button: its size kept for the next one, and
            // forgotten, so it opens again.
            w.on_window_should_close(cx, move |w, cx| {
                size.set(Some(popout::size_of(w)));
                let _ = this.update(cx, |this, _| this.tab_views.popouts.retain(|p| p.0 != id));
                true
            });
            cx.new(|cx| Popout::new(PopoutKind::Message(id), view.into(), focus, w, cx))
        });
        let (Ok(handle), Some(view)) = (opened, made) else {
            return;
        };
        let main = window.window_handle();
        let subscription = cx.subscribe(&view, move |_, _, event: &MessageEvent, cx| {
            let event = event.clone();
            cx.spawn(async move |this, cx| {
                let _ = cx.update_window(main, |_, window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.on_popout(id, handle, &event, window, cx);
                    });
                });
            })
            .detach();
        });
        self.tab_views
            .popouts
            .push((id, handle, view, subscription));
    }

    /// The options of the next window of its own: the size last used, a
    /// step down and right of the one before.
    fn popout_options(&mut self, title: &str, cx: &mut Context<Self>) -> gpui::WindowOptions {
        let n = self.tab_views.opened_windows;
        self.tab_views.opened_windows += 1;
        let size = self
            .tab_views
            .popout_size
            .get()
            .unwrap_or(popout::POPOUT_SIZE);
        popout::options_at(title, size, n, cx)
    }

    // -------------------------------------------------- composer windows

    /// `draft` in a composer window of its own (the Slint app's
    /// `open_draft_window`): its own composer, the app-drawn title bar, the
    /// OS title following the subject. Closing the window (Esc, Close, the
    /// title bar) saves the draft as Esc does in the pane; a draft whose
    /// save failed keeps its window.
    pub fn open_draft_window(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        if let Draft::Saved(row) = draft
            && self.raise_draft(row, window, cx)
        {
            return;
        }
        let n = self.tab_views.next_draft;
        self.tab_views.next_draft += 1;
        let title = text::window_title(&text::draft_title(""));
        let options = self.popout_options(&title, cx);
        let mut made = None;
        let opened = cx.open_window(options, |w, cx| {
            let view = cx.new(|cx| {
                let mut c = ComposerView::new(db, w, cx);
                c.set_own_place();
                c.set_windowed();
                c
            });
            let focus = view.focus_handle(cx);
            let popout =
                cx.new(|cx| Popout::new(PopoutKind::Draft(n), view.clone().into(), focus, w, cx));
            // After the window's own focus: the draft focuses To or the body.
            view.update(cx, |c, cx| c.open(Some(draft), w, cx));
            // The title bar's Close and Alt+F4 close it as Esc does; the
            // window goes when the composer says Closed.
            let closing = view.clone();
            w.on_window_should_close(cx, move |w, cx| {
                closing.update(cx, |c, cx| c.close(w, cx));
                false
            });
            made = Some(view);
            popout
        });
        let (Ok(handle), Some(view)) = (opened, made) else {
            return;
        };
        if !view.read(cx).composing() {
            // Nothing to show (the draft is gone, or still downloading): the
            // composer said why on the status line.
            let _ = handle.update(cx, |_, w, _| w.remove_window());
            return;
        }
        let main = window.window_handle();
        let events = cx.subscribe(&view, move |_, _, event: &ComposerEvent, cx| {
            let event = event.clone();
            cx.spawn(async move |this, cx| {
                let _ = cx.update_window(main, |_, window, cx| {
                    let _ = this.update(cx, |this, cx| this.on_draft_window(n, &event, window, cx));
                });
            })
            .detach();
        });
        let retitle = cx.observe(&view, move |this, view, cx| {
            this.retitle_draft_window(n, &view, cx)
        });
        self.tab_views.draft_windows.push(DraftWindow {
            n,
            handle,
            view: view.clone(),
            title,
            _subscriptions: [events, retitle],
        });
        self.retitle_draft_window(n, &view, cx);
    }

    /// The saved drafts shown in composer windows (tests and the shell).
    pub fn draft_windows(&self, cx: &gpui::App) -> Vec<Option<i64>> {
        self.tab_views
            .draft_windows
            .iter()
            .map(|w| w.view.read(cx).drafts.active().and_then(|d| d.row))
            .collect()
    }

    /// Composer window number `n`'s composer (tests).
    #[cfg(test)]
    pub fn draft_window_composer(
        &self,
        i: usize,
    ) -> Option<(gpui::AnyWindowHandle, Entity<ComposerView>)> {
        self.tab_views
            .draft_windows
            .get(i)
            .map(|w| (w.handle.into(), w.view.clone()))
    }

    /// Draft tab `i` in a composer window ("Open in new window", Ctrl+O):
    /// its draft is saved and opens in the window; the tab goes (it is not
    /// offered by Reopen: the draft is open). One that could not be saved
    /// stays in its tab with the error.
    pub fn draft_tab_to_window(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(TabKind::Draft(d)) = self.tabs.get(i).map(|t| t.kind) else {
            return;
        };
        let Some(view) = self.tab_views.drafts.get(&d).map(|v| v.0.clone()) else {
            return;
        };
        let (row, open) = view.update(cx, |c, cx| {
            let row = c.pop_out(cx);
            (row, c.is_open())
        });
        if open {
            return;
        }
        self.tab_views.drafts.remove(&d);
        let was_active = i == self.tabs.active();
        self.tabs.remove(i);
        if was_active {
            let now = self.tabs.active();
            self.select_tab(now, window, cx);
        }
        self.tabs_changed(cx);
        self.open_draft_window(row.map_or(Draft::New, Draft::Saved), window, cx);
    }

    /// The reading pane's draft in a composer window (Ctrl+O in the pane's
    /// composer), as a draft tab's.
    fn pane_draft_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(composer) = self.mail.as_ref().map(|m| m.read(cx).composer().clone()) else {
            return false;
        };
        let focused = composer
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if !focused || !composer.read(cx).is_open() {
            return false;
        }
        self.pane_draft_out(window, cx);
        true
    }

    /// The reading pane's draft goes to a composer window (the keys, or
    /// the composer's More menu).
    pub(super) fn pane_draft_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composer) = self.mail.as_ref().map(|m| m.read(cx).composer().clone()) else {
            return;
        };
        if !composer.read(cx).is_open() {
            return;
        }
        let (row, open) = composer.update(cx, |c, cx| {
            let row = c.pop_out(cx);
            (row, c.is_open())
        });
        if !open {
            self.open_draft_window(row.map_or(Draft::New, Draft::Saved), window, cx);
        }
    }

    /// Saved draft `row` is open in a draft tab or a composer window: that
    /// one is shown (the Slint app's "comes forward once"). False if none.
    fn raise_draft(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let holds = |view: &Entity<ComposerView>, cx: &Context<Self>| {
            let c = view.read(cx);
            c.composing() && c.drafts.find_row(row).is_some()
        };
        if let Some(w) = self
            .tab_views
            .draft_windows
            .iter()
            .find(|w| holds(&w.view, cx))
        {
            let _ = w.handle.update(cx, |_, w, _| w.activate_window());
            return true;
        }
        let tab = self
            .tab_views
            .drafts
            .iter()
            .find(|(_, (view, _))| holds(view, cx))
            .and_then(|(d, _)| self.tabs.find(TabKind::Draft(*d)));
        if let Some(i) = tab {
            self.select_tab(i, window, cx);
            return true;
        }
        false
    }

    /// Composer window `n`'s OS title follows its subject.
    fn retitle_draft_window(
        &mut self,
        n: u64,
        view: &Entity<ComposerView>,
        cx: &mut Context<Self>,
    ) {
        let Some(subject) = view.read(cx).drafts.active().map(|d| d.subject.clone()) else {
            return;
        };
        let title = text::window_title(&text::draft_title(&subject));
        let Some(w) = self.tab_views.draft_windows.iter_mut().find(|w| w.n == n) else {
            return;
        };
        if w.title != title {
            w.title.clone_from(&title);
            let _ = w.handle.update(cx, |_, w, _| w.set_window_title(&title));
        }
    }

    /// What composer window `n`'s composer asks: as a draft tab's, and the
    /// window goes when it closes.
    fn on_draft_window(
        &mut self,
        n: u64,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hub) = &self.hub {
            hub.update(cx, |hub, cx| hub.on_composer(event, cx));
        }
        match event {
            ComposerEvent::Closed => self.close_draft_window(n, cx),
            ComposerEvent::DraftsChanged { .. } | ComposerEvent::Unsent { .. } => {
                self.reload_drafts(window, cx)
            }
            ComposerEvent::Sending { row, .. } => {
                let pending = self
                    .mail
                    .as_ref()
                    .map(|m| m.read(cx).composer().read(cx).pending_sends())
                    .unwrap_or_default();
                self.tab_views
                    .window_sends
                    .retain(|r, _| pending.contains(r));
                if let Some(w) = self.tab_views.draft_windows.iter().find(|w| w.n == n) {
                    self.tab_views.window_sends.insert(*row, w.view.entity_id());
                }
            }
            ComposerEvent::ManageSignatures => {
                window.activate_window();
                self.show_section(Section::Signatures, window, cx);
            }
            ComposerEvent::Raise => {
                if let Some(w) = self.tab_views.draft_windows.iter().find(|w| w.n == n) {
                    let _ = w.handle.update(cx, |_, w, _| w.activate_window());
                }
            }
            _ => {}
        }
    }

    /// Composer window `n` goes (its draft saved or sent already).
    fn close_draft_window(&mut self, n: u64, cx: &mut Context<Self>) {
        let Some(at) = self.tab_views.draft_windows.iter().position(|w| w.n == n) else {
            return;
        };
        let w = self.tab_views.draft_windows.remove(at);
        let size = self.tab_views.popout_size.clone();
        let _ = w
            .handle
            .update(cx, |_, w, _| popout::close_keeping_size(w, &size));
    }

    /// A saved draft came or went: the Drafts count and the rows follow.
    fn reload_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mail) = &self.mail {
            mail.update(cx, |m, cx| {
                m.act(window, cx, |m| {
                    m.load_folders();
                    m.reload_rows();
                })
            });
        }
        self.tabs_changed(cx);
    }

    /// Every draft is saved: the pane's, the tabs', the windows' (the
    /// window closes, the app quits; the Slint app's `save_all`).
    pub fn save_drafts(&mut self, cx: &mut Context<Self>) {
        if let Some(mail) = &self.mail {
            let composer = mail.read(cx).composer().clone();
            composer.update(cx, |c, cx| c.save_all(cx));
        }
        for (view, _) in self.tab_views.drafts.values() {
            view.update(cx, |c, cx| c.save_all(cx));
        }
        for d in &self.tab_views.draft_windows {
            d.view.update(cx, |c, cx| c.save_all(cx));
        }
    }

    /// The composer windows open (their numbers).
    #[cfg(test)]
    pub(super) fn draft_window_numbers(&self) -> Vec<u64> {
        self.tab_views.draft_windows.iter().map(|d| d.n).collect()
    }

    /// Composer window `n`: its window and its composer.
    #[cfg(test)]
    pub(super) fn draft_window(
        &self,
        n: u64,
    ) -> Option<(gpui::AnyWindowHandle, Entity<ComposerView>)> {
        self.tab_views
            .draft_windows
            .iter()
            .find(|d| d.n == n)
            .map(|d| (d.handle.into(), d.view.clone()))
    }

    /// Message tab `id`'s reader, when the tab has its view.
    #[cfg(test)]
    pub(super) fn message_reader(&self, id: i64, cx: &gpui::App) -> Option<gpui::EntityId> {
        self.tab_views
            .messages
            .get(&id)
            .map(|(m, _)| m.read(cx).reader().entity_id())
    }

    /// The size the next window of its own opens at, once one has closed.
    #[cfg(test)]
    pub(super) fn popout_size(&self) -> Option<(f32, f32)> {
        self.tab_views.popout_size.get()
    }

    /// The message windows' handles.
    #[cfg(test)]
    pub(super) fn popout_handles(&self) -> Vec<gpui::AnyWindowHandle> {
        self.tab_views.popouts.iter().map(|p| p.1.into()).collect()
    }

    /// The language changed: the OS titles of the windows of their own
    /// (their content redraws by itself).
    pub(super) fn retitle_popouts(&mut self, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        for (id, handle, _, _) in &self.tab_views.popouts {
            if let Some((subject, _)) = tab_views::message_title(&db, *id) {
                let title = text::window_title(&subject);
                let _ = handle.update(cx, |_, w, _| w.set_window_title(&title));
            }
        }
        for d in &self.tab_views.draft_windows {
            d.view.update(cx, |_, cx| cx.notify());
        }
    }

    /// Draft tab `draft`'s composer, when it has one.
    #[cfg(test)]
    pub(crate) fn draft_composer(&self, draft: DraftId) -> Option<Entity<ComposerView>> {
        self.tab_views.drafts.get(&draft).map(|d| d.0.clone())
    }

    /// Message `id`'s tab's view, when it has one.
    #[cfg(test)]
    pub(super) fn tab_message_view(&self, id: i64) -> Option<Entity<MessageView>> {
        self.tab_views.messages.get(&id).map(|m| m.0.clone())
    }

    /// Message `id`'s window's view, when it has one.
    #[cfg(test)]
    pub(super) fn popout_view(&self, id: i64) -> Option<Entity<MessageView>> {
        self.tab_views
            .popouts
            .iter()
            .find(|p| p.0 == id)
            .map(|p| p.2.clone())
    }

    /// The message windows open (message ids).
    pub fn popouts(&self) -> Vec<i64> {
        self.tab_views.popouts.iter().map(|p| p.0).collect()
    }

    // ---------------------------------------------------------- the views

    fn message_view(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<MessageView>> {
        if let Some((view, _)) = self.tab_views.messages.get(&id) {
            return Some(view.clone());
        }
        let db = self.db.clone()?;
        let view = cx.new(|cx| MessageView::new(db, id, false, window, cx));
        let subscription = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            this.on_message_tab(id, event, window, cx);
        });
        self.tab_views
            .messages
            .insert(id, (view.clone(), subscription));
        Some(view)
    }

    fn draft_view(
        &mut self,
        draft: DraftId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ComposerView>> {
        if let Some((view, _)) = self.tab_views.drafts.get(&draft) {
            return Some(view.clone());
        }
        let db = self.db.clone()?;
        let view = cx.new(|cx| {
            let mut c = ComposerView::new(db, window, cx);
            c.set_own_place();
            c
        });
        let subscription = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            this.on_draft_tab(draft, event, window, cx);
        });
        // The tab's title follows the subject as it is typed (the Slint
        // app's `retitle_draft_tab`).
        let title = cx.observe(&view, move |this, view, cx| {
            this.retitle_draft(draft, &view, cx);
        });
        self.tab_views
            .drafts
            .insert(draft, (view.clone(), [subscription, title]));
        Some(view)
    }

    /// Draft tab `draft` is titled by its composer's subject ("New
    /// message" while it has none).
    fn retitle_draft(
        &mut self,
        draft: DraftId,
        view: &Entity<ComposerView>,
        cx: &mut Context<Self>,
    ) {
        let Some(subject) = view.read(cx).drafts.active().map(|d| d.subject.clone()) else {
            return;
        };
        let title = text::draft_title(&subject);
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| t.kind == TabKind::Draft(draft))
        else {
            return;
        };
        if tab.title != title {
            tab.title = title;
            cx.notify();
        }
    }

    /// The active tab's body, made on first show (a restored saved draft
    /// opens its composer then); `None` for the Mail tab.
    pub(super) fn tab_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match self.tabs.active_kind() {
            TabKind::Mail => None,
            TabKind::Message(id) => {
                let view = self.message_view(id, window, cx)?;
                Some(div().size_full().child(view).into_any_element())
            }
            TabKind::Draft(d) => {
                let view = self.draft_view(d, window, cx)?;
                Some(div().size_full().child(view).into_any_element())
            }
            TabKind::Saved(row) => {
                // A restored draft: a draft tab of this session from now on.
                let n = self.tab_views.next_draft;
                self.tab_views.next_draft += 1;
                let d = DraftId::new(n);
                self.tabs.active_tab_mut().kind = TabKind::Draft(d);
                let view = self.draft_view(d, window, cx)?;
                view.update(cx, |c, cx| c.open(Some(Draft::Saved(row)), window, cx));
                Some(div().size_full().child(view).into_any_element())
            }
        }
    }

    /// The strip over the content, from two tabs on.
    pub(super) fn tab_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = crate::theme::ActiveTokens::tokens(&**cx).clone();
        let this = cx.entity().downgrade();
        let (a, b, c) = (this.clone(), this.clone(), this);
        strip::render(
            &self.tabs,
            &t,
            Handlers {
                select: std::rc::Rc::new(move |i, window, cx| {
                    let _ = a.update(cx, |this, cx| this.select_tab(i, window, cx));
                }),
                close: std::rc::Rc::new(move |i, window, cx| {
                    let _ = b.update(cx, |this, cx| this.close_tab(i, window, cx));
                }),
                command: std::rc::Rc::new(move |i, cmd, window, cx| {
                    let _ = c.update(cx, |this, cx| this.tab_command(i, cmd, window, cx));
                }),
            },
        )
    }

    // ------------------------------------------------- showing and closing

    /// Show tab `i`; the keys go to what it shows.
    pub fn select_tab(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if i >= self.tabs.len() {
            return;
        }
        let changed = i != self.tabs.active();
        if changed {
            self.flush_leaving_draft(cx);
        }
        self.tabs.set_active(i);
        let focus = match self.tabs.active_kind() {
            TabKind::Mail => self.mail.as_ref().map(|m| m.read(cx).list_focus().clone()),
            TabKind::Message(id) => self
                .message_view(id, window, cx)
                .map(|v| v.focus_handle(cx)),
            TabKind::Draft(d) => self.draft_view(d, window, cx).map(|v| v.focus_handle(cx)),
            TabKind::Saved(_) => None,
        };
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        if changed {
            self.tabs_changed(cx);
        }
        cx.notify();
    }

    /// The shown tab is left: the draft it shows is saved now, not when its
    /// autosave delay ends (the Slint app's
    /// `tab_switch_flushes_autosave_of_the_leaving_draft`). The Mail tab's
    /// draft is the reading pane's.
    fn flush_leaving_draft(&mut self, cx: &mut Context<Self>) {
        let composer = match self.tabs.active_kind() {
            TabKind::Draft(d) => self.tab_views.drafts.get(&d).map(|v| v.0.clone()),
            TabKind::Mail => self.mail.as_ref().map(|m| m.read(cx).composer().clone()),
            TabKind::Message(_) | TabKind::Saved(_) => None,
        };
        if let Some(composer) = composer {
            composer.update(cx, |c, cx| c.flush_autosave(cx));
        }
    }

    /// Close tab `i` (never the Mail tab): it can be reopened; a draft is
    /// saved by its composer as it closes.
    pub fn close_tab(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        let kind = tab.kind;
        match kind {
            TabKind::Mail => return,
            TabKind::Message(id) => {
                self.tab_views.messages.remove(&id);
                self.tabs.remember(Reopen::Message(id));
            }
            TabKind::Draft(d) => {
                if let Some(view) = self.tab_views.drafts.get(&d).map(|v| v.0.clone()) {
                    let (row, open) = view.update(cx, |c, cx| {
                        c.flush_autosave(cx);
                        let row = c.drafts.active().and_then(|d| d.row);
                        c.open(None, window, cx);
                        (row, c.is_open())
                    });
                    // Its save failed: the tab stays, with the draft and
                    // the error, rather than losing the text.
                    if open {
                        return;
                    }
                    self.tab_views.drafts.remove(&d);
                    if let Some(row) = row {
                        self.tabs.remember(Reopen::Draft(row));
                    }
                    self.reload_drafts(window, cx);
                }
            }
            TabKind::Saved(row) => self.tabs.remember(Reopen::Draft(row)),
        }
        let was_active = i == self.tabs.active();
        self.tabs.remove(i);
        if was_active {
            let now = self.tabs.active();
            self.select_tab(now, window, cx);
        }
        self.tabs_changed(cx);
        cx.notify();
    }

    pub(crate) fn tab_command(
        &mut self,
        i: usize,
        cmd: TabCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match cmd {
            TabCommand::Close => self.close_tab(i, window, cx),
            TabCommand::CloseOthers => {
                let keep = self.tabs.get(i).map(|t| t.kind);
                let mut at = self.tabs.len();
                while at > 1 {
                    at -= 1;
                    if self.tabs.get(at).map(|t| t.kind) != keep {
                        self.close_tab(at, window, cx);
                    }
                }
            }
            TabCommand::CloseRight => {
                // From the right end down, as Close others: a draft tab
                // whose save failed stays (close_tab keeps it), and the
                // walk goes past it rather than trying it again for ever.
                let mut at = self.tabs.len();
                while at > i + 1 {
                    at -= 1;
                    self.close_tab(at, window, cx);
                }
            }
            TabCommand::OpenWindow => match self.tabs.get(i).map(|t| t.kind) {
                Some(TabKind::Message(id)) => {
                    self.close_tab(i, window, cx);
                    self.open_in_window(id, window, cx);
                }
                Some(TabKind::Draft(_)) => self.draft_tab_to_window(i, window, cx),
                Some(TabKind::Saved(row)) => {
                    self.tabs.remove(i);
                    self.tabs_changed(cx);
                    self.open_draft_window(Draft::Saved(row), window, cx);
                }
                _ => {}
            },
            TabCommand::Reopen => self.reopen_tab_now(window, cx),
        }
    }

    /// The tab closed last comes back (Ctrl+Shift+T).
    pub fn reopen_tab_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        while let Some(reopen) = self.tabs.take_closed() {
            match reopen {
                Reopen::Message(id) if tab_views::message_title(&db, id).is_some() => {
                    self.open_in_tab(id, false, window, cx);
                    return;
                }
                Reopen::Draft(row) if matches!(db.draft(row), Ok(Some(_))) => {
                    self.open_draft_tab(Draft::Saved(row), window, cx);
                    return;
                }
                // Gone since: the one before it.
                _ => {}
            }
        }
    }

    /// The tabs changed: the setting follows (when the tabs are reopened at
    /// start; never in a snapshot).
    fn tabs_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if self.snapshot.is_some() {
            return;
        }
        let Some(db) = self.db.clone() else {
            return;
        };
        if !db.settings().is_ok_and(|s| s.reopen_tabs) {
            return;
        }
        let drafts = &self.tab_views.drafts;
        let value = self.tabs.setting(|d| {
            drafts
                .get(&d)
                .and_then(|(view, _)| saved_row(view.read(cx)))
        });
        if let Err(e) = db.set_setting(tab_views::SETTING, &value) {
            eprintln!("komorebi-gpui: saving the tabs: {e:#}");
        }
    }

    /// The saved tabs come back (after the first frame, when the setting
    /// asks for it).
    pub(super) fn restore_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let Ok(settings) = db.settings() else {
            return;
        };
        if !settings.reopen_tabs || settings.tabs.is_empty() {
            return;
        }
        self.tabs = tab_views::restore(&db, &settings.tabs);
        let active = self.tabs.active();
        self.select_tab(active, window, cx);
    }

    /// The cache changed: tabs whose message left get the mark (and their
    /// view's notice), the titles' unread state follows.
    pub(super) fn mark_gone_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let mut changed = false;
        for tab in self.tabs.iter_mut() {
            let TabKind::Message(id) = tab.kind else {
                continue;
            };
            let now = tab_views::message_title(&db, id);
            let gone = now.is_none();
            let unread = now.is_some_and(|(_, u)| u);
            if tab.gone != gone || tab.unread != unread {
                tab.gone = gone;
                tab.unread = unread;
                changed = true;
            }
            if let Some((view, _)) = self.tab_views.messages.get(&id) {
                view.update(cx, |v, cx| {
                    v.set_gone(gone, cx);
                    v.refresh(cx);
                });
            }
        }
        // A sync can change a message's flags or folder: every window's
        // bar reads the cache again.
        for (id, _, view, _) in &self.tab_views.popouts {
            let gone = tab_views::message_title(&db, *id).is_none();
            view.update(cx, |v, cx| {
                v.set_gone(gone, cx);
                v.refresh(cx);
            });
        }
        if changed {
            cx.notify();
        }
    }

    /// The held move's number, when message `id` is in it.
    fn held_seq(&self, id: i64, cx: &Context<Self>) -> Option<u64> {
        let mail = self.mail.as_ref()?.read(cx).mail();
        mail.held().filter(|h| h.ids.contains(&id)).map(|h| h.seq)
    }

    /// Tab `at` of message `id` closes for a move: remembered while the
    /// move can be undone.
    fn note_moved(&mut self, id: i64, at: usize, cx: &Context<Self>) {
        if let Some(seq) = self.held_seq(id, cx) {
            self.tab_views.moved.push(Moved {
                seq,
                message: id,
                at,
                from_tab: true,
            });
        }
    }

    /// The held move changed (the Slint app's `move_out` and Undo): the
    /// tabs and windows of messages moved from the list close, and Undo
    /// brings the closed tabs back at their places.
    pub(super) fn follow_held(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(db) = self.db.clone() else {
            return;
        };
        let held: Option<(u64, Vec<i64>)> = self.mail.as_ref().and_then(|m| {
            let h = m.read(cx).mail().held()?;
            Some((h.seq, h.ids.iter().copied().collect()))
        });
        // Runs on every redraw of the mail view: nothing held and nothing
        // to bring back costs nothing.
        if held.is_none()
            && self.tab_views.moved.is_empty()
            && self.tab_views.moved_windows.is_empty()
        {
            return;
        }
        let open_ids = |this: &Self| {
            this.tabs
                .iter()
                .filter_map(|t| match t.kind {
                    TabKind::Message(id) => Some(id),
                    _ => None,
                })
                .chain(this.tab_views.popouts.iter().map(|p| p.0))
                .collect::<Vec<_>>()
        };
        if self.tab_views.moved.is_empty()
            && self.tab_views.moved_windows.is_empty()
            && let Some((_, ids)) = &held
            && !open_ids(self).iter().any(|id| ids.contains(id))
        {
            return;
        }
        if let Some((seq, ids)) = &held {
            let mut i = self.tabs.len();
            while i > 1 {
                i -= 1;
                if let Some(TabKind::Message(id)) = self.tabs.get(i).map(|t| t.kind)
                    && ids.contains(&id)
                {
                    self.tab_views.moved.push(Moved {
                        seq: *seq,
                        message: id,
                        at: i,
                        from_tab: false,
                    });
                    self.close_tab(i, window, cx);
                }
            }
            let gone: Vec<WindowHandle<Popout>> = self
                .tab_views
                .popouts
                .iter()
                .filter(|p| ids.contains(&p.0))
                .map(|p| p.1)
                .collect();
            for p in self.tab_views.popouts.iter().filter(|p| ids.contains(&p.0)) {
                self.tab_views.moved_windows.push((*seq, p.0));
            }
            self.tab_views.popouts.retain(|p| !ids.contains(&p.0));
            let size = self.tab_views.popout_size.clone();
            for handle in gone {
                let _ = handle.update(cx, |_, w, _| popout::close_keeping_size(w, &size));
            }
        }
        let now = held.map(|(seq, _)| seq);
        let mut back = Vec::new();
        self.tab_views.moved.retain(|m| {
            if now == Some(m.seq) {
                return true;
            }
            // Undone: the message is listed again. Done: it left the cache.
            if tab_views::message_title(&db, m.message).is_some() {
                back.push(*m);
            }
            false
        });
        let mut windows_back = false;
        self.tab_views.moved_windows.retain(|&(seq, id)| {
            if now == Some(seq) {
                return true;
            }
            windows_back |= tab_views::message_title(&db, id).is_some();
            false
        });
        if !back.is_empty() {
            back.sort_by_key(|m| m.at);
            for m in &back {
                self.reopen_at(m.message, m.at);
            }
            // The Slint app's `reopen_message_tabs`: the tab the move was
            // made from shows again; otherwise the shown tab stays.
            if let Some(i) = back
                .iter()
                .find(|m| m.from_tab)
                .and_then(|m| self.tabs.find_message(m.message))
            {
                self.select_tab(i, window, cx);
            }
            self.tabs_changed(cx);
        }
        if windows_back && let Some(mail) = self.mail.clone() {
            mail.update(cx, |m, cx| m.act(window, cx, |m| m.say_undo_window()));
        }
    }

    /// Message `id` in a tab at place `at` (or the end), unless it has
    /// one; the shown tab stays.
    fn reopen_at(&mut self, id: i64, at: usize) {
        let Some(db) = self.db.clone() else {
            return;
        };
        if self.tabs.find_message(id).is_some() {
            return;
        }
        let Some((title, unread)) = tab_views::message_title(&db, id) else {
            return;
        };
        let mut tab = Tab::new(TabKind::Message(id), title);
        tab.unread = unread;
        self.tabs.insert(at.clamp(1, self.tabs.len()), tab);
    }

    // ----------------------------------------------- what the views ask

    pub(super) fn on_message_tab(
        &mut self,
        id: i64,
        event: &MessageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            e if moves_out(e) => {
                self.message_action(event, window, cx);
                // Moved (held in the undo window): the tab closes and Undo
                // brings it back. A delete from Trash or Junk asks first;
                // once confirmed, `follow_held` closes the tab.
                if self.held_seq(id, cx).is_some()
                    && let Some(i) = self.tabs.find_message(id)
                {
                    self.note_moved(id, i, cx);
                    self.close_tab(i, window, cx);
                }
            }
            MessageEvent::OpenWindow(_) => {
                if let Some(i) = self.tabs.find_message(id) {
                    self.close_tab(i, window, cx);
                }
                self.open_in_window(id, window, cx);
            }
            MessageEvent::Close => {
                if let Some(i) = self.tabs.find_message(id) {
                    self.close_tab(i, window, cx);
                }
            }
            _ => self.message_action(event, window, cx),
        }
    }

    fn on_popout(
        &mut self,
        id: i64,
        handle: WindowHandle<Popout>,
        event: &MessageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            event,
            MessageEvent::Reply { .. }
                | MessageEvent::Forward { .. }
                | MessageEvent::ForwardAttached(_)
        ) {
            window.activate_window();
        }
        if !matches!(event, MessageEvent::Close) {
            self.message_action(event, window, cx);
        }
        // Moved (held): the window goes, as the tab does.
        let close = matches!(event, MessageEvent::Close)
            || (moves_out(event) && self.held_seq(id, cx).is_some());
        if close {
            self.tab_views.popouts.retain(|p| p.0 != id);
            let size = self.tab_views.popout_size.clone();
            let _ = handle.update(cx, |_, w, _| popout::close_keeping_size(w, &size));
        }
    }

    /// What a message tab or window asks, done by the mail view, the hub or
    /// a draft tab.
    pub(super) fn message_action(
        &mut self,
        event: &MessageEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MessageEvent::Reply { message, all } => self.open_draft_tab(
                Draft::Reply {
                    message: *message,
                    all: *all,
                },
                window,
                cx,
            ),
            MessageEvent::Forward { message } => self.open_draft_tab(
                Draft::Forward {
                    message: *message,
                    attached: false,
                },
                window,
                cx,
            ),
            MessageEvent::ForwardAttached(message) => self.open_draft_tab(
                Draft::Forward {
                    message: *message,
                    attached: true,
                },
                window,
                cx,
            ),
            MessageEvent::Archive(id) => self.quick(*id, Quick::Archive, window, cx),
            MessageEvent::Delete(id) => self.quick(*id, Quick::Delete, window, cx),
            MessageEvent::Move { message, folder } => {
                self.quick(*message, Quick::Move(*folder), window, cx)
            }
            MessageEvent::Junk(id) => self.quick(*id, Quick::Junk, window, cx),
            MessageEvent::NotJunk(id) => self.quick(*id, Quick::NotJunk, window, cx),
            MessageEvent::Flag(id) | MessageEvent::Read(id) => {
                let action = match event {
                    MessageEvent::Flag(_) => Quick::Flag,
                    _ => Quick::Read,
                };
                self.quick(*id, action, window, cx);
                // The tab's title and the other views of the message follow.
                self.mark_gone_tabs(cx);
                self.refresh_message_views(*id, cx);
            }
            MessageEvent::Reader(event) => self.tab_reader(event, window, cx),
            MessageEvent::OpenWindow(_) | MessageEvent::Close => {}
        }
    }

    /// The tab and the window of message `id` read the cache again for
    /// their command bars.
    fn refresh_message_views(&self, id: i64, cx: &mut Context<Self>) {
        if let Some((view, _)) = self.tab_views.messages.get(&id) {
            view.update(cx, |v, cx| v.refresh(cx));
        }
        for (_, _, view, _) in self.tab_views.popouts.iter().filter(|p| p.0 == id) {
            view.update(cx, |v, cx| v.refresh(cx));
        }
    }

    /// The mail view set read or important marks (its list, the keyboard,
    /// Mark all as read): every message tab and window reads its bar again,
    /// so Mark as read never says the opposite of the cache. Runs on every
    /// redraw of the mail view; with no new marks it costs one compare.
    pub(super) fn follow_flags(&mut self, cx: &mut Context<Self>) {
        let Some(seq) = self.mail.as_ref().map(|m| m.read(cx).mail().flags_seq()) else {
            return;
        };
        if seq == self.tab_views.flags_seen {
            return;
        }
        self.tab_views.flags_seen = seq;
        self.mark_gone_tabs(cx);
    }

    fn quick(&mut self, id: i64, action: Quick, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mail) = &self.mail {
            mail.update(cx, |m, cx| {
                m.act(window, cx, |m| m.act_on_message(id, action))
            });
        }
    }

    /// A tab's or a window's reader: downloads and the status line go to
    /// the hub; the read mark, Delete and Junk to the mail view.
    fn tab_reader(&mut self, event: &ReaderEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hub) = &self.hub
            && hub.update(cx, |hub, cx| hub.on_reader(event, cx))
        {
            return;
        }
        match event {
            ReaderEvent::MarkRead(id) => {
                let unread = self
                    .db
                    .as_ref()
                    .and_then(|db| tab_views::message_title(db, *id))
                    .is_some_and(|(_, unread)| unread);
                if unread {
                    self.quick(*id, Quick::Read, window, cx);
                    self.mark_gone_tabs(cx);
                }
            }
            ReaderEvent::Delete(id) => self.quick(*id, Quick::Delete, window, cx),
            // Report junk or Not junk from a tab's or a window's reader (the
            // mail view acts only on its own open message).
            ReaderEvent::Junk { message, junk } => {
                let action = if *junk { Quick::Junk } else { Quick::NotJunk };
                self.quick(*message, action, window, cx)
            }
            ReaderEvent::OpenPrivacy => self.show_section(Section::Privacy, window, cx),
            // "You started a reply": the saved reply in a tab of its own.
            ReaderEvent::ResumeReply { draft, .. } => {
                self.open_draft_tab(Draft::Saved(*draft), window, cx)
            }
            ReaderEvent::EditServerDraft(message) => {
                self.edit_server_draft_tab(*message, window, cx)
            }
            // A `mailto:` link in a tab's or a window's message: the
            // composer in the Mail tab, filled (the Slint app's popout
            // `on_mailto`).
            ReaderEvent::Compose(m) => {
                window.activate_window();
                self.compose_mailto(m.clone(), window, cx);
            }
            _ => {}
        }
    }

    fn on_draft_tab(
        &mut self,
        draft: DraftId,
        event: &ComposerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hub) = &self.hub {
            hub.update(cx, |hub, cx| hub.on_composer(event, cx));
        }
        match event {
            // Sent, discarded or closed: the tab goes (nothing to reopen).
            ComposerEvent::Closed => {
                self.tab_views.drafts.remove(&draft);
                if let Some(i) = self.tabs.find(TabKind::Draft(draft)) {
                    let was_active = i == self.tabs.active();
                    self.tabs.remove(i);
                    if was_active {
                        let now = self.tabs.active();
                        self.select_tab(now, window, cx);
                    }
                    self.tabs_changed(cx);
                }
            }
            ComposerEvent::DraftsChanged { .. } | ComposerEvent::Unsent { .. } => {
                if let Some(mail) = &self.mail {
                    mail.update(cx, |m, cx| {
                        m.act(window, cx, |m| {
                            m.load_folders();
                            m.reload_rows();
                        })
                    });
                }
                self.tabs_changed(cx);
            }
            ComposerEvent::ManageSignatures => self.show_section(Section::Signatures, window, cx),
            ComposerEvent::OpenInWindow => {
                if let Some(i) = self.tabs.find(TabKind::Draft(draft)) {
                    self.draft_tab_to_window(i, window, cx);
                }
            }
            ComposerEvent::Raise => {
                if let Some(i) = self.tabs.find(TabKind::Draft(draft)) {
                    self.select_tab(i, window, cx);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------- keys

    /// The message Shift+Enter and Ctrl+O act on: the open one, or the one
    /// selected row.
    fn current_message(&self, cx: &Context<Self>) -> Option<i64> {
        let mail = self.mail.as_ref()?.read(cx).mail();
        mail.open_id().or_else(|| match mail.selection().ids()[..] {
            [id] => Some(id),
            _ => None,
        })
    }

    pub(super) fn go_to_tab(&mut self, a: &GoToTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.tabs.number(a.0) {
            self.select_tab(i, window, cx);
        }
    }

    pub(super) fn close_tab_key(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let i = self.tabs.active();
        self.close_tab(i, window, cx);
    }

    pub(super) fn reopen_tab(
        &mut self,
        _: &ReopenTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reopen_tab_now(window, cx);
    }

    pub(super) fn move_tab_left(
        &mut self,
        _: &MoveTabLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let i = self.tabs.active();
        if i > 1 && self.tabs.move_tab(i, i - 1) {
            self.tabs_changed(cx);
        }
    }

    pub(super) fn move_tab_right(
        &mut self,
        _: &MoveTabRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let i = self.tabs.active();
        if i > 0 && i + 1 < self.tabs.len() && self.tabs.move_tab(i, i + 1) {
            self.tabs_changed(cx);
        }
    }

    pub(super) fn open_in_tab_key(
        &mut self,
        _: &OpenInTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.current_message(cx) {
            self.open_in_tab(id, false, window, cx);
        }
    }

    pub(super) fn open_in_window_key(
        &mut self,
        _: &OpenInWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A draft tab, or the pane's draft with the keys: its own window.
        if let TabKind::Draft(_) = self.tabs.active_kind() {
            let i = self.tabs.active();
            return self.draft_tab_to_window(i, window, cx);
        }
        if self.pane_draft_to_window(window, cx) {
            return;
        }
        // A row of the Drafts list: that draft in its own window.
        if let Some(row) = self.drafts_list_row(cx) {
            return self.draft_row_to_window(row, window, cx);
        }
        if let Some(id) = self.current_message(cx) {
            self.open_in_window(id, window, cx);
        }
    }

    /// The Drafts list's current row (a saved draft), while the Mail tab
    /// shows that list.
    fn drafts_list_row(&self, cx: &Context<Self>) -> Option<i64> {
        let mail = self.mail.as_ref()?.read(cx).mail();
        if mail.view() != crate::views::mail::model::View::Drafts {
            return None;
        }
        mail.current_row()
    }

    /// Saved draft `row` (a Drafts list row) in a composer window: the
    /// pane's draft moves there when the pane shows it, else the draft
    /// opens in a window of its own (one already open comes forward).
    fn draft_row_to_window(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        let in_pane = self.mail.as_ref().is_some_and(|m| {
            let m = m.read(cx);
            m.mail().draft_open() == Some(row) && m.composer().read(cx).is_open()
        });
        if in_pane {
            self.pane_draft_out(window, cx);
        } else {
            self.open_draft_window(Draft::Saved(row), window, cx);
        }
    }

    /// The main window closes (or the app quits): the message windows go
    /// with it. A composer window's draft is saved first, here and on the
    /// server, as Esc or Close does (it is reopened from Drafts). One whose
    /// save failed keeps its window, brought forward with the error, so
    /// its text is not lost: false then, and the caller neither closes the
    /// main window nor quits.
    pub fn close_popouts(&mut self, cx: &mut Context<Self>) -> bool {
        for (_, handle, _, _) in self.tab_views.popouts.drain(..) {
            let _ = handle.update(cx, |_, w, _| w.remove_window());
        }
        let mut kept = Vec::new();
        for w in std::mem::take(&mut self.tab_views.draft_windows) {
            let saved = w.view.update(cx, |c, cx| {
                let others = c.save_all(cx);
                c.park_composer(cx) && others
            });
            if saved {
                let _ = w.handle.update(cx, |_, w, _| w.remove_window());
            } else {
                let _ = w.handle.update(cx, |_, w, _| w.activate_window());
                kept.push(w);
            }
        }
        let all = kept.is_empty();
        self.tab_views.draft_windows = kept;
        all
    }
}

/// A tab's or a window's command that takes the message out of its folder
/// (Archive, Delete, Move, Report junk, Not junk).
fn moves_out(event: &MessageEvent) -> bool {
    matches!(
        event,
        MessageEvent::Archive(_)
            | MessageEvent::Delete(_)
            | MessageEvent::Move { .. }
            | MessageEvent::Junk(_)
            | MessageEvent::NotJunk(_)
    )
}

/// A composer's saved draft row, when it shows one: the row its draft was
/// saved to (a reply saved by its autosave too), or the saved draft it
/// opened.
fn saved_row(c: &ComposerView) -> Option<i64> {
    if !c.is_open() {
        return None;
    }
    c.drafts.active().and_then(|d| d.row).or(match c.draft() {
        Some(Draft::Saved(row)) => Some(row),
        _ => None,
    })
}
