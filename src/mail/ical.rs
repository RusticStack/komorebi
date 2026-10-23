//! Meeting invitations (iCalendar, RFC 5545, with iTIP methods, RFC 5546):
//! the first event of a `text/calendar` part or `.ics` file, read for the
//! reader's invite card, and the REPLY that answers it; and a whole
//! calendar resource ([`parse_resource`]: the series, its edited single
//! occurrences, exceptions and alarms) for the calendar (`crate::cal`),
//! written with [`write_event`] and edited in place with [`patch`].
//!
//! The input is untrusted, so this module denies the lints below (as
//! `render/` does), refuses input over [`MAX_BYTES`], [`MAX_LINES`] lines
//! or [`MAX_PROPS`] properties before reading it, and returns `None` for
//! anything it can't read: the message then shows as it always did. There
//! is a TZID is resolved through a VTIMEZONE in the same file, else through
//! Windows' own zone rules (`crate::cal::tz`); otherwise its wall time is
//! shown with the zone's name. Recurrences are summarized here and
//! expanded by `crate::cal::recur`.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use crate::i18n::Lang;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeDelta, TimeZone, Utc, Weekday};

/// Larger calendar parts are not read.
pub const MAX_BYTES: usize = 256 * 1024;
/// Physical lines read at most.
pub const MAX_LINES: usize = 20_000;
/// Properties (unfolded lines) read at most.
pub const MAX_PROPS: usize = 5_000;
const MAX_DEPTH: usize = 8;
const MAX_ATTENDEES: usize = 200;
const MAX_OBSERVANCES: usize = 32;
/// Characters kept of a one-line field, and of the description.
const MAX_TEXT: usize = 300;
const MAX_DESCRIPTION: usize = 2_000;
/// Edited single occurrences of one resource read at most.
pub const MAX_OVERRIDES: usize = 500;
/// EXDATE plus RDATE values of one event read at most.
pub const MAX_DATES: usize = 1_000;
/// Alarms of one event read at most.
pub const MAX_ALARMS: usize = 8;

/// When an event starts or ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct When {
    /// The time as written (in its own zone).
    pub wall: NaiveDateTime,
    /// The instant (unix seconds), when the file says it: a UTC time, or a
    /// TZID with its VTIMEZONE.
    pub utc: Option<i64>,
    /// A date (`VALUE=DATE`): an all-day event.
    pub all_day: bool,
    /// The TZID of a time whose instant is unknown; empty for a floating
    /// time.
    pub zone: String,
    /// The TZID as written (empty for UTC, floating and dates).
    pub tzid: String,
}

/// An organizer or attendee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    /// The `mailto:` address, without the scheme.
    pub addr: String,
    /// `PARTSTAT`, upper case.
    pub partstat: String,
    /// False only for an explicit `RSVP=FALSE`.
    pub rsvp: bool,
}

/// The first event of a calendar, as the invite card shows it. Text is
/// unescaped and stripped of control and bidi characters.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// `METHOD`, upper case (`REQUEST`, `CANCEL`, `PUBLISH`...); may be
    /// empty.
    pub method: String,
    pub uid: String,
    pub sequence: i64,
    pub dtstamp: String,
    pub start: When,
    pub end: Option<When>,
    pub summary: String,
    pub location: String,
    /// Cut at [`MAX_DESCRIPTION`] characters.
    pub description: String,
    /// `STATUS`, upper case.
    pub status: String,
    pub organizer: Option<Person>,
    pub attendees: Vec<Person>,
    pub rrule: String,
    /// The `RECURRENCE-ID` line after its name (parameters and value, as
    /// written), echoed in a reply; empty for the whole series.
    pub recurrence_id: String,
    /// Where to join online: a conference property, `URL` or an address
    /// in `LOCATION` (http or https only).
    pub join: String,
}

/// An answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Accepted = 1,
    Tentative = 2,
    Declined = 3,
}

impl Answer {
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1 => Answer::Accepted,
            2 => Answer::Tentative,
            3 => Answer::Declined,
            _ => return None,
        })
    }

    pub fn partstat(self) -> &'static str {
        match self {
            Answer::Accepted => "ACCEPTED",
            Answer::Tentative => "TENTATIVE",
            Answer::Declined => "DECLINED",
        }
    }

    /// The reply's subject prefix ("Accepted: Team sync").
    pub fn label(self) -> &'static str {
        match self {
            Answer::Accepted => "Accepted",
            Answer::Tentative => "Tentative",
            Answer::Declined => "Declined",
        }
    }

    /// "You accepted", for the card.
    pub fn done(self) -> &'static str {
        match self {
            Answer::Accepted => "accepted",
            Answer::Tentative => "tentatively accepted",
            Answer::Declined => "declined",
        }
    }
}

/// One content line: upper-case name, parameters (upper-case names,
/// unquoted values) and the raw value.
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    /// The line after the name.
    rest: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A STANDARD or DAYLIGHT part of a VTIMEZONE.
#[derive(Default, Clone, Debug, PartialEq)]
struct Observance {
    tzid: String,
    start: Option<NaiveDateTime>,
    offset: Option<i64>,
    /// TZOFFSETFROM: the offset before this observance starts.
    from: Option<i64>,
    rrule: String,
}

/// Read the first event of an iCalendar text. `None` if it is too large,
/// not a calendar, or has no event with a start.
pub fn parse(bytes: &[u8]) -> Option<Event> {
    let cal = read(bytes, 1)?;
    let first = cal.events.first()?;
    build(cal.method, &first.props, &cal.zones)
}

/// One VEVENT's properties and its VALARMs' properties.
struct RawEvent {
    props: Vec<Prop>,
    alarms: Vec<Vec<Prop>>,
}

/// A calendar's METHOD, its top-level events (at most `max`) and its
/// time zone observances.
struct RawCal {
    method: String,
    events: Vec<RawEvent>,
    zones: Vec<Observance>,
}

fn read(bytes: &[u8], max: usize) -> Option<RawCal> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let text = String::from_utf8_lossy(bytes);
    let mut stack: Vec<String> = Vec::new();
    let mut calendar = false;
    let mut method = String::new();
    let mut events: Vec<RawEvent> = Vec::new();
    // Inside an event that is kept.
    let mut open = false;
    let (mut tzid, mut obs, mut zones) = (String::new(), None::<Observance>, Vec::new());
    for line in unfold(&text)? {
        let Some(p) = content_line(&line) else {
            continue;
        };
        let parent = stack.last().map(String::as_str);
        match p.name.as_str() {
            "BEGIN" => {
                if stack.len() >= MAX_DEPTH {
                    return None;
                }
                let kind = p.value.trim().to_ascii_uppercase();
                match (parent, kind.as_str()) {
                    (None, "VCALENDAR") => calendar = true,
                    (Some("VCALENDAR"), "VEVENT") if events.len() < max => {
                        events.push(RawEvent {
                            props: Vec::new(),
                            alarms: Vec::new(),
                        });
                        open = true;
                    }
                    (Some("VEVENT"), "VALARM") if open && stack.len() == 2 => {
                        if let Some(e) = events.last_mut().filter(|e| e.alarms.len() < MAX_ALARMS) {
                            e.alarms.push(Vec::new());
                        }
                    }
                    (Some("VCALENDAR"), "VTIMEZONE") => tzid.clear(),
                    (Some("VTIMEZONE"), "STANDARD" | "DAYLIGHT") => {
                        obs = Some(Observance {
                            tzid: tzid.clone(),
                            ..Default::default()
                        })
                    }
                    _ => {}
                }
                stack.push(kind);
            }
            "END" => match stack.pop().as_deref() {
                Some("VEVENT") if stack.len() == 1 => open = false,
                Some("STANDARD" | "DAYLIGHT") => {
                    if let Some(o) = obs.take().filter(|_| zones.len() < MAX_OBSERVANCES) {
                        zones.push(o);
                    }
                }
                _ => {}
            },
            name => match parent {
                Some("VCALENDAR") if name == "METHOD" => {
                    method = p.value.trim().to_ascii_uppercase();
                }
                Some("VEVENT") if stack.len() == 2 && open => {
                    if let Some(e) = events.last_mut() {
                        e.props.push(p);
                    }
                }
                Some("VALARM") if stack.len() == 3 && open => {
                    if let Some(a) = events.last_mut().and_then(|e| e.alarms.last_mut()) {
                        a.push(p);
                    }
                }
                Some("VTIMEZONE") if name == "TZID" => tzid = p.value.trim().to_string(),
                Some("STANDARD" | "DAYLIGHT") => {
                    if let Some(o) = obs.as_mut() {
                        match name {
                            "DTSTART" => o.start = date_time(&p.value).map(|(t, _)| t),
                            "TZOFFSETTO" => o.offset = utc_offset(&p.value),
                            "TZOFFSETFROM" => o.from = utc_offset(&p.value),
                            "RRULE" => o.rrule = p.value.to_ascii_uppercase(),
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
        }
    }
    calendar.then_some(RawCal {
        method,
        events,
        zones,
    })
}

/// The content lines of `text`, unfolded (a line starting with a space or
/// tab continues the one before). `None` past the caps.
fn unfold(text: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        if n >= MAX_LINES {
            return None;
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(more) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                if last.len().saturating_add(more.len()) > MAX_BYTES {
                    return None;
                }
                last.push_str(more);
            }
        } else if !line.is_empty() {
            if out.len() >= MAX_PROPS {
                return None;
            }
            out.push(line.to_string());
        }
    }
    Some(out)
}

/// `s` split at `sep` outside double quotes.
fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut quoted, mut from) = (Vec::new(), false, 0);
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == sep && !quoted {
            out.extend(s.get(from..i));
            from = i.saturating_add(1);
        }
    }
    out.extend(s.get(from..));
    out
}

