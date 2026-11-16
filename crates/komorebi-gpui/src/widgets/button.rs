//! Buttons: the pill [`Button`] (primary ink, secondary wash, subtle,
//! on-inset paper, destructive); the round `IconButton` is in
//! `icon_button.rs`. Plain GPUI
//! divs, because Kit's `Button` draws its own radius, padding and focus
//! ring: GPUI gives a focusable div with a click listener Enter and Space,
//! the AccessKit Click action and the tab order. Both implement
//! `Selectable`, `Styled` and `InteractiveElement`, so Kit's
//! `DropdownMenu` and `ContextMenuExt` take them as triggers.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, Hsla, InteractiveElement, Interactivity, IntoElement,
    ParentElement, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::Selectable;
use gpui_kit::component::menu::DropdownMenu;

use super::ring_style;
use super::tooltip::WithTooltip as _;
use crate::access::{self, Key};
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, motion, radius, space};

pub(super) type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub use super::icon_button::IconButton;

/// The look of a pill button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// Ink fill, on-ink text, 600: New mail, Send, a dialog's main action.
    Primary,
    /// Wash fill: Reply, Cancel.
    #[default]
    Secondary,
    /// No fill until hovered: toolbar text buttons.
    Subtle,
    /// Paper on an inset card or notice.
    OnInset,
    /// Inset on paper: selects and quiet pills in a pane.
    Inset,
    /// A destructive action. Ink like Primary: the brand has no red; the
    /// word says what happens, and a confirm dialog guards it.
    Destructive,
}

/// The height of a pill button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Height {
    /// 32 px: most buttons.
    #[default]
    Regular,
    /// 36 px: Send, Reply.
    Medium,
    /// 40 px: New mail.
    Large,
}

impl Height {
    fn px(self) -> f32 {
        match self {
            Height::Regular => layout::CONTROL_H,
            Height::Medium => layout::CONTROL_H_MD,
            Height::Large => layout::CONTROL_H_LG,
        }
    }

    /// The leading icon's size: 16 px in a 32 px button (Fluent's dense
    /// size), 20 px in the 36 and 40 px ones, beside their 14 px label.
    pub fn icon(self) -> f32 {
        match self {
            Height::Regular => layout::ICON,
            Height::Medium | Height::Large => layout::ICON_LG,
        }
    }

    /// The padding before a leading icon: the label's 16 px less the
    /// glyph's own 2 px margin (Fluent's icons keep a 2 px margin inside
    /// their box), so icon and text sit optically even in the pill.
    pub fn icon_pad(self) -> f32 {
        match self {
            Height::Regular => space::SP_3,
            Height::Medium | Height::Large => space::SP_3,
        }
    }
}

/// Fill, hover fill and text colour of a variant.
fn colours(t: &Tokens, v: Variant, selected: bool) -> (Hsla, Hsla, Hsla) {
    let c = &t.color;
    match v {
        Variant::Primary | Variant::Destructive => {
            (c.ink, c.ink.opacity(motion::INK_HOVER), c.on_ink)
        }
        Variant::Secondary => (c.wash, c.wash, c.ink),
        Variant::Subtle if selected => (c.wash, c.wash, c.ink),
        Variant::Subtle => (c.clear, c.wash, c.ink),
        Variant::OnInset => (c.pill_on_inset, c.wash, c.ink),
        Variant::Inset => (c.inset, c.wash, c.ink),
    }
}

/// A pill button.
#[derive(IntoElement)]
pub struct Button {
    base: Stateful<Div>,
    label: Option<SharedString>,
    /// The accessible name when it says more than the label.
    name: Option<SharedString>,
    icon: Option<Icon>,
    /// A small trailing glyph (a select's chevron).
    trailing: Option<Icon>,
    variant: Variant,
    height: Height,
    disabled: bool,
    selected: bool,
    /// A select's trigger: its expanded state follows its menu
    /// (`Some(open)`); `None` is a plain button.
    expands: Option<bool>,
    /// The format bar's boxes: 8 px sides, a 4 px gap, and a label that
    /// ends in an ellipsis in a button given a fixed width.
    compact: bool,
    tooltip: Option<SharedString>,
    access: Option<Key>,
    on_click: Option<OnClick>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            label: None,
            name: None,
            icon: None,
            trailing: None,
            variant: Variant::default(),
            height: Height::default(),
            disabled: false,
            selected: false,
            expands: None,
            compact: false,
            tooltip: None,
            access: None,
            on_click: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A select's trigger: collapsed until its menu opens (Kit's popover
    /// marks it open through [`Selectable::open`]).
    pub fn expandable(mut self) -> Self {
        self.expands = Some(false);
        self
    }

    /// The format bar's box (font, size, alignment): tight sides and a
    /// label cut with an ellipsis when the button is given a width.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// The accessible name, when the label alone is not enough.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// A trailing 10 px glyph in ink-2 (a select's chevron).
    pub fn trailing(mut self, icon: Icon) -> Self {
        self.trailing = Some(icon);
        self
    }

