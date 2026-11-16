//! The composer (phase 4; mockup `compose`): a view of the active draft of
//! `komorebi::compose::Drafts`, the model both apps share. The header (the
//! title, Attach, Discard, Send), the From, To, Cc, Bcc and Subject rows
//! (recipients as chips, `recipients.rs`), the format bar (`toolbar.rs`,
//! over the bar shared with the signature editor, `format_bar.rs`),
//! the painted rich editor (`editor.rs`) or a plain-text body, the
//! attachments (`attachments.rs`), the Signature menu (`signatures.rs`)
//! and the caption under it all ("Draft saved 14:05").
//!
//! What a draft does (saved two seconds after the last edit and on every
//! way out, copied to the server, sent after the undo window) is
//! `controller.rs` over the library. The shell hears it through
//! [`ComposerEvent`]: it owns the status line, the undo offer, the sync
//! workers and the Drafts list.
//!
//! The mail view mounts it with [`ComposerView::new`] (docs/GPUI-MIGRATION.md
//! "Stage B plan") and asks for a draft with [`ComposerView::open`]
//! (`slot.rs`); the window installs its keys with [`bind_keys`].

pub mod a11y;
pub mod attachments;
pub mod controller;
pub mod editor;
pub mod format_bar;
/// Pictures read and prepared for the formatted text.
pub mod pictures;
pub mod preview;
pub mod recipients;
mod sends;
pub mod signatures;
pub mod slot;
pub mod text;
pub mod toolbar;

pub use self::slot::Draft;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement, Render, Role, SharedString,
    Styled, Subscription, Task, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, Textarea, TextareaState};
use komorebi::compose::{ComposeDefaults, DraftId, Drafts, Field, Guard, SendBlock};
use komorebi::db::{self, Db};

use self::editor::{EditorEvent, RichEditor};
use self::recipients::RecipientInput;
use crate::access::{AccessHost as _, Key};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo, layout, space};
use crate::widgets::button::{Button, Height, IconButton};
use crate::widgets::field::{FieldLook, TextField};
use crate::widgets::notice::{self, Notice};
use crate::widgets::select::Dropdown;

/// The key context of the composer.
pub const CONTEXT: &str = "Composer";

gpui_kit::actions!(
    composer,
    [Send, Close, RevealCc, RevealBcc, Attach, OpenLink]
);

/// The composer's keys and the body's (the Slint composer's `shortcut`).
pub fn bind_keys(cx: &mut App) {
    editor::bind_keys(cx);
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", Send, ctx),
        KeyBinding::new("alt-s", Send, ctx),
        KeyBinding::new("escape", Close, ctx),
        KeyBinding::new("ctrl-shift-c", RevealCc, ctx),
        KeyBinding::new("ctrl-shift-b", RevealBcc, ctx),
        KeyBinding::new("ctrl-shift-a", Attach, ctx),
        KeyBinding::new("ctrl-k", OpenLink, ctx),
        // The draft in a window of its own (the main window's tab host).
        KeyBinding::new("ctrl-o", crate::tabs::OpenInWindow, ctx),
    ]);
}

/// A line the composer has for the status line (typed; the shell words
/// it with [`Status::text`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The composer closed and the draft stays in Drafts.
    DraftSaved,
    CouldNotSave(String),
    /// Send pressed with something to fix (the field says why too).
    SendBlocked(SendBlock),
    /// Files that could not be attached.
    AttachFailed(Vec<String>),
    DraftGone,
    /// Saved attachments that could not be read back.
    LeftOut(String),
    NoAccount,
    NoAccountWrite,
    StillDownloading,
    /// Another draft took the composer; the one before was saved.
    EarlierDraftSaved,
    /// The pane's draft moved to a background tab ([`ComposerView::set_aside`]).
    KeptInTab,
    /// Something the cache or the file system said.
    Error(String),
}

impl Status {
    /// The sentence, in the language shown.
    pub fn text(&self) -> String {
        match self {
            Status::DraftSaved => text::draft_saved(),
            Status::CouldNotSave(e) => text::could_not_save(e),
            Status::SendBlocked(b) => text::send_block(*b),
            Status::AttachFailed(lines) => lines.join("; "),
            Status::DraftGone => text::draft_gone(),
            Status::LeftOut(names) => text::draft_left_out(names),
            Status::NoAccount => text::add_account_send(),
            Status::NoAccountWrite => text::add_account_write(),
            Status::StillDownloading => text::still_downloading(),
            Status::EarlierDraftSaved => text::earlier_draft_saved(),
            Status::KeptInTab => text::draft_kept_in_tab(),
            Status::Error(e) => e.clone(),
        }
    }

