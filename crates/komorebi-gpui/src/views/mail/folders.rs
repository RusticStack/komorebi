//! The folder pane on canvas (mockup `.nav`): New mail, then the lines
//! (`nav.rs`) as pills with the 6 px place spot, an account's line as its
//! quiet group label, Contacts and Settings at the foot. Narrow
//! (or folded by the user) it is the rail (`.rail`): 40 px tiles with
//! tooltips. One tab stop: Up, Down, Home, End move along the lines,
//! Enter or Space opens one, Right and Left fold and open (TreeView),
//! Shift+F10 or the Menu key opens the line's menu (`commands.rs`).

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Role, SharedString, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::ContextMenuExt as _;
use komorebi_i18n::tr;

use super::nav::{NavItem, NavKind};
use super::rows::DraggedRow;
use super::{MailEvent, MailView, Place, menus, text};
use crate::access::{self, Key};
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, radius, space};
use crate::widgets::button::{Button, Height, IconButton};
use crate::widgets::tooltip::WithTooltip as _;
use crate::widgets::{focus_ring, ring_style};
use komorebi::role::Role as FolderRole;

/// A line's text starts after its 24 px lead (the place spot sits in it).
const LINE_LEAD: f32 = space::SP_6;
/// The place spot's left edge inside a line (mockup: 10 px).
const SPOT_LEFT: f32 = space::SP_2_5;
/// The account label's indent (mockup `.grp`: 22 px).
const GROUP_LEAD: f32 = space::SP_5 + space::SP_0_5;

/// The icon of a line.
pub fn icon(item: &NavItem) -> Icon {
    match (item.kind, item.role) {
        (NavKind::AllInboxes, _) => Icon::Mail,
        (NavKind::Drafts, _) => Icon::Document,
        (NavKind::Outbox, _) => Icon::Send,
        (NavKind::Account(_), _) => Icon::Person,
        (_, Some(FolderRole::Inbox)) => Icon::Mail,
        (_, Some(FolderRole::Drafts)) => Icon::Document,
        (_, Some(FolderRole::Sent)) => Icon::Send,
        (_, Some(FolderRole::Archive)) => Icon::Archive,
        (_, Some(FolderRole::Junk)) => Icon::Warning,
        (_, Some(FolderRole::Trash)) => Icon::Delete,
        _ => Icon::Folder,
    }
}

fn line_id(kind: NavKind, rail: bool) -> ElementId {
    let name = match kind {
        NavKind::AllInboxes => "all".to_string(),
        NavKind::Drafts => "drafts".to_string(),
        NavKind::Outbox => "outbox".to_string(),
        NavKind::Account(id) => format!("a{id}"),
        NavKind::Folder(id) => format!("f{id}"),
    };
    let prefix = if rail { "rail" } else { "nav" };
    ElementId::Name(format!("{prefix}-{name}").into())
}

/// What a screen reader says for a line (the Slint `NavRow`'s label): its
/// name, the account's address, its level, switched off, its unread mail,
/// its total.
fn line_name(item: &NavItem) -> SharedString {
    let mut name = item.label.clone();
    if item.kind != NavKind::AllInboxes && !item.sub.is_empty() && item.sub != item.label {
        name.push_str(", ");
        name.push_str(&item.sub);
    }
    if matches!(item.kind, NavKind::Folder(_)) && item.level > 0 {
        name.push_str(&tr!(", level {0}", item.level));
    }
    if item.off && matches!(item.kind, NavKind::Account(_)) {
        name.push_str(", ");
        name.push_str(&tr!("switched off"));
    }
    if item.unread > 0 {
        name.push_str(", ");
        name.push_str(&tr!("{0} unread", item.unread));
    }
    if item.count > 0 {
        name.push_str(&format!(", {}", item.count));
    }
    name.into()
}

