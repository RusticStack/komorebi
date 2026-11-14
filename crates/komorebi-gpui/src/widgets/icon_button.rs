//! The round 36 px icon button (mockup `.ib`): a Fluent glyph in ink-2,
//! wash on hover and while toggled on, a tooltip and an accessible name
//! (BRAND.md: every icon-only control has both). A plain GPUI div like
//! `Button`, and a `DropdownMenu` trigger (the More menu).

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, InteractiveElement, Interactivity, IntoElement, ParentElement,
    RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement, Styled,
    Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::Selectable;
use gpui_kit::component::menu::DropdownMenu;

use super::button::OnClick;
use super::ring_style;
use super::tooltip::WithTooltip as _;
use crate::access::{self, Key};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, layout, motion, radius};

/// A round 36 px icon-only button with a tooltip and an accessible name
/// (BRAND.md: every icon-only control has both).
#[derive(IntoElement)]
pub struct IconButton {
    base: Stateful<Div>,
    icon: Icon,
    name: SharedString,
    tooltip: Option<SharedString>,
    small: bool,
    toggled: Option<bool>,
    selected: bool,
    disabled: bool,
    /// Off for a button the pointer reaches but Tab passes by (a list
    /// row's quick actions).
    tab_stop: bool,
    access: Option<Key>,
    on_click: Option<OnClick>,
}

impl IconButton {
    /// `name` is the accessible name, and the tooltip unless one is given.
    pub fn new(id: impl Into<ElementId>, icon: Icon, name: impl Into<SharedString>) -> Self {
        Self {
            base: div().id(id),
            icon,
            name: name.into(),
            tooltip: None,
            small: false,
            toggled: None,
            selected: false,
            disabled: false,
            tab_stop: true,
            access: None,
            on_click: None,
        }
    }

    /// The tooltip, when it says more than the name (a shortcut).
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// 32 px (the composer's format bar).
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// A toggle (Bold, Filter on): wash fill and ink glyph while on.
    pub fn toggled(mut self, on: bool) -> Self {
        self.toggled = Some(on);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Tab passes this button by; a click still reaches it (the Slint
    /// list's quick actions, test row 164).
    pub fn no_tab_stop(mut self) -> Self {
        self.tab_stop = false;
        self
    }

    /// The command's access key (see `Button::access_key`).
    pub fn access_key(mut self, key: Key) -> Self {
        self.access = Some(key);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl Styled for IconButton {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for IconButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for IconButton {}

impl Selectable for IconButton {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl DropdownMenu for IconButton {}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let on = self.toggled.unwrap_or(false) || self.selected;
        let size = if self.small {
            layout::ICON_BTN_SM
        } else {
            layout::ICON_BTN
        };
        let glyph = if on { c.ink } else { c.ink_2 };
        let mut el = self
            .base
            .role(Role::Button)
            .aria_label(self.name.clone())
            .when_some(self.toggled, |el, on| {
                el.aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
            })
            .size(px(size))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(radius::FULL))
            .when(on, |el| el.bg(c.wash))
            .child(
                svg()
                    .path(self.icon.path())
                    .size(px(layout::ICON))
                    .text_color(glyph),
            );
        if self.disabled {
            // UI Automation reads it as not available, not only dimmed.
            el = el
                .opacity(motion::DISABLED)
                .a11y_synthetic_children(|b| b.parent_node().set_disabled());
        } else {
            if let Some(key) = self.access {
                el = access::decorate(el, key, cx);
            }
            el = el
                .tab_index(0)
                .tab_stop(self.tab_stop)
                .hover(move |s| s.bg(c.wash))
                .focus_visible(ring_style(&t));
            if let Some(f) = self.on_click {
                el = el.on_click(move |e, w, cx| f(e, w, cx));
            }
        }
        let tip = self.tooltip.unwrap_or(self.name);
        el.brand_tooltip(tip).test_support()
    }
}
