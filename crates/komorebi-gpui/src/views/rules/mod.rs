//! Settings > Rules (docs/GPUI-MIGRATION.md phase 8; the Slint app's
//! `ui/rules.slint` and `src/app/rules.rs`): an account's rules as cards
//! (on and off, order, edit, delete), one rule's page with its conditions
//! and actions, and the activity of the sync worker's automatic runs.
//!
//! What the page does is the library's (`komorebi::rules_page`, shared with
//! the Slint app); this view holds its state, the text fields, and words it
//! (`text.rs`). The settings view mounts it in its content pane through
//! [`RulesView::new`] and calls [`RulesView::reload`] when the section
//! opens; nothing here runs at start. The look is the brand's settings
//! section (mockup `rules-*`): the title and its sentence, New rule at the
//! top right, a card per rule on inset with its switch.

mod editor;
mod list;
mod tests;
pub mod text;

use gpui::{
    App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle, Styled, Subscription, Window,
    div, prelude::*, px,
};
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::rules::{Field, Op, Rule};
use komorebi::rules_page::{Editor, Page, Problem};

use crate::theme::space;
use crate::views::Db;
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::pane::ScrollArea;

/// What the page tells the shell (its status line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesEvent {
    /// The rules went to the clipboard as a Sieve script.
    SieveCopied,
    /// The cache refused a change (the error chain, English).
    Failed(String),
}

/// One rule's page while it is open.
pub(crate) struct RuleForm {
    pub(crate) form: Editor,
    /// The name shown in the page's title: the saved one, or none for a
    /// new rule.
    pub(crate) saved_name: Option<String>,
    name: Entity<InputState>,
    /// A value field per condition, in order.
    values: Vec<Entity<InputState>>,
    pub(crate) problem: Option<Problem>,
    /// The cache refused the save (the error chain).
    pub(crate) failed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

pub struct RulesView {
    db: Db,
    pub(crate) page: Page,
    pub(crate) rule: Option<RuleForm>,
    /// The Sieve script was just copied: the page says so until the next
    /// change.
    pub(crate) copied: bool,
    /// While a card is dragged: the rule dragged (by id) and the card
    /// under the pointer (by place), where the drop indicator line shows.
    pub(crate) drop_at: Option<(i64, usize)>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<RulesEvent> for RulesView {}

impl Focusable for RulesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RulesView {
    /// The page over the window's cache. Empty until [`RulesView::reload`].
    pub fn new(db: Db, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            db,
            page: Page::default(),
            rule: None,
            copied: false,
            drop_at: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    /// Read the shown account's rules and activity again (the section
    /// opened, or a sync ran rules).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.page.reload(&self.db);
        cx.notify();
    }

    /// Whether one rule's page is open (the settings view's Back and Esc).
    pub fn is_editing(&self) -> bool {
        self.rule.is_some()
    }

    /// Made-up rules, a paused one and some activity, for snapshots and the
    /// example (written only with `KOMOREBI_OFFLINE`), and the second
    /// rule's page when `edit`.
    pub fn sample(&mut self, edit: bool, window: &mut Window, cx: &mut Context<Self>) {
        komorebi::rules_page::sample(&self.db);
        self.reload(cx);
        if edit {
            self.edit(1, window, cx);
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.copied = false;
        cx.notify();
    }

    pub fn pick_account(&mut self, index: usize, cx: &mut Context<Self>) {
        self.page.pick_account(&self.db, index);
        self.changed(cx);
    }

    pub fn toggle(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        self.page.set_enabled(&self.db, index, on);
        self.changed(cx);
    }

    pub fn move_rule(&mut self, index: usize, up: bool, cx: &mut Context<Self>) {
        self.page.move_rule(&self.db, index, up);
        self.changed(cx);
    }

    /// A card dropped on another: rule `id` takes the place of rule
    /// `onto`. Both by id, as the list is now (it may have been read again
    /// during the drag); a rule gone since moves nothing. A refused move
    /// says so ([`RulesEvent::Failed`]).
    pub fn drop_rule(&mut self, id: i64, onto: i64, cx: &mut Context<Self>) {
        self.drop_at = None;
        if id != onto
            && let Some(to) = self.page.index_of(onto)
            && self.page.index_of(id).is_some()
            && let Err(e) = self.page.move_rule_to(&self.db, id, to)
        {
            cx.emit(RulesEvent::Failed(format!("{e:#}")));
        }
        self.changed(cx);
    }

    /// Delete asks first (the Slint app's question).
    pub fn ask_delete(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.page.id_at(index) else {
            return;
        };
        let name = self.page.name_of(id);
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: text::delete_question(&name).into(),
                message: text::delete_question_text().into(),
                confirm: text::delete().into(),
                cancel: text::cancel().into(),
                destructive: true,
            },
            move |_, cx| {
                let _ = view.update(cx, |v, cx| v.delete_now(id, cx));
            },
            window,
            cx,
        );
    }

    /// Delete rule `id`, answered.
    pub fn delete_now(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Err(e) = self.page.delete(&self.db, id) {
            cx.emit(RulesEvent::Failed(format!("{e:#}")));
        }
        self.changed(cx);
    }

    pub fn clear_activity(&mut self, cx: &mut Context<Self>) {
        self.page.clear_activity(&self.db);
        self.changed(cx);
    }

