//! Display formatting for dates, sizes, names and avatars.

use crate::db::{Outbox, OutboxEntry};
use crate::tr::t;
use chrono::{DateTime, Datelike, Local, TimeZone};
use komorebi::i18n::Lang;

/// Month names abbreviated, as CLDR has them for pt-PT (format context).
const MONTHS_PT: [&str; 12] = [
    "jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.", "dez.",
];
const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// Weekdays abbreviated, Sunday first (`num_days_from_sunday`).
const WEEKDAYS_PT: [&str; 7] = ["dom.", "seg.", "ter.", "qua.", "qui.", "sex.", "sáb."];
const WEEKDAYS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn month(lang: Lang, t: &DateTime<Local>) -> &'static str {
    let i = t.month0() as usize;
    match lang {
        Lang::En => MONTHS_EN[i],
        Lang::PtPt => MONTHS_PT[i],
    }
}

fn weekday(lang: Lang, t: &DateTime<Local>) -> &'static str {
    let i = t.weekday().num_days_from_sunday() as usize;
    match lang {
        Lang::En => WEEKDAYS_EN[i],
        Lang::PtPt => WEEKDAYS_PT[i],
    }
}

/// The 24-hour time, `15:05`, in both languages.
fn clock(t: &DateTime<Local>) -> String {
    t.format("%H:%M").to_string()
}

/// The 24-hour local time of unix seconds `ts` (`14:05`), for "Draft
/// saved 14:05".
pub fn time(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or_else(String::new, |t| clock(&t))
}

const MONTHS_EN_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS_EN_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Windows' short-date picture for English (`M/d/yyyy`, `dd/MM/yyyy`, ...),
/// read once at start ([`load_short_date`]); unset or not understood, the
/// date is ISO 8601.
static SHORT_DATE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Keep `pattern` as the English short date when [`picture`] understands
/// it (the first call wins).
pub fn set_short_date(pattern: &str) -> bool {
    // A plain date: no time zone to load before the first frame.
    let probe = chrono::NaiveDate::from_ymd_opt(2024, 3, 1);
    if probe.and_then(|t| picture(pattern, &t)).is_none() {
        return false;
    }
    SHORT_DATE.set(pattern.to_string()).is_ok()
}

/// Read the user's short-date format from Windows (Region > Regional
/// format), once, before the first frame: one `GetLocaleInfoEx` call.
pub fn load_short_date() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SSHORTDATE};
        let mut buf = [0u16; 80];
        // SAFETY: a null locale name is LOCALE_NAME_USER_DEFAULT; the
        // buffer and its length in UTF-16 units match.
        let n = unsafe {
            GetLocaleInfoEx(
                std::ptr::null(),
                LOCALE_SSHORTDATE,
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        if n > 1 {
            let pattern = String::from_utf16_lossy(&buf[..n as usize - 1]);
            set_short_date(&pattern);
        }
    }
}

/// `t` in a Windows date picture (`d`, `dd`, `ddd`, `dddd`, `M` to `MMMM`,
/// `y`, `yy`, `yyyy`, `'quoted'` text, other characters as they are), with
/// English names; `None` for anything else (an era `g`, a time letter),
/// so the caller falls back to ISO 8601.
pub fn picture(pattern: &str, t: &impl Datelike) -> Option<String> {
    const QUOTE: char = '\x27';
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == QUOTE {
            // 'text', '' for a quote itself.
            let mut j = i + 1;
            if chars.get(j) == Some(&QUOTE) {
                out.push(QUOTE);
                i = j + 1;
                continue;
            }
            while j < chars.len() && chars[j] != QUOTE {
                out.push(chars[j]);
                j += 1;
            }
            i = j + 1;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        match (c, n) {
            ('d', 1) => out.push_str(&t.day().to_string()),
            ('d', 2) => out.push_str(&format!("{:02}", t.day())),
            ('d', 3) => out.push_str(WEEKDAYS_EN[t.weekday().num_days_from_sunday() as usize]),
            ('d', _) => out.push_str(WEEKDAYS_EN_FULL[t.weekday().num_days_from_sunday() as usize]),
            ('M', 1) => out.push_str(&t.month().to_string()),
            ('M', 2) => out.push_str(&format!("{:02}", t.month())),
            ('M', 3) => out.push_str(MONTHS_EN[t.month0() as usize]),
            ('M', _) => out.push_str(MONTHS_EN_FULL[t.month0() as usize]),
            ('y', 1) => out.push_str(&(t.year().rem_euclid(100)).to_string()),
            ('y', 2) => out.push_str(&format!("{:02}", t.year().rem_euclid(100))),
            ('y', 4 | 5) => out.push_str(&format!("{:04}", t.year())),
            (c, _) if c.is_alphabetic() => return None,
            (c, n) => out.extend(std::iter::repeat_n(c, n)),
        }
        i += n;
    }
    (!out.trim().is_empty()).then_some(out)
}

