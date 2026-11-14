//! The gallery's settings pane: cards, fields, notices, busy marks and the
//! Sent notice.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};

use super::Gallery;
use super::sections::{body, heading, row, title};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::widgets::button::{Button, Height, Variant};
use crate::widgets::field::{FieldLook, TextField};
use crate::widgets::loader::ProgressRing;
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::pane::Pane;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::{Checkbox, Toggle};

fn card(
    icon: Icon,
    title: &'static str,
    caption: &'static str,
    trailing: impl IntoElement,
    cx: &Context<Gallery>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let c = t.color;
    div()
        .min_h(px(layout::CARD_H))
        .flex()
        .items_center()
        .gap(px(space::SP_4))
        .px(px(space::SP_4))
        .mb(px(space::SP_1))
        .rounded(px(radius::CARD))
        .bg(c.inset)
        .child(
            gpui::svg()
                .path(icon.path())
                .size(px(layout::ICON))
                .text_color(c.ink_2),
        )
        .child(
            div()
                .flex_1()
                .py(px(space::SP_3))
                .child(div().typo(&t.text.body).child(title))
                .child(
                    div()
                        .typo(&t.text.caption)
                        .text_color(c.ink_3)
                        .child(caption),
                ),
        )
        .child(trailing)
        .into_any_element()
}

pub fn settings(g: &mut Gallery, cx: &mut Context<Gallery>) -> impl IntoElement {
    let s = &g.state;
    let toast = g.toast.clone();
    Pane::new("settings")
        .name("Settings")
        .child(title("Settings", cx))
        .child(
            body(cx)
                .child(heading("Cards", cx))
                .child(card(
                    Icon::Language,
                    "Language",
                    "Changes take effect at once",
                    Dropdown::new("lang", "Language", s.lang.clone())
                        .choices([("en", "English"), ("pt-PT", "Português (Portugal)")])
                        .on_inset(true)
                        .on_pick({
                            let view = cx.entity().downgrade();
                            move |v, _, cx| {
                                let v = v.clone();
                                let _ = view.update(cx, |g, cx| {
                                    g.state.lang = v;
                                    cx.notify();
                                });
                            }
                        }),
                    cx,
                ))
                .child(card(
                    Icon::Power,
                    "Start with Windows",
                    "Opens quietly in the notification area",
                    Toggle::new("start", "Start with Windows", s.start)
                        .state_label("On", "Off")
                        .on_change(set!(cx, |s, on| s.start = on)),
                    cx,
                ))
                .child(card(
                    Icon::Alert,
                    "Keep running when the window is closed",
                    "New mail still arrives and notifies you",
                    Toggle::new("keep", "Keep running", s.keep)
                        .state_label("On", "Off")
                        .on_change(set!(cx, |s, on| s.keep = on)),
                    cx,
                ))
                .child(heading("Fields", cx))
                .child(TextField::new(&g.field, "Display name"))
                .child(
                    TextField::new(&g.to, "To")
                        .look(FieldLook::Underline { label: "To".into() })
                        .trailing("Cc Bcc"),
                )
                .child(
                    row().pt(px(space::SP_3)).child(
                        Checkbox::new("read", "Mark as read when opened", s.read)
                            .on_change(set!(cx, |s, on| s.read = on)),
                    ),
                )
                .child(heading("Notices", cx))
                .child(Notice::new(
                    "offline",
                    Kind::Info,
                    "You are offline. Mail will send when you reconnect.",
                ))
                .child(div().h(px(space::SP_2)))
                .child(
                    Notice::new(
                        "error",
                        Kind::Error,
                        "Can't reach mail.example.com. Komorebi will try again in a minute.",
                    )
                    .action(
                        Button::new("retry")
                            .variant(Variant::OnInset)
                            .label("Try again"),
                    ),
                )
                .child(heading("Busy", cx))
                .child(
                    row()
                        .gap(px(space::SP_4))
                        .child(g.loader.clone())
                        .child(ProgressRing::new("ring", "Saving attachment", 0.62))
                        .child(
                            Button::new("sending")
                                .variant(Variant::Secondary)
                                .height(Height::Medium)
                                .label("Sending…"),
                        ),
                )
                .child(div().h(px(space::SP_4)))
                .child(div().flex().justify_center().child(toast)),
        )
}
