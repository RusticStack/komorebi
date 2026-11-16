//! The reader (docs/GPUI-MIGRATION.md phase 3): the open message's subject,
//! sender header, notices, invite card, its body and its attachments, in
//! the paper pane the mail view gives it (`ReaderView::new(db, window,
//! cx)`, the slot the list lane mounts).
//!
//! An HTML body is laid out and rasterized by the library's render worker
//! (`komorebi::reader`), in 256 px tiles drawn by the tile element
//! ([`tile`]); the reader's UI-side state is the library's
//! [`Session`], the same logic as the Slint app's reader: the tiles near the
//! view, text selection on the worker (press, drag, double and triple
//! click, Ctrl+A, Ctrl+C, Escape), links (hover and the target strip, Tab
//! focus, Enter, `#anchor` scrolls, `mailto:` to the composer, web links
//! through the confirm prompt, [`links`]), remote images with consent
//! ([`banner`]), dimmed colours in the dark theme and "show original
//! colours", the text view when no page comes within a second. Screen
//! readers get the page's lines as text nodes with bounds ([`a11y`]) and
//! each link near the view as a named link.
//!
//! The reader opens and saves attachments, saves the invitation, asks the
//! RSVP question, explains the sender notice and runs the source commands
//! itself ([`actions`]), so they work in every place it shows. What
//! belongs to other views goes to the owner as a [`ReaderEvent`]: marking
//! read (the sync workers do it), composing, downloading a body, the
//! invite reply's dispatch, the status line, Resume of a reply in Drafts.
//! Keys: the page's in [`CONTEXT`]; the scroll keys and Ctrl+S anywhere in
//! the reader ([`PANE_CONTEXT`]).

pub mod a11y;
pub mod actions;
pub mod attachments;
pub mod banner;
mod format;
pub mod invite;
pub mod links;
#[cfg(test)]
pub(crate) mod testing;
pub mod tile;
pub mod worker;

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui::{
    AccessibleAction, App, Bounds, ClipboardItem, Context, CursorStyle, EventEmitter, FocusHandle,
    Focusable, IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Render, Role, ScrollHandle, SharedString, Styled, Task, Window, canvas,
    div, point, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{
    MoveDown, MoveEnd, MoveHome, MovePageDown, MovePageUp, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use komorebi::db::{self, Account, MarkRead};
use komorebi::i18n::Lang;
use komorebi::invite::{self as inv, Tool};
use komorebi::mail::ical::Answer;
use komorebi::mail::mime::{self, Body};
use komorebi::mail::render::RenderError;
use komorebi::mail::render::links::Mailto;
use komorebi::reader::session::{Effect, Frame, Session};
use komorebi::reader::{
    Backend, FALLBACK_AFTER, RESIZE_DEBOUNCE, ReadWait, SelectOp, Shell, Source, SystemShell,
};
use komorebi::role::Role as FolderRole;
use komorebi::trust;
use komorebi_i18n::tr;

use self::banner::{Bar, Kind};
use self::worker::{Incoming, TileImage};
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, reader, space};
use crate::views::Db;
use crate::widgets::button::{Button, Variant};
use crate::widgets::loader::Loader;
use crate::widgets::spot::SenderSpot;

gpui_kit::actions!(
    reader,
    [
        SelectAll,
        CopySelection,
        ClearSelection,
        NextLink,
        PreviousLink,
        FollowLink,
        LineDown,
        LineUp,
        PageDown,
        PageUp,
        ToTop,
        ToEnd,
        SaveEml
    ]
);

/// The page's key context (it has the keyboard focus after a click).
pub const CONTEXT: &str = "Reader";
/// The whole reader's context: the scroll keys and Ctrl+S, wherever the
/// focus is inside it (the page, the text view, a button), apart from
/// Space, which presses a focused button (it pages only on the page).
pub const PANE_CONTEXT: &str = "ReaderPane";
/// What PageUp and PageDown scroll: 90 percent of the view (the Slint
/// reader's page keys).
const PAGE_SHARE: f32 = 0.9;
/// Raw messages up to this size are parsed on the UI thread, so a text
/// message opens at once; larger ones on the render worker (the Slint
/// app's `SYNC_PARSE_MAX`).
const SYNC_PARSE_MAX: usize = 256 * 1024;
/// A text body longer than this is cut, with a note (`MAX_BODY_BYTES`).
const MAX_BODY_BYTES: usize = 300_000;
/// "Downloading message" shows after this; a server out of reach says so
/// after the second (FLUENT2 "Wait UX", as the Slint reader).
const LOADING_TEXT_AFTER: Duration = Duration::from_secs(1);
const UNREACHABLE_AFTER: Duration = Duration::from_secs(2);
/// Screen readers get the links within this many screens of the view.
const LINK_NODES_SCREENS: f32 = 1.0;

/// The page's keys, in its own context so the window's keys still win
/// (cookbook_actions). Install once at start (`main`, the test harness).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", CopySelection, Some(CONTEXT)),
        KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        KeyBinding::new("tab", NextLink, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PreviousLink, Some(CONTEXT)),
        KeyBinding::new("enter", FollowLink, Some(CONTEXT)),
        KeyBinding::new("space", PageDown, Some(CONTEXT)),
        KeyBinding::new("shift-space", PageUp, Some(CONTEXT)),
        KeyBinding::new("down", LineDown, Some(PANE_CONTEXT)),
        KeyBinding::new("up", LineUp, Some(PANE_CONTEXT)),
        KeyBinding::new("pagedown", PageDown, Some(PANE_CONTEXT)),
        KeyBinding::new("pageup", PageUp, Some(PANE_CONTEXT)),
        KeyBinding::new("home", ToTop, Some(PANE_CONTEXT)),
        KeyBinding::new("end", ToEnd, Some(PANE_CONTEXT)),
        KeyBinding::new("ctrl-s", SaveEml, Some(PANE_CONTEXT)),
    ]);
}

/// What the reader asks of its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum ReaderEvent {
    /// The open message has been open long enough (or at once): mark it
    /// read (the cache and the server; the sync workers do that).
    MarkRead(i64),
    /// A `mailto:` link: a prefilled composer.
    Compose(Mailto),
    /// The open message's body is not cached: download it.
    FetchBody(i64),
    /// Retry the download of a body whose server was out of reach.
    RetryBody(i64),
    /// The invite reply was queued in the outbox: send it.
    ReplyQueued {
        row: i64,
        account: i64,
    },
    /// A sentence for the status line.
    Status {
        text: String,
        error: bool,
    },
    /// "Privacy settings" (remote images).
    OpenPrivacy,
    /// The sender notice's Report junk (`true`) or Not junk.
    Junk {
        message: i64,
        junk: bool,
    },
    Delete(i64),
    /// A server draft's Edit.
    EditServerDraft(i64),
    /// Resume on "You started a reply to this message": the reply saved
    /// in Drafts (`drafts` row `draft`) to message `message`.
    ResumeReply {
        message: i64,
        draft: i64,
    },
}

struct Open {
    msg: db::Message,
    body: Option<Body>,
}

