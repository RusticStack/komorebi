//! The rules list: the title and its sentence with New rule (and Copy as
//! Sieve) at the right, the account picker with two accounts or more, a
//! card per rule on inset (name, summary, the paused line, its switch and
//! its "..." menu), then the activity of automatic runs. A card drags by its
//! grip onto another to take its place, a line in the gap showing where it
//! lands (Move up and Move down in the menu are the keyboard's way).

use std::time::Duration;

use gpui::{
    AnyElement, Context, CursorStyle, DragMoveEvent, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};

use super::{RulesView, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, motion, radius, space};
use crate::widgets::button::{Button, IconButton, Variant};
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::ring_style;
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Toggle;
use crate::widgets::{menu, tooltip};

pub fn render(
    view: &mut RulesView,
    _window: &mut Window,
    cx: &mut Context<RulesView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    // A drag that ended anywhere (dropped outside a card, or cancelled)
    // leaves no indicator behind.
    if !cx.has_active_drag() {
        view.drop_at = None;
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_4))
        .child(header(view, &t, cx))
        .children(account_picker(view, cx))
        .children(view.copied.then(|| {
            Notice::new("rules-copied", Kind::Info, text::sieve_copied()).into_any_element()
        }))
        .children((!view.page.cards.is_empty()).then(|| {
            // The order matters (the first rule runs first), as the mockup
            // says above the cards.
            let said = text::in_order(view.page.cards.len());
            div()
                .id("rules-count")
                .role(Role::Heading)
                .aria_label(said.clone())
                .pt(px(space::SP_2))
                .typo(&t.text.body_strong)
                .text_color(t.color.ink)
                .child(said)
                .test_support()
                .into_any_element()
        }))
        .child(cards(view, &t, cx))
        .children(activity(view, &t, cx))
        .into_any_element()
}

/// "Rules" and its sentence; New rule (and Copy as Sieve once there are
/// rules) at the right.
fn header(view: &RulesView, t: &Tokens, cx: &mut Context<RulesView>) -> AnyElement {
    let add = cx.listener(|this, _, window, cx| this.add(window, cx));
    let copy = cx.listener(|this, _, _, cx| this.copy_sieve(cx));
    div()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap(px(space::SP_4))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(space::SP_1))
                .child(
                    div()
                        .id("rules-title")
                        .role(Role::Heading)
                        .aria_label(text::title())
                        // The mockup's page title: the large step, as the
                        // contact's name.
                        .typo(&t.text.large)
                        .text_color(t.color.ink)
                        .child(text::title())
                        .test_support(),
                )
                .child(
                    div()
                        .typo(&t.text.body)
                        .text_color(t.color.ink_2)
                        .child(text::intro()),
                ),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .gap(px(space::SP_2))
                .when(!view.page.cards.is_empty(), |el| {
                    el.child(
                        Button::new("rules-sieve")
                            .variant(Variant::Subtle)
                            .label(text::copy_sieve())
                            .tooltip(text::copy_sieve_tip())
                            .on_click(copy),
                    )
                })
                .child(
                    Button::new("rules-add")
                        .icon(Icon::Add)
                        .label(text::new_rule())
                        .on_click(add),
                ),
        )
        .into_any_element()
}

/// The account picker, with two accounts or more.
fn account_picker(view: &RulesView, cx: &mut Context<RulesView>) -> Option<AnyElement> {
    if view.page.accounts.len() < 2 {
        return None;
    }
    let current = view.page.account_index().to_string();
    let choices: Vec<(String, String)> = view
        .page
        .accounts
        .iter()
        .enumerate()
        .map(|(i, (_, email))| (i.to_string(), email.clone()))
        .collect();
    let entity = cx.entity().downgrade();
    Some(
        div()
            .flex()
            .child(
                Dropdown::new("rules-account", text::account(), current)
                    .choices(choices)
                    .on_pick(move |v, _, cx| {
                        if let Ok(i) = v.parse::<usize>() {
                            let _ = entity.update(cx, |this, cx| this.pick_account(i, cx));
                        }
                    }),
            )
            .into_any_element(),
    )
}

