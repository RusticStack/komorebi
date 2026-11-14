//! The details pane (mockup `contacts-*`, right pane): Edit and Delete at
//! the top right, the large avatar, the name and the job line, New
//! message, the read-only and conflict notices, then label and value
//! lines: each address (with New message to and Copy), each number (with
//! Copy), the groups, the notes and the address book. With no contact
//! chosen, the count.

use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, Styled, WeakEntity, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::contacts_page::Detail;

use super::{ContactsView, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, space};
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::pane::ScrollArea;
use crate::widgets::spot::SenderSpot;

type Weak = WeakEntity<ContactsView>;

fn click(
    entity: &Weak,
    f: impl Fn(&mut ContactsView, &mut Window, &mut Context<ContactsView>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = entity.clone();
    move |_, window, cx| {
        let _ = entity.update(cx, |this, cx| f(this, window, cx));
    }
}

pub fn render(
    view: &mut ContactsView,
    single: bool,
    _window: &mut Window,
    cx: &mut Context<ContactsView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let entity = cx.entity().downgrade();
    let Some(d) = view.page.shown().cloned() else {
        return div()
            .id("contact-none")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .typo(&t.text.body)
            .text_color(t.color.ink_3)
            .child(text::count(view.page.total()))
            .into_any_element();
    };
    let bar = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_1))
        .pt(px(space::SP_3))
        .px(px(space::SP_3))
        .when(single, |el| {
            el.child(
                IconButton::new("contact-back", Icon::ChevronLeft, text::back())
                    .on_click(click(&entity, |this, window, cx| this.back(window, cx))),
            )
        })
        .child(div().flex_1())
        .child(
            IconButton::new("contact-edit", Icon::Edit, text::edit())
                .disabled(d.read_only)
                .on_click(click(&entity, |this, window, cx| this.edit(window, cx))),
        )
        .child(
            IconButton::new("contact-delete", Icon::Delete, text::delete())
                .disabled(d.read_only)
                .on_click(click(&entity, |this, window, cx| {
                    this.ask_delete(window, cx)
                })),
        );
    div()
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .child(bar)
        .child(
            div().flex_1().min_h_0().child(
                ScrollArea::new("contact-scroll", &view.side_scroll).child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_6))
                        .px(px(space::SP_10 + space::SP_2))
                        .pb(px(space::SP_8))
                        .child(head(&d, &t, &entity))
                        .children(notices(&d, &entity))
                        .child(lines(view, &d, &t, &entity)),
                ),
            ),
        )
        .into_any_element()
}

/// The avatar, the name, the job line and New message.
fn head(d: &Detail, t: &Tokens, entity: &Weak) -> AnyElement {
    let c = &d.contact;
    let org = text::org_line(&c.title, &c.org);
    let spot = if d.name.is_empty() {
        d.first_email.as_str()
    } else {
        d.name.as_str()
    };
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(space::SP_4))
        // The large avatar (the mockup's `.cd .av`): 72 px on wash, the
        // initials in the title step in ink, as the shared spot draws it.
        .child(
            SenderSpot::new(spot)
                .size(layout::AVATAR_LG)
                .id("contact-avatar"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_1))
                .child(
                    div()
                        .id("contact-name")
                        .role(Role::Heading)
                        .aria_label(d.name.clone())
                        .typo(&t.text.large)
                        .text_color(t.color.ink)
                        .child(d.name.clone())
                        .test_support(),
                )
                .when(!org.is_empty(), |el| {
                    el.child(
                        div()
                            .typo(&t.text.body)
                            .text_color(t.color.ink_2)
                            .child(org),
                    )
                }),
        )
        .child(
            div().flex().flex_row().gap(px(space::SP_2)).child(
                Button::new("contact-write")
                    .icon(Icon::Mail)
                    .label(text::new_message())
                    .disabled(c.emails.is_empty())
                    .on_click(click(entity, |this, _, cx| this.write_to("", cx))),
            ),
        )
        .into_any_element()
}

