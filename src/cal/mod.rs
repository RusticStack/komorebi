//! The calendar: events read from iCalendar resources (`mail::ical`),
//! their recurrences expanded ([`recur`]) and their times resolved
//! ([`tz`]); the local store and its queries ([`store`]).
//!
//! An occurrence is kept as UTC seconds. All-day and floating times are
//! placed with the current Windows zone at expansion time and flagged
//! ([`FLAG_LOCAL`]) so they can be placed again after a zone change.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

pub mod edit;
pub mod layout;
pub mod recur;
pub mod remind;
pub mod store;
pub mod tz;
mod tz_table;
pub mod write;

use crate::mail::ical::{Item, Resource, When};
use chrono::{Local, NaiveDateTime, TimeDelta, TimeZone};

/// Placed with the current zone (all-day or floating).
pub const FLAG_LOCAL: i64 = 1;
/// An all-day occurrence.
pub const FLAG_ALL_DAY: i64 = 2;
/// An edited single occurrence (an override).
pub const FLAG_OVERRIDE: i64 = 4;
/// The zone is unknown: the wall time is shown with its name.
pub const FLAG_UNKNOWN_ZONE: i64 = 8;

/// How a series was expanded (`events.recur`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecurState {
    None = 0,
    Expanded = 1,
    /// A rule this module doesn't expand: the first occurrence only.
    Refused = 2,
    /// Over the size cap: not read.
    TooLarge = 3,
}

/// One occurrence of an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub start: i64,
    pub end: i64,
    /// The occurrence's own start as written (`YYYYMMDDTHHMMSS`), its
    /// RECURRENCE-ID; empty for a single event.
    pub recurrence_id: String,
    pub flags: i64,
}

/// What an expansion gives for one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    pub occurrences: Vec<Occurrence>,
    pub state: RecurState,
    /// The end of the last occurrence (UTC) when the series ends; `None`
    /// for a series without end.
    pub series_end: Option<i64>,
}

/// The current Windows zone's instant for a wall time (the first one in
/// an overlap, the time after a gap).
pub fn local_utc(wall: NaiveDateTime) -> Option<i64> {
    match Local.from_local_datetime(&wall) {
        chrono::LocalResult::Single(t) => Some(t.timestamp()),
        chrono::LocalResult::Ambiguous(a, _) => Some(a.timestamp()),
        chrono::LocalResult::None => {
            let later = wall.checked_add_signed(TimeDelta::try_hours(1)?)?;
            Local
                .from_local_datetime(&later)
                .earliest()
                .map(|t| t.timestamp())
        }
    }
}

/// What the stored occurrences depend on besides the events: the month
/// (the window moves with it) and the current zone's offsets in January
/// and July of `now`'s year (a zone change). `None` if `now` is invalid.
pub fn window_key(now: i64) -> Option<String> {
    let t = chrono::DateTime::from_timestamp(now, 0)?.naive_utc();
    let off = |month: u32| -> Option<i64> {
        let w = chrono::NaiveDate::from_ymd_opt(chrono::Datelike::year(&t), month, 1)?
            .and_hms_opt(12, 0, 0)?;
        w.and_utc().timestamp().checked_sub(local_utc(w)?)
    };
    Some(format!("{}|{}|{}", t.format("%Y-%m"), off(1)?, off(7)?))
}

/// How the times of one event are placed.
struct Placer<'a> {
    res: &'a Resource,
    tzid: String,
    utc: bool,
    local: &'a dyn Fn(NaiveDateTime) -> Option<i64>,
    flags: i64,
}

impl Placer<'_> {
    fn new<'a>(
        res: &'a Resource,
        start: &When,
        local: &'a dyn Fn(NaiveDateTime) -> Option<i64>,
    ) -> Placer<'a> {
        let mut flags = 0;
        if start.all_day {
            flags |= FLAG_ALL_DAY | FLAG_LOCAL;
        } else if start.utc.is_none() {
            flags |= FLAG_LOCAL;
            if !start.tzid.is_empty() {
                flags |= FLAG_UNKNOWN_ZONE;
            }
        }
        Placer {
            res,
            tzid: start.tzid.clone(),
            utc: start.utc.is_some() && start.tzid.is_empty() && !start.all_day,
            local,
            flags,
        }
    }

    fn place(&self, wall: NaiveDateTime) -> Option<i64> {
        if self.utc {
            return Some(wall.and_utc().timestamp());
        }
        if self.flags & FLAG_LOCAL == 0 {
            return self.res.to_utc(&self.tzid, wall);
        }
        (self.local)(wall)
    }

    /// The wall time (in the event's zone) of an instant, for a UTC UNTIL.
    fn wall_of(&self, utc: NaiveDateTime) -> NaiveDateTime {
        if self.utc {
            return utc;
        }
        // The offset at that instant, from the zone at the same wall time.
        let at = utc.and_utc().timestamp();
        self.place(utc)
            .and_then(|t| at.checked_sub(t))
            .and_then(TimeDelta::try_seconds)
            .and_then(|off| utc.checked_add_signed(off))
            .unwrap_or(utc)
    }
}

