//! MIME parsing via `mail-parser`: header rows for the list, and readable
//! bodies plus attachments for the reader.

use super::html::html_to_text;
use crate::db::NewHeader;
use mail_parser::{Address, Message, MessageParser, MessagePart, MimeHeaders, PartType};

/// The header fields fetched during sync. Kept small so a first sync of a
/// large folder is mostly envelope data.
pub const HEADER_FIELDS: &str = "FROM TO CC SUBJECT DATE MESSAGE-ID CONTENT-TYPE";

/// Build a list row from the header bytes returned by
/// `BODY.PEEK[HEADER.FIELDS (...)]`.
pub fn parse_header(raw: &[u8]) -> NewHeader {
    let mut h = NewHeader::default();
    let Some(msg) = MessageParser::default().parse_headers(raw) else {
        return h;
    };
    h.subject = msg.subject().unwrap_or_default().trim().to_string();
    if let Some((name, addr)) = msg.from().and_then(first_addr) {
        h.from_name = name;
        h.from_addr = addr;
    }
    h.to = addresses(msg.to());
    h.cc = addresses(msg.cc());
    h.date = msg.date().map(|d| d.to_timestamp()).unwrap_or(0);
    h.msgid = msg.message_id().unwrap_or_default().to_string();
    h.has_attach = msg.content_type().is_some_and(|ct| {
        ct.ctype().eq_ignore_ascii_case("multipart")
            && ct
                .subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
    });
    h
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentInfo {
    pub index: usize,
    pub name: String,
    pub size: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Body {
    pub text: String,
    pub attachments: Vec<AttachmentInfo>,
    /// Space-separated Message-IDs for the References header of a reply.
    pub references: String,
    /// Reply-To address, if the sender set one.
    pub reply_to: String,
}

/// Parse a full RFC 822 message into readable text and attachment metadata.
/// HTML-only messages are converted to text; nothing remote is ever loaded.
pub fn parse_body(raw: &[u8]) -> Body {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Body {
            text: String::from_utf8_lossy(raw).into_owned(),
            ..Default::default()
        };
    };
    let text = match (msg.text_body.first(), msg.html_body.first()) {
        // Prefer a real text/plain part; mail-parser synthesizes text from
        // HTML when only HTML exists, so check the part type.
        (Some(&i), _) if matches!(msg.parts[i as usize].body, PartType::Text(_)) => {
            tidy(&msg.body_text(0).unwrap_or_default())
        }
        (_, Some(_)) => tidy(&html_to_text(&msg.body_html(0).unwrap_or_default())),
        _ => String::new(),
    };
    let attachments = listed(&msg)
        .enumerate()
        .map(|(index, part)| AttachmentInfo {
            index,
            name: attachment_name(part, index),
            size: part.contents().len(),
        })
        .collect();
    // References for a reply: the parent's References (or, failing that, its
    // In-Reply-To), then the parent's own Message-ID (RFC 5322 3.6.4).
    let mut references = String::new();
    let parents = msg
        .references()
        .as_text_list()
        .or_else(|| msg.in_reply_to().as_text_list());
    for id in parents
        .unwrap_or_default()
        .iter()
        .map(|s| s.as_ref())
        .chain(msg.message_id())
    {
        if !references.is_empty() {
            references.push(' ');
        }
        references.push('<');
        references.push_str(id.trim_matches(['<', '>']));
        references.push('>');
    }
    let reply_to = msg
        .reply_to()
        .and_then(first_addr)
        .map(|(_, addr)| addr)
        .unwrap_or_default();
    Body {
        text,
        attachments,
        references,
        reply_to,
    }
}

/// Attachments shown to the user: everything mail-parser classifies as an
/// attachment except inline images referenced from the HTML body (a
/// Content-ID without a file name), unless explicitly marked as attachment.
/// Every function numbering attachments goes through this filter, so the
/// indices stay consistent.
fn listed<'a>(msg: &'a Message<'a>) -> impl Iterator<Item = &'a MessagePart<'a>> {
    msg.attachments().filter(|part| {
        part.content_id().is_none()
            || part.attachment_name().is_some()
            || part
                .content_disposition()
                .is_some_and(|d| d.ctype().eq_ignore_ascii_case("attachment"))
    })
}

