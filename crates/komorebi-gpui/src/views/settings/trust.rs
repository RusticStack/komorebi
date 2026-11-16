//! The certificate question (the Slint app's trust dialog): what the
//! server presented, the one saved before when it changed, and how far to
//! trust it (this certificate, or its issuer). Shared by an account's page
//! and the setup flow's manual servers.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, InteractiveElement as _, KeyDownEvent, ParentElement, Role, SharedString,
    StatefulInteractiveElement as _, Styled, Toggled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use komorebi_i18n::tr;

use super::card::{radio_handles, radio_step};
use super::pins::{self, CertReview};
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::ring_style;

type OnTrust = Rc<dyn Fn(usize, &mut App)>;

/// Ask whether to trust `review`'s certificate; `on_trust` hears the trust
/// choice picked, `on_cancel` a Cancel.
pub fn open(
    review: &CertReview,
    before: Option<&str>,
    on_trust: impl Fn(usize, &mut App) + 'static,
    on_cancel: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let prompt = pins::trust_prompt(review, before);
    let items = review.choice_items();
    let picked = Rc::new(Cell::new(0usize));
    let on_trust: OnTrust = Rc::new(on_trust);
    let on_cancel: Rc<dyn Fn(&mut App)> = Rc::new(on_cancel);
    let t = cx.tokens().clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        let c = t.color;
        let choice = picked.get();
        let count = items.len();
        let handles = radio_handles(&"trust-choices".into(), count, choice, window, cx);
        let code = |label: &str, code: &str| {
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_1))
                .child(
                    div()
                        .typo(&t.text.caption)
                        .text_color(c.ink_3)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .w(px(geo::FINGERPRINT_W))
                        .p(px(space::SP_3))
                        .rounded(px(radius::CARD))
                        .bg(c.inset)
                        .font_family("Consolas")
                        .typo(&t.text.caption)
                        .text_color(c.ink)
                        .child(code.to_string()),
                )
        };
        let choices = items.iter().enumerate().map(|(ix, item)| {
            let picked = picked.clone();
            let on = ix == choice;
            div()
                .id(SharedString::from(format!("trust-choice-{ix}")))
                .track_focus(&handles[ix])
                .role(Role::RadioButton)
                .aria_label(item.label.clone())
                .aria_toggled(if on { Toggled::True } else { Toggled::False })
                .aria_position_in_set(ix + 1)
                .aria_size_of_set(count)
                .flex()
                .gap(px(space::SP_3))
                .p(px(space::SP_2))
                .rounded(px(radius::CARD))
                .when(on, |el| el.bg(c.inset))
                .hover(move |s| s.bg(c.wash))
                .focus_visible(ring_style(&t))
                .child(
                    div()
                        .mt(px(space::SP_0_5))
                        .size(px(layout::ICON))
                        .flex_none()
                        .rounded(px(radius::FULL))
                        .border(px(layout::FOCUS_RING))
                        .border_color(if on { c.accent } else { c.ink_2 })
                        .when(on, |el| el.bg(c.accent)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().typo(&t.text.body).child(item.label.clone()))
                        .child(
                            div()
                                .typo(&t.text.caption)
                                .text_color(c.ink_3)
                                .child(item.description.clone()),
                        ),
                )
                .on_click(move |_, window, _| {
                    picked.set(ix);
                    window.refresh();
                })
                .test_support()
        });
        let keys = picked.clone();
        let key_handles = handles.clone();
        let ok = on_trust.clone();
        let cancel = on_cancel.clone();
        let chosen = picked.clone();
        dialog
            .w(px(layout::DIALOG_W + geo::FORM_W / 4.))
            .bg(c.paper)
            .text_color(c.ink)
            .rounded(px(radius::CARD))
            .shadow(Vec::new())
            .close_button(false)
            .p(px(space::SP_6))
            .title(div().typo(&t.text.subtitle).child(prompt.title.clone()))
            .child(
                div()
                    .id("trust-body")
                    .max_h(px(layout::WINDOW_MIN.1 - layout::TITLEBAR_H))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_3))
                    .child(
                        div()
                            .typo(&t.text.body)
                            .text_color(c.ink_2)
                            .child(prompt.text.clone()),
                    )
                    .when(!prompt.detail.is_empty(), |el| {
                        el.child(
                            div()
                                .typo(&t.text.caption)
                                .text_color(c.ink_2)
                                .child(prompt.detail.clone()),
                        )
                    })
                    .child(code(&prompt.code_label, &prompt.code))
                    .when(!prompt.old.is_empty(), |el| {
                        el.child(code(&prompt.old_label, &prompt.old)).child(
                            div()
                                .typo(&t.text.caption)
                                .text_color(c.ink_2)
                                .child(prompt.caution.clone()),
                        )
                    })
                    .child(
                        div()
                            .id("trust-choices")
                            .role(Role::RadioGroup)
                            .aria_label(prompt.title.clone())
                            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                                if e.keystroke.modifiers.modified() {
                                    return;
                                }
                                let at = key_handles
                                    .iter()
                                    .position(|h| h.is_focused(window))
                                    .unwrap_or(choice);
                                let Some(to) = radio_step(e.keystroke.key.as_str(), at, count)
                                else {
                                    return;
                                };
                                cx.stop_propagation();
                                keys.set(to);
                                window.focus(&key_handles[to], cx);
                                window.refresh();
                            })
                            .flex()
                            .flex_col()
                            .gap(px(space::SP_1))
                            .children(choices),
                    )
                    .child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(c.ink_3)
                            .child(prompt.note.clone()),
                    ),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap(px(space::SP_2))
                    .pt(px(space::SP_4))
                    .child(Button::new("trust-cancel").label(tr!("Cancel")).on_click(
                        move |_, window, cx| {
                            cancel(cx);
                            window.close_dialog(cx);
                        },
                    ))
                    .child(
                        Button::new("trust-ok")
                            .variant(Variant::Primary)
                            .label(prompt.ok.clone())
                            .on_click(move |_, window, cx| {
                                ok(chosen.get(), cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}
