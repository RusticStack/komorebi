//! Time zones for calendar events: a TZID normalized (Mozilla and
//! softwarestudio prefixes, UTC and `Etc/GMT` forms, Outlook's Windows ids
//! and IANA names through [`super::tz_table`]), and the offset of a wall
//! time in a zone. The rules are Windows' own (`TIME_ZONE_INFORMATION`,
//! read with `GetTimeZoneInformationForYear`); the maths on them is pure
//! and tested here with fixture rules.
//!
//! DST policy (RFC 5545 3.3.5): a wall time in the spring-forward gap
//! uses the offset before the transition; one in the fall-back overlap
//! takes the first occurrence (the daylight offset).

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Weekday};

/// A yearly transition, as `SYSTEMTIME` writes it in a relative rule:
/// the `week`-th (5: the last) `weekday` (0 Sunday) of `month`, at
/// `hour:minute` local time (before the change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub month: u32,
    pub week: u32,
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

/// A zone's rule for one year, shaped like `TIME_ZONE_INFORMATION`:
/// UTC = local + `bias` (+ `std_bias` or `dst_bias`) minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub bias: i32,
    pub std_bias: i32,
    pub dst_bias: i32,
    /// When standard time starts; `None` for a zone without DST.
    pub std_date: Option<Transition>,
    /// When daylight time starts.
    pub dst_date: Option<Transition>,
}

/// A normalized TZID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Zone {
    /// UTC or a fixed offset (seconds east of UTC).
    Fixed(i64),
    /// A Windows time zone id ("GMT Standard Time").
    Windows(&'static str),
    /// Not known: shown as its wall time with the zone's name.
    Unknown,
}

/// The `n`-th `wd` of a month (from its end when `n` is negative).
pub fn nth_weekday(year: i32, month: u32, wd: Weekday, n: i8) -> Option<NaiveDate> {
    if n >= 0 {
        return NaiveDate::from_weekday_of_month_opt(year, month, wd, u8::try_from(n).ok()?);
    }
    let last = last_day(year, month)?;
    let mut day = last;
    while day.weekday() != wd {
        day = day.pred_opt()?;
    }
    let back = TimeDelta::try_weeks(i64::from(n).checked_add(1)?.checked_abs()?)?;
    day.checked_sub_signed(back).filter(|d| d.month() == month)
}

/// The last day of a month.
pub fn last_day(year: i32, month: u32) -> Option<NaiveDate> {
    let first_next = match month {
        12 => NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)?,
        m => NaiveDate::from_ymd_opt(year, m.checked_add(1)?, 1)?,
    };
    first_next.pred_opt()
}

fn weekday_from_sunday(n: u32) -> Option<Weekday> {
    Some(match n {
        0 => Weekday::Sun,
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        6 => Weekday::Sat,
        _ => return None,
    })
}

impl Transition {
    /// The local time it happens in `year`.
    fn at(&self, year: i32) -> Option<NaiveDateTime> {
        let wd = weekday_from_sunday(self.weekday)?;
        let day = if self.week >= 5 {
            nth_weekday(year, self.month, wd, -1)?
        } else {
            nth_weekday(year, self.month, wd, i8::try_from(self.week).ok()?)?
        };
        Some(day.and_time(NaiveTime::from_hms_opt(self.hour, self.minute, 0)?))
    }
}

impl Rule {
    fn std_offset(&self) -> Option<i64> {
        i64::from(self.bias)
            .checked_add(i64::from(self.std_bias))?
            .checked_mul(-60)
    }

    fn dst_offset(&self) -> Option<i64> {
        i64::from(self.bias)
            .checked_add(i64::from(self.dst_bias))?
            .checked_mul(-60)
    }

    /// The offset (seconds east of UTC) of wall time `wall`, with the
    /// gap and overlap policy in the module notes.
    pub fn offset(&self, wall: NaiveDateTime) -> Option<i64> {
        let std = self.std_offset()?;
        let (Some(sd), Some(dd)) = (self.std_date, self.dst_date) else {
            return Some(std);
        };
        if sd.month == 0 || dd.month == 0 {
            return Some(std);
        }
        let dst = self.dst_offset()?;
        let year = wall.year();
        let (dst_start, std_start) = (dd.at(year)?, sd.at(year)?);
        // Local times from the end of the gap (wall clocks jump forward
        // by `delta`); the overlap before `std_start` stays daylight.
        let delta = TimeDelta::try_seconds(dst.checked_sub(std)?.max(0))?;
        let dst_from = dst_start.checked_add_signed(delta)?;
        let in_dst = if dst_start <= std_start {
            dst_from <= wall && wall < std_start
        } else {
            wall >= dst_from || wall < std_start
        };
        Some(if in_dst { dst } else { std })
    }
}