/// Read only, and the conflict's choice.
fn notices(d: &Detail, entity: &Weak) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if d.read_only {
        out.push(
            Notice::new(
                "contact-read-only",
                Kind::Info,
                format!("{} {}", text::read_only(), text::read_only_text()),
            )
            .into_any_element(),
        );
    }
    if d.conflict {
        out.push(
            Notice::new(
                "contact-conflict",
                Kind::Warning,
                format!("{} {}", text::conflict(), text::conflict_text()),
            )
            .action(
                Button::new("contact-keep-mine")
                    .variant(Variant::OnInset)
                    .label(text::keep_mine())
                    .on_click(click(entity, |this, _, cx| this.keep_mine(cx))),
            )
            .action(
                Button::new("contact-keep-server")
                    .variant(Variant::OnInset)
                    .label(text::keep_server())
                    .on_click(click(entity, |this, _, cx| this.keep_server(cx))),
            )
            .into_any_element(),
        );
    }
    out
}

/// One label and value line, with its buttons at the right.
fn line(
    t: &Tokens,
    id: SharedString,
    label: String,
    value: String,
    kind: String,
    buttons: Vec<AnyElement>,
) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .min_h(px(layout::CONTROL_H))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_3))
        .child(
            div()
                .w(px(layout::FIELD_LABEL_W + layout::FIELD_LABEL_W))
                .flex_none()
                .typo(&t.text.body)
                .text_color(t.color.ink_2)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_baseline()
                .gap(px(space::SP_2))
                .child(
                    div()
                        .typo(&t.text.body)
                        .text_color(t.color.ink)
                        .child(value),
                )
                .when(!kind.is_empty(), |el| {
                    el.child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(t.color.ink_3)
                            .child(kind),
                    )
                }),
        )
        .children(buttons)
        .into_any_element()
}

fn lines(view: &ContactsView, d: &Detail, t: &Tokens, entity: &Weak) -> AnyElement {
    let c = &d.contact;
    let mut out: Vec<AnyElement> = Vec::new();
    for (i, e) in c.emails.iter().enumerate() {
        let (to, copy) = (e.addr.clone(), e.addr.clone());
        out.push(line(
            t,
            format!("contact-email-{i}").into(),
            if i == 0 { text::email() } else { String::new() },
            e.addr.clone(),
            text::label(&e.label),
            vec![
                IconButton::new(
                    ("contact-write-to", i),
                    Icon::Compose,
                    text::new_message_to(&e.addr),
                )
                .small()
                .on_click(click(entity, move |this, _, cx| this.write_to(&to, cx)))
                .into_any_element(),
                IconButton::new(("contact-copy-email", i), Icon::Copy, text::copy())
                    .small()
                    .on_click(click(entity, move |this, _, cx| this.copy(&copy, cx)))
                    .into_any_element(),
            ],
        ));
    }
    for (i, p) in c.phones.iter().enumerate() {
        let copy = p.number.clone();
        out.push(line(
            t,
            format!("contact-phone-{i}").into(),
            if i == 0 { text::phone() } else { String::new() },
            p.number.clone(),
            text::label(&p.label),
            vec![
                IconButton::new(("contact-copy-phone", i), Icon::Copy, text::copy())
                    .small()
                    .on_click(click(entity, move |this, _, cx| this.copy(&copy, cx)))
                    .into_any_element(),
            ],
        ));
    }
    let groups: Vec<&str> = c
        .groups
        .iter()
        .map(String::as_str)
        .filter(|g| !g.is_empty())
        .collect();
    if !groups.is_empty() {
        out.push(line(
            t,
            "contact-groups".into(),
            text::groups(),
            groups.join(", "),
            String::new(),
            Vec::new(),
        ));
    }
    if !c.note.is_empty() {
        out.push(line(
            t,
            "contact-note".into(),
            text::notes(),
            c.note.clone(),
            String::new(),
            Vec::new(),
        ));
    }
    let book = if d.book.is_empty() {
        text::on_this_computer()
    } else {
        d.book.clone()
    };
    out.push(line(
        t,
        "contact-book".into(),
        text::address_book(),
        book,
        String::new(),
        Vec::new(),
    ));
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_1))
        .children(out)
        .when(view.copied, |el| {
            el.child(
                div()
                    .id("contact-copied")
                    .role(Role::Status)
                    .aria_label(text::copied())
                    .pt(px(space::SP_2))
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(text::copied())
                    .test_support(),
            )
        })
        .into_any_element()
}
