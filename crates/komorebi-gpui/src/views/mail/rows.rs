//! A message row (mockup `.row`): the sender spot at its top left (solid
//! ink while unread; a round check while rows can be checked), then three
//! lines: the sender (600 while unread) with its marks and date, the
//! subject (ink while unread, ink-2 once read), the preview (ink-3). Rows
//! are `ListRow`s (16 px corners, wash and the ink marker when selected);
//! the list holds the focus and the keys, and its current row draws the
//! ring and is the active descendant for screen readers.

use gpui::AppContext as _;
use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Styled, div, prelude::*, px, svg,
};
use gpui_kit::component::menu::ContextMenuExt as _;
use komorebi::rows::Row;

use super::{MailEvent, MailView, menus, text};
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{TextStyled as _, Tokens, layout, space};
use crate::widgets::button::IconButton;
use crate::widgets::pane::ListRow;
use crate::widgets::spot::SenderSpot;

/// How a row shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowLook {
    /// Wash and the marker: open, or in the selection.
    pub selected: bool,
    /// Rows show round checks (Select messages, or several selected).
    pub checks: bool,
    /// The list's current row while the list has keyboard focus.
    pub ring: bool,
    /// The list's current row (the active descendant).
    pub cursor: bool,
    /// A server Drafts folder, Drafts or the Outbox: never bold.
    pub plain: bool,
    /// The row shows its quick actions (hovered, or the keyboard's row),
    /// in the views of messages.
    pub quick: bool,
    pub position: usize,
    pub of: usize,
}

/// What a click with these modifiers does to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Open,
    Toggle,
    Extend,
}

/// Ctrl toggles, Shift extends, a plain click opens (and toggles while
/// the check boxes are on, as in Outlook).
pub fn click_kind(modifiers: Modifiers, select_mode: bool) -> Click {
    if modifiers.control || (select_mode && !modifiers.shift) {
        Click::Toggle
    } else if modifiers.shift {
        Click::Extend
    } else {
        Click::Open
    }
}

/// A double or middle click opens a tab on a message only, never on a
/// Drafts or Outbox row (the Slint list's `selected-folder >= 0`).
pub fn opens_tabs(mail: &super::model::Mail) -> bool {
    mail.lists_messages()
}

fn row_id(prefix: &str, id: i64) -> ElementId {
    ElementId::Name(format!("{prefix}-{id}").into())
}

