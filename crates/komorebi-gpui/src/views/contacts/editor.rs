//! The editor (the Slint app's `EditPane`): the title, what is wrong, the
//! name fields, the organization and job title, an address line per email
//! and a number line per phone (each with Remove, and Add), the groups
//! (separated by commas) and the notes, then Save and Cancel. Cancel asks
//! first when something changed.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, ParentElement, Role, SharedString,
    Styled, WeakEntity, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::contacts_page::{Form, Problem};

use super::{ContactForm, ContactsView, Side, field, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, space};
use crate::widgets::button::{Button, IconButton};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::field::TextField;
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::pane::ScrollArea;

type Weak = WeakEntity<ContactsView>;

impl ContactsView {
    /// A new contact, in the local book.
    pub fn new_contact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = komorebi::contacts_page::new_form(&self.db) {
            self.open_form(form, window, cx);
        }
    }

    /// Edit the contact shown (not in a read-only book).
    pub fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.page.shown() else {
            return;
        };
        if d.read_only {
            return;
        }
        let form = Form::open(&d.contact);
        self.open_form(form, window, cx);
    }

    fn open_form(&mut self, form: Form, window: &mut Window, cx: &mut Context<Self>) {
        let name = field(&form.name, window, cx);
        let subscriptions =
            vec![
                cx.subscribe_in(&name, window, |this, _, e: &InputEvent, window, cx| {
                    if matches!(e, InputEvent::PressEnter { .. }) {
                        this.save_contact(window, cx);
                    }
                }),
            ];
        let emails = form
            .emails
            .iter()
            .map(|e| email_field(e, window, cx))
            .collect();
        let phones = form.phones.iter().map(|p| field(p, window, cx)).collect();
        let f = ContactForm {
            given: field(&form.given, window, cx),
            family: field(&form.family, window, cx),
            org: field(&form.org, window, cx),
            title: field(&form.title, window, cx),
            groups: field(&form.groups, window, cx),
            note: field(&form.note, window, cx),
            name,
            emails,
            phones,
            form,
            problem: None,
            failed: None,
            _subscriptions: subscriptions,
        };
        f.name.update(cx, |s, cx| s.focus(window, cx));
        self.side = Side::Edit(Box::new(f));
        self.details_shown = true;
        self.copied = false;
        self.side_scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    /// The fields' text into the form.
    fn take_fields(&mut self, cx: &App) -> Option<&mut ContactForm> {
        let Side::Edit(f) = &mut self.side else {
            return None;
        };
        let read = |e: &Entity<InputState>| e.read(cx).value().to_string();
        f.form.name = read(&f.name);
        f.form.given = read(&f.given);
        f.form.family = read(&f.family);
        f.form.org = read(&f.org);
        f.form.title = read(&f.title);
        f.form.groups = read(&f.groups);
        f.form.note = read(&f.note);
        for (i, e) in f.emails.iter().enumerate() {
            f.form.set_email(i, &read(e));
        }
        for (i, p) in f.phones.iter().enumerate() {
            f.form.set_phone(i, &read(p));
        }
        Some(f)
    }

    pub fn add_email(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = email_field("", window, cx);
        if let Some(f) = self.take_fields(cx) {
            f.form.add_email();
            f.emails.push(input.clone());
            input.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    pub fn remove_email(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(f) = self.take_fields(cx)
            && index < f.emails.len()
        {
            f.form.remove_email(index);
            f.emails.remove(index);
        }
        cx.notify();
    }

    pub fn add_phone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = field("", window, cx);
        if let Some(f) = self.take_fields(cx) {
            f.form.add_phone();
            f.phones.push(input.clone());
            input.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    pub fn remove_phone(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(f) = self.take_fields(cx)
            && index < f.phones.len()
        {
            f.form.remove_phone(index);
            f.phones.remove(index);
        }
        cx.notify();
    }

    /// Save the editor; it stays open with the reason when it can't be.
    pub fn save_contact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = self.db.clone();
        let Some(f) = self.take_fields(cx) else {
            return;
        };
        match f.form.save(&db) {
            Ok(Ok(c)) => {
                self.side = Side::Detail;
                self.page.show_after_reload(c.id);
                self.page.query.clear();
                self.search.update(cx, |s, cx| s.set_value("", window, cx));
                self.reload(true, cx);
                self.details_shown = true;
                self.kick(cx);
                self.list_focus.focus(window, cx);
            }
            Ok(Err(problem)) => {
                // The field at fault takes the focus.
                let at = match &problem {
                    Problem::Empty => Some(f.name.clone()),
                    Problem::Email(addr) => f
                        .emails
                        .iter()
                        .find(|e| e.read(cx).value().trim() == addr.trim())
                        .cloned(),
                };
                f.problem = Some(problem);
                f.failed = None;
                if let Some(input) = at {
                    input.update(cx, |s, cx| s.focus(window, cx));
                }
                cx.notify();
            }
            Err(e) => {
                f.problem = None;
                f.failed = Some(format!("{e:#}"));
                cx.notify();
            }
        }
    }

    /// Cancel: ask first when something changed.
    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.take_fields(cx).is_some_and(|f| f.form.changed());
        if !changed {
            self.discard(window, cx);
            return;
        }
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::discard_question().into(),
                message: SharedString::default(),
                confirm: text::discard().into(),
                cancel: text::cancel().into(),
                destructive: true,
            },
            move |window, cx| {
                let _ = view.update(cx, |v, cx| v.discard(window, cx));
            },
            window,
            cx,
        );
    }

    /// Throw the editor's changes away.
    pub fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.side = Side::Detail;
        self.details_shown = self.page.shown().is_some();
        self.list_focus.focus(window, cx);
        cx.notify();
    }
}

