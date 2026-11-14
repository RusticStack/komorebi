//! The status line of the sync parts: the sentence the hub keeps (a
//! problem, a caveat, a send's end, "Sending in 10 s", a first download's
//! count), drawn as a notice (InfoBar) at the top of the list pane, under
//! the pills, as the mockup's `error` screen has it, with the button its
//! action asks for and a Close pill.

use gpui::{AnyElement, App, Entity, IntoElement, ParentElement, Styled, div, px};

use super::hub::Hub;
use super::sync::{Line, Severity, StatusAction};
use super::{sync_text, text};
use crate::theme::space;
use crate::widgets::button::{Button, Variant};
use crate::widgets::notice::{Kind, Notice};

/// Whether the line can be closed: not the Undo offer (it ends by itself).
/// The standing line of an account out of reach can (FLUENT2 "InfoBar": a
/// bar the user can close); it does not come back until the account syncs.
pub fn dismissable(line: &Line) -> bool {
    !matches!(line.action, StatusAction::UndoSend(_))
}

fn kind(severity: Severity) -> Kind {
    match severity {
        Severity::Info => Kind::Info,
        Severity::Error => Kind::Error,
        Severity::Caution => Kind::Warning,
    }
}

/// The hub's line as a notice, or nothing.
pub fn line(hub: &Entity<Hub>, cx: &App) -> Option<AnyElement> {
    let line = hub.read(cx).sync().line()?.clone();
    let words = sync_text::said(&line.said);
    if words.is_empty() {
        return None;
    }
    let mut notice = Notice::new("sync-status", kind(line.severity), words);
    if let Some(label) = sync_text::action_label(line.action) {
        let hub = hub.clone();
        notice = notice.action(
            Button::new("sync-status-action")
                .variant(Variant::OnInset)
                .label(label)
                .on_click(move |_, _, cx| hub.update(cx, |hub, cx| hub.press_action(cx))),
        );
    }
    if dismissable(&line) {
        let hub = hub.clone();
        notice = notice.action(
            Button::new("sync-status-close")
                .variant(Variant::Subtle)
                .label(text::close())
                .on_click(move |_, _, cx| hub.update(cx, |hub, cx| hub.dismiss(cx))),
        );
    }
    Some(
        div()
            .flex_none()
            .px(px(space::SP_3))
            .pt(px(space::SP_1))
            .pb(px(space::SP_2))
            .child(notice)
            .into_any_element(),
    )
}