/// A rail tile's name and tooltip: an account tile says its address and
/// its Inbox's unread mail.
fn tile_name(item: &NavItem) -> SharedString {
    match item.kind {
        NavKind::Account(_) if item.unread > 0 => tr!(
            "rail account tooltip" => "{0} - {1} ({2} unread)",
            item.label,
            item.sub,
            item.unread
        )
        .into(),
        NavKind::Account(_) => {
            tr!("rail account tooltip" => "{0} - {1}", item.label, item.sub).into()
        }
        _ => line_name(item),
    }
}

/// A line's name with its caution badge's words after it.
fn with_badge(name: SharedString, badge: Option<&SharedString>) -> SharedString {
    match badge {
        Some(b) => format!("{name}, {b}").into(),
        None => name,
    }
}

/// The caution badge of an account out of reach: the warning glyph in ink
/// (no status hues, BRAND.md); its words are in the line's name, so the
/// button is named for what it does. A click opens the account's page (the
/// Slint NavItem's alert); from the keyboard the line's menu (Shift+F10)
/// has the same Account settings, as the tree's lines are one tab stop.
fn caution(account: i64, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    div()
        .id(ElementId::Name(format!("caution-{account}").into()))
        .role(Role::Button)
        .aria_label(tr!("Account settings"))
        .flex_none()
        .cursor_pointer()
        .on_click(cx.listener(move |_, _, _, cx| {
            cx.stop_propagation();
            cx.emit(MailEvent::EditAccount(account));
        }))
        .child(
            svg()
                .path(Icon::Warning.path())
                .size(px(layout::ICON_SM))
                .text_color(t.color.ink),
        )
        .into_any_element()
}

/// The count at a line's end: unread mail, else a quiet total.
fn count(item: &NavItem, t: &Tokens) -> Option<AnyElement> {
    let (n, colour) = if item.unread > 0 {
        (item.unread, t.color.ink_2)
    } else if item.count > 0 {
        (item.count, t.color.ink_3)
    } else {
        return None;
    };
    Some(
        div()
            .flex_none()
            .typo(&t.text.caption)
            .text_color(colour)
            .child(komorebi::format::thousands(
                komorebi_i18n::current(),
                n as usize,
            ))
            .into_any_element(),
    )
}

pub fn render(
    view: &mut MailView,
    rail: bool,
    window: &mut Window,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let nav = view.nav_items(rail);
    // The keyboard's lines: the full pane's while it is open over the rail.
    let keyboard_here = !(rail && view.pane_open);
    let focused =
        keyboard_here && view.nav_focus.is_focused(window) && window.last_input_was_keyboard();
    let cursor = view.nav_cursor.min(nav.items.len().saturating_sub(1));
    let sets = tree_sets(&nav.items);
    let lines: Vec<AnyElement> = nav
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let at = Line {
                index: i,
                set: sets[i],
                pill: nav.pill == Some(i),
                cursor: keyboard_here && i == cursor,
                ring: focused && i == cursor,
            };
            // An account out of reach carries the caution badge until a
            // sync goes through (the hub's state).
            let badge = match item.kind {
                NavKind::Account(_) => view.badge(item.account).map(SharedString::from),
                _ => None,
            };
            if rail {
                tile(item, at, badge, &t, cx)
            } else if matches!(item.kind, NavKind::Account(_)) {
                group(item, at, badge, &t, cx)
            } else {
                line(item, at, &t, cx)
            }
        })
        .collect();
    let width = if rail { layout::RAIL_W } else { layout::NAV_W };
    div()
        .id(if rail { "folder-rail" } else { "folder-pane" })
        .flex_none()
        .w(px(width))
        .h_full()
        .flex()
        .flex_col()
        .px(px(if rail { space::SP_1 } else { space::SP_2 }))
        .py(px(space::SP_1))
        .when(rail, |el| el.child(pane_toggle(view.pane_open, cx)))
        .child(new_mail(rail, &t, cx))
        .child({
            // While the full pane is open over the rail, it has the focus.
            let lines = div()
                .id(if rail { "rail-lines" } else { "folders" })
                .key_context(super::commands::NAV)
                .role(Role::Tree)
                .aria_label(text::folders())
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap(px(space::SP_0_5))
                .overflow_y_scroll()
                .children(lines);
            if rail && view.pane_open {
                lines.into_any_element()
            } else {
                lines
                    .track_scroll(&view.nav_scroll)
                    .test_support()
                    .track_focus(&view.nav_focus)
                    .into_any_element()
            }
        })
        .child(footer(rail, view.place(), &t, cx))
        .into_any_element()
}

