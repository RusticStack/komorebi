//! The Outbox entry's pane (docs/GPUI-MIGRATION.md phase 2, the sync
//! parts; the Slint app's `src/app/outbox.rs` and the `o-show` pane): a
//! message waiting to be sent, or one whose send failed, with its state in
//! words, the server's reply behind Details, its first lines, and Edit,
//! Retry and Discard. A failure sending again can't fix leads with Edit.

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Role, Styled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::db::{Outbox, OutboxEntry};
use komorebi_i18n::tr;

use super::MailView;
use super::model::View;
use crate::icons::Icon;
use crate::theme::{TextStyled as _, Tokens, layout, space};
use crate::widgets::button::{Button, Height, IconButton, Variant};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::notice::{Kind, Notice};

/// Lines of the text shown under the state (the Slint app's).
const EXCERPT_LINES: usize = 3;
const EXCERPT_CHARS: usize = 280;

/// The first lines of what was written (formatting marks taken out, no
/// quote or signature).
pub fn excerpt(body: &str) -> String {
    let plain = komorebi::mail::compose_html::plain(body);
    let lines: Vec<&str> = plain
        .lines()
        .take_while(|l| *l != "-- " && !l.trim_start().starts_with('>'))
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .take(EXCERPT_LINES)
        .collect();
    let text = lines.join("\n");
    match text.char_indices().nth(EXCERPT_CHARS) {
        Some((i, _)) => format!("{}\u{2026}", text[..i].trim_end()),
        None => text,
    }
}

/// What the pane offers for an entry in this state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offers {
    pub edit: bool,
    pub retry: bool,
    pub discard: bool,
    /// A refusal that sending again can't fix: Edit leads.
    pub permanent: bool,
    /// Failed or unconfirmed: the state is a problem.
    pub problem: bool,
}

pub fn offers(e: &OutboxEntry) -> Offers {
    use Outbox::*;
    Offers {
        edit: e.state != Sending && !e.invite_reply,
        retry: matches!(e.state, Failed | Unconfirmed),
        discard: matches!(e.state, Queued | Failed | Unconfirmed),
        permanent: e.state == Failed && komorebi::mail::smtp::classify(&e.last_error).permanent(),
        problem: matches!(e.state, Failed | Unconfirmed),
    }
}

impl MailView {
    /// The Outbox row shown in the reading pane: the one row selected.
    pub fn outbox_open(&self) -> Option<i64> {
        if self.mail.view() != View::Outbox {
            return None;
        }
        match self.mail.selection().ids()[..] {
            [row] => Some(row),
            _ => None,
        }
    }

    fn outbox_entry(&self) -> Option<(i64, OutboxEntry)> {
        let row = self.outbox_open()?;
        let entry = self.mail.db().outbox_entry(row).ok().flatten()?;
        Some((row, entry))
    }

