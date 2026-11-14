use crate::{ActiveTheme, Disableable, StyledExt, h_flex};
use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, // KOMOREBI PATCH: div
};
use gpui_base::TestSupportExt as _;
use smallvec::SmallVec;

#[derive(IntoElement)]
pub(crate) struct MenuItemElement {
    id: ElementId,
    aria_label: Option<SharedString>,
    /// KOMOREBI PATCH: the item's access key, and whether its chip shows.
    access_key: Option<(char, bool)>,
    style: StyleRefinement,
    disabled: bool,
    selected: bool,
    /// SKUA PATCH (G3-F20): hover/selected fill instead of the theme `accent`.
    hover_bg: Option<gpui::Hsla>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    on_hover: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
    children: SmallVec<[AnyElement; 2]>,
}

impl MenuItemElement {
    /// Create a new MenuItem with the given ID.
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        let id: ElementId = id.into();
        Self {
            id: id.clone(),
            aria_label: None,
            access_key: None, // KOMOREBI PATCH
            style: StyleRefinement::default(),
            disabled: false,
            selected: false,
            hover_bg: None,
            on_click: None,
            on_hover: None,
            children: SmallVec::new(),
        }
    }

    /// Set ListItem as the selected item style.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// SKUA PATCH (G3-F20): hover/selected fill; `None` keeps the theme `accent`.
    pub(crate) fn hover_bg(mut self, color: Option<gpui::Hsla>) -> Self {
        self.hover_bg = color;
        self
    }

    /// Set the accessible label for the menu item.
    pub(crate) fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// KOMOREBI PATCH: the item's access key: told to assistive technology
    /// as its keyboard shortcut, and drawn as a chip at the item's end while
    /// `shown`.
    pub(crate) fn access_key(mut self, key: Option<char>, shown: bool) -> Self {
        self.access_key = key.map(|key| (key, shown));
        self
    }

    /// Set the disabled state of the MenuItem.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set a handler for when the MenuItem is clicked.
    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set a handler for when the mouse enters the MenuItem.
    #[allow(unused)]
    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(handler));
        self
    }
}

impl Disableable for MenuItemElement {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for MenuItemElement {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for MenuItemElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for MenuItemElement {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // SKUA PATCH (G3-F20)
        let hover_bg: gpui::Fill = match self.hover_bg {
            Some(color) => color.into(),
            None => cx.theme().tokens.accent.into(),
        };
        h_flex()
            .id(self.id)
            .test_support()
            .role(Role::MenuItem)
            .when_some(self.aria_label, |this, label| this.aria_label(label))
            // KOMOREBI PATCH: the access key.
            .when_some(self.access_key, |this, (key, _)| {
                this.aria_keyshortcuts(key.to_uppercase().to_string())
            })
            .aria_selected(self.selected)
            .gap_x_1()
            .py_1()
            .px_2()
            .text_base()
            .text_color(cx.theme().foreground)
            .relative()
            .items_center()
            .justify_between()
            .refine_style(&self.style)
            .when_some(self.on_hover, |this, on_hover| {
                this.on_hover(move |hovered, window, cx| (on_hover)(hovered, window, cx))
            })
            .when(!self.disabled, |this| {
                // Not `group_hover` on the item's own group: GPUI styles an
                // element before registering its group, and that fallback keeps
                // the pointer's item lit after a key moves the highlight away.
                this.hover(|this| {
                    this.bg(hover_bg.clone()) // SKUA PATCH (G3-F20)
                        .text_color(cx.theme().accent_foreground)
                })
                .when(self.selected, |this| {
                    this.bg(hover_bg) // SKUA PATCH (G3-F20)
                        .text_color(cx.theme().accent_foreground)
                })
                .when_some(self.on_click, |this, on_click| {
                    this.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(on_click)
                })
            })
            .when(self.disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .children(self.children)
            // KOMOREBI PATCH: the access key's chip.
            .when_some(
                self.access_key.filter(|(_, shown)| *shown),
                |this, (key, _)| {
                    let key = key.to_ascii_uppercase();
                    this.child(
                        div()
                            .id(("menu-access-key", key as usize))
                            .test_support()
                            .flex_none()
                            .px_1()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().foreground)
                            .text_color(cx.theme().background)
                            .text_xs()
                            .child(key.to_string()),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn aria_label_sets_accessible_name(_cx: &mut gpui::TestAppContext) {
        let item = MenuItemElement::new("open").aria_label("Open");

        assert_eq!(item.aria_label, Some("Open".into()));
    }
}
