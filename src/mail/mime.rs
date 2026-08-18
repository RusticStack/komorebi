//! MIME parsing via `mail-parser`.

use super::types::{ParsedAttachment, ParsedMessage};
use anyhow::{Result, anyhow};
use mail_parser::{Address, MessageParser, MessagePart, MimeHeaders};

/// Parse raw RFC822 bytes into a [`ParsedMessage`].
pub fn parse(raw: &[u8]) -> Result<ParsedMessage> {
    let msg = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| anyhow!("could not parse message"))?;

    let mut out = ParsedMessage {
        subject: msg.subject().map(str::to_string),
        date: msg.date().map(|d| d.to_timestamp()),
        ..Default::default()
    };

    if let Some(from) = msg.from() {
        if let Some(addr) = first_addr(from) {
            out.sender_name = addr.name.as_deref().map(|s| s.to_string());
            out.sender_email = addr.address.as_deref().map(|s| s.to_string());
        }
    }
    out.to = address_list(msg.to());
    out.cc = address_list(msg.cc());

    // Prefer the plain-text body; fall back to HTML stripped of tags.
    if let Some(text) = msg.body_text(0) {
        let snippet = make_snippet(&text);
        out.body_text = Some(text.into_owned());
        out.snippet = Some(snippet);
    } else if let Some(html) = msg.body_html(0) {
        let text = strip_html(&html);
        out.body_html = Some(html.into_owned());
        out.body_text = Some(text.clone());
        out.snippet = Some(make_snippet(&text));
    }

    for i in 0..msg.attachment_count() {
        if let Some(part) = msg.attachment(i as u32) {
            if let Some(body) = part_body(part) {
                out.attachments.push(ParsedAttachment {
                    display_name: part
                        .attachment_name()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("attachment-{i}")),
                    mime: part.content_type().map(|ct| {
                        format!(
                            "{}/{}",
                            ct.c_type,
                            ct.c_subtype.as_deref().unwrap_or_default()
                        )
                    }),
                    data: body,
                });
            }
        }
    }

    Ok(out)
}

fn part_body(part: &MessagePart<'_>) -> Option<Vec<u8>> {
    match &part.body {
        mail_parser::PartType::Binary(bytes) | mail_parser::PartType::InlineBinary(bytes) => {
            Some(bytes.to_vec())
        }
        mail_parser::PartType::Text(text) | mail_parser::PartType::Html(text) => {
            Some(text.as_bytes().to_vec())
        }
        _ => None,
    }
}

fn first_addr<'a>(address: &'a Address<'a>) -> Option<mail_parser::Addr<'a>> {
    match address {
        Address::List(list) => list.first().cloned(),
        Address::Group(groups) => groups.first().and_then(|g| g.addresses.first().cloned()),
    }
}

fn address_list(address: Option<&Address<'_>>) -> Vec<String> {
    let mut out = Vec::new();
    match address {
        Some(Address::List(list)) => {
            for addr in list {
                if let Some(a) = &addr.address {
                    out.push(a.to_string());
                }
            }
        }
        Some(Address::Group(groups)) => {
            for group in groups {
                for addr in &group.addresses {
                    if let Some(a) = &addr.address {
                        out.push(a.to_string());
                    }
                }
            }
        }
        None => {}
    }
    out
}

/// First ~200 characters of text as a one-line snippet.
pub fn make_snippet(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(200).collect();
    if collapsed.chars().count() > 200 {
        out.push('…');
    }
    out
}

/// Minimal HTML → text: strip tags, decode common entities, collapse space.
/// This is *not* a sanitizer - it only produces readable plain text for the
/// native reader (no remote content is ever loaded by the client).
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut skip = 0usize;
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if skip > 0 {
            skip -= 1;
            i += 1;
            continue;
        }
        match c {
            b'<' => {
                let lower = html[i..].to_ascii_lowercase();
                if lower.starts_with("<script") {
                    in_script = true;
                } else if lower.starts_with("</script") {
                    in_script = false;
                } else if lower.starts_with("<br")
                    || lower.starts_with("</p")
                    || lower.starts_with("</div")
                {
                    out.push('\n');
                }
                in_tag = true;
            }
            b'>' => in_tag = false,
            _ if !in_tag && !in_script => {
                if c == b'&' {
                    // decode a handful of common entities
                    let rest = &html[i..];
                    let (entity, len) = decode_entity(rest);
                    out.push_str(&entity);
                    skip = len.saturating_sub(1);
                } else {
                    out.push(c as char);
                }
            }
            _ => {}
        }
        i += 1;
    }
    // collapse runs of whitespace into single spaces
    let mut result = String::with_capacity(out.len());
    let mut last_space = false;
    for ch in out.chars() {
        if ch.is_whitespace() {
            if !last_space {
                result.push(' ');
            }
            last_space = true;
        } else {
            result.push(ch);
            last_space = false;
        }
    }
    result.trim().to_string()
}