    /// A leading icon (New mail's pencil, Reply's arrow).
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }

    pub fn height(mut self, height: Height) -> Self {
        self.height = height;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// The command's access key (Alt and its letter, `crate::access`): the
    /// button draws its chip while Alt is held and tells UI Automation the
    /// letter. The shortcut itself is the command's action, bound once.
    pub fn access_key(mut self, key: Key) -> Self {
        self.access = Some(key);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Button {}

impl Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }

    // A select shows its open menu as expanded, not as selected; any
    // other trigger keeps the default (selected while its menu is open).
    fn open(mut self, open: bool) -> Self {
        match self.expands {
            Some(_) => {
                self.expands = Some(open);
                self
            }
            None => self.selected(open),
        }
    }

    fn is_open(&self) -> bool {
        self.expands.unwrap_or(self.selected)
    }
}

impl DropdownMenu for Button {}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let (fill, hover, fg) = colours(&t, self.variant, self.selected);
        let strong = matches!(self.variant, Variant::Primary | Variant::Destructive);
        let text = if strong {
            &t.text.body_strong
        } else {
            &t.text.body
        };
        let name = self
            .name
            .clone()
            .or_else(|| self.label.clone())
            .unwrap_or_default();
        let pad_left = if self.compact {
            space::SP_2
        } else if self.icon.is_some() {
            self.height.icon_pad()
        } else {
            space::SP_4
        };
        let (pad_right, gap) = if self.compact {
            (space::SP_2, space::SP_1)
        } else {
            (space::SP_4, space::SP_2)
        };
        let compact = self.compact;
        let icon_size = self.height.icon();
        let mut el = self
            .base
            // A select's trigger stays a Button node with an expanded state
            // (UIA's ExpandCollapse pattern, as a WinUI menu button has).
            .role(Role::Button)
            .when_some(self.expands, |el, open| el.aria_expanded(open))
            .aria_label(name)
            .h(px(scaled::control(&t, self.height.px())))
            .min_w(px(scaled::control(&t, self.height.px())))
            .pl(px(pad_left))
            .pr(px(pad_right))
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(gap))
            .rounded(px(radius::FULL))
            .bg(fill)
            .text_color(fg)
            .typo(text)
            .whitespace_nowrap()
            .when_some(self.icon, |el, icon| {
                el.child(
                    svg()
                        .path(icon.state(icon_size, self.selected))
                        .size(px(icon_size))
                        .flex_none()
                        .text_color(fg),
                )
            })
            .when_some(self.label, |el, label| {
                if compact {
                    el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(label),
                    )
                } else {
                    el.child(label)
                }
            })
            .when_some(self.trailing, |el, icon| {
                el.child(
                    svg()
                        .path(icon.path())
                        .size(px(layout::ICON_SM))
                        .flex_none()
                        .text_color(t.color.ink_2),
                )
            });
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
                .hover(move |s| s.bg(hover))
                .active(move |s| s.opacity(motion::INK_HOVER))
                .focus_visible(ring_style(&t));
            if let Some(f) = self.on_click {
                el = el.on_click(move |e, w, cx| f(e, w, cx));
            }
        }
        el.when_some(self.tooltip, |el, tip| el.brand_tooltip(tip))
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
        clicks: usize,
        bold: bool,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = cx.entity().downgrade();
            let bold_view = view.clone();
            div()
                .flex()
                .gap(px(space::SP_2))
                .child(
                    Button::new("new-mail")
                        .primary()
                        .icon(Icon::Compose)
                        .label("New mail")
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |v, cx| {
                                v.clicks += 1;
                                cx.notify();
                            });
                        }),
                )
                .child(Button::new("off").label("Unavailable").disabled(true))
                .child(
                    IconButton::new("bold", Icon::TextSize, "Bold")
                        .tooltip("Bold (Ctrl+B)")
                        .toggled(self.bold)
                        .on_click(move |_, _, cx| {
                            let _ = bold_view.update(cx, |v, cx| {
                                v.bold = !v.bold;
                                cx.notify();
                            });
                        }),
                )
        }
    }

    #[gpui_kit::test]
    fn buttons_have_names_roles_and_click(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo {
            clicks: 0,
            bold: false,
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(window.find_by_label("New mail").role(), Some(Role::Button));
            let bold = window.find_by_label("Bold");
            assert_eq!(bold.role(), Some(Role::Button));
            window.click("new-mail", cx);
            window.click("off", cx);
            window.click("bold", cx);
        })
        .expect("click");
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(v.clicks, 1, "the disabled button does nothing");
            assert!(v.bold);
        });
    }

    #[test]
    fn primary_is_ink_and_subtle_is_clear_until_selected() {
        let t = crate::theme::tokens(&crate::theme::ThemeInput::new(
            komorebi::db::ThemeChoice::Light,
            komorebi::theme::SystemTheme::default(),
        ));
        assert_eq!(colours(&t, Variant::Primary, false).0, t.color.ink);
        assert_eq!(colours(&t, Variant::Destructive, false).0, t.color.ink);
        assert_eq!(colours(&t, Variant::Subtle, false).0, t.color.clear);
        assert_eq!(colours(&t, Variant::Subtle, true).0, t.color.wash);
    }
}
