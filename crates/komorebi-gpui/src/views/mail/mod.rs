//! The mail view (docs/GPUI-MIGRATION.md phase 2): the folder pane or rail
//! on canvas, the message list and the reading pane on paper (the dapple
//! layout: 8 px apart, no line between), with the reader and the composer
//! mounted through their typed constructors.
//!
//! The state is [`model::Mail`] (no GPUI, plain tests); this view routes
//! keys, clicks and menus to it, then does what it asks ([`model::Effect`]:
//! timers, the composer, the focus, a question) and redraws. The parts:
//! `folders.rs` (pane and rail), `list.rs` and `rows.rs` (the list),
//! `filters.rs` and `search.rs`, `reading.rs` (the reading pane's command
//! bar and selection panel), `menus.rs` (row, list and pane menus, Move),
//! `commands.rs` (the keys), `text.rs` (the words).
//!
//! The sync parts (the shell lane): `hub.rs` (the account workers, their
//! events, the jobs, the status line), `sync.rs` (what they said, as
//! state), `sync_text.rs` (its words), `status.rs` (the line as a notice
//! in the list pane) and `outbox.rs` (the Outbox entry's pane).

mod calm;
pub mod commands;
mod filters;
mod folder_edit;
#[cfg(test)]
mod folder_edit_tests;
mod folders;
pub mod hub;
mod list;
mod menus;
pub mod model;
pub mod nav;
mod outbox;
mod panes;
mod reading;
mod rows;
mod scene;
mod search;
mod sky;
pub mod status;
pub mod sync;
mod sync_tests;
pub mod sync_text;
#[cfg(test)]
mod tests;
pub mod text;
pub mod triage;

pub use search::field as search_field;
pub use search::scope_box as search_scope_box;

use std::collections::HashMap;
use std::time::Instant;

use gpui::{
    AnyView, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, ScrollStrategy, Styled,
    Subscription, UniformListScrollHandle, Window, div, prelude::*, px,
};
use gpui_kit::component::input::{InputEvent, InputState};

use self::hub::{Hub, HubEvent};
use self::model::{Effect, Escaped, Mail, Question};
use self::nav::NavKind;
use self::sync::{Reaction, State};
use self::triage::Flag;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, space};
use crate::views::Db;
use crate::views::composer::{ComposerEvent, ComposerView, Draft};
use crate::views::reader::{ReaderEvent, ReaderView};
use crate::views::settings::Section;
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::loader::Loader;

/// What the mail view tells its window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailEvent {
    /// The inboxes' unread count changed (the window title shows it).
    Unread(i64),
    /// Contacts or Settings was chosen at the pane's foot (the window shows
    /// the view).
    Show(Place),
    /// Message `id` asks for a tab of its own (double click, middle click
    /// with `background`, Shift+Enter, the menus).
    OpenTab { id: i64, background: bool },
    /// Message `id` asks for a window of its own (Ctrl+O, the menus).
    OpenWindow(i64),
    /// The cache or the held move changed (the window updates its tabs).
    Changed,
    /// Open the page of account `id` in Settings (the pane's and the
    /// status line's "Account settings", "Update password", "Review
    /// certificate").
    EditAccount(i64),
    /// A folder, the Inbox or New mail was chosen while a view (Settings,
    /// Contacts) shows beside the pane: the window takes it away.
    ClosePlace,
    /// A link for the window's link prompt (the What's new bar).
    OpenLink(String),
    /// Settings on `section` (Privacy from the reader, Signatures from the
    /// composer); left open if it already shows.
    OpenSettings(crate::views::settings::Section),
    /// The pane's draft in a composer window of its own (the composer's
    /// More menu).
    OpenDraftWindow,
    /// Saved draft `row` in a tab of its own: a pane draft set aside for
    /// another (`background`, "Draft kept in a tab"), or an undone send
    /// that came from a tab or a composer window (`sent_by`: the composer
    /// that sent it, so a composer window's comes back in a window).
    OpenDraftTab {
        row: i64,
        background: bool,
        sent_by: Option<gpui::EntityId>,
    },
}

/// The views the pane's foot opens, and the ones the window mounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Contacts,
    Settings,
    /// Reached from Settings (the window mounts it).
    Rules,
    /// The first-account flow, instead of the mail while there is no
    /// account.
    Onboarding,
}

/// The widths the layout steps at (the Slint app's and the mockups').
fn compact(width: f32) -> bool {
    width < layout::COMPACT_BELOW_W
}

/// The F6 stops (the Slint `stop-at` order, without the tab strip, which
/// the window owns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Search,
    Folders,
    List,
    Reading,
}

pub const STOPS: [Stop; 4] = [Stop::Search, Stop::Folders, Stop::List, Stop::Reading];

/// The stop after `now` (before it, `back`) among those that show (`ok`,
/// in [`STOPS`]' order); `None` when no other shows.
pub fn next_stop(now: Stop, ok: &[bool; 4], back: bool) -> Option<Stop> {
    let at = STOPS.iter().position(|&s| s == now).unwrap_or(2);
    let n = STOPS.len();
    (1..n)
        .map(|d| if back { (at + n - d) % n } else { (at + d) % n })
        .find(|&j| ok[j])
        .map(|j| STOPS[j])
}

/// The mail view's group for the file drop overlay.
const DROP_GROUP: &str = "mail-drop";

/// "Drop to attach to a new message" over the mail while files are
/// dragged over it (the Slint `DropOverlay`); hidden otherwise.
fn drop_overlay(cx: &Context<MailView>) -> gpui::AnyElement {
    let t = cx.tokens().clone();
    div()
        .absolute()
        .inset_0()
        .invisible()
        .group_drag_over::<gpui::ExternalPaths>(DROP_GROUP, |s| s.visible())
        .flex()
        .items_center()
        .justify_center()
        .bg(t.color.wash)
        .rounded(px(crate::theme::radius::CARD))
        .child(
            div()
                .px(px(space::SP_4))
                .py(px(space::SP_2))
                .rounded(px(crate::theme::radius::FULL))
                .bg(t.color.paper)
                .typo(&t.text.body_strong)
                .text_color(t.color.ink)
                .child(text::drop_to_attach()),
        )
        .into_any_element()
}

/// Below this width one pane shows at a time (the list, or the message).
pub const ONE_PANE_BELOW_W: f32 = 720.;