fn decode_entity(rest: &str) -> (String, usize) {
    let end = rest.find(';').unwrap_or(0);
    if end == 0 {
        return ("&".into(), 1);
    }
    let entity = &rest[..=end];
    let decoded: String = match entity {
        "&amp;" => "&".into(),
        "&lt;" => "<".into(),
        "&gt;" => ">".into(),
        "&quot;" => "\"".into(),
        "&apos;" => "'".into(),
        "&nbsp;" => " ".into(),
        _ => {
            // numeric entities: &#NN; / &#xHH;
            let inner = &entity[1..entity.len() - 1];
            if let Some(num) = inner
                .strip_prefix("#x")
                .or_else(|| inner.strip_prefix("#X"))
            {
                u32::from_str_radix(num, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
            } else if let Some(num) = inner.strip_prefix('#') {
                num.parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
            } else {
                None
            }
            .unwrap_or_else(|| entity.to_string())
        }
    };
    (decoded, entity.len())
}

/// Safe plain-text rendering of a body for the native reader. Never loads
/// remote content; URLs are shown as text only.
pub fn plain_text_body(text: &str) -> String {
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"From: Jane Doe <jane@example.com>\r\nTo: me@example.org\r\nCc: team@example.org\r\nSubject: =?UTF-8?B?SGVsbG8g8J+mgA==?=\r\nDate: Tue, 4 Jun 2026 10:00:00 +0200\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"BOUND\"\r\n\r\n--BOUND\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHi there! This is the body.\r\n--BOUND\r\nContent-Type: text/plain; name=\"notes.txt\"\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nattachment bytes here\r\n--BOUND--\r\n";

    #[test]
    fn parses_envelope_and_attachments() {
        let parsed = parse(SAMPLE).unwrap();
        assert_eq!(parsed.subject.as_deref(), Some("Hello 🦀"));
        assert_eq!(parsed.sender_name.as_deref(), Some("Jane Doe"));
        assert_eq!(parsed.sender_email.as_deref(), Some("jane@example.com"));
        assert_eq!(parsed.to, vec!["me@example.org"]);
        assert_eq!(parsed.cc, vec!["team@example.org"]);
        assert_eq!(
            parsed.body_text.as_deref(),
            Some("Hi there! This is the body.")
        );
        assert_eq!(
            parsed.snippet.as_deref(),
            Some("Hi there! This is the body.")
        );
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].display_name, "notes.txt");
        assert!(parsed.date.is_some());
    }

    #[test]
    fn strips_html_and_entities() {
        let text = strip_html(
            "<div><p>Hello &amp; <b>world</b></p><br><script>alert(1)</script><p>Done</p></div>",
        );
        assert_eq!(text, "Hello & world Done");
        assert_eq!(strip_html("a &#65; &#x42; b"), "a A B b");
    }

    #[test]
    fn snippet_collapses_and_truncates() {
        let long = (0..50)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let snippet = make_snippet(&long);
        assert!(snippet.chars().count() <= 201);
        assert!(!snippet.contains('\n'));
    }

    #[test]
    fn empty_html_yields_empty_text() {
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn header_value_helpers_compile() {
        use mail_parser::MimeHeaders;
        // Ensure we can read headers and the content type from a parsed message.
        let parsed = MessageParser::default().parse(SAMPLE).unwrap();
        let subject = parsed
            .header_values("Subject")
            .next()
            .map(|h| h.as_text())
            .flatten();
        assert_eq!(subject, Some("Hello 🦀"));
        let ct = parsed.content_type().unwrap();
        assert_eq!(ct.c_type.as_ref(), "multipart");
    }
}
