//! The Contacts view (docs/GPUI-MIGRATION.md phase 9; the Slint app's
//! `ui/contacts.slint`, `src/app/contacts.rs` and `contacts_sync.rs`): the
//! list with search and a group filter, the details of the chosen contact,
//! its editor, and the address-book (CardDAV) sync pane.
//!
//! What the view does is the library's (`komorebi::contacts_page` and
//! `komorebi::contacts_sync`, shared with the Slint app); this view keeps
//! their state, the text fields and the sync passes' threads, and words
//! them (`text.rs`). The look is the brand's (mockup `contacts-*`): two
//! paper panes on canvas, the list (title, count, New contact, the "..."
//! menu, search, rows with their avatars) and the details (the large
//! avatar and name, the command pills, label and value lines). Below
//! [`ONE_PANE_BELOW`] one pane shows at a time, with Back.
//!
//! The window mounts it in the content area beside the folder pane through
//! [`ContactsView::new`] and calls [`ContactsView::opened`] when it shows;
//! nothing here runs at start. Writing to a contact asks the shell for the
//! composer ([`ContactsEvent::Compose`]).

mod detail;
mod editor;
mod list;
pub mod sync;
mod tests;
pub mod text;

use std::collections::HashSet;

use gpui::{
    App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle, Styled, Subscription, Window,
    div, prelude::*, px,
};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::contacts_page::{Compose, Form, Page, Problem};
use komorebi::db::Account;

use crate::theme::{layout, space};
use crate::views::Db;
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::pane::Pane;

/// Below this window width one pane shows at a time (the list, or the
/// details with Back): the rail, the compact list and a details pane of
/// the same width.
pub const ONE_PANE_BELOW: f32 = layout::RAIL_W + layout::LIST_W_COMPACT + layout::LIST_W_COMPACT;

/// What the view asks of the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactsEvent {
    /// New message from `account` to `to` (a mailbox with the name): the
    /// shell opens the composer, as the Slint app's `start_draft`.
    Compose { account: i64, to: String },
    /// No account to write from: "Add an account to write messages."
    NoAccount,
    /// An address or a number went to the clipboard.
    Copied,
    /// The cache refused a change (the error chain, English).
    Failed(String),
}

/// What the details pane shows.
pub(crate) enum Side {
    /// The contact shown (or the count when none is).
    Detail,
    /// The editor, over a saved contact or a new one.
    Edit(Box<ContactForm>),
    /// The address-book sync pane.
    Sync(Box<sync::SyncPane>),
}