fn content_line(line: &str) -> Option<Prop> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(i)
    })?;
    let head = line.get(..colon)?;
    let mut parts = split_unquoted(head, ';').into_iter();
    let first = parts.next()?;
    let name = first.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter_map(|p| {
            let (n, v) = p.split_once('=')?;
            Some((n.trim().to_ascii_uppercase(), v.trim().replace('"', "")))
        })
        .collect();
    Some(Prop {
        params,
        value: line.get(colon.saturating_add(1)..)?.to_string(),
        rest: line.get(first.len()..).unwrap_or_default().to_string(),
        name,
    })
}

fn build(method: String, props: &[Prop], zones: &[Observance]) -> Option<Event> {
    let get = |name: &str| props.iter().find(|p| p.name == name);
    let text = |name: &str| get(name).map_or(String::new(), |p| one_line(&unescape(&p.value)));
    let start = when(get("DTSTART")?, zones)?;
    let end = get("DTEND").and_then(|p| when(p, zones)).or_else(|| {
        let d = duration(&get("DURATION")?.value)?;
        Some(When {
            wall: start.wall.checked_add_signed(d)?,
            utc: match start.utc {
                Some(t) => Some(t.checked_add(d.num_seconds())?),
                None => None,
            },
            ..start.clone()
        })
    });
    let join = [
        "X-GOOGLE-CONFERENCE",
        "X-MICROSOFT-SKYPETEAMSMEETINGURL",
        "X-MICROSOFT-ONLINEMEETINGCONFURL",
        "URL",
    ]
    .iter()
    .find_map(|n| web(&get(n)?.value))
    .or_else(|| url_in(&text("LOCATION")))
    .unwrap_or_default();
    let mut description = clean(&unescape(
        get("DESCRIPTION").map_or("", |p| p.value.as_str()),
    ));
    if let Some((cut, _)) = description.char_indices().nth(MAX_DESCRIPTION) {
        description.truncate(cut);
        description.push('…');
    }
    Some(Event {
        method,
        uid: text("UID"),
        sequence: get("SEQUENCE")
            .and_then(|p| p.value.trim().parse().ok())
            .unwrap_or(0),
        dtstamp: text("DTSTAMP"),
        start,
        end,
        summary: text("SUMMARY"),
        location: text("LOCATION"),
        description: description.trim().to_string(),
        status: text("STATUS").to_ascii_uppercase(),
        organizer: get("ORGANIZER").and_then(person),
        attendees: props
            .iter()
            .filter(|p| p.name == "ATTENDEE")
            .filter_map(person)
            .take(MAX_ATTENDEES)
            .collect(),
        rrule: text("RRULE").to_ascii_uppercase(),
        recurrence_id: get("RECURRENCE-ID").map_or(String::new(), |p| clean(&p.rest)),
        join,
    })
}

/// TEXT unescaped (RFC 5545 3.3.11).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

/// Without control characters (except newlines) and the invisible bidi
/// and zero-width characters that can disguise text.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            let hidden = matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
            !(hidden || (c.is_control() && c != '\n'))
        })
        .collect()
}

/// A one-line field: cleaned, newlines as spaces, trimmed and capped.
fn one_line(s: &str) -> String {
    let s: String = clean(s)
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .take(MAX_TEXT)
        .collect();
    s.trim().to_string()
}

fn person(p: &Prop) -> Option<Person> {
    let v = p.value.trim();
    let addr = v
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("mailto:"))
        .and_then(|_| v.get(7..))?
        .trim();
    // A plain address only: a comma, bracket or quote could add recipients to
    // the reply, which goes to the organizer alone.
    let valid = addr.len() <= 254
        && addr.contains('@')
        && !addr
            .contains(|c: char| c.is_whitespace() || c.is_control() || ",;<>()[]\\:\"".contains(c))
        && addr.parse::<lettre::Address>().is_ok();
    valid.then(|| Person {
        name: one_line(p.param("CN").unwrap_or_default()),
        addr: one_line(addr),
        partstat: p
            .param("PARTSTAT")
            .unwrap_or("NEEDS-ACTION")
            .to_ascii_uppercase(),
        rsvp: !p
            .param("RSVP")
            .is_some_and(|r| r.eq_ignore_ascii_case("FALSE")),
    })
}

/// An http or https address, checked as clicked links are.
fn web(url: &str) -> Option<String> {
    let url = super::render::links::sanitize(url.trim())?;
    (url.starts_with("https:") || url.starts_with("http:")).then_some(url)
}

/// The first web address in a text (a `LOCATION` that is a meeting link).
fn url_in(text: &str) -> Option<String> {
    let at = text.to_ascii_lowercase().find("http")?;
    let url = text.get(at..)?.split([' ', '<', '>', '"']).next()?;
    web(url.trim_end_matches(['.', ',', ';', ')']))
}

/// `YYYYMMDD[THHMMSS[Z]]`: the time, and whether it is UTC.
fn date_time(value: &str) -> Option<(NaiveDateTime, bool)> {
    let v = value.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<u32>().ok();
    let year = i32::try_from(num(0, 4)?).ok()?;
    let date = NaiveDate::from_ymd_opt(year, num(4, 6)?, num(6, 8)?)?;
    if v.len() == 8 {
        return date.and_hms_opt(0, 0, 0).map(|t| (t, false));
    }
    if v.get(8..9) != Some("T") {
        return None;
    }
    let time = date.and_hms_opt(num(9, 11)?, num(11, 13)?, num(13, 15)?)?;
    match v.get(15..) {
        Some("") => Some((time, false)),
        Some("Z" | "z") => Some((time, true)),
        _ => None,
    }
}

/// `DTSTART` or `DTEND`, resolved as far as the file allows.
fn when(p: &Prop, zones: &[Observance]) -> Option<When> {
    let date = p
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
    let tzid = p.param("TZID").map(one_line).unwrap_or_default();
    when_value(&p.value, date, &tzid, zones)
}

