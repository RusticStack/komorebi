//! Filter pills and chips.
//!
//! [`FilterPills`]: the list's All, Unread, Important (mockup `.pv`): words
//! 24 px apart, the current one ink at 600 with a 4 px ink spot under it,
//! the others ink-2. A tab list for screen readers; Left and Right are the
//! list's keys (phase 2), click and Enter pick one here.
//!
//! [`Chip`]: a recipient in the composer (mockup `.chip`): a 28 px wash
//! pill with a 22 px avatar on paper, the name, and a dismiss glyph.

use std::rc::Rc;

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Window,
    div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use super::ring_style;
use super::spot::SenderSpot;
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};

type OnSelect = Rc<dyn Fn(usize, &mut Window, &mut App)>;
type OnRemove = Rc<dyn Fn(&mut Window, &mut App)>;

/// A row of filter pills; one is current.
#[derive(IntoElement)]
pub struct FilterPills {
    id: SharedString,
    name: SharedString,
    items: Vec<SharedString>,
    selected: usize,
    on_select: Option<OnSelect>,
}

impl FilterPills {
    /// `name` is the group's accessible name ("Filter").
    pub fn new(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            items: Vec::new(),
            selected: 0,
            on_select: None,
        }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.items = items.into_iter().map(Into::into).collect();
        self
    }

    pub fn selected(mut self, ix: usize) -> Self {
        self.selected = ix;
        self
    }

    pub fn on_select(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for FilterPills {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let count = self.items.len();
        let pills = self.items.into_iter().enumerate().map(|(ix, label)| {
            let on = ix == self.selected;
            let pick = self.on_select.clone();
            div()
                .id(ElementId::Name(format!("{}-{ix}", self.id).into()))
                .role(Role::Tab)
                .aria_label(label.clone())
                .aria_selected(on)
                .aria_position_in_set(ix + 1)
                .aria_size_of_set(count)
                .tab_index(0)
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .pb(px(space::SP_2))
                .rounded(px(radius::FULL))
                .typo(if on {
                    &t.text.body_strong
                } else {
                    &t.text.body
                })
                .text_color(if on { c.ink } else { c.ink_2 })
                .hover(move |s| s.text_color(c.ink))
                .focus_visible(ring_style(&t))
                .child(label)
                .when(on, |el| {
                    el.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .size(px(layout::TAB_SPOT))
                            .rounded(px(radius::FULL))
                            .bg(c.ink),
                    )
                })
                .when_some(pick, |el, pick| {
                    el.on_click(move |_, w, cx| pick(ix, w, cx))
                })
                .test_support()
        });
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(Role::TabList)
            .aria_label(self.name)
            .flex()
            .flex_row()
            .gap(px(space::SP_6))
            .children(pills)
            .test_support()
    }
}

/// A recipient chip.
#[derive(IntoElement)]
pub struct Chip {
    id: SharedString,
    name: SharedString,
    remove_name: SharedString,
    on_remove: Option<OnRemove>,
}

impl Chip {
    /// `remove_name` is the dismiss button's accessible name ("Remove
    /// Marta Silva").
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        remove_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            remove_name: remove_name.into(),
            on_remove: None,
        }
    }

    pub fn on_remove(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_remove = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let remove_id = ElementId::Name(format!("{}-remove", self.id).into());
        div()
            .id(ElementId::Name(self.id))
            .role(Role::Group)
            .aria_label(self.name.clone())
            .h(px(scaled::control(&t, layout::CHIP_H)))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::SP_2))
            .pl(px(space::SP_0_5 + layout::HAIRLINE))
            .pr(px(space::SP_1))
            .rounded(px(radius::FULL))
            .bg(c.wash)
            .text_color(c.ink)
            .typo(&t.text.body)
            .child(
                SenderSpot::new(&self.name)
                    .size(layout::AVATAR_SM)
                    .on_wash(true),
            )
            .child(self.name)
            .child(
                div()
                    .id(remove_id)
                    .role(Role::Button)
                    .aria_label(self.remove_name)
                    .tab_index(0)
                    .size(px(layout::ICON + space::SP_1))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::FULL))
                    .hover(move |s| s.bg(c.paper))
                    .focus_visible(ring_style(&t))
                    .child(
                        svg()
                            .path(Icon::Dismiss.path())
                            .size(px(layout::ICON_SM))
                            .text_color(c.ink_2),
                    )
                    .when_some(self.on_remove, |el, f| {
                        el.on_click(move |_, w, cx| f(w, cx))
                    })
                    .test_support(),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{Context, Render, TestAppContext};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    struct Demo {
        filter: usize,
        removed: bool,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let a = cx.entity().downgrade();
            let b = a.clone();
            div()
                .flex()
                .flex_col()
                .child(
                    FilterPills::new("filter", "Filter")
                        .items(["All", "Unread", "Important"])
                        .selected(self.filter)
                        .on_select(move |ix, _, cx| {
                            let _ = a.update(cx, |v, cx| {
                                v.filter = ix;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    Chip::new("to-marta", "Marta Silva", "Remove Marta Silva").on_remove(
                        move |_, cx| {
                            let _ = b.update(cx, |v, cx| {
                                v.removed = true;
                                cx.notify();
                            });
                        },
                    ),
                )
        }
    }

    #[gpui_kit::test]
    fn pills_select_and_chips_remove(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo {
            filter: 0,
            removed: false,
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(window.find_by_label("Filter").role(), Some(Role::TabList));
            let all = window.find_by_label("All");
            assert_eq!((all.role(), all.selected()), (Some(Role::Tab), Some(true)));
            assert_eq!(
                window.find_by_label("Remove Marta Silva").role(),
                Some(Role::Button)
            );
            window.click(ElementId::Name("filter-1".into()), cx);
            window.click(ElementId::Name("to-marta-remove".into()), cx);
        })
        .expect("click");
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!((v.filter, v.removed), (1, true)));
    }
}
