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
            Err(e) => return self.set_status(&format!("{e:#}"), true),
        };
        for (row, built) in pending {
            if self.pending_sends.contains_key(&row) {
                continue;
            }
            if built {
                if let Err(e) = self.db.outbox_queue(row) {
                    self.set_status(&format!("{e:#}"), true);
                }
                continue;
            }
            let rebuilt = composer::load(&self.db, row)
                .ok()
                .flatten()
                .filter(|(_, missing)| missing.is_empty())
                .and_then(|(d, _)| {
                    let account = self.accounts.iter().find(|a| a.id == d.account_id)?;
                    Some((account.id, composer::outgoing(account, d)))
                });
            match rebuilt {
                Some((account, outgoing)) => {
                    self.queue_send(row, account, outgoing, Duration::ZERO)
                }
                None => {
                    // Never sent half: it goes back to Drafts as it was.
                    self.db.outbox_unsend(row).ok();
                    self.set_status(
                        "A message sent as Komorebi closed couldn't be prepared again; it is back in Drafts.",
                        true,
                    );
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
            Err(e) => self.set_status(&format!("{e:#}"), true),
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
            Err(e) => self.set_status(&format!("{e:#}"), true),
        }
    }

    /// A row of the Outbox view was clicked: show it with its actions.
    pub(super) fn open_outbox(&mut self, row: i64) {
        let ui = self.ui();
        if ui.get_composing() {
            if let Some(id) = self.drafts.active_id() {
                self.save_draft(id);
            }
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
            "(no subject)".into()
        } else {
            e.subject.as_str().into()
        });
        ui.set_o_to(format::recipients(&e.to).into());
        ui.set_o_status(format::outbox_status(&e, chrono::Utc::now().timestamp()).into());
        ui.set_o_problem(matches!(e.state, Failed | Unconfirmed));
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
            Err(e) => self.set_status(&format!("{e:#}"), true),
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
            Err(e) => self.set_status(&format!("{e:#}"), true),
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
                self.set_status("Message discarded.", false);
            }
            Ok((false, _)) => {}
            Err(e) => self.set_status(&format!("{e:#}"), true),
        }
        self.outbox_changed();
    }

    /// A send ended; its row is already settled.
    pub(super) fn on_sent(&mut self, reply_to: Option<i64>, outcome: Outcome) {
        self.outbox_changed();
        if let Some(m) = reply_to {
            self.rows.changed(m);
        }
        match outcome {
            Outcome::Delivered => self.set_status("Message sent.", false),
            Outcome::DeliveredNotSaved(e) => {
                self.set_status(&format!("Message sent, but {e}"), true)
            }
            Outcome::Retry(e) => self.set_status(
                &format!("Not sent yet: {e}. It stays in the Outbox and is tried again shortly."),
                true,
            ),
            Outcome::Failed(e) => {
                self.set_status(&format!("Not sent: {e}. It is in the Outbox."), true)
            }
            Outcome::Unconfirmed(e) => self.set_status(&e, true),
        }
    }
}
