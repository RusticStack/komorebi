//! The Sent notice (BRAND.md "Sending"; mockup `.toast`): an ink pill with
//! the seal and "Sent to Marta Silva", at the bottom centre of the reader,
//! for 2 s. The seal is neutral like the rest of the app (docs/BRANDING.md):
//! drawn in the on-ink colour, as the text is.
//!
//! Cost (IMPLEMENTATION.md section 5): fade in over 250 ms, hold with no
//! frames (a timer, not an animation), fade out over 250 ms: about 30
//! frames in all. Under reduce motion it appears and goes without fades.
//! The host view places [`Toast`] (an entity) over the reader.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, Context, ElementId, IntoElement, ParentElement, Render, Role,
    SharedString, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, motion, radius, space};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Entering,
    Holding,
    Leaving,
}

/// The Sent notice's state.
pub struct Toast {
    shown: Option<(SharedString, Phase)>,
    /// Each `show` gets a new one: a newer notice cancels an older timer.
    epoch: u64,
}

impl Default for Toast {
    fn default() -> Self {
        Self::new()
    }
}

impl Toast {
    pub fn new() -> Self {
        Self {
            shown: None,
            epoch: 0,
        }
    }

    pub fn message(&self) -> Option<&SharedString> {
        self.shown.as_ref().map(|(m, _)| m)
    }

    /// Shows `message`, replacing any notice on screen.
    pub fn show(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.shown = Some((message.into(), Phase::Entering));
        cx.notify();
        let t = cx.tokens();
        let fade = t.duration(motion::PANE_MS);
        let hold = Duration::from_millis(motion::SENT_HOLD_MS);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(fade + hold).await; // timer-exempt: the Sent notice's hold (brand motion)
            let go_on = this.update(cx, |toast, cx| toast.advance(epoch, Phase::Leaving, cx));
            if !matches!(go_on, Ok(true)) {
                return;
            }
            cx.background_executor().timer(fade).await; // timer-exempt: the notice's fade out
            let _ = this.update(cx, |toast, cx| {
                if toast.epoch == epoch {
                    toast.shown = None;
                    cx.notify();
                }
            });
        })
        .detach();
        // Past the fade in, the notice holds with no animation.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(fade).await; // timer-exempt: the notice's fade in
            let _ = this.update(cx, |toast, cx| toast.advance(epoch, Phase::Holding, cx));
        })
        .detach();
    }

    fn advance(&mut self, epoch: u64, phase: Phase, cx: &mut Context<Self>) -> bool {
        if self.epoch != epoch {
            return false;
        }
        if let Some((_, p)) = &mut self.shown {
            *p = phase;
            cx.notify();
        }
        true
    }
}

impl Render for Toast {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((message, phase)) = self.shown.clone() else {
            return div().into_any_element();
        };
        let t = cx.tokens().clone();
        let c = t.color;
        let pill = div()
            .id(ElementId::Name(format!("sent-{}", self.epoch).into()))
            .role(Role::Status)
            .aria_label(message.clone())
            // Said by Narrator while the focus is elsewhere.
            .a11y_synthetic_children(|b| b.parent_node().set_live(gpui::accesskit::Live::Polite))
            .h(px(scaled::control(&t, layout::TOAST_H)))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::SP_3))
            .pl(px(space::SP_3))
            .pr(px(space::SP_5))
            .rounded(px(radius::FULL))
            .bg(c.ink)
            .text_color(c.on_ink)
            .typo(&t.text.body)
            .whitespace_nowrap()
            .child(
                svg()
                    .path(Icon::Seal.path())
                    .size(px(layout::TOAST_SEAL))
                    .flex_none()
                    .text_color(c.on_ink),
            )
            .child(message)
            .test_support();
        let fade = t.duration(motion::PANE_MS);
        let key = ElementId::Name(format!("sent-fade-{}-{phase:?}", self.epoch).into());
        match phase {
            Phase::Holding => pill.into_any_element(),
            _ if fade.is_zero() => pill.into_any_element(),
            Phase::Entering => pill
                .with_animation(
                    key,
                    Animation::new(fade).with_easing(motion::ease_enter),
                    |el, delta| el.opacity(delta),
                )
                .into_any_element(),
            Phase::Leaving => pill
                .with_animation(
                    key,
                    Animation::new(fade).with_easing(motion::fade_out),
                    |el, delta| el.opacity(delta),
                )
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::TestAppContext;

    #[gpui::test]
    fn the_notice_holds_two_seconds_then_goes(cx: &mut TestAppContext) {
        test_support::init(cx, false);
        let (toast, cx) = cx.add_window_view(|_, _| Toast::new());
        toast.update(cx, |t, cx| t.show("Sent to Marta Silva", cx));
        cx.executor()
            .advance_clock(Duration::from_millis(motion::SENT_HOLD_MS - 100));
        cx.run_until_parked();
        assert_eq!(
            toast.read_with(cx, |t, _| t.message().cloned()),
            Some("Sent to Marta Silva".into())
        );
        cx.executor()
            .advance_clock(Duration::from_millis(motion::PANE_MS * 2 + 200));
        cx.run_until_parked();
        assert_eq!(toast.read_with(cx, |t, _| t.message().cloned()), None);
    }

    #[gpui::test]
    fn a_newer_notice_replaces_the_older_timer(cx: &mut TestAppContext) {
        test_support::init(cx, false);
        let (toast, cx) = cx.add_window_view(|_, _| Toast::new());
        toast.update(cx, |t, cx| t.show("Sent to Marta Silva", cx));
        cx.executor().advance_clock(Duration::from_millis(1500));
        cx.run_until_parked();
        toast.update(cx, |t, cx| t.show("Sent to Rui Mendes", cx));
        cx.executor().advance_clock(Duration::from_millis(1500));
        cx.run_until_parked();
        assert_eq!(
            toast.read_with(cx, |t, _| t.message().cloned()),
            Some("Sent to Rui Mendes".into()),
            "the first timer did not hide the second notice"
        );
    }
}