/// The reader view.
pub struct ReaderView {
    db: Db,
    session: Session<TileImage>,
    open: Option<Open>,
    accounts: Vec<Account>,
    mark_read: MarkRead,
    read_wait: ReadWait,
    /// The page's keyboard focus.
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The sideways scroll of a page wider than the column.
    hscroll: ScrollHandle,
    /// The body column's bounds (the page's top-left and the column width),
    /// and the page's own bounds, from the last prepaint.
    column: Rc<Cell<Option<Bounds<Pixels>>>>,
    page_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The page's lines of text, for screen readers.
    lines: Rc<Vec<komorebi::mail::render::TextLine>>,
    invite_change: bool,
    invite_details: bool,
    /// The image bar was closed for the message shown.
    images_closed: bool,
    /// "Always load from this sender" was just pressed for this address.
    remote_always: Option<String>,
    /// The open message's sender is allowed (the caption).
    remote_allowed: bool,
    /// The open message's server is out of reach (from the owner).
    unreachable: bool,
    /// The waits under "not downloaded yet": the ring's words, the
    /// out-of-reach sentence.
    loading_text: bool,
    unreachable_late: bool,
    waits: u64,
    /// A link's confirm prompt is open.
    prompting: bool,
    loader: gpui::Entity<Loader>,
    /// Opens attachments in their default app.
    opener: actions::Opener,
    /// A source command (More menu) waits for this message's body.
    source_wait: Option<(i64, actions::SourceUse)>,
    /// The text view: a read-only multi-line field, so it can be selected
    /// and copied, and the message and a hash of the text it holds.
    text: Option<gpui::Entity<TextareaState>>,
    text_of: Option<(i64, u64)>,
    /// The owner opens a saved reply on [`ReaderEvent::ResumeReply`]: only
    /// then does "You started a reply to this message" show.
    resume: bool,
    _pump: Task<()>,
}

impl EventEmitter<ReaderEvent> for ReaderView {}