/// File names and bytes of the attachments whose index satisfies `want`,
/// from a single parse of the message.
pub fn attachments(raw: &[u8], want: impl Fn(usize) -> bool) -> Vec<(String, Vec<u8>)> {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    listed(&msg)
        .enumerate()
        .filter(|(i, _)| want(*i))
        .map(|(i, part)| (attachment_name(part, i), part.contents().to_vec()))
        .collect()
}

/// File name and bytes of attachment `index` (as numbered by `parse_body`).
pub fn attachment(raw: &[u8], index: usize) -> Option<(String, Vec<u8>)> {
    attachments(raw, |i| i == index).pop()
}

fn attachment_name(part: &MessagePart, index: usize) -> String {
    let name = part
        .attachment_name()
        .map(sanitize_filename)
        .unwrap_or_default();
    if name.is_empty() {
        format!("attachment-{}", index + 1)
    } else {
        name
    }
}

/// Make an attachment name safe to offer in the save dialog: no path
/// separators or characters Windows refuses, no invisible bidi or zero-width
/// characters that disguise the extension (`a\u{202E}fdp.exe`), no reserved
/// device names, and a sane length that keeps the extension.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut name = cleaned.trim_matches([' ', '.']).to_string();

    let stem = name.split('.').next().unwrap_or_default();
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
    ) || (stem.len() == 4
        && stem
            .get(..3)
            .is_some_and(|p| p.eq_ignore_ascii_case("COM") || p.eq_ignore_ascii_case("LPT"))
        && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        name.insert(0, '_');
    }

    const MAX: usize = 180;
    if name.len() > MAX {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if e.len() <= 16 => (s, e),
            _ => (name.as_str(), ""),
        };
        let keep = stem.floor_char_boundary(MAX - ext.len() - 1);
        name = if ext.is_empty() {
            stem[..keep].to_string()
        } else {
            format!("{}.{ext}", &stem[..keep])
        };
    }
    name
}

/// One-line preview for the message list.
pub fn preview(text: &str) -> String {
    let mut out = String::with_capacity(160);
    for word in text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(str::split_whitespace)
    {
        if out.len() + word.len() > 160 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Normalize line endings (`lines()` handles CRLF), drop leading blank
/// lines and trailing spaces, and squeeze runs of blank lines.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if out.is_empty() || blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(if out.is_empty() {
            line.trim_start()
        } else {
            line
        });
        out.push('\n');
    }
    out.truncate(out.trim_end().len());
    out
}

fn first_addr(address: &Address) -> Option<(String, String)> {
    let addr = match address {
        Address::List(list) => list.first(),
        Address::Group(groups) => groups.first().and_then(|g| g.addresses.first()),
    }?;
    Some((
        addr.name().unwrap_or_default().trim().to_string(),
        addr.address().unwrap_or_default().trim().to_string(),
    ))
}

