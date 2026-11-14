//! The invite card's logic, shared by both apps: what the card for a
//! meeting invitation in the open message shows and offers (`mail::ical`,
//! the answer given in the `rsvp` table, the local calendar), what the
//! prompt before an answer warns about, and the answer itself: recorded,
//! the local calendar kept in step, and the reply queued in the outbox for
//! the organizer only. Words are the apps' (typed values here).

use crate::db::{self, Account, Db, DraftRow};
use crate::i18n::Lang;
use crate::mail::ical::{self, Answer, Event, Person};
use crate::mail::{mime, smtp};
use chrono::{DateTime, Local};

/// The state line of the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Cancelled,
    Accepted,
    Tentative,
    Declined,
    /// The event is over.
    Over,
    /// A newer version of an invitation.
    Updated,
}

/// A card action besides the answers. The numbers are the 0.7 UI's
/// `invite` codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Join = 0,
    SaveIcs = 4,
    /// Answer again.
    Change = 5,
    /// Show the description.
    Details = 6,
    /// A cancelled event still in the local calendar.
    Remove = 7,
}

/// What the card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// The summary; empty: the event has none.
    pub title: String,
    /// When it is, in the app's language and the local time zone.
    pub when: String,
    /// The organizer as a mailbox (`Name <addr>`).
    pub organizer: Option<String>,
    pub location: String,
    /// How it repeats (empty: it does not).
    pub repeats: String,
    /// A copy is in the calendar.
    pub in_calendar: bool,
    pub status: Option<Status>,
    pub details: String,
    pub cancelled: bool,
    /// Accept, Tentative and Decline show.
    pub answers: bool,
    /// The answer given (the filled button while `answers` shows).
    pub answer: Option<Answer>,
    pub tools: Vec<Tool>,
}

/// The attendee that is the user: the account's address, else one of the
/// message's To and Cc addresses (`None`: not listed).
pub fn me<'a>(account_email: &str, m: &db::Message, ev: &'a Event) -> Option<&'a Person> {
    let list = format!("{}, {}", m.to, m.cc);
    let entries = smtp::split_addresses(&list);
    let others: Vec<&str> = entries.iter().map(|e| smtp::mailbox_parts(e).1).collect();
    ev.me(account_email, &others)
}

/// The card for invitation `ev` in message `m` of the account with
/// `account_email` (`None`: the account is gone), at `now`. `change`: the
/// user pressed Change on an answered invitation. A cancellation from the
/// organizer marks the stored copy cancelled.
pub fn card(
    db: &Db,
    account_email: Option<&str>,
    m: &db::Message,
    ev: &Event,
    lang: Lang,
    now: &DateTime<Local>,
    change: bool,
) -> Card {
    let past = ev.past(now);
    let me = account_email.and_then(|a| me(a, m, ev));
    let answerable = ev.answerable(me, past);
    // An answer to an earlier version of the invite doesn't count.
    let answer = db
        .rsvp(m.account_id, &ev.uid, &ev.recurrence_id)
        .ok()
        .flatten()
        .filter(|&(seq, _)| seq >= ev.sequence)
        .and_then(|(_, code)| Answer::from_code(code));
    let status = if ev.cancelled() {
        Some(Status::Cancelled)
    } else if let Some(a) = answer {
        Some(match a {
            Answer::Accepted => Status::Accepted,
            Answer::Tentative => Status::Tentative,
            Answer::Declined => Status::Declined,
        })
    } else if past {
        Some(Status::Over)
    } else if ev.sequence > 0 && ev.method == "REQUEST" {
        Some(Status::Updated)
    } else {
        None
    };
    let in_calendar = !ev.uid.is_empty() && matches!(db.event_by_uid(&ev.uid), Ok(Some(_)));
    let mut tools = Vec::new();
    if answerable && answer.is_some() && !change {
        tools.push(Tool::Change);
    }
    if !ev.join.is_empty() && !ev.cancelled() {
        tools.push(Tool::Join);
    }
    if in_calendar && ev.cancelled() && ev.recurrence_id.is_empty() {
        tools.push(Tool::Remove);
    }
    if !ev.description.is_empty() {
        tools.push(Tool::Details);
    }
    tools.push(Tool::SaveIcs);
    Card {
        title: ev.summary.clone(),
        when: ev.when_text_in(lang, &Local),
        organizer: ev
            .organizer
            .as_ref()
            .map(|o| smtp::mailbox(&o.name, &o.addr)),
        location: ev.location.clone(),
        repeats: ev.repeats_in(lang),
        in_calendar,
        status,
        details: ev.description.clone(),
        cancelled: ev.cancelled(),
        answers: answerable && (answer.is_none() || change),
        answer,
        tools,
    }
}