/// The rail's Folders button: the full pane opens over the content (and
/// closes again).
fn pane_toggle(open: bool, cx: &mut Context<MailView>) -> AnyElement {
    let tip = if open {
        tr!("Folders (Close navigation)")
    } else {
        tr!("Folders (Open navigation)")
    };
    div()
        .flex()
        .justify_center()
        .mb(px(space::SP_2))
        .child(
            IconButton::new("pane-toggle", Icon::Navigation, text::folders())
                .tooltip(tip)
                .access_key(Key::Folders)
                .toggled(open)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_pane(window, cx))),
        )
        .into_any_element()
}

/// The full pane over the content, from the rail's Folders button: it
/// closes on a click outside, on Esc, and when a folder opens.
pub fn overlay(view: &mut MailView, window: &mut Window, cx: &mut Context<MailView>) -> AnyElement {
    let c = cx.tokens().color;
    let pane = render(view, false, window, cx);
    gpui::deferred(
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(layout::RAIL_W))
            .flex()
            .child(
                div()
                    .id("folder-overlay")
                    .occlude()
                    .h_full()
                    .bg(c.canvas)
                    .border_r(px(layout::HAIRLINE))
                    .border_color(c.line)
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                        this.close_pane(window, cx);
                    }))
                    .child(pane),
            ),
    )
    .into_any_element()
}

/// New mail: the ink pill (a round ink button in the rail).
fn new_mail(rail: bool, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let open = cx.listener(|this, _, window, cx| this.new_mail(window, cx));
    let c = t.color;
    if rail {
        let pill = div()
            .id("new-mail")
            .role(Role::Button)
            .aria_label(text::new_mail())
            .tab_index(0)
            .size(px(layout::CONTROL_H_LG))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::FULL))
            .bg(c.ink)
            .hover(move |s| s.opacity(crate::theme::motion::INK_HOVER))
            .focus_visible(ring_style(t))
            .on_click(open)
            .child(
                svg()
                    .path(Icon::Compose.path())
                    .size(px(layout::ICON))
                    .text_color(c.on_ink),
            );
        return div()
            .flex()
            .justify_center()
            .mt(px(space::SP_1))
            .mb(px(space::SP_5))
            .child(
                access::decorate(pill, Key::NewMail, cx)
                    .brand_tooltip(text::new_mail_tip())
                    .test_support(),
            )
            .into_any_element();
    }
    div()
        .mt(px(space::SP_1))
        .mx(px(space::SP_1))
        .mb(px(space::SP_5))
        .child(
            Button::new("new-mail")
                .primary()
                .height(Height::Large)
                .icon(Icon::Compose)
                .label(text::new_mail())
                .tooltip(text::new_mail_tip())
                .access_key(Key::NewMail)
                .w_full()
                .on_click(open),
        )
        .into_any_element()
}

/// The foot: Contacts and Settings (no Calendar in 1.0; their views come in later
/// phases; the window hears which was chosen).
fn footer(rail: bool, open: Option<Place>, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let items = [
        (Place::Contacts, Icon::People, text::contacts()),
        (Place::Settings, Icon::Settings, text::settings()),
    ];
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(space::SP_0_5))
        .pt(px(space::SP_2))
        .children(items.into_iter().map(|(place, icon, label)| {
            let id = ElementId::Name(format!("place-{place:?}").into());
            let c = t.color;
            let el = div()
                .id(id)
                .role(Role::Button)
                .aria_label(label.clone())
                .tab_index(0)
                .h(px(scaled::nav_item(t)))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::SP_3_5))
                .when(rail, |el| {
                    el.w(px(layout::CONTROL_H_LG))
                        .mx_auto()
                        .justify_center()
                        .brand_tooltip(label.clone())
                })
                .when(!rail, |el| el.pl(px(LINE_LEAD)).pr(px(space::SP_3_5)))
                .rounded(px(radius::FULL))
                .when(open == Some(place), |el| el.bg(c.wash))
                .hover(move |s| s.bg(c.wash))
                .focus_visible(ring_style(t))
                .typo(&t.text.body)
                .text_color(c.ink)
                // A toggle: UIA reports no selection on a button.
                .aria_toggled(if open == Some(place) {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(MailEvent::Show(place))))
                .child(
                    svg()
                        .path(icon.path())
                        .size(px(layout::ICON))
                        .flex_none()
                        .text_color(c.ink_2),
                )
                .when(!rail, |el| el.child(label));
            let el = match place {
                Place::Settings => access::decorate(el, Key::Settings, cx),
                _ => el,
            };
            el.test_support()
        }))
        .into_any_element()
}

