//! Recurrence expansion (RFC 5545 3.3.10 and 3.8.5): an RRULE with
//! EXDATE and RDATE, expanded in the event's wall-clock time. The caller
//! converts each occurrence to an instant in the event's zone.
//!
//! Supported: FREQ=DAILY, WEEKLY, MONTHLY and YEARLY; INTERVAL, COUNT and
//! UNTIL; BYDAY (with ordinals in MONTHLY and YEARLY), BYMONTHDAY (also
//! negative), BYMONTH and WKST; BYSETPOS in MONTHLY. Anything else is
//! refused with a reason ([`Refused`]); the caller then shows the first
//! occurrence only. DTSTART is always the first occurrence. Invalid dates
//! (February 30) are skipped. The input is untrusted: iterations, results
//! and years are capped.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use super::tz::{last_day, nth_weekday};
use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, TimeDelta, Timelike, Weekday};

/// Periods walked at most for one expansion.
pub const MAX_STEPS: usize = 100_000;
/// Occurrences returned at most for one series and range.
pub const MAX_INSTANCES: usize = 5_000;
/// Years outside this range end the walk.
pub const MIN_YEAR: i32 = 1900;
pub const MAX_YEAR: i32 = 2200;
const MAX_BY: usize = 64;

/// Why a rule is not expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// SECONDLY, MINUTELY or HOURLY.
    SubDaily,
    /// BYWEEKNO or BYYEARDAY.
    ByWeekOrYearDay,
    /// BYHOUR, BYMINUTE or BYSECOND other than DTSTART's.
    ByTime,
    /// RSCALE or SKIP (RFC 7529).
    Rscale,
    /// BYSETPOS outside MONTHLY.
    SetPos,
    /// A part this module doesn't read, or a bad value.
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// The UNTIL of a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// A date: the whole day counts (compared leniently).
    Date(NaiveDate),
    /// A floating or zoned wall time.
    Wall(NaiveDateTime),
    /// A UTC time; the caller's `to_wall` converts it.
    Utc(NaiveDateTime),
}

/// A parsed RRULE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<Until>,
    /// (ordinal, 0 for none; weekday).
    pub by_day: Vec<(i8, Weekday)>,
    pub by_month_day: Vec<i8>,
    pub by_month: Vec<u32>,
    pub by_set_pos: Vec<i16>,
    pub wkst: Weekday,
}

pub fn weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

fn list<T>(v: &str, f: impl Fn(&str) -> Option<T>) -> Result<Vec<T>, Refused> {
    let out: Option<Vec<T>> = v.split(',').take(MAX_BY).map(|s| f(s.trim())).collect();
    out.filter(|o| !o.is_empty()).ok_or(Refused::Invalid)
}

fn stamp(v: &str) -> Option<(NaiveDateTime, bool)> {
    let v = v.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let date = NaiveDate::from_ymd_opt(i32::try_from(num(0, 4)?).ok()?, num(4, 6)?, num(6, 8)?)?;
    if v.len() == 8 {
        return Some((date.and_hms_opt(0, 0, 0)?, false));
    }
    if v.get(8..9) != Some("T") {
        return None;
    }
    let t = date.and_hms_opt(num(9, 11)?, num(11, 13)?, num(13, 15)?)?;
    match v.get(15..) {
        Some("") => Some((t, false)),
        Some("Z" | "z") => Some((t, true)),
        _ => None,
    }
}