impl ReaderView {
    /// The reader over `db` (the window's connection to the cache, shared
    /// with the mail view), with the render worker that starts on the
    /// first open.
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A test that planted a fake render worker gets it (once).
        #[cfg(test)]
        if let Some((backend, shell, rx)) = testing::take() {
            return Self::with_backend(db, backend, shell, rx, window, cx);
        }
        let (worker, rx) = worker::start(None, worker::network_fetch());
        Self::with_backend(db, Box::new(worker), Box::new(SystemShell), rx, window, cx)
    }

    /// The reader with another backend and shell (tests: a fake worker, no
    /// browser).
    pub fn with_backend(
        db: Db,
        backend: Box<dyn Backend>,
        shell: Box<dyn Shell>,
        mut rx: futures::channel::mpsc::UnboundedReceiver<Incoming>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pump = cx.spawn(async move |this, cx| {
            while let Some(incoming) = rx.next().await {
                if this
                    .update(cx, |r, cx| r.on_incoming(incoming, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let accounts = db.accounts().unwrap_or_default();
        let mark_read = db
            .settings()
            .map(|s| s.mark_read)
            .unwrap_or(MarkRead::After(Duration::from_secs(1)));
        ReaderView {
            db,
            session: Session::new(backend, shell),
            open: None,
            accounts,
            mark_read,
            read_wait: ReadWait::default(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            hscroll: ScrollHandle::new(),
            column: Rc::default(),
            page_bounds: Rc::default(),
            lines: Rc::default(),
            invite_change: false,
            invite_details: false,
            images_closed: false,
            remote_always: None,
            remote_allowed: false,
            unreachable: false,
            loading_text: false,
            unreachable_late: false,
            waits: 0,
            prompting: false,
            loader: cx.new(|_| Loader::new(tr!("Downloading message\u{a0}…"))),
            opener: actions::system_opener(),
            source_wait: None,
            text: None,
            text_of: None,
            resume: false,
            _pump: pump,
        }
    }

    // ---------------------------------------------------------- reading

    /// The message shown.
    pub fn message(&self) -> Option<i64> {
        self.open.as_ref().map(|o| o.msg.id)
    }

    pub fn session(&self) -> &Session<TileImage> {
        &self.session
    }

    /// How far the page is scrolled down (logical px), for tests.
    #[cfg(test)]
    pub(crate) fn scrolled(&self) -> f32 {
        -f32::from(self.scroll.offset().y)
    }

    /// What the view shows is complete: no render pending, and the page's
    /// tiles in view are all drawn (for snapshots, which wait for it).
    pub fn settled(&self) -> bool {
        if self.open.is_none() {
            return true;
        }
        if self.session.waiting()
            || self.session.partial()
            || self.open.as_ref().is_some_and(|o| o.msg.raw.is_none())
        {
            return false;
        }
        let (Some(page), true) = (self.session.page(), self.session.page_shown()) else {
            return true;
        };
        let frame = self.session.frame();
        tile::visible(frame.top, frame.height, page.height).is_none_or(|(a, b)| {
            (a..=b).all(|i| {
                self.session
                    .tiles()
                    .get(i as usize)
                    .is_some_and(|t| t.image.is_some())
            })
        })
    }

    /// The laid-out page shows, not the text view (the bench's check).
    pub fn page_view(&self) -> bool {
        self.session.page_shown()
    }

    /// Tiles in view with nothing painted yet (the bench's blank count).
    pub fn blank_tiles(&self) -> u32 {
        let (Some(page), true) = (self.session.page(), self.session.page_shown()) else {
            return 0;
        };
        let frame = self.session.frame();
        tile::visible(frame.top, frame.height, page.height).map_or(0, |(a, b)| {
            (a..=b)
                .filter(|&i| {
                    self.session
                        .tiles()
                        .get(i as usize)
                        .is_none_or(|t| t.image.is_none())
                })
                .count() as u32
        })
    }

    /// Every tile of the read-ahead window is painted (the bench's complete).
    pub fn ahead_ready(&self) -> bool {
        if !self.settled() {
            return false;
        }
        let (Some(page), true) = (self.session.page(), self.session.page_shown()) else {
            return true;
        };
        let frame = self.session.frame();
        komorebi::reader::wanted_tiles(frame.top, frame.height, page.height)
            .into_iter()
            .all(|i| {
                self.session
                    .tiles()
                    .get(i as usize)
                    .is_some_and(|t| t.image.is_some())
            })
    }

    /// Scroll for the bench; returns how far the view moved.
    pub fn bench_read_step(&mut self, dy: f32, cx: &mut Context<Self>) -> f32 {
        let before = f32::from(self.scroll.offset().y);
        self.scroll_by(dy, cx);
        before - f32::from(self.scroll.offset().y)
    }

    /// The open message's parsed body, once there is one.
    pub fn body(&self) -> Option<&Body> {
        self.open.as_ref()?.body.as_ref()
    }

    /// The accounts changed (the invite card's attendee, the sender
    /// notice's server).
    pub fn set_accounts(&mut self, accounts: Vec<Account>, cx: &mut Context<Self>) {
        self.accounts = accounts;
        cx.notify();
    }

    /// The mark-read setting changed.
    pub fn set_mark_read(&mut self, mode: MarkRead) {
        self.mark_read = mode;
    }

    /// Whether the open message's server is out of reach (a body that is
    /// not downloaded then says so, with Retry).
    pub fn set_unreachable(&mut self, unreachable: bool, cx: &mut Context<Self>) {
        if self.unreachable != unreachable {
            self.unreachable = unreachable;
            cx.notify();
        }
    }

    // ---------------------------------------------------------- opening

    /// Make message `id` the open one: its body, or a download of it, and
    /// its read delay. False when it is not cached.
    pub fn open(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) -> bool {
        crate::bench::open_begin();
        // The key reads the theme and scale as they are now.
        self.sync_frame(window, cx);
        let mut msg = match self.db.message(id) {
            Ok(Some(m)) => m,
            Ok(None) => return false,
            Err(e) => {
                cx.emit(ReaderEvent::Status {
                    text: format!("{e:#}"),
                    error: true,
                });
                return false;
            }
        };
        let other = self.message() != Some(id);
        if other {
            // The "will always load" bar belongs to the message it was
            // pressed on; "View as plain text" and Change last for one
            // message.
            self.remote_always = None;
            self.remote_allowed = false;
            self.invite_change = false;
            self.invite_details = false;
            self.images_closed = false;
            self.scroll.set_offset(point(Pixels::ZERO, Pixels::ZERO));
            self.hscroll.set_offset(point(Pixels::ZERO, Pixels::ZERO));
            // A source command waits for the message it was asked on.
            self.source_wait = None;
        }
        match self.read_wait.opened(id, !msg.seen, self.mark_read) {
            Some(Duration::ZERO) => {
                cx.emit(ReaderEvent::MarkRead(id));
                msg.seen = true;
            }
            Some(delay) => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(delay).await; // timer-exempt: the mark-read delay the setting asks for
                    let _ = this.update(cx, |r, cx| r.read_due(cx));
                })
                .detach();
            }
            None => {}
        }
        let fetch = msg.raw.is_none();
        self.open = Some(Open { msg, body: None });
        if fetch {
            self.session.close();
            self.start_waits(cx);
            cx.emit(ReaderEvent::FetchBody(id));
        } else {
            self.load_body(window, cx);
        }
        self.after(window, cx);
        cx.notify();
        true
    }

    /// The open message's body arrived (downloaded): show it.
    pub fn body_arrived(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.message() != Some(id) {
            return;
        }
        if let Ok(Some(m)) = self.db.message(id)
            && let Some(open) = self.open.as_mut()
        {
            open.msg.raw = m.raw;
        }
        self.load_body(window, cx);
        self.after(window, cx);
        self.source_arrived(id, window, cx);
        cx.notify();
    }

    /// Nothing is shown any more.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.source_wait = None;
        self.read_wait.cancel();
        self.session.close();
        self.lines = Rc::default();
        self.after(window, cx);
        cx.notify();
    }

    fn read_due(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.read_wait.due(self.message()) {
            if let Some(open) = self.open.as_mut() {
                open.msg.seen = true;
            }
            cx.emit(ReaderEvent::MarkRead(id));
        }
    }

    /// The ring's words after a second; out of reach, the sentence after
    /// two (a newer wait replaces an older one).
    fn start_waits(&mut self, cx: &mut Context<Self>) {
        self.waits += 1;
        self.loading_text = false;
        self.unreachable_late = false;
        let token = self.waits;
        self.loader.update(cx, |l, cx| l.set_active(true, cx));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOADING_TEXT_AFTER).await; // timer-exempt: the wait UX's first second
            let _ = this.update(cx, |r, cx| {
                if r.waits == token {
                    r.loading_text = true;
                    cx.notify();
                }
            });
            cx.background_executor()
                .timer(UNREACHABLE_AFTER - LOADING_TEXT_AFTER) // timer-exempt: the out-of-reach sentence's delay
                .await;
            let _ = this.update(cx, |r, cx| {
                if r.waits == token {
                    r.unreachable_late = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Show the open message's body: text at once, or HTML through the
    /// render worker (which also parses large messages off this thread).
    fn load_body(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.loader.update(cx, |l, cx| l.set_active(false, cx));
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some(raw) = open.msg.raw.clone() else {
            return;
        };
        let id = open.msg.id;
        // Keyed on the From address alone, never the display name.
        let remote = self.db.remote_allowed(&open.msg.from_addr).unwrap_or(false);
        self.remote_allowed = remote;
        if raw.len() > SYNC_PARSE_MAX {
            self.session.open(id, Source::Raw(raw.into()), remote);
            return;
        }
        let (body, input) = mime::parse_reader(&raw);
        let backfill = backfill_importance(&mut open.msg, &body);
        open.body = Some(body);
        if input.prefers_plain {
            self.session.close();
        } else {
            self.session
                .open(id, Source::Input(std::sync::Arc::new(input)), remote);
        }
        self.keep_importance(backfill, cx);
    }

    /// A body cached before v13 told the open message's importance: its
    /// row keeps it (one row, only this once).
    fn keep_importance(&self, backfill: Option<(i64, db::Importance)>, cx: &mut Context<Self>) {
        if let Some((id, importance)) = backfill
            && let Err(e) = self.db.set_importance(id, importance)
        {
            cx.emit(ReaderEvent::Status {
                text: format!("{e:#}"),
                error: true,
            });
        }
    }

    fn on_incoming(&mut self, incoming: Incoming, cx: &mut Context<Self>) {
        match incoming {
            Incoming::Tile {
                generation,
                index,
                image,
            } => {
                self.session.show_tile(generation, index, image);
                if crate::bench::enabled() {
                    crate::bench::open_event("first_tile");
                }
            }
            Incoming::Reply(reply) => {
                let text = matches!(reply, komorebi::reader::Reply::Text { .. });
                let page = matches!(reply, komorebi::reader::Reply::Page { .. });
                let body = self.session.on_reply(reply, |p| {
                    worker::image(&p).unwrap_or_else(|| worker::image(&blank()).expect("blank"))
                });
                if text {
                    self.lines = Rc::new(self.session.lines().to_vec());
                }
                if page && crate::bench::enabled() {
                    crate::bench::open_event("page");
                }
                if let Some(body) = body
                    && let Some(open) = self.open.as_mut()
                    && open.body.is_none()
                {
                    let backfill = backfill_importance(&mut open.msg, &body);
                    open.body = Some(body);
                    self.keep_importance(backfill, cx);
                    if crate::bench::enabled() {
                        crate::bench::open_event("parsed");
                    }
                }
            }
        }
        self.after_effects(cx);
        if crate::bench::enabled() && self.settled() {
            crate::bench::open_event("settled");
        }
        cx.notify();
    }

    // ---------------------------------------------------------- effects

    /// After a session call with the window at hand: its effects, the
    /// confirm prompt, the tiles to release.
    fn after(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.after_effects(cx);
        if self.session.prompt().is_some() && !self.prompting {
            self.prompting = true;
            let view = cx.entity().downgrade();
            if let Some(prompt) = self.session.prompt().cloned() {
                links::confirm(
                    &prompt,
                    move |open, _window, cx| {
                        let _ = view.update(cx, |r, cx| {
                            r.prompting = false;
                            if open {
                                r.session.open_prompt();
                            } else {
                                r.session.cancel_prompt();
                            }
                            cx.notify();
                        });
                    },
                    window,
                    cx,
                );
            }
        }
    }

    fn after_effects(&mut self, cx: &mut Context<Self>) {
        for effect in self.session.take_effects() {
            match effect {
                Effect::ScrollTo(top) => self.scroll_to(top),
                Effect::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                Effect::Mailto(m) => cx.emit(ReaderEvent::Compose(m)),
                Effect::Fallback(token) => {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(FALLBACK_AFTER).await; // timer-exempt: the text view stands in after a second
                        let _ = this.update(cx, |r, cx| {
                            r.session.fallback_due(token);
                            cx.notify();
                        });
                    })
                    .detach();
                }
                Effect::Debounce(token) => {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(RESIZE_DEBOUNCE).await; // timer-exempt: the relayout waits for resizing to pause
                        let _ = this.update(cx, |r, cx| {
                            r.session.resize_due(token);
                            r.after_effects(cx);
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }
        }
        let dropped = self.session.take_dropped();
        if !dropped.is_empty() {
            // Out of the sprite atlas of every window, after this update.
            cx.defer(move |cx| {
                for image in dropped {
                    cx.drop_image(image, None);
                }
            });
        }
    }

    /// Scroll so that page y `top` is at the top of the view (as far as
    /// the content allows).
    fn scroll_to(&self, top: f32) {
        let Some(column) = self.column.get() else {
            return;
        };
        let view = self.scroll.bounds();
        let offset = self.scroll.offset();
        let max = self.scroll.max_offset();
        // Where the page starts in the content.
        let page_top = f32::from(column.origin.y - view.origin.y - offset.y);
        let y = (-(page_top + top)).clamp(-f32::from(max.y), 0.0);
        self.scroll.set_offset(point(offset.x, px(y)));
    }

    /// The view measured itself (or the theme changed): the visible band.
    fn sync_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(column) = self.column.get() else {
            return;
        };
        let view = self.scroll.bounds();
        let height = f32::from(view.size.height);
        let frame = Frame {
            column: f32::from(column.size.width),
            scale: window.scale_factor(),
            dark: cx.tokens().scheme.is_dark(),
            top: f32::from(view.origin.y - column.origin.y),
            height,
        };
        if frame != self.session.frame() {
            self.session.set_frame(frame);
            self.after_effects(cx);
            cx.notify();
        }
    }

    // ---------------------------------------------------------- commands

    /// Load the open message's remote images: this once, or from now on for
    /// its sender's exact address.
    pub fn load_remote(&mut self, always: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        if always {
            let addr = open.msg.from_addr.clone();
            if let Err(e) = self.db.allow_remote(&addr) {
                cx.emit(ReaderEvent::Status {
                    text: format!("{e:#}"),
                    error: true,
                });
                return;
            }
            self.remote_always = Some(addr);
        }
        self.session.allow_remote();
        self.after(window, cx);
        cx.notify();
    }

    /// Undo on "Images from … will always load".
    pub fn undo_remote_always(&mut self, cx: &mut Context<Self>) {
        let Some(addr) = self.remote_always.take() else {
            return;
        };
        if let Err(e) = self.db.forget_remote(&addr) {
            cx.emit(ReaderEvent::Status {
                text: format!("{e:#}"),
                error: true,
            });
        }
        self.remote_allowed = false;
        cx.notify();
    }

    /// "View as plain text" on or off, for the message shown.
    pub fn set_plain(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.session.set_plain(on);
        self.after(window, cx);
        cx.notify();
    }

    /// "Show original colours" (dark theme).
    pub fn show_original(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.session.show_original(on);
        self.after(window, cx);
        cx.notify();
    }

    /// Follow an address that is not a link on the page (Unsubscribe, an
    /// invite's Join): the composer for `mailto:`, the prompt for the web.
    pub fn follow(&mut self, href: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.session.follow(href, "");
        self.after(window, cx);
        cx.notify();
    }

    /// Answer the open invitation (message `msg`; the owner asked first):
    /// record the answer and, with `send`, queue the reply.
    pub fn answer_invite(&mut self, msg: i64, answer: Answer, send: bool, cx: &mut Context<Self>) {
        let Some(open) = self.open.as_ref().filter(|o| o.msg.id == msg) else {
            return;
        };
        let Some(ev) = open.body.as_ref().and_then(|b| b.invite.as_deref()) else {
            return;
        };
        let Some(account) = self.accounts.iter().find(|a| a.id == open.msg.account_id) else {
            return;
        };
        if send {
            match inv::queue_reply(&self.db, account, &open.msg, ev, answer) {
                Ok(Some(row)) => cx.emit(ReaderEvent::ReplyQueued {
                    row,
                    account: account.id,
                }),
                Ok(None) => return,
                Err(e) => {
                    let text = tr!("Couldn't send the reply: {0}", format!("{e:#}"));
                    cx.emit(ReaderEvent::Status { text, error: true });
                    return;
                }
            }
        }
        // Kept for the card only: Komorebi 1.0 has no calendar to store it in.
        let status = match inv::record_answer(&self.db, account, ev, answer) {
            Err(e) => (format!("{e:#}"), true),
            Ok(()) if send => (
                tr!("Your reply is in the Outbox; only the organizer gets it."),
                false,
            ),
            Ok(()) => (
                tr!("Answer kept on this computer; no reply was sent."),
                false,
            ),
        };
        cx.emit(ReaderEvent::Status {
            text: status.0,
            error: status.1,
        });
        self.invite_change = false;
        cx.notify();
    }

    fn invite_press(&mut self, press: invite::Press, window: &mut Window, cx: &mut Context<Self>) {
        if self.message().is_none() {
            return;
        }
        match press {
            invite::Press::Answer(answer) => self.ask_rsvp(answer, window, cx),
            invite::Press::Tool(Tool::Join) => {
                let join = self
                    .body()
                    .and_then(|b| b.invite.as_ref())
                    .map(|ev| ev.join.clone())
                    .unwrap_or_default();
                if !join.is_empty() {
                    self.follow(&join, window, cx);
                }
            }
            invite::Press::Tool(Tool::Change) => self.invite_change = true,
            invite::Press::Tool(Tool::Details) => self.invite_details = !self.invite_details,
            invite::Press::Tool(Tool::SaveIcs) => self.save_invite(cx),
            // Komorebi 1.0 has no calendar: the card never offers it.
            invite::Press::Tool(Tool::Remove) => {}
        }
        cx.notify();
    }

    // ---------------------------------------------------------- pointer

    /// A window position in page coordinates.
    fn page_point(&self, at: gpui::Point<Pixels>) -> Option<(f32, f32)> {
        let b = self.page_bounds.get()?;
        Some((f32::from(at.x - b.origin.x), f32::from(at.y - b.origin.y)))
    }

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Any press focuses the page, so Ctrl+C works after Select all.
        window.focus(&self.focus, cx);
        let Some((x, y)) = self.page_point(e.position) else {
            return;
        };
        if e.modifiers.shift {
            self.session.extend(x, y);
        } else {
            self.session.press(x, y, Instant::now());
        }
        self.after(window, cx);
    }

    fn on_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((x, y)) = self.page_point(e.position) else {
            return;
        };
        if e.pressed_button == Some(MouseButton::Left) {
            self.session.extend(x, y);
            self.after(window, cx);
        } else if self.session.hover_at(Some((x, y))) {
            cx.notify();
        }
    }

    fn on_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.modifiers.shift {
            return;
        }
        if let Some((x, y)) = self.page_point(e.position) {
            self.session.release(x, y);
            self.after(window, cx);
            cx.notify();
        }
    }

    // ---------------------------------------------------------- keys

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.session.select(SelectOp::All);
        self.after(window, cx);
    }

    fn copy(&mut self, _: &CopySelection, window: &mut Window, cx: &mut Context<Self>) {
        self.session.select(SelectOp::Copy);
        self.after(window, cx);
    }

    /// Escape on the page: the selection and the link focus go. With
    /// neither, Escape is the owner's (Back to the list, closing a message
    /// window).
    fn clear(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.selection().is_empty() && self.session.focused_link().is_none() {
            cx.propagate();
            return;
        }
        self.session.clear();
        self.after(window, cx);
        cx.notify();
    }

    fn next_link(&mut self, _: &NextLink, window: &mut Window, cx: &mut Context<Self>) {
        // Off the last link, Tab moves on out of the page.
        if !self.session.focus_next(false) {
            window.focus_next(cx);
        }
        self.after(window, cx);
        cx.notify();
    }

    fn previous_link(&mut self, _: &PreviousLink, window: &mut Window, cx: &mut Context<Self>) {
        if !self.session.focus_next(true) {
            window.focus_prev(cx);
        }
        self.after(window, cx);
        cx.notify();
    }

    /// Enter on the focused link. With no link focused, Enter is the
    /// owner's (it never stops at the page).
    fn follow_link(&mut self, _: &FollowLink, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.focused_link().is_none() {
            cx.propagate();
            return;
        }
        self.session.activate_focused();
        self.after(window, cx);
        cx.notify();
    }

    /// Scroll the reader by `dy` (down when positive), within the content.
    pub fn scroll_by(&mut self, dy: f32, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        let max = f32::from(self.scroll.max_offset().y);
        let y = (f32::from(offset.y) - dy).clamp(-max, 0.0);
        self.scroll.set_offset(point(offset.x, px(y)));
        cx.notify();
    }

    /// The view's height (what a page key scrolls a share of).
    fn view_height(&self) -> f32 {
        f32::from(self.scroll.bounds().size.height)
    }

    fn line_down(&mut self, _: &LineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(reader::SCROLL_LINE, cx);
    }

    fn line_up(&mut self, _: &LineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(-reader::SCROLL_LINE, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(self.view_height() * PAGE_SHARE, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(-self.view_height() * PAGE_SHARE, cx);
    }

    fn scroll_top(&mut self, _: &ToTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(f32::MIN, cx);
    }

    fn scroll_end(&mut self, _: &ToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(f32::MAX, cx);
    }

    fn save_eml_key(&mut self, _: &SaveEml, window: &mut Window, cx: &mut Context<Self>) {
        self.source(actions::SourceUse::SaveEml, window, cx);
    }

    /// The text view's field, holding the text shown (set again only when
    /// the message or the text changes, so a selection survives a frame).
    fn text_field(
        &mut self,
        id: i64,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<TextareaState> {
        let state = self
            .text
            .get_or_insert_with(|| {
                cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, usize::MAX))
            })
            .clone();
        let of = Some((id, text_hash(&text)));
        if self.text_of != of {
            self.text_of = of;
            state.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        state
    }

    /// A movement key of the text view's field: the reader scrolls by `dy`
    /// and the field's caret stays (it is not shown).
    fn text_key(&mut self, dy: f32, cx: &mut Context<Self>) {
        if self.text.is_some() && self.session.text_shown() && self.open.is_some() {
            self.scroll_by(dy, cx);
            cx.stop_propagation();
        }
    }

    /// The owner opens the saved reply on [`ReaderEvent::ResumeReply`]:
    /// "You started a reply to this message" shows from now on.
    pub fn handle_resume(&mut self, cx: &mut Context<Self>) {
        self.resume = true;
        cx.notify();
    }

    /// The text view's field (tests).
    pub fn text_view(&self) -> Option<&gpui::Entity<TextareaState>> {
        self.text.as_ref()
    }
}

impl Focusable for ReaderView {
    /// The page's, or the text view's field while it stands in.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.text {
            Some(text) if self.session.text_shown() && self.open.is_some() => {
                text.read(cx).focus_handle(cx)
            }
            _ => self.focus.clone(),
        }
    }
}