pub struct MailView {
    mail: Mail,
    hub: Entity<Hub>,
    reader: Entity<ReaderView>,
    composer: Entity<ComposerView>,
    search: Entity<InputState>,
    list_focus: FocusHandle,
    nav_focus: FocusHandle,
    /// The reading pane's own stop (the selection panel, an Outbox entry,
    /// the command bar): the message keys reach the list from it too.
    reading_focus: FocusHandle,
    list_scroll: UniformListScrollHandle,
    nav_scroll: ScrollHandle,
    /// The folder pane's keyboard line.
    nav_cursor: usize,
    /// The Move flyout's filter.
    move_filter: Entity<InputState>,
    /// The last unread count told to the window.
    told_unread: Option<i64>,
    /// The first frame was drawn (the default folds are written after it).
    drawn: bool,
    /// Effects that wait for the window (asked for by a timer).
    waiting: Vec<Effect>,
    /// The open flyout (Move, or a menu opened from the keyboard).
    flyout: Option<menus::Flyout>,
    flyout_focus: FocusHandle,
    /// The window is narrower than the compact width (last frame).
    compact_now: bool,
    /// The search box's placeholder as last set.
    placeholder: String,
    /// The row under the pointer (its quick actions show).
    hovered_row: Option<i64>,
    /// The full folder pane is open over the content (from the rail).
    pane_open: bool,
    /// The pane scrolls to its keyboard line once it is laid out.
    nav_follow: bool,
    /// The three spots in the list's header while an account syncs.
    loader: Entity<Loader>,
    /// Accounts out of reach at their last sync, with what their badge
    /// says ("Can't reach imap.example.com"), until one goes through.
    badges: HashMap<i64, String>,
    /// The Outbox entry whose server reply shows (Details).
    outbox_details: Option<i64>,
    /// A view the window mounts beside the pane in place of the list and
    /// the reading pane (Settings, Contacts, Rules).
    place: Option<(Place, AnyView)>,
    /// The folder pane and the list as cached views (`panes.rs`), made on
    /// the first render, and the shape this view last gave them.
    panes: Option<panes::Panes>,
    shape: panes::Shape,
    /// The calm panel shows beside the wider list (nothing selected; the
    /// last frame's `calm::mode`).
    calm_now: bool,
    /// The Today summary and what it was read for.
    calm: Option<(calm::Key, calm::Summary)>,
    /// The Rest scene's own view, made the first time Rest shows.
    rest_scene: Option<gpui::Entity<scene::RestScene>>,
    /// A folder's name typed in the pane (New folder, Rename).
    folder_edit: Option<folder_edit::FolderEdit>,
    /// Each account's icon (`views::account_icon`), read from the cache.
    account_looks: crate::views::account_icon::Looks,
    /// Accounts whose domain icon is being fetched.
    icons_fetching: std::collections::HashSet<i64>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MailEvent> for MailView {}

impl MailView {
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hub = cx.new(|cx| Hub::new(db.clone(), cx));
        Self::with_hub(db, hub, window, cx)
    }