/// A TZID normalized (see the module notes).
pub fn normalize(tzid: &str) -> Zone {
    let mut id = tzid.trim().trim_matches('"');
    // "/mozilla.org/20050126_1/Europe/Lisbon", "/softwarestudio.org/Olson_20011030_5/America/New_York"
    if let Some(rest) = id.strip_prefix('/') {
        let mut parts = rest.splitn(3, '/');
        let _vendor = parts.next();
        let _version = parts.next();
        id = parts.next().unwrap_or(rest);
    }
    match id.to_ascii_uppercase().as_str() {
        "UTC" | "GMT" | "Z" | "ETC/UTC" | "ETC/GMT" | "ETC/UCT" | "UCT" | "ETC/ZULU" | "ZULU"
        | "ETC/UNIVERSAL" | "UNIVERSAL" | "ETC/GREENWICH" | "GREENWICH" | "ETC/GMT0"
        | "ETC/GMT+0" | "ETC/GMT-0" | "GMT0" | "UTC0" => return Zone::Fixed(0),
        _ => {}
    }
    // Etc/GMT+5 is five hours *behind* UTC (POSIX sign).
    if let Some(n) = id
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("Etc/GMT"))
        .and_then(|_| id.get(7..))
    {
        let hours = n.parse::<i64>().ok().filter(|h| (-14..=12).contains(h));
        return hours
            .and_then(|h| h.checked_mul(-3600))
            .map_or(Zone::Unknown, Zone::Fixed);
    }
    if let Some(w) = super::tz_table::windows_id(id) {
        return Zone::Windows(w);
    }
    super::tz_table::iana(id).map_or(Zone::Unknown, Zone::Windows)
}

/// The UTC instant (unix seconds) of wall time `wall` in zone `tzid`;
/// `None` for an unknown zone (or a Windows id this system lacks).
pub fn to_utc(tzid: &str, wall: NaiveDateTime) -> Option<i64> {
    let off = match normalize(tzid) {
        Zone::Fixed(off) => off,
        Zone::Windows(id) => system_rule(id, wall.year())?.offset(wall)?,
        Zone::Unknown => return None,
    };
    wall.and_utc().timestamp().checked_sub(off)
}

#[cfg(windows)]
mod sys {
    use super::{Rule, Transition};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, EnumDynamicTimeZoneInformation,
        GetTimeZoneInformationForYear, TIME_ZONE_INFORMATION,
    };

    /// Rules read, at most this many (zone, year) pairs.
    const MAX_CACHED: usize = 64;
    static CACHE: Mutex<Vec<(&'static str, i32, Option<Rule>)>> = Mutex::new(Vec::new());

    fn transition(t: &SYSTEMTIME) -> Option<Transition> {
        // wYear 0: a relative (yearly) rule; an absolute one is used as is
        // for its own year only, which the Windows database doesn't use
        // for the "ForYear" answer.
        (t.wMonth != 0).then_some(Transition {
            month: u32::from(t.wMonth),
            week: u32::from(t.wDay),
            weekday: u32::from(t.wDayOfWeek),
            hour: u32::from(t.wHour),
            minute: u32::from(t.wMinute),
        })
    }

    fn read(id: &str, year: i32) -> Option<Rule> {
        let wanted: Vec<u16> = id.encode_utf16().collect();
        let year = u16::try_from(year).ok()?;
        for index in 0..1_000u32 {
            let mut d = DYNAMIC_TIME_ZONE_INFORMATION::default();
            // SAFETY: `d` is a valid out pointer for the call.
            if unsafe { EnumDynamicTimeZoneInformation(index, &mut d) } != 0 {
                return None;
            }
            let len = d.TimeZoneKeyName.iter().position(|&c| c == 0)?;
            if d.TimeZoneKeyName.get(..len) != Some(wanted.as_slice()) {
                continue;
            }
            let mut tz = TIME_ZONE_INFORMATION::default();
            // SAFETY: both pointers are valid for the call.
            if unsafe { GetTimeZoneInformationForYear(year, &d, &mut tz) } == 0 {
                return None;
            }
            return Some(Rule {
                bias: tz.Bias,
                std_bias: tz.StandardBias,
                dst_bias: tz.DaylightBias,
                std_date: transition(&tz.StandardDate),
                dst_date: transition(&tz.DaylightDate),
            });
        }
        None
    }

    pub fn rule(id: &'static str, year: i32) -> Option<Rule> {
        let Ok(mut cache) = CACHE.lock() else {
            return read(id, year);
        };
        if let Some((_, _, r)) = cache.iter().find(|(i, y, _)| *i == id && *y == year) {
            return *r;
        }
        let r = read(id, year);
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.push((id, year, r));
        r
    }
}

