//! A message's source, for "Show original" and "Save as .eml": its header
//! block made readable (unfolded, encoded words decoded, control characters
//! escaped), its body source, and the file name an export is offered under.
//!
//! The input is untrusted, so this module denies the lints below (as
//! `render/` does) and reads at most [`MAX_SHOWN`] bytes of each part.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

use super::mime::sanitize_filename;
use chrono::{Local, TimeZone};
use mail_parser::HeaderValue;
use mail_parser::parsers::MessageStream;

/// The header block and the body source are each shown up to this size.
pub const MAX_SHOWN: usize = 256 * 1024;

/// An exported message's file name is at most this many characters.
const MAX_NAME_CHARS: usize = 120;

/// What "Show original" shows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Original {
    /// One line per header field, unfolded and decoded.
    pub headers: String,
    /// The body as sent (transfer encodings left as they are).
    pub body: String,
}

/// The source of raw message `raw`, readable and safe to show.
pub fn original(raw: &[u8]) -> Original {
    let (head, body) = split(raw);
    Original {
        headers: shown(head, header_lines),
        body: shown(body, |b| escape(&String::from_utf8_lossy(b))),
    }
}

/// `part` read by `read`, cut at [`MAX_SHOWN`] bytes with a line saying
/// how much is left out.
fn shown(part: &[u8], read: impl Fn(&[u8]) -> String) -> String {
    let (kept, rest) = part.split_at(part.len().min(MAX_SHOWN));
    let mut out = read(kept);
    if !rest.is_empty() {
        out.push_str(&format!("\n\n[{} more bytes]", group(rest.len())));
    }
    out
}

/// The header block (up to, without, the first empty line) and the body.
/// A message with no empty line is all headers.
fn split(raw: &[u8]) -> (&[u8], &[u8]) {
    let mut start = 0usize;
    for line in raw.split_inclusive(|&b| b == b'\n') {
        let end = start.saturating_add(line.len());
        if line == b"\r\n" || line == b"\n" {
            return (
                raw.get(..start).unwrap_or_default(),
                raw.get(end..).unwrap_or_default(),
            );
        }
        start = end;
    }
    (raw, &[])
}

/// Header fields one per line: continuation lines joined, encoded words
/// decoded, runs of white space made one space.
fn header_lines(head: &[u8]) -> String {
    let mut out = String::with_capacity(head.len());
    let mut field: Vec<u8> = Vec::new();
    for line in head.split_inclusive(|&b| b == b'\n') {
        if matches!(line.first(), Some(b' ' | b'\t')) && !field.is_empty() {
            field.extend_from_slice(line);
            continue;
        }
        push_field(&mut out, &field);
        field.clear();
        field.extend_from_slice(line);
    }
    push_field(&mut out, &field);
    out
}