fn email_field(
    value: &str,
    window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(text::email_placeholder())
    })
}

fn click(
    entity: &Weak,
    f: impl Fn(&mut ContactsView, &mut Window, &mut Context<ContactsView>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = entity.clone();
    move |_, window, cx| {
        let _ = entity.update(cx, |this, cx| f(this, window, cx));
    }
}

/// A field with its label above (Caption, ink-2), and an optional note
/// under it.
fn labelled(t: &Tokens, label: String, body: impl IntoElement, note: Option<String>) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_1))
        .child(
            div()
                .typo(&t.text.caption)
                .text_color(t.color.ink_2)
                .child(label),
        )
        .child(body)
        .children(note.map(|n| {
            div()
                .typo(&t.text.caption)
                .text_color(t.color.ink_3)
                .child(n)
        }))
        .into_any_element()
}

pub fn render(
    view: &mut ContactsView,
    single: bool,
    _window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let entity = cx.entity().downgrade();
    let Side::Edit(f) = &view.side else {
        return div().into_any_element();
    };
    let title = if f.form.is_saved() {
        text::edit_contact()
    } else {
        text::new_contact()
    };
    let problem = f
        .failed
        .as_deref()
        .map(text::detail)
        .or_else(|| f.problem.as_ref().map(text::problem));
    let pill = |state: &Entity<InputState>, name: String| {
        labelled(&t, name.clone(), TextField::new(state, name), None)
    };
    let email_lines = f.emails.iter().enumerate().map(|(i, e)| {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_2))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(TextField::new(e, text::email_n(i + 1))),
            )
            .child(
                IconButton::new(
                    ("contact-remove-email", i),
                    Icon::Delete,
                    text::remove_email_n(i + 1),
                )
                .on_click(click(&entity, move |this, _, cx| this.remove_email(i, cx))),
            )
    });
    let phone_lines = f.phones.iter().enumerate().map(|(i, p)| {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_2))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(TextField::new(p, text::phone_n(i + 1))),
            )
            .child(
                IconButton::new(
                    ("contact-remove-phone", i),
                    Icon::Delete,
                    text::remove_phone_n(i + 1),
                )
                .on_click(click(&entity, move |this, _, cx| this.remove_phone(i, cx))),
            )
    });
    let form = div()
        .w_full()
        .max_w(px(layout::DIALOG_W + layout::LIST_W_COMPACT / 2.))
        .flex()
        .flex_col()
        .gap(px(space::SP_4))
        .children(problem.map(|p| Notice::new("contact-problem", Kind::Error, p)))
        .child(pill(&f.name, text::name()))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(space::SP_3))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(layout::LIST_W_COMPACT / 2.))
                        .child(pill(&f.given, text::given_name())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(layout::LIST_W_COMPACT / 2.))
                        .child(pill(&f.family, text::family_name())),
                ),
        )
        .child(pill(&f.org, text::organization()))
        .child(pill(&f.title, text::job_title()))
        .child(labelled(
            &t,
            text::email(),
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_2))
                .children(email_lines)
                .child(
                    div().flex().child(
                        Button::new("contact-add-email")
                            .icon(Icon::Add)
                            .label(text::add_email())
                            .on_click(click(&entity, |this, window, cx| {
                                this.add_email(window, cx)
                            })),
                    ),
                ),
            None,
        ))
        .child(labelled(
            &t,
            text::phone(),
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_2))
                .children(phone_lines)
                .child(
                    div().flex().child(
                        Button::new("contact-add-phone")
                            .icon(Icon::Add)
                            .label(text::add_phone())
                            .on_click(click(&entity, |this, window, cx| {
                                this.add_phone(window, cx)
                            })),
                    ),
                ),
            None,
        ))
        .child(labelled(
            &t,
            text::groups(),
            TextField::new(&f.groups, text::groups()),
            Some(text::groups_note()),
        ))
        .child(pill(&f.note, text::notes()));
    // Save and Cancel stay in view below the scrolling form, so a long
    // contact never hides its commands.
    let commands = div()
        .flex_none()
        .flex()
        .flex_row()
        .gap(px(space::SP_2))
        .px(px(space::SP_8))
        .py(px(space::SP_4))
        .border_t_1()
        .border_color(t.color.line)
        .child(
            Button::new("contact-save")
                .primary()
                .label(text::save())
                .on_click(click(&entity, |this, window, cx| {
                    this.save_contact(window, cx)
                })),
        )
        .child(
            Button::new("contact-cancel")
                .label(text::cancel())
                .on_click(click(&entity, |this, window, cx| {
                    this.cancel_edit(window, cx)
                })),
        );
    div()
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div().flex_1().min_h_0().child(
                ScrollArea::new("contact-edit-scroll", &view.side_scroll).child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_4))
                        .px(px(space::SP_8))
                        .pt(px(space::SP_6))
                        .pb(px(space::SP_8))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(space::SP_2))
                                .when(single, |el| {
                                    el.child(
                                        IconButton::new(
                                            "contact-edit-back",
                                            Icon::ChevronLeft,
                                            text::back(),
                                        )
                                        .on_click(click(&entity, |this, window, cx| {
                                            this.back(window, cx)
                                        })),
                                    )
                                })
                                .child(
                                    div()
                                        .id("contact-edit-title")
                                        .role(Role::Heading)
                                        .aria_label(title.clone())
                                        .typo(&t.text.subtitle)
                                        .text_color(t.color.ink)
                                        .child(title)
                                        .test_support(),
                                ),
                        )
                        .child(form),
                ),
            ),
        )
        .child(commands)
        .into_any_element()
}
