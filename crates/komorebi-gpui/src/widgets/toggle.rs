//! The toggle switch and the round checkbox (BRAND.md: no square
//! checkbox; checks are round). Mockup `.tog`: a 40 x 20 pill; off is a
//! 1 px ink-2 ring with an ink-2 knob at the left, on is ink with an on-ink
//! knob at the right. The checkbox is the multi-select check with a label.
//! Both are focusable, toggle on click, Enter and Space, and report their
//! state to screen readers.

use std::rc::Rc;

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Toggled,
    Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use super::ring_style;
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, motion, radius, space};

type OnChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

fn toggled(on: bool) -> Toggled {
    if on { Toggled::True } else { Toggled::False }
}

/// A switch. `on_change` gets the new state.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    name: SharedString,
    on: bool,
    /// The "On" / "Off" word beside the track (Settings shows it).
    state_label: Option<(SharedString, SharedString)>,
    disabled: bool,
    on_change: Option<OnChange>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>, on: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            on,
            state_label: None,
            disabled: false,
            on_change: None,
        }
    }

    /// The words for on and off, shown left of the track.
    pub fn state_label(
        mut self,
        on: impl Into<SharedString>,
        off: impl Into<SharedString>,
    ) -> Self {
        self.state_label = Some((on.into(), off.into()));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let on = self.on;
        let travel =
            layout::TOGGLE_W - layout::TOGGLE_KNOB - layout::TOGGLE_INSET - layout::TOGGLE_INSET;
        let track = div()
            .relative()
            .w(px(layout::TOGGLE_W))
            .h(px(layout::TOGGLE_H))
            .flex_none()
            .rounded(px(radius::FULL))
            .when(on, |el| el.bg(c.accent))
            .when(!on, |el| {
                el.border(px(layout::HAIRLINE)).border_color(c.ink_2)
            })
            .child(
                div()
                    .absolute()
                    .top(px(
                        layout::TOGGLE_INSET - if on { 0. } else { layout::HAIRLINE }
                    ))
                    .left(px(
                        layout::TOGGLE_INSET + if on { travel } else { -layout::HAIRLINE }
                    ))
                    .size(px(layout::TOGGLE_KNOB))
                    .rounded(px(radius::FULL))
                    .bg(if on { c.on_accent } else { c.ink_2 }),
            );
        let mut el = div()
            .id(self.id)
            .role(Role::Switch)
            .aria_label(self.name)
            .aria_toggled(toggled(on))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::SP_3))
            .rounded(px(radius::FULL))
            .text_color(c.ink_2)
            .typo(&t.text.preview)
            .when_some(self.state_label, |el, (yes, no)| {
                el.child(if on { yes } else { no })
            })
            .child(track);
        if self.disabled {
            el = el.opacity(motion::DISABLED);
        } else {
            el = el.tab_index(0).focus_visible(ring_style(&t));
            if let Some(f) = self.on_change {
                el = el.on_click(move |_, w, cx| f(!on, w, cx));
            }
        }
        el.test_support()
    }
}

/// A round checkbox with a label.
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    disabled: bool,
    on_change: Option<OnChange>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked,
            disabled: false,
            on_change: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let checked = self.checked;
        let mark = div()
            .size(px(layout::CHECK))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(radius::FULL))
            .when(checked, |el| {
                el.bg(c.accent).child(
                    svg()
                        .path(Icon::Checkmark.path())
                        .size(px(layout::ICON))
                        .text_color(c.on_accent),
                )
            })
            .when(!checked, |el| {
                el.border(px(layout::HAIRLINE)).border_color(c.ink_3)
            });
        let mut el = div()
            .id(self.id)
            .role(Role::CheckBox)
            .aria_label(self.label.clone())
            .aria_toggled(toggled(checked))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::SP_3))
            .pr(px(space::SP_2))
            .rounded(px(radius::FULL))
            .text_color(c.ink)
            .typo(&t.text.body)
            .child(mark)
            .child(self.label);
        if self.disabled {
            el = el.opacity(motion::DISABLED);
        } else {
            el = el.tab_index(0).focus_visible(ring_style(&t));
            if let Some(f) = self.on_change {
                el = el.on_click(move |_, w, cx| f(!checked, w, cx));
            }
        }
        el.test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{Context, Render, TestAppContext};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    struct Demo {
        start: bool,
        read: bool,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let a = cx.entity().downgrade();
            let b = a.clone();
            div()
                .flex()
                .gap(px(space::SP_4))
                .child(
                    Toggle::new("start", "Start with Windows", self.start)
                        .state_label("On", "Off")
                        .on_change(move |on, _, cx| {
                            let _ = a.update(cx, |v, cx| {
                                v.start = on;
                                cx.notify();
                            });
                        }),
                )
                .child(Checkbox::new("read", "Mark as read", self.read).on_change(
                    move |on, _, cx| {
                        let _ = b.update(cx, |v, cx| {
                            v.read = on;
                            cx.notify();
                        });
                    },
                ))
        }
    }

    #[gpui_kit::test]
    fn toggles_and_checks_flip_and_report_state(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo {
            start: false,
            read: true,
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window.find_by_label("Start with Windows").role(),
                Some(Role::Switch)
            );
            assert_eq!(
                window.find_by_label("Mark as read").role(),
                Some(Role::CheckBox)
            );
            window.click("start", cx);
            window.click("read", cx);
        })
        .expect("click");
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert!(v.start && !v.read));
    }
}