/// One date or date-time value with its `VALUE=DATE` and TZID.
fn when_value(value: &str, date: bool, tzid: &str, zones: &[Observance]) -> Option<When> {
    let (wall, utc) = date_time(value)?;
    let all_day = date || value.trim().len() == 8;
    let instant = if all_day {
        None
    } else if utc {
        Some(Utc.from_utc_datetime(&wall).timestamp())
    } else {
        zoned(zones, tzid, wall)
    };
    let tzid = if all_day || utc { "" } else { tzid };
    Some(When {
        wall,
        utc: instant,
        all_day,
        zone: if instant.is_none() && !all_day {
            tzid.to_string()
        } else {
            String::new()
        },
        tzid: tzid.to_string(),
    })
}

/// The instant of wall time `wall` in zone `tzid`: through the file's
/// VTIMEZONE, else Windows' rules for the zone; `None` for a floating
/// time or an unknown zone.
fn zoned(zones: &[Observance], tzid: &str, wall: NaiveDateTime) -> Option<i64> {
    if tzid.is_empty() {
        return None;
    }
    match offset_at(zones, tzid, wall) {
        Some(off) => Utc.from_utc_datetime(&wall).timestamp().checked_sub(off),
        None => crate::cal::tz::to_utc(tzid, wall),
    }
}

/// `+HHMM[SS]` or `-HHMM[SS]` in seconds.
fn utc_offset(value: &str) -> Option<i64> {
    let v = value.trim();
    let sign = match v.get(..1)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let num = |a: usize, b: usize| v.get(a..b).map_or(Some(0), |s| s.parse::<i64>().ok());
    let (h, m, s) = (num(1, 3)?, num(3, 5)?, num(5, 7)?);
    if v.len() < 5 || h > 23 || m > 59 || s > 59 {
        return None;
    }
    h.checked_mul(3600)?
        .checked_add(m.checked_mul(60)?)?
        .checked_add(s)?
        .checked_mul(sign)
}

/// The UTC offset of zone `tzid` at local time `wall`, from its VTIMEZONE:
/// the observance whose latest onset is at or before `wall`.
///
/// A wall time in the gap an onset opens (clocks jump forward) takes the
/// offset before it (RFC 5545 3.3.5); one in an overlap takes the first
/// occurrence (the observance before the onset).
fn offset_at(zones: &[Observance], tzid: &str, wall: NaiveDateTime) -> Option<i64> {
    let ours: Vec<&Observance> = zones.iter().filter(|o| o.tzid == tzid).collect();
    let latest = ours
        .iter()
        .flat_map(|o| {
            let years = [wall.year().checked_sub(1), Some(wall.year())];
            years
                .into_iter()
                .flatten()
                .filter_map(|y| onset(o, y))
                .filter(|t| *t <= wall)
                .map(move |t| (t, *o))
        })
        .max_by_key(|(t, _)| *t);
    if let Some((t, o)) = latest {
        // In the gap: the offset before the onset.
        if let (Some(to), Some(from)) = (o.offset, o.from)
            && to > from
            && let Some(gap) = to.checked_sub(from).and_then(TimeDelta::try_seconds)
            && t.checked_add_signed(gap).is_some_and(|end| wall < end)
        {
            return Some(from);
        }
        return o.offset;
    }
    // Before every onset: the first observance's offset.
    ours.first().and_then(|o| o.offset)
}

/// When observance `o` starts in `year`: its `DTSTART` if it has no
/// yearly rule (and starts that year), else the rule's day (`BYMONTH`
/// with `BYDAY` such as `-1SU`) at the `DTSTART` time.
fn onset(o: &Observance, year: i32) -> Option<NaiveDateTime> {
    let start = o.start?;
    if o.rrule.is_empty() {
        return (start.year() == year).then_some(start);
    }
    if year < start.year() {
        return None;
    }
    let rule = |key: &str| {
        o.rrule
            .split(';')
            .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
    };
    if rule("FREQ") != Some("YEARLY") {
        return None;
    }
    let month = rule("BYMONTH").map_or(Some(start.month()), |m| m.parse().ok())?;
    let day = match rule("BYDAY") {
        Some(by) => {
            let split = by.len().checked_sub(2)?;
            let (n, wd) = (by.get(..split)?, weekday(by.get(split..)?)?);
            let n: i8 = if n.is_empty() { 1 } else { n.parse().ok()? };
            nth_weekday(year, month, wd, n)?
        }
        None => NaiveDate::from_ymd_opt(year, month, start.day())?,
    };
    let t = day.and_time(start.time());
    let until = rule("UNTIL").and_then(date_time).map(|(u, _)| u);
    until.is_none_or(|u| t <= u).then_some(t)
}