    /// The mail view over a hub the window shares with its tabs.
    pub fn with_hub(db: Db, hub: Entity<Hub>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        commands::bind_keys(cx);
        let reader = cx.new(|cx| ReaderView::new(db.clone(), window, cx));
        // This pane opens the saved reply (`ReaderEvent::ResumeReply`).
        reader.update(cx, |r, cx| r.handle_resume(cx));
        let composer = cx.new(|cx| ComposerView::new(db.clone(), window, cx));
        let mail = Mail::new(db);
        let placeholder = search::placeholder(&mail);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.clone()));
        let move_filter = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            // Typing searches (set_value from code does not emit Change).
            cx.subscribe_in(&search, window, |this, input, event, window, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value().to_string();
                    this.act(window, cx, |m| m.search_edited(&text));
                }
            }),
            cx.subscribe_in(&reader, window, |this, _, event, window, cx| {
                this.on_reader(event, window, cx);
            }),
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                this.on_composer(event, window, cx);
            }),
            cx.subscribe_in(&hub, window, |this, _, event, window, cx| {
                this.on_hub(event, window, cx);
            }),
            cx.subscribe_in(&move_filter, window, |_, _, event, _, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
        ];
        let list_focus = cx.focus_handle();
        let nav_focus = cx.focus_handle();
        let mut this = Self {
            mail,
            hub,
            reader,
            composer,
            search,
            list_focus,
            nav_focus,
            reading_focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            nav_scroll: ScrollHandle::new(),
            nav_cursor: 0,
            move_filter,
            told_unread: None,
            drawn: false,
            waiting: Vec::new(),
            flyout: None,
            flyout_focus: cx.focus_handle(),
            compact_now: false,
            placeholder,
            hovered_row: None,
            pane_open: false,
            nav_follow: false,
            loader: cx.new(|_| Loader::new(sync_text::syncing())),
            badges: HashMap::new(),
            outbox_details: None,
            place: None,
            panes: None,
            shape: panes::Shape::default(),
            calm_now: false,
            calm: None,
            rest_scene: None,
            folder_edit: None,
            account_looks: Default::default(),
            icons_fetching: Default::default(),
            _subscriptions: subscriptions,
        };
        this.reload_icons();
        this.fetch_due_icons(cx);
        this.follow_pill(false);
        this.take_sync_state(cx);
        this
    }

    /// The accounts' icons, read again (an icon chosen, removed, fetched;
    /// the switch).
    fn reload_icons(&mut self) {
        self.account_looks =
            crate::views::account_icon::looks(self.mail.db(), self.mail.accounts());
    }

    /// An account's icon, if it has one.
    pub fn account_look(&self, account: i64) -> Option<&crate::views::account_icon::Look> {
        self.account_looks.get(&account)
    }

    /// Ask the domains whose icon is due (an account just added, or not
    /// asked for `favicon::REFRESH`), off the UI thread; what comes back
    /// is kept and shown. Never under `KOMOREBI_OFFLINE` or with the switch
    /// off (`favicon::due`), and never from the tests (no network there).
    fn fetch_due_icons(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let now = chrono::Utc::now().timestamp();
        let due: Vec<(i64, String)> = self
            .mail
            .accounts()
            .iter()
            .filter(|a| !self.icons_fetching.contains(&a.id))
            .filter(|a| komorebi::favicon::due(self.mail.db(), a, now))
            .filter_map(|a| komorebi::favicon::domain_to_ask(a).map(|d| (a.id, d)))
            .collect();
        for (account, domain) in due {
            self.icons_fetching.insert(account);
            let task = cx.background_spawn(async move {
                komorebi::favicon::fetch(&domain, &komorebi::favicon::get)
            });
            cx.spawn(async move |this, cx| {
                let found = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.icons_fetching.remove(&account);
                    let now = chrono::Utc::now().timestamp();
                    komorebi::favicon::keep(this.mail.db(), account, found, now).ok();
                    this.reload_icons();
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The caution badge's words for an account out of reach, if it is.
    pub fn badge(&self, account: i64) -> Option<&str> {
        self.badges.get(&account).map(String::as_str)
    }

    /// The pane's keyboard line goes to the open view's line, scrolled
    /// into view (a start, a new view).
    fn follow_pill(&mut self, rail: bool) {
        self.nav_cursor = self.nav_items(rail).pill.unwrap_or(0);
        if self.drawn {
            self.nav_scroll.scroll_to_item(self.nav_cursor);
        } else {
            // The first frame lays the lines out before the pane scrolls
            // (GPUI applies a scroll to an item from the frame after).
            self.nav_follow = true;
        }
    }

    pub fn mail(&self) -> &Mail {
        &self.mail
    }

    /// The view shown beside the pane, if any.
    pub fn place(&self) -> Option<Place> {
        self.place.as_ref().map(|(p, _)| *p)
    }

    /// Show `place` beside the pane in place of the list and the reading
    /// pane, or the mail again (`None`). The folder pane and the rail stay;
    /// choosing a folder asks the window to take the view away
    /// ([`MailEvent::ClosePlace`]).
    pub fn set_place(&mut self, place: Option<(Place, AnyView)>, cx: &mut Context<Self>) {
        self.place = place;
        cx.notify();
    }

    /// The cache's settings or accounts changed (Settings, the account
    /// form): the pane, the lists, the composer and the reader read them
    /// again.
    pub fn reload_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.act(window, cx, |m| {
            m.load_folders();
            m.reload_rows();
        });
        self.reload_icons();
        self.fetch_due_icons(cx);
        if let Ok(settings) = self.mail.db().settings() {
            self.reader
                .update(cx, |r, _| r.set_mark_read(settings.mark_read));
        }
        self.composer.update(cx, |c, cx| c.reload_accounts(cx));
        cx.notify();
    }

    /// Land on `account`'s Inbox (the end of the first-run flow), else on
    /// all inboxes; the list has the keys.
    pub fn show_account_inbox(
        &mut self,
        account: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inbox = self
            .mail
            .folders()
            .iter()
            .find(|f| f.account_id == account && f.role == komorebi::role::Role::Inbox)
            .map(|f| f.id);
        match inbox {
            Some(id) => {
                self.act(window, cx, |m| m.set_view(model::View::Folder(id)));
                let rail = self.keys_on_rail(window);
                self.follow_pill(rail);
            }
            None => self.go_inbox(window, cx),
        }
        window.focus(&self.list_focus, cx);
    }

    /// A choice in the pane while a view shows beside it: the view goes.
    fn leave_place(&mut self, cx: &mut Context<Self>) {
        if self.place.is_some() {
            cx.emit(MailEvent::ClosePlace);
        }
    }

    pub fn reader(&self) -> &Entity<ReaderView> {
        &self.reader
    }

    pub fn composer(&self) -> &Entity<ComposerView> {
        &self.composer
    }

    /// The search field's state (the window draws it in the title bar).
    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    /// The list's focus (the list's keys go there).
    pub fn list_focus(&self) -> &FocusHandle {
        &self.list_focus
    }

    /// How far the list is scrolled (logical px, signed), for the A/B
    /// harness's scroll step (`bench.rs`).
    pub(crate) fn bench_list_offset(&self) -> f32 {
        f32::from(self.list_scroll.0.borrow().base_handle.offset().y)
    }

    pub fn nav_focus(&self) -> &FocusHandle {
        &self.nav_focus
    }

    /// Run `f` on the state, then do what it asks and redraw.
    pub fn act(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Mail)) {
        f(&mut self.mail);
        self.settle(cx);
        self.window_effects(window, cx);
        self.sync_placeholder(window, cx);
    }

    /// The search's placeholder names the view, in the language shown.
    fn sync_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = search::placeholder(&self.mail);
        if self.placeholder != placeholder {
            self.placeholder = placeholder.clone();
            self.search
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
    }

    /// [`MailView::act`] without the window (a timer): what needs the
    /// window waits for the next frame.
    fn act_later(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Mail)) {
        f(&mut self.mail);
        self.settle(cx);
    }

    /// Timers, the reader, the scroll position, the title's count; the
    /// effects that need the window are kept for [`Self::window_effects`].
    fn settle(&mut self, cx: &mut Context<Self>) {
        for effect in self.mail.take_effects() {
            match effect {
                Effect::ReadAfter(id, delay) => {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(delay).await; // timer-exempt: the mark-as-read delay (setting mark_read_delay_ms)
                        let _ = this.update(cx, |this, cx| {
                            if this.mail.open_id() == Some(id) {
                                this.act_later(cx, Mail::read_due);
                            }
                        });
                    })
                    .detach();
                }
                Effect::UndoAfter(seq, after) => {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(after).await; // timer-exempt: the Undo offer's window (WCAG 2.2.1)
                        let _ = this.update(cx, |this, cx| {
                            this.act_later(cx, |m| m.undo_due(seq, Instant::now()));
                        });
                    })
                    .detach();
                }
                Effect::SearchAfter(seq, after) => {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(after).await; // timer-exempt: the search waits for a pause in typing on large folders
                        let _ = this.update(cx, |this, cx| {
                            this.act_later(cx, |m| m.search_due(seq));
                        });
                    })
                    .detach();
                }
                other => self.waiting.push(other),
            }
        }
        // The server changes go to the hub, which hands them to the
        // account's workers (dropped when it has none: the cache keeps the
        // pending state).
        let jobs = self.mail.take_jobs();
        if !jobs.is_empty() {
            self.hub.update(cx, |hub, _| {
                for (account, job) in jobs {
                    hub.send(account, job);
                }
            });
        }
        // The reader follows the open message in `sync_reader` (it needs
        // the window: the page's key reads the theme and scale).
        if let Some(ix) = self.mail.current_index() {
            self.list_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        let unread = self.mail.inbox_unread();
        if self.told_unread != Some(unread) {
            self.told_unread = Some(unread);
            cx.emit(MailEvent::Unread(unread));
        }
        cx.notify();
    }

    /// The effects that need the window: the composer, a question, the
    /// focus.
    fn window_effects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for effect in std::mem::take(&mut self.waiting) {
            match effect {
                Effect::OpenDraft(id) => {
                    self.composer
                        .update(cx, |c, cx| c.open(Some(Draft::Saved(id)), window, cx));
                }
                Effect::CloseComposer => {
                    // Another message was chosen: a draft with writing in it
                    // goes to a tab of its own; an empty one goes.
                    if !self.make_room(cx) {
                        self.composer.update(cx, |c, cx| c.open(None, window, cx));
                    }
                }
                Effect::Ask(question) => self.ask(question, window, cx),
                Effect::FocusList => window.focus(&self.list_focus, cx),
                Effect::ReadAfter(..) | Effect::UndoAfter(..) | Effect::SearchAfter(..) => {}
            }
        }
        self.sync_reader(window, cx);
    }

    /// The reader shows the list's open message, or nothing.
    fn sync_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.mail.open_id();
        if self.reader.read(cx).message() == open {
            return;
        }
        self.reader.update(cx, |r, cx| match open {
            Some(id) => {
                r.open(id, window, cx);
            }
            None => r.close(window, cx),
        });
    }

    /// What the composer tells the mail view: a closed composer gives the
    /// list its keys back, and a saved or removed draft redraws the Drafts
    /// count and list. The status line, the Undo send offer and the server
    /// jobs (copies, sends) wait for the shell's status line and the
    /// account workers (docs/GPUI-MIGRATION.md stage B, remaining).
    fn on_composer(&mut self, event: &ComposerEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.hub.update(cx, |hub, cx| hub.on_composer(event, cx));
        match event {
            ComposerEvent::ManageSignatures => {
                cx.emit(MailEvent::OpenSettings(Section::Signatures))
            }
            ComposerEvent::OpenInWindow => cx.emit(MailEvent::OpenDraftWindow),
            ComposerEvent::Closed => {
                window.focus(&self.list_focus, cx);
                cx.notify();
            }
            ComposerEvent::DraftsChanged { .. } | ComposerEvent::Unsent { .. } => {
                self.act(window, cx, |m| {
                    m.load_folders();
                    m.reload_rows();
                });
            }
            _ => {}
        }
    }

    /// What the reader asks of the mail view. Marking read is the list's
    /// own (its read delay, [`Effect::ReadAfter`]); downloads, attachments,
    /// the invite's dispatch and the status line wait for the account
    /// workers and the shell (docs/GPUI-MIGRATION.md stage B, remaining).
    fn on_reader(&mut self, event: &ReaderEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.hub.update(cx, |hub, cx| hub.on_reader(event, cx));
        match event {
            ReaderEvent::OpenPrivacy => cx.emit(MailEvent::OpenSettings(Section::Privacy)),
            ReaderEvent::Compose(m) => self.compose_mailto(None, m.clone(), window, cx),
            // "You started a reply": the saved reply, in the pane.
            ReaderEvent::ResumeReply { draft, .. } => {
                self.open_in_pane(Draft::Saved(*draft), window, cx)
            }
            ReaderEvent::EditServerDraft(message) => {
                let message = *message;
                self.composer
                    .update(cx, |c, cx| c.edit_server_draft(message, window, cx));
                let focus = self.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
                cx.notify();
            }
            ReaderEvent::Delete(id) if self.mail.open_id() == Some(*id) => {
                self.act(window, cx, Mail::delete);
            }
            ReaderEvent::Junk { message, junk } if self.mail.open_id() == Some(*message) => {
                if *junk {
                    self.act(window, cx, Mail::report_junk);
                } else {
                    self.act(window, cx, Mail::not_junk);
                }
            }
            _ => {}
        }
    }

    /// The confirm dialog of a question (no default button: the action is
    /// Delete, and Escape or Cancel keep the mail).
    fn ask(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) {
        let n = match question {
            Question::DeleteFrom { .. } => self.mail.pending_deletes(),
        };
        let (title, message, confirm) = text::question(question, n);
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: title.into(),
                message: message.into(),
                confirm: confirm.into(),
                cancel: text::cancel().into(),
                destructive: true,
            },
            move |window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.act(window, cx, Mail::delete_confirmed);
                });
            },
            window,
            cx,
        );
    }

    /// New mail: the composer opens in the reading pane.
    pub fn new_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mail.accounts().is_empty() {
            return;
        }
        self.leave_place(cx);
        let account = self.compose_account();
        self.make_room(cx);
        self.act(window, cx, Mail::close_reader);
        self.composer
            .update(cx, |c, cx| c.open_new(account, window, cx));
        let focus = self.composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// A `mailto:` link, or Write to in Contacts: the composer opens in the
    /// reading pane with the fields filled, from `account` (none: the first
    /// account).
    pub fn compose_mailto(
        &mut self,
        account: Option<i64>,
        mailto: komorebi::mail::render::links::Mailto,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mail.accounts().is_empty() {
            return;
        }
        let account = account.or_else(|| self.compose_account());
        self.make_room(cx);
        self.act(window, cx, Mail::close_reader);
        self.composer
            .update(cx, |c, cx| c.compose_mailto(mailto, account, window, cx));
        let focus = self.composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Ctrl+E, F3, Ctrl+F: the search field takes the focus.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    pub fn go_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_place(cx);
        self.act(window, cx, Mail::go_inbox);
        let rail = self.keys_on_rail(window);
        self.follow_pill(rail);
    }

    /// The hub (shared with the window's tabs and status line).
    pub fn hub(&self) -> &Entity<Hub> {
        &self.hub
    }

    /// What the hub tells the mail view: the sync state for the list and
    /// the reader, and the cache changes the workers made.
    fn on_hub(&mut self, event: &HubEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            HubEvent::Changed => self.take_sync_state(cx),
            HubEvent::React(reaction) => self.react(reaction, window, cx),
            HubEvent::UndoSend(row) => {
                let row = *row;
                if self.composer.read(cx).pending_sends().contains(&row) {
                    // A send from a draft tab or a composer window comes
                    // back there (the Slint app's reopen), not in the
                    // pane: the pane's own draft is left as it is.
                    let own = self.composer.entity_id();
                    let sent_by = self.composer.read(cx).sent_by(row);
                    if sent_by.is_some_and(|by| by != own) {
                        if let Some(row) = self
                            .composer
                            .update(cx, |c, cx| c.unsend_for_elsewhere(row, cx))
                        {
                            cx.emit(MailEvent::OpenDraftTab {
                                row,
                                background: false,
                                sent_by,
                            });
                        }
                    } else {
                        self.composer
                            .update(cx, |c, cx| c.undo_send(row, window, cx));
                    }
                }
            }
            HubEvent::EditAccount { id, .. } => cx.emit(MailEvent::EditAccount(*id)),
            HubEvent::Tick => {}
            HubEvent::WhatsNew => cx.emit(MailEvent::OpenLink(
                crate::views::settings::about::Page::Changelog.url(),
            )),
        }
    }

    /// The list's states, the first-download progress and the reader's
    /// out-of-reach flag, from the hub.
    fn take_sync_state(&mut self, cx: &mut Context<Self>) {
        let (states, fresh, unreachable, syncing, badges) = {
            let hub = self.hub.read(cx);
            let sync = hub.sync();
            let open = self.mail.open().map(|b| b.account_id);
            let badges: HashMap<i64, String> = self
                .mail
                .accounts()
                .iter()
                .filter_map(|a| {
                    // A rejected sign-in first (the Slint NavItem's warn),
                    // then an account out of reach.
                    if hub.attention(a.id) == Some(komorebi::mail::errors::Attention::SignIn) {
                        return Some((a.id, text::sign_in_rejected()));
                    }
                    sync.badge(a.id).then(|| {
                        let said = sync_text::account_status(
                            Some(&State::Unreachable),
                            None,
                            &a.imap.host,
                        );
                        (a.id, said)
                    })
                })
                .collect();
            (
                sync.list_states(),
                sync.first_downloads(),
                open.is_some_and(|a| hub.unreachable(a)),
                sync.syncing(),
                badges,
            )
        };
        self.mail.set_sync(states, fresh);
        self.badges = badges;
        self.reader
            .update(cx, |r, cx| r.set_unreachable(unreachable, cx));
        self.loader.update(cx, |l, cx| l.set_active(syncing, cx));
        cx.notify();
    }

    /// A cache change the workers made, or a send that ended.
    fn react(&mut self, reaction: &Reaction, window: &mut Window, cx: &mut Context<Self>) {
        match reaction {
            Reaction::FoldersChanged | Reaction::RowsChanged(_) | Reaction::Updated => {
                self.act(window, cx, |m| {
                    m.load_folders();
                    m.reload_rows();
                })
            }
            Reaction::BodyReady(id) => {
                let id = *id;
                if self.reader.read(cx).message() == Some(id) {
                    self.reader
                        .update(cx, |r, cx| r.body_arrived(id, window, cx));
                }
            }
            Reaction::Sent { .. } => self.act(window, cx, |m| {
                m.load_folders();
                m.reload_rows();
            }),
            Reaction::BodyFailed { id, problem } => {
                let (id, problem) = (*id, sync_text::problem(problem));
                self.reader
                    .update(cx, |r, cx| r.body_failed(id, &problem, cx));
            }
            Reaction::Warm(_) | Reaction::SyncFolder { .. } | Reaction::DraftCopyLeft(_) => {}
        }
        cx.emit(MailEvent::Changed);
    }

    /// F5: the folders and counts again from the cache, and a sync of
    /// every account.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hub.update(cx, |hub, cx| hub.refresh(cx));
        self.act(window, cx, |m| {
            m.load_folders();
            m.reload_rows();
        });
    }

    /// Files dropped on the mail view: a new message with them attached
    /// (while composing, the composer takes them itself).
    pub fn drop_files(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() || self.composer.read(cx).is_open() {
            return;
        }
        self.new_mail(window, cx);
        if self.composer.read(cx).is_open() {
            self.composer.update(cx, |c, cx| c.attach_paths(paths, cx));
        }
    }

    /// A message row dropped on folder `folder` (drag to folder): the
    /// selection moves when the row is in it, else that row alone (selected
    /// first, never opened); a folder it cannot move to is ignored (another
    /// account's, a Drafts folder, the folder it is in).
    pub fn drop_on_folder(
        &mut self,
        row: i64,
        folder: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mail.lists_messages() {
            return;
        }
        let in_selection =
            self.mail.selection().ids().contains(&row) || self.mail.open_id() == Some(row);
        if !in_selection {
            // Checked before the row is selected, so a drop that cannot
            // move keeps the user's selection.
            if !self.mail.drop_choices(row).iter().any(|o| o.id == folder) {
                return;
            }
            self.act(window, cx, |m| {
                m.row_context(row);
            });
        }
        if self.mail.move_choices().iter().any(|o| o.id == folder) {
            self.act(window, cx, |m| m.move_to(folder));
        }
    }

    /// The one-pane layout's Back to list (Esc): the message closes and the
    /// list shows again with the keys.
    pub fn back_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.act(window, cx, Mail::close_reader);
        window.focus(&self.list_focus, cx);
    }

    /// An account line's Refresh: the folders and counts again from the
    /// cache, and a sync of that account only.
    pub fn refresh_account(&mut self, account: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.hub
            .update(cx, |hub, cx| hub.retry_account(account, cx));
        self.act(window, cx, |m| {
            m.load_folders();
            m.reload_rows();
        });
    }

    /// Ctrl+Shift+E: the folder pane folds to the rail, or opens again.
    pub fn toggle_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let collapsed = !self.mail.nav_collapsed();
        self.act(window, cx, |m| m.set_nav_collapsed(collapsed));
    }

    /// The search field's list keys (Up, Down, Enter, Escape), from the
    /// title bar.
    pub fn search_nav(
        &mut self,
        key: crate::widgets::field::Nav,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        search::navigate(self, key, window, cx);
    }

    /// A snapshot's search (`KOMOREBI_SNAPSHOT_SEARCH`), as if typed.
    pub fn snapshot_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |s, cx| s.set_value(text, window, cx));
        self.act(window, cx, |m| {
            m.search_edited(text);
            m.search_due(m.search_seq);
        });
    }

    /// A snapshot's rows (`KOMOREBI_SNAPSHOT_SELECT`): one opens, several
    /// are selected as Ctrl+click selects them.
    pub fn snapshot_select(&mut self, rows: &[usize], window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<i64> = rows
            .iter()
            .filter_map(|&ix| self.mail.rows().id_at(ix))
            .collect();
        match ids[..] {
            [] => {}
            [one] => self.act(window, cx, |m| m.row_clicked(one)),
            _ => {
                for id in ids {
                    self.act(window, cx, |m| m.row_toggled(id));
                }
            }
        }
    }

    /// The window was drawn: what waited for the first frame.
    fn after_first_frame(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.drawn, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.mail.flush_default_folds();
                this.resume_outbox(cx);
                if std::mem::take(&mut this.nav_follow) {
                    this.nav_scroll.scroll_to_item(this.nav_cursor);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------- list actions

    fn next_row(&mut self, _: &commands::NextRow, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.select_offset(1));
    }

    fn previous_row(&mut self, _: &commands::PreviousRow, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.select_offset(-1));
    }

    fn first_row(&mut self, _: &commands::FirstRow, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.select_edge(false));
    }

    fn last_row(&mut self, _: &commands::LastRow, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.select_edge(true));
    }

    /// The rows a page holds (at least one).
    fn page(&self, cx: &Context<Self>) -> isize {
        let height = self
            .list_scroll
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .height
            .as_f32();
        let row = crate::theme::metrics::scaled::message_slot(cx.tokens());
        ((height / row).floor() as isize).max(1)
    }

    fn page_down(&mut self, _: &commands::PageDownRows, w: &mut Window, cx: &mut Context<Self>) {
        let page = self.page(cx);
        self.act(w, cx, |m| m.select_offset(page));
    }

    fn page_up(&mut self, _: &commands::PageUpRows, w: &mut Window, cx: &mut Context<Self>) {
        let page = self.page(cx);
        self.act(w, cx, |m| m.select_offset(-page));
    }

    fn open_row(&mut self, _: &commands::OpenRow, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| {
            m.open_current();
        });
    }

    fn extend_down(&mut self, _: &commands::ExtendDown, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.extend_by(1));
    }

    fn extend_up(&mut self, _: &commands::ExtendUp, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.extend_by(-1));
    }

    fn select_all(&mut self, _: &commands::SelectAllRows, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, Mail::select_all);
    }

    fn escape(&mut self, _: &commands::EscapeList, w: &mut Window, cx: &mut Context<Self>) {
        let mut done = Escaped::Nothing;
        self.act(w, cx, |m| done = m.escape());
        if done == Escaped::ClearedSearch {
            self.search.update(cx, |s, cx| s.set_value("", w, cx));
        }
        if done == Escaped::ClosedMessage {
            w.focus(&self.list_focus, cx);
        }
        // Nothing else to close: the status line goes, last.
        if done == Escaped::Nothing {
            self.hub.update(cx, |hub, cx| hub.dismiss(cx));
        }
    }

    // ---------------------------------------------- message actions

    fn mark_read(&mut self, _: &commands::MarkRead, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.mark(Flag::Seen, true));
    }

    fn mark_unread(&mut self, _: &commands::MarkUnread, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, |m| m.mark(Flag::Seen, false));
    }

    fn toggle_important(
        &mut self,
        _: &commands::ToggleImportant,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.act(w, cx, Mail::toggle_flag);
    }

    fn archive(&mut self, _: &commands::Archive, w: &mut Window, cx: &mut Context<Self>) {
        // Not in Drafts or the Outbox, whose rows are no messages.
        if self.mail.lists_messages() {
            self.act(w, cx, Mail::archive);
        }
    }

    fn delete(&mut self, _: &commands::Delete, w: &mut Window, cx: &mut Context<Self>) {
        if self.mail.lists_messages() {
            self.act(w, cx, Mail::delete);
        } else if self.mail.view() == model::View::Drafts {
            // Delete on a draft asks first, as Discard does.
            self.discard_draft(w, cx);
        }
    }

    /// A Drafts row's Discard (its menu, Delete): the draft opens in the
    /// composer, which asks first when it holds words.
    pub fn discard_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.act(window, cx, |m| {
            m.open_current();
        });
        if self.composer.read(cx).is_open() {
            self.composer.update(cx, |c, cx| c.discard(window, cx));
        }
    }

    fn undo(&mut self, _: &commands::Undo, w: &mut Window, cx: &mut Context<Self>) {
        self.act(w, cx, Mail::undo);
    }

    fn save_eml_key(&mut self, _: &commands::SaveEml, w: &mut Window, cx: &mut Context<Self>) {
        if self.mail.lists_messages() {
            self.source_command(crate::views::reader::actions::SourceUse::SaveEml, w, cx);
        }
    }

    fn move_to(&mut self, _: &commands::MoveTo, w: &mut Window, cx: &mut Context<Self>) {
        menus::open_move(self, w, cx);
    }

    fn row_menu(&mut self, _: &commands::RowMenu, w: &mut Window, cx: &mut Context<Self>) {
        menus::open_row_menu_at_cursor(self, w, cx);
    }

    /// Reply, Reply all and Forward: a row selected without opening it
    /// opens first, then the composer opens on it (phase 4 writes it).
    /// The account a new message is written from: the open folder's, or the
    /// open message's (All inboxes, Drafts, the Outbox); none: the first.
    fn compose_account(&self) -> Option<i64> {
        self.mail
            .view_account()
            .map(|a| a.id)
            .or_else(|| self.mail.open().map(|b| b.account_id))
    }

    /// The pane's draft makes room for another: one with writing in it is
    /// saved and goes to a tab of its own, behind the shown one (the Slint
    /// app's `pane_draft_aside`). True when the composer was closed so.
    fn make_room(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.composer.read(cx).is_open() {
            return false;
        }
        let row = self.composer.update(cx, |c, cx| c.set_aside(cx));
        if let Some(row) = row {
            cx.emit(MailEvent::OpenDraftTab {
                row,
                background: true,
                sent_by: None,
            });
        }
        !self.composer.read(cx).is_open()
    }

    /// `draft` in the reading pane, with the keyboard.
    fn open_in_pane(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |c, cx| c.open(Some(draft), window, cx));
        let focus = self.composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn respond(
        &mut self,
        draft: impl FnOnce(i64) -> Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mail.open_id().is_none() {
            let ids = self.mail.selection().ids();
            if let [id] = ids[..] {
                self.act(window, cx, |m| {
                    m.select_message(id);
                });
            }
        }
        let Some(id) = self.mail.open_id() else {
            return;
        };
        let draft = draft(id);
        self.composer
            .update(cx, |c, cx| c.open(Some(draft), window, cx));
        let focus = self.composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn reply(&mut self, _: &commands::Reply, w: &mut Window, cx: &mut Context<Self>) {
        self.respond(
            |message| Draft::Reply {
                message,
                all: false,
            },
            w,
            cx,
        );
    }

    fn reply_all(&mut self, _: &commands::ReplyAll, w: &mut Window, cx: &mut Context<Self>) {
        self.respond(|message| Draft::Reply { message, all: true }, w, cx);
    }

    fn forward(&mut self, _: &commands::Forward, w: &mut Window, cx: &mut Context<Self>) {
        let draft = |message| Draft::Forward {
            message,
            attached: false,
        };
        self.respond(draft, w, cx);
    }

    // ------------------------------------------------- pane actions

    fn nav_items(&self, compact: bool) -> nav::Nav {
        if compact {
            self.mail.rail()
        } else {
            self.mail.nav()
        }
    }

    fn nav_step(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        let n = self.nav_items(self.keys_on_rail(window)).items.len();
        if n > 0 {
            self.nav_cursor = self.nav_cursor.saturating_add_signed(delta).min(n - 1);
            self.nav_scroll.scroll_to_item(self.nav_cursor);
            cx.notify();
        }
    }

    fn nav_down(&mut self, _: &commands::NavDown, w: &mut Window, cx: &mut Context<Self>) {
        self.nav_step(1, w, cx);
    }

    fn nav_up(&mut self, _: &commands::NavUp, w: &mut Window, cx: &mut Context<Self>) {
        self.nav_step(-1, w, cx);
    }

    fn nav_first(&mut self, _: &commands::NavFirst, w: &mut Window, cx: &mut Context<Self>) {
        self.nav_step(isize::MIN, w, cx);
    }

    fn nav_last(&mut self, _: &commands::NavLast, w: &mut Window, cx: &mut Context<Self>) {
        self.nav_step(isize::MAX, w, cx);
    }

    /// Enter or Space on a line: it opens (an account line folds), and the
    /// keyboard stays in the pane.
    fn nav_open(&mut self, _: &commands::NavOpen, w: &mut Window, cx: &mut Context<Self>) {
        let rail = self.keys_on_rail(w);
        if let Some(item) = self.nav_items(rail).items.get(self.nav_cursor).cloned() {
            self.nav_clicked(item.kind, rail, w, cx);
            w.focus(&self.nav_focus, cx);
        }
    }

    /// Right: a folded line opens, then the keyboard goes to its first
    /// child (TreeView; not in the rail).
    fn nav_expand(&mut self, _: &commands::NavExpand, w: &mut Window, cx: &mut Context<Self>) {
        if self.keys_on_rail(w) {
            return;
        }
        let items = self.mail.nav().items;
        let i = self.nav_cursor;
        let Some(item) = items.get(i) else {
            return;
        };
        if item.has_children && !item.expanded {
            let kind = item.kind;
            self.act(w, cx, |m| m.nav_toggle(kind));
        } else if item.has_children && items.get(i + 1).is_some_and(|n| n.parent == Some(i)) {
            self.nav_cursor = i + 1;
            cx.notify();
        }
    }

    /// Left: an open line folds, else the keyboard goes to its parent.
    fn nav_collapse(&mut self, _: &commands::NavCollapse, w: &mut Window, cx: &mut Context<Self>) {
        if self.keys_on_rail(w) {
            return;
        }
        let items = self.mail.nav().items;
        let Some(item) = items.get(self.nav_cursor) else {
            return;
        };
        if item.has_children && item.expanded {
            let kind = item.kind;
            self.act(w, cx, |m| m.nav_toggle(kind));
        } else if let Some(parent) = item.parent {
            self.nav_cursor = parent;
            cx.notify();
        }
    }

    fn nav_menu(&mut self, _: &commands::NavMenu, w: &mut Window, cx: &mut Context<Self>) {
        menus::open_nav_menu_at_cursor(self, w, cx);
    }

    /// A line of the pane or a tile of the rail was chosen.
    fn nav_clicked(
        &mut self,
        kind: NavKind,
        rail: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view_before = self.mail.view();
        self.leave_place(cx);
        self.act(window, cx, |m| m.nav_clicked(kind, rail));
        if self.mail.view() != view_before {
            self.pane_open = false;
        }
        if self.mail.view() != view_before {
            self.follow_pill(rail);
        } else if let Some(i) = self
            .nav_items(rail)
            .items
            .iter()
            .position(|r| r.kind == kind)
        {
            self.nav_cursor = i;
        }
    }

    /// The rail's Folders button: the full pane over the content.
    pub fn toggle_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pane_open {
            return self.close_pane(window, cx);
        }
        // Wide enough for the pane: the button shows it again (the user
        // had folded it, Ctrl+Shift+E).
        if self.mail.nav_collapsed() && !compact(window.viewport_size().width.as_f32()) {
            return self.act(window, cx, |m| m.set_nav_collapsed(false));
        }
        self.pane_open = true;
        self.nav_cursor = self.mail.nav().pill.unwrap_or(0);
        window.focus(&self.nav_focus, cx);
        cx.notify();
    }

    pub fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.pane_open) {
            window.focus(&self.list_focus, cx);
            cx.notify();
        }
    }

    fn next_pane(&mut self, _: &commands::NextPane, w: &mut Window, cx: &mut Context<Self>) {
        self.cycle_pane(false, w, cx);
    }

    fn previous_pane(
        &mut self,
        _: &commands::PreviousPane,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_pane(true, w, cx);
    }

    fn close_pane_key(&mut self, _: &commands::ClosePane, w: &mut Window, cx: &mut Context<Self>) {
        if self.pane_open {
            self.close_pane(w, cx);
        } else {
            cx.propagate();
        }
    }

    /// F6 (`back`: Shift+F6): the keyboard goes round the panes, the Slint
    /// `cycle-pane`: the title bar's search, the folders (or the rail),
    /// the list, the reading pane (the composer, or the open message),
    /// skipping the ones that do not show. A dialog keeps the focus.
    pub fn cycle_pane(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        if window.has_active_dialog(cx) {
            return;
        }
        let now = self.pane_of_focus(window, cx);
        let ok = self.pane_stops(window, cx);
        let Some(next) = next_stop(now, &ok, back) else {
            return;
        };
        match next {
            Stop::Search => self.focus_search(window, cx),
            Stop::Folders => window.focus(&self.nav_focus, cx),
            Stop::List => window.focus(&self.list_focus, cx),
            Stop::Reading => {
                let focus = if self.composer.read(cx).is_open() {
                    self.composer.read(cx).focus_handle(cx)
                } else if self.mail.open_id().is_some() {
                    self.reader.read(cx).focus_handle(cx)
                } else {
                    self.reading_focus.clone()
                };
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }

    /// The pane that holds the keyboard focus (none: the list's stop).
    fn pane_of_focus(&self, window: &Window, cx: &gpui::App) -> Stop {
        if self
            .search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            Stop::Search
        } else if self.nav_focus.contains_focused(window, cx) {
            Stop::Folders
        } else if self.reading_focus.contains_focused(window, cx)
            || self
                .composer
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
            || self
                .reader
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        {
            Stop::Reading
        } else {
            Stop::List
        }
    }

    /// Which of the stops show now, in [`STOPS`]' order.
    fn pane_stops(&self, window: &Window, cx: &gpui::App) -> [bool; 4] {
        let one_pane = window.viewport_size().width.as_f32() < ONE_PANE_BELOW_W;
        let composing = self.composer.read(cx).is_open();
        let reading = composing
            || self.mail.open_id().is_some()
            || self.mail.selection().len() > 1
            || self.outbox_open().is_some()
            // The calm panel is the reading pane's stop while it shows.
            || self.calm_now;
        let mail = self.place.is_none();
        [
            // The search shows once there is an account.
            !self.mail.accounts().is_empty(),
            true,
            mail && !(one_pane && reading),
            mail && reading,
        ]
    }

    /// The pane shows as the rail: a narrow window, or folded by the user.
    fn is_rail(&self, window: &Window) -> bool {
        self.mail.nav_collapsed() || compact(window.viewport_size().width.as_f32())
    }

    /// The folder keys move along the rail (not while the full pane is
    /// open over it).
    fn keys_on_rail(&self, window: &Window) -> bool {
        self.is_rail(window) && !self.pane_open
    }
}