    /// An error (announced at once), not news.
    pub fn is_error(&self) -> bool {
        !matches!(
            self,
            Status::DraftSaved | Status::EarlierDraftSaved | Status::KeptInTab
        )
    }
}

/// What the composer tells the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposerEvent {
    /// The composer has nothing to show any more (closed, sent, discarded).
    Closed,
    /// A saved draft came or went (`count`) or changed: the Drafts count,
    /// the Draft tag of the message it replies to, the Drafts list.
    DraftsChanged { reply_to: Option<i64>, count: bool },
    /// A line for the status line.
    Status(Status),
    /// Copy saved draft `row` to the server's Drafts folder (the account's
    /// sync worker, `Job::SaveDraft`).
    SaveToServer { row: i64, account: i64 },
    /// Remove a draft's copy on the server (`Job::DeleteDraft`).
    RemoveServerCopy(db::ServerCopy),
    /// A send is in its undo window: the shell shows the Undo offer
    /// ("Sending “Plans” in 10 s") until `Queued` or `Unsent`.
    Sending {
        row: i64,
        subject: String,
        secs: u64,
    },
    /// Built and past its undo window: hand outbox row `row` to its
    /// account's send worker (`Job::Send`).
    Queued { row: i64, account: i64 },
    /// A send went back to Drafts (Undo, or its build failed).
    Unsent { row: i64 },
    /// "Manage signatures…": open Settings > Signatures.
    ManageSignatures,
    /// A saved draft asked for elsewhere (the Drafts list, Reopen, Undo)
    /// is shown by this composer, a draft tab's or a window's: its tab or
    /// window comes forward instead of a second copy opening.
    Raise,
    /// "Open in new window" (the More menu): the owner pops this draft
    /// out into a composer window of its own.
    OpenInWindow,
}