fn weekday(s: &str) -> Option<Weekday> {
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

use crate::cal::tz::nth_weekday;

/// A non-negative `DURATION` (`P1W`, `P1DT2H`, `PT30M`).
pub(crate) fn duration(value: &str) -> Option<TimeDelta> {
    let v = value.trim();
    let v = v.strip_prefix('+').unwrap_or(v).strip_prefix('P')?;
    let (mut secs, mut num, mut time) = (0i64, 0i64, false);
    for c in v.chars() {
        let unit = match c {
            '0'..='9' => {
                num = num
                    .checked_mul(10)?
                    .checked_add(i64::from(c.to_digit(10)?))?;
                continue;
            }
            'T' => {
                time = true;
                continue;
            }
            'W' => 604_800,
            'D' => 86_400,
            'H' if time => 3_600,
            'M' if time => 60,
            'S' if time => 1,
            _ => return None,
        };
        secs = secs.checked_add(num.checked_mul(unit)?)?;
        num = 0;
    }
    TimeDelta::try_seconds(secs)
}

impl When {
    /// The wall time to show: in `tz` when the instant is known.
    fn shown<Tz: TimeZone>(&self, tz: &Tz) -> NaiveDateTime {
        self.utc
            .and_then(|t| tz.timestamp_opt(t, 0).single())
            .map_or(self.wall, |d| d.naive_local())
    }

    /// As a property's parameters and value (`:20261008T130000Z`).
    fn value(&self) -> String {
        match (self.all_day, self.utc) {
            (true, _) => self.wall.format(";VALUE=DATE:%Y%m%d").to_string(),
            (false, Some(t)) => DateTime::from_timestamp(t, 0)
                .unwrap_or_default()
                .format(":%Y%m%dT%H%M%SZ")
                .to_string(),
            (false, None) if self.zone.is_empty() => self.wall.format(":%Y%m%dT%H%M%S").to_string(),
            (false, None) => format!(
                ";TZID=\"{}\"{}",
                self.zone.replace('"', ""),
                self.wall.format(":%Y%m%dT%H%M%S")
            ),
        }
    }
}

impl Event {
    /// Cancelled by the organizer: `METHOD:CANCEL` or `STATUS:CANCELLED`.
    pub fn cancelled(&self) -> bool {
        self.method == "CANCEL" || self.status == "CANCELLED"
    }

    /// Over by `now`: its end (or start) has passed; a recurring event
    /// only after its `UNTIL` date.
    pub fn past<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> bool {
        let today = now.naive_local();
        if !self.rrule.is_empty() {
            return self
                .rrule
                .split(';')
                .find_map(|kv| kv.strip_prefix("UNTIL="))
                .and_then(|u| date_time(u.get(..8)?))
                .map(|(u, _)| u.date())
                .is_some_and(|u| u < today.date());
        }
        let end = self.end.as_ref().unwrap_or(&self.start);
        match end.utc {
            Some(t) => t < now.timestamp(),
            None if end.all_day && self.end.is_none() => end.wall.date() < today.date(),
            None => end.wall < today,
        }
    }

    /// The attendee that is the user: the one with the account's address,
    /// else (another address of the user's the invite was sent to) the one
    /// attendee among `others` (the message's To and Cc), only when there
    /// is exactly one: never one of several other recipients.
    pub fn me(&self, account: &str, others: &[&str]) -> Option<&Person> {
        let is = |p: &&Person, a: &str| p.addr.eq_ignore_ascii_case(a);
        if let Some(p) = self.attendees.iter().find(|p| is(p, account)) {
            return Some(p);
        }
        let mut listed = self
            .attendees
            .iter()
            .filter(|p| others.iter().any(|a| is(p, a)));
        match (listed.next(), listed.next()) {
            (Some(p), None) => Some(p),
            _ => None,
        }
    }

    /// Whether the card offers Accept, Tentative and Decline: a request
    /// with an organizer to answer, not cancelled, not over, and not
    /// `RSVP=FALSE` for `me`.
    pub fn answerable(&self, me: Option<&Person>, past: bool) -> bool {
        self.method == "REQUEST"
            && self.organizer.is_some()
            && !self.cancelled()
            && !past
            && me.is_none_or(|p| p.rsvp)
    }

    /// When, in `tz`: "Thu 2026-10-08 14:00-15:00", a date for an all-day
    /// event, and the zone's name when the instant is unknown.
    pub fn when_text<Tz: TimeZone>(&self, tz: &Tz) -> String {
        self.when_text_in(Lang::En, tz)
    }

    /// `when_text` in the language shown: English `Thu 2026-10-08 14:00-15:00`,
    /// pt-PT `qui. 08/10/2026 14:00-15:00`.
    pub fn when_text_in<Tz: TimeZone>(&self, lang: Lang, tz: &Tz) -> String {
        let day = |d: NaiveDate| match lang {
            Lang::En => d.format("%a %Y-%m-%d").to_string(),
            Lang::PtPt => format!(
                "{} {}",
                weekday_name(lang, d.weekday()),
                d.format("%d/%m/%Y")
            ),
        };
        let all_day = match lang {
            Lang::En => "all day",
            Lang::PtPt => "todo o dia",
        };
        let s = &self.start;
        if s.all_day {
            let first = s.wall.date();
            // An all-day DTEND is the day after the last.
            let last = self.end.as_ref().and_then(|e| e.wall.date().pred_opt());
            return match last.filter(|l| *l > first) {
                Some(l) => format!("{} - {}, {all_day}", day(first), day(l)),
                None => format!("{}, {all_day}", day(first)),
            };
        }
        let start = s.shown(tz);
        let mut out = format!("{} {}", day(start.date()), start.format("%H:%M"));
        if let Some(end) = self.end.as_ref().map(|e| e.shown(tz)) {
            if end.date() == start.date() {
                out.push_str(&end.format("-%H:%M").to_string());
            } else {
                out.push_str(&format!(" - {} {}", day(end.date()), end.format("%H:%M")));
            }
        }
        if s.utc.is_none() {
            let zone = if s.zone.is_empty() {
                match lang {
                    Lang::En => "floating time",
                    Lang::PtPt => "hora flutuante",
                }
            } else {
                &s.zone
            };
            out.push_str(&format!(" ({zone})"));
        }
        out
    }

    /// The recurrence in a few words ("Repeats weekly on Mon, Wed until
    /// 2026-12-31"); "Repeats" for a rule beyond that; empty if none.
    pub fn repeats(&self) -> String {
        self.repeats_in(Lang::En)
    }

    /// `repeats` in the language shown (pt-PT: "Repete-se semanalmente a
    /// seg., qua. até 31/12/2026").
    pub fn repeats_in(&self, lang: Lang) -> String {
        if self.rrule.is_empty() {
            return String::new();
        }
        let plain = || match lang {
            Lang::En => "Repeats".to_string(),
            Lang::PtPt => "Repete-se".to_string(),
        };
        let (mut freq, mut every, mut days, mut until) = ("", 1u32, Vec::new(), None);
        for kv in self.rrule.split(';') {
            let Some((k, v)) = kv.split_once('=') else {
                return plain();
            };
            match k {
                "FREQ" => freq = v,
                "INTERVAL" => every = v.parse().unwrap_or(0),
                "BYDAY" => days = v.split(',').map(weekday).collect(),
                "UNTIL" => until = v.get(..8).and_then(date_time).map(|(u, _)| u.date()),
                "COUNT" | "WKST" => {}
                _ => return plain(),
            }
        }
        // (adverb, plural unit) for the interval of one and of several.
        let (adverb, unit) = match (lang, freq) {
            (Lang::En, "DAILY") => ("daily", "days"),
            (Lang::En, "WEEKLY") => ("weekly", "weeks"),
            (Lang::En, "MONTHLY") => ("monthly", "months"),
            (Lang::En, "YEARLY") => ("yearly", "years"),
            (Lang::PtPt, "DAILY") => ("diariamente", "dias"),
            (Lang::PtPt, "WEEKLY") => ("semanalmente", "semanas"),
            (Lang::PtPt, "MONTHLY") => ("mensalmente", "meses"),
            (Lang::PtPt, "YEARLY") => ("anualmente", "anos"),
            _ => return plain(),
        };
        let mut out = match (every, lang) {
            (0, _) => return plain(),
            (1, Lang::En) => format!("Repeats {adverb}"),
            (1, Lang::PtPt) => format!("Repete-se {adverb}"),
            (n, Lang::En) => format!("Repeats every {n} {unit}"),
            (n, Lang::PtPt) => format!("Repete-se a cada {n} {unit}"),
        };
        if !days.is_empty() {
            let names: Option<Vec<String>> = days
                .into_iter()
                .map(|d| d.map(|d| weekday_name(lang, d).to_string()))
                .collect();
            match names {
                Some(names) if freq == "WEEKLY" => {
                    out.push_str(match lang {
                        Lang::En => " on ",
                        Lang::PtPt => " a ",
                    });
                    out.push_str(&names.join(", "));
                }
                _ => return plain(),
            }
        }
        if let Some(u) = until {
            match lang {
                Lang::En => out.push_str(&u.format(" until %Y-%m-%d").to_string()),
                Lang::PtPt => out.push_str(&u.format(" até %d/%m/%Y").to_string()),
            }
        }
        out
    }
}

/// A weekday abbreviated: `Mon`; pt-PT `seg.`.
fn weekday_name(lang: Lang, d: Weekday) -> &'static str {
    const EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const PT: [&str; 7] = ["seg.", "ter.", "qua.", "qui.", "sex.", "sáb.", "dom."];
    let i = d.num_days_from_monday() as usize;
    match lang {
        Lang::En => EN.get(i).copied().unwrap_or_default(),
        Lang::PtPt => PT.get(i).copied().unwrap_or_default(),
    }
}

/// The iTIP REPLY (RFC 5546 3.2.3) answering `ev` for attendee `addr`
/// (`name` may be empty): the same UID, SEQUENCE and RECURRENCE-ID, the
/// organizer, one ATTENDEE with the answer and a fresh DTSTAMP (`now`),
/// with CRLF line ends and lines folded at 75 octets.
pub fn reply(ev: &Event, name: &str, addr: &str, answer: Answer, now: DateTime<Utc>) -> String {
    let cn = |name: &str| {
        let name = one_line(name).replace('"', "'");
        if name.is_empty() {
            String::new()
        } else {
            format!(";CN=\"{name}\"")
        }
    };
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "PRODID:-//Komorebi//Komorebi//EN".into(),
        "VERSION:2.0".into(),
        "METHOD:REPLY".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", escape(&ev.uid)),
        format!("SEQUENCE:{}", ev.sequence),
        now.format("DTSTAMP:%Y%m%dT%H%M%SZ").to_string(),
        format!("DTSTART{}", ev.start.value()),
    ];
    if !ev.recurrence_id.is_empty() {
        lines.push(format!("RECURRENCE-ID{}", one_line(&ev.recurrence_id)));
    }
    if let Some(o) = &ev.organizer {
        lines.push(format!("ORGANIZER{}:mailto:{}", cn(&o.name), o.addr));
    }
    lines.push(format!(
        "ATTENDEE;PARTSTAT={}{}:mailto:{}",
        answer.partstat(),
        cn(name),
        one_line(addr)
    ));
    if !ev.summary.is_empty() {
        lines.push(format!("SUMMARY:{}", escape(&ev.summary)));
    }
    lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
    lines.iter().map(|l| fold(l)).collect()
}

