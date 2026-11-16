//! Settings > Signatures (the Slint app's `src/app/signatures.rs`, settings
//! half): the named signatures as cards (Edit, Delete), Add signature, and
//! a signature's page: its name, its formatted text in the composer's own
//! body ([`RichEditor`]) with the composer's format bar (`format_bar.rs`,
//! shared so both stay in step) less what a signature cannot hold (a
//! quote), Link with its address row, and More (tables, the picture's
//! size). Pictures go in the text like the composer's: Insert picture,
//! dropped or pasted, as many as it needs (a logo beside the name in a
//! table, a row of logos under it); they are saved with the signature.
//! Delete asks first and says which accounts lose it as a default.

use std::rc::Rc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use komorebi::compose::rich::Action;
use komorebi::db::{SignatureError, SignatureInfo};
use komorebi::mail::rich::edit::Editor;
use komorebi::mail::rich::edit::TableOp;
use komorebi::mail::rich::{Doc, Images};
use komorebi_i18n::tr;

use super::card::{CARD_MAX_W, Card, labelled, note, page_head};
use super::{Section, text};
use crate::icons::Icon;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::Db;
use crate::views::composer::editor::{EditorEvent, RichEditor};
use crate::views::composer::format_bar::{BarState, Ctl, FormatBar};
use crate::views::composer::text::words;
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::field::TextField;
use crate::widgets::notice::{self, Notice};

/// What the signatures page tells Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignaturesEvent {
    /// A signature was saved or deleted (the account pages and the
    /// composer's menu read them again).
    Changed,
}

/// Why a signature's page can't be saved, or what failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigError {
    EmptyName,
    NameTaken,
    /// Files dropped or pasted that are not pictures it can hold.
    PictureRefused,
    Failed(String),
}

impl SigError {
    fn text(&self) -> String {
        match self {
            SigError::EmptyName => text::sig_name_empty(),
            SigError::NameTaken => text::sig_name_taken(),
            SigError::PictureRefused => text::sig_picture_refused(),
            SigError::Failed(e) => text::not_saved(e),
        }
    }
}

/// An entry of the signature bar's More menu.
#[derive(Clone)]
enum SigMore {
    Format(Action),
    Table,
    OwnSize,
    UseOriginal,
    AltText,
}

/// The open signature's page.
struct Open {
    /// None: a new signature.
    id: Option<i64>,
    name: Entity<InputState>,
    body: Entity<RichEditor>,
    /// Link: the address row under the bar, and its error.
    link: Entity<InputState>,
    link_open: bool,
    link_error: bool,
    /// The row edits the selected picture's alt text, not a link.
    link_alt: bool,
    /// The format bar's focus and width.
    fmt: BarState,
    _subscriptions: Vec<Subscription>,
}

/// Settings > Signatures.
pub struct SignaturesPage {
    db: Db,
    list: Vec<SignatureInfo>,
    open: Option<Open>,
    error: Option<SigError>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SignaturesEvent> for SignaturesPage {}

impl Focusable for SignaturesPage {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SignaturesPage {
    pub fn new(db: Db, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            list: db.signatures().unwrap_or_default(),
            db,
            open: None,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        }
    }

    /// Read the list again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.list = self.db.signatures().unwrap_or_default();
        cx.notify();
    }

    /// The signatures listed.
    pub fn list(&self) -> &[SignatureInfo] {
        &self.list
    }

    /// Whether a signature's page shows.
    pub fn editing(&self) -> bool {
        self.open.is_some()
    }

    /// The error under the page, if any.
    pub fn error(&self) -> Option<&SigError> {
        self.error.as_ref()
    }

    /// The open signature's name box.
    pub fn name_input(&self) -> Option<&Entity<InputState>> {
        self.open.as_ref().map(|o| &o.name)
    }

    /// The open signature's body.
    pub fn body(&self) -> Option<&Entity<RichEditor>> {
        self.open.as_ref().map(|o| &o.body)
    }