    /// The enabled rules as a Sieve script on the clipboard.
    pub fn copy_sieve(&mut self, cx: &mut Context<Self>) {
        let script = self.page.sieve(&self.db);
        cx.write_to_clipboard(ClipboardItem::new_string(script));
        self.copied = true;
        cx.emit(RulesEvent::SieveCopied);
        cx.notify();
    }

    // ---- One rule's page

    /// Open the page of a new rule.
    pub fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_form(Editor::blank(text::new_rule()), None, window, cx);
    }

    /// Open the page of rule `index`.
    pub fn edit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.page.rules.get(index).cloned() else {
            return;
        };
        let form = Editor::open(&rule, &self.page.choices);
        self.open_form(form, Some(rule.name), window, cx);
    }

    fn open_form(
        &mut self,
        form: Editor,
        saved_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| InputState::new(window, cx).default_value(form.name.clone()));
        let mut subscriptions = vec![cx.subscribe_in(
            &name,
            window,
            |this, input, e: &InputEvent, window, cx| match e {
                InputEvent::Change => {
                    let value = input.read(cx).value().to_string();
                    if let Some(r) = this.rule.as_mut() {
                        r.form.name = value;
                    }
                }
                InputEvent::PressEnter { .. } => this.save(window, cx),
                _ => {}
            },
        )];
        let mut values = Vec::new();
        for c in &form.conds {
            let (input, sub) = self.value_field(&c.value, window, cx);
            values.push(input);
            subscriptions.push(sub);
        }
        name.update(cx, |s, cx| s.focus(window, cx));
        self.rule = Some(RuleForm {
            form,
            saved_name,
            name,
            values,
            problem: None,
            failed: None,
            _subscriptions: subscriptions,
        });
        self.copied = false;
        self.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    /// A condition's value field; its edits reach the form by the field's
    /// place in the list (which moves when a row above is removed).
    fn value_field(
        &self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Subscription) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value.to_string())
                .placeholder(text::value())
        });
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, input, e: &InputEvent, window, cx| match e {
                InputEvent::Change => {
                    let value = input.read(cx).value().to_string();
                    if let Some(r) = this.rule.as_mut()
                        && let Some(i) = r.values.iter().position(|v| v == input)
                        && let Some(c) = r.form.conds.get(i).cloned()
                    {
                        r.form.set_cond(i, c.field, c.op, &value);
                    }
                }
                InputEvent::PressEnter { .. } => this.save(window, cx),
                _ => {}
            },
        );
        (input, sub)
    }

    /// Another condition (up to the most a rule holds), its value field
    /// focused.
    pub fn add_cond(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(len) = self.rule.as_ref().map(|r| r.form.conds.len()) else {
            return;
        };
        let (input, sub) = self.value_field("", window, cx);
        let Some(r) = self.rule.as_mut() else {
            return;
        };
        r.form.add_cond();
        if r.form.conds.len() > len {
            input.update(cx, |s, cx| s.focus(window, cx));
            r.values.push(input);
            r._subscriptions.push(sub);
        }
        cx.notify();
    }

    pub fn remove_cond(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(r) = self.rule.as_mut()
            && index < r.values.len()
        {
            r.form.remove_cond(index);
            r.values.remove(index);
            // The name's subscription is first; dropping one ends it.
            drop(r._subscriptions.remove(index + 1));
        }
        cx.notify();
    }

    /// Set condition `index`'s field (its value stays).
    pub fn set_field(&mut self, index: usize, field: Field, cx: &mut Context<Self>) {
        if let Some(r) = self.rule.as_mut()
            && let Some(c) = r.form.conds.get(index).cloned()
        {
            r.form.set_cond(index, field, c.op, &c.value);
        }
        cx.notify();
    }

    pub fn set_op(&mut self, index: usize, op: Op, cx: &mut Context<Self>) {
        if let Some(r) = self.rule.as_mut()
            && let Some(c) = r.form.conds.get(index).cloned()
        {
            r.form.set_cond(index, c.field, op, &c.value);
        }
        cx.notify();
    }

    /// Change the open form (match, actions) and redraw.
    pub fn update_form(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Editor)) {
        if let Some(r) = self.rule.as_mut() {
            f(&mut r.form);
        }
        cx.notify();
    }

    /// Save the open rule; it stays open with the reason when it can't be.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.rule.as_mut() else {
            return;
        };
        let rule: Rule = match r.form.build(&self.page.choices) {
            Ok(rule) => rule,
            Err(problem) => {
                r.problem = Some(problem);
                r.failed = None;
                cx.notify();
                return;
            }
        };
        match self.page.save(&self.db, &rule) {
            Ok(()) => self.close_form(window, cx),
            Err(e) => {
                if let Some(r) = self.rule.as_mut() {
                    r.problem = None;
                    r.failed = Some(format!("{e:#}"));
                }
                cx.notify();
            }
        }
    }

    /// Close the rule's page without saving.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_form(window, cx);
    }

    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rule = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.key == "escape" && !k.modifiers.modified() && self.rule.is_some() {
            self.cancel(window, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for RulesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.rule.is_some() {
            editor::render(self, window, cx)
        } else {
            list::render(self, window, cx)
        };
        div()
            .id("rules")
            .key_context("Rules")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                ScrollArea::new("rules-scroll", &self.scroll).child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(space::SP_4))
                        .px(px(space::SP_8))
                        .pt(px(space::SP_8))
                        .pb(px(space::SP_8))
                        .child(body),
                ),
            )
    }
}