/// The editor's fields while it is open.
pub(crate) struct ContactForm {
    pub(crate) form: Form,
    pub(crate) name: Entity<InputState>,
    pub(crate) given: Entity<InputState>,
    pub(crate) family: Entity<InputState>,
    pub(crate) org: Entity<InputState>,
    pub(crate) title: Entity<InputState>,
    pub(crate) groups: Entity<InputState>,
    pub(crate) note: Entity<InputState>,
    pub(crate) emails: Vec<Entity<InputState>>,
    pub(crate) phones: Vec<Entity<InputState>>,
    pub(crate) problem: Option<Problem>,
    /// The cache refused the save (the error chain).
    pub(crate) failed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

pub struct ContactsView {
    pub(crate) db: Db,
    pub(crate) accounts: Vec<Account>,
    pub(crate) page: Page,
    pub(crate) side: Side,
    /// In one-pane layout, the details show (else the list).
    pub(crate) details_shown: bool,
    pub(crate) search: Entity<InputState>,
    /// Accounts with a sync pass or a request running.
    pub(crate) busy: HashSet<i64>,
    /// When each account's last pass started.
    pub(crate) last_pass: std::collections::HashMap<i64, std::time::Instant>,
    /// Something was copied: the details say so until the next change.
    pub(crate) copied: bool,
    pub(crate) list_focus: FocusHandle,
    pub(crate) list_scroll: VirtualListScrollHandle,
    /// The list's entries for `page.rows` (none: build them again; every
    /// change of the rows clears it).
    pub(crate) entries: Option<list::Entries>,
    pub(crate) side_scroll: ScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ContactsEvent> for ContactsView {}

impl Focusable for ContactsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ContactsView {
    /// The view over the window's cache. Empty until
    /// [`ContactsView::opened`].
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(text::search()));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, input, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    this.page.query = input.read(cx).value().to_string();
                    this.reload(false, cx);
                }
            }),
            // The placeholder lives outside `render`: it follows the
            // language by hand (docs/GPUI-I18N.md).
            cx.observe_global_in::<crate::i18n::CurrentLang>(window, |this, window, cx| {
                this.search
                    .update(cx, |s, cx| s.set_placeholder(text::search(), window, cx));
            }),
        ];
        Self {
            db,
            accounts: Vec::new(),
            page: Page::default(),
            side: Side::Detail,
            details_shown: false,
            search,
            busy: HashSet::new(),
            last_pass: std::collections::HashMap::new(),
            copied: false,
            list_focus: cx.focus_handle(),
            list_scroll: VirtualListScrollHandle::new(),
            entries: None,
            side_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The view showed: read the accounts, the groups and the list, and
    /// start a sync pass for each account that syncs (the Slint app's
    /// `contacts_toggled(true)`).
    pub fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.accounts = self.db.accounts().unwrap_or_default();
        self.reload(true, cx);
        self.kick(cx);
        self.list_focus.focus(window, cx);
    }

    /// The view hid: an open editor is let go (the Slint app's
    /// `contacts_toggled(false)`).
    pub fn closed(&mut self, cx: &mut Context<Self>) {
        if matches!(self.side, Side::Edit(_)) {
            self.side = Side::Detail;
        }
        cx.notify();
    }

    /// Read the list again (and the groups when `groups`), keeping the
    /// contact shown.
    pub fn reload(&mut self, groups: bool, cx: &mut Context<Self>) {
        self.page.reload(&self.db, groups);
        self.entries = None;
        self.copied = false;
        cx.notify();
    }

    /// Made-up contacts in the local book for snapshots and the example
    /// (only with `KOMOREBI_OFFLINE`, because they are written to the
    /// cache), the view open on the first one, and `then`: `edit` opens its
    /// editor, `sync` the sync pane.
    pub fn sample(&mut self, then: &str, window: &mut Window, cx: &mut Context<Self>) {
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            komorebi::contacts_page::sample(&self.db);
        }
        self.opened(window, cx);
        self.select(Some(0), cx);
        match then {
            "edit" => self.edit(window, cx),
            "sync" => self.open_sync(window, cx),
            _ => {}
        }
    }

    pub fn pick_group(&mut self, index: usize, cx: &mut Context<Self>) {
        self.page.pick_group(&self.db, index);
        self.entries = None;
        self.copied = false;
        cx.notify();
    }

    /// Show the contact at list position `index` (none: nothing).
    pub fn select(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.page.select(&self.db, index);
        self.details_shown = index.is_some();
        self.copied = false;
        if let Some(i) = index {
            self.scroll_to_row(i, cx);
        }
        cx.notify();
    }

    /// Back, in one-pane layout: the editor cancels, the sync pane closes,
    /// the details go back to the list.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.side {
            Side::Edit(_) => self.cancel_edit(window, cx),
            Side::Sync(_) => self.close_sync(cx),
            Side::Detail => {
                self.details_shown = false;
                self.list_focus.focus(window, cx);
                cx.notify();
            }
        }
    }

    // ---- The contact shown

    /// Delete asks first; a synced contact is removed from the server too.
    pub fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.page.shown() else {
            return;
        };
        if d.read_only {
            return;
        }
        let synced = !d.book.is_empty();
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::delete_question().into(),
                message: text::delete_question_text(synced).into(),
                confirm: text::delete().into(),
                cancel: text::cancel().into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = view.update(cx, |v, cx| v.delete_now(cx));
            },
            window,
            cx,
        );
    }

    /// Delete the contact shown, answered.
    pub fn delete_now(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.page.delete_shown(&self.db) {
            cx.emit(ContactsEvent::Failed(format!("{e:#}")));
        }
        self.entries = None;
        self.details_shown = false;
        self.reload(true, cx);
        self.kick(cx);
    }

    /// New message to `addr`, or to the contact's preferred address (empty
    /// `addr`), from the account that owns its book.
    pub fn write_to(&mut self, addr: &str, cx: &mut Context<Self>) {
        let Some(c) = self.page.shown().map(|d| d.contact.clone()) else {
            return;
        };
        match komorebi::contacts_page::compose_to(&self.db, &self.accounts, &c, addr) {
            Compose::Nothing => {}
            Compose::NoAccount => cx.emit(ContactsEvent::NoAccount),
            Compose::To { account, to } => cx.emit(ContactsEvent::Compose { account, to }),
        }
    }

    pub fn copy(&mut self, value: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
        self.copied = true;
        cx.emit(ContactsEvent::Copied);
        cx.notify();
    }

    /// Keep the local version of a conflicted contact: it replaces the
    /// server's at the next push.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.page.shown().map(|d| d.contact.clone()) else {
            return;
        };
        if komorebi::contacts_page::keep_mine(&self.db, &c) {
            self.kick(cx);
        }
        self.reload(true, cx);
    }

    pub fn keep_server(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.page.shown().map(|d| d.contact.clone()) {
            komorebi::contacts_page::keep_server(&self.db, &c);
        }
        self.page.refresh_shown(&self.db);
        self.entries = None;
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.key == "escape" && !k.modifiers.modified() {
            match self.side {
                Side::Edit(_) | Side::Sync(_) => {
                    self.back(window, cx);
                    cx.stop_propagation();
                }
                Side::Detail if self.details_shown && one_pane(window) => {
                    self.back(window, cx);
                    cx.stop_propagation();
                }
                Side::Detail => {}
            }
        }
    }
}

