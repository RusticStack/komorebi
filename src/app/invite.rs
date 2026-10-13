//! The invite card: the meeting invitation in the open message
//! (`mail::ical`), the answer given to it (the `rsvp` table) and the reply,
//! which goes only to the organizer, through the outbox.

use super::*;
use crate::mail::ical::{self, Answer, Event, Person};
use crate::mail::smtp;

impl App {
    /// The open message and its invitation, if it has one.
    fn open_invite(&self) -> Option<(&db::Message, &Event)> {
        let open = self.open.as_ref()?;
        Some((&open.msg, open.body.as_ref()?.invite.as_ref()?))
    }

    /// The attendee that is the user: the account's address, else one of
    /// the message's To and Cc addresses (`None`: not listed).
    fn invite_me<'a>(&self, m: &db::Message, ev: &'a Event) -> Option<&'a Person> {
        let account = self.accounts.iter().find(|a| a.id == m.account_id)?;
        let list = format!("{}, {}", m.to, m.cc);
        let entries = smtp::split_addresses(&list);
        let others: Vec<&str> = entries.iter().map(|e| smtp::mailbox_parts(e).1).collect();
        ev.me(&account.email, &others)
    }

    /// Fill the invite card for the open message (hidden without one).
    pub(super) fn show_invite(&self) {
        let ui = self.ui();
        let Some((m, ev)) = self.open_invite() else {
            ui.set_i_title(SharedString::new());
            return;
        };
        let past = ev.past(&chrono::Local::now());
        let answerable = ev.answerable(self.invite_me(m, ev), past);
        // An answer to an earlier version of the invite doesn't count.
        let answer = self
            .db
            .rsvp(m.account_id, &ev.uid, &ev.recurrence_id)
            .ok()
            .flatten()
            .filter(|&(seq, _)| seq >= ev.sequence)
            .and_then(|(_, code)| Answer::from_code(code));
        let state = if ev.cancelled() {
            t!(invoke_invite_cancelled)
        } else if let Some(a) = answer {
            match a {
                Answer::Accepted => t!(invoke_invite_accepted),
                Answer::Tentative => t!(invoke_invite_tentative),
                Answer::Declined => t!(invoke_invite_declined),
            }
        } else if past {
            t!(invoke_invite_over)
        } else if ev.sequence > 0 && ev.method == "REQUEST" {
            t!(invoke_invite_updated)
        } else {
            String::new()
        };
        let mut lines = vec![ev.when_text_in(self.lang, &chrono::Local)];
        if let Some(o) = &ev.organizer {
            lines.push(t!(
                invoke_invite_organizer,
                smtp::mailbox(&o.name, &o.addr).into()
            ));
        }
        if !ev.location.is_empty() {
            lines.push(t!(invoke_invite_where, ev.location.as_str().into()));
        }
        lines.push(ev.repeats_in(self.lang));
        let title = if ev.summary.is_empty() {
            t!(invoke_invite_no_title)
        } else {
            ev.summary.clone()
        };
        ui.set_i_title(title.into());
        ui.set_i_lines(lines.join("\n").trim_end().into());
        ui.set_i_state(state.into());
        ui.set_i_details(ev.description.as_str().into());
        ui.set_i_cancelled(ev.cancelled());
        let answers: &[i32] = if answerable && (answer.is_none() || self.invite_change) {
            &[1, 2, 3]
        } else {
            &[]
        };
        let mut tools = Vec::new();
        if answerable && answer.is_some() && !self.invite_change {
            tools.push(5);
        }
        if !ev.join.is_empty() && !ev.cancelled() {
            tools.push(0);
        }
        if !ev.description.is_empty() {
            tools.push(6);
        }
        tools.push(4);
        ui.set_i_answer(answer.map_or(0, |a| a as i32));
        ui.set_i_answers(ModelRc::new(VecModel::from(answers.to_vec())));
        ui.set_i_tools(ModelRc::new(VecModel::from(tools)));
    }

    /// A card action other than Save .ics (`invite` in the UI).
    pub(super) fn invite_action(&mut self, action: i32) {
        let Some((m, ev)) = self.open_invite() else {
            return;
        };
        let msg = m.id;
        match action {
            0 if !ev.join.is_empty() => self.reader.follow(ev.join.clone()),
            5 => {
                self.invite_change = true;
                self.show_invite();
            }
            _ => {
                if let Some(answer) = Answer::from_code(action.into()) {
                    self.ask(Question::Rsvp(msg, answer));
                }
            }
        }
    }

    /// The prompt before answering: who the reply goes to (exactly), and
    /// a warning when the organizer is not in the sender's domain or the
    /// user is not an attendee.
    pub(super) fn rsvp_prompt(&self, answer: Answer) -> (String, String) {
        let title = match answer {
            Answer::Accepted => t!(invoke_q_rsvp_title_accepted),
            Answer::Tentative => t!(invoke_q_rsvp_title_tentative),
            Answer::Declined => t!(invoke_q_rsvp_title_declined),
        };
        let Some((m, ev)) = self.open_invite() else {
            return (title, String::new());
        };
        let account = self.account_email(m.account_id);
        let organizer = ev
            .organizer
            .as_ref()
            .map_or(String::new(), |o| smtp::mailbox(&o.name, &o.addr));
        // Whole sentences, one after the other.
        let mut text = t!(
            invoke_q_rsvp_text,
            organizer.as_str().into(),
            account.as_str().into()
        );
        let domain = |a: &str| {
            let host = a
                .rsplit_once('@')
                .map_or("", |(_, d)| d)
                .to_ascii_lowercase();
            mail::render::links::registrable(&host).to_string()
        };
        let theirs = ev
            .organizer
            .as_ref()
            .map_or(String::new(), |o| domain(&o.addr));
        if theirs != domain(&m.from_addr) {
            text.push(' ');
            text.push_str(&t!(
                invoke_q_rsvp_domain,
                theirs.as_str().into(),
                domain(&m.from_addr).into()
            ));
        }
        match self.invite_me(m, ev) {
            None => {
                text.push(' ');
                text.push_str(&t!(invoke_q_rsvp_unlisted, account.as_str().into()));
            }
            Some(p) if !p.addr.eq_ignore_ascii_case(&account) => {
                text.push(' ');
                text.push_str(&t!(invoke_q_rsvp_alias, p.addr.as_str().into()));
            }
            Some(_) => {}
        }
        text.push(' ');
        text.push_str(&t!(invoke_q_rsvp_local));
        (title, text)
    }

    /// Answer the open invitation (message `msg`): record the answer and,
    /// with `send`, queue the reply to the organizer in the outbox.
    pub(super) fn rsvp(&mut self, msg: i64, answer: Answer, send: bool) {
        let Some((m, ev)) = self.open_invite().filter(|(m, _)| m.id == msg) else {
            return;
        };
        let Some(account) = self.accounts.iter().find(|a| a.id == m.account_id) else {
            return;
        };
        let (uid, rid, seq) = (ev.uid.clone(), ev.recurrence_id.clone(), ev.sequence);
        if send {
            let me = self.invite_me(m, ev);
            let me = me.map_or((account.name.as_str(), account.email.as_str()), |p| {
                (p.name.as_str(), p.addr.as_str())
            });
            let now = chrono::Utc::now();
            let Some(out) = ical::reply_message(ev, &account.name, &account.email, me, answer, now)
            else {
                return;
            };
            let draft = db::DraftRow {
                account_id: account.id,
                to: out.to.clone(),
                subject: out.subject.clone(),
                body: out.body.clone(),
                message_id: out.message_id.clone(),
                ..Default::default()
            };
            match smtp::build_raw(out).and_then(|raw| self.db.queue_reply(&draft, &raw)) {
                Ok(row) => self.dispatch(row, account.id),
                Err(e) => {
                    return self.set_status(
                        &t!(invoke_could_not_send_reply, format!("{e:#}").into()),
                        true,
                    );
                }
            }
        }
        let account_id = account.id;
        if let Err(e) = self
            .db
            .record_rsvp(account_id, &uid, &rid, seq, answer as i64)
        {
            self.set_status(&text::detail(&e), true);
        } else if send {
            self.set_status(&t!(invoke_reply_in_outbox), false);
        } else {
            self.set_status(&t!(invoke_answer_kept), false);
        }
        self.invite_change = false;
        if send {
            self.outbox_changed();
        }
        self.show_invite();
    }
}

/// Save .ics: the invitation's calendar file, as `invite.ics`.
pub(super) fn save_invite() {
    let data = with_app(|a| mime::calendar_file(a.open.as_ref()?.msg.raw.as_deref()?)).flatten();
    if let Some(data) = data {
        save_file(&t!(invoke_dlg_save_invitation), "invite.ics", data);
    }
}
