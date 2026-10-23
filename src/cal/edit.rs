//! The event editor's model: the simple repeat rules it offers, reading
//! an existing rule back into one of them, the date and time fields, and
//! the resource fields ([`Edit`]) a filled-in form becomes.
//!
//! Times are written so every client reads them the same way: an all-day
//! event as dates (the end the day after the last), a single timed event
//! as UTC, and a repeating timed event as local wall-clock time (floating),
//! so it keeps its hour across daylight saving changes.

use crate::mail::ical::{Edit, When};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc, Weekday};

/// A repeat the editor offers. "Custom" is not offered; a rule that is
/// none of these is kept as it is and shown read only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    None = 0,
    Daily = 1,
    /// Every week on the start's weekday.
    Weekly = 2,
    /// Every month on the start's day of the month.
    MonthlyDay = 3,
    /// Every month on the start's weekday of its week (the second Tuesday,
    /// or the last Friday when it is the month's last).
    MonthlyNth = 4,
    Yearly = 5,
}

impl Repeat {
    pub const ALL: [Repeat; 6] = [
        Repeat::None,
        Repeat::Daily,
        Repeat::Weekly,
        Repeat::MonthlyDay,
        Repeat::MonthlyNth,
        Repeat::Yearly,
    ];

    pub fn from_code(code: i32) -> Repeat {
        Repeat::ALL
            .into_iter()
            .find(|r| *r as i32 == code)
            .unwrap_or(Repeat::None)
    }
}

/// The week of the month of `d` as an RRULE ordinal: 1 to 4, or -1 when
/// it is that weekday's last in the month.
pub fn nth_of(d: NaiveDate) -> i8 {
    let next = d.checked_add_signed(TimeDelta::days(7));
    if next.is_none_or(|n| n.month() != d.month()) {
        return -1;
    }
    i8::try_from(d.day0().checked_div(7).unwrap_or(0)).map_or(1, |w| w.saturating_add(1))
}

fn day_code(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// The RRULE of `r` for a series starting on `start` (empty for none).
pub fn rrule(r: Repeat, start: NaiveDate) -> String {
    match r {
        Repeat::None => String::new(),
        Repeat::Daily => "FREQ=DAILY".into(),
        Repeat::Weekly => format!("FREQ=WEEKLY;BYDAY={}", day_code(start.weekday())),
        Repeat::MonthlyDay => format!("FREQ=MONTHLY;BYMONTHDAY={}", start.day()),
        Repeat::MonthlyNth => format!(
            "FREQ=MONTHLY;BYDAY={}{}",
            nth_of(start),
            day_code(start.weekday())
        ),
        Repeat::Yearly => "FREQ=YEARLY".into(),
    }
}

/// A rule's parts upper-cased and sorted, `INTERVAL=1` and `WKST` left
/// out (they don't change these rules).
fn norm(rule: &str) -> Vec<String> {
    let mut parts: Vec<String> = rule
        .split(';')
        .map(|p| p.trim().to_ascii_uppercase())
        .filter(|p| !p.is_empty() && p != "INTERVAL=1" && !p.starts_with("WKST="))
        .collect();
    parts.sort();
    parts
}

/// Which offered repeat `rule` is for a series starting on `start`;
/// `None` when it is none of them (an end, an interval, several days...).
pub fn repeat_of(rule: &str, start: NaiveDate) -> Option<Repeat> {
    let have = norm(rule);
    if have.is_empty() {
        return Some(Repeat::None);
    }
    for r in Repeat::ALL {
        if norm(&rrule(r, start)) == have {
            return Some(r);
        }
    }
    // The short forms other clients write (the day implied by DTSTART).
    match have.as_slice() {
        [f] if f == "FREQ=WEEKLY" => Some(Repeat::Weekly),
        [f] if f == "FREQ=MONTHLY" => Some(Repeat::MonthlyDay),
        _ => None,
    }
}

/// A date typed in the editor: `2026-10-23` always, and with slashes
/// day first (`23/10/2026`) when `day_first`, else month first.
pub fn parse_date(s: &str, day_first: bool) -> Option<NaiveDate> {
    let s = s.trim();
    let nums: Vec<u32> = s
        .split(['-', '/', '.'])
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let (y, m, d) = match nums.as_slice() {
        [y, m, d] if s.contains('-') => (*y, *m, *d),
        [a, b, y] if day_first => (*y, *b, *a),
        [a, b, y] => (*y, *a, *b),
        _ => return None,
    };
    if !(1900..=2200).contains(&y) {
        return None;
    }
    NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, m, d)
}