/// What the prompt before an answer says: who the reply goes to (exactly),
/// and the warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// The organizer as a mailbox (empty: none).
    pub organizer: String,
    pub account: String,
    /// The organizer's registrable domain and the sender's, when they
    /// differ.
    pub domains: Option<(String, String)>,
    /// The account is not an attendee.
    pub unlisted: bool,
    /// The user is listed under another of their addresses.
    pub alias: Option<String>,
}

/// The prompt for answering `ev` in message `m` from `account_email`.
pub fn prompt(account_email: &str, m: &db::Message, ev: &Event) -> Prompt {
    let organizer = ev
        .organizer
        .as_ref()
        .map_or(String::new(), |o| smtp::mailbox(&o.name, &o.addr));
    let domain = |a: &str| {
        let host = a
            .rsplit_once('@')
            .map_or("", |(_, d)| d)
            .to_ascii_lowercase();
        crate::mail::render::links::registrable(&host).to_string()
    };
    let theirs = ev
        .organizer
        .as_ref()
        .map_or(String::new(), |o| domain(&o.addr));
    let sender = domain(&m.from_addr);
    let listed = me(account_email, m, ev);
    Prompt {
        organizer,
        account: account_email.to_string(),
        domains: (theirs != sender).then_some((theirs, sender)),
        unlisted: listed.is_none(),
        alias: listed
            .filter(|p| !p.addr.eq_ignore_ascii_case(account_email))
            .map(|p| p.addr.clone()),
    }
}

/// Queue the reply to `ev` (in message `m`) for its organizer in the
/// outbox: the row to dispatch, or `None` when the invitation has no one
/// to reply to.
pub fn queue_reply(
    db: &Db,
    account: &Account,
    m: &db::Message,
    ev: &Event,
    answer: Answer,
) -> anyhow::Result<Option<i64>> {
    let listed = me(&account.email, m, ev);
    let me = listed.map_or((account.name.as_str(), account.email.as_str()), |p| {
        (p.name.as_str(), p.addr.as_str())
    });
    let now = chrono::Utc::now();
    let Some(out) = ical::reply_message(ev, &account.name, &account.email, me, answer, now) else {
        return Ok(None);
    };
    let draft = DraftRow {
        account_id: account.id,
        to: out.to.clone(),
        subject: out.subject.clone(),
        body: out.body.clone(),
        message_id: out.message_id.clone(),
        ..Default::default()
    };
    let raw = smtp::build_raw(out)?;
    Ok(Some(db.queue_reply(&draft, &raw)?))
}

/// Keep the answer in the `rsvp` table, and nothing else: the card shows
/// it, and an answer to an older version of the invitation stops counting.
/// This is what the GPUI app calls; it has no calendar, so it never writes
/// the local calendar tables.
pub fn record_answer(db: &Db, account: &Account, ev: &Event, answer: Answer) -> anyhow::Result<()> {
    db.record_rsvp(
        account.id,
        &ev.uid,
        &ev.recurrence_id,
        ev.sequence,
        answer as i64,
    )?;
    Ok(())
}

