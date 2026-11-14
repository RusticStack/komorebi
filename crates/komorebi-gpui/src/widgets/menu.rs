//! Menus and context menus: Kit's `PopupMenu` (keyboard, submenus,
//! actions, AccessKit menu items) restyled for the brand through the
//! vendored `PopupMenu::menu_style` hook (SKUA-PATCH.md 3): paper, a 12 px
//! radius, wash under the hovered item, and no shadow. A paper menu over a
//! paper pane needs an edge, and the brand allows no shadow, so the menu's
//! edge is the one 1 px `line` (docs/GPUI-MIGRATION.md, phase 1 decisions).
//!
//! Use [`brand`] first in every menu builder:
//! `.context_menu(|m, _, cx| menu::brand(m, cx).menu("Reply", Box::new(Reply)))`
//! and `IconButton::dropdown_menu(..)` the same way.

use gpui::{App, px};
use gpui_kit::component::menu::{PopupMenu, PopupMenuStyle};

use crate::theme::{ActiveTokens as _, radius};

/// The brand's menu surface.
pub fn style(cx: &App) -> PopupMenuStyle {
    let c = cx.tokens().color;
    PopupMenuStyle {
        border: Some(c.line),
        radius: Some(px(radius::CARD)),
        shadow: Some(Vec::new()),
        item_hover: Some(c.wash),
        separator: Some(c.line),
    }
}

/// Applies the brand's surface to a menu being built.
pub fn brand(menu: PopupMenu, cx: &App) -> PopupMenu {
    menu.menu_style(style(cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::Icon;
    use crate::widgets::button::IconButton;
    use crate::widgets::test_support;
    use gpui::AppContext as _;
    use gpui::InteractiveElement as _;
    use gpui::{
        Context, IntoElement, MouseButton, ParentElement as _, Render, Role, Styled as _,
        TestAppContext, Window, div,
    };
    use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};

    struct Demo {
        picked: Option<&'static str>,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = cx.entity().downgrade();
            div()
                .size_full()
                .child(
                    IconButton::new("more", Icon::More, "More actions").dropdown_menu(
                        move |menu, _, cx| {
                            let view = view.clone();
                            brand(menu, cx)
                                .item(PopupMenuItem::new("Mark as unread").on_click(
                                    move |_, _, cx| {
                                        let _ = view.update(cx, |v, cx| {
                                            v.picked = Some("unread");
                                            cx.notify();
                                        });
                                    },
                                ))
                                .separator()
                                .item(PopupMenuItem::new("Move to"))
                        },
                    ),
                )
                .child(
                    div().id("row").size_full().context_menu(|menu, _, cx| {
                        brand(menu, cx).item(PopupMenuItem::new("Reply"))
                    }),
                )
        }
    }

    fn click_label(window: &mut Window, label: &str, cx: &mut App) {
        window.render_frame(cx);
        let center = window.find_by_label(label).bounds().center();
        window.pointer_move(center, None, cx);
        window.pointer_down(center, MouseButton::Left, cx);
        window.pointer_up(center, MouseButton::Left, cx);
        window.render_frame(cx);
    }

    #[gpui_kit::test]
    fn the_menu_opens_with_menu_items_and_runs_them(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |_, _| Demo { picked: None });
        cx.update_window(window.into(), |_, window, cx| {
            click_label(window, "More actions", cx)
        })
        .expect("open");
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find_by_label("Mark as unread").role(),
                Some(Role::MenuItem)
            );
            click_label(window, "Mark as unread", cx);
        })
        .expect("pick");
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!(v.picked, Some("unread")));
    }

    #[gpui::test]
    fn the_surface_is_flat_with_the_line_edge(cx: &mut TestAppContext) {
        test_support::init(cx, true);
        cx.update(|cx| {
            let s = style(cx);
            assert_eq!(s.shadow.map(|s| s.len()), Some(0), "no shadow");
            assert_eq!(s.border, Some(cx.tokens().color.line));
            assert_eq!(s.item_hover, Some(cx.tokens().color.wash));
        });
    }
}