/// The folder a dragged message can be dropped on (folder lines only).
fn drop_folder(kind: NavKind) -> Option<i64> {
    match kind {
        NavKind::Folder(id) => Some(id),
        _ => None,
    }
}

/// A line's place in the tree for screen readers: its level (1 at the
/// top), its position among the lines of the same parent (from 1) and how
/// many they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeSet {
    pub level: usize,
    pub position: usize,
    pub size: usize,
}

/// Each line's [`TreeSet`], from the parent links (the drawn `level` stops
/// at `tree::MAX_LEVEL`; the links do not).
pub fn tree_sets(items: &[NavItem]) -> Vec<TreeSet> {
    let mut sets = vec![TreeSet::default(); items.len()];
    let mut counts: std::collections::HashMap<Option<usize>, usize> = Default::default();
    for (i, item) in items.iter().enumerate() {
        let n = counts.entry(item.parent).or_default();
        *n += 1;
        let level = item
            .parent
            .map_or(1, |p| sets.get(p).map_or(1, |s| s.level + 1));
        sets[i] = TreeSet {
            level,
            position: *n,
            size: 0,
        };
    }
    for (i, item) in items.iter().enumerate() {
        sets[i].size = counts.get(&item.parent).copied().unwrap_or(1);
    }
    sets
}

/// Where a line is and how it shows.
#[derive(Clone, Copy)]
struct Line {
    index: usize,
    /// Its level, position and set size in the tree (screen readers say
    /// "level 2, 3 of 7").
    set: TreeSet,
    /// The open view's line.
    pill: bool,
    /// The keyboard's line (the active descendant while the pane has the
    /// focus).
    cursor: bool,
    /// The keyboard's line while the pane has keyboard focus: the ring.
    ring: bool,
}

/// A line of the full pane.
fn line(item: &NavItem, at: Line, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let Line {
        index: i,
        pill,
        cursor,
        ring,
        ..
    } = at;
    let c = t.color;
    let kind = item.kind;
    let lead = LINE_LEAD + scaled::indent(item.level);
    let fold = item.has_children.then(|| chevron(item, false, t, cx));
    let entity = cx.entity().downgrade();
    let element = div()
        .id(line_id(kind, false))
        .role(Role::TreeItem)
        .aria_label(line_name(item))
        .aria_level(at.set.level)
        .aria_position_in_set(at.set.position)
        .aria_size_of_set(at.set.size)
        .aria_selected(pill)
        .when(item.has_children, |el| el.aria_expanded(item.expanded))
        .relative()
        .h(px(scaled::nav_item(t)))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::SP_3_5))
        .pl(px(lead))
        .pr(px(space::SP_3_5))
        .rounded(px(radius::FULL))
        .typo(&t.text.body)
        .text_color(c.ink)
        .when(item.off, |el| el.opacity(crate::theme::motion::DISABLED))
        .when(pill, |el| el.bg(c.wash))
        .when(!pill, |el| el.hover(move |s| s.bg(c.wash)))
        // A message dragged here moves to this folder.
        .when_some(drop_folder(kind), |el, folder| {
            el.drag_over::<DraggedRow>(move |s, _, _, _| s.bg(c.wash))
                .on_drop(cx.listener(move |this, row: &DraggedRow, window, cx| {
                    this.drop_on_folder(row.id, folder, window, cx);
                }))
        })
        .when(ring, |el| el.shadow(focus_ring(t)))
        .when(cursor, |el| el.aria_active_descendant())
        .when(pill, |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(SPOT_LEFT))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .size(px(layout::PLACE_SPOT))
                            .rounded(px(radius::FULL))
                            .bg(c.ink),
                    ),
            )
        })
        .child(
            svg()
                .path(icon(item).path())
                .size(px(layout::ICON))
                .flex_none()
                .text_color(c.ink_2),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.label.clone()),
        )
        .children(count(item, t))
        .children(fold)
        .when(!item.sub.is_empty(), |el| {
            el.brand_tooltip(item.sub.clone())
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            this.nav_cursor = i;
            this.nav_clicked(kind, false, window, cx);
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, _, cx| {
                this.nav_cursor = i;
                cx.notify();
            }),
        )
        .test_support();
    div()
        .id(ElementId::Name(
            format!("menu-{}", line_id(kind, false)).into(),
        ))
        .flex_none()
        .child(element)
        .context_menu(move |menu, window, cx| {
            menus::nav_menu(entity.clone(), kind, false, menu, window, cx)
        })
        .into_any_element()
}