/// The wall time of a RECURRENCE-ID ("`;TZID=...:20261008T140000`") and
/// whether it is UTC.
fn rid_wall(rid: &str) -> Option<(NaiveDateTime, bool)> {
    let v = rid.rsplit(':').next()?.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let date =
        chrono::NaiveDate::from_ymd_opt(i32::try_from(num(0, 4)?).ok()?, num(4, 6)?, num(6, 8)?)?;
    if v.len() == 8 {
        return Some((date.and_hms_opt(0, 0, 0)?, false));
    }
    let t = date.and_hms_opt(num(9, 11)?, num(11, 13)?, num(13, 15)?)?;
    Some((t, v.ends_with(['Z', 'z'])))
}

fn rid_key(wall: NaiveDateTime, all_day: bool) -> String {
    if all_day {
        wall.format("%Y%m%d").to_string()
    } else {
        wall.format("%Y%m%dT%H%M%S").to_string()
    }
}

fn span(item: &Item) -> TimeDelta {
    let ev = &item.event;
    match (&ev.end, ev.start.utc, ev.end.as_ref().and_then(|e| e.utc)) {
        (_, Some(s), Some(e)) => {
            TimeDelta::try_seconds(e.saturating_sub(s).max(0)).unwrap_or_default()
        }
        (Some(e), _, _) => recur::span(ev.start.wall, Some(e.wall)),
        // No end: a day for an all-day event, else none.
        (None, _, _) if ev.start.all_day => TimeDelta::try_days(1).unwrap_or_default(),
        (None, _, _) => TimeDelta::zero(),
    }
}

/// The occurrences of `res` that overlap `[from, to)` (UTC seconds),
/// placing local times with `local` (the current zone; a fixture in
/// tests).
pub fn expand(
    res: &Resource,
    from: i64,
    to: i64,
    local: &dyn Fn(NaiveDateTime) -> Option<i64>,
) -> Expanded {
    let mut out = Vec::new();
    let mut state = RecurState::None;
    let mut series_end = None;
    let mut taken: Vec<String> = Vec::new();
    // Overrides first: they replace their occurrence of the series.
    for o in &res.overrides {
        let p = Placer::new(res, &o.event.start, local);
        if let Some((wall, utc)) = rid_wall(&o.event.recurrence_id) {
            let key = if utc {
                let master = res
                    .master
                    .as_ref()
                    .map(|m| Placer::new(res, &m.event.start, local));
                let w = master.map_or(wall, |m| m.wall_of(wall));
                rid_key(w, o.event.start.all_day)
            } else {
                rid_key(wall, o.event.start.all_day)
            };
            taken.push(key.clone());
            if o.event.cancelled() {
                continue;
            }
            let Some(start) = p.place(o.event.start.wall) else {
                continue;
            };
            let end = start.saturating_add(span(o).num_seconds());
            if start < to && end.max(start.saturating_add(1)) > from {
                out.push(Occurrence {
                    start,
                    end,
                    recurrence_id: key,
                    flags: p.flags | FLAG_OVERRIDE,
                });
            }
        }
    }
    if let Some(m) = &res.master {
        let p = Placer::new(res, &m.event.start, local);
        let len = span(m);
        let dtstart = m.event.start.wall;
        let all_day = m.event.start.all_day;
        // The range in wall time, a day wider each side for the offsets.
        let wall = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.naive_utc());
        let day = TimeDelta::try_days(2).unwrap_or_default();
        let lo = wall(from)
            .and_then(|w| w.checked_sub_signed(day))
            .and_then(|w| w.checked_sub_signed(len));
        let hi = wall(to).and_then(|w| w.checked_add_signed(day));
        let mut starts = vec![dtstart];
        if !m.event.rrule.is_empty() || !m.rdates.is_empty() {
            let rule = if m.event.rrule.is_empty() {
                // RDATE only: a rule that yields DTSTART alone.
                recur::Rule::parse("FREQ=DAILY;COUNT=1", dtstart)
            } else {
                recur::Rule::parse(&m.event.rrule, dtstart)
            };
            match (rule, lo, hi) {
                (Ok(rule), Some(lo), Some(hi)) => {
                    let ex: Vec<NaiveDateTime> = m.exdates.iter().map(|w| wall_in(&p, w)).collect();
                    let rd: Vec<NaiveDateTime> = m.rdates.iter().map(|w| wall_in(&p, w)).collect();
                    let e = recur::expand(&rule, dtstart, lo, hi, &ex, &rd, &|u| p.wall_of(u));
                    starts = e.starts;
                    state = RecurState::Expanded;
                    series_end = e
                        .series_end
                        .and_then(|w| p.place(w))
                        .map(|s| s.saturating_add(len.num_seconds()));
                    if m.event.rrule.is_empty() {
                        series_end = None;
                    }
                }
                _ => state = RecurState::Refused,
            }
        } else {
            series_end = p
                .place(dtstart)
                .map(|s| s.saturating_add(len.num_seconds()));
        }
        for w in starts {
            let key = if state == RecurState::None {
                String::new()
            } else {
                rid_key(w, all_day)
            };
            if !key.is_empty() && taken.contains(&key) {
                continue;
            }
            let Some(start) = p.place(w) else {
                continue;
            };
            let end = start.saturating_add(len.num_seconds());
            if start < to && end.max(start.saturating_add(1)) > from {
                out.push(Occurrence {
                    start,
                    end,
                    recurrence_id: key,
                    flags: p.flags,
                });
            }
        }
    }
    out.sort_by_key(|o| o.start);
    Expanded {
        occurrences: out,
        state,
        series_end,
    }
}