/// What the text view's field is keyed on: the text itself, hashed.
fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// A body cached before v13 tells the importance its sender set (the
/// Slint app's `backfill_importance`).
fn backfill_importance(msg: &mut db::Message, body: &Body) -> Option<(i64, db::Importance)> {
    if msg.importance != db::Importance::Normal || body.importance == db::Importance::Normal {
        return None;
    }
    msg.importance = body.importance;
    Some((msg.id, body.importance))
}

/// A one-pixel tile, for a reply the view never gets (tiles come
/// converted on the worker).
fn blank() -> komorebi::reader::Pixels {
    komorebi::reader::Pixels::new(1, 1, std::sync::Arc::from(vec![0u8; 4])).expect("1 x 1")
}

/// What the note above a text body standing in for a failed render says.
fn failed_note(error: RenderError) -> Option<String> {
    match error {
        RenderError::TooLarge => Some(tr!(
            "This message is too large to show formatted, so it is shown as plain text."
        )),
        RenderError::Deadline => Some(tr!(
            "This message took too long to lay out, so it is shown as plain text."
        )),
        RenderError::Malformed | RenderError::NoHtml => None,
    }
}

/// The sender notice's sentence (the Slint app's `trust_notice`).
pub(crate) fn trust_notice(reasons: &[trust::Reason]) -> String {
    use trust::Reason as R;
    let one = |r: &R| -> String {
        match r {
            R::Dmarc => tr!("it failed your server's sender check (DMARC)"),
            R::SpfDkim => tr!("it failed your server's sender checks (SPF and DKIM)"),
            R::NameDomain { shown, real } => {
                tr!("the name shows {0}, but the address is at {1}", shown, real)
            }
            R::Punycode { ascii, unicode } => {
                tr!(
                    "{0} is the international name \u{201c}{1}\u{201d}",
                    ascii,
                    unicode
                )
            }
            R::MixedScript { domain, script } => {
                let name = match *script {
                    "Cyrillic" => tr!("Cyrillic"),
                    "Greek" => tr!("Greek"),
                    other => other.to_string(),
                };
                tr!("{0} mixes Latin and {1} letters", domain, name)
            }
        }
    };
    match reasons {
        [] => String::new(),
        [a] => tr!("Reason: {0}.", one(a)),
        [a, b, ..] => tr!("Reasons: {0}; {1}.", one(a), one(b)),
    }
}