/// The message answering `ev` from the account `from_name <from_email>`
/// for attendee `me` (name, address): addressed only to the organizer (no
/// Cc, no other attendee), subject "Accepted: <title>", a line of text and
/// the REPLY as its `text/calendar` alternative. `None` without an
/// organizer to answer.
pub fn reply_message(
    ev: &Event,
    from_name: &str,
    from_email: &str,
    me: (&str, &str),
    answer: Answer,
    now: DateTime<Utc>,
) -> Option<super::smtp::Outgoing> {
    let organizer = ev.organizer.as_ref()?;
    let who = if from_name.trim().is_empty() {
        from_email
    } else {
        from_name.trim()
    };
    Some(super::smtp::Outgoing {
        from_name: from_name.into(),
        from_email: from_email.into(),
        to: super::smtp::mailbox(&organizer.name, &organizer.addr),
        subject: format!("{}: {}", answer.label(), ev.summary),
        body: format!("{who} {} this invitation.\n", answer.done()),
        message_id: super::smtp::message_id(from_email),
        calendar: Some(super::smtp::CalendarPart {
            method: "REPLY",
            ics: reply(ev, me.0, me.1, answer, now),
        }),
        ..Default::default()
    })
}

/// TEXT escaped (RFC 5545 3.3.11).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | ';' | ',' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// One line folded at 75 octets, never inside a character, ending in CRLF.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len().saturating_add(8));
    let mut len = 0usize;
    for c in line.chars() {
        if len.saturating_add(c.len_utf8()) > 75 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len = len.saturating_add(c.len_utf8());
    }
    out.push_str("\r\n");
    out
}

/// A reminder (VALARM): minutes before the start (or the end), negative
/// for after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alarm {
    pub before_min: i64,
    pub related_end: bool,
}

/// One VEVENT of a resource with what the calendar needs beyond the card.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub event: Event,
    pub exdates: Vec<When>,
    pub rdates: Vec<When>,
    pub alarms: Vec<Alarm>,
    /// `TRANSP`, upper case (`TRANSPARENT`: doesn't block time).
    pub transp: String,
}

/// A calendar resource (one UID): the series or single event, and its
/// edited single occurrences (with `RECURRENCE-ID`).
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub method: String,
    pub master: Option<Item>,
    pub overrides: Vec<Item>,
    zones: Vec<Observance>,
}

impl Resource {
    /// The instant of wall time `wall` in zone `tzid`, as the resource's
    /// own VTIMEZONE (else Windows) says; `None` when floating or unknown.
    pub fn to_utc(&self, tzid: &str, wall: NaiveDateTime) -> Option<i64> {
        zoned(&self.zones, tzid, wall)
    }

    /// The UID (the master's, else the first override's).
    pub fn uid(&self) -> &str {
        self.master
            .as_ref()
            .or(self.overrides.first())
            .map_or("", |i| i.event.uid.as_str())
    }
}

/// A whole resource: every top-level VEVENT (one master, up to
/// [`MAX_OVERRIDES`] overrides). `None` past the caps, when it isn't a
/// calendar, or when no event has a start.
pub fn parse_resource(bytes: &[u8]) -> Option<Resource> {
    let cal = read(bytes, MAX_OVERRIDES.saturating_add(1))?;
    let mut master = None;
    let mut overrides = Vec::new();
    for raw in &cal.events {
        let Some(event) = build(cal.method.clone(), &raw.props, &cal.zones) else {
            continue;
        };
        let mut dates = 0usize;
        let mut list = |name: &str| -> Vec<When> {
            let mut out = Vec::new();
            for p in raw.props.iter().filter(|p| p.name == name) {
                let date = p
                    .param("VALUE")
                    .is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
                let tzid = p.param("TZID").map(one_line).unwrap_or_default();
                for v in p.value.split(',') {
                    if dates >= MAX_DATES {
                        return out;
                    }
                    dates = dates.saturating_add(1);
                    // A PERIOD (RDATE): its start.
                    let v = v.split('/').next().unwrap_or_default();
                    out.extend(when_value(v, date, &tzid, &cal.zones));
                }
            }
            out
        };
        let exdates = list("EXDATE");
        let rdates = list("RDATE");
        let alarms = raw.alarms.iter().filter_map(|a| alarm(a)).collect();
        let transp = raw
            .props
            .iter()
            .find(|p| p.name == "TRANSP")
            .map_or(String::new(), |p| p.value.trim().to_ascii_uppercase());
        let item = Item {
            event,
            exdates,
            rdates,
            alarms,
            transp,
        };
        if item.event.recurrence_id.is_empty() && master.is_none() {
            master = Some(item);
        } else if !item.event.recurrence_id.is_empty() {
            overrides.push(item);
        }
    }
    if master.is_none() && overrides.is_empty() {
        return None;
    }
    Some(Resource {
        method: cal.method,
        master,
        overrides,
        zones: cal.zones,
    })
}

/// A VALARM's trigger as minutes before; only relative triggers.
fn alarm(props: &[Prop]) -> Option<Alarm> {
    let t = props.iter().find(|p| p.name == "TRIGGER")?;
    if t.param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
    {
        return None;
    }
    let v = t.value.trim();
    let (sign, rest) = match v.strip_prefix('-') {
        Some(r) => (1i64, r),
        None => (-1i64, v.strip_prefix('+').unwrap_or(v)),
    };
    let mins = duration(rest)?.num_minutes().checked_mul(sign)?;
    Some(Alarm {
        before_min: mins,
        related_end: t
            .param("RELATED")
            .is_some_and(|r| r.eq_ignore_ascii_case("END")),
    })
}

/// The logical lines of a raw resource: each line with its folded
/// continuation lines, as written (line ends kept).
fn logical_lines(raw: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut pos = 0usize;
    for line in raw.split_inclusive('\n') {
        let next = pos.saturating_add(line.len());
        if pos > start && !line.starts_with([' ', '\t']) {
            out.extend(raw.get(start..pos));
            start = pos;
        }
        pos = next;
    }
    if pos > start {
        out.extend(raw.get(start..pos));
    }
    out
}

/// The property name of a logical line (upper case).
fn line_name(line: &str) -> String {
    line.split([':', ';'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase()
}

/// The raw text without the calendar's METHOD (a stored copy of an
/// invitation, which a CalDAV resource must not carry), CRLF kept.
pub fn strip_method(raw: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::with_capacity(raw.len());
    for l in logical_lines(raw) {
        let name = line_name(l);
        match name.as_str() {
            "BEGIN" => depth = depth.saturating_add(1),
            "END" => depth = depth.saturating_sub(1),
            "METHOD" if depth == 1 => continue,
            _ => {}
        }
        out.push_str(l);
    }
    out
}

/// What [`write_event`] and [`patch`] write.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start: When,
    pub end: Option<When>,
    /// An RRULE value, or empty.
    pub rrule: String,
    /// Minutes before the start for Komorebi's reminder.
    pub alarm_min: Option<i64>,
    pub now: DateTime<Utc>,
}

impl Edit {
    fn lines(&self) -> Vec<String> {
        let mut l = vec![format!("DTSTART{}", self.start.value())];
        if let Some(e) = &self.end {
            l.push(format!("DTEND{}", e.value()));
        }
        for (name, v) in [
            ("SUMMARY", &self.summary),
            ("LOCATION", &self.location),
            ("DESCRIPTION", &self.description),
        ] {
            if !v.trim().is_empty() {
                l.push(format!("{name}:{}", escape(&clean(v))));
            }
        }
        if !self.rrule.is_empty() {
            l.push(format!("RRULE:{}", one_line(&self.rrule).replace(':', "")));
        }
        l
    }

    fn alarm(&self) -> Vec<String> {
        match self.alarm_min {
            Some(m) => vec![
                "BEGIN:VALARM".into(),
                "X-KOMOREBI:1".into(),
                "ACTION:DISPLAY".into(),
                "DESCRIPTION:Reminder".into(),
                format!("TRIGGER:-PT{}M", m.max(0)),
                "END:VALARM".into(),
            ],
            None => Vec::new(),
        }
    }
}

/// A new resource for `e`: VCALENDAR (PRODID `-//Komorebi//EN`, no
/// METHOD) with one VEVENT, CRLF ends, folded at 75 octets.
pub fn write_event(e: &Edit) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Komorebi//EN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", escape(&e.uid)),
        e.now.format("DTSTAMP:%Y%m%dT%H%M%SZ").to_string(),
        "SEQUENCE:0".into(),
    ];
    lines.extend(e.lines());
    lines.extend(e.alarm());
    lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
    lines.iter().map(|l| fold(l)).collect()
}