/// A date of EXDATE or RDATE as a wall time in the event's zone.
fn wall_in(p: &Placer, w: &When) -> NaiveDateTime {
    match w.utc {
        Some(t) if !p.utc && p.flags & FLAG_LOCAL == 0 && w.tzid.is_empty() => {
            chrono::DateTime::from_timestamp(t, 0).map_or(w.wall, |d| p.wall_of(d.naive_utc()))
        }
        _ => w.wall,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::mail::ical::parse_resource;

    fn ts(s: &str) -> i64 {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
            .unwrap()
            .and_utc()
            .timestamp()
    }

    /// Lisbon as the "current" zone, by fixture rule.
    fn lisbon(w: NaiveDateTime) -> Option<i64> {
        let r = tz::Rule {
            bias: 0,
            std_bias: 0,
            dst_bias: -60,
            std_date: Some(tz::Transition {
                month: 10,
                week: 5,
                weekday: 0,
                hour: 2,
                minute: 0,
            }),
            dst_date: Some(tz::Transition {
                month: 3,
                week: 5,
                weekday: 0,
                hour: 1,
                minute: 0,
            }),
        };
        w.and_utc().timestamp().checked_sub(r.offset(w)?)
    }

    const NY: &str = "BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\n\
        BEGIN:DAYLIGHT\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nDTSTART:20070311T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nEND:DAYLIGHT\r\n\
        BEGIN:STANDARD\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nDTSTART:20071104T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nEND:STANDARD\r\n\
        END:VTIMEZONE\r\n";

    fn cal(body: &str) -> Resource {
        let raw = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{NY}{body}END:VCALENDAR\r\n");
        parse_resource(raw.as_bytes()).unwrap()
    }

    #[test]
    fn weekly_across_dst_keeps_wall_time() {
        let r = cal(
            "BEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=America/New_York:20260302T090000\r\n\
            DTEND;TZID=America/New_York:20260302T100000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\n",
        );
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        assert_eq!(e.state, RecurState::Expanded);
        let starts: Vec<i64> = e.occurrences.iter().map(|o| o.start).collect();
        // 09:00 EST, then 09:00 EDT after 8 March.
        assert_eq!(
            starts,
            [
                ts("2026-03-02 14:00"),
                ts("2026-03-09 13:00"),
                ts("2026-03-16 13:00")
            ]
        );
        assert_eq!(e.occurrences[0].end - e.occurrences[0].start, 3600);
        assert_eq!(e.series_end, Some(ts("2026-03-16 14:00")));
    }

    #[test]
    fn spring_gap_and_fall_overlap_through_vtimezone() {
        let r = cal(
            "BEGIN:VEVENT\r\nUID:g\r\nDTSTART;TZID=America/New_York:20260308T023000\r\nEND:VEVENT\r\n",
        );
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        // 02:30 doesn't exist: the offset before the gap (-05:00).
        assert_eq!(e.occurrences[0].start, ts("2026-03-08 07:30"));
        let r = cal(
            "BEGIN:VEVENT\r\nUID:o\r\nDTSTART;TZID=America/New_York:20261101T013000\r\nEND:VEVENT\r\n",
        );
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        // 01:30 happens twice: the first (EDT, -04:00).
        assert_eq!(e.occurrences[0].start, ts("2026-11-01 05:30"));
    }

    #[test]
    fn overrides_exdates_and_cancelled_instances() {
        let r = cal(
            "BEGIN:VEVENT\r\nUID:s\r\nDTSTART:20260601T090000Z\r\nDURATION:PT30M\r\nRRULE:FREQ=DAILY;COUNT=5\r\n\
            EXDATE:20260603T090000Z\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20260602T090000Z\r\nDTSTART:20260602T150000Z\r\nDURATION:PT1H\r\nSUMMARY:Moved\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20260604T090000Z\r\nDTSTART:20260604T090000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20260605T090000Z\r\nDTSTART:20260605T100000Z\r\nEND:VEVENT\r\n",
        );
        assert_eq!(r.overrides.len(), 3);
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        let got: Vec<(i64, i64)> = e
            .occurrences
            .iter()
            .map(|o| (o.start, o.flags & FLAG_OVERRIDE))
            .collect();
        assert_eq!(
            got,
            [
                (ts("2026-06-01 09:00"), 0),
                (ts("2026-06-02 15:00"), FLAG_OVERRIDE),
                (ts("2026-06-05 10:00"), FLAG_OVERRIDE),
            ]
        );
        assert_eq!(e.occurrences[1].end - e.occurrences[1].start, 3600);
    }

    #[test]
    fn all_day_and_refused() {
        let r = cal(
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART;VALUE=DATE:20260715\r\nDTEND;VALUE=DATE:20260717\r\nEND:VEVENT\r\n",
        );
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        let o = &e.occurrences[0];
        // Local midnight in Lisbon summer (UTC+1), two days long.
        assert_eq!(o.start, ts("2026-07-14 23:00"));
        assert_eq!(o.end - o.start, 2 * 86_400);
        assert_eq!(
            o.flags & (FLAG_ALL_DAY | FLAG_LOCAL),
            FLAG_ALL_DAY | FLAG_LOCAL
        );
        let r = cal(
            "BEGIN:VEVENT\r\nUID:h\r\nDTSTART:20260715T090000Z\r\nRRULE:FREQ=HOURLY\r\nEND:VEVENT\r\n",
        );
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        assert_eq!(e.state, RecurState::Refused);
        assert_eq!(e.occurrences.len(), 1);
    }

    #[test]
    fn unknown_zone_is_placed_as_local_and_flagged() {
        let r = cal(
            "BEGIN:VEVENT\r\nUID:u\r\nDTSTART;TZID=Mars/Base:20260115T090000\r\nEND:VEVENT\r\n",
        );
        let e = expand(&r, ts("2026-01-01 00:00"), ts("2027-01-01 00:00"), &lisbon);
        assert_eq!(e.occurrences[0].start, ts("2026-01-15 09:00"));
        assert!(e.occurrences[0].flags & FLAG_UNKNOWN_ZONE != 0);
    }

    #[test]
    fn many_series_expand_quickly() {
        let mut body = String::new();
        for i in 0..200 {
            body.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:{i}\r\nDTSTART;TZID=America/New_York:20250106T090000\r\nRRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\n"
            ));
        }
        let raw = format!("BEGIN:VCALENDAR\r\n{NY}{body}END:VCALENDAR\r\n");
        // One resource per UID in the store; here one parse each.
        let started = std::time::Instant::now();
        let r = parse_resource(raw.as_bytes()).unwrap();
        let mut n = 0;
        for _ in 0..200 {
            n += expand(&r, ts("2026-01-01 00:00"), ts("2029-01-01 00:00"), &lisbon)
                .occurrences
                .len();
        }
        assert_eq!(n, 200 * 156);
        // Generous for the debug build; the release target is 20 ms.
        assert!(started.elapsed().as_secs() < 10);
    }
}