    /// Edit: the entry goes back to Drafts and opens in the composer.
    pub fn outbox_edit(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |c, cx| c.undo_send(row, window, cx));
        self.outbox_changed(window, cx);
    }

    /// Retry now; one that may have gone out asks first.
    pub fn outbox_retry(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        let entry = match self.mail.db().outbox_entry(row) {
            Ok(Some(e)) => e,
            Ok(None) => return self.outbox_changed(window, cx),
            Err(e) => return self.say_detail(&e, cx),
        };
        if entry.state != Outbox::Unconfirmed {
            return self.retry_now(row, window, cx);
        }
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: tr!("Send this message again?").into(),
                message: tr!(
                    "It may already have been sent. Check your Sent folder first, or the recipients may get it twice."
                )
                .into(),
                confirm: tr!("Send again").into(),
                cancel: super::text::cancel().into(),
                destructive: false,
            },
            move |window, cx| {
                let _ = view.update(cx, |this, cx| this.retry_now(row, window, cx));
            },
            window,
            cx,
        );
    }

    pub fn retry_now(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.mail.db().outbox_retry(row) {
            Ok(true) => {
                if let Ok(Some(e)) = self.mail.db().outbox_entry(row) {
                    self.hub.read(cx).dispatch(row, e.account_id);
                }
            }
            Ok(false) => {}
            Err(e) => self.say_detail(&e, cx),
        }
        self.outbox_changed(window, cx);
    }

    /// Discard, after asking.
    pub fn outbox_discard(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: tr!("Discard this message?").into(),
                message: tr!(
                    "It is deleted here and from your Drafts folder on the server, with its attachments."
                )
                .into(),
                confirm: tr!("Discard").into(),
                cancel: super::text::cancel().into(),
                destructive: true,
            },
            move |window, cx| {
                let _ = view.update(cx, |this, cx| this.discard_outbox_now(row, window, cx));
            },
            window,
            cx,
        );
    }

    pub fn discard_outbox_now(&mut self, row: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.mail.db().discard_outbox(row) {
            Ok((true, copy)) => {
                self.hub.update(cx, |hub, cx| {
                    hub.remove_server_copy(copy);
                    hub.say(super::sync::Said::Discarded, false, cx);
                });
            }
            Ok((false, _)) => {}
            Err(e) => self.say_detail(&e, cx),
        }
        self.outbox_changed(window, cx);
    }

    /// After the first frame (the Slint `resume_outbox`): a Send cut short
    /// by the last exit left its row in the undo window. It goes now (its
    /// message built again if it was not stored; a draft that cannot be
    /// built goes back to Drafts and the line says so), then whatever is
    /// due goes to the send workers.
    pub fn resume_outbox(&mut self, cx: &mut Context<Self>) {
        let db = self.mail.db().clone();
        let pending = match db.outbox_pending() {
            Ok(rows) => rows,
            Err(e) => return self.say_detail(&e, cx),
        };
        let busy = self.composer.read(cx).pending_sends();
        let mut back_to_drafts = false;
        for (row, built) in pending {
            if busy.contains(&row) {
                continue;
            }
            if built {
                if let Err(e) = db.outbox_queue(row) {
                    self.say_detail(&e, cx);
                }
                continue;
            }
            let rebuilt = komorebi::compose::load(&db, row)
                .ok()
                .flatten()
                .filter(|(_, missing)| missing.is_empty())
                .and_then(|(d, _)| {
                    let account = self.mail.accounts().iter().find(|a| a.id == d.account_id)?;
                    Some((account.id, komorebi::compose::outgoing(&db, account, d)))
                });
            match rebuilt {
                Some((account, outgoing)) => self.composer.update(cx, |c, cx| {
                    c.queue_send(row, account, outgoing, std::time::Duration::ZERO, cx)
                }),
                None => {
                    // Never sent half: back to Drafts as it was.
                    db.outbox_unsend(row).ok();
                    back_to_drafts = true;
                }
            }
        }
        self.hub.update(cx, |hub, cx| {
            if back_to_drafts {
                hub.say(super::sync::Said::RebuildFailed, true, cx);
            }
            hub.drain_outbox(cx);
        });
        self.act_later(cx, |m| {
            m.load_folders();
            m.reload_rows();
        });
    }

    /// The Outbox changed: its count, its rows and the open entry.
    pub fn outbox_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.act(window, cx, |m| {
            m.load_counts();
            if m.view() == View::Outbox {
                m.reload_rows();
            }
        });
    }

    fn say_detail(&mut self, e: &anyhow::Error, cx: &mut Context<Self>) {
        let detail = format!("{e:#}");
        self.hub.update(cx, |hub, cx| {
            hub.say(super::sync::Said::Detail(detail), true, cx)
        });
    }
}

