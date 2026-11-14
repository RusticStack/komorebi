//! The reader's bars: the notices above the body (the sender notice, the
//! encrypted state, a server draft) and the remote-image bars over the page
//! (blocked, could not be loaded, "will always load" with Undo), as the
//! Slint app shows them, in the brand's notice look (`widgets::notice`: an
//! inset card, a glyph and words, pills on paper), plus a close button
//! where the Slint bar is closable.

use std::rc::Rc;

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Window,
    div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use komorebi_i18n::{tr, trn};

use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::widgets::button::{Button, IconButton, Variant};

type Act = Rc<dyn Fn(&mut Window, &mut App)>;

/// What a bar is about: the glyph and the role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    /// Look twice (the sender notice, images that could not be loaded).
    Caution,
}

/// A notice bar with a title (optional), a sentence, pills and a close
/// button.
#[derive(IntoElement)]
pub struct Bar {
    id: ElementId,
    kind: Kind,
    title: Option<SharedString>,
    text: SharedString,
    actions: Vec<(SharedString, SharedString, Act)>,
    close: Option<Act>,
}

impl Bar {
    pub fn new(id: impl Into<ElementId>, kind: Kind, text: impl Into<SharedString>) -> Self {
        Bar {
            id: id.into(),
            kind,
            title: None,
            text: text.into(),
            actions: Vec::new(),
            close: None,
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A pill under the sentence; `id` is its element id.
    pub fn action(
        mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.actions.push((id.into(), label.into(), Rc::new(f)));
        self
    }

    /// The close button (for this message only).
    pub fn closable(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.close = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Bar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let (icon, role) = match self.kind {
            Kind::Info => (Icon::Info, Role::Status),
            Kind::Caution => (Icon::Warning, Role::Status),
        };
        let name: SharedString = match &self.title {
            Some(title) => format!("{title} {}", self.text).into(),
            None => self.text.clone(),
        };
        let actions = self.actions.into_iter().map(|(id, label, f)| {
            Button::new(ElementId::Name(id))
                .variant(Variant::OnInset)
                .label(label)
                .on_click(move |_, window, cx| f(window, cx))
        });
        let has_actions = actions.len() > 0;
        div()
            .id(self.id.clone())
            .role(role)
            .aria_label(name)
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
                    .gap(px(space::SP_1))
                    .when_some(self.title, |el, title| {
                        el.child(div().typo(&t.text.body_strong).child(title))
                    })
                    .child(self.text)
                    .when(has_actions, |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(px(space::SP_2))
                                .mt(px(space::SP_2))
                                .children(actions),
                        )
                    }),
            )
            .when_some(self.close, |el, f| {
                el.child(
                    IconButton::new(
                        ElementId::Name(format!("{}-close", self.id).into()),
                        Icon::Dismiss,
                        tr!("Close"),
                    )
                    .small()
                    .on_click(move |_, window, cx| f(window, cx)),
                )
            })
            .test_support()
    }
}

/// The remote-image bar's sentence: `blocked` images left out (from
/// `sender`, when known), or after consent, `failed` that could not be
/// loaded.
pub fn images_text(blocked: u32, failed: u32, sender: &str) -> String {
    if blocked == 0 {
        trn!(
            "{n} image couldn't be loaded.",
            "{n} images couldn't be loaded.",
            failed as usize
        )
    } else if sender.is_empty() {
        trn!(
            "{n} remote image blocked. If you load them, the sender can see when you open this message and your IP address.",
            "{n} remote images blocked. If you load them, the sender can see when you open this message and your IP address.",
            blocked as usize
        )
    } else {
        trn!(
            "{n} remote image from {0} blocked. If you load them, the sender can see when you open this message and your IP address.",
            "{n} remote images from {0} blocked. If you load them, the sender can see when you open this message and your IP address.",
            blocked as usize,
            sender
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    #[test]
    fn the_image_bar_counts_and_names_the_sender() {
        with_lang(Lang::En, || {
            assert_eq!(images_text(0, 1, ""), "1 image couldn't be loaded.");
            assert_eq!(images_text(0, 2, "a@x"), "2 images couldn't be loaded.");
            assert!(
                images_text(3, 0, "ana@example.org")
                    .starts_with("3 remote images from ana@example.org blocked.")
            );
            assert!(images_text(1, 0, "").starts_with("1 remote image blocked."));
        });
        with_lang(Lang::PtPt, || {
            let pt = images_text(3, 0, "ana@example.org");
            assert!(pt.contains("ana@example.org") && pt.contains('3'), "{pt}");
            assert_ne!(
                pt,
                with_lang(Lang::En, || images_text(3, 0, "ana@example.org"))
            );
        });
    }
}