/// An English date with the year: Windows' short date, else ISO 8601.
fn short_date_en(t: &DateTime<Local>) -> String {
    SHORT_DATE
        .get()
        .and_then(|p| picture(p, t))
        .unwrap_or_else(|| t.format("%Y-%m-%d").to_string())
}

/// Compact list date: time today, weekday this week, day+month this year,
/// full date otherwise. English: `09:05`, `Mon`, `Mar 1`, and Windows'
/// short date (`3/1/2024`; ISO `2024-03-01` when it can't be read);
/// pt-PT: `09:05`, `seg.`, `1 mar.`, `01/03/2024`.
pub fn list_date(lang: Lang, ts: i64, now: DateTime<Local>) -> String {
    let Some(t) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    if ts == 0 {
        return String::new();
    }
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    if days == 0 {
        clock(&t)
    } else if (1..7).contains(&days) {
        weekday(lang, &t).to_string()
    } else if t.year() == now.year() {
        match lang {
            Lang::En => format!("{} {}", month(lang, &t), t.day()),
            Lang::PtPt => format!("{} {}", t.day(), month(lang, &t)),
        }
    } else {
        match lang {
            Lang::En => short_date_en(&t),
            Lang::PtPt => t.format("%d/%m/%Y").to_string(),
        }
    }
}

/// The reader's date: as [`full_date`], with Windows' short date in
/// English (`Wed 10/7/2026 15:05`). Quote headers in sent mail keep
/// [`full_date`], so what is sent doesn't depend on this PC's settings.
pub fn reader_date(lang: Lang, ts: i64) -> String {
    match (lang, Local.timestamp_opt(ts, 0).single()) {
        (Lang::En, Some(t)) if ts != 0 => {
            format!("{} {} {}", weekday(lang, &t), short_date_en(&t), clock(&t))
        }
        _ => full_date(lang, ts),
    }
}

/// A date with weekday and time: `Wed 2026-10-07 15:05`, `qua. 07/10/2026
/// 15:05`.
pub fn full_date(lang: Lang, ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(t) if ts != 0 => {
            let day = match lang {
                Lang::En => t.format("%Y-%m-%d"),
                Lang::PtPt => t.format("%d/%m/%Y"),
            };
            format!("{} {day} {}", weekday(lang, &t), clock(&t))
        }
        _ => String::new(),
    }
}

/// Where an outbox message is, at `now` (unix seconds), in one line, in the
/// language shown. A refusal reads as a plain sentence for its SMTP class
/// ([`send_sentence`]); the record itself is shown only behind Details.
pub fn outbox_status(e: &OutboxEntry, now: i64) -> String {
    match e.state {
        Outbox::Pending => t!(invoke_outbox_pending),
        Outbox::Queued => match e.next_try.and_then(|t| Local.timestamp_opt(t, 0).single()) {
            Some(at) if at.timestamp() > now && !e.last_error.is_empty() => t!(
                invoke_outbox_retry,
                send_sentence(&e.last_error, &e.to, false).into(),
                clock(&at).into()
            ),
            _ => t!(invoke_outbox_waiting),
        },
        Outbox::Sending => t!(invoke_sending),
        Outbox::Failed => send_sentence(&e.last_error, &e.to, true),
        Outbox::Unconfirmed => t!(invoke_send_unconfirmed),
    }
}