/// The pane for the open Outbox entry, or `None` when none is open.
pub fn render(view: &MailView, t: &Tokens, cx: &mut Context<MailView>) -> Option<AnyElement> {
    let (row, e) = view.outbox_entry()?;
    let o = offers(&e);
    let subject = if e.subject.trim().is_empty() {
        tr!("(no subject)")
    } else {
        e.subject.clone()
    };
    let status = komorebi::format::outbox_status(&e, chrono::Utc::now().timestamp());
    let edit_label = tr!("Edit");
    let retry_label = tr!("Retry");
    let discard_label = tr!("Discard");
    let mut bar = div()
        .id("outbox-commands")
        .role(Role::Toolbar)
        .h(px(layout::CMD_BAR_H))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(space::SP_1));
    // The lead action is the labelled pill (Retry, or Edit for a refusal
    // sending again can't fix); the others are icon buttons.
    let edit_leads = o.permanent || !o.retry;
    if o.edit {
        bar = if edit_leads {
            bar.child(
                Button::new("outbox-edit")
                    .icon(Icon::Compose)
                    .label(edit_label.clone())
                    .height(Height::Medium)
                    .on_click(cx.listener(move |this, _, w, cx| this.outbox_edit(row, w, cx))),
            )
        } else {
            bar.child(
                IconButton::new("outbox-edit", Icon::Compose, edit_label.clone())
                    .on_click(cx.listener(move |this, _, w, cx| this.outbox_edit(row, w, cx))),
            )
        };
    }
    if o.retry {
        bar = if edit_leads && o.edit {
            bar.child(
                IconButton::new("outbox-retry", Icon::Send, retry_label.clone())
                    .on_click(cx.listener(move |this, _, w, cx| this.outbox_retry(row, w, cx))),
            )
        } else {
            bar.child(
                Button::new("outbox-retry")
                    .icon(Icon::Send)
                    .label(retry_label.clone())
                    .height(Height::Medium)
                    .on_click(cx.listener(move |this, _, w, cx| this.outbox_retry(row, w, cx))),
            )
        };
    }
    if o.discard {
        bar = bar.child(
            IconButton::new("outbox-discard", Icon::Delete, discard_label)
                .on_click(cx.listener(move |this, _, w, cx| this.outbox_discard(row, w, cx))),
        );
    }
    let mut notice = Notice::new(
        "outbox-state",
        if o.problem { Kind::Error } else { Kind::Info },
        status,
    );
    let detail = e.last_error.trim().to_string();
    if o.problem && !detail.is_empty() && view.outbox_details != Some(row) {
        notice = notice.action(
            Button::new("outbox-details")
                .variant(Variant::OnInset)
                .label(tr!("Details"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.outbox_details = Some(row);
                    cx.notify();
                })),
        );
    }
    let body = excerpt(&e.body);
    Some(
        div()
            .id("outbox-entry")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(space::SP_3))
            .px(px(space::SP_5))
            .child(bar.test_support())
            .child(
                div()
                    .id("outbox-subject")
                    .role(Role::Heading)
                    .aria_label(subject.clone())
                    .typo(&t.text.title)
                    .text_color(t.color.ink)
                    .child(subject)
                    .test_support(),
            )
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(t.color.ink_2)
                    .child(tr!("To: {0}", komorebi::format::recipients(&e.to))),
            )
            .child(notice)
            .when(
                view.outbox_details == Some(row) && !detail.is_empty(),
                |el| {
                    el.child(
                        div()
                            .id("outbox-detail")
                            .role(Role::Paragraph)
                            .aria_label(detail.clone())
                            .typo(&t.text.caption)
                            .text_color(t.color.ink_2)
                            .child(detail.clone())
                            .test_support(),
                    )
                },
            )
            .when(!body.is_empty(), |el| {
                el.child(
                    div()
                        .typo(&t.text.body)
                        .text_color(t.color.ink)
                        .whitespace_normal()
                        .child(body),
                )
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: Outbox, error: &str) -> OutboxEntry {
        OutboxEntry {
            account_id: 1,
            to: "ana@example.com".into(),
            subject: "Plans".into(),
            state,
            next_try: None,
            last_error: error.into(),
            updated: 0,
            has_attach: false,
            invite_reply: false,
            body: String::new(),
        }
    }

    /// What each state offers (the Slint app's `show_outbox_entry`).
    #[test]
    fn each_state_offers_its_actions() {
        let queued = offers(&entry(Outbox::Queued, ""));
        assert!(queued.edit && !queued.retry && queued.discard && !queued.problem);
        let sending = offers(&entry(Outbox::Sending, ""));
        assert!(!sending.edit && !sending.retry && !sending.discard);
        let failed = offers(&entry(Outbox::Failed, "550 5.1.1 no such user"));
        assert!(failed.edit && failed.retry && failed.discard && failed.problem);
        assert!(failed.permanent, "a 5xx refusal leads with Edit");
        let unsure = offers(&entry(Outbox::Unconfirmed, ""));
        assert!(unsure.retry && unsure.problem && !unsure.permanent);
        let invite = offers(&OutboxEntry {
            invite_reply: true,
            ..entry(Outbox::Queued, "")
        });
        assert!(!invite.edit);
    }

    /// The first lines as written, without the quote or the signature.
    #[test]
    fn the_excerpt_stops_at_the_quote_and_the_signature() {
        assert_eq!(excerpt("Hi\n\nSee you\n-- \nAna"), "Hi\nSee you");
        assert_eq!(excerpt("One\n> quoted\nTwo"), "One");
        let long = "x".repeat(400);
        assert!(excerpt(&long).ends_with('\u{2026}'));
    }
}
