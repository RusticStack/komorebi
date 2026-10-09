//! The synthetic meeting invitations in `tests/fixtures/invite/`: what the
//! reader's invite card reads from each, time conversion, and a byte
//! mutation loop over every calendar part (no panic, no hang).

use chrono::{FixedOffset, TimeZone, Utc};
use komorebi::mail::ical::{self, Event};
use komorebi::mail::mime;
use std::time::{Duration, Instant};

const ME: &str = "seed@komorebi.invalid";

fn raw(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/fixtures/invite/{name}.eml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn invite(name: &str) -> Event {
    mime::parse_body(&raw(name))
        .invite
        .map_or_else(|| panic!("{name}: no invite"), |ev| *ev)
}

/// Before every fixture's event (they are in 2027, or recur until then).
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 9, 12, 0, 0).unwrap()
}

/// Whether the card offers Accept, Tentative and Decline.
fn answerable(ev: &Event) -> bool {
    ev.answerable(ev.me(ME, &[]), ev.past(&now()))
}

#[test]
fn google_request() {
    let ev = invite("01-google-request");
    assert_eq!(ev.method, "REQUEST");
    assert_eq!(ev.summary, "Quarterly planning");
    assert_eq!(ev.uid, "0a1b2c3d4e5f@google.com");
    assert_eq!(ev.location, "Room 2.04, Lisbon office");
    assert_eq!(ev.join, "https://meet.google.com/abc-defg-hij");
    assert!(ev.description.starts_with("Agenda:\n- review Q1 numbers"));
    assert_eq!(ev.organizer.as_ref().unwrap().addr, "ana.silva@example.com");
    assert_eq!(ev.attendees.len(), 3);
    assert_eq!(ev.me(ME, &[]).unwrap().partstat, "NEEDS-ACTION");
    assert!(answerable(&ev));
    // Lisbon in March is UTC+0.
    let lisbon_winter = FixedOffset::east_opt(0).unwrap();
    assert_eq!(ev.when_text(&lisbon_winter), "Tue 2027-03-16 14:00-15:00");
    // Save .ics takes the calendar part; the list shows only invite.ics.
    let names: Vec<_> = mime::parse_body(&raw("01-google-request"))
        .attachments
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["invite.ics"]);
    let file = mime::calendar_file(&raw("01-google-request")).unwrap();
    assert!(file.starts_with(b"BEGIN:VCALENDAR"));
}

#[test]
fn outlook_request_with_windows_zone() {
    let ev = invite("02-outlook-request");
    assert_eq!(ev.summary, "Design review");
    assert_eq!(ev.organizer.as_ref().unwrap().name, "Mendes, Rui");
    assert_eq!(
        ev.join,
        "https://teams.example.com/l/meetup-join/19%3ameeting_abc"
    );
    assert!(ev.uid.ends_with("AABBCCDDEEFF") && !ev.uid.contains(' '));
    // 24 March 2027 is before the last Sunday of March: UTC+1.
    let start = Utc.with_ymd_and_hms(2027, 3, 24, 9, 0, 0).unwrap();
    assert_eq!(ev.start.utc, Some(start.timestamp()));
    assert_eq!(ev.end.as_ref().unwrap().utc, Some(start.timestamp() + 3600));
    assert!(ev.start.zone.is_empty());
    assert!(answerable(&ev));
}

#[test]
fn vtimezone_across_a_dst_change() {
    // The same VTIMEZONE: after the last Sunday of March it is UTC+2, and
    // back to UTC+1 after the last Sunday of October.
    let text = String::from_utf8(raw("02-outlook-request")).unwrap();
    let ics = mime::calendar_file(text.as_bytes()).unwrap();
    let ics = String::from_utf8(ics).unwrap();
    let at = |wall: &str| {
        let moved = ics
            .replace("20270324T100000", wall)
            .replace("20270324T110000", wall);
        ical::parse(moved.as_bytes()).unwrap().start.utc.unwrap()
    };
    let utc = |m, d, h| {
        Utc.with_ymd_and_hms(2027, m, d, h, 0, 0)
            .unwrap()
            .timestamp()
    };
    assert_eq!(at("20270328T010000"), utc(3, 28, 0));
    assert_eq!(at("20270329T100000"), utc(3, 29, 8));
    assert_eq!(at("20271030T100000"), utc(10, 30, 8));
    assert_eq!(at("20271101T100000"), utc(11, 1, 9));
    assert_eq!(at("20270115T100000"), utc(1, 15, 9));
    // The hour that doesn't exist (02:30 on 28 March) and the one that
    // happens twice (02:30 on 31 October) still resolve, without panic.
    assert!(at("20270328T023000") > 0);
    assert!(at("20271031T023000") > 0);
}

#[test]
fn cancel_has_no_rsvp() {
    let ev = invite("03-cancel");
    assert!(ev.cancelled());
    assert_eq!(ev.sequence, 1);
    assert!(!answerable(&ev));
}

#[test]
fn update_asks_again() {
    let ev = invite("04-update");
    assert_eq!(ev.sequence, 2);
    assert!(!ev.cancelled() && answerable(&ev));
    // The meeting link is in LOCATION.
    assert_eq!(ev.join, "https://meet.example.com/budget-sync");
}

#[test]
fn publish_ics_file() {
    let ev = invite("05-publish-ics-file");
    assert_eq!(ev.method, "PUBLISH");
    assert_eq!(ev.join, "https://meetup.example.net/events/rust-prod-2027");
    assert_eq!(ev.location, "Auditorium, Rua Exemplo 12, Lisbon");
    // DURATION gives the end.
    let end = ev.end.as_ref().unwrap().utc.unwrap();
    assert_eq!(end - ev.start.utc.unwrap(), 2 * 3600);
    assert!(!answerable(&ev), "nothing to answer");
    let utc = FixedOffset::east_opt(0).unwrap();
    assert_eq!(ev.when_text(&utc), "Thu 2027-04-22 17:30-19:30");
}