/// The row of message (or draft) `r`, in its slot of the list.
pub fn render(r: &Row, look: RowLook, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    let c = t.color;
    let id = r.id;
    let unread = r.unread && !look.plain;
    let view = cx.entity().downgrade();
    let spot: AnyElement = if look.checks {
        div()
            .id(row_id("check", id))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                window.focus(&this.list_focus, cx);
                this.act(window, cx, |m| m.row_toggled(id));
            }))
            .child(SenderSpot::new(&r.who).check(format!("checkbox-{id}"), look.selected))
            .into_any_element()
    } else {
        SenderSpot::new(&r.who)
            .unread(unread)
            .on_wash(look.selected)
            .into_any_element()
    };
    let name_weight = if unread {
        t.text.body_strong.weight
    } else {
        t.text.body.weight
    };
    // A row's marks: Fluent's 16 px glyphs at their own size (crisp 1 px
    // strokes; at 12 px they were scaled down to faint ones).
    let mark = |icon: Icon| {
        svg()
            .path(icon.at(layout::ICON))
            .size(px(layout::ICON))
            .flex_none()
            .text_color(c.ink_3)
    };
    let line1 = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_2))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_weight(name_weight)
                .text_color(c.ink)
                .child(r.sender.clone()),
        )
        .when(r.draft, |el| {
            el.child(
                div()
                    .flex_none()
                    .typo(&t.text.caption)
                    .font_weight(t.text.body_strong.weight)
                    .text_color(c.ink_2)
                    .child(super::text::draft()),
            )
        })
        .when(r.problem, |el| el.child(mark(Icon::ErrorCircle)))
        // The sender's importance (not the user's Important flag): a bold
        // "!" in ink, and the Down glyph in ink-3 for low (the Slint
        // `MailItem`'s marks; the row's name says them).
        .when(r.high_importance, |el| {
            el.child(
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(c.ink)
                    .child("!"),
            )
        })
        .when(r.low_importance, |el| el.child(mark(Icon::ArrowDown)))
        .when(r.flagged, |el| el.child(mark(Icon::Flag)))
        .when(r.answered, |el| el.child(mark(Icon::Reply)))
        .when(r.attach, |el| el.child(mark(Icon::Attach)))
        .child(
            div()
                .flex_none()
                .typo(&t.text.caption)
                .text_color(c.ink_3)
                .child(r.date.clone()),
        );
    let subject = if r.subject.trim().is_empty() {
        text::no_subject()
    } else {
        r.subject.clone()
    };
    let line2 = div()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_color(if unread { c.ink } else { c.ink_2 })
        .child(subject);
    let line3 = div()
        .mt(px(scaled::PREVIEW_GAP))
        .flex()
        .flex_row()
        .gap(px(space::SP_1))
        .typo(&t.text.preview)
        .text_color(c.ink_3)
        .when(!r.tag.is_empty(), |el| {
            el.child(
                div()
                    .flex_none()
                    .max_w(px(layout::ROW_TAG_MAX_W))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(c.ink_2)
                    .child(r.tag.clone()),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(r.preview.clone()),
        );
    let modifiers_view = view.clone();
    let quick = look.quick.then(|| quick_actions(r, t, cx));
    let row = ListRow::new(row_id("row", id), text::row_name(r))
        .focusable(false)
        .selected(look.selected)
        // A checked row is wash alone; the marker is the open row's.
        .marker(look.selected && !look.checks)
        .ring(look.ring)
        .current(look.cursor)
        .position(look.position, look.of)
        .height(scaled::message_row(t))
        .on_click(move |event, window, cx| {
            let modifiers = event.modifiers();
            let _ = modifiers_view.update(cx, |this, cx| {
                window.focus(&this.list_focus, cx);
                match click_kind(modifiers, this.mail.select_mode()) {
                    Click::Toggle => this.act(window, cx, |m| m.row_toggled(id)),
                    Click::Extend => this.act(window, cx, |m| m.row_extended(id)),
                    Click::Open => this.act(window, cx, |m| m.row_clicked(id)),
                }
                // A double click opens the message in a tab of its own
                // (never a Drafts or Outbox row: those are no messages).
                if event.standard_click()
                    && event.click_count() == 2
                    && matches!(click_kind(modifiers, false), Click::Open)
                    && opens_tabs(&this.mail)
                {
                    cx.emit(MailEvent::OpenTab {
                        id,
                        background: false,
                    });
                }
            });
        })
        .child(
            div()
                .flex_none()
                .self_start()
                .mt(px(scaled::ROW_PAD + space::SP_0_5))
                .child(spot),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .typo(&t.text.body)
                .child(line1)
                .child(line2)
                .child(line3),
        )
        .children(quick);
    let chip = DraggedRow {
        id,
        label: if r.subject.trim().is_empty() {
            text::no_subject()
        } else {
            r.subject.clone()
        },
    };
    div()
        .id(row_id("slot", id))
        // Drag to a folder line (the selection when the row is in it).
        .when(!look.plain, |el| {
            el.on_drag(chip, |chip, _, _, cx| cx.new(|_| chip.clone()))
        })
        .h(px(scaled::message_slot(t)))
        .pb(px(scaled::ROW_GAP))
        .px(px(space::SP_2))
        // A middle click opens the message in a tab behind this one.
        .on_mouse_up(
            MouseButton::Middle,
            cx.listener(move |this, _, _, cx| {
                if opens_tabs(&this.mail) {
                    cx.emit(MailEvent::OpenTab {
                        id,
                        background: true,
                    });
                }
            }),
        )
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            let was = this.hovered_row;
            if *hovered {
                this.hovered_row = Some(id);
            } else if this.hovered_row == Some(id) {
                this.hovered_row = None;
            }
            if was != this.hovered_row {
                // Only the list shows the row's quick actions: the folder
                // pane and the reading pane stay as they were drawn.
                this.notify_list(cx);
            }
        }))
        .child(
            div()
                .id(row_id("menu", id))
                .size_full()
                .child(row)
                .context_menu(move |menu, window, cx| {
                    menus::row_menu(view.clone(), id, menu, window, cx)
                }),
        )
        .into_any_element()
}

