//! The gallery's three panes: controls, lists, settings and notices.

use gpui::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, px};

use super::Gallery;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, space};
use crate::widgets::button::{Button, Height, IconButton, Variant};
use crate::widgets::marks::{Badge, Kbd};
use crate::widgets::pane::{ListRow, Pane};
use crate::widgets::pill::{Chip, FilterPills};
use crate::widgets::spot::SenderSpot;
use crate::widgets::tabs::{Segmented, TabSpec, TabStrip};

/// What the gallery's controls change.
pub struct State {
    pub filter: usize,
    pub tab: usize,
    pub theme: usize,
    pub start: bool,
    pub keep: bool,
    pub read: bool,
    pub lang: SharedString,
    pub bold: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            filter: 0,
            tab: 0,
            theme: 0,
            start: false,
            keep: true,
            read: true,
            lang: "en".into(),
            bold: true,
        }
    }
}

/// Runs `f` on the gallery's state and redraws.
macro_rules! set {
    ($cx:expr, |$s:ident, $v:ident| $body:expr) => {{
        let view = $cx.entity().downgrade();
        move |$v, _: &mut gpui::Window, cx: &mut gpui::App| {
            let _ = view.update(cx, |g: &mut Gallery, cx| {
                let $s = &mut g.state;
                $body;
                cx.notify();
            });
        }
    }};
}

pub(super) fn heading(text: &'static str, cx: &Context<Gallery>) -> AnyElement {
    let t = cx.tokens();
    div()
        .pt(px(space::SP_5))
        .pb(px(space::SP_2))
        .typo(&t.text.body_strong)
        .child(text)
        .into_any_element()
}

pub(super) fn row() -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SP_2))
}

pub(super) fn body(cx: &Context<Gallery>) -> gpui::Div {
    let t = cx.tokens();
    div()
        .flex()
        .flex_col()
        .px(px(space::SP_5))
        .pb(px(space::SP_5))
        .typo(&t.text.body)
}

pub(super) fn title(text: &'static str, cx: &Context<Gallery>) -> AnyElement {
    let t = cx.tokens();
    div()
        .px(px(space::SP_5))
        .pt(px(space::SP_4))
        .typo(&t.text.subtitle)
        .child(text)
        .into_any_element()
}

