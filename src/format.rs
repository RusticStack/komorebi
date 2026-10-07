//! Display formatting for dates, sizes, names and avatars.

use chrono::{DateTime, Datelike, Local, TimeZone};

/// Compact list date: time today, weekday this week, day+month this year,
/// full date otherwise.
pub fn list_date(ts: i64, now: DateTime<Local>) -> String {
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
        t.format("%H:%M").to_string()
    } else if (1..7).contains(&days) {
        t.format("%a").to_string()
    } else if t.year() == now.year() {
        t.format("%b %-d").to_string()
    } else {
        t.format("%Y-%m-%d").to_string()
    }
}

pub fn full_date(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(t) if ts != 0 => t.format("%a %Y-%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

pub fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

pub fn sender(name: &str, addr: &str) -> String {
    if name.trim().is_empty() {
        addr.to_string()
    } else {
        name.trim().to_string()
    }
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
        assert_eq!(list_date(ts(2026, 10, 7, 9), now), "09:05");
        assert_eq!(list_date(ts(2026, 10, 5, 9), now), "Mon");
        assert_eq!(list_date(ts(2026, 3, 1, 9), now), "Mar 1");
        assert_eq!(list_date(ts(2024, 3, 1, 9), now), "2024-03-01");
        assert_eq!(list_date(0, now), "");
    }

    #[test]
    fn names() {
        assert_eq!(initials("Jane Doe", "jane@x.com"), "JD");
        assert_eq!(initials("", "jane.doe@x.com"), "J");
        assert_eq!(initials("Ana María de la Cruz", "a@x.com"), "AC");
        assert_eq!(sender("", "a@x.com"), "a@x.com");
        assert_eq!(size(2048), "2 KB");
    }
}