/// The composer view.
pub struct ComposerView {
    db: Rc<Db>,
    accounts: Vec<db::Account>,
    defaults: ComposeDefaults,
    undo_delay: Duration,
    pub(crate) drafts: Drafts,
    pub(crate) body: Entity<RichEditor>,
    plain: Entity<TextareaState>,
    to: RecipientInput,
    cc: RecipientInput,
    bcc: RecipientInput,
    subject: Entity<InputState>,
    link: Entity<InputState>,
    link_open: bool,
    /// The link row edits the selected picture's alt text.
    link_alt: bool,
    link_error: bool,
    /// The send guards' question, while it shows.
    guard: Option<Guard>,
    /// The last line for the status line (also shown in the composer).
    status: Option<Status>,
    /// The highlighted suggestion.
    suggest_index: usize,
    focus: FocusHandle,
    /// A draft shows.
    composing: bool,
    /// The pending autosave and when the last edit was (the executor's
    /// clock, so tests move it).
    autosave: Option<Task<()>>,
    last_edit: Instant,
    /// When each saved row was last copied to the server.
    server_saves: HashMap<i64, Instant>,
    /// Saved drafts whose attachment and picture copies are being written
    /// off the UI thread: by row, the draft, how many saves are writing,
    /// a server copy asked for meanwhile (and whether at once), and whether
    /// Send was pressed meanwhile (it goes once they are written).
    writing: HashMap<i64, (DraftId, usize, Option<bool>, bool)>,
    /// The app's sends in their undo window (shared by every composer).
    sends: sends::Sends,
    /// Fields are being filled from the draft: their events are not edits.
    filling: bool,
    /// Preview, while it is on, and the renders started.
    preview: Option<preview::Preview>,
    preview_seq: u64,
    /// The device scale the preview was drawn at.
    preview_scale: f32,
    /// The address field that had the focus at the last render (leaving
    /// it checks its addresses).
    focused_field: Option<Field>,
    /// What the mail view's slot last asked for ([`ComposerView::open`]).
    asked: Option<Draft>,
    /// Files from Explorer are being dragged over the composer ("Drop to
    /// attach" shows).
    drop_hover: bool,
    /// The composer has a place of its own (a draft tab, a composer
    /// window), not the reading pane's: a saved draft it shows comes
    /// forward here when asked for elsewhere ([`ComposerEvent::Raise`]).
    own_place: bool,
    /// The composer is a composer window's (no Open in new window).
    windowed: bool,
    /// The format bar's buttons: one Tab stop, the one last used; Left and
    /// Right move along the bar; and its measured width (`format_bar.rs`).
    pub(crate) fmt: format_bar::BarState,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for ComposerView {}

impl ComposerView {
    /// The composer over cache `db` (the window's connection, shared with
    /// the mail view: its accounts, settings, drafts and address index).
    /// Nothing shows until a draft is started or opened.
    pub fn new(db: crate::views::Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::access::release_on_deactivate(window, cx);
        let settings = db.settings().unwrap_or_default();
        let accounts = db.accounts().unwrap_or_default();
        let body = cx.new(|cx| RichEditor::new(Some(db.clone()), cx));
        let plain = cx.new(|cx| TextareaState::new(window, cx).rows(geo::PLAIN_ROWS));
        let subject = cx.new(|cx| InputState::new(window, cx));
        let link = cx.new(|cx| InputState::new(window, cx));
        let to = RecipientInput::new(Field::To, window, cx);
        let cc = RecipientInput::new(Field::Cc, window, cx);
        let bcc = RecipientInput::new(Field::Bcc, window, cx);
        let mut subscriptions = vec![
            cx.subscribe(&body, |this, _, e: &EditorEvent, cx| this.body_event(e, cx)),
            cx.subscribe_in(&plain, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    this.plain_edited(cx);
                }
            }),
            cx.subscribe_in(&subject, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    this.subject_edited(cx);
                }
            }),
            cx.subscribe_in(
                &link,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter { .. } => this.insert_link(window, cx),
                    InputEvent::Change => {
                        this.link_error = false;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        for field in [Field::To, Field::Cc, Field::Bcc] {
            let input = match field {
                Field::To => to.input.clone(),
                Field::Cc => cc.input.clone(),
                _ => bcc.input.clone(),
            };
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, _, e: &InputEvent, window, cx| {
                    this.recipient_event(field, e, window, cx)
                },
            ));
        }
        // The composer is embedded as a cached view (`views::mail::reading`):
        // GPUI replays its last frame unless it is notified, so it follows
        // every entity its render reads (a caret blink, a selection).
        subscriptions.push(cx.observe(&body, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&plain, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&subject, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&link, |_, _, cx| cx.notify()));
        for input in [&to.input, &cc.input, &bcc.input] {
            subscriptions.push(cx.observe(input, |_, _, cx| cx.notify()));
        }
        // Quitting saves every open draft (synchronous: an async save would
        // never run at quit, cookbook_window_state).
        subscriptions.push(cx.on_app_quit(|this: &mut Self, cx| {
            this.save_all(cx);
            async {}
        }));
        let sends = sends::Sends::of(cx);
        sends.register(cx.weak_entity());
        let now = cx.background_executor().now();
        ComposerView {
            db,
            accounts,
            defaults: ComposeDefaults::from_settings(&settings),
            undo_delay: Duration::from_secs(settings.undo_send_secs),
            drafts: Drafts::default(),
            body,
            plain,
            to,
            cc,
            bcc,
            subject,
            link,
            link_open: false,
            link_alt: false,
            link_error: false,
            guard: None,
            status: None,
            suggest_index: 0,
            focus: cx.focus_handle(),
            composing: false,
            autosave: None,
            last_edit: now,
            server_saves: HashMap::new(),
            writing: HashMap::new(),
            sends,
            filling: false,
            focused_field: None,
            preview: None,
            preview_seq: 0,
            preview_scale: 1.0,
            asked: None,
            drop_hover: false,
            own_place: false,
            windowed: false,
            fmt: format_bar::BarState::new(cx),
            _subscriptions: subscriptions,
        }
    }

    /// The composer is a draft tab's or a composer window's.
    pub fn set_own_place(&mut self) {
        self.own_place = true;
    }

    /// The composer is a composer window's.
    pub fn set_windowed(&mut self) {
        self.windowed = true;
    }

    /// Whether the composer is a composer window's.
    pub fn is_windowed(&self) -> bool {
        self.windowed
    }

    /// A draft shows.
    pub fn composing(&self) -> bool {
        self.composing
    }

    /// The last status line.
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// The send guards' question, while it shows.
    pub fn guard(&self) -> Option<Guard> {
        self.guard
    }

    /// The accounts (re-read when Settings change them).
    pub fn reload_accounts(&mut self, cx: &mut Context<Self>) {
        self.accounts = self.db.accounts().unwrap_or_default();
        let settings = self.db.settings().unwrap_or_default();
        self.defaults = ComposeDefaults::from_settings(&settings);
        self.undo_delay = Duration::from_secs(settings.undo_send_secs);
        cx.notify();
    }

    /// The new-message settings (Settings > General > Compose; tests).
    pub fn set_defaults(&mut self, defaults: ComposeDefaults) {
        self.defaults = defaults;
    }

    /// How long Send waits so it can be undone.
    pub fn set_undo_delay(&mut self, delay: Duration) {
        self.undo_delay = delay;
    }

    /// Which address field has the focus now; the one left is checked.
    fn follow_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        let now = [Field::To, Field::Cc, Field::Bcc].into_iter().find(|f| {
            self.input(*f)
                .is_some_and(|i| i.input.read(cx).focus_handle(cx).is_focused(window))
        });
        if now != self.focused_field {
            if let Some(left) = self.focused_field {
                self.recipient_left(left, cx);
            }
            self.focused_field = now;
        }
    }

    pub(crate) fn db(&self) -> &Db {
        &self.db
    }

    fn input(&self, field: Field) -> Option<&RecipientInput> {
        match field {
            Field::To => Some(&self.to),
            Field::Cc => Some(&self.cc),
            Field::Bcc => Some(&self.bcc),
            _ => None,
        }
    }

    /// The draft's sender line: its account's name and address.
    fn account_choices(&self) -> Vec<(SharedString, SharedString)> {
        self.accounts
            .iter()
            .map(|a| {
                let label = if a.name.trim().is_empty() {
                    a.email.clone()
                } else {
                    format!("{} {}", a.name.trim(), a.email)
                };
                (
                    SharedString::from(a.id.to_string()),
                    SharedString::from(label),
                )
            })
            .collect()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let title = self
            .drafts
            .active()
            .map(|d| text::title(d.kind))
            .unwrap_or_default();
        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .gap(px(geo::HEADER_GAP))
            .pt(px(geo::HEADER_TOP))
            .pb(px(geo::HEADER_BOTTOM))
            .pl(px(geo::INSET_LEFT))
            .pr(px(geo::HEADER_RIGHT))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .typo(&t.text.subtitle)
                    .text_color(t.color.ink)
                    .child(title),
            )
            .child(
                IconButton::new("compose-attach", Icon::Attach, text::words::attach())
                    .tooltip(text::words::attach_tip())
                    .access_key(Key::Attach)
                    .on_click(cx.listener(|this, _, window, cx| this.attach(window, cx))),
            )
            .child(
                IconButton::new("compose-discard", Icon::Delete, text::words::discard())
                    .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
            )
            .child(
                div().pl(px(geo::SEND_GAP)).child(
                    Button::new("compose-send")
                        .primary()
                        .height(Height::Medium)
                        .icon(Icon::Send)
                        .label(text::words::send())
                        .tooltip(text::words::send_tip())
                        .access_key(Key::Send)
                        .on_click(cx.listener(|this, _, window, cx| this.send(false, window, cx))),
                ),
            )
    }

    fn render_guard(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let guard = self.guard?;
        Some(
            div().px(px(geo::HEADER_RIGHT)).pb(px(space::SP_2)).child(
                Notice::new("compose-guard", notice::Kind::Warning, text::guard(guard))
                    .action(
                        Button::new("compose-send-anyway")
                            .variant(crate::widgets::button::Variant::OnInset)
                            .label(text::words::send_anyway())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send(true, window, cx)),
                            ),
                    )
                    .action(
                        Button::new("compose-keep-editing")
                            .variant(crate::widgets::button::Variant::OnInset)
                            .label(text::words::keep_editing())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.guard = None;
                                this.focus_body(window, cx);
                                cx.notify();
                            })),
                    ),
            ),
        )
    }

    fn render_from(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let current = self.drafts.active().map_or(0, |d| d.account_id);
        let choices = self.account_choices();
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .gap(px(geo::FIELD_GAP))
            .min_h(px(layout::FIELD_H))
            .py(px(geo::FIELD_PAD_Y))
            .ml(px(geo::INSET_LEFT))
            .mr(px(geo::INSET_RIGHT))
            .border_b(px(layout::HAIRLINE))
            .border_color(t.color.line)
            .typo(&t.text.body)
            .child(
                div()
                    .w(px(layout::FIELD_LABEL_W))
                    .flex_none()
                    .text_color(t.color.ink_3)
                    .child(text::words::from()),
            );
        if choices.len() > 1 {
            row.child(
                Dropdown::new("compose-from", text::words::from(), current.to_string())
                    .choices(choices)
                    .on_pick(cx.listener(|this, value: &SharedString, window, cx| {
                        if let Ok(id) = value.parse::<i64>() {
                            this.pick_account(id, window, cx);
                        }
                    })),
            )
            .into_any_element()
        } else {
            let account = self.accounts.iter().find(|a| a.id == current);
            let (name, email) = account.map_or((String::new(), String::new()), |a| {
                (a.name.trim().to_string(), a.email.clone())
            });
            row.child(
                div()
                    .id("compose-from")
                    .role(Role::Label)
                    .aria_label(SharedString::from(
                        format!("{name} {email}").trim().to_string(),
                    ))
                    .flex()
                    .gap(px(space::SP_1))
                    .child(div().text_color(t.color.ink).child(name))
                    .child(div().text_color(t.color.ink_3).child(email))
                    .test_support(),
            )
            .into_any_element()
        }
    }

    fn render_subject(&self) -> impl IntoElement + use<> {
        div()
            .flex_none()
            .ml(px(geo::INSET_LEFT))
            .mr(px(geo::INSET_RIGHT))
            .child(TextField::new(&self.subject, text::words::subject()).look(
                FieldLook::Underline {
                    label: text::words::subject().into(),
                },
            ))
    }

    fn render_caption(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let saved = self
            .drafts
            .active()
            .and_then(|d| d.saved_at)
            .map(|at| text::draft_saved_at(&text::time(at)));
        let line = match &self.status {
            Some(s) if s.is_error() => Some(s.text()),
            _ => saved,
        };
        div()
            .id("compose-caption")
            .role(Role::Status)
            .aria_label(SharedString::from(line.clone().unwrap_or_default()))
            .flex_none()
            .mx(px(geo::INSET_LEFT))
            .pt(px(space::SP_2))
            .pb(px(geo::CAPTION_BOTTOM))
            .typo(&t.text.caption)
            .text_color(t.color.ink_3)
            .children(line)
            .test_support()
    }

    fn render_body(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let formatted = self.body.read(cx).is_formatted();
        // A formatted draft's document is in the editor while it shows.
        let folded = match self.body.read(cx).editor() {
            Some(ed) => ed.has_quote() && ed.quote_folded(),
            None => self.drafts.active().is_some_and(|d| d.quote_folded()),
        };
        let words = (
            SharedString::from(text::words::body_name()),
            SharedString::from(text::words::body_placeholder()),
        );
        self.body
            .update(cx, |b, _| b.set_words(words.0.clone(), words.1.clone()));
        let body = if formatted {
            div()
                .flex_1()
                .min_h(px(geo::BODY_MIN_H))
                .px(px(geo::INSET_LEFT))
                .py(px(geo::BODY_PAD_Y))
                .child(self.body.clone())
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h(px(geo::BODY_MIN_H))
                // Kit's field pads its text: the text lines up with the labels.
                .pl(px(geo::INSET_LEFT - geo::PLAIN_PAD))
                .pr(px(geo::INSET_LEFT))
                .py(px(geo::BODY_PAD_Y))
                // Ctrl+Z right after a switch to plain text: the formatted
                // message back (Kit's own undo otherwise).
                .capture_action(cx.listener(
                    |this, _: &gpui_kit::component::input::Undo, window, cx| {
                        if this.drafts.active().is_some_and(|d| d.can_unplain()) {
                            this.set_rich(true, window, cx);
                            cx.stop_propagation();
                        }
                    },
                ))
                .child(
                    Styled::h(
                        Textarea::new(&self.plain)
                            .appearance(false)
                            .aria_label(words.0.clone()),
                        gpui::relative(1.),
                    )
                    .w_full()
                    .typo(&t.text.message),
                )
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(body)
            .when(folded, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_row()
                        .px(px(geo::INSET_LEFT))
                        .pb(px(space::SP_2))
                        .child(
                            Button::new("compose-show-quote")
                                .variant(crate::widgets::button::Variant::Subtle)
                                .label(text::words::show_quote())
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.unfold_quote(window, cx)
                                    }),
                                ),
                        ),
                )
            })
    }
}