/// An account's line: its quiet label (mockup `.grp`), folding its tree.
fn group(
    item: &NavItem,
    at: Line,
    badge: Option<SharedString>,
    t: &Tokens,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let Line {
        index: i,
        cursor,
        ring,
        ..
    } = at;
    let c = t.color;
    let kind = item.kind;
    let entity = cx.entity().downgrade();
    let element = div()
        .id(line_id(kind, false))
        .role(Role::TreeItem)
        .aria_label(with_badge(line_name(item), badge.as_ref()))
        .aria_level(at.set.level)
        .aria_position_in_set(at.set.position)
        .aria_size_of_set(at.set.size)
        .aria_expanded(item.expanded)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::SP_2))
        .mt(px(space::SP_1))
        .mb(px(space::SP_2))
        .pl(px(GROUP_LEAD))
        .pr(px(space::SP_3_5))
        .rounded(px(radius::FULL))
        .typo(&t.text.caption)
        .text_color(c.ink_3)
        .when(item.off, |el| el.opacity(crate::theme::motion::DISABLED))
        .when(ring, |el| el.shadow(focus_ring(t)))
        .when(cursor, |el| el.aria_active_descendant())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.label.clone()),
        )
        .when(!item.expanded, |el| el.children(count(item, t)))
        .when(badge.is_some(), |el| el.child(caution(item.account, t, cx)))
        .child(chevron(item, true, t, cx))
        .when(item.sub != item.label, |el| {
            el.brand_tooltip(item.sub.clone())
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            this.nav_cursor = i;
            this.act(window, cx, |m| m.nav_toggle(kind));
        }))
        .test_support();
    div()
        .id(ElementId::Name(
            format!("menu-{}", line_id(kind, false)).into(),
        ))
        .flex_none()
        .child(element)
        .context_menu(move |menu, window, cx| {
            menus::nav_menu(entity.clone(), kind, false, menu, window, cx)
        })
        .into_any_element()
}

/// The fold glyph of a parent line (a click folds or opens it).
fn chevron(item: &NavItem, quiet: bool, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let kind = item.kind;
    let glyph = if item.expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let id = ElementId::Name(format!("fold-{kind:?}").into());
    let paper = t.color.paper;
    div()
        .id(id)
        .size(px(layout::ICON + space::SP_1))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(radius::FULL))
        .hover(move |s| s.bg(paper))
        .child(
            svg()
                .path(glyph.path())
                .size(px(layout::ICON_SM))
                .text_color(if quiet { t.color.ink_3 } else { t.color.ink_2 }),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.act(window, cx, |m| m.nav_toggle(kind));
        }))
        .into_any_element()
}

