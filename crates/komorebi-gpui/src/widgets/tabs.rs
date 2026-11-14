//! Tabs and the segmented control.
//!
//! [`TabStrip`] (for the tabs phase, docs/GPUI-MIGRATION.md phase 5): pill
//! tabs on canvas, the current one ink at 600 with the 4 px place spot under
//! it (BRAND.md: tabs mark the current place with a spot), the others ink-2
//! with wash on hover; a closable tab has a round dismiss button.
//!
//! [`Segmented`] (mockup `.seg`): an inset track with 32 px segments; the
//! current one is paper (`segment_on` in dark) and 600 (Theme: System,
//! Light, Dark; the calendar's Month, Week).

use std::rc::Rc;

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Window,
    div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use super::ring_style;
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};

type OnIndex = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// One tab of a strip.
#[derive(Debug, Clone)]
pub struct TabSpec {
    pub label: SharedString,
    pub closable: bool,
}

impl TabSpec {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            closable: false,
        }
    }

    pub fn closable(mut self) -> Self {
        self.closable = true;
        self
    }
}

/// A row of tabs.
#[derive(IntoElement)]
pub struct TabStrip {
    id: SharedString,
    name: SharedString,
    tabs: Vec<TabSpec>,
    selected: usize,
    close_name: SharedString,
    on_select: Option<OnIndex>,
    on_close: Option<OnIndex>,
}

impl TabStrip {
    /// `close_name` names each tab's dismiss button ("Close tab").
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        close_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            tabs: Vec::new(),
            selected: 0,
            close_name: close_name.into(),
            on_select: None,
            on_close: None,
        }
    }

    pub fn tabs(mut self, tabs: impl IntoIterator<Item = TabSpec>) -> Self {
        self.tabs = tabs.into_iter().collect();
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

    pub fn on_close(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for TabStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let count = self.tabs.len();
        let tabs = self.tabs.into_iter().enumerate().map(|(ix, tab)| {
            let on = ix == self.selected;
            let select = self.on_select.clone();
            let close = self.on_close.clone();
            let close_id = ElementId::Name(format!("{}-close-{ix}", self.id).into());
            div()
                .id(ElementId::Name(format!("{}-{ix}", self.id).into()))
                .role(Role::Tab)
                .aria_label(tab.label.clone())
                .aria_selected(on)
                .aria_position_in_set(ix + 1)
                .aria_size_of_set(count)
                .tab_index(0)
                .relative()
                .h(px(scaled::control(&t, layout::CONTROL_H)))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::SP_1))
                .pl(px(space::SP_3))
                .pr(px(if tab.closable {
                    space::SP_1
                } else {
                    space::SP_3
                }))
                .rounded(px(radius::FULL))
                .typo(if on {
                    &t.text.body_strong
                } else {
                    &t.text.body
                })
                .text_color(if on { c.ink } else { c.ink_2 })
                .hover(move |s| s.bg(c.wash))
                .focus_visible(ring_style(&t))
                .child(tab.label.clone())
                .when(on, |el| {
                    el.child(
                        div()
                            .absolute()
                            .bottom(px(-space::SP_1))
                            .left_0()
                            .right_0()
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(layout::TAB_SPOT))
                                    .rounded(px(radius::FULL))
                                    .bg(c.ink),
                            ),
                    )
                })
                .when(tab.closable, |el| {
                    el.child(
                        div()
                            .id(close_id)
                            .role(Role::Button)
                            .aria_label(format!("{}: {}", self.close_name, tab.label))
                            .size(px(layout::CHIP_H - space::SP_1))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(radius::FULL))
                            .hover(move |s| s.bg(c.paper))
                            .child(
                                svg()
                                    .path(Icon::Dismiss.path())
                                    .size(px(layout::ICON_SM))
                                    .text_color(c.ink_2),
                            )
                            .when_some(close, |el, f| {
                                el.on_click(move |_, w, cx| {
                                    cx.stop_propagation();
                                    f(ix, w, cx)
                                })
                            })
                            .test_support(),
                    )
                })
                .when_some(select, |el, f| el.on_click(move |_, w, cx| f(ix, w, cx)))
                .test_support()
        });
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(Role::TabList)
            .aria_label(self.name)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_1))
            .children(tabs)
            .test_support()
    }
}

/// A segmented control.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    name: SharedString,
    items: Vec<SharedString>,
    selected: usize,
    on_select: Option<OnIndex>,
}

impl Segmented {
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

impl RenderOnce for Segmented {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let count = self.items.len();
        let segments = self.items.into_iter().enumerate().map(|(ix, label)| {
            let on = ix == self.selected;
            let select = self.on_select.clone();
            div()
                .id(ElementId::Name(format!("{}-{ix}", self.id).into()))
                .role(Role::Tab)
                .aria_label(label.clone())
                .aria_selected(on)
                .aria_position_in_set(ix + 1)
                .aria_size_of_set(count)
                .tab_index(0)
                .h(px(scaled::control(&t, layout::CONTROL_H)))
                .flex()
                .items_center()
                .px(px(space::SP_3))
                .rounded(px(radius::FULL))
                .typo(if on {
                    &t.text.body_strong
                } else {
                    &t.text.body
                })
                .text_color(if on { c.ink } else { c.ink_2 })
                .when(on, |el| el.bg(c.segment_on))
                .focus_visible(ring_style(&t))
                .child(label)
                .when_some(select, |el, f| el.on_click(move |_, w, cx| f(ix, w, cx)))
                .test_support()
        });
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(Role::TabList)
            .aria_label(self.name)
            .flex()
            .flex_none()
            // Its own width, also in a column.
            .self_start()
            .p(px(space::SP_0_5))
            .rounded(px(radius::FULL))
            .bg(c.inset)
            .children(segments)
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
        tab: usize,
        closed: Option<usize>,
        theme: usize,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (a, b, d) = (
                cx.entity().downgrade(),
                cx.entity().downgrade(),
                cx.entity().downgrade(),
            );
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_4))
                .child(
                    TabStrip::new("tabs", "Open messages", "Close tab")
                        .tabs([
                            TabSpec::new("Inbox"),
                            TabSpec::new("Photos from Gerês").closable(),
                        ])
                        .selected(self.tab)
                        .on_select(move |ix, _, cx| {
                            let _ = a.update(cx, |v, cx| {
                                v.tab = ix;
                                cx.notify();
                            });
                        })
                        .on_close(move |ix, _, cx| {
                            let _ = b.update(cx, |v, _| v.closed = Some(ix));
                        }),
                )
                .child(
                    Segmented::new("theme", "Theme")
                        .items(["System", "Light", "Dark"])
                        .selected(self.theme)
                        .on_select(move |ix, _, cx| {
                            let _ = d.update(cx, |v, cx| {
                                v.theme = ix;
                                cx.notify();
                            });
                        }),
                )
        }
    }

    #[gpui_kit::test]
    fn tabs_select_and_close_segments_select(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo {
            tab: 0,
            closed: None,
            theme: 0,
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window.find_by_label("Open messages").role(),
                Some(Role::TabList)
            );
            assert_eq!(window.find_by_label("Theme").role(), Some(Role::TabList));
            assert_eq!(
                window.find_by_label("Close tab: Photos from Gerês").role(),
                Some(Role::Button)
            );
            window.click(ElementId::Name("tabs-1".into()), cx);
            window.click(ElementId::Name("tabs-close-1".into()), cx);
            window.click(ElementId::Name("theme-2".into()), cx);
        })
        .expect("click");
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!((v.tab, v.closed, v.theme), (1, Some(1), 2));
        });
    }
}