impl Focusable for ComposerView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

// How many times the composer drew on this thread (tests: the composer is a
// cached view in the reading pane, `views/mail/reading.rs`).
#[cfg(test)]
thread_local! {
    static RENDERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The composer draws counted so far on this thread.
#[cfg(test)]
pub(crate) fn renders() -> usize {
    RENDERS.with(std::cell::Cell::get)
}

impl Render for ComposerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        RENDERS.with(|n| n.set(n.get() + 1));
        if !self.composing {
            return div().id("composer").size_full().into_any_element();
        }
        self.follow_focus(window, cx);
        let t = cx.tokens().clone();
        let show_cc = self.drafts.active().is_some_and(|d| d.show_cc);
        let formatted = self.body.read(cx).is_formatted();
        let header = self.render_header(cx);
        let guard = self.render_guard(cx);
        let from = self.render_from(cx);
        let to = self.render_recipients(Field::To, !show_cc, window, cx);
        let cc = show_cc.then(|| self.render_recipients(Field::Cc, false, window, cx));
        let bcc = show_cc.then(|| self.render_recipients(Field::Bcc, false, window, cx));
        let subject = self.render_subject();
        let link = self.render_link(cx);
        let bar = self.render_toolbar(formatted, cx);
        let body = match self.render_preview(cx) {
            Some(p) => p.into_any_element(),
            None => self.render_body(cx).into_any_element(),
        };
        let attachments = self.render_attachments(cx);
        let caption = self.render_caption(cx);
        // The drag ended or left the window: GPUI dropped its drag.
        if !cx.has_active_drag() {
            self.drop_hover = false;
        }
        // Over the formatted text, image files go in the text (the editor
        // shows where); anywhere else they are attached.
        let on_text = self.body.read(cx).drop_over();
        let dropping = (self.drop_hover && !on_text).then(|| render_drop_overlay(&t));
        div()
            .id("composer")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .access_keys()
            .on_action(cx.listener(|this, _: &Send, window, cx| this.send(false, window, cx)))
            .on_action(cx.listener(|this, _: &Close, window, cx| this.close(window, cx)))
            .on_action(
                cx.listener(|this, _: &RevealCc, window, cx| this.reveal_cc(Field::Cc, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &RevealBcc, window, cx| {
                    this.reveal_cc(Field::Bcc, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Attach, window, cx| this.attach(window, cx)))
            .on_action(cx.listener(|this, _: &OpenLink, window, cx| this.open_link(window, cx)))
            // Files dropped on the composer are attached (Explorer's drop);
            // while they are dragged over it, "Drop to attach" shows.
            .on_drag_move(cx.listener(
                |this, e: &gpui::DragMoveEvent<gpui::ExternalPaths>, _, cx| {
                    let over = e.bounds.contains(&e.event.position);
                    if over != this.drop_hover {
                        this.drop_hover = over;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.drop_hover = false;
                cx.notify();
                this.attach_paths(paths.paths().to_vec(), cx)
            }))
            // Ctrl+V with files copied in Explorer, from any of the Kit
            // fields (the address rows, Subject, a plain-text body): they
            // are attached, as the formatted body does (the Slint
            // composer's `c-paste-files`). Text pastes as usual.
            .capture_action(
                cx.listener(|this, _: &gpui_kit::component::input::Paste, _, cx| {
                    let files = komorebi::files::clipboard_files();
                    if !files.is_empty() {
                        this.attach_paths(files, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            // Ctrl+Enter and Esc from a Kit field: the field's own bindings
            // are deeper than the composer's.
            .capture_action(cx.listener(|this, a: &Enter, window, cx| {
                if a.secondary {
                    this.send(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.link_open {
                    this.close_link(window, cx);
                } else if this.drafts.suggest.is_some() {
                    this.close_suggestions(cx);
                } else {
                    this.close(window, cx);
                }
                cx.stop_propagation();
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.color.paper)
            .text_color(t.color.ink)
            .child(header)
            .children(guard)
            .child(from)
            .child(to)
            .children(cc)
            .children(bcc)
            .child(subject)
            .children(link)
            .children(bar)
            .child(body)
            .children(attachments)
            .child(caption)
            .children(dropping)
            .into_any_element()
    }
}

/// "Drop to attach" over the whole composer (the Slint app's file-hover
/// overlay): a wash with the words in the middle, said as a status.
fn render_drop_overlay(t: &crate::theme::Tokens) -> impl IntoElement + use<> {
    let words = SharedString::from(text::words::drop_to_attach());
    div()
        .id("compose-drop")
        .role(Role::Status)
        .aria_label(words.clone())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.color.wash)
        .typo(&t.text.subtitle)
        .text_color(t.color.ink)
        .child(words)
        .test_support()
}

#[cfg(test)]
mod tests;