/// A card per rule, or what an empty list says.
fn cards(view: &RulesView, t: &Tokens, cx: &mut Context<RulesView>) -> AnyElement {
    let cards = &view.page.cards;
    if cards.is_empty() {
        return div()
            .id("rules-empty")
            .typo(&t.text.body)
            .text_color(t.color.ink_2)
            .child(text::none_yet())
            .into_any_element();
    }
    let last = cards.len() - 1;
    let mut list = div()
        .id("rules-list")
        .role(Role::List)
        .aria_label(text::title())
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_1));
    for (i, card) in cards.iter().enumerate() {
        let summary = text::summary(&card.parts);
        let entity = cx.entity().downgrade();
        let open = cx.listener(move |this, _, window, cx| this.edit(i, window, cx));
        let name = SharedString::from(card.name.clone());
        let toggle = {
            let entity = entity.clone();
            move |on: bool, _: &mut Window, cx: &mut gpui::App| {
                let _ = entity.update(cx, |this, cx| this.toggle(i, on, cx));
            }
        };
        let menu_name = name.clone();
        let more = IconButton::new(
            ("rule-more", i),
            Icon::More,
            text::more_actions_named(&card.name),
        )
        .dropdown_menu(move |m, _, cx| {
            let m = menu::brand(m, cx);
            let (e1, e2, e3, e4) = (
                entity.clone(),
                entity.clone(),
                entity.clone(),
                entity.clone(),
            );
            m.item(
                PopupMenuItem::new(text::edit()).on_click(move |_, window, cx| {
                    let _ = e1.update(cx, |this, cx| this.edit(i, window, cx));
                }),
            )
            .item(
                PopupMenuItem::new(text::move_up())
                    .disabled(i == 0)
                    .on_click(move |_, _, cx| {
                        let _ = e2.update(cx, |this, cx| this.move_rule(i, true, cx));
                    }),
            )
            .item(
                PopupMenuItem::new(text::move_down())
                    .disabled(i == last)
                    .on_click(move |_, _, cx| {
                        let _ = e3.update(cx, |this, cx| this.move_rule(i, false, cx));
                    }),
            )
            .separator()
            .item(PopupMenuItem::new(text::delete_named(&menu_name)).on_click(
                move |_, window, cx| {
                    let _ = e4.update(cx, |this, cx| this.ask_delete(i, window, cx));
                },
            ))
        });
        let onto = card.id;
        let drop = cx.listener(move |this, d: &DraggedRule, _, cx| this.drop_rule(d.id, onto, cx));
        // The card under the pointer during a drag (any drag move reaches
        // every card; each keeps only the ones inside its bounds).
        let over = cx.listener(move |this, e: &DragMoveEvent<DraggedRule>, _, cx| {
            let inside = e.bounds.contains(&e.event.position);
            let dragged = e.drag(cx).id;
            if inside && this.drop_at != Some((dragged, i)) {
                this.drop_at = Some((dragged, i));
                cx.notify();
            } else if !inside && this.drop_at.is_some_and(|(_, at)| at == i) {
                this.drop_at = None;
                cx.notify();
            }
        });
        let edge = view
            .drop_at
            .and_then(|(dragged, at)| (at == i).then_some(dragged))
            .and_then(|dragged| view.page.index_of(dragged))
            .and_then(|from| drop_edge(from, i));
        let wash = t.color.wash;
        let card_el = div()
            .id(("rule", i))
            .relative()
            .role(Role::ListItem)
            .aria_label(name.clone())
            .aria_description(summary.clone())
            .aria_position_in_set(i + 1)
            .aria_size_of_set(cards.len())
            .w_full()
            .min_h(px(layout::CARD_H))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_3))
            .pl(px(space::SP_2))
            .pr(px(space::SP_3))
            .py(px(space::SP_3))
            .rounded(px(radius::CARD))
            .bg(t.color.inset)
            // Drag reorder: the grip at the card's start is the handle (so
            // a press on Edit, the switch or "..." never starts a drag);
            // the card under the pointer shows where it lands. The keys'
            // way is the "..." menu's Move up and Move down.
            .drag_over::<DraggedRule>(move |style, _, _, _| style.bg(wash))
            .on_drag_move(over)
            .on_drop(drop)
            .children(edge.map(|edge| drop_line(i, edge, t)))
            .child(grip(i, card.id, name.clone(), t))
            .child(
                // The name and summary open the rule's page: a button named
                // "Edit" and the rule (the Slint card's button), described
                // by the summary, so each card's button has its own name.
                div()
                    .id(("rule-open", i))
                    .role(Role::Button)
                    .aria_label(text::edit_named(&card.name))
                    .aria_description(summary.clone())
                    .tab_index(0)
                    .focus_visible(ring_style(t))
                    .rounded(px(radius::ROW))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_0_5))
                    .cursor_pointer()
                    .on_click(open)
                    .child(
                        div()
                            .typo(&t.text.body_strong)
                            .text_color(t.color.ink)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(t.color.ink_2)
                            .child(summary),
                    )
                    .when(card.paused, |el| {
                        el.child(
                            div()
                                .id(("rule-paused", i))
                                .role(Role::Status)
                                .aria_label(text::paused())
                                .pt(px(space::SP_1))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(space::SP_1))
                                .typo(&t.text.caption)
                                .text_color(t.color.ink)
                                .child(
                                    svg()
                                        .path(Icon::Warning.path())
                                        .size(px(layout::ICON))
                                        .flex_none()
                                        .text_color(t.color.ink),
                                )
                                .child(text::paused())
                                .test_support(),
                        )
                    })
                    .test_support(),
            )
            .child(
                Toggle::new(("rule-toggle", i), name, card.enabled)
                    .state_label(text::on(), text::off())
                    .on_change(toggle),
            )
            .child(more)
            .test_support();
        list = list.child(card_el);
    }
    list.into_any_element()
}

/// Which side of a card the drop indicator line is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Above,
    Below,
}