/// A row's quick actions (the Slint `MailItem`'s): Archive, Delete, Mark
/// as important, Mark as read, on that row only (never opened, so never
/// read by opening), over the row's right end.
fn quick_actions(r: &Row, t: &Tokens, cx: &mut Context<MailView>) -> AnyElement {
    use super::text::{Command, command, command_tip};
    use super::triage::Quick;
    let id = r.id;
    let button =
        |slug: &str, icon: Icon, cmd: Command, action: Quick, cx: &mut Context<MailView>| {
            // No Tab stops: Tab from the row goes on to the reading pane
            // (test row 164); the row's keys run the same commands.
            IconButton::new(row_id(slug, id), icon, command(cmd))
                .small()
                .no_tab_stop()
                .tooltip(command_tip(cmd))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.act(window, cx, |m| m.quick_action(id, action));
                }))
        };
    let flag = if r.flagged {
        Command::Unflag
    } else {
        Command::Flag
    };
    let read = if r.unread {
        Command::MarkRead
    } else {
        Command::MarkUnread
    };
    let read_icon = if r.unread { Icon::MailRead } else { Icon::Mail };
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(space::SP_2))
        .flex()
        .flex_row()
        .items_center()
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(space::SP_0_5))
                .p(px(space::SP_0_5))
                .rounded(px(crate::theme::radius::FULL))
                .bg(t.color.paper)
                .child(button(
                    "quick-archive",
                    Icon::Archive,
                    Command::Archive,
                    Quick::Archive,
                    cx,
                ))
                .child(button(
                    "quick-delete",
                    Icon::Delete,
                    Command::Delete,
                    Quick::Delete,
                    cx,
                ))
                .child(button("quick-flag", Icon::Flag, flag, Quick::Flag, cx))
                .child(button("quick-read", read_icon, read, Quick::Read, cx)),
        )
        .into_any_element()
}

/// A message row being dragged to a folder: its id (the drop moves the
/// selection when the row is in it) and what the chip under the pointer
/// says.
#[derive(Clone, Debug)]
pub struct DraggedRow {
    pub id: i64,
    pub label: String,
}

impl gpui::Render for DraggedRow {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app: &gpui::App = cx;
        let t = crate::theme::ActiveTokens::tokens(app).clone();
        div()
            .px(px(space::SP_3))
            .py(px(space::SP_1))
            .max_w(px(layout::ROW_TAG_MAX_W))
            .rounded(px(crate::theme::radius::FULL))
            .bg(t.color.paper)
            .border_1()
            .border_color(t.color.line)
            .typo(&t.text.caption)
            .text_color(t.color.ink)
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(self.label.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Slint list's clicks (`message-toggled`, `message-extended`,
    /// `message-selected`).
    #[test]
    fn modifiers_choose_what_a_click_does() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(click_kind(Modifiers::default(), false), Click::Open);
        assert_eq!(click_kind(ctrl, false), Click::Toggle);
        assert_eq!(click_kind(shift, false), Click::Extend);
        // With the check boxes on, a click checks the row; Shift still
        // selects a range.
        assert_eq!(click_kind(Modifiers::default(), true), Click::Toggle);
        assert_eq!(click_kind(shift, true), Click::Extend);
    }
}