#[test]
fn all_day() {
    let ev = invite("06-all-day");
    assert!(ev.start.all_day);
    let any = FixedOffset::west_opt(5 * 3600).unwrap();
    assert_eq!(
        ev.when_text(&any),
        "Sat 2027-04-10 - Sun 2027-04-11, all day"
    );
    assert!(answerable(&ev));
    // Over once the last day has passed, wherever the user is.
    let monday = FixedOffset::east_opt(0)
        .unwrap()
        .with_ymd_and_hms(2027, 4, 12, 0, 30, 0)
        .unwrap();
    assert!(ev.past(&monday));
    let sunday = monday - chrono::TimeDelta::try_hours(1).unwrap();
    assert!(!ev.past(&sunday));
}

#[test]
fn recurring_weekly_with_an_unknown_zone() {
    let ev = invite("07-recurring-weekly");
    assert_eq!(ev.repeats(), "Repeats weekly on Mon, Wed until 2027-12-31");
    // No VTIMEZONE: the wall time, labelled with the zone.
    assert_eq!(ev.start.utc, None);
    let far = FixedOffset::east_opt(9 * 3600).unwrap();
    assert_eq!(
        ev.when_text(&far),
        "Mon 2026-10-12 09:30-09:45 (Europe/Lisbon)"
    );
    // Its first meeting is past, but the series goes on until its end.
    let later = Utc.with_ymd_and_hms(2027, 6, 1, 0, 0, 0).unwrap();
    assert!(!ev.past(&later) && answerable(&ev));
    let after = Utc.with_ymd_and_hms(2028, 1, 2, 0, 0, 0).unwrap();
    assert!(ev.past(&after));
}

#[test]
fn malformed_and_oversized_fall_back() {
    let body = mime::parse_body(&raw("08-malformed"));
    assert!(body.invite.is_none());
    assert_eq!(body.text, "The calendar part below is not valid iCalendar.");
    // A part over the cap is not read at all.
    let mut ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:big@x\r\n\
                   DTSTART:20270101T100000Z\r\nSUMMARY:Big\r\n"
        .to_string();
    let small = format!("{ics}END:VEVENT\r\nEND:VCALENDAR\r\n");
    assert!(ical::parse(small.as_bytes()).is_some());
    while ics.len() <= ical::MAX_BYTES {
        ics.push_str("X-PAD:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
    }
    ics.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    assert!(ical::parse(ics.as_bytes()).is_none());
    let msg =
        format!("From: a@example.com\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\n{ics}");
    assert!(mime::parse_body(msg.as_bytes()).invite.is_none());
}

#[test]
fn me_falls_back_to_the_message_recipients() {
    let ev = invite("02-outlook-request");
    assert_eq!(
        ev.me("other@komorebi.invalid", &["ana.silva@example.com"])
            .unwrap()
            .addr,
        "ana.silva@example.com"
    );
    assert!(ev.me("other@komorebi.invalid", &["x@y.z"]).is_none());
    // Two attendees among the recipients: neither is taken for the user.
    let both = ["seed@komorebi.invalid", "ana.silva@example.com"];
    assert!(ev.me("other@komorebi.invalid", &both).is_none());
    // The account's own address always wins.
    assert_eq!(ev.me(ME, &both).unwrap().addr, ME);
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[test]
fn mutated_calendars_never_panic_or_hang() {
    let seeds: Vec<Vec<u8>> = [
        "01-google-request",
        "02-outlook-request",
        "03-cancel",
        "05-publish-ics-file",
        "06-all-day",
        "07-recurring-weekly",
    ]
    .iter()
    .map(|n| mime::calendar_file(&raw(n)).unwrap())
    .collect();
    let iters: usize = std::env::var("KOMOREBI_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let interesting: &[&[u8]] = &[
        b"\r\n ",
        b";",
        b":",
        b"\"",
        b"=",
        b"\\",
        b"BEGIN:VEVENT\r\n",
        b"END:VEVENT\r\n",
        b"BEGIN:VTIMEZONE\r\n",
        b"TZID=",
        b"RRULE:FREQ=YEARLY;BYDAY=-9SU;BYMONTH=13\r\n",
        b"DURATION:P99999999999W\r\n",
        b"TZOFFSETTO:+9999\r\n",
        b"\xff\xfe",
        "\u{202E}".as_bytes(),
    ];
    let started = Instant::now();
    for i in 0..iters {
        let seed = rng.below(seeds.len());
        let mut m = seeds[seed].clone();
        for _ in 0..1 + rng.below(8) {
            let at = rng.below(m.len() + 1);
            match rng.below(4) {
                0 => m.truncate(at),
                1 if at < m.len() => m[at] = rng.next() as u8,
                2 => {
                    let ins = interesting[rng.below(interesting.len())];
                    m.splice(at..at, ins.iter().copied());
                }
                _ => {
                    let from = rng.below(m.len() + 1);
                    let to = (from + rng.below(200)).min(m.len());
                    let chunk = m[from..to].to_vec();
                    m.splice(at..at, chunk);
                }
            }
        }
        let t = Instant::now();
        if let Some(ev) = ical::parse(&m) {
            let utc = FixedOffset::east_opt(0).unwrap();
            ev.when_text(&utc);
            ev.repeats();
            ev.past(&now());
            ical::reply(&ev, "Me", ME, ical::Answer::Declined, now());
        }
        assert!(
            t.elapsed() < Duration::from_millis(500),
            "mutant {i} of seed {seed} took {:?}",
            t.elapsed()
        );
    }
    assert!(started.elapsed() < Duration::from_secs(60));
}