/// The plain sentence for a send that didn't go through, from its Outbox
/// record (`last_error`) and its To line. `gave_up`: no automatic try
/// follows, so a temporary failure says what to do.
pub fn send_sentence(record: &str, to: &str, gave_up: bool) -> String {
    use komorebi::mail::smtp::{SendClass::*, classify, mailbox_parts, split_addresses};
    let f = classify(record);
    match f.class {
        NoSuchRecipient => {
            let single = || {
                let list = split_addresses(to);
                (list.len() == 1).then(|| mailbox_parts(&list[0]).1.to_string())
            };
            match f.address.or_else(single) {
                Some(a) => t!(invoke_send_no_such, a.into()),
                None => t!(invoke_send_no_such_any),
            }
        }
        BadAddress => t!(invoke_send_bad_address),
        MailboxFull => t!(invoke_send_mailbox_full),
        TooLarge => t!(invoke_send_too_large),
        SenderRefused => t!(invoke_send_sender_refused),
        Policy => t!(invoke_send_policy),
        SignIn => t!(invoke_send_sign_in),
        Certificate => t!(invoke_send_certificate),
        Busy if gave_up => t!(invoke_send_busy_gave_up),
        Busy => t!(invoke_send_busy),
        Unreachable if gave_up => t!(invoke_send_unreachable_gave_up),
        Unreachable => t!(invoke_send_unreachable),
        Other => t!(invoke_send_other),
    }
}

/// A size in bytes: `512 B`, `2 KB`, `1.2 MB`; pt-PT has the decimal comma
/// and a no-break space before the unit (`1,2 MB`).
pub fn size(lang: Lang, bytes: usize) -> String {
    let (n, unit) = match bytes {
        b if b < 1024 => (b.to_string(), "B"),
        b if b < 1024 * 1024 => (format!("{:.0}", b as f64 / 1024.0), "KB"),
        b => (format!("{:.1}", b as f64 / (1024.0 * 1024.0)), "MB"),
    };
    match lang {
        Lang::En => format!("{n} {unit}"),
        Lang::PtPt => format!("{}\u{a0}{unit}", n.replace('.', ",")),
    }
}

/// An attachment chip's icon, from the file name's extension: 0 a file,
/// 1 a picture, 2 a document, 3 a calendar, 4 a message.
pub fn file_kind(name: &str) -> i32 {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic" | "tif" | "tiff" | "svg" => 1,
        "pdf" | "txt" | "md" | "rtf" | "doc" | "docx" | "odt" | "xls" | "xlsx" | "ods" | "csv"
        | "ppt" | "pptx" | "odp" => 2,
        "ics" | "vcs" => 3,
        "eml" | "msg" => 4,
        _ => 0,
    }
}

/// A count with group separators: `103,000`; in pt-PT `103 000` with a
/// no-break space (U+00A0) and, as CLDR has it for pt-PT (minimum grouping
/// digits 2), none for four digits: `1000`, `10 000`.
pub fn thousands(lang: Lang, n: usize) -> String {
    let (sep, min_digits) = match lang {
        Lang::En => (',', 4),
        Lang::PtPt => ('\u{a0}', 5),
    };
    let digits = n.to_string();
    let group = digits.len() >= min_digits;
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if group && i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

pub fn sender(name: &str, addr: &str) -> String {
    if name.trim().is_empty() {
        addr.to_string()
    } else {
        name.trim().to_string()
    }
}

/// A cached To/Cc list (`Name <a@x>, "Doe, Jane" <j@x>, b@x`) as the
/// reader header shows it: display names, or the address where there is
/// none.
pub fn recipients(list: &str) -> String {
    let mut out = String::new();
    for entry in komorebi::mail::smtp::split_addresses(list) {
        let (name, addr) = komorebi::mail::smtp::mailbox_parts(&entry);
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&sender(&name, addr));
    }
    out
}

pub fn initials(name: &str, addr: &str) -> String {
    let source = sender(name, addr);
    let mut words = source
        .split(|c: char| c.is_whitespace() || c == '.' || c == '@' || c == '_')
        .filter(|w| w.chars().next().is_some_and(char::is_alphanumeric));
    let first = words.next().and_then(|w| w.chars().next());
    let second = if name.trim().is_empty() {
        None
    } else {
        words.next_back().and_then(|w| w.chars().next())
    };
    first
        .into_iter()
        .chain(second)
        .flat_map(char::to_uppercase)
        .collect()
}

