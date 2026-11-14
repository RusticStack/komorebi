//! The tab strip (docs/WINUI3.md "TabView"; BRAND.md: tabs mark the current
//! place with a spot): over the content, from two tabs on. The Mail tab is
//! its glyph and word and never closes; a message or draft tab has its
//! title, elided, and a round close button. The current tab is ink at 600
//! with the 4 px place spot under it, the others ink-2 with wash on hover.
//! A middle click closes a tab; a right click opens its menu (Close tab,
//! Close other tabs, Close tabs to the right, Open in new window, Reopen
//! closed tab). The strip scrolls sideways when the tabs do not fit.
//!
//! It draws the library's `Tabs` and reports what was asked through the
//! three callbacks; the window acts on them.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, MouseButton, ParentElement,
    Role, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use komorebi::tabs::{TabKind, Tabs};

use super::text;
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{TextStyled as _, Tokens, layout, radius, space};
use crate::widgets::menu;
use crate::widgets::ring_style;
use crate::widgets::tooltip::WithTooltip as _;

/// What a tab's menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCommand {
    Close,
    CloseOthers,
    CloseRight,
    OpenWindow,
    Reopen,
}

type OnIndex = Rc<dyn Fn(usize, &mut Window, &mut App)>;
type OnCommand = Rc<dyn Fn(usize, TabCommand, &mut Window, &mut App)>;

/// The strip's callbacks.
#[derive(Clone)]
pub struct Handlers {
    pub select: OnIndex,
    pub close: OnIndex,
    pub command: OnCommand,
}

/// The widest a tab grows before its title is elided.
const TAB_MAX_W: f32 = layout::NAV_W;

/// The strip, or nothing while the Mail tab is alone.
pub fn render(tabs: &Tabs, t: &Tokens, on: Handlers) -> Option<AnyElement> {
    if tabs.len() < 2 {
        return None;
    }
    let c = t.color;
    let count = tabs.len();
    let reopenable = tabs.has_closed();
    let items = tabs.iter().enumerate().map(|(ix, tab)| {
        let on_tab = ix == tabs.active();
        let mail = tab.kind == TabKind::Mail;
        let draft = matches!(tab.kind, TabKind::Draft(_) | TabKind::Saved(_));
        let message = matches!(tab.kind, TabKind::Message(_));
        let title = if mail {
            text::mail()
        } else {
            tab.title.clone()
        };
        let name = if mail {
            title.clone()
        } else {
            text::tab_name(&title, draft, tab.unread, ix + 1, count)
        };
        let glyph = match tab.kind {
            TabKind::Mail => Icon::Mail,
            TabKind::Message(_) => Icon::MailRead,
            TabKind::Draft(_) | TabKind::Saved(_) => Icon::Compose,
        };
        let select = on.select.clone();
        let close = on.close.clone();
        let middle = on.close.clone();
        let command = on.command.clone();
        let right = ix + 1 < count;
        div()
            .id(ElementId::Name(format!("tab-{ix}").into()))
            .role(Role::Tab)
            .aria_label(name)
            .aria_selected(on_tab)
            .aria_position_in_set(ix + 1)
            .aria_size_of_set(count)
            .tab_index(0)
            .relative()
            .h(px(scaled::control(t, layout::CONTROL_H)))
            .max_w(px(TAB_MAX_W))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::SP_2))
            .pl(px(space::SP_3))
            .pr(px(if mail { space::SP_3 } else { space::SP_1 }))
            .rounded(px(radius::FULL))
            .typo(if on_tab {
                &t.text.body_strong
            } else {
                &t.text.body
            })
            .text_color(if on_tab { c.ink } else { c.ink_2 })
            .hover(move |s| s.bg(c.wash))
            .focus_visible(ring_style(t))
            .child(
                svg()
                    .path(glyph.path())
                    .flex_none()
                    .size(px(layout::ICON))
                    .text_color(if on_tab { c.ink } else { c.ink_2 }),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    // A message gone from the server reads quieter.
                    .when(tab.gone, |el| el.text_color(c.ink_3))
                    .child(title),
            )
            .when(on_tab, |el| {
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
            .when(!mail, |el| {
                el.child(
                    div()
                        .id(ElementId::Name(format!("tab-close-{ix}").into()))
                        .role(Role::Button)
                        .aria_label(format!("{}: {}", text::close_tab(), tab.title))
                        .flex_none()
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
                        .on_click(move |_, w, cx| {
                            cx.stop_propagation();
                            close(ix, w, cx)
                        })
                        .brand_tooltip(text::close_tab_tip())
                        .test_support(),
                )
            })
            .on_click(move |_, w, cx| select(ix, w, cx))
            .when(!mail, |el| {
                el.on_mouse_up(MouseButton::Middle, move |_, w, cx| middle(ix, w, cx))
            })
            .test_support()
            .context_menu(move |m, _, cx| {
                let item = |label: String, cmd: TabCommand, enabled: bool| {
                    let command = command.clone();
                    PopupMenuItem::new(label)
                        .disabled(!enabled)
                        .on_click(move |_, w, cx| command(ix, cmd, w, cx))
                };
                menu::brand(m, cx)
                    .item(item(text::close_tab(), TabCommand::Close, !mail))
                    .item(item(
                        text::close_others(),
                        TabCommand::CloseOthers,
                        count > 2 || mail,
                    ))
                    .item(item(text::close_right(), TabCommand::CloseRight, right))
                    .separator()
                    .item(item(
                        text::open_window(),
                        TabCommand::OpenWindow,
                        message || draft,
                    ))
                    .item(item(text::reopen(), TabCommand::Reopen, reopenable))
            })
    });
    Some(
        div()
            .id("tab-strip")
            .role(Role::TabList)
            .aria_label(text::strip())
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_1))
            .px(px(space::PANE_GAP))
            .pb(px(space::SP_2))
            .overflow_x_scroll()
            .children(items)
            .test_support()
            .into_any_element(),
    )
}