/// Windows' rule for zone `id` in `year` (cached in memory).
#[cfg(windows)]
pub fn system_rule(id: &'static str, year: i32) -> Option<Rule> {
    sys::rule(id, year)
}

#[cfg(not(windows))]
pub fn system_rule(_id: &'static str, _year: i32) -> Option<Rule> {
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    fn wall(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    /// America/New_York since 2007: second Sunday of March 02:00, first
    /// Sunday of November 02:00.
    fn new_york() -> Rule {
        Rule {
            bias: 300,
            std_bias: 0,
            dst_bias: -60,
            std_date: Some(Transition {
                month: 11,
                week: 1,
                weekday: 0,
                hour: 2,
                minute: 0,
            }),
            dst_date: Some(Transition {
                month: 3,
                week: 2,
                weekday: 0,
                hour: 2,
                minute: 0,
            }),
        }
    }

    /// Europe/Lisbon (GMT Standard Time): last Sunday of March 01:00, last
    /// Sunday of October 02:00.
    fn lisbon() -> Rule {
        Rule {
            bias: 0,
            std_bias: 0,
            dst_bias: -60,
            std_date: Some(Transition {
                month: 10,
                week: 5,
                weekday: 0,
                hour: 2,
                minute: 0,
            }),
            dst_date: Some(Transition {
                month: 3,
                week: 5,
                weekday: 0,
                hour: 1,
                minute: 0,
            }),
        }
    }

    /// Australia/Sydney: DST from the first Sunday of October to the first
    /// Sunday of April.
    fn sydney() -> Rule {
        Rule {
            bias: -600,
            std_bias: 0,
            dst_bias: -60,
            std_date: Some(Transition {
                month: 4,
                week: 1,
                weekday: 0,
                hour: 3,
                minute: 0,
            }),
            dst_date: Some(Transition {
                month: 10,
                week: 1,
                weekday: 0,
                hour: 2,
                minute: 0,
            }),
        }
    }

    #[test]
    fn new_york_across_both_transitions() {
        let ny = new_york();
        assert_eq!(ny.offset(wall("2026-03-08 01:59")), Some(-5 * 3600));
        // The gap (02:00 to 03:00 doesn't exist): the offset before it.
        assert_eq!(ny.offset(wall("2026-03-08 02:30")), Some(-5 * 3600));
        assert_eq!(ny.offset(wall("2026-03-08 03:00")), Some(-4 * 3600));
        assert_eq!(ny.offset(wall("2026-07-01 12:00")), Some(-4 * 3600));
        // The overlap (01:00 to 02:00 happens twice): the first one, EDT.
        assert_eq!(ny.offset(wall("2026-11-01 01:30")), Some(-4 * 3600));
        assert_eq!(ny.offset(wall("2026-11-01 02:00")), Some(-5 * 3600));
        assert_eq!(ny.offset(wall("2026-12-25 09:00")), Some(-5 * 3600));
    }

    #[test]
    fn lisbon_across_both_transitions() {
        let l = lisbon();
        assert_eq!(l.offset(wall("2026-03-29 00:59")), Some(0));
        assert_eq!(l.offset(wall("2026-03-29 01:30")), Some(0));
        assert_eq!(l.offset(wall("2026-03-29 02:00")), Some(3600));
        assert_eq!(l.offset(wall("2026-10-25 01:30")), Some(3600));
        assert_eq!(l.offset(wall("2026-10-25 02:00")), Some(0));
    }

    #[test]
    fn southern_hemisphere() {
        let s = sydney();
        assert_eq!(s.offset(wall("2026-01-15 12:00")), Some(11 * 3600));
        assert_eq!(s.offset(wall("2026-06-15 12:00")), Some(10 * 3600));
        assert_eq!(s.offset(wall("2026-10-04 02:30")), Some(10 * 3600));
        assert_eq!(s.offset(wall("2026-10-04 03:00")), Some(11 * 3600));
        assert_eq!(s.offset(wall("2026-04-05 02:30")), Some(11 * 3600));
        assert_eq!(s.offset(wall("2026-04-05 03:00")), Some(10 * 3600));
    }

    #[test]
    fn no_dst_and_bad_rules() {
        let tokyo = Rule {
            bias: -540,
            std_bias: 0,
            dst_bias: 0,
            std_date: None,
            dst_date: None,
        };
        assert_eq!(tokyo.offset(wall("2026-07-01 12:00")), Some(9 * 3600));
        let bad = Rule {
            std_date: Some(Transition {
                month: 13,
                week: 9,
                weekday: 9,
                hour: 99,
                minute: 0,
            }),
            ..new_york()
        };
        assert_eq!(bad.offset(wall("2026-07-01 12:00")), None);
        let huge = Rule {
            bias: i32::MAX,
            std_bias: i32::MAX,
            ..tokyo
        };
        let _ = huge.offset(wall("2026-07-01 12:00"));
    }

    #[test]
    fn tzid_normalization() {
        assert_eq!(normalize("UTC"), Zone::Fixed(0));
        assert_eq!(normalize("Etc/GMT+5"), Zone::Fixed(-5 * 3600));
        assert_eq!(normalize("Etc/GMT-14"), Zone::Fixed(14 * 3600));
        assert_eq!(normalize("Etc/GMT+99"), Zone::Unknown);
        assert_eq!(
            normalize("Europe/Lisbon"),
            Zone::Windows("GMT Standard Time")
        );
        assert_eq!(
            normalize("/mozilla.org/20050126_1/America/New_York"),
            Zone::Windows("Eastern Standard Time")
        );
        assert_eq!(
            normalize("/softwarestudio.org/Olson_20011030_5/Europe/Paris"),
            Zone::Windows("Romance Standard Time")
        );
        // Outlook writes Windows ids as the TZID.
        assert_eq!(
            normalize("\"W. Europe Standard Time\""),
            Zone::Windows("W. Europe Standard Time")
        );
        assert_eq!(normalize("Mars/Olympus_Mons"), Zone::Unknown);
        assert_eq!(normalize(""), Zone::Unknown);
        assert_eq!(normalize("/"), Zone::Unknown);
        assert_eq!(to_utc("Mars/Olympus_Mons", wall("2026-07-01 12:00")), None);
        assert_eq!(
            to_utc("Etc/UTC", wall("2026-07-01 12:00")),
            Some(wall("2026-07-01 12:00").and_utc().timestamp())
        );
    }

    /// Windows' own rules (needs the system's zone database).
    #[cfg(windows)]
    #[test]
    fn windows_rules() {
        let utc = |s: &str| wall(s).and_utc().timestamp();
        assert_eq!(
            to_utc("America/New_York", wall("2026-07-01 12:00")),
            Some(utc("2026-07-01 16:00"))
        );
        assert_eq!(
            to_utc("America/New_York", wall("2026-12-01 12:00")),
            Some(utc("2026-12-01 17:00"))
        );
        assert_eq!(
            to_utc("America/New_York", wall("2026-03-08 02:30")),
            Some(utc("2026-03-08 07:30"))
        );
        assert_eq!(
            to_utc("Europe/Lisbon", wall("2026-07-01 12:00")),
            Some(utc("2026-07-01 11:00"))
        );
        assert_eq!(
            to_utc("Europe/Lisbon", wall("2026-01-01 12:00")),
            Some(utc("2026-01-01 12:00"))
        );
        assert_eq!(
            to_utc("Asia/Tokyo", wall("2026-01-01 12:00")),
            Some(utc("2026-01-01 03:00"))
        );
    }
}