/// A tile of the rail.
fn tile(
    item: &NavItem,
    at: Line,
    badge: Option<SharedString>,
    t: &Tokens,
    cx: &mut Context<MailView>,
) -> AnyElement {
    let Line {
        index: i,
        pill,
        cursor,
        ring,
        ..
    } = at;
    let c = t.color;
    let kind = item.kind;
    let entity = cx.entity().downgrade();
    let glyph: AnyElement = match kind {
        NavKind::Account(_) => div()
            .typo(&t.text.caption)
            .text_color(c.ink_2)
            .child(crate::widgets::spot::initials(&item.label))
            .into_any_element(),
        _ => svg()
            .path(icon(item).path())
            .size(px(layout::ICON))
            .text_color(if pill { c.ink } else { c.ink_2 })
            .into_any_element(),
    };
    let element = div()
        .id(line_id(kind, true))
        .role(Role::TreeItem)
        .aria_label(with_badge(tile_name(item), badge.as_ref()))
        .aria_level(at.set.level)
        .aria_position_in_set(at.set.position)
        .aria_size_of_set(at.set.size)
        .aria_selected(pill)
        .relative()
        .when_some(drop_folder(kind), |el, folder| {
            el.drag_over::<DraggedRow>(move |s, _, _, _| s.bg(c.wash))
                .on_drop(cx.listener(move |this, row: &DraggedRow, window, cx| {
                    this.drop_on_folder(row.id, folder, window, cx);
                }))
        })
        .flex_none()
        .size(px(layout::CONTROL_H_LG))
        .mx_auto()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius::FULL))
        .when(pill, |el| el.bg(c.wash))
        .when(!pill, |el| el.hover(move |s| s.bg(c.wash)))
        .when(ring, |el| el.shadow(focus_ring(t)))
        .when(cursor, |el| el.aria_active_descendant())
        .when(pill, |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(-(space::SP_1 + layout::PLACE_SPOT / 2.)))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .size(px(layout::PLACE_SPOT))
                            .rounded(px(radius::FULL))
                            .bg(c.ink),
                    ),
            )
        })
        .child(glyph)
        .when(badge.is_some(), |el| {
            el.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .size(px(layout::ICON_SM))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::FULL))
                    .bg(c.canvas)
                    .child(
                        svg()
                            .path(Icon::Warning.path())
                            .size(px(layout::ICON_SM))
                            .text_color(c.ink),
                    ),
            )
        })
        .brand_tooltip(with_badge(tile_name(item), badge.as_ref()))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.nav_cursor = i;
            this.nav_clicked(kind, true, window, cx);
        }))
        .test_support();
    div()
        .id(ElementId::Name(
            format!("menu-{}", line_id(kind, true)).into(),
        ))
        .flex_none()
        .child(element)
        .context_menu(move |menu, window, cx| {
            menus::nav_menu(entity.clone(), kind, true, menu, window, cx)
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(parent: Option<usize>) -> NavItem {
        NavItem {
            kind: NavKind::AllInboxes,
            label: String::new(),
            role: None,
            unread: 0,
            count: 0,
            level: 0,
            has_children: false,
            expanded: false,
            parent,
            account: 0,
            sub: String::new(),
            off: false,
        }
    }

    /// Narrator says "level 2, 2 of 3" from these: each line's depth from
    /// its parent links, and its place among its parent's lines.
    #[test]
    fn tree_lines_know_their_level_and_place() {
        // All inboxes, Drafts, an account with Inbox, Work (and Work's
        // child), Sent.
        let items = vec![
            item(None),
            item(None),
            item(None),
            item(Some(2)),
            item(Some(2)),
            item(Some(4)),
            item(Some(2)),
        ];
        let sets = tree_sets(&items);
        let at = |i: usize| (sets[i].level, sets[i].position, sets[i].size);
        assert_eq!(at(0), (1, 1, 3));
        assert_eq!(at(2), (1, 3, 3));
        assert_eq!(at(3), (2, 1, 3));
        assert_eq!(at(5), (3, 1, 1));
        assert_eq!(at(6), (2, 3, 3));
    }
}
