//! Tooltip (cookbook_tooltip): an ink pill with on-ink caption text, shown
//! after `TOOLTIP_DELAY_MS` over the target. Ink, because a paper tooltip
//! over a paper pane would need an edge or a shadow, and the brand has
//! neither. The target carries the same text as its AccessKit description;
//! the tooltip itself has role Tooltip so tests find it.

use std::time::Duration;

use gpui::{
    AnyView, App, ElementId, ParentElement, Role, SharedString, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;

use crate::theme::{ActiveTokens as _, TextStyled as _, layout, motion, radius, space};

/// The tooltip surface for `text`.
pub fn view(text: SharedString, window: &mut Window, cx: &mut App) -> AnyView {
    let t = cx.tokens().clone();
    let label = text.clone();
    Tooltip::element(move |_window, _cx| {
        div()
            .id(ElementId::Name(format!("tooltip-{label}").into()))
            .role(Role::Tooltip)
            .aria_label(label.clone())
            .max_w(px(layout::TOOLTIP_MAX_W))
            .child(label.clone())
            .test_support()
    })
    .bg(t.color.ink)
    .text_color(t.color.on_ink)
    .typo(&t.text.caption)
    .border_0()
    .shadow(Vec::new())
    .rounded(px(radius::FULL))
    .px(px(space::SP_3))
    .py(px(space::SP_1))
    .build(window, cx)
}

/// Gives a stateful element the brand tooltip, its delay and the matching
/// AccessKit description.
pub trait WithTooltip: StatefulInteractiveElement + Sized {
    fn brand_tooltip(self, text: impl Into<SharedString>) -> Self {
        let text: SharedString = text.into();
        let shown = text.clone();
        self.aria_description(text)
            .tooltip(move |window, cx| view(shown.clone(), window, cx))
            .tooltip_show_delay(Duration::from_millis(motion::TOOLTIP_DELAY_MS))
    }

    /// The brand tooltip for words the accessible name already holds (a
    /// label cut short at the pane's width): no description, so a screen
    /// reader does not say them twice.
    fn name_tooltip(self, text: impl Into<SharedString>) -> Self {
        let shown: SharedString = text.into();
        self.tooltip(move |window, cx| view(shown.clone(), window, cx))
            .tooltip_show_delay(Duration::from_millis(motion::TOOLTIP_DELAY_MS))
    }
}

impl<T: StatefulInteractiveElement> WithTooltip for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{Context, IntoElement, Render, TestAppContext};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    struct Demo;
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(
                    div()
                        .id("target")
                        .size(px(layout::ICON_BTN))
                        .brand_tooltip("Archive (Backspace)")
                        .test_support(),
                )
                .child(div().id("away").size(px(layout::ICON_BTN)).test_support())
        }
    }

    #[gpui_kit::test]
    fn the_tooltip_shows_after_the_delay(cx: &mut TestAppContext) {
        let (window, _) = test_support::open(cx, |_, _| Demo);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.hover("target", cx);
            window.render_frame(cx);
            assert!(window.try_find_by_label("Archive (Backspace)").is_none());
        })
        .expect("hover");
        cx.executor()
            .advance_clock(Duration::from_millis(motion::TOOLTIP_DELAY_MS * 2));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find_by_label("Archive (Backspace)").role(),
                Some(Role::Tooltip)
            );
        })
        .expect("shown");
    }
}
