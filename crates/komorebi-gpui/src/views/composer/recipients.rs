//! To, Cc and Bcc (mockup `.fld` with `.chip`s): the draft's field text is
//! the model, as in the Slint app (`komorebi::compose::recipients`). The
//! entries before the last separator show as chips; what follows it is the
//! Kit field's text, the address being typed. A typed `,` or `;` makes a
//! chip, Enter makes one of the address typed (or takes the highlighted
//! suggestion), Backspace in an empty field removes the last chip, and a
//! chip's dismiss button removes it. Suggestions come from the address
//! index (`komorebi::compose::suggest`) under the field typed in.

use gpui::{
    AppContext as _, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement, Role, SharedString, Styled, Window, deferred, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{Backspace, InputEvent, InputState, MoveDown, MoveUp};
use komorebi::compose::{self, Field, recipients};
use komorebi::db::contacts::SuggestKind;

use super::{ComposerView, text};
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo, layout, radius, space};
use crate::widgets::field::{ChipEntry, ChipField};
use crate::widgets::spot::SenderSpot;

/// One address field's Kit input (the address being typed).
pub struct RecipientInput {
    pub field: Field,
    pub input: Entity<InputState>,
}

impl RecipientInput {
    pub fn new(field: Field, window: &mut Window, cx: &mut Context<ComposerView>) -> Self {
        RecipientInput {
            field,
            input: cx.new(|cx| InputState::new(window, cx)),
        }
    }
}

fn label(field: Field) -> String {
    match field {
        Field::Cc => text::words::cc(),
        Field::Bcc => text::words::bcc(),
        _ => text::words::to(),
    }
}

impl ComposerView {
    /// An event of the address field `field`'s input.
    pub(crate) fn recipient_event(
        &mut self,
        field: Field,
        e: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.filling {
            return;
        }
        match e {
            InputEvent::Change => self.recipients_typed(field, window, cx),
            InputEvent::PressEnter {
                secondary: false, ..
            } => {
                if self
                    .drafts
                    .suggest
                    .as_ref()
                    .is_some_and(|s| s.field == field)
                {
                    self.accept_suggestion(self.suggest_index, window, cx);
                } else {
                    self.commit_recipient(field, window, cx);
                }
            }
            _ => {}
        }
    }

    /// The focus left address field `field`: its addresses are checked.
    pub(crate) fn recipient_left(&mut self, field: Field, cx: &mut Context<Self>) {
        if let Some(d) = self.drafts.active_mut() {
            d.validate(field);
        }
        cx.notify();
    }

    /// The draft's text of `field` with `tail` as the address typed.
    fn recipients_typed(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.input(field).map(|i| i.input.clone()) else {
            return;
        };
        let tail = input.read(cx).value().to_string();
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        let old = d.field(field).to_string();
        let full = format!("{}{tail}", recipients::head(&old));
        if full == old {
            return;
        }
        let cursor = compose::caret_after_edit(&old, &full);
        d.edit(field, &full);
        // A separator typed: what is before it is a chip now.
        let rest = recipients::tail(&full).to_string();
        if rest != tail {
            self.filling = true;
            input.update(cx, |s, cx| s.set_value(rest, window, cx));
            self.filling = false;
        }
        self.suggest(field, cursor, cx);
        self.edited(cx);
    }

    /// Enter: the address typed becomes a chip.
    fn commit_recipient(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        let old = d.field(field).to_string();
        let new = recipients::commit(&old);
        if new == old {
            return;
        }
        d.edit(field, &new);
        self.set_tail(field, &new, window, cx);
        self.edited(cx);
    }

    /// Chip `index` of `field` removed (its button, or Backspace).
    pub(crate) fn remove_recipient(
        &mut self,
        field: Field,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        let old = d.field(field).to_string();
        let new = recipients::remove(&old, index);
        if new == old {
            return;
        }
        d.edit(field, &new);
        self.set_tail(field, &new, window, cx);
        self.edited(cx);
    }

