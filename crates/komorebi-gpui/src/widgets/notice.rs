//! The notice (InfoBar; mockup `.note`): an inset card with a glyph, a
//! sentence and optional pills. Calm: ink and greys only, the kind is told
//! by the glyph and the words (BRAND.md "Voice"), never by a hue. Offline,
//! a server that cannot be reached, a password refused, remote images.

use gpui::{
    AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled,
    Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};

/// What the notice is about; only the glyph and the role change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Info,
    Warning,
    /// Something failed; screen readers hear it at once.
    Error,
}

#[derive(IntoElement)]
pub struct Notice {
    id: ElementId,
    kind: Kind,
    text: SharedString,
    actions: Vec<AnyElement>,
}

impl Notice {
    pub fn new(id: impl Into<ElementId>, kind: Kind, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            kind,
            text: text.into(),
            actions: Vec::new(),
        }
    }

    /// A pill under the sentence (use `Variant::OnInset` buttons).
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for Notice {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let (icon, role) = match self.kind {
            Kind::Info => (Icon::Info, Role::Status),
            Kind::Warning => (Icon::Warning, Role::Status),
            Kind::Error => (Icon::ErrorCircle, Role::Alert),
        };
        let has_actions = !self.actions.is_empty();
        div()
            .id(self.id)
            .role(role)
            .aria_label(self.text.clone())
            // Its words change while the focus is elsewhere: Narrator says
            // them (an error at once, the rest when it is free).
            .a11y_synthetic_children(move |b| {
                b.parent_node().set_live(if role == Role::Alert {
                    gpui::accesskit::Live::Assertive
                } else {
                    gpui::accesskit::Live::Polite
                })
            })
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(space::SP_3))
            .px(px(space::SP_4))
            .py(px(space::SP_3))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .text_color(c.ink)
            .typo(&t.text.preview)
            .child(
                svg()
                    .path(icon.path())
                    .size(px(layout::ICON))
                    .flex_none()
                    .mt(px(layout::HAIRLINE))
                    .text_color(c.ink_2),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_3))
                    .child(self.text)
                    .when(has_actions, |el| {
                        el.child(div().flex().gap(px(space::SP_2)).children(self.actions))
                    }),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::button::{Button, Variant};
    use crate::widgets::test_support;
    use gpui::{Context, Render, TestAppContext};
    use gpui_kit::test::TestQueryExt as _;

    struct Demo;
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .child(Notice::new(
                    "offline",
                    Kind::Info,
                    "You are offline. Mail will send when you reconnect.",
                ))
                .child(
                    Notice::new(
                        "auth",
                        Kind::Error,
                        "The server did not accept the password. Check it and try again.",
                    )
                    .action(
                        Button::new("auth-fix")
                            .variant(Variant::OnInset)
                            .label("Sign in"),
                    ),
                )
        }
    }

    #[gpui_kit::test]
    fn notices_are_status_or_alert(cx: &mut TestAppContext) {
        let (window, _) = test_support::open(cx, |_, _| Demo);
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window
                    .find_by_label("You are offline. Mail will send when you reconnect.")
                    .role(),
                Some(Role::Status)
            );
            assert_eq!(
                window
                    .find_by_label(
                        "The server did not accept the password. Check it and try again."
                    )
                    .role(),
                Some(Role::Alert)
            );
            assert_eq!(window.find_by_label("Sign in").role(), Some(Role::Button));
        })
        .expect("draw");
    }
}
