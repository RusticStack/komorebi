//! Links on the page: the confirm prompt before a web link opens in the
//! browser (the host apart, the whole address, a line per warning; Open
//! has Enter only when there is nothing to warn about, as in the Slint
//! app), the strip that shows the hovered or focused link's target over
//! the reader's bottom corner, and the warnings' sentences.

use std::rc::Rc;

use gpui::{
    App, IntoElement, ParentElement as _, Role, SharedString, Styled as _, Window, div, prelude::*,
    px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use komorebi::mail::render::links::Warning;
use komorebi::reader::session::Prompt;
use komorebi_i18n::tr;

use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, reader, space};
use crate::widgets::button::{Button, Variant};

/// The prompt's line for a warning about a link shown as `text` that goes
/// to `host` (the Slint app's `warning_text`).
pub fn warning_text(w: Warning, host: &str, text: &str) -> String {
    match w {
        Warning::TextMismatch => {
            let shown: String = text.chars().take(80).collect();
            tr!(
                "The link text says \u{201c}{0}\u{201d}, but it opens {1}.",
                shown,
                host
            )
        }
        Warning::Punycode => tr!(
            "{0} is an international name, which can imitate the letters of another site.",
            host
        ),
        Warning::UserInfo => tr!(
            "The address puts a user name before the real site, which is {0}.",
            host
        ),
        Warning::RawIp => tr!("{0} is a bare IP address, not a site name.", host),
    }
}

/// Opens the confirm prompt for `prompt`; `answer(true)` opens the link,
/// `answer(false)` drops it (Cancel, Escape, or the dialog closing).
pub fn confirm(
    prompt: &Prompt,
    answer: impl Fn(bool, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let answer = Rc::new(answer);
    let t = cx.tokens().clone();
    let host: SharedString = prompt.host.clone().into();
    let address: SharedString = format!("{}{}{}", prompt.before, prompt.host, prompt.after).into();
    let warnings: Vec<SharedString> = prompt
        .warnings
        .iter()
        .map(|&w| warning_text(w, &prompt.host, &prompt.text).into())
        .collect();
    let warned = !warnings.is_empty();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let c = t.color;
        let (open, cancel, enter, closed) = (
            answer.clone(),
            answer.clone(),
            answer.clone(),
            answer.clone(),
        );
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(space::SP_3))
            .child(
                div()
                    .id("link-host")
                    .role(Role::Label)
                    .aria_label(host.clone())
                    .typo(&t.text.body_strong)
                    .text_color(c.ink)
                    .child(host.clone())
                    .test_support(),
            )
            .child(
                div()
                    .typo(&t.text.caption)
                    .text_color(c.ink_2)
                    .child(address.clone()),
            );
        for (i, w) in warnings.iter().enumerate() {
            body = body.child(
                div()
                    .id(("link-warning", i))
                    .role(Role::Alert)
                    .aria_label(w.clone())
                    .typo(&t.text.body)
                    .text_color(c.ink)
                    .child(w.clone())
                    .test_support(),
            );
        }
        dialog
            .w(px(layout::DIALOG_W))
            .bg(c.paper)
            .text_color(c.ink)
            .rounded(px(radius::CARD))
            .shadow(Vec::new())
            .close_button(false)
            .p(px(space::SP_6))
            .title(
                div()
                    .typo(&t.text.subtitle)
                    .child(tr!("Open this link in your browser?")),
            )
            .child(body)
            .footer(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap(px(space::SP_2))
                    .pt(px(space::SP_4))
                    .child(Button::new("link-cancel").label(tr!("Cancel")).on_click(
                        move |_, window, cx| {
                            cancel(false, window, cx);
                            window.close_dialog(cx);
                        },
                    ))
                    .child(
                        Button::new("link-open")
                            .variant(Variant::Primary)
                            .label(tr!("Open"))
                            .on_click(move |_, window, cx| {
                                open(true, window, cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
            // Enter opens only a link with nothing to warn about.
            .on_ok(move |_, window, cx| {
                if warned {
                    return false;
                }
                enter(true, window, cx);
                true
            })
            .on_close(move |_, window, cx| closed(false, window, cx))
    });
}

/// The hovered (else focused) link's target, over the reader's bottom
/// left corner, as browsers show it: an ink pill, as tooltips are.
pub fn target_strip(href: SharedString, cx: &App) -> impl IntoElement {
    let t = cx.tokens().clone();
    div()
        .absolute()
        .left(px(reader::TARGET_INSET))
        .bottom(px(reader::TARGET_INSET))
        .max_w(px(reader::TARGET_MAX_W))
        .px(px(space::SP_3))
        .py(px(space::SP_1))
        .rounded(px(radius::FULL))
        .bg(t.color.ink)
        .text_color(t.color.on_ink)
        .typo(&t.text.caption)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(href)
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    #[test]
    fn warnings_read_as_in_the_slint_app() {
        with_lang(Lang::En, || {
            assert_eq!(
                warning_text(
                    Warning::TextMismatch,
                    "evil.example",
                    "https://bank.example"
                ),
                "The link text says \u{201c}https://bank.example\u{201d}, but it opens evil.example."
            );
            assert_eq!(
                warning_text(Warning::RawIp, "10.0.0.1", ""),
                "10.0.0.1 is a bare IP address, not a site name."
            );
        });
        with_lang(Lang::PtPt, || {
            for w in [
                Warning::TextMismatch,
                Warning::Punycode,
                Warning::UserInfo,
                Warning::RawIp,
            ] {
                let pt = warning_text(w, "evil.example", "x");
                assert!(pt.contains("evil.example"), "{w:?}: {pt}");
                assert_ne!(
                    pt,
                    with_lang(Lang::En, || warning_text(w, "evil.example", "x"))
                );
            }
        });
    }
}