/// Keep the answer: in the `rsvp` table, and in the local calendar
/// (Accept and Tentative store the invitation, Decline removes a stored
/// copy, of the one occurrence only when the invitation is for one). No
/// app calls this in 1.0 (the GPUI app calls [`record_answer`]); it stays
/// with the calendar code and its tables for 1.1.0, when the calendar
/// returns.
pub fn record(
    db: &Db,
    account: &Account,
    m: &db::Message,
    ev: &Event,
    answer: Answer,
) -> anyhow::Result<()> {
    record_answer(db, account, ev, answer)?;
    let me_addr =
        me(&account.email, m, ev).map_or_else(|| account.email.clone(), |p| p.addr.clone());
    let raw = m.raw.as_deref().and_then(mime::calendar_file);
    let cal = db.local_calendar()?;
    let now = chrono::Utc::now().timestamp();
    let (uid, rid) = (ev.uid.as_str(), ev.recurrence_id.as_str());
    match answer {
        Answer::Declined if !rid.is_empty() => {
            db.decline_occurrence(cal, uid, rid, now)?;
        }
        Answer::Declined => {
            db.remove_event_uid(cal, uid)?;
        }
        Answer::Accepted | Answer::Tentative => {
            if let Some(raw) = raw {
                db.store_invite(cal, &raw, &me_addr, answer.partstat(), now)?;
            }
        }
    }
    Ok(())
}

/// A cancellation from the organizer marks the stored copy in the local
/// calendar cancelled (it stays until the user removes it). No app calls
/// this in 1.0: the GPUI app has no calendar, so [`card`] no longer writes
/// the calendar tables when a cancellation is opened. It stays with
/// [`record`] and [`remove`] for 1.1.0, when the calendar returns and the
/// open message marks the copy again. Whole series only (an occurrence's
/// cancellation has a `RECURRENCE-ID`).
pub fn mark_cancelled(db: &Db, ev: &Event) -> anyhow::Result<bool> {
    if !ev.cancelled() || ev.uid.is_empty() || !ev.recurrence_id.is_empty() {
        return Ok(false);
    }
    db.cancel_event_uid(&ev.uid, ev.sequence)
}

/// "Remove from calendar" on a cancelled event.
pub fn remove(db: &Db, ev: &Event) -> anyhow::Result<()> {
    let cal = db.local_calendar()?;
    db.remove_event_uid(cal, &ev.uid)?;
    Ok(())
}

/// Save .ics: the invitation's calendar file in message `m`.
pub fn calendar_file(m: &db::Message) -> Option<Vec<u8>> {
    mime::calendar_file(m.raw.as_deref()?)
}

#[cfg(test)]
mod tests {
    //! The library half of the Slint app's `invite_card_answers_and_replies`
    //! (GPUI-TEST-PORT.md row 139): what the card offers, the prompt, the
    //! reply queued for the organizer only, the answer kept.

    use super::*;
    use crate::seed::{self, EMAIL};