/// A date as the editor shows it (what [`parse_date`] reads back).
pub fn show_date(d: NaiveDate, day_first: bool) -> String {
    if day_first {
        d.format("%d/%m/%Y").to_string()
    } else {
        d.format("%Y-%m-%d").to_string()
    }
}

/// A 24-hour time typed in the editor: `9:30`, `09:30`, `9.30` or `9`.
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    let s = s.trim();
    let (h, m) = match s.split_once([':', '.', 'h']) {
        Some((h, m)) => (h.trim(), m.trim()),
        None => (s, "0"),
    };
    let m = if m.is_empty() { "0" } else { m };
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// What the editor holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub summary: String,
    pub location: String,
    pub description: String,
    pub all_day: bool,
    /// The first day and time (wall clock, this PC's zone).
    pub start: NaiveDateTime,
    /// The end; for an all-day event, the last day (inclusive).
    pub end: NaiveDateTime,
    /// The rule, as [`rrule`] writes it or kept from the event.
    pub rrule: String,
    pub alarm_min: Option<i64>,
}

/// Why a form can't be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormError {
    /// The end is before the start.
    EndBeforeStart,
    /// A time that doesn't exist here (`to_utc` failed).
    BadTime,
}

/// The resource fields for `form`, with `to_utc` placing a local wall
/// time (the current zone).
pub fn edit(
    form: &Form,
    uid: &str,
    now: DateTime<Utc>,
    to_utc: &dyn Fn(NaiveDateTime) -> Option<i64>,
) -> Result<Edit, FormError> {
    let when = |wall: NaiveDateTime, utc: Option<i64>| When {
        wall,
        utc,
        all_day: form.all_day,
        zone: String::new(),
        tzid: String::new(),
    };
    let (start, end) = if form.all_day {
        let first = form.start.date();
        let last = form.end.date();
        if last < first {
            return Err(FormError::EndBeforeStart);
        }
        let after = last
            .succ_opt()
            .ok_or(FormError::BadTime)?
            .and_time(NaiveTime::MIN);
        (
            when(first.and_time(NaiveTime::MIN), None),
            when(after, None),
        )
    } else {
        if form.end < form.start {
            return Err(FormError::EndBeforeStart);
        }
        if form.rrule.is_empty() {
            let s = to_utc(form.start).ok_or(FormError::BadTime)?;
            let e = to_utc(form.end).ok_or(FormError::BadTime)?;
            (when(form.start, Some(s)), when(form.end, Some(e.max(s))))
        } else {
            (when(form.start, None), when(form.end, None))
        }
    };
    Ok(Edit {
        uid: uid.to_string(),
        summary: form.summary.clone(),
        location: form.location.clone(),
        description: form.description.clone(),
        start,
        end: Some(end),
        rrule: form.rrule.clone(),
        alarm_min: form.alarm_min,
        now,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::mail::ical::{parse_resource, write_event};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn rules_round_trip_and_odd_ones_are_kept_apart() {
        let tue = d(2026, 10, 13); // the second Tuesday
        assert_eq!(rrule(Repeat::Weekly, tue), "FREQ=WEEKLY;BYDAY=TU");
        assert_eq!(rrule(Repeat::MonthlyNth, tue), "FREQ=MONTHLY;BYDAY=2TU");
        assert_eq!(rrule(Repeat::MonthlyDay, tue), "FREQ=MONTHLY;BYMONTHDAY=13");
        // The last Friday of October 2026 is the 30th: -1, not 5.
        assert_eq!(nth_of(d(2026, 10, 30)), -1);
        assert_eq!(nth_of(d(2026, 10, 1)), 1);
        assert_eq!(nth_of(d(2026, 10, 22)), 4);
        for r in Repeat::ALL {
            assert_eq!(repeat_of(&rrule(r, tue), tue), Some(r));
        }
        assert_eq!(
            repeat_of("byday=TU;freq=weekly;INTERVAL=1;WKST=MO", tue),
            Some(Repeat::Weekly)
        );
        assert_eq!(repeat_of("FREQ=WEEKLY", tue), Some(Repeat::Weekly));
        assert_eq!(repeat_of("FREQ=WEEKLY;BYDAY=WE", tue), None);
        assert_eq!(repeat_of("FREQ=DAILY;COUNT=5", tue), None);
        assert_eq!(repeat_of("FREQ=WEEKLY;INTERVAL=2", tue), None);
        assert_eq!(Repeat::from_code(4), Repeat::MonthlyNth);
        assert_eq!(Repeat::from_code(99), Repeat::None);
    }

    #[test]
    fn dates_and_times_typed() {
        assert_eq!(parse_date("2026-10-23", true), Some(d(2026, 10, 23)));
        assert_eq!(parse_date("23/10/2026", true), Some(d(2026, 10, 23)));
        assert_eq!(parse_date("10/23/2026", false), Some(d(2026, 10, 23)));
        assert_eq!(parse_date("31/02/2026", true), None);
        assert_eq!(parse_date("1/1/99999", true), None);
        assert_eq!(parse_date("tomorrow", true), None);
        assert_eq!(show_date(d(2026, 3, 5), true), "05/03/2026");
        assert_eq!(show_date(d(2026, 3, 5), false), "2026-03-05");
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        assert_eq!(parse_time("9:30"), t(9, 30));
        assert_eq!(parse_time("09.05"), t(9, 5));
        assert_eq!(parse_time("14"), t(14, 0));
        assert_eq!(parse_time("14h"), t(14, 0));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("9:75"), None);
    }

    fn form(all_day: bool, rule: &str) -> Form {
        let start = d(2026, 10, 13).and_hms_opt(9, 0, 0).unwrap();
        Form {
            summary: "Plan".into(),
            location: String::new(),
            description: "Line one\nLine two".into(),
            all_day,
            start,
            end: start + TimeDelta::minutes(90),
            rrule: rule.into(),
            alarm_min: Some(15),
        }
    }

    #[test]
    fn forms_become_resources() {
        // Lisbon in October: UTC+1.
        let to_utc = |w: NaiveDateTime| Some(w.and_utc().timestamp() - 3600);
        let now = DateTime::from_timestamp(1_790_000_000, 0).unwrap();
        let one = edit(&form(false, ""), "u1", now, &to_utc).unwrap();
        let raw = write_event(&one);
        assert!(raw.contains("DTSTART:20261013T080000Z\r\n"), "{raw}");
        assert!(raw.contains("DTEND:20261013T093000Z\r\n"));
        assert!(raw.contains("TRIGGER:-PT15M"));
        assert!(raw.contains("DESCRIPTION:Line one\\nLine two"));
        let res = parse_resource(raw.as_bytes()).unwrap();
        assert_eq!(res.master.unwrap().event.summary, "Plan");
        // A series: floating wall time, so it keeps 09:00 over DST.
        let rule = rrule(Repeat::Weekly, d(2026, 10, 13));
        let series = edit(&form(false, &rule), "u2", now, &to_utc).unwrap();
        let raw = write_event(&series);
        assert!(raw.contains("DTSTART:20261013T090000\r\n"), "{raw}");
        assert!(raw.contains("RRULE:FREQ=WEEKLY;BYDAY=TU\r\n"));
        // All day: dates, the end the day after the last.
        let mut f = form(true, "");
        f.end = d(2026, 10, 14).and_hms_opt(0, 0, 0).unwrap();
        let raw = write_event(&edit(&f, "u3", now, &to_utc).unwrap());
        assert!(raw.contains("DTSTART;VALUE=DATE:20261013\r\n"));
        assert!(raw.contains("DTEND;VALUE=DATE:20261015\r\n"));
        // An end before the start, and a time that doesn't exist here.
        let mut f = form(false, "");
        f.end = f.start - TimeDelta::minutes(1);
        assert_eq!(edit(&f, "u", now, &to_utc), Err(FormError::EndBeforeStart));
        let gap = |_: NaiveDateTime| None;
        assert_eq!(
            edit(&form(false, ""), "u", now, &gap),
            Err(FormError::BadTime)
        );
    }
}
