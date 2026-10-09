//! Display formatting for dates, sizes, names and avatars.

use crate::db::{Outbox, OutboxEntry};
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

/// Where an outbox message is, at `now` (unix seconds), in one line.
pub fn outbox_status(e: &OutboxEntry, now: i64) -> String {
    match e.state {
        Outbox::Pending => "Sending in a few seconds".into(),
        Outbox::Queued => match e.next_try.and_then(|t| Local.timestamp_opt(t, 0).single()) {
            Some(at) if at.timestamp() > now => format!(
                "Not sent yet: {}. Trying again at {}.",
                e.last_error,
                at.format("%H:%M")
            ),
            _ => "Waiting to send".into(),
        },
        Outbox::Sending => "Sending…".into(),
        Outbox::Failed => format!("Not sent: {}", e.last_error),
        Outbox::Unconfirmed => e.last_error.clone(),
    }
}

pub fn size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// A count with thousands separators: `103,000`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
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
        assert_eq!(list_date(ts(2026, 10, 7, 9), now), "09:05");
        assert_eq!(list_date(ts(2026, 10, 5, 9), now), "Mon");
        assert_eq!(list_date(ts(2026, 3, 1, 9), now), "Mar 1");
        assert_eq!(list_date(ts(2024, 3, 1, 9), now), "2024-03-01");
        assert_eq!(list_date(0, now), "");
    }

    #[test]
    fn counts_with_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(12_400), "12,400");
        assert_eq!(thousands(103_000), "103,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn names() {
        assert_eq!(initials("Jane Doe", "jane@x.com"), "JD");
        assert_eq!(initials("", "jane.doe@x.com"), "J");
        assert_eq!(initials("Ana María de la Cruz", "a@x.com"), "AC");
        assert_eq!(sender("", "a@x.com"), "a@x.com");
        assert_eq!(size(2048), "2 KB");
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

    #[test]
    fn outbox_lines() {
        let entry = |state, next_try, error: &str| OutboxEntry {
            account_id: 1,
            to: String::new(),
            subject: String::new(),
            state,
            next_try,
            last_error: error.into(),
            updated: 0,
            has_attach: false,
            invite_reply: false,
        };
        let now = 1_000_000;
        assert_eq!(
            outbox_status(&entry(Outbox::Failed, None, "550 no such user"), now),
            "Not sent: 550 no such user"
        );
        let retry = outbox_status(&entry(Outbox::Queued, Some(now + 60), "450 busy"), now);
        assert!(
            retry.starts_with("Not sent yet: 450 busy. Trying again at "),
            "{retry}"
        );
        // Due now (or offline): just waiting.
        assert_eq!(
            outbox_status(&entry(Outbox::Queued, Some(now), "450 busy"), now),
            "Waiting to send"
        );
        assert_eq!(
            outbox_status(&entry(Outbox::Unconfirmed, None, "may have"), now),
            "may have"
        );
    }
}