    /// Open signature `id`'s page (None: a new one).
    pub fn open(&mut self, id: Option<i64>, window: &mut Window, cx: &mut Context<Self>) {
        let sig = id.and_then(|id| self.db.signature(id).ok().flatten());
        let (name_text, doc, images) = match &sig {
            Some(s) => (s.name.clone(), s.doc(), s.images.clone()),
            None => (String::new(), Doc::default(), Images::default()),
        };
        let name = cx.new(|cx| InputState::new(window, cx));
        name.update(cx, |s, cx| s.set_value(name_text, window, cx));
        let db = Rc::clone(&self.db);
        let body = cx.new(|cx| {
            let mut ed = RichEditor::new(Some(db), cx);
            ed.set_words(
                tr!("Signature").into(),
                tr!("Your name, and what people should know about you").into(),
            );
            ed.set_doc(Some(Editor::with_images(doc, images)), 0, cx);
            ed
        });
        name.update(cx, |s, cx| s.focus(window, cx));
        let link = cx.new(|cx| InputState::new(window, cx).placeholder(words::link_placeholder()));
        let subscriptions = vec![
            // The bar's toggles follow the caret.
            cx.subscribe(&body, |this, _, e: &EditorEvent, cx| match e {
                // The bar follows the caret; the box follows the text's height.
                EditorEvent::Moved | EditorEvent::Changed | EditorEvent::Resized => cx.notify(),
                // Not pictures: a signature holds no attachments.
                EditorEvent::PasteFiles(_) => {
                    this.error = Some(SigError::PictureRefused);
                    cx.notify();
                }
                EditorEvent::DropOver(_) => {}
            }),
            cx.subscribe_in(
                &link,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter { .. } => this.insert_link(window, cx),
                    InputEvent::Change => {
                        if let Some(open) = &mut this.open
                            && open.link_error
                        {
                            open.link_error = false;
                            cx.notify();
                        }
                    }
                    _ => {}
                },
            ),
        ];
        self.open = Some(Open {
            id: sig.map(|s| s.id),
            name,
            body,
            link,
            link_open: false,
            link_error: false,
            link_alt: false,
            fmt: BarState::new(cx),
            _subscriptions: subscriptions,
        });
        self.error = None;
        cx.notify();
    }

    /// The snapshot action `signature-pictures` (made-up data, only under
    /// `KOMOREBI_OFFLINE`): a new signature's page holding a signature
    /// pasted from Outlook, a logo beside the name and partner logos under.
    pub fn sample(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_none() {
            return;
        }
        self.open(None, window, cx);
        let (name, doc, images) = komorebi::compose::sample::signature();
        if let Some(open) = &self.open {
            open.name.update(cx, |s, cx| s.set_value(name, window, cx));
            open.body.update(cx, |b, cx| {
                b.set_doc(Some(Editor::with_images(doc, images)), 0, cx);
            });
        }
    }

    /// Esc: the link row closes first, then the page (back to the list,
    /// nothing saved). True if either was open.
    pub fn close(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(open) = &mut self.open
            && open.link_open
        {
            open.link_open = false;
            open.link_error = false;
            open.link_alt = false;
            cx.notify();
            return true;
        }
        self.close_page(cx)
    }

    /// Cancel: back to the list, nothing saved. True if a page was open.
    pub fn close_page(&mut self, cx: &mut Context<Self>) -> bool {
        let was = self.open.take().is_some();
        self.error = None;
        self.reload(cx);
        was
    }

    /// Save the open signature.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        let name = open.name.read(cx).value().to_string();
        let (doc, images) = open
            .body
            .read(cx)
            .editor()
            .map(|e| (e.doc().clone(), e.images().clone()))
            .unwrap_or_default();
        let saved = match open.id {
            Some(id) => self
                .db
                .update_signature(id, &name, &doc, &images)
                .map(|()| id),
            None => self.db.add_signature(&name, &doc, &images),
        };
        match saved {
            Ok(_) => {
                self.close_page(cx);
                cx.emit(SignaturesEvent::Changed);
            }
            Err(e) => {
                self.error = Some(match e.downcast_ref::<SignatureError>() {
                    Some(SignatureError::EmptyName) => SigError::EmptyName,
                    Some(SignatureError::NameTaken) => SigError::NameTaken,
                    _ => SigError::Failed(format!("{e:#}")),
                });
                cx.notify();
            }
        }
    }

    /// Whether the link row shows.
    pub fn link_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.link_open)
    }

    /// Whether the link row says the address is not one.
    pub fn link_error(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.link_error)
    }

    /// The link row's address box.
    pub fn link_input(&self) -> Option<&Entity<InputState>> {
        self.open.as_ref().map(|o| &o.link)
    }

    /// Link (Ctrl+K): the address row, focused, empty.
    pub fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        open.link_open = true;
        open.link_error = false;
        open.link_alt = false;
        let link = open.link.clone();
        link.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// More > Picture > Alt text…: the link row, as the selected picture's
    /// alt text (as in the composer).
    pub fn open_alt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        let Some(alt) = open.body.read(cx).picture_alt() else {
            return;
        };
        open.link_open = true;
        open.link_error = false;
        open.link_alt = true;
        let link = open.link.clone();
        link.update(cx, |s, cx| {
            s.set_value(alt, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// Whether the row edits a picture's alt text.
    pub fn alt_open(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.link_open && o.link_alt)
    }

    /// Insert (Enter in the row): the address on the selection, or the
    /// error under the row when it is not a web or e-mail address; an
    /// empty row just closes.
    pub fn insert_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        let url = open.link.read(cx).value().to_string();
        let body = open.body.clone();
        let alt = open.link_alt;
        if alt {
            body.update(cx, |b, cx| b.set_picture_alt(&url, cx));
        }
        if alt || url.trim().is_empty() || body.update(cx, |b, cx| b.link(&url, cx)) {
            open.link_open = false;
            open.link_error = false;
            open.link_alt = false;
            let focus = body.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        } else {
            open.link_error = true;
        }
        cx.notify();
    }

    fn close_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.open {
            open.link_open = false;
            open.link_error = false;
            open.link_alt = false;
            let focus = open.body.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// A command from More on the signature's text (tables, the picture).
    fn more(&mut self, act: SigMore, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(act, SigMore::AltText) {
            return self.open_alt(window, cx);
        }
        let Some(open) = &self.open else {
            return;
        };
        let body = open.body.clone();
        body.update(cx, |b, cx| match act {
            SigMore::AltText => {}
            SigMore::Format(a) => b.act(&a, cx),
            SigMore::Table => b.insert_table(1, 2, cx),
            SigMore::OwnSize => {
                b.picture_own_size(cx);
            }
            SigMore::UseOriginal => {
                b.use_original(cx);
            }
        });
        let focus = body.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Delete, answered.
    pub fn delete_now(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.db.delete_signature(id) {
            Ok(_) => {
                self.error = None;
                cx.emit(SignaturesEvent::Changed);
            }
            Err(e) => self.error = Some(SigError::Failed(format!("{e:#}"))),
        }
        self.reload(cx);
    }

    fn ask_delete(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .list
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let users = self.db.signature_users(id).map_or(0, |u| u.len());
        let mut message = text::q_sig_delete_text();
        if users > 0 {
            message.push(' ');
            message.push_str(&text::q_sig_delete_users(users));
        }
        let this = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::q_sig_delete_title(&name).into(),
                message: message.into(),
                confirm: tr!("Delete").into(),
                cancel: tr!("Cancel").into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = this.update(cx, |this, cx| this.delete_now(id, cx));
            },
            window,
            cx,
        );
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let empty = self.list.is_empty();
        let cards: Vec<AnyElement> = self
            .list
            .iter()
            .map(|s| {
                let id = s.id;
                Card::new(SharedString::from(format!("sig-{id}")), s.name.clone())
                    .icon(Icon::Signature)
                    .note(s.first_line.clone())
                    .control(
                        Button::new(SharedString::from(format!("sig-edit-{id}")))
                            .label(tr!("Edit"))
                            .name(tr!("Edit {0}", s.name))
                            .variant(Variant::OnInset)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open(Some(id), window, cx)
                            })),
                    )
                    .control(
                        IconButton::new(
                            SharedString::from(format!("sig-delete-{id}")),
                            Icon::Delete,
                            tr!("Delete {0}", s.name),
                        )
                        .tooltip(tr!("Delete {0}", s.name))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.ask_delete(id, window, cx)),
                        ),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(page_head(Section::Signatures.name(), None, cx))
            .when(empty, |el| {
                el.child(div().mt(px(space::SP_4)).child(note(
                    tr!("No signatures yet. Add one to use it in new mail and replies."),
                    cx,
                )))
            })
            .when(!empty, |el| {
                el.child(div().mt(px(space::SP_6)).children(cards))
            })
            .child(
                div().mt(px(space::SP_4)).child(
                    Button::new("sig-add")
                        .label(tr!("Add signature"))
                        .icon(Icon::Add)
                        .variant(if empty {
                            Variant::Primary
                        } else {
                            Variant::Secondary
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.open(None, window, cx))),
                ),
            )
            .children(self.render_error())
            .into_any_element()
    }

    fn render_error(&self) -> Option<AnyElement> {
        let e = self.error.as_ref()?;
        Some(
            div()
                .mt(px(space::SP_3))
                .max_w(px(CARD_MAX_W))
                .child(Notice::new("sig-error", notice::Kind::Error, e.text()))
                .into_any_element(),
        )
    }

    fn render_open(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let t = cx.tokens().clone();
        let c = t.color;
        let body = open.body.clone();
        let f = body.read(cx).format().unwrap_or_default();
        let link_on = f.link.is_some() || open.link_open;
        let in_table = f.in_table;
        let merged = f.table_merged;
        let picture = body.read(cx).selected_picture().is_some();
        let original = body.read(cx).has_original();
        let body_h = body.read(cx).page_height().max(layout::CARD_H);
        let page = cx.entity().downgrade();
        // The composer's bar, less what a signature cannot hold (a quote,
        // a table), and Link pinned after it.
        let bar = FormatBar::new(
            "sig",
            &Ctl::SIGNATURE,
            f,
            &open.fmt,
            cx.entity_id(),
            move |action, window, cx| {
                body.update(cx, |b, cx| b.act(action, cx));
                let focus = body.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            },
        )
        .pinned({
            let page = page.clone();
            move |focus| {
                IconButton::new("sig-link", Icon::Link, words::link_tip())
                    .small()
                    .toggled(link_on)
                    .tooltip(words::link_tip())
                    .track_focus(&focus)
                    .on_click(move |_, window, cx| {
                        let _ = page.update(cx, |this, cx| this.open_link(window, cx));
                    })
                    .into_any_element()
            }
        })
        // More: what left the bar, then a table and the selected
        // picture's commands, as in the composer.
        .more(move |m, window, cx| {
            let p = page.clone();
            let item = move |label: String, act: SigMore| {
                let p = p.clone();
                PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    let act = act.clone();
                    let _ = p.update(cx, |this, cx| this.more(act, window, cx));
                })
            };
            let mut m = m.item(item(words::insert_table(), SigMore::Table));
            if in_table {
                let tables = item.clone();
                m = m.submenu(words::table(), window, cx, move |m, _, cx| {
                    TableOp::offered(merged).iter().copied().fold(
                        crate::widgets::menu::brand(m, cx),
                        |m, op| {
                            m.item(tables(
                                words::table_op(op),
                                SigMore::Format(Action::Table(op)),
                            ))
                        },
                    )
                });
            }
            if picture {
                let pics = item.clone();
                m = m.submenu(words::picture(), window, cx, move |m, _, cx| {
                    let mut m = crate::widgets::menu::brand(m, cx)
                        .item(pics(words::alt_text(), SigMore::AltText))
                        .item(pics(
                            words::picture_bigger(),
                            SigMore::Format(Action::SizeStep(true)),
                        ))
                        .item(pics(
                            words::picture_smaller(),
                            SigMore::Format(Action::SizeStep(false)),
                        ))
                        .item(pics(words::picture_own_size(), SigMore::OwnSize));
                    if original {
                        m = m.item(pics(words::use_original(), SigMore::UseOriginal));
                    }
                    m
                });
            }
            m
        });
        let link_row = open.link_open.then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_1))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::SP_2))
                        .child(div().flex_1().min_w_0().child(TextField::new(
                            &open.link,
                            if open.link_alt {
                                words::alt_text_field()
                            } else {
                                words::link_address()
                            },
                        )))
                        .child(
                            Button::new("sig-link-insert")
                                .label(words::insert())
                                .variant(Variant::Primary)
                                .tooltip(words::insert_link_tip())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.insert_link(window, cx)),
                                ),
                        )
                        .child(
                            IconButton::new("sig-link-cancel", Icon::Dismiss, words::cancel_tip())
                                .small()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.close_link(window, cx)),
                                ),
                        ),
                )
                .when(open.link_error && !open.link_alt, |el| {
                    el.child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(c.ink)
                            .child(words::link_bad()),
                    )
                })
        });
        let editor = div()
            .w_full()
            .max_w(px(geo::FORM_W))
            .min_h(px(geo::SIG_BODY_H))
            .p(px(space::SP_3))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .flex()
            .flex_col()
            .gap(px(space::SP_2))
            .child(bar)
            .children(link_row)
            // As tall as the signature (a layout table with logos), so all of
            // it shows; at least a card's height.
            .child(div().h(px(body_h)).child(open.body.clone()));
        let heading = open
            .id
            .and_then(|id| self.list.iter().find(|s| s.id == id))
            .map_or_else(|| tr!("Add signature"), |s| s.name.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(geo::FIELD_GAP))
            .child(page_head(heading, None, cx))
            .child(div().max_w(px(geo::FORM_W)).child(labelled(
                tr!("Name"),
                TextField::new(&open.name, tr!("Name")),
                None,
                cx,
            )))
            .child(labelled(tr!("Signature"), editor, None, cx))
            .children(self.render_error())
            .child(
                div()
                    .flex()
                    .gap(px(space::SP_2))
                    .child(
                        Button::new("sig-save")
                            .label(tr!("Save"))
                            .variant(Variant::Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        Button::new("sig-cancel")
                            .label(tr!("Cancel"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_page(cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SignaturesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = if self.open.is_some() {
            self.render_open(cx)
        } else {
            self.render_list(cx)
        };
        div()
            .id("settings-signatures")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &super::Link, window, cx| this.open_link(window, cx)))
            .w_full()
            .child(page)
            .test_support()
    }
}