fn push_field(out: &mut String, field: &[u8]) {
    if field.iter().all(u8::is_ascii_whitespace) {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    let Some(colon) = field.iter().position(|&b| b == b':') else {
        // Not a header field: shown as it is, on one line.
        let text = String::from_utf8_lossy(field);
        out.push_str(&escape(&one_line(&text)));
        return;
    };
    let (name, value) = field.split_at(colon);
    let value = value.get(1..).unwrap_or_default();
    out.push_str(&escape(String::from_utf8_lossy(name).trim()));
    out.push(':');
    // The parser reads one value up to the end of its last line.
    let mut value = value.to_vec();
    if value.last() != Some(&b'\n') {
        value.push(b'\n');
    }
    let decoded = match MessageStream::new(&value).parse_unstructured() {
        HeaderValue::Text(text) => text.into_owned(),
        _ => String::new(),
    };
    let decoded = one_line(&decoded);
    if !decoded.is_empty() {
        out.push(' ');
        out.push_str(&escape(&decoded));
    }
}

/// `text` with line breaks and runs of white space made single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` with control characters, and the invisible ones that reorder or
/// hide text, shown as `<U+XXXX>`. Line breaks stay (a CR only as part of
/// CRLF), and a tab becomes spaces.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push('\n'),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\t' => out.push_str("    "),
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'..='\u{2029}'
            | '\u{2060}'..='\u{2064}'
            | '\u{061C}'
            | '\u{FEFF}' => out.push_str(&format!("<U+{:04X}>", u32::from(c))),
            c if c.is_control() => out.push_str(&format!("<U+{:04X}>", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// `n` with thousands separators: `1,234,567`.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len().saturating_add(8));
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && digits.len().saturating_sub(i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The file name a message is exported under: `<subject> (<date>).eml`,
/// made safe for Windows ([`sanitize_filename`]) and at most 120
/// characters, the extension kept.
pub fn eml_name(subject: &str, date: i64) -> String {
    let day = Local
        .timestamp_opt(date, 0)
        .single()
        .filter(|_| date != 0)
        .map(|t| t.format(" (%Y-%m-%d)").to_string())
        .unwrap_or_default();
    named(subject, &day)
}

/// The name of a forwarded message's attachment: `<subject>.eml`.
pub fn attachment_name(subject: &str) -> String {
    named(subject, "")
}

/// `<subject><suffix>.eml`, safe and capped at [`MAX_NAME_CHARS`].
fn named(subject: &str, suffix: &str) -> String {
    let stem = one_line(subject);
    let stem = sanitize_filename(&stem);
    let stem = if stem.is_empty() {
        "message"
    } else {
        stem.as_str()
    };
    let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count().saturating_add(4));
    let stem: String = stem.chars().take(room).collect();
    // Cutting can leave a trailing space or dot, which Windows drops.
    format!("{}{suffix}.eml", stem.trim_end_matches([' ', '.']))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_unfolded_decoded_and_escaped() {
        let raw = b"Received: from a.example\r\n\tby b.example; Mon, 1 Jan 2024\r\n\
            Subject: =?UTF-8?B?Sm/Do28=?= and =?ISO-8859-1?Q?caf=E9?=\r\n\
            X-Evil: a\x1b[31mred\xe2\x80\xaeexe\xe2\x80\xa8x\r\n\
            Message-ID: <m1@example.org>\r\n\
            \r\n\
            Body line\r\n\tindented\x07\r\n";
        let o = original(raw);
        assert_eq!(
            o.headers,
            "Received: from a.example by b.example; Mon, 1 Jan 2024\n\
             Subject: João and café\n\
             X-Evil: a<U+001B>[31mred<U+202E>exe x\n\
             Message-ID: <m1@example.org>"
        );
        assert_eq!(o.body, "Body line\n    indented<U+0007>\n");
    }

    #[test]
    fn odd_messages_never_fail() {
        // No body, LF line ends, a line that is no field, an empty value.
        let o = original(b"Subject: hi\nnonsense line\nX-Empty:\n");
        assert_eq!(o.headers, "Subject: hi\nnonsense line\nX-Empty:");
        assert_eq!(o.body, "");
        assert_eq!(original(b""), Original::default());
        assert_eq!(original(b"\r\n\r\nbody").body, "\nbody");
        // Byte noise, including a dangling encoded word and a bare CR.
        let mut seed = 7u32;
        for len in 0..400 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    b"=?Qb:\r\n \t\x00\xff\xc3a"[(seed >> 16) as usize % 13]
                })
                .collect();
            let _ = original(&bytes);
        }
    }

    #[test]
    fn long_parts_are_cut_with_the_bytes_left() {
        let mut raw = b"Subject: big\r\n\r\n".to_vec();
        raw.extend(std::iter::repeat_n(b'a', MAX_SHOWN + 1_234_567));
        let o = original(&raw);
        assert!(o.body.ends_with("\n\n[1,234,567 more bytes]"));
        assert_eq!(o.body.len(), MAX_SHOWN + "\n\n[1,234,567 more bytes]".len());
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
    }

    #[test]
    fn export_names_are_safe_and_short() {
        let date = Local
            .with_ymd_and_hms(2024, 3, 9, 12, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(
            eml_name("Re: Invoice 12/34?", date),
            "Re_ Invoice 12_34_ (2024-03-09).eml"
        );
        assert_eq!(eml_name("", date), "message (2024-03-09).eml");
        assert_eq!(eml_name("  ...  ", 0), "message.eml");
        assert_eq!(eml_name("NUL", 0), "_NUL.eml");
        assert_eq!(eml_name("a\u{202E}fdp.exe", 0), "a_fdp.exe.eml");
        assert_eq!(attachment_name("Lunch\r\n plans"), "Lunch plans.eml");
        let long = eml_name(&"a".repeat(300), date);
        assert_eq!(long.chars().count(), 120);
        assert!(long.ends_with("aa (2024-03-09).eml"));
        let wide = eml_name(&"ã".repeat(300), date);
        assert!(wide.chars().count() <= 120 && wide.ends_with("ã (2024-03-09).eml"));
        let dotted = attachment_name(&format!("{}. x", "a".repeat(114)));
        assert_eq!(dotted, format!("{}.eml", "a".repeat(114)));
    }
}