/// Stable Fluent 2 persona color for an address.
pub fn avatar_color(addr: &str) -> slint::Color {
    const COLORS: &[u32] = &[
        0x0f6cbd, 0xc239b3, 0x13a10e, 0xca5010, 0x8764b8, 0x038387, 0xd13438, 0x4f6bed, 0x986f0b,
        0x00727c, 0xb146c2, 0x5c2e91,
    ];
    let h = addr
        .to_lowercase()
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    slint::Color::from_argb_encoded(0xff00_0000 | COLORS[h as usize % COLORS.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_dates() {
        let now = Local.with_ymd_and_hms(2026, 10, 7, 15, 0, 0).unwrap();
        let ts = |y, m, d, h| {
            Local
                .with_ymd_and_hms(y, m, d, h, 5, 0)
                .unwrap()
                .timestamp()
        };
        let en = |ts| list_date(Lang::En, ts, now);
        assert_eq!(en(ts(2026, 10, 7, 9)), "09:05");
        assert_eq!(en(ts(2026, 10, 5, 9)), "Mon");
        assert_eq!(en(ts(2026, 3, 1, 9)), "Mar 1");
        assert_eq!(en(ts(2024, 3, 1, 9)), "2024-03-01");
        assert_eq!(en(0), "");
    }

    #[test]
    fn list_dates_in_portuguese() {
        let now = Local.with_ymd_and_hms(2026, 10, 7, 15, 0, 0).unwrap();
        let ts = |y, m, d, h| {
            Local
                .with_ymd_and_hms(y, m, d, h, 5, 0)
                .unwrap()
                .timestamp()
        };
        let pt = |ts| list_date(Lang::PtPt, ts, now);
        assert_eq!(pt(ts(2026, 10, 7, 9)), "09:05");
        assert_eq!(pt(ts(2026, 10, 7, 21)), "21:05", "24-hour clock");
        assert_eq!(pt(ts(2026, 10, 5, 9)), "seg.");
        assert_eq!(pt(ts(2026, 10, 1, 9)), "qui.", "6 days back is a weekday");
        assert_eq!(pt(ts(2026, 3, 1, 9)), "1 mar.");
        assert_eq!(pt(ts(2026, 1, 1, 9)), "1 jan.", "start of the year");
        assert_eq!(pt(ts(2026, 9, 30, 9)), "30 set.");
        assert_eq!(pt(ts(2024, 3, 1, 9)), "01/03/2024");
        // Year rollover: Dec 31 of last year, seen on Jan 3.
        let jan = Local.with_ymd_and_hms(2027, 1, 20, 8, 0, 0).unwrap();
        assert_eq!(
            list_date(Lang::PtPt, ts(2026, 12, 31, 9), jan),
            "31/12/2026"
        );
        assert_eq!(list_date(Lang::En, ts(2026, 12, 31, 9), jan), "2026-12-31");
        assert_eq!(pt(0), "");
    }

    #[test]
    fn every_month_and_weekday_has_a_name() {
        let now = Local.with_ymd_and_hms(2026, 12, 31, 12, 0, 0).unwrap();
        let months: Vec<String> = (1..=12)
            .map(|m| {
                let t = Local.with_ymd_and_hms(2026, m, 15, 9, 0, 0).unwrap();
                list_date(Lang::PtPt, t.timestamp(), now)
            })
            .collect();
        assert_eq!(
            months.join(" "),
            "15 jan. 15 fev. 15 mar. 15 abr. 15 mai. 15 jun. 15 jul. 15 ago. 15 set. 15 out. 15 nov. 15 dez."
        );
        // 2026-10-04 is a Sunday.
        let week: Vec<String> = (4..=10)
            .map(|d| {
                let t = Local.with_ymd_and_hms(2026, 10, d, 9, 0, 0).unwrap();
                weekday(Lang::PtPt, &t).to_string()
            })
            .collect();
        assert_eq!(week.join(" "), "dom. seg. ter. qua. qui. sex. sáb.");
    }

    #[test]
    fn windows_short_date_pictures() {
        let t = Local.with_ymd_and_hms(2024, 3, 1, 9, 5, 0).unwrap();
        let p = |pattern| picture(pattern, &t);
        // en-US, en-GB, ISO (sv-SE, en-CA), de-DE, en-IE style, en-ZA.
        assert_eq!(p("M/d/yyyy").as_deref(), Some("3/1/2024"));
        assert_eq!(p("dd/MM/yyyy").as_deref(), Some("01/03/2024"));
        assert_eq!(p("yyyy-MM-dd").as_deref(), Some("2024-03-01"));
        assert_eq!(p("d.M.yyyy").as_deref(), Some("1.3.2024"));
        assert_eq!(p("dd-MMM-yy").as_deref(), Some("01-Mar-24"));
        assert_eq!(p("yyyy/MM/dd").as_deref(), Some("2024/03/01"));
        assert_eq!(p("d/M/y").as_deref(), Some("1/3/24"));
        assert_eq!(p("ddd d MMMM yyyy").as_deref(), Some("Fri 1 March 2024"));
        assert_eq!(p("dddd").as_deref(), Some("Friday"));
        // Quoted text, and '' for a quote.
        assert_eq!(p("d 'de' MMM").as_deref(), Some("1 de Mar"));
        assert_eq!(p("yyyy''MM").as_deref(), Some("2024'03"));
        // An era, a time letter, three y's or an empty picture: not
        // understood, so the caller shows ISO.
        assert_eq!(p("dd/MM/gg"), None);
        assert_eq!(p("yyyy-MM-dd HH"), None);
        assert_eq!(p("yyy"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("'x'").as_deref(), Some("x"));
        // Rejected pictures are not kept (the ISO fallback stays).
        assert!(!set_short_date("gg yyyy"));
        assert!(!set_short_date(""));
    }

    #[test]
    fn full_dates() {
        let ts = Local
            .with_ymd_and_hms(2026, 10, 7, 15, 5, 0)
            .unwrap()
            .timestamp();
        assert_eq!(full_date(Lang::En, ts), "Wed 2026-10-07 15:05");
        assert_eq!(full_date(Lang::PtPt, ts), "qua. 07/10/2026 15:05");
        assert_eq!(full_date(Lang::PtPt, 0), "");
        // The reader's date: pt-PT as above; English with the short date
        // (ISO while no Windows picture is kept, as in tests).
        assert_eq!(reader_date(Lang::PtPt, ts), "qua. 07/10/2026 15:05");
        assert_eq!(reader_date(Lang::En, ts), "Wed 2026-10-07 15:05");
        assert_eq!(reader_date(Lang::En, 0), "");
    }

    #[test]
    fn counts_with_separators() {
        let en = |n| thousands(Lang::En, n);
        assert_eq!(en(0), "0");
        assert_eq!(en(999), "999");
        assert_eq!(en(1_000), "1,000");
        assert_eq!(en(12_400), "12,400");
        assert_eq!(en(103_000), "103,000");
        assert_eq!(en(1_234_567), "1,234,567");
    }

    #[test]
    fn counts_with_separators_in_portuguese() {
        // CLDR pt-PT: no-break space groups, and none under five digits.
        let pt = |n| thousands(Lang::PtPt, n);
        assert_eq!(pt(0), "0");
        assert_eq!(pt(2), "2");
        assert_eq!(pt(999), "999");
        assert_eq!(pt(1_000), "1000");
        assert_eq!(pt(9_999), "9999");
        assert_eq!(pt(10_000), "10\u{a0}000");
        assert_eq!(pt(12_400), "12\u{a0}400");
        assert_eq!(pt(103_000), "103\u{a0}000");
        assert_eq!(pt(1_234_567), "1\u{a0}234\u{a0}567");
    }

    #[test]
    fn sizes() {
        assert_eq!(size(Lang::En, 0), "0 B");
        assert_eq!(size(Lang::En, 1023), "1023 B");
        assert_eq!(size(Lang::En, 2048), "2 KB");
        assert_eq!(size(Lang::En, 1_258_291), "1.2 MB");
        assert_eq!(size(Lang::PtPt, 0), "0\u{a0}B");
        assert_eq!(size(Lang::PtPt, 2048), "2\u{a0}KB");
        assert_eq!(size(Lang::PtPt, 1_572_864), "1,5\u{a0}MB");
        assert_eq!(size(Lang::PtPt, 10 * 1024 * 1024), "10,0\u{a0}MB");
    }

    #[test]
    fn names() {
        assert_eq!(initials("Jane Doe", "jane@x.com"), "JD");
        assert_eq!(initials("", "jane.doe@x.com"), "J");
        assert_eq!(initials("Ana María de la Cruz", "a@x.com"), "AC");
        assert_eq!(sender("", "a@x.com"), "a@x.com");
    }

    #[test]
    fn file_kinds() {
        assert_eq!(file_kind("Photo.JPG"), 1);
        assert_eq!(file_kind("report.pdf"), 2);
        assert_eq!(file_kind("invite.ics"), 3);
        assert_eq!(file_kind("fwd.eml"), 4);
        assert_eq!(file_kind("smime.p7m"), 0);
        assert_eq!(file_kind("README"), 0);
    }

    #[test]
    fn recipient_names() {
        assert_eq!(
            recipients(
                r#"Me <me@x.org>, "Doe, Jane" <j@x.org>, "Al \"Bo\" C" <al@x.org>, bob@x.org"#
            ),
            r#"Me, Doe, Jane, Al "Bo" C, bob@x.org"#
        );
        assert_eq!(recipients(r#""" <a@x.org>, <b@x.org>"#), "a@x.org, b@x.org");
        assert_eq!(recipients(""), "");
    }

    fn entry(state: Outbox, next_try: Option<i64>, error: &str, to: &str) -> OutboxEntry {
        OutboxEntry {
            account_id: 1,
            to: to.into(),
            subject: String::new(),
            state,
            next_try,
            last_error: error.into(),
            updated: 0,
            has_attach: false,
            invite_reply: false,
            body: String::new(),
        }
    }

    const UNKNOWN: &str =
        "permanent error (550): 5.1.1 <bob@x.org>: Recipient address rejected: User unknown";
    const BUSY: &str = "transient error (450): 4.2.0 mailbox busy";

    #[test]
    fn outbox_lines() {
        let _ui = crate::tr::tests::window();
        let now = 1_000_000;
        // A refusal reads as a sentence for its class; the server's words
        // stay behind Details.
        assert_eq!(
            outbox_status(
                &entry(Outbox::Failed, None, UNKNOWN, "a@x.org, bob@x.org"),
                now
            ),
            "bob@x.org doesn't exist. Check the address and send again."
        );
        // No address in the reply: the only recipient, else any of them.
        let bare = "permanent error (550): no such user";
        assert_eq!(
            outbox_status(&entry(Outbox::Failed, None, bare, "Ana <ana@x.org>"), now),
            "ana@x.org doesn't exist. Check the address and send again."
        );
        assert_eq!(
            outbox_status(&entry(Outbox::Failed, None, bare, "a@x.org, b@x.org"), now),
            "One of the addresses doesn't exist. Check the addresses and send again."
        );
        let retry = outbox_status(&entry(Outbox::Queued, Some(now + 60), BUSY, ""), now);
        assert!(
            retry.starts_with("The server couldn't take the message right now. Trying again at "),
            "{retry}"
        );
        // Out of tries: what to do instead.
        assert_eq!(
            outbox_status(&entry(Outbox::Failed, None, BUSY, ""), now),
            "The server couldn't take the message, and Komorebi stopped trying. Select Retry to send it again."
        );
        // Due now (or offline): just waiting.
        assert_eq!(
            outbox_status(&entry(Outbox::Queued, Some(now), BUSY, ""), now),
            "Waiting to send"
        );
        assert_eq!(
            outbox_status(&entry(Outbox::Unconfirmed, None, "may have", ""), now),
            "The connection broke while sending, so it may have gone out. Check Sent before you send it again."
        );
        assert_eq!(
            outbox_status(
                &entry(Outbox::Failed, None, "permanent error (554): no", ""),
                now
            ),
            "The server refused the message. Details says why."
        );
    }

    #[test]
    fn outbox_lines_in_portuguese() {
        let _ui = crate::tr::tests::window();
        assert!(komorebi::i18n::select(Lang::PtPt));
        let now = 1_000_000;
        let at = Local
            .timestamp_opt(now + 60, 0)
            .single()
            .unwrap()
            .format("%H:%M")
            .to_string();
        let lines = [
            outbox_status(&entry(Outbox::Pending, None, "", ""), now),
            outbox_status(&entry(Outbox::Queued, Some(now + 60), BUSY, ""), now),
            outbox_status(&entry(Outbox::Queued, Some(now), BUSY, ""), now),
            outbox_status(&entry(Outbox::Failed, None, UNKNOWN, ""), now),
            outbox_status(&entry(Outbox::Unconfirmed, None, "pode ter sido", ""), now),
        ];
        assert_eq!(lines[0], "A enviar dentro de instantes");
        assert_eq!(
            lines[1],
            format!(
                "O servidor não pôde aceitar a mensagem neste momento. Nova tentativa às {at}."
            )
        );
        assert_eq!(lines[2], "A aguardar o envio");
        assert_eq!(
            lines[3],
            "bob@x.org não existe. Verifique o endereço e envie novamente."
        );
        assert!(
            lines[4].starts_with("A ligação caiu durante o envio"),
            "{}",
            lines[4]
        );
        // No English left in any of them.
        for l in &lines {
            assert!(!l.contains("error") && !l.contains("Recipient"), "{l}");
        }
        komorebi::i18n::select(Lang::En);
    }
}
