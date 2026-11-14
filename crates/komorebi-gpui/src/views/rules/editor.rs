//! One rule's page: Back and the title, the name, the conditions (All or
//! Any, then a row per condition: field, operator, value, remove), the
//! actions (Move to folder with its folder, Delete, Mark as read, Mark as
//! important, Stop processing more rules), what is wrong, Save and Cancel.

use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, ParentElement, Role, SharedString, Styled,
    WeakEntity, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::rules::{Field, MAX_CONDITIONS, Op};
use komorebi::rules_page::Editor;

use super::{RuleForm, RulesView, text};
use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, Tokens, layout, space};
use crate::widgets::button::{Button, IconButton};
use crate::widgets::field::TextField;
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Checkbox;

type Weak = WeakEntity<RulesView>;

pub fn render(
    view: &mut RulesView,
    _window: &mut Window,
    cx: &mut Context<RulesView>,
) -> AnyElement {
    let t = cx.tokens().clone();
    let entity = cx.entity().downgrade();
    let Some(rule) = view.rule.as_ref() else {
        return div().into_any_element();
    };
    let back = cx.listener(|this, _, window, cx| this.cancel(window, cx));
    let save = cx.listener(|this, _, window, cx| this.save(window, cx));
    let cancel = cx.listener(|this, _, window, cx| this.cancel(window, cx));
    let title = rule.saved_name.clone().unwrap_or_else(text::new_rule);
    let problem = rule
        .failed
        .as_deref()
        .map(text::detail)
        .or_else(|| rule.problem.map(text::problem));
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_6))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(space::SP_2))
                .child(IconButton::new("rule-back", Icon::ChevronLeft, text::back()).on_click(back))
                .child(
                    div()
                        .id("rule-title")
                        .role(Role::Heading)
                        .aria_label(title.clone())
                        .flex_1()
                        .min_w_0()
                        .typo(&t.text.subtitle)
                        .text_color(t.color.ink)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(title)
                        .test_support(),
                ),
        )
        .child(group(
            &t,
            text::rule_name(),
            div()
                .w_full()
                .max_w(px(layout::DIALOG_W))
                .child(TextField::new(&rule.name, text::rule_name())),
        ))
        .child(conditions(rule, &entity, &t))
        .child(actions(view, rule, &entity, &t))
        .children(problem.map(|p| Notice::new("rule-problem", Kind::Error, p)))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(space::SP_2))
                .child(
                    Button::new("rule-save")
                        .primary()
                        .label(text::save())
                        .disabled(rule.form.conds.is_empty())
                        .on_click(save),
                )
                .child(
                    Button::new("rule-cancel")
                        .label(text::cancel())
                        .on_click(cancel),
                ),
        )
        .into_any_element()
}

/// A labelled group of the page: its header (Body Strong), then `body`.
fn group(t: &Tokens, header: String, body: impl IntoElement) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(space::SP_2))
        .child(
            div()
                .typo(&t.text.body_strong)
                .text_color(t.color.ink)
                .child(header),
        )
        .child(body)
        .into_any_element()
}