impl Rule {
    /// An RRULE value, checked against the event's start `dtstart`.
    pub fn parse(rule: &str, dtstart: NaiveDateTime) -> Result<Rule, Refused> {
        let mut r = Rule {
            freq: Freq::Daily,
            interval: 1,
            count: None,
            until: None,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month: Vec::new(),
            by_set_pos: Vec::new(),
            wkst: Weekday::Mon,
        };
        let mut freq = None;
        let same = |v: &str, now: u32| -> Result<(), Refused> {
            match v.trim().parse::<u32>() {
                Ok(n) if n == now && !v.contains(',') => Ok(()),
                _ => Err(Refused::ByTime),
            }
        };
        for part in rule.trim().split(';').filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=').ok_or(Refused::Invalid)?;
            let k = k.trim().to_ascii_uppercase();
            let v = v.trim().to_ascii_uppercase();
            match k.as_str() {
                "FREQ" => {
                    freq = Some(match v.as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        "SECONDLY" | "MINUTELY" | "HOURLY" => return Err(Refused::SubDaily),
                        _ => return Err(Refused::Invalid),
                    })
                }
                "INTERVAL" => {
                    r.interval = v
                        .parse()
                        .ok()
                        .filter(|n| (1..=10_000).contains(n))
                        .ok_or(Refused::Invalid)?
                }
                "COUNT" => r.count = Some(v.parse().map_err(|_| Refused::Invalid)?),
                "UNTIL" => {
                    let (t, utc) = stamp(&v).ok_or(Refused::Invalid)?;
                    r.until = Some(if v.len() == 8 {
                        Until::Date(t.date())
                    } else if utc {
                        Until::Utc(t)
                    } else {
                        Until::Wall(t)
                    });
                }
                "BYDAY" => {
                    r.by_day = list(&v, |d| {
                        let split = d.len().checked_sub(2)?;
                        let wd = weekday(d.get(split..)?)?;
                        let n = d.get(..split)?;
                        let n: i8 = if n.is_empty() { 0 } else { n.parse().ok()? };
                        (-53..=53).contains(&n).then_some((n, wd))
                    })?
                }
                "BYMONTHDAY" => {
                    r.by_month_day = list(&v, |d| {
                        d.parse::<i8>()
                            .ok()
                            .filter(|n| *n != 0 && (-31..=31).contains(n))
                    })?
                }
                "BYMONTH" => {
                    r.by_month = list(&v, |m| {
                        m.parse::<u32>().ok().filter(|m| (1..=12).contains(m))
                    })?
                }
                "BYSETPOS" => {
                    r.by_set_pos = list(&v, |p| {
                        p.parse::<i16>()
                            .ok()
                            .filter(|n| *n != 0 && (-366..=366).contains(n))
                    })?
                }
                "WKST" => r.wkst = weekday(&v).ok_or(Refused::Invalid)?,
                "BYHOUR" => same(&v, dtstart.hour())?,
                "BYMINUTE" => same(&v, dtstart.minute())?,
                "BYSECOND" => same(&v, dtstart.second())?,
                "BYWEEKNO" | "BYYEARDAY" => return Err(Refused::ByWeekOrYearDay),
                "RSCALE" | "SKIP" => return Err(Refused::Rscale),
                _ if k.starts_with("X-") => {}
                _ => return Err(Refused::Invalid),
            }
        }
        r.freq = freq.ok_or(Refused::Invalid)?;
        if !r.by_set_pos.is_empty() && r.freq != Freq::Monthly {
            return Err(Refused::SetPos);
        }
        // Ordinals only mean something in MONTHLY and YEARLY.
        if matches!(r.freq, Freq::Daily | Freq::Weekly) && r.by_day.iter().any(|(n, _)| *n != 0) {
            return Err(Refused::Invalid);
        }
        Ok(r)
    }

    fn month_ok(&self, d: NaiveDate) -> bool {
        self.by_month.is_empty() || self.by_month.contains(&d.month())
    }

    fn month_day_ok(&self, d: NaiveDate) -> bool {
        self.by_month_day.is_empty()
            || self
                .by_month_day
                .iter()
                .any(|&n| month_day(d.year(), d.month(), n) == Some(d))
    }

    fn day_ok(&self, d: NaiveDate) -> bool {
        self.by_day.is_empty() || self.by_day.iter().any(|(_, wd)| *wd == d.weekday())
    }

    /// The candidate dates of one period, sorted.
    fn period(&self, start: NaiveDate, dtstart: NaiveDate) -> Option<Vec<NaiveDate>> {
        let mut out = Vec::new();
        match self.freq {
            Freq::Daily => {
                if self.month_ok(start) && self.month_day_ok(start) && self.day_ok(start) {
                    out.push(start);
                }
            }
            Freq::Weekly => {
                for i in 0..7u64 {
                    let d = start.checked_add_days(chrono::Days::new(i))?;
                    let wanted = if self.by_day.is_empty() {
                        d.weekday() == dtstart.weekday()
                    } else {
                        self.day_ok(d)
                    };
                    if wanted && self.month_ok(d) {
                        out.push(d);
                    }
                }
            }
            Freq::Monthly => {
                if self.month_ok(start) {
                    out = self.in_month(start.year(), start.month(), dtstart)?;
                    out.sort_unstable();
                    out.dedup();
                }
                if !self.by_set_pos.is_empty() {
                    out = set_pos(&out, &self.by_set_pos);
                }
            }
            Freq::Yearly => {
                let months: Vec<u32> = if self.by_month.is_empty() {
                    if self.by_day.iter().any(|(n, _)| *n != 0) && self.by_month_day.is_empty() {
                        // "20MO" in a year with no BYMONTH: the n-th of the year.
                        return self.nth_in_year(start.year());
                    }
                    // BYDAY alone: every month; otherwise DTSTART's month.
                    if self.by_month_day.is_empty() && !self.by_day.is_empty() {
                        (1..=12).collect()
                    } else {
                        vec![dtstart.month()]
                    }
                } else {
                    self.by_month.clone()
                };
                for m in months {
                    out.extend(self.in_month(start.year(), m, dtstart)?);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        Some(out)
    }

    /// The dates of `month` the BYMONTHDAY and BYDAY parts pick.
    fn in_month(&self, year: i32, month: u32, dtstart: NaiveDate) -> Option<Vec<NaiveDate>> {
        let mut out = Vec::new();
        if !self.by_month_day.is_empty() {
            for &n in &self.by_month_day {
                if let Some(d) = month_day(year, month, n).filter(|d| self.day_ok(*d)) {
                    out.push(d);
                }
            }
        } else if !self.by_day.is_empty() {
            for &(n, wd) in &self.by_day {
                if n != 0 {
                    out.extend(nth_weekday(year, month, wd, n));
                } else {
                    let mut d = NaiveDate::from_weekday_of_month_opt(year, month, wd, 1)?;
                    while d.month() == month {
                        out.push(d);
                        d = d.checked_add_days(chrono::Days::new(7))?;
                    }
                }
            }
        } else {
            // DTSTART's day of the month (none in a shorter month).
            out.extend(NaiveDate::from_ymd_opt(year, month, dtstart.day()));
        }
        Some(out)
    }

    /// YEARLY with ordinal BYDAY and no BYMONTH: the n-th weekday of the year.
    fn nth_in_year(&self, year: i32) -> Option<Vec<NaiveDate>> {
        let first = NaiveDate::from_ymd_opt(year, 1, 1)?;
        let last = NaiveDate::from_ymd_opt(year, 12, 31)?;
        let mut out = Vec::new();
        for &(n, wd) in &self.by_day {
            let all: Vec<NaiveDate> = first
                .iter_days()
                .take_while(|d| *d <= last)
                .filter(|d| d.weekday() == wd)
                .collect();
            let pick = match n {
                0 => {
                    out.extend(all);
                    continue;
                }
                n if n > 0 => all.get(usize::try_from(n).ok()?.checked_sub(1)?),
                n => all
                    .len()
                    .checked_sub(usize::try_from(n.checked_neg()?).ok()?)
                    .and_then(|i| all.get(i)),
            };
            out.extend(pick.copied());
        }
        out.sort_unstable();
        Some(out)
    }
}

/// Day `n` of a month (negative from its end).
fn month_day(year: i32, month: u32, n: i8) -> Option<NaiveDate> {
    if n > 0 {
        return NaiveDate::from_ymd_opt(year, month, u32::try_from(n).ok()?);
    }
    let last = last_day(year, month)?;
    let back = u64::try_from(i64::from(n).checked_add(1)?.checked_abs()?).ok()?;
    last.checked_sub_days(chrono::Days::new(back))
        .filter(|d| d.month() == month)
}

fn set_pos(dates: &[NaiveDate], pos: &[i16]) -> Vec<NaiveDate> {
    let mut out: Vec<NaiveDate> = pos
        .iter()
        .filter_map(|&p| {
            let i = if p > 0 {
                usize::try_from(p).ok()?.checked_sub(1)?
            } else {
                dates
                    .len()
                    .checked_sub(usize::try_from(p.checked_neg()?).ok()?)?
            };
            dates.get(i).copied()
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// What an expansion gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// Occurrence starts (wall time) in the range, sorted.
    pub starts: Vec<NaiveDateTime>,
    /// A cap stopped the walk before the rule ended.
    pub capped: bool,
    /// The last occurrence of the whole series, when it ends (COUNT or
    /// UNTIL) and the walk reached it.
    pub series_end: Option<NaiveDateTime>,
}

/// The occurrences of `rule` starting at `dtstart` that start in
/// `[from, to)` (wall times), less `exdates`, plus `rdates`. `to_wall`
/// turns a UTC UNTIL into the event's wall time.
pub fn expand(
    rule: &Rule,
    dtstart: NaiveDateTime,
    from: NaiveDateTime,
    to: NaiveDateTime,
    exdates: &[NaiveDateTime],
    rdates: &[NaiveDateTime],
    to_wall: &dyn Fn(NaiveDateTime) -> NaiveDateTime,
) -> Expansion {
    let until = rule.until.map(|u| match u {
        Until::Date(d) => d
            .and_hms_opt(23, 59, 59)
            .unwrap_or(d.and_time(dtstart.time())),
        Until::Wall(t) => t,
        Until::Utc(t) => to_wall(t),
    });
    let time = dtstart.time();
    let first = dtstart.date();
    let mut period = match rule.freq {
        Freq::Daily => Some(first),
        Freq::Weekly => {
            let back = (7u32
                .saturating_add(first.weekday().num_days_from_monday())
                .saturating_sub(rule.wkst.num_days_from_monday()))
                % 7;
            first.checked_sub_days(chrono::Days::new(u64::from(back)))
        }
        Freq::Monthly => first.with_day(1),
        Freq::Yearly => first.with_day(1).and_then(|d| d.with_month(1)),
    };
    let mut out = Vec::new();
    let mut counted: u32 = 0;
    let mut capped = false;
    let mut ended = false;
    let mut last = None;
    let excluded = |t: &NaiveDateTime| exdates.contains(t);
    // DTSTART always counts first.
    let mut push = |t: NaiveDateTime, out: &mut Vec<NaiveDateTime>, counted: &mut u32| -> bool {
        if let Some(c) = rule.count
            && *counted >= c
        {
            return false;
        }
        if until.is_some_and(|u| t > u) {
            return false;
        }
        *counted = counted.saturating_add(1);
        last = Some(t);
        if t >= from && t < to && !excluded(&t) {
            out.push(t);
        }
        true
    };
    push(dtstart, &mut out, &mut counted);
    let mut steps = 0usize;
    'walk: while let Some(p) = period {
        steps = steps.saturating_add(1);
        if steps > MAX_STEPS || out.len() >= MAX_INSTANCES {
            capped = true;
            break;
        }
        if !(MIN_YEAR..=MAX_YEAR).contains(&p.year()) {
            capped = true;
            break;
        }
        // Past the range and nothing to count any more.
        if p.and_time(time) >= to && rule.count.is_none() {
            break;
        }
        for d in rule.period(p, first).unwrap_or_default() {
            let t = d.and_time(time);
            if t <= dtstart {
                continue;
            }
            if !push(t, &mut out, &mut counted) {
                ended = true;
                break 'walk;
            }
            if t >= to && rule.count.is_some() && !ended {
                // COUNT must walk from DTSTART, but not past the range.
                break 'walk;
            }
        }
        let n = rule.interval;
        period = match rule.freq {
            Freq::Daily => p.checked_add_days(chrono::Days::new(u64::from(n))),
            Freq::Weekly => p.checked_add_days(chrono::Days::new(u64::from(n).saturating_mul(7))),
            Freq::Monthly => p.checked_add_months(Months::new(n)),
            Freq::Yearly => p.checked_add_months(Months::new(n.saturating_mul(12))),
        };
        if period.is_none() {
            break;
        }
    }
    let series_end = last.filter(|_| ended);
    for &r in rdates {
        if r >= from && r < to && !excluded(&r) && !out.contains(&r) && out.len() < MAX_INSTANCES {
            out.push(r);
        }
    }
    out.sort_unstable();
    Expansion {
        starts: out,
        capped,
        series_end,
    }
}

/// The span between a start and an end, for an occurrence's end.
pub fn span(start: NaiveDateTime, end: Option<NaiveDateTime>) -> TimeDelta {
    end.map(|e| e.signed_duration_since(start))
        .filter(|d| *d >= TimeDelta::zero())
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn run(rule: &str, start: &str, from: &str, to: &str) -> Vec<String> {
        run_with(rule, start, from, to, &[], &[])
    }

    fn run_with(
        rule: &str,
        start: &str,
        from: &str,
        to: &str,
        ex: &[&str],
        r: &[&str],
    ) -> Vec<String> {
        let dt = t(start);
        let rule = Rule::parse(rule, dt).unwrap();
        let ex: Vec<_> = ex.iter().map(|s| t(s)).collect();
        let r: Vec<_> = r.iter().map(|s| t(s)).collect();
        expand(&rule, dt, t(from), t(to), &ex, &r, &|x| x)
            .starts
            .iter()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .collect()
    }

    #[test]
    fn every_second_tuesday() {
        assert_eq!(
            run(
                "FREQ=MONTHLY;BYDAY=2TU;COUNT=4",
                "2026-01-13 10:00",
                "2026-01-01 00:00",
                "2027-01-01 00:00"
            ),
            ["2026-01-13", "2026-02-10", "2026-03-10", "2026-04-14"]
        );
    }

    #[test]
    fn last_weekday_of_the_month() {
        assert_eq!(
            run(
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
                "2026-01-30 09:00",
                "2026-01-01 00:00",
                "2026-06-01 00:00"
            ),
            [
                "2026-01-30",
                "2026-02-27",
                "2026-03-31",
                "2026-04-30",
                "2026-05-29"
            ]
        );
    }

    #[test]
    fn last_day_of_the_month() {
        assert_eq!(
            run(
                "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=3",
                "2026-01-31 09:00",
                "2026-01-01 00:00",
                "2027-01-01 00:00"
            ),
            ["2026-01-31", "2026-02-28", "2026-03-31"]
        );
    }

    #[test]
    fn feb_29_yearly_skips_common_years() {
        assert_eq!(
            run(
                "FREQ=YEARLY",
                "2024-02-29 09:00",
                "2024-01-01 00:00",
                "2033-01-01 00:00"
            ),
            ["2024-02-29", "2028-02-29", "2032-02-29"]
        );
        // Monthly on the 31st: only the long months.
        assert_eq!(
            run(
                "FREQ=MONTHLY;COUNT=4",
                "2026-01-31 09:00",
                "2026-01-01 00:00",
                "2027-01-01 00:00"
            ),
            ["2026-01-31", "2026-03-31", "2026-05-31", "2026-07-31"]
        );
    }

    #[test]
    fn count_with_exdate_and_rdate() {
        // EXDATE removes one of the COUNT occurrences (it still counts).
        assert_eq!(
            run_with(
                "FREQ=DAILY;COUNT=5",
                "2026-03-01 09:00",
                "2026-01-01 00:00",
                "2027-01-01 00:00",
                &["2026-03-03 09:00"],
                &["2026-04-01 09:00"]
            ),
            [
                "2026-03-01",
                "2026-03-02",
                "2026-03-04",
                "2026-03-05",
                "2026-04-01"
            ]
        );
    }

    #[test]
    fn until_date_with_a_date_time_start() {
        assert_eq!(
            run(
                "FREQ=WEEKLY;UNTIL=20260317",
                "2026-03-03 18:00",
                "2026-01-01 00:00",
                "2027-01-01 00:00"
            ),
            ["2026-03-03", "2026-03-10", "2026-03-17"]
        );
        // A UTC UNTIL goes through `to_wall`.
        let dt = t("2026-03-03 18:00");
        let rule = Rule::parse("FREQ=WEEKLY;UNTIL=20260317T170000Z", dt).unwrap();
        let plus_one = |x: NaiveDateTime| x + TimeDelta::hours(1);
        let e = expand(
            &rule,
            dt,
            t("2026-01-01 00:00"),
            t("2027-01-01 00:00"),
            &[],
            &[],
            &plus_one,
        );
        assert_eq!(e.starts.len(), 3);
        assert_eq!(e.series_end, Some(t("2026-03-17 18:00")));
    }

    #[test]
    fn rfc_examples() {
        // Every other week on Tuesday and Thursday, for 8 occurrences.
        assert_eq!(
            run(
                "FREQ=WEEKLY;INTERVAL=2;COUNT=8;WKST=SU;BYDAY=TU,TH",
                "1997-09-02 09:00",
                "1997-01-01 00:00",
                "1998-01-01 00:00"
            ),
            [
                "1997-09-02",
                "1997-09-04",
                "1997-09-16",
                "1997-09-18",
                "1997-09-30",
                "1997-10-02",
                "1997-10-14",
                "1997-10-16"
            ]
        );
        // Monthly on the first Friday for 4 occurrences.
        assert_eq!(
            run(
                "FREQ=MONTHLY;COUNT=4;BYDAY=1FR",
                "1997-09-05 09:00",
                "1997-01-01 00:00",
                "1999-01-01 00:00"
            ),
            ["1997-09-05", "1997-10-03", "1997-11-07", "1997-12-05"]
        );
        // Yearly in June and July for 4.
        assert_eq!(
            run(
                "FREQ=YEARLY;COUNT=4;BYMONTH=6,7",
                "1997-06-10 09:00",
                "1997-01-01 00:00",
                "2000-01-01 00:00"
            ),
            ["1997-06-10", "1997-07-10", "1998-06-10", "1998-07-10"]
        );
        // Every Thanksgiving (fourth Thursday of November).
        assert_eq!(
            run(
                "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH;COUNT=3",
                "2026-11-26 12:00",
                "2026-01-01 00:00",
                "2030-01-01 00:00"
            ),
            ["2026-11-26", "2027-11-25", "2028-11-23"]
        );
        // Friday the 13th.
        assert_eq!(
            run(
                "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13;COUNT=3",
                "2026-02-13 09:00",
                "2026-01-01 00:00",
                "2030-01-01 00:00"
            ),
            ["2026-02-13", "2026-03-13", "2026-11-13"]
        );
        // The 20th Monday of the year.
        assert_eq!(
            run(
                "FREQ=YEARLY;BYDAY=20MO;COUNT=2",
                "1997-05-19 09:00",
                "1997-01-01 00:00",
                "2000-01-01 00:00"
            ),
            ["1997-05-19", "1998-05-18"]
        );
    }

    #[test]
    fn range_is_a_window() {
        assert_eq!(
            run(
                "FREQ=DAILY",
                "2020-01-01 09:00",
                "2026-03-01 00:00",
                "2026-03-04 00:00"
            ),
            ["2026-03-01", "2026-03-02", "2026-03-03"]
        );
    }

    #[test]
    fn refused_rules() {
        let dt = t("2026-03-01 09:00");
        let r = |s: &str| Rule::parse(s, dt).unwrap_err();
        assert_eq!(r("FREQ=HOURLY"), Refused::SubDaily);
        assert_eq!(r("FREQ=MINUTELY"), Refused::SubDaily);
        assert_eq!(r("FREQ=SECONDLY"), Refused::SubDaily);
        assert_eq!(r("FREQ=YEARLY;BYWEEKNO=20"), Refused::ByWeekOrYearDay);
        assert_eq!(r("FREQ=YEARLY;BYYEARDAY=100"), Refused::ByWeekOrYearDay);
        assert_eq!(r("FREQ=DAILY;BYHOUR=9,17"), Refused::ByTime);
        assert_eq!(r("FREQ=DAILY;BYHOUR=10"), Refused::ByTime);
        assert_eq!(r("FREQ=YEARLY;RSCALE=CHINESE"), Refused::Rscale);
        assert_eq!(r("FREQ=WEEKLY;BYSETPOS=1;BYDAY=MO"), Refused::SetPos);
        assert_eq!(r("FREQ=FORTNIGHTLY"), Refused::Invalid);
        assert_eq!(r("INTERVAL=2"), Refused::Invalid);
        assert_eq!(r("FREQ=DAILY;BYDAY=1MO"), Refused::Invalid);
        assert_eq!(r("FREQ=DAILY;EXRULE=x"), Refused::Invalid);
        assert!(Rule::parse("FREQ=DAILY;BYHOUR=9;BYMINUTE=0", dt).is_ok());
    }

    #[test]
    fn caps_stop_hostile_rules() {
        // A huge COUNT from long ago walks at most MAX_STEPS periods.
        let dt = t("1901-01-01 09:00");
        let rule = Rule::parse("FREQ=DAILY;COUNT=10000000", dt).unwrap();
        let e = expand(
            &rule,
            dt,
            t("2190-01-01 00:00"),
            t("2190-02-01 00:00"),
            &[],
            &[],
            &|x| x,
        );
        assert!(e.capped);
        // A daily rule from 1901 with no end: only the range is kept.
        let rule = Rule::parse("FREQ=DAILY", dt).unwrap();
        let e = expand(
            &rule,
            dt,
            t("1950-01-01 00:00"),
            t("2199-01-01 00:00"),
            &[],
            &[],
            &|x| x,
        );
        assert!(e.capped);
        assert!(e.starts.len() <= MAX_INSTANCES);
        // A rule matching nothing (BYMONTHDAY=31 in February) ends at the caps.
        let rule =
            Rule::parse("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=31", t("2026-01-31 09:00")).unwrap();
        let e = expand(
            &rule,
            t("2026-01-31 09:00"),
            t("2026-01-01 00:00"),
            t("2300-01-01 00:00"),
            &[],
            &[],
            &|x| x,
        );
        assert!(e.capped);
        // Garbage never panics.
        for s in [
            "",
            ";;;",
            "FREQ=",
            "FREQ=DAILY;INTERVAL=0",
            "FREQ=DAILY;COUNT=-1",
            "FREQ=MONTHLY;BYMONTHDAY=0",
            "FREQ=MONTHLY;BYDAY=99MO",
            "FREQ=DAILY;UNTIL=2026",
            "FREQ=MONTHLY;BYSETPOS=9999;BYDAY=MO",
        ] {
            assert!(Rule::parse(s, dt).is_err(), "{s}");
        }
    }

    #[test]
    fn interval_and_count() {
        let dt = t("2026-03-01 09:00");
        let rule = Rule::parse("FREQ=DAILY;INTERVAL=10;COUNT=3", dt).unwrap();
        let e = expand(
            &rule,
            dt,
            t("2026-01-01 00:00"),
            t("2027-01-01 00:00"),
            &[],
            &[],
            &|x| x,
        );
        assert_eq!(
            e.starts,
            [
                t("2026-03-01 09:00"),
                t("2026-03-11 09:00"),
                t("2026-03-21 09:00")
            ]
        );
        assert_eq!(e.series_end, Some(t("2026-03-21 09:00")));
    }
}