pub fn controls(g: &mut Gallery, cx: &mut Context<Gallery>) -> impl IntoElement {
    let s = &g.state;
    Pane::new("controls")
        .name("Controls")
        .child(title("Controls", cx))
        .child(
            body(cx)
                .child(heading("Buttons", cx))
                .child(
                    Button::new("new-mail")
                        .primary()
                        .height(Height::Large)
                        .icon(Icon::Compose)
                        .label("New mail")
                        .w(px(layout::NAV_W - space::SP_4)),
                )
                .child(
                    row()
                        .pt(px(space::SP_2))
                        .child(
                            Button::new("reply")
                                .height(Height::Medium)
                                .icon(Icon::Reply)
                                .label("Reply"),
                        )
                        .child(
                            Button::new("send")
                                .primary()
                                .height(Height::Medium)
                                .icon(Icon::Send)
                                .label("Send"),
                        )
                        .child(
                            Button::new("subtle")
                                .variant(Variant::Subtle)
                                .label("Subtle"),
                        )
                        .child(
                            Button::new("delete")
                                .variant(Variant::Destructive)
                                .label("Delete"),
                        )
                        .child(Button::new("off").label("Disabled").disabled(true)),
                )
                .child(heading("Icon buttons", cx))
                .child(
                    row()
                        .child(IconButton::new("i-reply", Icon::Forward, "Forward"))
                        .child(IconButton::new("i-archive", Icon::Archive, "Archive"))
                        .child(IconButton::new("i-delete", Icon::Delete, "Delete"))
                        .child(IconButton::new("i-read", Icon::Mail, "Mark as unread"))
                        .child(
                            IconButton::new("i-bold", Icon::TextSize, "Bold")
                                .small()
                                .toggled(s.bold)
                                .on_click({
                                    let view = cx.entity().downgrade();
                                    move |_, _, cx| {
                                        let _ = view.update(cx, |g, cx| {
                                            g.state.bold = !g.state.bold;
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                        .child(IconButton::new("i-more", Icon::More, "More actions")),
                )
                .child(heading("Filter pills", cx))
                .child(
                    FilterPills::new("filter", "Filter")
                        .items(["All", "Unread", "Important"])
                        .selected(s.filter)
                        .on_select(set!(cx, |s, ix| s.filter = ix)),
                )
                .child(heading("Tabs", cx))
                .child(
                    TabStrip::new("tabs", "Open messages", "Close tab")
                        .tabs([
                            TabSpec::new("Inbox"),
                            TabSpec::new("Photos from Gerês").closable(),
                            TabSpec::new("Design review").closable(),
                        ])
                        .selected(s.tab)
                        .on_select(set!(cx, |s, ix| s.tab = ix)),
                )
                .child(heading("Segmented", cx))
                .child(
                    Segmented::new("theme", "Theme")
                        .items(["System", "Light", "Dark"])
                        .selected(s.theme)
                        .on_select(set!(cx, |s, ix| s.theme = ix)),
                )
                .child(heading("Marks", cx))
                .child(
                    row()
                        .gap(px(space::SP_6))
                        .child(Kbd::new("ctrl-e"))
                        .child(Kbd::new("ctrl-shift-7"))
                        .child(Badge::new("badge", 2, "2 unread")),
                ),
        )
}

/// A sample message row.
struct Msg {
    from: &'static str,
    subject: &'static str,
    preview: &'static str,
    unread: bool,
    selected: bool,
    /// Multi-select: the spot is a check.
    check: Option<bool>,
}

fn message(ix: usize, m: Msg, cx: &Context<Gallery>) -> AnyElement {
    let Msg {
        from,
        subject,
        preview,
        unread,
        selected,
        check,
    } = m;
    let t = cx.tokens().clone();
    let c = t.color;
    let spot = match check {
        Some(on) => SenderSpot::new(from).check(format!("check-{ix}"), on),
        None => SenderSpot::new(from).unread(unread).on_wash(selected),
    };
    ListRow::new(
        ("msg", ix),
        format!("{}{from}, {subject}", if unread { "Unread, " } else { "" }),
    )
    .selected(selected || check == Some(true))
    .height(layout::ROW_H)
    .child(spot)
    .child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .typo(if unread {
                        &t.text.body_strong
                    } else {
                        &t.text.body
                    })
                    .child(from)
                    .child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child("09:04"),
                    ),
            )
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(if unread { c.ink } else { c.ink_2 })
                    .child(subject),
            )
            .child(
                div()
                    .typo(&t.text.preview)
                    .text_color(c.ink_3)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(preview),
            ),
    )
    .into_any_element()
}

pub fn lists(g: &mut Gallery, cx: &mut Context<Gallery>) -> impl IntoElement {
    let _ = g;
    Pane::new("lists")
        .name("Lists")
        .child(title("Lists", cx))
        .child(
            div()
                .flex()
                .flex_col()
                .px(px(space::SP_2))
                .pt(px(space::SP_3))
                .child(message(
                    0,
                    Msg {
                        from: "Marta Silva",
                        subject: "Photos from Gerês",
                        preview: "Here they are, finally. The one at the lake is my favourite.",
                        unread: true,
                        selected: true,
                        check: None,
                    },
                    cx,
                ))
                .child(message(
                    1,
                    Msg {
                        from: "Rui Mendes",
                        subject: "Design review on Friday",
                        preview: "Can we go through the new screens together before the meeting?",
                        unread: true,
                        selected: false,
                        check: None,
                    },
                    cx,
                ))
                .child(message(
                    2,
                    Msg {
                        from: "Field Notes",
                        subject: "Slow things, done well",
                        preview: "Design is not only how it looks. It is also how long it lasts.",
                        unread: false,
                        selected: false,
                        check: None,
                    },
                    cx,
                ))
                .child(heading_pad("Multi-select", cx))
                .child(message(
                    3,
                    Msg {
                        from: "Ana Rocha",
                        subject: "Weekend at the coast",
                        preview: "We booked the house in Comporta. Two rooms, a garden.",
                        unread: false,
                        selected: false,
                        check: Some(true),
                    },
                    cx,
                ))
                .child(message(
                    4,
                    Msg {
                        from: "Lumen Studio",
                        subject: "Your order has shipped",
                        preview: "Four new glazes and a linen lamp are on their way.",
                        unread: true,
                        selected: false,
                        check: Some(false),
                    },
                    cx,
                )),
        )
        .child(
            body(cx)
                .child(heading("Chips and avatars", cx))
                .child(
                    row()
                        .child(Chip::new("chip-marta", "Marta Silva", "Remove Marta Silva"))
                        .child(Chip::new("chip-rui", "Rui Mendes", "Remove Rui Mendes")),
                )
                .child(
                    row()
                        .pt(px(space::SP_3))
                        .child(SenderSpot::new("Inês Duarte").size(layout::AVATAR_SM))
                        .child(SenderSpot::new("Inês Duarte").size(layout::AVATAR_MD))
                        .child(
                            SenderSpot::new("Inês Duarte")
                                .unread(true)
                                .size(layout::AVATAR_LG),
                        ),
                ),
        )
}

fn heading_pad(text: &'static str, cx: &Context<Gallery>) -> AnyElement {
    div()
        .px(px(space::SP_3))
        .child(heading(text, cx))
        .into_any_element()
}