/// Where a rule dragged from place `from` lands when dropped on the card
/// at `onto`: it takes that card's place, so a rule moving up goes in above
/// it and one moving down below it. On its own card: nowhere.
pub(crate) fn drop_edge(from: usize, onto: usize) -> Option<Edge> {
    match from.cmp(&onto) {
        std::cmp::Ordering::Greater => Some(Edge::Above),
        std::cmp::Ordering::Less => Some(Edge::Below),
        std::cmp::Ordering::Equal => None,
    }
}

/// The drop indicator: a line in the gap above or below card `i`, drawn
/// out of the layout (absolute) so nothing moves while dragging.
fn drop_line(i: usize, edge: Edge, t: &Tokens) -> AnyElement {
    // Centred in the gap between two cards.
    let out = -(space::SP_1 + space::SP_0_5) / 2.;
    div()
        .id(("rule-drop-line", i))
        .absolute()
        .left_0()
        .right_0()
        .h(px(space::SP_0_5))
        .rounded(px(radius::FULL))
        .bg(t.color.accent)
        .map(|el| match edge {
            Edge::Above => el.top(px(out)),
            Edge::Below => el.bottom(px(out)),
        })
        .test_support()
        .into_any_element()
}

/// A card's drag handle: Fluent's vertical re-order dots, the open hand
/// over it, the tooltip and name "Drag to reorder {rule}". Not a tab stop:
/// the menu's Move up and Move down are the keyboard's and Narrator's way
/// to reorder, and the name says what the dots are when Narrator reads the
/// card.
fn grip(i: usize, id: i64, name: SharedString, t: &Tokens) -> AnyElement {
    let said = SharedString::from(text::drag_to_reorder(&name));
    div()
        .id(("rule-grip", i))
        .role(Role::Image)
        .aria_label(said.clone())
        .flex_none()
        .self_stretch()
        .w(px(layout::ICON))
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::OpenHand)
        // The brand tooltip, without its description (the name already
        // says it).
        .tooltip({
            let said = said.clone();
            move |window, cx| tooltip::view(said.clone(), window, cx)
        })
        .tooltip_show_delay(Duration::from_millis(motion::TOOLTIP_DELAY_MS))
        .child(
            svg()
                .path(Icon::ReOrder.path())
                .size(px(layout::ICON))
                .flex_none()
                .text_color(t.color.ink_3),
        )
        .on_drag(DraggedRule { id, name }, |d, _, _, cx| {
            cx.new(|_| d.clone())
        })
        .test_support()
        .into_any_element()
}

/// A rule card being dragged, and what follows the pointer: its name on a
/// card. The rule is named by id, not by its place at the drag's start.
#[derive(Clone)]
pub(crate) struct DraggedRule {
    pub(crate) id: i64,
    name: SharedString,
}

impl Render for DraggedRule {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        div()
            .px(px(space::SP_5))
            .py(px(space::SP_3))
            .rounded(px(radius::CARD))
            .bg(t.color.inset)
            .border(px(layout::HAIRLINE))
            .border_color(t.color.line)
            .typo(&t.text.body_strong)
            .text_color(t.color.ink)
            .child(self.name.clone())
    }
}

/// What the automatic runs did, newest first, with Clear activity.
fn activity(view: &RulesView, t: &Tokens, cx: &mut Context<RulesView>) -> Option<AnyElement> {
    let lines = &view.page.activity;
    if lines.is_empty() {
        return None;
    }
    let clear = cx.listener(|this, _, _, cx| this.clear_activity(cx));
    let header = div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_2))
        .child(
            div()
                .id("rules-activity")
                .role(Role::Heading)
                .aria_label(text::activity())
                .flex_1()
                .typo(&t.text.body_strong)
                .text_color(t.color.ink)
                .child(text::activity())
                .test_support(),
        )
        .child(
            Button::new("rules-clear")
                .variant(Variant::Subtle)
                .label(text::clear_activity())
                .on_click(clear),
        );
    let rows = lines.iter().enumerate().map(|(i, line)| {
        let failed = line.kind == komorebi::rules_page::LogKind::Failed;
        let said = text::log_text(line);
        div()
            .id(("rule-log", i))
            .role(Role::ListItem)
            .aria_label(said.clone())
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(space::SP_2))
            .px(px(space::SP_4))
            .py(px(space::SP_3))
            .rounded(px(radius::CARD))
            .bg(t.color.inset)
            .when(failed, |el| {
                el.child(
                    svg()
                        .path(Icon::Warning.path())
                        .size(px(layout::ICON))
                        .mt(px(space::SP_0_5))
                        .flex_none()
                        .text_color(t.color.ink),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .typo(&t.text.caption)
                            .text_color(t.color.ink_3)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(text::log_title(line)),
                    )
                    .child(div().typo(&t.text.body).text_color(t.color.ink).child(said)),
            )
            .test_support()
    });
    Some(
        div()
            .w_full()
            .pt(px(space::SP_6))
            .flex()
            .flex_col()
            .gap(px(space::SP_1))
            .child(header)
            .child(
                div()
                    .id("rules-log")
                    .role(Role::List)
                    .aria_label(text::activity())
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(space::SP_1))
                    .children(rows),
            )
            .into_any_element(),
    )
}