/// The encrypted notice.
fn crypt_notice(kind: &str) -> String {
    tr!(
        "This message is encrypted ({0}). Komorebi cannot decrypt it; save the encrypted file to open it in a program that has the key.",
        kind
    )
}

impl ReaderView {
    fn header(&self, open: &Open, lang: Lang, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let m = &open.msg;
        let from = format::sender(&m.from_name, &m.from_addr);
        let to = format::recipients(&m.to);
        let cc = format::recipients(&m.cc);
        let to_line = if cc.is_empty() {
            tr!("To: {0}", to)
        } else {
            tr!("To: {0}   Cc: {1}", to, cc)
        };
        let date = format::reader_date(lang, m.date);
        let importance = match m.importance {
            db::Importance::High => Some(tr!("Sent with high importance")),
            db::Importance::Low => Some(tr!("Sent with low importance")),
            db::Importance::Normal => None,
        };
        let unsubscribe = open
            .body
            .as_ref()
            .is_some_and(|b| !b.unsubscribe.is_empty());
        let view = cx.entity().downgrade();
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(reader::HEADER_GAP))
            .mt(px(reader::HEADER_TOP))
            .mb(px(reader::HEADER_BOTTOM - reader::BLOCK_GAP))
            .child(SenderSpot::new(&from).size(layout::AVATAR_MD))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_baseline()
                            .gap(px(space::SP_1))
                            .min_w_0()
                            .child(
                                div()
                                    .id("reader-from")
                                    .role(Role::Label)
                                    .aria_label(tr!("From: {0}", from))
                                    .typo(&t.text.body_strong)
                                    .whitespace_nowrap()
                                    .child(from.clone())
                                    .test_support(),
                            )
                            .child(
                                div()
                                    .id("reader-address")
                                    .role(Role::Label)
                                    .aria_label(tr!("Sender address: {0}", m.from_addr))
                                    .min_w_0()
                                    .typo(&t.text.caption)
                                    .text_color(c.ink_3)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(m.from_addr.clone())
                                    .test_support(),
                            ),
                    )
                    .child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child(to_line),
                    )
                    .when_some(importance, |el, words| {
                        el.child(
                            div()
                                .id("reader-importance")
                                .role(Role::Label)
                                .aria_label(words.clone())
                                .typo(&t.text.caption)
                                .text_color(c.ink_2)
                                .child(words)
                                .test_support(),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(space::SP_1))
                    .child(
                        div()
                            .id("reader-date")
                            .role(Role::Label)
                            .aria_label(tr!("Date: {0}", date))
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .whitespace_nowrap()
                            .child(date.clone())
                            .test_support(),
                    )
                    .when(unsubscribe, |el| {
                        el.child(
                            Button::new("reader-unsubscribe")
                                .variant(Variant::Subtle)
                                .label(tr!("Unsubscribe"))
                                .on_click(move |_, window, cx| {
                                    let _ = view.update(cx, |r, cx| {
                                        let uri = r
                                            .body()
                                            .map(|b| b.unsubscribe.clone())
                                            .unwrap_or_default();
                                        if !uri.is_empty() {
                                            r.follow(&uri, window, cx);
                                        }
                                    });
                                }),
                        )
                    }),
            )
    }

    /// The bars above the body: a server draft, the sender notice and the
    /// encrypted state.
    fn notices(&self, open: &Open, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut out = Vec::new();
        let m = &open.msg;
        let id = m.id;
        let role = self
            .db
            .folder(m.folder_id)
            .ok()
            .flatten()
            .map_or(FolderRole::Other, |f| f.role);
        let view = cx.entity().downgrade();
        // A reply to this message saved in Drafts: Resume opens it (the
        // Slint bar's Resume is Reply, which picks the draft up).
        // Not on a server draft (its own bar says Edit), and only when the
        // owner opens the reply ([`ReaderView::handle_resume`]).
        if self.resume
            && role != FolderRole::Drafts
            && let Ok(Some(draft)) = self.db.draft_for_message(id)
        {
            let v = view.clone();
            out.push(
                Bar::new(
                    "reader-draft-reply",
                    Kind::Info,
                    tr!("You started a reply to this message."),
                )
                .action("reader-resume", tr!("Resume"), move |_, cx| {
                    let _ = v.update(cx, |_, cx| {
                        cx.emit(ReaderEvent::ResumeReply { message: id, draft })
                    });
                })
                .into_any_element(),
            );
        }
        if role == FolderRole::Drafts {
            let v = view.clone();
            out.push(
                Bar::new(
                    "reader-server-draft",
                    Kind::Info,
                    tr!(
                        "This draft was saved on another device or in webmail. To keep writing it here, select Edit."
                    ),
                )
                .action("reader-edit-draft", tr!("Edit"), move |_, cx| {
                    let _ = v.update(cx, |_, cx| cx.emit(ReaderEvent::EditServerDraft(id)));
                })
                .into_any_element(),
            );
        }
        let Some(body) = &open.body else {
            return out;
        };
        let host = self
            .accounts
            .iter()
            .find(|a| a.id == m.account_id)
            .map_or("", |a| a.imap.host.as_str());
        let reasons = trust::reasons(&body.auth, host, &m.from_name, &m.from_addr);
        let notice = trust_notice(&reasons);
        let crypt = body.encrypted.map(|k| crypt_notice(k.label()));
        if notice.is_empty() && crypt.is_none() {
            return out;
        }
        let text = match (&crypt, notice.is_empty()) {
            (Some(c), true) => c.clone(),
            (Some(c), false) => format!("{notice} {c}"),
            (None, _) => notice.clone(),
        };
        let mut bar = Bar::new(
            "reader-notice",
            if notice.is_empty() {
                Kind::Info
            } else {
                Kind::Caution
            },
            text,
        );
        if !notice.is_empty() {
            let junk = role != FolderRole::Junk;
            let (a, b, w) = (view.clone(), view.clone(), view.clone());
            bar = bar
                .title(tr!("This message may not be from who it says"))
                .action(
                    "reader-junk",
                    if junk {
                        tr!("Report junk")
                    } else {
                        tr!("Not junk")
                    },
                    move |_, cx| {
                        let _ =
                            a.update(cx, |_, cx| cx.emit(ReaderEvent::Junk { message: id, junk }));
                    },
                )
                .action("reader-delete", tr!("Delete"), move |_, cx| {
                    let _ = b.update(cx, |_, cx| cx.emit(ReaderEvent::Delete(id)));
                })
                .action("reader-why", tr!("Why?"), move |window, cx| {
                    let _ = w.update(cx, |r, cx| r.why_sender(window, cx));
                });
        }
        out.push(bar.into_any_element());
        out
    }

    /// The remote-image bars and captions over the page.
    fn image_bars(&self, open: &Open, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut out = Vec::new();
        if !self.session.page_shown() {
            return out;
        }
        let t = cx.tokens().clone();
        let view = cx.entity().downgrade();
        if let Some(addr) = &self.remote_always {
            let (undo, privacy, close) = (view.clone(), view.clone(), view.clone());
            out.push(
                Bar::new(
                    "reader-always",
                    Kind::Info,
                    tr!("Images from {0} will always load.", addr),
                )
                .action("reader-always-undo", tr!("Undo"), move |_, cx| {
                    let _ = undo.update(cx, |r, cx| r.undo_remote_always(cx));
                })
                .action("reader-privacy", tr!("Privacy settings"), move |_, cx| {
                    let _ = privacy.update(cx, |_, cx| cx.emit(ReaderEvent::OpenPrivacy));
                })
                .closable(move |_, cx| {
                    let _ = close.update(cx, |r, cx| {
                        r.remote_always = None;
                        cx.notify();
                    });
                })
                .into_any_element(),
            );
        } else if self.remote_allowed {
            let privacy = view.clone();
            out.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SP_1))
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(tr!("Images load from this sender"))
                    .child("\u{b7}")
                    .child(
                        Button::new("reader-remote-change")
                            .variant(Variant::Subtle)
                            .label(tr!("remote images" => "Change"))
                            .name(tr!("Change which senders' images load"))
                            .on_click(move |_, _, cx| {
                                let _ =
                                    privacy.update(cx, |_, cx| cx.emit(ReaderEvent::OpenPrivacy));
                            }),
                    )
                    .into_any_element(),
            );
        }
        let (blocked, failed) = (self.session.remote_blocked(), self.session.remote_failed());
        if (blocked > 0 || failed > 0) && !self.images_closed {
            let sender = open.msg.from_addr.clone();
            let text = banner::images_text(blocked, failed, &sender);
            let mut bar = Bar::new(
                "reader-images",
                if blocked > 0 {
                    Kind::Info
                } else {
                    Kind::Caution
                },
                text,
            );
            if blocked > 0 {
                let once = view.clone();
                bar = bar.action("reader-load-images", tr!("Load images"), move |w, cx| {
                    let _ = once.update(cx, |r, cx| r.load_remote(false, w, cx));
                });
                if !sender.is_empty() {
                    let always = view.clone();
                    bar = bar.action(
                        "reader-always-load",
                        tr!("Always load from this sender"),
                        move |w, cx| {
                            let _ = always.update(cx, |r, cx| r.load_remote(true, w, cx));
                        },
                    );
                }
            }
            let close = view.clone();
            bar = bar.closable(move |_, cx| {
                let _ = close.update(cx, |r, cx| {
                    r.images_closed = true;
                    cx.notify();
                });
            });
            out.push(bar.into_any_element());
        }
        out
    }

    /// The page (or nothing, while waiting or in the text view), with the
    /// column measure that reports the band in view.
    fn page(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let view = cx.entity().downgrade();
        let measure_view = view.clone();
        let column = self.column.clone();
        let measure = canvas(
            move |bounds, window, cx| {
                if column.get() != Some(bounds) {
                    column.set(Some(bounds));
                    let v = measure_view.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = v.update(cx, |r, cx| r.sync_frame(window, cx));
                    });
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();
        // The page reaches into the column's padding by the body's own
        // margin, so its text starts where the subject and the header do
        // (a page with a colour of its own shows it 8 px wider each side).
        let wrapper = div()
            .id("reader-column")
            .relative()
            .mx(px(-reader::BODY_MARGIN))
            .child(measure);
        let (Some(page), true) = (self.session.page().copied(), self.session.page_shown()) else {
            return wrapper;
        };
        let frame = self.session.frame();
        let tiles: Vec<(u32, f32, TileImage)> = tile::visible(frame.top, frame.height, page.height)
            .map(|(a, b)| {
                (a..=b)
                    .filter_map(|i| {
                        let tile = self.session.tiles().get(i as usize)?;
                        Some((i, tile.height, tile.image.clone()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let paint = tile::PagePaint {
            bg: reader::page_colour(page.bg),
            tiles,
            width: page.width,
            height: page.height,
            selection: self.session.selection().to_vec(),
            selection_color: reader::selection(&t.color),
            focus: self.session.focus_rects(),
            ring: t.color.ink,
            ring_width: reader::LINK_RING,
            ring_radius: reader::LINK_RING_RADIUS,
        };
        let element = tile::page_element(paint, self.page_bounds.clone(), |_, _, _| {});
        // Screen readers: the lines, and the links near the view.
        let lines = self.lines.clone();
        let text_selection = self.session.text_selection();
        let bounds = self.page_bounds.clone();
        let scale = window.scale_factor();
        let band = (
            frame.top - LINK_NODES_SCREENS * frame.height,
            frame.top + (1.0 + LINK_NODES_SCREENS) * frame.height,
        );
        let focused = self.session.focused_link();
        let link_nodes = self
            .session
            .links()
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let r = *l.rects.first()?;
                (r.y + r.h >= band.0 && r.y <= band.1).then(|| {
                    let v = view.clone();
                    let name = if l.text.is_empty() { &l.href } else { &l.text };
                    div()
                        .id(("reader-link", i))
                        // The link Tab reached is what UI Automation reports
                        // while the page holds the focus.
                        .when(focused == Some(i), |el| el.aria_active_descendant())
                        .absolute()
                        .left(px(r.x))
                        .top(px(r.y))
                        .w(px(r.w))
                        .h(px(r.h))
                        .role(Role::Link)
                        .aria_label(SharedString::from(name.clone()))
                        .aria_description(SharedString::from(l.href.clone()))
                        .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                            let _ = v.update(cx, |r, cx| {
                                r.session.activate(i);
                                r.after(window, cx);
                                cx.notify();
                            });
                        })
                        .test_support()
                })
            });
        // A page that fills the column (its width only snapped down) starts
        // at the column's left, so its text lines up with the subject; one
        // held at the widest page is centred.
        let page_x = if frame.column - page.width < komorebi::reader::WIDTH_STEP {
            0.0
        } else {
            ((frame.column - page.width) / (2. * reader::PAGE_GRID)).round() * reader::PAGE_GRID
        };
        let hover = self.session.hover_link();
        let selected = !self.session.selection().is_empty();
        let menu_view = view.clone();
        let surface = div()
            .id("reader-page")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::next_link))
            .on_action(cx.listener(Self::previous_link))
            .on_action(cx.listener(Self::follow_link))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|r, _: &MouseDownEvent, window, cx| window.focus(&r.focus, cx)),
            )
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_hover(cx.listener(|r, hovered: &bool, _, cx| {
                if !*hovered && r.session.hover_at(None) {
                    cx.notify();
                }
            }))
            .cursor(if hover {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            })
            .role(Role::Document)
            .aria_label(tr!("Message"))
            // The page's own ring while no link carries one.
            .when(focused.is_none(), |el| {
                el.focus_visible(crate::widgets::ring_style(&t))
            })
            .a11y_synthetic_children(move |builder| {
                if let Some(page) = bounds.get() {
                    let nodes = a11y::text_nodes(&lines, page, scale);
                    a11y::push_selected(builder, nodes, text_selection);
                }
            })
            .relative()
            .flex_none()
            .w(px(page.width))
            .h(px(page.height))
            .ml(px(page_x))
            .child(element)
            .children(link_nodes)
            .test_support()
            .context_menu(move |menu, _window, cx| {
                let (copy, all) = (menu_view.clone(), menu_view.clone());
                crate::widgets::menu::brand(menu, cx)
                    .item(
                        PopupMenuItem::new(tr!("Copy"))
                            .disabled(!selected)
                            .on_click(move |_, window, cx| {
                                let _ = copy.update(cx, |r, cx| {
                                    r.session.select(SelectOp::Copy);
                                    r.after(window, cx);
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new(tr!("Select all")).on_click(move |_, window, cx| {
                            let _ = all.update(cx, |r, cx| {
                                r.session.select(SelectOp::All);
                                r.after(window, cx);
                            });
                        }),
                    )
            });
        if page.width > frame.column {
            // Fixed-width mail at the smallest scale: the page scrolls
            // sideways; the vertical wheel still scrolls the message.
            wrapper.child(
                div()
                    .id("reader-page-x")
                    .w_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.hscroll)
                    .child(surface),
            )
        } else {
            wrapper.child(surface)
        }
    }
}

impl Render for ReaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The theme or the scale may have changed with no new bounds.
        self.sync_frame(window, cx);
        let t = cx.tokens().clone();
        let c = t.color;
        let lang = crate::i18n::lang(cx);
        let root =
            div()
                .id("reader")
                .key_context(PANE_CONTEXT)
                .on_action(cx.listener(Self::line_down))
                .on_action(cx.listener(Self::line_up))
                .on_action(cx.listener(Self::page_down))
                .on_action(cx.listener(Self::page_up))
                .on_action(cx.listener(Self::scroll_top))
                .on_action(cx.listener(Self::scroll_end))
                .on_action(cx.listener(Self::save_eml_key))
                // The text view's field has its own Up/Down, Page and Home/End
                // (deeper than this context): the reader scrolls instead.
                .capture_action(
                    cx.listener(|r, _: &MoveDown, _, cx| r.text_key(reader::SCROLL_LINE, cx)),
                )
                .capture_action(
                    cx.listener(|r, _: &MoveUp, _, cx| r.text_key(-reader::SCROLL_LINE, cx)),
                )
                .capture_action(cx.listener(|r, _: &MovePageDown, _, cx| {
                    r.text_key(r.view_height() * PAGE_SHARE, cx)
                }))
                .capture_action(cx.listener(|r, _: &MovePageUp, _, cx| {
                    r.text_key(-r.view_height() * PAGE_SHARE, cx)
                }))
                .capture_action(cx.listener(|r, _: &MoveHome, _, cx| r.text_key(f32::MIN, cx)))
                .capture_action(cx.listener(|r, _: &MoveEnd, _, cx| r.text_key(f32::MAX, cx)))
                .size_full()
                .relative()
                .flex()
                .flex_col()
                // No fill of its own: the pane under it fills it (paper, or
                // see-through over a background picture).
                .text_color(c.ink);
        let Some(open) = self.open.take() else {
            return root;
        };
        let subject = if open.msg.subject.is_empty() {
            tr!("(no subject)")
        } else {
            open.msg.subject.clone()
        };
        let header = self.header(&open, lang, cx);
        let notices = self.notices(&open, cx);
        let images = self.image_bars(&open, cx);
        let invite = open
            .body
            .as_ref()
            .and_then(|b| b.invite.as_deref())
            .map(|ev| {
                let account = self
                    .accounts
                    .iter()
                    .find(|a| a.id == open.msg.account_id)
                    .map(|a| a.email.clone());
                let card = inv::card(
                    &self.db,
                    account.as_deref(),
                    &open.msg,
                    ev,
                    lang,
                    &chrono::Local::now(),
                    self.invite_change,
                );
                let view = cx.entity().downgrade();
                invite::InviteCard::new(card, self.invite_details, move |press, window, cx| {
                    let _ = view.update(cx, |r, cx| r.invite_press(press, window, cx));
                })
            });
        let loading = open.msg.raw.is_none();
        let waiting = loading.then(|| {
            if self.unreachable {
                let id = open.msg.id;
                let view = cx.entity().downgrade();
                div()
                    .when(self.unreachable_late, |el| {
                        el.flex().flex_col().gap(px(space::SP_2)).child(
                            div()
                                .id("reader-not-downloaded")
                                .role(Role::Status)
                                .aria_label(tr!(
                                    "This message hasn't been downloaded yet. It downloads when Komorebi can reach the server."
                                ))
                                .typo(&t.text.body)
                                .text_color(c.ink_2)
                                .child(tr!(
                                    "This message hasn't been downloaded yet. It downloads when Komorebi can reach the server."
                                ))
                                .test_support(),
                        )
                        .child(
                            Button::new("reader-retry")
                                .label(tr!("Retry"))
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |r, cx| {
                                        r.start_waits(cx);
                                        cx.emit(ReaderEvent::RetryBody(id));
                                    });
                                }),
                        )
                    })
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SP_2))
                    .child(self.loader.clone())
                    .when(self.loading_text, |el| {
                        el.child(
                            div()
                                .typo(&t.text.body)
                                .text_color(c.ink_2)
                                .child(tr!("Downloading message\u{a0}…")),
                        )
                    })
                    .into_any_element()
            }
        });
        let note = (!self.session.html() && !self.session.waiting())
            .then(|| self.session.failed().and_then(failed_note))
            .flatten();
        let page = self.page(window, cx);
        let truncated = (self.session.page_shown() && self.session.truncated()).then(|| {
            let view = cx.entity().downgrade();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(space::SP_2))
                .pt(px(space::SP_3))
                .typo(&t.text.body)
                .text_color(c.ink_2)
                .child(tr!("[Message shortened for display]"))
                .child(
                    Button::new("reader-plain")
                        .variant(Variant::Subtle)
                        .label(tr!("View as plain text"))
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |r, cx| r.set_plain(true, window, cx));
                        }),
                )
        });
        let text = (self.session.text_shown() && !loading)
            .then(|| open.body.as_ref().map(|b| b.text.clone()))
            .flatten()
            .map(|text| {
                let text = if text.len() > MAX_BODY_BYTES {
                    let cut = text.floor_char_boundary(MAX_BODY_BYTES);
                    format!(
                        "{}\n\n{}",
                        &text[..cut],
                        tr!("[Message shortened for display]")
                    )
                } else {
                    text
                };
                // A read-only field, as the Slint app's: a Tab stop with a
                // focus ring, selectable with the mouse and the keyboard.
                let field = self.text_field(open.msg.id, text, window, cx);
                // Drawn into the column's padding by the field's own inset,
                // so the text starts where the subject does.
                div()
                    .id("reader-text")
                    .max_w(px(reader::TEXT_MAX_W))
                    .mx(px(-reader::TEXT_INSET))
                    .child(
                        Textarea::new(&field)
                            .readonly(true)
                            .appearance(false)
                            .aria_label(tr!("Message"))
                            .w_full()
                            .typo(&t.text.message)
                            .text_color(c.ink),
                    )
            });
        let attachments = open
            .body
            .as_ref()
            .filter(|b| !b.attachments.is_empty())
            .map(|b| {
                let view = cx.entity().downgrade();
                attachments::Attachments::new(b.attachments.clone(), lang, move |press, _, cx| {
                    let _ = view.update(cx, |r, cx| match press {
                        attachments::Press::Open(index) => r.open_attachment(index, cx),
                        attachments::Press::Save(index) => r.save_attachment(index, cx),
                    });
                })
            });
        // The reader's own width (not the column's, which the padding
        // changes).
        let width = f32::from(self.scroll.bounds().size.width);
        let narrow = width > 0.0 && width < reader::NARROW_BELOW;
        let pad_x = if narrow {
            reader::PAD_X_NARROW
        } else {
            reader::PAD_X
        };
        let column = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(reader::BLOCK_GAP))
            .pt(px(reader::PAD_TOP))
            .px(px(pad_x))
            .pb(px(reader::PAD_BOTTOM))
            .child(
                div()
                    .id("reader-subject")
                    .role(Role::Heading)
                    .aria_label(tr!("Subject: {0}", subject))
                    .typo(&t.text.title)
                    .child(subject.clone())
                    .test_support(),
            )
            .child(header)
            .children(notices)
            .children(invite)
            .children(waiting)
            .children(note.map(|n| div().typo(&t.text.caption).text_color(c.ink_2).child(n)))
            .children(images)
            .child(page)
            .children(truncated)
            .children(text)
            .children(attachments);
        let target = self.session.target();
        self.open = Some(open);
        root.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("reader-scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(column)
                        .test_support(),
                )
                .vertical_scrollbar(&self.scroll),
        )
        .when(!target.is_empty(), |el| {
            el.child(links::target_strip(target.into(), cx))
        })
    }
}

#[cfg(test)]
mod tests;