/// A question with two answers where closing it (Escape) is the second:
/// "Remove the synced contacts?" with Remove and Keep. The look is
/// `widgets::dialog::confirm`'s; only the second button acts here.
pub(crate) fn ask_two(
    spec: Confirm,
    yes: impl Fn(&mut Window, &mut App) + 'static,
    no: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui_kit::component::WindowExt as _;
    use std::rc::Rc;

    use crate::theme::{ActiveTokens as _, TextStyled as _, radius};
    use crate::widgets::button::{Button, Variant};

    let (yes, no) = (Rc::new(yes), Rc::new(no));
    let t = cx.tokens().clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let c = t.color;
        let (ok, keep, escape) = (yes.clone(), no.clone(), no.clone());
        dialog
            .w(px(layout::DIALOG_W))
            .bg(c.paper)
            .text_color(c.ink)
            .rounded(px(radius::CARD))
            .shadow(Vec::new())
            .close_button(false)
            .p(px(space::SP_6))
            .title(div().typo(&t.text.subtitle).child(spec.title.clone()))
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(spec.message.clone()),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap(px(space::SP_2))
                    .pt(px(space::SP_4))
                    .child(
                        Button::new("dialog-cancel")
                            .label(spec.cancel.clone())
                            .on_click(move |_, window, cx| {
                                keep(window, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("dialog-confirm")
                            .variant(if spec.destructive {
                                Variant::Destructive
                            } else {
                                Variant::Primary
                            })
                            .label(spec.confirm.clone())
                            .on_click(move |_, window, cx| {
                                ok(window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
            .on_cancel(move |_, window, cx| {
                escape(window, cx);
                true
            })
    });
}

/// Whether the window is narrow enough for one pane at a time.
pub(crate) fn one_pane(window: &Window) -> bool {
    f32::from(window.viewport_size().width) < ONE_PANE_BELOW
}

impl Render for ContactsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let single = one_pane(window);
        let compact = f32::from(window.viewport_size().width) < layout::COMPACT_BELOW_W;
        let list_w = if compact {
            layout::LIST_W_COMPACT
        } else {
            layout::LIST_W
        };
        let side_open = self.details_shown || !matches!(self.side, Side::Detail);
        let show_list = !single || !side_open;
        let show_side = !single || side_open;
        div()
            .id("contacts")
            .key_context("Contacts")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_row()
            .gap(px(space::PANE_GAP))
            .when(show_list, |el| {
                el.child(
                    div()
                        .h_full()
                        .min_w_0()
                        .when(single, |el| el.flex_1())
                        .when(!single, |el| el.flex_none().w(px(list_w)))
                        .child(
                            Pane::new("contacts-list-pane")
                                .name(text::contacts())
                                .child(list::render(self, window, cx)),
                        ),
                )
            })
            .when(show_side, |el| {
                let side = match &self.side {
                    Side::Detail => detail::render(self, single, window, cx),
                    Side::Edit(_) => editor::render(self, single, window, cx),
                    Side::Sync(_) => sync::render(self, single, window, cx),
                };
                el.child(
                    div()
                        .h_full()
                        .flex_1()
                        .min_w_0()
                        .child(Pane::new("contacts-side-pane").child(side)),
                )
            })
    }
}

/// A new single-line field holding `value`.
pub(crate) fn field(
    value: &str,
    window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}
