//! Surfaces: the paper [`Pane`] (12 px corners on canvas, 8 px apart: the
//! gap is the divider, no hairline between panes), a [`ScrollArea`] with
//! Kit's scrollbar (thumb ink-3, track clear, from the Kit theme), and the
//! [`ListRow`] container every list row is built on (16 px corners; wash
//! and a 4 x 20 ink marker at the left edge when selected, because wash
//! alone is 1.16:1 on paper; the focus ring for keyboard focus).

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, RenderOnce, Role,
    ScrollHandle, SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::scroll::ScrollableElement as _;

use super::ring_style;
use crate::theme::{ActiveTokens as _, layout, radius, space};

/// A paper pane. Lay panes out in a row or column with
/// `gap(px(space::PANE_GAP))` on canvas.
#[derive(IntoElement)]
pub struct Pane {
    children: Vec<AnyElement>,
    name: Option<SharedString>,
    id: ElementId,
}

impl Pane {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            children: Vec::new(),
            name: None,
            id: id.into(),
        }
    }

    /// A landmark name for screen readers ("Messages", "Reading pane").
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl ParentElement for Pane {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Pane {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = cx.tokens().color;
        div()
            .id(self.id)
            .when_some(self.name, |el, name| el.role(Role::Pane).aria_label(name))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(radius::SURFACE))
            .bg(c.paper)
            .text_color(c.ink)
            .children(self.children)
            .test_support()
    }
}

/// A vertical scroll area with Kit's scrollbar over its right edge
/// (cookbook_scrollbar). The caller keeps the `ScrollHandle` to scroll
/// rows into view.
#[derive(IntoElement)]
pub struct ScrollArea {
    id: ElementId,
    handle: ScrollHandle,
    children: Vec<AnyElement>,
}

impl ScrollArea {
    pub fn new(id: impl Into<ElementId>, handle: &ScrollHandle) -> Self {
        Self {
            id: id.into(),
            handle: handle.clone(),
            children: Vec::new(),
        }
    }
}

impl ParentElement for ScrollArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ScrollArea {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .id(self.id)
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.handle)
                    .children(self.children),
            )
            .vertical_scrollbar(&self.handle)
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A list row: the shape, states and semantics; the content is the
/// caller's (a message row, a contact, a folder).
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    name: SharedString,
    selected: bool,
    position: Option<(usize, usize)>,
    height: Option<f32>,
    children: Vec<AnyElement>,
    on_click: Option<OnClick>,
    /// A tab stop of its own (rows of a small list); a long list is one tab
    /// stop and draws the ring on its current row ([`ListRow::ring`]).
    focusable: bool,
    ring: bool,
    current: bool,
    /// The ink marker at the left edge (on by default when selected).
    marker: Option<bool>,
}

impl ListRow {
    /// `name` is what a screen reader says for the row ("Unread, Marta
    /// Silva, Photos from Gerês").
    pub fn new(id: impl Into<ElementId>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            selected: false,
            position: None,
            height: None,
            children: Vec::new(),
            on_click: None,
            focusable: true,
            ring: false,
            current: false,
            marker: None,
        }
    }

    /// Not a tab stop: the list it is in holds the focus (and its keys).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// The keyboard's current row of a list that holds the focus: the
    /// focus ring, drawn here because the row itself has no focus.
    pub fn ring(mut self, ring: bool) -> Self {
        self.ring = ring;
        self
    }

    /// Whether the selected row's ink marker shows (a checked row of a
    /// multi-select is wash alone: the marker is the open row's).
    pub fn marker(mut self, marker: bool) -> Self {
        self.marker = Some(marker);
        self
    }

    /// The list's current row: screen readers hear it as focused while the
    /// list holds the focus (the active descendant).
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 1-based position and the list's length ("3 of 120").
    pub fn position(mut self, position: usize, of: usize) -> Self {
        self.position = Some((position, of));
        self
    }

    /// A fixed height (virtual lists need one: `layout::ROW_H`).
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl ParentElement for ListRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ListRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let selected = self.selected;
        let marker = self.marker.unwrap_or(selected);
        div()
            .id(self.id)
            .role(Role::ListItem)
            .aria_label(self.name)
            .aria_selected(selected)
            .when_some(self.position, |el, (pos, of)| {
                el.aria_position_in_set(pos).aria_size_of_set(of)
            })
            .when(self.focusable, |el| el.tab_index(0))
            .when(self.current, |el| el.aria_active_descendant())
            .relative()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_3))
            .px(px(space::SP_3))
            .when_some(self.height, |el, h| el.h(px(h)).flex_none())
            .rounded(px(radius::ROW))
            .when(selected, |el| el.bg(c.wash))
            .when(!selected, |el| el.hover(move |s| s.bg(c.wash)))
            .focus_visible(ring_style(&t))
            .when(self.ring, |el| el.shadow(super::focus_ring(&t)))
            .when(marker, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(px(space::SP_0_5 + layout::HAIRLINE))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .w(px(layout::MARKER_W))
                                .h(px(layout::MARKER_H))
                                .rounded(px(radius::FULL))
                                .bg(c.ink),
                        ),
                )
            })
            .children(self.children)
            .when_some(self.on_click, |el, f| {
                el.on_click(move |e, w, cx| f(e, w, cx))
            })
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
        selected: usize,
        scroll: ScrollHandle,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let rows = (0..30).map(|ix| {
                let view = cx.entity().downgrade();
                ListRow::new(("row", ix), format!("Message {ix}"))
                    .selected(ix == self.selected)
                    .position(ix + 1, 30)
                    .height(layout::ROW_H)
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |v, cx| {
                            v.selected = ix;
                            cx.notify();
                        });
                    })
                    .child(format!("Message {ix}"))
            });
            div().size_full().p(px(space::PANE_GAP)).child(
                Pane::new("list")
                    .name("Messages")
                    .child(ScrollArea::new("rows", &self.scroll).children(rows)),
            )
        }
    }

    #[gpui_kit::test]
    fn rows_are_selectable_list_items_in_a_named_pane(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo {
            selected: 0,
            scroll: ScrollHandle::new(),
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(window.find_by_label("Messages").role(), Some(Role::Pane));
            let first = window.find_by_label("Message 0");
            assert_eq!(
                (first.role(), first.selected()),
                (Some(Role::ListItem), Some(true))
            );
            window.click(("row", 2usize), cx);
        })
        .expect("click");
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!(v.selected, 2));
        let scroll = view.read_with(cx, |v, _| v.scroll.clone());
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert!(scroll.max_offset().y > px(0.), "30 rows overflow the pane");
        })
        .expect("scroll");
    }
}