/// One edited occurrence of a series, as a VCALENDAR holding a single
/// VEVENT with `RECURRENCE-ID{rid}` (`rid`: the parameters and value after
/// the name, in the series' own form), for merging into the resource.
pub fn override_event(e: &Edit, rid: &str) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", escape(&e.uid)),
        e.now.format("DTSTAMP:%Y%m%dT%H%M%SZ").to_string(),
        format!("RECURRENCE-ID{}", one_line(rid)),
        "SEQUENCE:0".into(),
    ];
    lines.extend(e.lines());
    lines.extend(e.alarm());
    lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
    lines.iter().map(|l| fold(l)).collect()
}

/// The master VEVENT of `raw` edited: SUMMARY, LOCATION, DESCRIPTION,
/// DTSTART, DTEND/DURATION, RRULE and the VALARM Komorebi wrote replaced,
/// SEQUENCE bumped and DTSTAMP renewed; every other line kept byte for
/// byte. `None` when `raw` has no master event.
pub fn patch(raw: &str, e: &Edit) -> Option<String> {
    const REPLACED: [&str; 10] = [
        "SUMMARY",
        "LOCATION",
        "DESCRIPTION",
        "DTSTART",
        "DTEND",
        "DURATION",
        "RRULE",
        "SEQUENCE",
        "DTSTAMP",
        "LAST-MODIFIED",
    ];
    let lines = logical_lines(raw);
    let mut out = String::with_capacity(raw.len().saturating_add(256));
    let mut stack: Vec<String> = Vec::new();
    // The VEVENT being read is the master (no RECURRENCE-ID): decided at
    // its end, so its lines are held until then.
    let mut held: Vec<&str> = Vec::new();
    let mut in_ours_alarm = false;
    let mut alarm_buf: Vec<&str> = Vec::new();
    let mut done = false;
    let mut sequence = 0i64;
    for l in lines {
        let name = line_name(l);
        let value = || {
            l.split_once(':')
                .map_or("", |(_, v)| v)
                .trim()
                .to_ascii_uppercase()
        };
        let depth = stack.len();
        if name == "BEGIN" {
            stack.push(value());
        }
        let in_event = stack.get(1).is_some_and(|k| k == "VEVENT") && !done;
        if !in_event {
            if name == "END" {
                stack.pop();
            }
            out.push_str(l);
            continue;
        }
        // Inside a top-level VEVENT.
        if name == "BEGIN" && depth == 2 && value() == "VALARM" {
            in_ours_alarm = true;
            alarm_buf.clear();
            alarm_buf.push(l);
            continue;
        }
        if in_ours_alarm {
            alarm_buf.push(l);
            if name == "END" && stack.len() == 3 {
                stack.pop();
                in_ours_alarm = false;
                // Komorebi's own alarm is dropped (rewritten); others kept.
                if !alarm_buf.iter().any(|a| line_name(a) == "X-KOMOREBI") {
                    held.extend(alarm_buf.iter());
                }
            } else if name == "END" {
                stack.pop();
            }
            continue;
        }
        if name == "END" && depth == 2 {
            stack.pop();
            let master = !held.iter().any(|h| line_name(h) == "RECURRENCE-ID");
            if master {
                for h in held.drain(..) {
                    let n = line_name(h);
                    if n == "SEQUENCE" {
                        sequence = h
                            .split_once(':')
                            .and_then(|(_, v)| v.trim().parse().ok())
                            .unwrap_or(0);
                    }
                    if !REPLACED.contains(&n.as_str()) {
                        out.push_str(h);
                    }
                }
                let mut new = vec![
                    format!("SEQUENCE:{}", sequence.saturating_add(1)),
                    e.now.format("DTSTAMP:%Y%m%dT%H%M%SZ").to_string(),
                ];
                new.extend(e.lines());
                new.extend(e.alarm());
                for n in new {
                    out.push_str(&fold(&n));
                }
                done = true;
            } else {
                for h in held.drain(..) {
                    out.push_str(h);
                }
            }
            out.push_str(l);
            continue;
        }
        if name == "END" {
            stack.pop();
        }
        held.push(l);
    }
    done.then_some(out)
}