impl Focusable for MailView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.list_focus.clone()
    }
}

impl Render for MailView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.after_first_frame(cx);
        if !self.waiting.is_empty() {
            self.window_effects(window, cx);
        }
        // A timer may have changed the open message (no window there).
        self.sync_reader(window, cx);
        self.mail.set_lang(komorebi_i18n::current());
        let width = window.viewport_size().width.as_f32();
        self.compact_now = compact(width);
        let rail = self.is_rail(window);
        let one_pane = width < ONE_PANE_BELOW_W;
        self.sync_placeholder(window, cx);
        let composing = self.composer.read(cx).is_open();
        let reading = composing
            || self.mail.open_id().is_some()
            || self.mail.selection().len() > 1
            || self.outbox_open().is_some();
        let list_empty = self.mail.rows().is_empty() && !composing;
        let place = self.place.as_ref().map(|(_, view)| view.clone());
        let place_none = place.is_none();
        // One pane: the message (or composer) over the list.
        let show_list = !(one_pane && reading);
        let show_reader = !list_empty && (!one_pane || reading);
        let list_w = if compact(width) {
            layout::LIST_W_COMPACT
        } else {
            layout::LIST_W
        };
        // Nothing selected: the list grows and the calm panel (Today or
        // Rest) takes the rest of the room.
        let calm_mode = if place_none {
            calm::mode(&calm::Moment {
                composing,
                open: self.mail.open_id().is_some(),
                selected: self.mail.selection().len(),
                outbox: self.outbox_open().is_some(),
                one_pane,
                list_empty,
                lists_messages: self.mail.lists_messages(),
                searching: self.mail.searching(),
                unread: self.mail.view_unread(),
            })
        } else {
            calm::Mode::Split
        };
        self.calm_now = calm_mode != calm::Mode::Split;
        let nav_w = if rail { layout::RAIL_W } else { layout::NAV_W };
        let calm_w = layout::calm_w(width, nav_w);
        self.shape = panes::Shape {
            rail,
            wide: !show_reader || one_pane || self.calm_now,
            list_w,
        };
        let shape = self.shape;
        let (folders, list) = self.panes(cx);
        div()
            .id("mail")
            .key_context("Mail")
            .on_action(cx.listener(Self::mark_read))
            .on_action(cx.listener(Self::mark_unread))
            .on_action(cx.listener(Self::toggle_important))
            .on_action(cx.listener(Self::archive))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::move_to))
            .on_action(cx.listener(Self::save_eml_key))
            .on_action(cx.listener(Self::reply))
            .on_action(cx.listener(Self::reply_all))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::next_row))
            .on_action(cx.listener(Self::previous_row))
            .on_action(cx.listener(Self::first_row))
            .on_action(cx.listener(Self::last_row))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::open_row))
            .on_action(cx.listener(Self::extend_down))
            .on_action(cx.listener(Self::extend_up))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::row_menu))
            .on_action(cx.listener(Self::nav_down))
            .on_action(cx.listener(Self::nav_up))
            .on_action(cx.listener(Self::nav_first))
            .on_action(cx.listener(Self::nav_last))
            .on_action(cx.listener(Self::nav_open))
            .on_action(cx.listener(Self::nav_expand))
            .on_action(cx.listener(Self::nav_collapse))
            .on_action(cx.listener(Self::nav_menu))
            .on_action(cx.listener(Self::new_folder_key))
            .on_action(cx.listener(Self::rename_folder_key))
            .on_action(cx.listener(Self::delete_folder_key))
            .on_action(cx.listener(Self::close_pane_key))
            .on_action(cx.listener(Self::next_pane))
            .on_action(cx.listener(Self::previous_pane))
            // Files dropped here (not on the composer) start a new message
            // with them attached, as in the Slint app.
            .group(DROP_GROUP)
            .on_drop(
                cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                    this.drop_files(paths.paths().to_vec(), window, cx);
                }),
            )
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .gap(px(space::PANE_GAP))
            .pr(px(space::PANE_GAP))
            .pb(px(space::PANE_GAP))
            .text_color(cx.tokens().color.ink)
            .child(folders.cached(shape.folders_style()))
            .when(place.is_none() && show_list, |el| {
                el.child(list.cached(shape.list_style()))
            })
            .when(place.is_none() && show_reader, |el| {
                if calm_mode == calm::Mode::Split {
                    el.child(reading::render(self, composing, one_pane, window, cx))
                } else {
                    el.child(calm::render(self, calm_mode, calm_w, window, cx))
                }
            })
            .children(place.map(|view| {
                // The status line stays in sight over Settings and Contacts
                // (sync errors, a send's end, the Undo send offer), at the
                // top of their column as in the Slint app.
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .children(status::line(&self.hub, cx))
                    .child(div().flex_1().min_h_0().child(view))
                    .into_any_element()
            }))
            .when(rail && self.pane_open, |el| {
                el.child(folders::overlay(self, window, cx))
            })
            .when(!composing && place_none, |el| el.child(drop_overlay(cx)))
            .children(menus::render_flyout(self, cx))
    }
}
