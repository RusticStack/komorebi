//! The user's own changes in the store: overrides and EXDATEs written into
//! the resource with every other line kept, and the local calendar's
//! changes never queued.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use crate::cal::edit::{Form, edit};
use chrono::{DateTime, NaiveDate};

const NOW: i64 = 1_790_000_000; // 2026-09-21

const SERIES: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Other//EN\r\nX-WR-CALNAME:Mine\r\n\
BEGIN:VEVENT\r\nUID:s@x\r\nDTSTART:20261005T090000Z\r\nDTEND:20261005T100000Z\r\n\
SUMMARY:Stand-up\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nX-APPLE-STRUCTURED-LOCATION;VALUE=URI:geo:1,2\r\n\
BEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER:-PT5M\r\nX-APPLE-DEFAULT-ALARM:TRUE\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

fn form(summary: &str, day: u32, h: u32) -> Form {
    let start = NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(h, 0, 0)
        .unwrap();
    Form {
        summary: summary.into(),
        location: String::new(),
        description: String::new(),
        all_day: false,
        start,
        end: start + chrono::TimeDelta::hours(1),
        rrule: String::new(),
        alarm_min: None,
    }
}

#[test]
fn one_occurrence_edited_and_one_left_out() {
    let db = Db::in_memory().unwrap();
    let cal = db.local_calendar().unwrap();
    let id = db.write_event(cal, None, SERIES, "", NOW).unwrap();
    let rows = db.agenda(NOW, NOW + 60 * 86_400).unwrap();
    assert_eq!(rows.len(), 4);
    // The second occurrence moved to 15:00 (UTC fixture).
    let key = db.occurrence_key(id, rows[1].start).unwrap().unwrap();
    assert_eq!(key, "20261012T090000");
    let e = edit(
        &form("Stand-up, late", 12, 15),
        "s@x",
        DateTime::from_timestamp(NOW, 0).unwrap(),
        &|w| Some(w.and_utc().timestamp()),
    )
    .unwrap();
    db.edit_occurrence(id, &key, &e, "", NOW).unwrap();
    let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
    assert!(ics.contains("RECURRENCE-ID:20261012T090000Z\r\n"), "{ics}");
    for kept in [
        "X-WR-CALNAME:Mine",
        "X-APPLE-STRUCTURED-LOCATION",
        "X-APPLE-DEFAULT-ALARM",
    ] {
        assert!(ics.contains(kept), "{kept} lost");
    }
    let rows = db.agenda(NOW, NOW + 60 * 86_400).unwrap();
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .any(|r| r.flags & super::super::FLAG_OVERRIDE != 0 && r.start % 86_400 == 15 * 3600)
    );
    // Editing the same occurrence again replaces its override.
    db.edit_occurrence(id, &key, &e, "", NOW).unwrap();
    let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
    assert_eq!(ics.matches("RECURRENCE-ID").count(), 1);
    // Leaving the edited one out: an EXDATE, and its override goes.
    db.delete_occurrence(id, &key, NOW).unwrap();
    let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
    assert!(ics.contains("EXDATE:20261012T090000Z\r\n"));
    assert!(!ics.contains("RECURRENCE-ID"));
    assert!(ics.contains("X-APPLE-DEFAULT-ALARM"));
    assert_eq!(db.agenda(NOW, NOW + 60 * 86_400).unwrap().len(), 3);
    // The local calendar sends nothing.
    assert_eq!(db.unsent_changes(None).unwrap(), 0);
    db.delete_event(id, false, NOW).unwrap();
    assert!(db.agenda(NOW, NOW + 60 * 86_400).unwrap().is_empty());
}

#[test]
fn recurrence_ids_follow_the_series_form() {
    let w = |all_day: bool, utc: Option<i64>, tzid: &str| When {
        wall: NaiveDate::from_ymd_opt(2026, 10, 5)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap(),
        utc,
        all_day,
        zone: String::new(),
        tzid: tzid.into(),
    };
    assert_eq!(
        rid_for(&w(true, None, ""), "20261012"),
        ";VALUE=DATE:20261012"
    );
    assert_eq!(
        rid_for(&w(false, Some(1), ""), "20261012T090000"),
        ":20261012T090000Z"
    );
    assert_eq!(
        rid_for(&w(false, Some(1), "Europe/Lisbon"), "20261012T090000"),
        ";TZID=Europe/Lisbon:20261012T090000"
    );
    assert_eq!(
        rid_for(&w(false, None, ""), "20261012T090000"),
        ":20261012T090000"
    );
    assert_eq!(file_name("a b/c@x"), "a-b-c-x.ics");
}

#[test]
fn a_zoned_series_over_the_dst_change_keeps_its_wall_time() {
    // A weekly 09:00 Lisbon series over the October change: an override
    // on the week after the change keeps its wall time.
    let db = Db::in_memory().unwrap();
    let cal = db.local_calendar().unwrap();
    let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Lisbon\r\n\
BEGIN:STANDARD\r\nDTSTART:19701025T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\n\
DTSTART:19700329T010000\r\nTZOFFSETFROM:+0000\r\nTZOFFSETTO:+0100\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:dst\r\nDTSTART;TZID=Europe/Lisbon:20261019T090000\r\n\
DTEND;TZID=Europe/Lisbon:20261019T100000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nSUMMARY:Gym\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
    let id = db.write_event(cal, None, raw, "", NOW).unwrap();
    let rows = db.agenda(NOW, NOW + 60 * 86_400).unwrap();
    assert_eq!(rows.len(), 3);
    // 09:00 summer time is 08:00 UTC; after the change 09:00 UTC.
    assert_eq!(rows[0].start % 86_400, 8 * 3600);
    assert_eq!(rows[1].start % 86_400, 9 * 3600);
    let key = db.occurrence_key(id, rows[1].start).unwrap().unwrap();
    assert_eq!(key, "20261026T090000");
    db.delete_occurrence(id, &key, NOW).unwrap();
    let ics = String::from_utf8(db.event_ics(id).unwrap().unwrap()).unwrap();
    assert!(
        ics.contains("EXDATE;TZID=Europe/Lisbon:20261026T090000\r\n"),
        "{ics}"
    );
    let rows = db.agenda(NOW, NOW + 60 * 86_400).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].start % 86_400, 9 * 3600);
}