/// `raw` with attendee `addr`'s PARTSTAT set to `partstat` in every
/// VEVENT (and RSVP dropped); `None` if `addr` is not an attendee.
pub fn set_partstat(raw: &str, addr: &str, partstat: &str) -> Option<String> {
    let mut found = false;
    let mut out = String::with_capacity(raw.len());
    for l in logical_lines(raw) {
        if line_name(l) != "ATTENDEE" {
            out.push_str(l);
            continue;
        }
        let unfolded: String = l
            .replace("\r\n ", "")
            .replace("\r\n\t", "")
            .replace("\n ", "")
            .trim_end_matches(['\r', '\n'])
            .to_string();
        let Some(p) = content_line(&unfolded) else {
            out.push_str(l);
            continue;
        };
        let theirs = p.value.trim();
        let mine = theirs
            .get(..7)
            .filter(|s| s.eq_ignore_ascii_case("mailto:"))
            .and_then(|_| theirs.get(7..))
            .is_some_and(|a| a.trim().eq_ignore_ascii_case(addr));
        if !mine {
            out.push_str(l);
            continue;
        }
        found = true;
        let head = unfolded.get(
            ..unfolded
                .len()
                .saturating_sub(p.value.len())
                .saturating_sub(1),
        );
        let params: Vec<&str> = split_unquoted(head.unwrap_or("ATTENDEE"), ';')
            .into_iter()
            .skip(1)
            .filter(|kv| {
                let k = kv
                    .split('=')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_uppercase();
                k != "PARTSTAT" && k != "RSVP"
            })
            .collect();
        let mut line = String::from("ATTENDEE");
        for kv in params {
            line.push(';');
            line.push_str(kv);
        }
        line.push_str(";PARTSTAT=");
        line.push_str(partstat);
        line.push(':');
        line.push_str(&p.value);
        out.push_str(&fold(&line));
    }
    found.then_some(out)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn cal(event: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n{event}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    #[test]
    fn unfolds_unescapes_and_splits_quoted_params() {
        let ev = parse(cal(
            "UID:a@x\r\nDTSTART:20261008T130000Z\r\nSUMMARY:Plan\\, review \\; sign\r\n  off\\nnow\r\n\
             ORGANIZER;CN=\"Doe, Jane: PM\";SENT-BY=\"mailto:a@x\":MAILTO:jane@x.org\r\n\
             ATTENDEE;RSVP=FALSE;PARTSTAT=accepted:mailto:me@x.org\r\n\
             DESCRIPTION:Line 1\\nLine 2\u{202E}\u{7}\r\n",
        ).as_bytes())
        .unwrap();
        assert_eq!(ev.summary, "Plan, review ; sign off now");
        let o = ev.organizer.unwrap();
        assert_eq!(
            (o.name.as_str(), o.addr.as_str()),
            ("Doe, Jane: PM", "jane@x.org")
        );
        assert_eq!(ev.attendees[0].partstat, "ACCEPTED");
        assert!(!ev.attendees[0].rsvp);
        assert_eq!(ev.description, "Line 1\nLine 2");
        assert_eq!(ev.method, "REQUEST");
    }

    #[test]
    fn organizer_address_cannot_add_recipients() {
        for bad in [
            "a@x.org,evil@y.org",
            "a@x.org;evil@y.org",
            "a@x.org>,<e@y.org",
            "\"a,b\"@x.org",
        ] {
            let ev = parse(
                cal(&format!(
                    "UID:a@x
DTSTART:20261008T130000Z
ORGANIZER:mailto:{bad}
"
                ))
                .as_bytes(),
            )
            .unwrap();
            assert!(ev.organizer.is_none(), "{bad}");
        }
    }

    #[test]
    fn garbage_is_none() {
        for raw in [
            "",
            "hello",
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
            "BEGIN:VEVENT\r\nDTSTART:20261008\r\nEND:VEVENT",
            &cal("UID:x\r\nDTSTART:2026\r\n"),
            &cal("UID:x\r\nDTSTART:20261340T000000\r\n"),
        ] {
            assert!(parse(raw.as_bytes()).is_none(), "{raw}");
        }
        let deep = format!("BEGIN:VCALENDAR\r\n{}", "BEGIN:X\r\n".repeat(20));
        assert!(parse(deep.as_bytes()).is_none());
        let many = cal(&"X-A:1\r\n".repeat(MAX_PROPS));
        assert!(parse(many.as_bytes()).is_none());
    }

    #[test]
    fn durations_and_end() {
        assert_eq!(duration("PT1H30M"), TimeDelta::try_minutes(90));
        assert_eq!(duration("P1W"), TimeDelta::try_days(7));
        assert_eq!(duration("P1DT2H"), TimeDelta::try_hours(26));
        assert_eq!(duration("-PT1H"), None);
        assert_eq!(duration("P99999999999999999999D"), None);
        let ev = parse(cal("DTSTART:20261008T130000Z\r\nDURATION:PT45M\r\n").as_bytes()).unwrap();
        let end = ev.end.unwrap();
        assert_eq!(end.utc.unwrap() - ev.start.utc.unwrap(), 45 * 60);
    }

    #[test]
    fn offsets() {
        assert_eq!(utc_offset("+0100"), Some(3600));
        assert_eq!(utc_offset("-0530"), Some(-19800));
        assert_eq!(utc_offset("+053015"), Some(19815));
        assert_eq!(utc_offset("0100"), None);
        assert_eq!(utc_offset("+99"), None);
    }

    #[test]
    fn nth_weekdays() {
        let d = |y, m, wd, n| nth_weekday(y, m, wd, n).unwrap().to_string();
        assert_eq!(d(2026, 3, Weekday::Sun, -1), "2026-03-29");
        assert_eq!(d(2026, 10, Weekday::Sun, -1), "2026-10-25");
        assert_eq!(d(2026, 3, Weekday::Sun, 2), "2026-03-08");
        assert_eq!(d(2026, 12, Weekday::Thu, -1), "2026-12-31");
        assert_eq!(nth_weekday(2026, 2, Weekday::Mon, 5), None);
        assert_eq!(nth_weekday(2026, 2, Weekday::Mon, -5), None);
    }

    #[test]
    fn when_and_repeats_in_portuguese() {
        let lisbon = FixedOffset::east_opt(3600).unwrap();
        let ev = parse(
            cal("DTSTART:20261008T130000Z
DTEND:20261008T140000Z
")
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            ev.when_text_in(Lang::PtPt, &lisbon),
            "qui. 08/10/2026 14:00-15:00"
        );
        let floating = parse(
            cal("DTSTART:20261008T130000
")
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            floating.when_text_in(Lang::PtPt, &lisbon),
            "qui. 08/10/2026 13:00 (hora flutuante)"
        );
        assert_eq!(ev.when_text_in(Lang::En, &lisbon), ev.when_text(&lisbon));
        let mut r = ev.clone();
        r.rrule = "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;UNTIL=20261231T235959Z".into();
        assert_eq!(
            r.repeats_in(Lang::PtPt),
            "Repete-se a cada 2 semanas a ter., qui. até 31/12/2026"
        );
        r.rrule = "FREQ=DAILY".into();
        assert_eq!(r.repeats_in(Lang::PtPt), "Repete-se diariamente");
        r.rrule = "FREQ=HOURLY".into();
        assert_eq!(r.repeats_in(Lang::PtPt), "Repete-se");
    }

    #[test]
    fn repeats_summaries() {
        let r = |rule: &str| {
            let mut ev = parse(cal("DTSTART:20261008T130000Z\r\n").as_bytes()).unwrap();
            ev.rrule = rule.into();
            ev.repeats()
        };
        assert_eq!(r(""), "");
        assert_eq!(r("FREQ=DAILY"), "Repeats daily");
        assert_eq!(r("FREQ=WEEKLY;BYDAY=MO,WE"), "Repeats weekly on Mon, Wed");
        assert_eq!(
            r("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20261231T235959Z"),
            "Repeats every 2 weeks on Tue until 2026-12-31"
        );
        assert_eq!(r("FREQ=MONTHLY;COUNT=5"), "Repeats monthly");
        assert_eq!(r("FREQ=YEARLY"), "Repeats yearly");
        assert_eq!(r("FREQ=MONTHLY;BYDAY=1MO"), "Repeats");
        assert_eq!(r("FREQ=MONTHLY;BYSETPOS=-1;BYDAY=FR"), "Repeats");
        assert_eq!(r("FREQ=WEEKLY;BYDAY=1XX"), "Repeats");
        assert_eq!(r("garbage"), "Repeats");
    }

    #[test]
    fn reply_echoes_and_folds() {
        let ev = parse(cal(
            "UID:abc,1@x\r\nSEQUENCE:2\r\nDTSTART:20261008T130000Z\r\nRECURRENCE-ID;TZID=Europe/Lisbon:20261008T140000\r\n\
             ORGANIZER;CN=Jane:mailto:jane@x.org\r\nSUMMARY:Ação de formação - a very long title that goes past seventy five octets\r\n",
        ).as_bytes())
        .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 9, 30, 0).unwrap();
        let ics = reply(&ev, "Zé \"Z\"", "me@x.org", Answer::Tentative, now);
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "{line}");
        }
        assert!(!ics.replace("\r\n", "").contains('\n'));
        for want in [
            "METHOD:REPLY",
            "UID:abc\\,1@x",
            "SEQUENCE:2",
            "DTSTAMP:20261001T093000Z",
            "RECURRENCE-ID;TZID=Europe/Lisbon:20261008T140000",
            "ORGANIZER;CN=\"Jane\":mailto:jane@x.org",
            "ATTENDEE;PARTSTAT=TENTATIVE;CN=\"Zé 'Z'\":mailto:me@x.org",
        ] {
            assert!(ics.contains(want), "{want} in {ics}");
        }
        // It reads back as the same event.
        let back = parse(ics.as_bytes()).unwrap();
        assert_eq!((back.uid.as_str(), back.sequence), ("abc,1@x", 2));
        assert_eq!(back.summary, ev.summary);
        assert_eq!(back.recurrence_id, ev.recurrence_id);
        assert_eq!(back.start, ev.start);
        assert_eq!(back.attendees.len(), 1);
        assert_eq!(back.attendees[0].partstat, "TENTATIVE");
    }

    #[test]
    fn when_text_in_a_fixed_zone() {
        let lisbon_summer = FixedOffset::east_opt(3600).unwrap();
        let ev = parse(cal("DTSTART:20261008T130000Z\r\nDTEND:20261008T140000Z\r\n").as_bytes())
            .unwrap();
        assert_eq!(ev.when_text(&lisbon_summer), "Thu 2026-10-08 14:00-15:00");
        let floating = parse(cal("DTSTART:20261008T130000\r\n").as_bytes()).unwrap();
        assert_eq!(
            floating.when_text(&lisbon_summer),
            "Thu 2026-10-08 13:00 (floating time)"
        );
    }

    const APPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Apple Inc.//macOS 14//EN\r\nMETHOD:REQUEST\r\n\
        BEGIN:VEVENT\r\nUID:r1@x\r\nSEQUENCE:3\r\nDTSTART:20261008T130000Z\r\nDTEND:20261008T140000Z\r\n\
        SUMMARY:Old title\r\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\nX-LONG:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n bbbbbbbbbbbb\r\n\
        ORGANIZER;CN=Jane:mailto:jane@x.org\r\nATTENDEE;CN=Me;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:me@x.org\r\n\
        ATTENDEE;PARTSTAT=ACCEPTED:mailto:bob@x.org\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE:20261015T130000Z,20261022T130000Z\r\n\
        BEGIN:VALARM\r\nX-WR-ALARMUID:abc\r\nACTION:AUDIO\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
        END:VEVENT\r\n\
        BEGIN:VEVENT\r\nUID:r1@x\r\nRECURRENCE-ID:20261029T130000Z\r\nDTSTART:20261029T150000Z\r\nSUMMARY:Moved\r\nEND:VEVENT\r\n\
        END:VCALENDAR\r\n";

    #[test]
    fn resource_with_overrides_dates_and_alarms() {
        let r = parse_resource(APPLE.as_bytes()).unwrap();
        let m = r.master.as_ref().unwrap();
        assert_eq!(r.uid(), "r1@x");
        assert_eq!(m.exdates.len(), 2);
        assert_eq!(
            m.alarms,
            [Alarm {
                before_min: 15,
                related_end: false
            }]
        );
        assert_eq!(r.overrides.len(), 1);
        assert_eq!(r.overrides[0].event.summary, "Moved");
        // `parse` still reads the first event for the card.
        assert_eq!(parse(APPLE.as_bytes()).unwrap().summary, "Old title");
    }

    #[test]
    fn strip_method_and_partstat() {
        let raw = strip_method(APPLE);
        assert!(!raw.contains("METHOD:"));
        assert!(raw.contains("PRODID:-//Apple Inc.//macOS 14//EN\r\n"));
        let raw = set_partstat(&raw, "ME@x.org", "ACCEPTED").unwrap();
        assert!(raw.contains("ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@x.org\r\n"));
        assert!(raw.contains("ATTENDEE;PARTSTAT=ACCEPTED:mailto:bob@x.org\r\n"));
        assert!(set_partstat(&raw, "nobody@x.org", "ACCEPTED").is_none());
        let ev = parse_resource(raw.as_bytes()).unwrap();
        let me = ev.master.unwrap().event;
        assert_eq!(me.attendees[0].partstat, "ACCEPTED");
        assert!(ev.method.is_empty());
    }

    fn edit() -> Edit {
        Edit {
            uid: "new-1@komorebi".into(),
            summary: "Dentist, 2nd floor; bring card".into(),
            location: "Rua Augusta".into(),
            description: "Line 1\nLine 2".into(),
            start: When {
                wall: NaiveDate::from_ymd_opt(2026, 11, 3)
                    .unwrap()
                    .and_hms_opt(9, 30, 0)
                    .unwrap(),
                utc: None,
                all_day: false,
                zone: "Europe/Lisbon".into(),
                tzid: "Europe/Lisbon".into(),
            },
            end: None,
            rrule: "FREQ=MONTHLY;BYDAY=1TU".into(),
            alarm_min: Some(30),
            now: Utc.with_ymd_and_hms(2026, 10, 1, 9, 30, 0).unwrap(),
        }
    }

    #[test]
    fn write_event_round_trips() {
        let ics = write_event(&edit());
        assert!(!ics.contains("METHOD"));
        assert!(ics.contains("PRODID:-//Komorebi//EN\r\n"));
        let r = parse_resource(ics.as_bytes()).unwrap();
        let m = r.master.unwrap();
        assert_eq!(m.event.summary, "Dentist, 2nd floor; bring card");
        assert_eq!(m.event.description, "Line 1\nLine 2");
        assert_eq!(m.event.rrule, "FREQ=MONTHLY;BYDAY=1TU");
        assert_eq!(m.event.start.tzid, "Europe/Lisbon");
        assert_eq!(m.alarms[0].before_min, 30);
    }

    #[test]
    fn patch_keeps_what_it_does_not_know() {
        let mut e = edit();
        e.summary = "New title".into();
        e.alarm_min = None;
        let out = patch(APPLE, &e).unwrap();
        for kept in [
            "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\n",
            "X-LONG:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n bbbbbbbbbbbb\r\n",
            "ATTENDEE;CN=Me;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:me@x.org\r\n",
            "X-WR-ALARMUID:abc\r\n",
            "EXDATE:20261015T130000Z,20261022T130000Z\r\n",
            "RECURRENCE-ID:20261029T130000Z\r\n",
            "SUMMARY:Moved\r\n",
        ] {
            assert!(out.contains(kept), "{kept}");
        }
        assert!(out.contains("SEQUENCE:4\r\n"));
        assert!(!out.contains("Old title"));
        let r = parse_resource(out.as_bytes()).unwrap();
        assert_eq!(r.master.as_ref().unwrap().event.summary, "New title");
        assert_eq!(r.master.unwrap().event.sequence, 4);
        assert_eq!(r.overrides.len(), 1);
        // Our own alarm is replaced, not added twice.
        e.alarm_min = Some(10);
        let once = patch(&write_event(&e), &e).unwrap();
        assert_eq!(once.matches("BEGIN:VALARM").count(), 1);
        assert!(patch("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", &e).is_none());
    }

    #[test]
    fn hostile_resources() {
        assert!(parse_resource(&vec![b'a'; MAX_BYTES + 1]).is_none());
        for cut in (0..APPLE.len()).step_by(37) {
            let _ = parse_resource(APPLE.get(..cut).unwrap_or_default().as_bytes());
            let _ = patch(APPLE.get(..cut).unwrap_or_default(), &edit());
            let _ = strip_method(APPLE.get(..cut).unwrap_or_default());
        }
        let many = format!(
            "BEGIN:VCALENDAR\r\n{}END:VCALENDAR\r\n",
            "BEGIN:VEVENT\r\nUID:a\r\nRECURRENCE-ID:20260101T000000Z\r\nDTSTART:20260101T000000Z\r\nEND:VEVENT\r\n"
                .repeat(MAX_OVERRIDES + 50)
        );
        assert_eq!(
            parse_resource(many.as_bytes()).unwrap().overrides.len(),
            MAX_OVERRIDES + 1
        );
        let dates = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260101T000000Z\r\nEXDATE:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            vec!["20260101T000000Z"; MAX_DATES + 10].join(",")
        );
        assert_eq!(
            parse_resource(dates.as_bytes())
                .unwrap()
                .master
                .unwrap()
                .exdates
                .len(),
            MAX_DATES
        );
    }

    #[test]
    fn spring_gap_through_vtimezone() {
        let ev = parse(
            "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:NY\r\n\
             BEGIN:DAYLIGHT\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nDTSTART:20070311T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nEND:DAYLIGHT\r\n\
             BEGIN:STANDARD\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nDTSTART:20071104T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nEND:STANDARD\r\n\
             END:VTIMEZONE\r\nBEGIN:VEVENT\r\nDTSTART;TZID=NY:20260308T023000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
                .as_bytes(),
        )
        .unwrap();
        let want = Utc
            .with_ymd_and_hms(2026, 3, 8, 7, 30, 0)
            .unwrap()
            .timestamp();
        assert_eq!(ev.start.utc, Some(want));
    }
}
