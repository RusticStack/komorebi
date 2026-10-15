//! The outbox: sent messages until their send settles (`db::Outbox`).
//!
//! Send moves a draft here and the composer starts the build and the undo
//! window (`composer::queue_send`). This side hands queued rows to the
//! account's send worker (`mail::Job::Send`) when they come due: on the
//! 1-minute pass, on Refresh, on Retry and at start. A row is only ever
//! sent by the one send that takes it (`Db::claim_outbox`), and an
//! unconfirmed send is never sent again without the user. The Outbox view
//! lists the rows; a failed or unconfirmed one offers Edit, Retry and
//! Discard.

use super::*;

impl App {
    /// After the first paint: a Send cut short by the last exit left its
    /// row in the undo window. It is sent now (its message built again if
    /// it wasn't stored), then whatever is due.
    pub(super) fn resume_outbox(&mut self) {
        let pending = match self.db.outbox_pending() {
            Ok(rows) => rows,
            Err(e) => return self.set_status(&text::detail(&e), true),
        };
        for (row, built) in pending {
            if self.pending_sends.contains_key(&row) {
                continue;
            }
            if built {
                if let Err(e) = self.db.outbox_queue(row) {
                    self.set_status(&text::detail(&e), true);
                }
                continue;
            }
            let rebuilt = composer::load(&self.db, row)
                .ok()
                .flatten()
                .filter(|(_, missing)| missing.is_empty())
                .and_then(|(d, _)| {
                    let account = self.accounts.iter().find(|a| a.id == d.account_id)?;
                    Some((account.id, composer::outgoing(&self.db, account, d)))
                });
            match rebuilt {
                Some((account, outgoing)) => {
                    self.queue_send(row, account, outgoing, Duration::ZERO)
                }
                None => {
                    // Never sent half: it goes back to Drafts as it was.
                    self.db.outbox_unsend(row).ok();
                    self.set_status(&t!(invoke_rebuild_failed), true);
                    self.drafts_changed(None, true);
                }
            }
        }
        self.drain_outbox();
        self.outbox_changed();
    }

    /// Hand every queued row that is due to its account's send worker.
    /// Handing one over twice is harmless: only one send can take it.
    pub(super) fn drain_outbox(&self) {
        match self.db.outbox_due(chrono::Utc::now().timestamp()) {
            Ok(rows) => {
                for (row, account) in rows {
                    self.dispatch(row, account);
                }
            }
            Err(e) => self.set_status(&text::detail(&e), true),
        }
    }

    /// The outbox changed: its nav count, its view and the open entry.
    pub(super) fn outbox_changed(&mut self) {
        self.refresh_outbox_count();
        self.rebuild_nav();
        if self.view == View::Outbox {
            self.reload_rows();
        }
        self.show_outbox_entry();
    }

    pub(super) fn refresh_outbox_count(&mut self) {
        match self.db.count_outbox() {
            Ok(n) => self.outbox_count = n,
            Err(e) => self.set_status(&text::detail(&e), true),
        }
    }

    /// A row of the Outbox view was clicked: show it with its actions.
    pub(super) fn open_outbox(&mut self, row: i64) {
        let ui = self.ui();
        if ui.get_composing() {
            self.park_composer();
            ui.set_composing(false);
        }
        self.outbox_open = Some(row);
        self.show_outbox_entry();
        self.sync_selection_index();
    }

    /// Show the open outbox entry (hidden once it is gone).
    pub(super) fn show_outbox_entry(&mut self) {
        let ui = self.ui();
        let entry = self
            .outbox_open
            .and_then(|row| self.db.outbox_entry(row).ok().flatten());
        let Some(e) = entry else {
            self.outbox_open = None;
            ui.set_o_show(false);
            return;
        };
        use db::Outbox::*;
        ui.set_o_show(true);
        ui.set_o_subject(if e.subject.trim().is_empty() {
            t!(invoke_no_subject).into()
        } else {
            e.subject.as_str().into()
        });
        ui.set_o_to(format::recipients(&e.to).into());
        ui.set_o_status(format::outbox_status(&e, chrono::Utc::now().timestamp()).into());
        ui.set_o_problem(matches!(e.state, Failed | Unconfirmed));
        // The server's own words, behind Details (while there are any).
        ui.set_o_detail(e.last_error.as_str().into());
        // A refusal that sending again can't fix: Edit leads, Retry follows.
        ui.set_o_permanent(e.state == Failed && mail::smtp::classify(&e.last_error).permanent());
        ui.set_o_body(outbox_excerpt(&e.body).into());
        ui.set_o_can_edit(e.state != Sending && !e.invite_reply);
        ui.set_o_can_retry(matches!(e.state, Failed | Unconfirmed));
        ui.set_o_can_discard(matches!(e.state, Queued | Failed | Unconfirmed));
    }

    /// Edit: the open entry goes back to Drafts and opens in the composer.
    pub(super) fn outbox_edit(&mut self) {
        if let Some(row) = self.outbox_open {
            self.undo_send(row);
        }
    }

    /// Retry the open entry now. One that may have gone out asks first.
    pub(super) fn outbox_retry(&mut self) {
        let Some(row) = self.outbox_open else {
            return;
        };
        match self.db.outbox_entry(row) {
            Ok(Some(e)) if e.state == db::Outbox::Unconfirmed => self.ask(Question::Resend(row)),
            Ok(Some(_)) => self.retry_now(row),
            Ok(None) => self.outbox_changed(),
            Err(e) => self.set_status(&text::detail(&e), true),
        }
    }

    pub(super) fn retry_now(&mut self, row: i64) {
        match self.db.outbox_retry(row) {
            Ok(true) => {
                if let Ok(Some(e)) = self.db.outbox_entry(row) {
                    self.dispatch(row, e.account_id);
                }
            }
            Ok(false) => {}
            Err(e) => self.set_status(&text::detail(&e), true),
        }
        self.outbox_changed();
    }

    /// Discard the open entry, after asking.
    pub(super) fn outbox_discard(&mut self) {
        if let Some(row) = self.outbox_open {
            self.ask(Question::DiscardOutbox(row));
        }
    }

    pub(super) fn discard_outbox_now(&mut self, row: i64) {
        match self.db.discard_outbox(row) {
            Ok((true, copy)) => {
                self.remove_server_copy(copy);
                self.set_status(&t!(invoke_message_discarded), false);
            }
            Ok((false, _)) => {}
            Err(e) => self.set_status(&text::detail(&e), true),
        }
        self.outbox_changed();
    }

    /// A send ended; its row is already settled.
    pub(super) fn on_sent(&mut self, reply_to: Option<i64>, outcome: Outcome) {
        self.outbox_changed();
        if let Some(m) = reply_to {
            self.rows.changed(m);
        }
        let error = !matches!(outcome, Outcome::Delivered);
        if let Some(text) = text::outcome(&outcome) {
            self.set_status(&text, error);
        }
        // A send that failed for good stays said until it is dismissed.
        if matches!(outcome, Outcome::Failed(_) | Outcome::Unconfirmed(_)) {
            self.status_timer.stop();
        }
    }
}

/// Lines of an Outbox message's text shown under its sentence.
const EXCERPT_LINES: usize = 3;
const EXCERPT_CHARS: usize = 280;

/// The first lines of what was written (formatting marks taken out, no
/// quote or signature), for the open Outbox entry.
pub(super) fn outbox_excerpt(body: &str) -> String {
    let plain = mail::compose_html::plain(body);
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