    fn invite(method: &str, organizer: &str, attendees: &str, extra: &str) -> String {
        format!(
            "From: Ana <ana@example.org>\r\nTo: {EMAIL}\r\nSubject: Plan\r\n\
             Content-Type: text/calendar; method={method}\r\n\r\nBEGIN:VCALENDAR\r\n\
             METHOD:{method}\r\nBEGIN:VEVENT\r\nUID:plan@example.org\r\nSEQUENCE:1\r\n\
             DTSTART:20991008T130000Z\r\nSUMMARY:Planning\r\n\
             DESCRIPTION:Agenda\r\nORGANIZER;CN=Ana:mailto:{organizer}\r\n{attendees}{extra}\
             END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    /// A cache with the seed account and one message carrying `raw`.
    fn open(raw: &str) -> (Db, Account, db::Message, Event) {
        let mut db = Db::in_memory().unwrap();
        let made = seed::seed(&mut db, 1).unwrap();
        let id = db.message_ids(&[made.inbox], "").unwrap()[0];
        db.set_body(id, raw.as_bytes(), "", false, Default::default())
            .unwrap();
        let mut m = db.message(id).unwrap().unwrap();
        // The list row's sender (an organizer in its domain raises no
        // warning) and recipients, as the sync stores them.
        m.from_addr = "ana@example.org".into();
        m.to = EMAIL.into();
        let (body, _) = mime::parse_reader(m.raw.as_deref().unwrap());
        let ev = *body.invite.expect("an invitation");
        let account = db.accounts().unwrap().remove(0);
        (db, account, m, ev)
    }

    fn now() -> DateTime<Local> {
        Local::now()
    }

    #[test]
    fn the_card_offers_answers_then_change_and_the_reply_goes_to_the_organizer() {
        let both = format!(
            "ATTENDEE;RSVP=TRUE:mailto:{EMAIL}\r\nATTENDEE;RSVP=TRUE:mailto:carol@example.com\r\n"
        );
        let (db, account, m, ev) = open(&invite("REQUEST", "ana@example.org", &both, ""));
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), false);
        assert_eq!(c.title, "Planning");
        assert_eq!(c.status, Some(Status::Updated));
        assert_eq!(c.organizer.as_deref(), Some("Ana <ana@example.org>"));
        assert!(c.answers && c.answer.is_none());
        assert_eq!(c.tools, [Tool::Details, Tool::SaveIcs]);
        assert_eq!(c.details, "Agenda");

        let p = prompt(EMAIL, &m, &ev);
        assert_eq!(p.organizer, "Ana <ana@example.org>");
        assert_eq!(p.account, EMAIL);
        assert!(p.domains.is_none() && !p.unlisted && p.alias.is_none());

        // Accept with a reply: one outbox row for the organizer, the
        // answer kept.
        let row = queue_reply(&db, &account, &m, &ev, Answer::Accepted)
            .unwrap()
            .expect("a reply");
        record(&db, &account, &m, &ev, Answer::Accepted).unwrap();
        let due = db.outbox_due(i64::MAX).unwrap();
        assert_eq!(due, [(row, account.id)]);
        let e = db.outbox_entry(row).unwrap().unwrap();
        assert_eq!(
            (e.to.as_str(), e.subject.as_str()),
            ("Ana <ana@example.org>", "Accepted: Planning")
        );
        assert!(e.invite_reply);
        assert_eq!(
            db.rsvp(account.id, "plan@example.org", "").unwrap(),
            Some((1, 1))
        );
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), false);
        assert_eq!(c.status, Some(Status::Accepted));
        assert!(!c.answers);
        assert_eq!(c.tools, [Tool::Change, Tool::Details, Tool::SaveIcs]);
        // Accepting stored it in the calendar.
        assert!(c.in_calendar);

        // Change: the answers again, the given one marked; Decline without
        // a reply keeps it here and takes it out of the calendar.
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), true);
        assert!(c.answers);
        assert_eq!(c.answer, Some(Answer::Accepted));
        record(&db, &account, &m, &ev, Answer::Declined).unwrap();
        assert_eq!(db.outbox_due(i64::MAX).unwrap().len(), 1);
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), false);
        assert_eq!(c.status, Some(Status::Declined));
        assert!(!c.in_calendar);
    }

    #[test]
    fn the_gpui_answer_is_kept_without_touching_the_calendar_tables() {
        let both = format!("ATTENDEE;RSVP=TRUE:mailto:{EMAIL}\r\n");
        let (db, account, m, ev) = open(&invite("REQUEST", "ana@example.org", &both, ""));
        let events = |db: &Db| {
            db.event_by_uid(&ev.uid)
                .unwrap()
                .map(|(id, _)| id)
                .into_iter()
                .count()
        };
        assert_eq!(events(&db), 0);
        record_answer(&db, &account, &ev, Answer::Accepted).unwrap();
        // The answer shows on the card; nothing went into the calendar.
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), false);
        assert_eq!(c.status, Some(Status::Accepted));
        assert_eq!(c.answer, Some(Answer::Accepted));
        assert!(!c.in_calendar);
        assert_eq!(events(&db), 0);
        // Declining later changes the answer, still without the calendar.
        record_answer(&db, &account, &ev, Answer::Declined).unwrap();
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), false);
        assert_eq!(c.status, Some(Status::Declined));
        assert_eq!(events(&db), 0);
        // `record` (the calendar, back in 1.1.0) still stores it.
        record(&db, &account, &m, &ev, Answer::Accepted).unwrap();
        assert_eq!(events(&db), 1);
    }

    #[test]
    fn the_prompt_warns_of_a_foreign_organizer_and_an_unlisted_account() {
        let other = "ATTENDEE;RSVP=TRUE:mailto:carol@example.com\r\n";
        let (_, _, m, ev) = open(&invite("REQUEST", "x@elsewhere.example", other, ""));
        let p = prompt(EMAIL, &m, &ev);
        assert_eq!(
            p.domains,
            Some(("elsewhere.example".into(), "example.org".into()))
        );
        assert!(p.unlisted);
    }

    #[test]
    fn a_cancelled_invitation_offers_no_answers() {
        let both = format!("ATTENDEE;RSVP=TRUE:mailto:{EMAIL}\r\n");
        let (db, _, m, ev) = open(&invite(
            "CANCEL",
            "ana@example.org",
            &both,
            "STATUS:CANCELLED\r\n",
        ));
        let c = card(&db, Some(EMAIL), &m, &ev, Lang::En, &now(), false);
        assert!(c.cancelled && !c.answers);
        assert_eq!(c.status, Some(Status::Cancelled));
        assert!(!c.tools.contains(&Tool::Join));
        assert!(calendar_file(&m).is_some_and(|f| !f.is_empty()));
    }

    /// The cancelled flag of the stored copy of `uid`.
    fn stored_cancelled(db: &Db, uid: &str) -> bool {
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE uid = ?1 AND cancelled = 1",
                [uid],
                |r| r.get(0),
            )
            .unwrap();
        n > 0
    }

    #[test]
    fn opening_a_cancellation_does_not_write_the_calendar_tables() {
        let both = format!("ATTENDEE;RSVP=TRUE:mailto:{EMAIL}\r\n");
        // A copy stored earlier (a 0.7 cache, or the calendar of 1.1.0).
        let (db, account, m, ev) = open(&invite("REQUEST", "ana@example.org", &both, ""));
        record(&db, &account, &m, &ev, Answer::Accepted).unwrap();
        assert!(!stored_cancelled(&db, "plan@example.org"));
        // The organizer's cancellation (a higher SEQUENCE) arrives.
        let cancel = invite("CANCEL", "ana@example.org", &both, "STATUS:CANCELLED\r\n")
            .replace("SEQUENCE:1", "SEQUENCE:2");
        let (_, _, cm, cev) = open(&cancel);
        assert!(cev.cancelled());
        let c = card(&db, Some(EMAIL), &cm, &cev, Lang::En, &now(), false);
        assert_eq!(c.status, Some(Status::Cancelled));
        // The card read the table and wrote nothing.
        assert!(c.in_calendar);
        assert!(!stored_cancelled(&db, "plan@example.org"));
        // The calendar's own call (1.1.0) does mark it, once.
        assert!(mark_cancelled(&db, &cev).unwrap());
        assert!(stored_cancelled(&db, "plan@example.org"));
        assert!(!mark_cancelled(&db, &cev).unwrap());
        // A request is not a cancellation.
        assert!(!mark_cancelled(&db, &ev).unwrap());
    }
}