    /// Backspace in an empty field: the last chip goes.
    fn backspace_on_empty(
        &mut self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.input(field).map(|i| i.input.clone()) else {
            return false;
        };
        if !input.read(cx).value().is_empty() {
            return false;
        }
        let count = self.drafts.active().map_or(0, |d| {
            recipients::tokens(recipients::head(d.field(field))).len()
        });
        if count == 0 {
            return false;
        }
        self.remove_recipient(field, count - 1, window, cx);
        true
    }

    /// The field's input shows the text being typed of `full`.
    fn set_tail(&mut self, field: Field, full: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.input(field).map(|i| i.input.clone()) else {
            return;
        };
        let tail = recipients::tail(full).to_string();
        self.filling = true;
        input.update(cx, |s, cx| s.set_value(tail, window, cx));
        self.filling = false;
    }

    /// Suggestions for the address at `cursor` of `field` (none: hidden).
    fn suggest(&mut self, field: Field, cursor: usize, cx: &mut Context<Self>) {
        let shown = self
            .drafts
            .active()
            .and_then(|d| compose::suggest(self.db(), d, field, cursor));
        self.drafts.suggest = shown;
        self.suggest_index = 0;
        cx.notify();
    }

    pub(crate) fn close_suggestions(&mut self, cx: &mut Context<Self>) {
        if self.drafts.suggest.take().is_some() {
            cx.notify();
        }
    }

    fn step_suggestion(&mut self, down: bool, cx: &mut Context<Self>) {
        let n = self.drafts.suggest.as_ref().map_or(0, |s| s.items.len());
        if n == 0 {
            return;
        }
        self.suggest_index = if down {
            (self.suggest_index + 1) % n
        } else {
            (self.suggest_index + n - 1) % n
        };
        cx.notify();
    }

    /// Take suggestion `index`: it replaces the address being typed, as
    /// `Name <addr>, ` (a chip), and the caret stays in the field.
    pub(crate) fn accept_suggestion(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.drafts.suggest.take() else {
            return;
        };
        let Some(d) = self.drafts.active_mut() else {
            return;
        };
        let Some((new, _caret)) = compose::accept_suggestion(d, &s, index) else {
            return;
        };
        d.edit(s.field, &new);
        self.set_tail(s.field, &new, window, cx);
        self.focus_field(s.field, window, cx);
        self.edited(cx);
    }