/// A click handler that runs `f` on the view.
fn on_click(
    entity: &Weak,
    f: impl Fn(&mut RulesView, &mut Window, &mut Context<RulesView>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = entity.clone();
    move |_, window, cx| {
        let _ = entity.update(cx, |this, cx| f(this, window, cx));
    }
}

/// All or Any, a row per condition, Add condition.
fn conditions(rule: &RuleForm, entity: &Weak, t: &Tokens) -> AnyElement {
    let form = &rule.form;
    let picked = entity.clone();
    let matching = Dropdown::new(
        "rule-match",
        text::conditions(),
        if form.match_any { "1" } else { "0" },
    )
    .choices([("0", text::all_must_match()), ("1", text::any_can_match())])
    .on_pick(move |v, _, cx| {
        let on = v.as_ref() == "1";
        let _ = picked.update(cx, |this, cx| this.update_form(cx, |f| f.match_any = on));
    });
    let rows = form
        .conds
        .iter()
        .zip(&rule.values)
        .enumerate()
        .map(|(i, (c, input))| {
            let field_pick = entity.clone();
            let op_pick = entity.clone();
            let n = i + 1;
            let field = Dropdown::new(
                SharedString::from(format!("rule-field-{i}")),
                text::condition_n(n),
                c.field.code().to_string(),
            )
            .choices(
                Field::ALL
                    .iter()
                    .map(|f| (f.code().to_string(), text::field(*f))),
            )
            .on_pick(move |v, _, cx| {
                if let Some(f) = v.parse::<i64>().ok().and_then(Field::from_code) {
                    let _ = field_pick.update(cx, |this, cx| this.set_field(i, f, cx));
                }
            });
            let op = Dropdown::new(
                SharedString::from(format!("rule-op-{i}")),
                text::condition_op_n(n),
                c.op.code().to_string(),
            )
            .choices(Op::ALL.iter().map(|o| (o.code().to_string(), text::op(*o))))
            .on_pick(move |v, _, cx| {
                if let Some(o) = v.parse::<i64>().ok().and_then(Op::from_code) {
                    let _ = op_pick.update(cx, |this, cx| this.set_op(i, o, cx));
                }
            });
            let has_value = c.field != Field::HasAttachment;
            div()
                .id(("rule-cond", i))
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(space::SP_2))
                .child(field)
                .when(has_value, |el| el.child(op))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(layout::LIST_W_COMPACT))
                        .when(has_value, |el| {
                            el.child(TextField::new(input, text::condition_value_n(n)))
                        }),
                )
                .child(
                    IconButton::new(
                        ("rule-cond-remove", i),
                        Icon::Delete,
                        text::remove_condition_n(n),
                    )
                    .on_click(on_click(entity, move |this, _, cx| this.remove_cond(i, cx))),
                )
        });
    group(
        t,
        text::conditions(),
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(space::SP_2))
            .child(div().flex().child(matching))
            .children(rows)
            .child(
                div().flex().child(
                    Button::new("rule-add-cond")
                        .icon(Icon::Add)
                        .label(text::add_condition())
                        .disabled(form.conds.len() >= MAX_CONDITIONS)
                        .on_click(on_click(entity, |this, window, cx| {
                            this.add_cond(window, cx)
                        })),
                ),
            ),
    )
}

/// One action's checkbox, wired to the form.
fn check(
    id: &'static str,
    label: String,
    on: bool,
    entity: &Weak,
    set: fn(&mut Editor, bool),
) -> Checkbox {
    let entity = entity.clone();
    Checkbox::new(id, label, on).on_change(move |v, _, cx| {
        let _ = entity.update(cx, |this, cx| this.update_form(cx, |f| set(f, v)));
    })
}

/// "Do this": the actions and Stop processing more rules. Move to folder
/// and Delete exclude each other (the library's `choose_move`).
fn actions(view: &RulesView, rule: &RuleForm, entity: &Weak, t: &Tokens) -> AnyElement {
    let form = &rule.form;
    let choices = &view.page.choices;
    let folder = form.move_on.then(|| {
        let picked = entity.clone();
        Dropdown::new(
            "rule-folder",
            text::move_to_folder(),
            form.move_to.to_string(),
        )
        .choices(choices.iter().enumerate().map(|(i, c)| {
            // An em space per level stands in for the tree's indent.
            let pad = "\u{2003}".repeat(c.depth);
            (i.to_string(), format!("{pad}{}", c.display))
        }))
        .on_pick(move |v, _, cx| {
            if let Ok(i) = v.parse::<usize>() {
                let _ = picked.update(cx, |this, cx| this.update_form(cx, |f| f.move_to = i));
            }
        })
    });
    let move_check = check(
        "rule-act-move",
        text::move_to_folder(),
        form.move_on,
        entity,
        Editor::choose_move,
    )
    .disabled(choices.is_empty());
    group(
        t,
        text::do_this(),
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(space::SP_2))
            .child(
                div()
                    .min_h(px(layout::CONTROL_H))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(space::SP_3))
                    .child(move_check)
                    .children(folder),
            )
            .child(check(
                "rule-act-delete",
                text::delete(),
                form.delete,
                entity,
                Editor::choose_delete,
            ))
            .child(check(
                "rule-act-read",
                text::mark_read(),
                form.read,
                entity,
                |f, v| f.read = v,
            ))
            .child(check(
                "rule-act-flag",
                text::mark_important(),
                form.flag,
                entity,
                |f, v| f.flag = v,
            ))
            .child(check(
                "rule-act-stop",
                text::stop(),
                form.stop,
                entity,
                |f, v| f.stop = v,
            )),
    )
}
