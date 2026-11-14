//! Settings > Signatures (the Slint app's `src/app/signatures.rs`, settings
//! half): the named signatures as cards (Edit, Delete), Add signature, and
//! a signature's page: its name, its formatted text in the composer's own
//! body ([`RichEditor`]) with the Slint app's reduced format bar (Bold,
//! Italic, Underline, Strikethrough, the font colour, the two lists, the
//! indents, Link with its address row, Clear formatting and the alignment;
//! no quote or table), and at most one logo (PNG or JPEG, 200 KB at most).
//! Delete asks first and says which accounts lose it as a default.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Image,
    ImageFormat, InteractiveElement as _, IntoElement, ParentElement, PathPromptOptions, Render,
    SharedString, Styled, Subscription, Window, div, img, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use komorebi::compose::rich::Action;
use komorebi::db::{SignatureError, SignatureInfo};
use komorebi::mail::rich::edit::Editor;
use komorebi::mail::rich::{Align, ColorId, Doc, ListKind, Logo, LogoError, MAX_LOGO_BYTES};
use komorebi_i18n::tr;

use super::card::{CARD_MAX_W, Card, labelled, note, page_head};
use super::{Section, text};
use crate::icons::Icon;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::views::Db;
use crate::views::composer::editor::{EditorEvent, RichEditor};
use crate::views::composer::text::words;
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::field::TextField;
use crate::widgets::menu;
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
    LogoRefused,
    Failed(String),
}

impl SigError {
    fn text(&self) -> String {
        match self {
            SigError::EmptyName => text::sig_name_empty(),
            SigError::NameTaken => text::sig_name_taken(),
            SigError::LogoRefused => text::sig_logo_refused(),
            SigError::Failed(e) => text::not_saved(e),
        }
    }
}

/// A logo file, checked as the Slint app checks it: 200 KB at most, PNG or
/// JPEG.
pub fn read_logo(path: &std::path::Path) -> Result<Logo, LogoError> {
    let bytes = std::fs::metadata(path)
        .ok()
        .filter(|m| m.len() <= MAX_LOGO_BYTES as u64)
        .and_then(|_| std::fs::read(path).ok())
        .ok_or(LogoError::TooLarge)?;
    Logo::from_bytes(&bytes)
}