    /// The row of address field `field`: its label, chips and input, the
    /// inline error, and (To, while Cc and Bcc are hidden) "Cc Bcc".
    pub(crate) fn render_recipients(
        &mut self,
        field: Field,
        offer_cc: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let Some(input) = self.input(field).map(|i| i.input.clone()) else {
            return div().into_any_element();
        };
        let (value, error, need) = self.drafts.active().map_or_else(Default::default, |d| {
            (
                d.field(field).to_string(),
                d.error(field).map(str::to_string),
                field == Field::To && d.need_recipient,
            )
        });
        let view = cx.entity().downgrade();
        let chips = recipients::tokens(recipients::head(&value))
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let view = view.clone();
                ChipEntry {
                    id: SharedString::from(format!("{}-chip-{i}", id_of(field))),
                    label: t.label.clone().into(),
                    remove_name: text::words::remove(&t.label).into(),
                    valid: t.valid,
                    on_remove: std::rc::Rc::new(move |window, cx| {
                        let _ =
                            view.update(cx, |this, cx| this.remove_recipient(field, i, window, cx));
                    }),
                }
            })
            .collect();
        let error = error
            .map(|a| text::address_invalid(&a))
            .or_else(|| need.then(text::add_recipient));
        let mut row = ChipField::new(&input, label(field))
            .id(id_of(field))
            .chips(chips)
            .expanded(
                self.drafts
                    .suggest
                    .as_ref()
                    .is_some_and(|s| s.field == field && !s.items.is_empty()),
            )
            .error(error.map(SharedString::from));
        if offer_cc {
            let cc_view = view.clone();
            row = row.trailing(
                SharedString::from(format!("{}  {}", text::words::cc(), text::words::bcc())),
                SharedString::from(text::words::add_cc()),
                move |window, cx| {
                    let _ = cc_view.update(cx, |this, cx| this.reveal_cc(Field::Cc, window, cx));
                },
            );
        }
        let shows = self
            .drafts
            .suggest
            .as_ref()
            .is_some_and(|s| s.field == field);
        let list = if shows {
            self.render_suggestions(cx)
        } else {
            None
        };
        let keys = cx.entity().downgrade();
        let up = keys.clone();
        let back = keys.clone();
        div()
            .relative()
            .flex_none()
            .ml(px(geo::INSET_LEFT))
            .mr(px(geo::INSET_RIGHT))
            .when(shows, |el| {
                let down = keys.clone();
                el.capture_action(move |_: &MoveDown, _, cx| {
                    let _ = down.update(cx, |this, cx| this.step_suggestion(true, cx));
                    cx.stop_propagation();
                })
                .capture_action(move |_: &MoveUp, _, cx| {
                    let _ = up.update(cx, |this, cx| this.step_suggestion(false, cx));
                    cx.stop_propagation();
                })
            })
            .capture_action(move |_: &Backspace, window, cx| {
                let removed = back
                    .update(cx, |this, cx| this.backspace_on_empty(field, window, cx))
                    .unwrap_or(false);
                if removed {
                    cx.stop_propagation();
                }
            })
            .child(row)
            .children(list)
            .into_any_element()
    }

    /// The suggestions under the field typed in, over what follows it.
    fn render_suggestions(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let s = self.drafts.suggest.as_ref()?;
        let t = cx.tokens().clone();
        let c = t.color;
        let count = s.items.len();
        let items: Vec<_> =
            s.items
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let on = i == self.suggest_index;
                    let name = match &r.kind {
                        SuggestKind::Group(m) => format!("{} ({})", r.name, m.len()),
                        _ if r.name.trim().is_empty() => r.addr.clone(),
                        _ => r.name.clone(),
                    };
                    let aria = SharedString::from(format!("{name}, {}", r.addr));
                    div()
                        .id(ElementId::Name(format!("compose-suggestion-{i}").into()))
                        // Options of a list box: the one Up and Down
                        // highlight is selected, so UI Automation raises
                        // ElementSelected and Narrator reads it while the
                        // focus stays in the address field.
                        .role(Role::ListBoxOption)
                        .aria_label(aria)
                        .aria_selected(on)
                        .aria_position_in_set(i + 1)
                        .aria_size_of_set(count)
                        .h(px(scaled::control(&t, geo::SUGGEST_ROW_H)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(space::SP_2_5))
                        .px(px(space::SP_3))
                        .rounded(px(radius::ROW))
                        .when(on, |el| el.bg(c.wash))
                        .hover(move |s| s.bg(c.wash))
                        .child(SenderSpot::new(&name).size(layout::AVATAR_SM).on_wash(on))
                        .child(div().text_color(c.ink).child(name.clone()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(c.ink_3)
                                .typo(&t.text.caption)
                                .child(r.addr.clone()),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.accept_suggestion(i, window, cx)
                        }))
                        .test_support()
                })
                .collect();
        Some(deferred(
            div()
                .id("compose-suggestions")
                .role(Role::ListBox)
                .aria_label(SharedString::from(text::words::suggested_recipients()))
                .absolute()
                .top_full()
                .left(px(layout::FIELD_LABEL_W + geo::FIELD_GAP))
                .w(px(geo::SUGGEST_W))
                .p(px(space::SP_1))
                .bg(c.paper)
                .border(px(layout::HAIRLINE))
                .border_color(c.line)
                .rounded(px(radius::CARD))
                .typo(&t.text.body)
                .occlude()
                .children(items)
                .test_support(),
        ))
    }
}

/// The element id of an address field's row.
fn id_of(field: Field) -> &'static str {
    match field {
        Field::Cc => "compose-cc",
        Field::Bcc => "compose-bcc",
        _ => "compose-to",
    }
}