/// An address header as one RFC 5322 list. Display names with specials are
/// quoted, so `"Doe, Jane" <j@x.org>` survives being split again for
/// reply-all.
fn addresses(address: Option<&Address>) -> String {
    let mut out = String::new();
    for a in address.into_iter().flat_map(|a| a.iter()) {
        let Some(email) = a.address() else { continue };
        if !out.is_empty() {
            out.push_str(", ");
        }
        match a.name().map(str::trim).filter(|n| !n.is_empty()) {
            Some(name)
                if name.contains([
                    ',', ';', '<', '>', '"', '(', ')', '@', ':', '[', ']', '\\', '.',
                ]) =>
            {
                out.push('"');
                for c in name.chars() {
                    if matches!(c, '"' | '\\') {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out.push_str("\" <");
                out.push_str(email);
                out.push('>');
            }
            Some(name) => {
                out.push_str(name);
                out.push_str(" <");
                out.push_str(email);
                out.push('>');
            }
            None => out.push_str(email),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &[u8] = b"From: Jane Doe <jane@example.com>\r\nTo: Me <me@example.org>, bob@example.org\r\nSubject: =?UTF-8?B?SGVsbG8g8J+mgA==?=\r\nDate: Tue, 4 Jun 2026 10:00:00 +0200\r\nMessage-ID: <abc@example.com>\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n";

    const FULL: &[u8] = b"From: Jane <jane@example.com>\r\nTo: me@example.org\r\nSubject: Report\r\nMessage-ID: <m2@example.com>\r\nReferences: <m0@example.com> <m1@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHi,\r\n\r\n\r\n\r\nSee attached.\r\n> quoted\r\n--B\r\nContent-Type: application/pdf; name=\"../../evil.pdf\"\r\nContent-Disposition: attachment; filename=\"../../evil.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--B--\r\n";

    #[test]
    fn header_row() {
        let h = parse_header(HEADER);
        assert_eq!(h.subject, "Hello 🦀");
        assert_eq!(h.from_name, "Jane Doe");
        assert_eq!(h.from_addr, "jane@example.com");
        assert_eq!(h.to, "Me <me@example.org>, bob@example.org");
        assert_eq!(h.msgid, "abc@example.com");
        assert!(h.has_attach);
        assert!(h.date > 0);
    }

    #[test]
    fn body_attachments_and_references() {
        let b = parse_body(FULL);
        assert_eq!(b.text, "Hi,\n\nSee attached.\n> quoted");
        assert_eq!(b.attachments.len(), 1);
        assert_eq!(b.attachments[0].name, "_.._evil.pdf");
        assert_eq!(b.attachments[0].size, 5);
        assert_eq!(
            b.references,
            "<m0@example.com> <m1@example.com> <m2@example.com>"
        );
        let (name, data) = attachment(FULL, 0).unwrap();
        assert_eq!(name, "_.._evil.pdf");
        assert_eq!(data, b"%PDF-");
        assert!(attachment(FULL, 1).is_none());
    }

    #[test]
    fn html_only_body_becomes_text() {
        let raw = b"From: a@b.c\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>there</b></p><p>Second</p><script>x()</script>";
        assert_eq!(parse_body(raw).text, "Hello there\n\nSecond");
    }

    #[test]
    fn preview_skips_quotes_and_truncates() {
        assert_eq!(preview("Hi\n> old stuff\n  there  "), "Hi there");
        assert!(preview(&"word ".repeat(100)).len() <= 160);
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_filename("a/b\\c:d.txt"), "a_b_c_d.txt");
        assert_eq!(sanitize_filename(" .hidden. "), "hidden");
        assert_eq!(sanitize_filename("a\u{202E}fdp.exe"), "a_fdp.exe");
        assert_eq!(sanitize_filename("NUL.txt"), "_NUL.txt");
        assert_eq!(sanitize_filename("com1"), "_com1");
        assert_eq!(sanitize_filename("COM10.txt"), "COM10.txt");
        assert_eq!(sanitize_filename("a€"), "a€");
        let long = format!("{}.pdf", "é".repeat(300));
        let s = sanitize_filename(&long);
        assert!(s.len() <= 180 && s.ends_with(".pdf"), "{s}");
    }

    #[test]
    fn quotes_display_names_with_specials() {
        let raw = b"From: a@b.c\r\nTo: \"Doe, Jane\" <j@x.org>, \"Al \\\"Bo\\\" C\" <al@x.org>, bob@x.org\r\n\r\n";
        let to = parse_header(raw).to;
        assert_eq!(
            to,
            r#""Doe, Jane" <j@x.org>, "Al \"Bo\" C" <al@x.org>, bob@x.org"#
        );
        assert_eq!(crate::mail::smtp::split_addresses(&to).len(), 3);
    }

    #[test]
    fn references_fall_back_to_in_reply_to() {
        let raw = b"From: a@b.c\r\nMessage-ID: <self@x>\r\nIn-Reply-To: <parent@x>\r\n\r\nhi";
        assert_eq!(parse_body(raw).references, "<parent@x> <self@x>");
    }

    #[test]
    fn inline_cid_images_are_not_attachments() {
        let raw = b"From: a@b.c\r\nContent-Type: multipart/related; boundary=R\r\n\r\n--R\r\nContent-Type: text/html\r\n\r\n<p>Hi <img src=\"cid:logo\"></p>\r\n--R\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R\r\nContent-Type: image/png; name=photo.png\r\nContent-ID: <photo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R--\r\n";
        let b = parse_body(raw);
        assert_eq!(b.attachments.len(), 1, "{:?}", b.attachments);
        assert_eq!(b.attachments[0].name, "photo.png");
        assert_eq!(attachment(raw, 0).unwrap().0, "photo.png");
    }

    #[test]
    fn tidy_squeezes_and_trims() {
        assert_eq!(tidy("\r\n\r\n  a  \r\n\r\n\r\nb\r\n\r\n"), "a\n\nb");
    }
}