/// The open signature's page.
struct Open {
    /// None: a new signature.
    id: Option<i64>,
    name: Entity<InputState>,
    body: Entity<RichEditor>,
    logo: Option<Logo>,
    /// Link: the address row under the bar, and its error.
    link: Entity<InputState>,
    link_open: bool,
    link_error: bool,
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
        let (name_text, doc, logo) = match &sig {
            Some(s) => (s.name.clone(), s.doc(), s.logo()),
            None => (String::new(), Doc::default(), None),
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
            ed.set_doc(Some(Editor::new(doc)), 0, cx);
            ed
        });
        name.update(cx, |s, cx| s.focus(window, cx));
        let link = cx.new(|cx| InputState::new(window, cx).placeholder(words::link_placeholder()));
        let subscriptions = vec![
            // The bar's toggles follow the caret.
            cx.subscribe(&body, |_, _, e: &EditorEvent, cx| {
                if matches!(e, EditorEvent::Moved | EditorEvent::Changed) {
                    cx.notify();
                }
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
            logo,
            link,
            link_open: false,
            link_error: false,
            _subscriptions: subscriptions,
        });
        self.error = None;
        cx.notify();
    }

    /// Esc: the link row closes first, then the page (back to the list,
    /// nothing saved). True if either was open.
    pub fn close(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(open) = &mut self.open
            && open.link_open
        {
            open.link_open = false;
            open.link_error = false;
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
        let doc = open
            .body
            .read(cx)
            .editor()
            .map(|e| e.doc().clone())
            .unwrap_or_default();
        let saved = match open.id {
            Some(id) => self
                .db
                .update_signature(id, &name, &doc, open.logo.as_ref())
                .map(|()| id),
            None => self.db.add_signature(&name, &doc, open.logo.as_ref()),
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
        let link = open.link.clone();
        link.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
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
        if url.trim().is_empty() || body.update(cx, |b, cx| b.link(&url, cx)) {
            open.link_open = false;
            open.link_error = false;
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
            let focus = open.body.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// A logo picked (or refused).
    pub fn set_logo(&mut self, logo: Result<Logo, LogoError>, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        match logo {
            Ok(logo) => {
                open.logo = Some(logo);
                self.error = None;
            }
            Err(_) => self.error = Some(SigError::LogoRefused),
        }
        cx.notify();
    }

    fn pick_logo(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!("Add a logo").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let logo = read_logo(&path);
                let _ = this.update(cx, |this, cx| this.set_logo(logo, cx));
            }
        })
        .detach();
    }

    fn remove_logo(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.open {
            open.logo = None;
        }
        cx.notify();
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
        let act = |id: &'static str, icon: Icon, name: String, on: bool, action: Action| {
            let body = body.clone();
            IconButton::new(id, icon, name.clone())
                .small()
                .toggled(on)
                .tooltip(name)
                .on_click(move |_, _, cx| body.update(cx, |b, cx| b.act(&action, cx)))
        };
        let pick = {
            let body = body.clone();
            move |action: Action| {
                let body = body.clone();
                move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut gpui::App| {
                    body.update(cx, |b, cx| b.act(&action, cx))
                }
            }
        };
        let colour = f.color;
        let colours = {
            let pick = pick.clone();
            IconButton::new("sig-colour", Icon::Color, words::font_colour())
                .small()
                .tooltip(words::font_colour())
                .dropdown_menu(move |m, _, cx| {
                    std::iter::once(None)
                        .chain(ColorId::ALL.iter().copied().map(Some))
                        .fold(menu::brand(m, cx), |m, c| {
                            m.item(
                                PopupMenuItem::new(words::colour(c))
                                    .checked(c == colour)
                                    .on_click(pick(Action::Color(c))),
                            )
                        })
                })
        };
        let align = f.align;
        let aligns = IconButton::new("sig-align", Icon::More, words::more())
            .small()
            .tooltip(words::more())
            .dropdown_menu(move |m, _, cx| {
                [
                    (words::align_left(), Align::Left),
                    (words::align_center(), Align::Center),
                    (words::align_right(), Align::Right),
                    (words::justify(), Align::Justify),
                ]
                .into_iter()
                .fold(menu::brand(m, cx), |m, (label, a)| {
                    m.item(
                        PopupMenuItem::new(label)
                            .checked(a == align)
                            .on_click(pick(Action::Align(a))),
                    )
                })
            });
        let gap = || div().w(px(space::SP_2));
        let bar = div()
            .id("sig-format-bar")
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SP_1))
            .child(act(
                "sig-bold",
                Icon::Bold,
                words::bold(),
                f.b,
                Action::Bold,
            ))
            .child(act(
                "sig-italic",
                Icon::Italic,
                words::italic(),
                f.i,
                Action::Italic,
            ))
            .child(act(
                "sig-underline",
                Icon::Underline,
                words::underline(),
                f.u,
                Action::Underline,
            ))
            .child(act(
                "sig-strike",
                Icon::Strikethrough,
                words::strike(),
                f.s,
                Action::Strike,
            ))
            .child(colours)
            .child(gap())
            .child(act(
                "sig-bullets",
                Icon::Bullets,
                words::bullets(),
                f.list == Some(ListKind::Bullet),
                Action::Bullets,
            ))
            .child(act(
                "sig-numbers",
                Icon::Numbers,
                words::numbers(),
                f.list == Some(ListKind::Number),
                Action::Numbers,
            ))
            .child(act(
                "sig-outdent",
                Icon::IndentLess,
                words::outdent(),
                false,
                Action::Indent(false),
            ))
            .child(act(
                "sig-indent",
                Icon::IndentMore,
                words::indent(),
                false,
                Action::Indent(true),
            ))
            .child(gap())
            .child(
                IconButton::new("sig-link", Icon::Link, words::link_tip())
                    .small()
                    .toggled(f.link.is_some() || open.link_open)
                    .tooltip(words::link_tip())
                    .on_click(cx.listener(|this, _, window, cx| this.open_link(window, cx))),
            )
            .child(act(
                "sig-clear",
                Icon::ClearFormat,
                words::clear(),
                false,
                Action::Clear,
            ))
            .child(aligns);
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
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(TextField::new(&open.link, words::link_address())),
                        )
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
                .when(open.link_error, |el| {
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
            .child(div().min_h(px(layout::CARD_H)).child(open.body.clone()));
        let logo: AnyElement = match &open.logo {
            Some(l) => {
                let format = if l.mime == "image/png" {
                    ImageFormat::Png
                } else {
                    ImageFormat::Jpeg
                };
                let r = l.reference();
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SP_3))
                    .child(
                        img(Arc::new(Image::from_bytes(format, l.bytes.clone())))
                            .w(px(f32::from(r.width)))
                            .h(px(f32::from(r.height)))
                            .max_w(px(geo::FORM_W)),
                    )
                    .child(
                        Button::new("sig-logo-remove")
                            .label(tr!("Remove logo"))
                            .variant(Variant::Subtle)
                            .on_click(cx.listener(|this, _, _, cx| this.remove_logo(cx))),
                    )
                    .into_any_element()
            }
            None => Button::new("sig-logo-add")
                .label(tr!("Add logo…"))
                .icon(Icon::Image)
                .on_click(cx.listener(|this, _, _, cx| this.pick_logo(cx)))
                .into_any_element(),
        };
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
            .child(labelled(
                tr!("Logo"),
                logo,
                Some(tr!("The logo must be a PNG or JPEG file of 200 KB or less.").into()),
                cx,
            ))
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
