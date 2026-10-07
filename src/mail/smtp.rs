//! Outgoing mail: building RFC 5322 messages and sending them over SMTP.

use crate::providers::{Security, Server};
use anyhow::{Context, Result, bail};
use lettre::Transport;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::SmtpTransport;
use lettre::transport::smtp::authentication::Credentials;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct OutAttachment {
    pub filename: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Outgoing {
    pub from_name: String,
    pub from_email: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<OutAttachment>,
    /// Message-ID being replied to, without angle brackets.
    pub in_reply_to: String,
    /// Full References header value for replies.
    pub references: String,
}

struct Addresses {
    from: Mailbox,
    to: Vec<Mailbox>,
    cc: Vec<Mailbox>,
    bcc: Vec<Mailbox>,
}

fn parse_addresses(out: &Outgoing) -> Result<Addresses> {
    let email = out.from_email.parse().context("invalid sender address")?;
    let name = Some(out.from_name.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let list = |field: &str| -> Result<Vec<Mailbox>> {
        split_addresses(field)
            .iter()
            .map(|a| {
                a.parse()
                    .with_context(|| format!("“{a}” is not a valid address"))
            })
            .collect()
    };
    let parsed = Addresses {
        from: Mailbox::new(name, email),
        to: list(&out.to)?,
        cc: list(&out.cc)?,
        bcc: list(&out.bcc)?,
    };
    if parsed.to.is_empty() && parsed.cc.is_empty() && parsed.bcc.is_empty() {
        bail!("add at least one recipient");
    }
    Ok(parsed)
}

/// Validate sender and recipients without building the message (cheap enough
/// for the UI thread; building base64-encodes every attachment).
pub fn check_recipients(out: &Outgoing) -> Result<()> {
    parse_addresses(out).map(drop)
}

/// A unique Message-ID on the sender's own domain. lettre's default would
/// embed this machine's hostname; this embeds nothing about the machine.
fn message_id(from_email: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros());
    let domain = from_email.rsplit_once('@').map_or("localhost", |(_, d)| d);
    format!(
        "<{micros}.{}.{}@{domain}>",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Build the RFC 5322 message, consuming the attachment buffers.
pub fn build(out: Outgoing) -> Result<Message> {
    let addrs = parse_addresses(&out)?;
    let mut b = Message::builder()
        .message_id(Some(message_id(&out.from_email)))
        .from(addrs.from)
        .subject(out.subject.trim());
    for mb in addrs.to {
        b = b.to(mb);
    }
    for mb in addrs.cc {
        b = b.cc(mb);
    }
    for mb in addrs.bcc {
        b = b.bcc(mb);
    }
    if !out.in_reply_to.is_empty() {
        b = b.in_reply_to(format!("<{}>", out.in_reply_to.trim_matches(['<', '>'])));
    }
    if !out.references.is_empty() {
        b = b.references(out.references);
    }

    // `Body::new` converts a String's line endings to CRLF.
    let text = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(out.body);
    let msg = if out.attachments.is_empty() {
        b.singlepart(text)?
    } else {
        let mut mixed = MultiPart::mixed().singlepart(text);
        for a in out.attachments {
            let ct = ContentType::parse(content_type(&a.filename))
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
            mixed = mixed.singlepart(Attachment::new(a.filename).body(a.data, ct));
        }
        b.multipart(mixed)?
    };
    Ok(msg)
}

pub fn send(server: &Server, username: &str, password: &str, msg: &Message) -> Result<()> {
    let builder = match server.security {
        Security::Tls => SmtpTransport::relay(&server.host)?,
        Security::StartTls => SmtpTransport::starttls_relay(&server.host)?,
    };
    let mailer = builder
        .port(server.port)
        .credentials(Credentials::new(username.into(), password.into()))
        .timeout(Some(Duration::from_secs(60)))
        .build();
    mailer.send(msg).context("sending failed")?;
    Ok(())
}

/// Split an address field on commas/semicolons, ignoring those inside quotes
/// or angle brackets (`"Doe, Jane" <jane@example.com>`).
pub fn split_addresses(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut angle, mut escaped) = (false, false, false);
    for c in list.chars() {
        if escaped {
            escaped = false;
            cur.push(c);
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' if !quoted && !angle => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn content_type(filename: &str) -> &'static str {
    let ext = filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ics" => "text/calendar",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn sample() -> Outgoing {
        Outgoing {
            from_name: "Me".into(),
            from_email: "me@example.com".into(),
            to: "\"Doe, Jane\" <jane@example.com>; bob@example.com".into(),
            cc: String::new(),
            bcc: "secret@example.com".into(),
            subject: "Re: Plans".into(),
            body: "Line 1\nLine 2".into(),
            attachments: vec![],
            in_reply_to: "abc@example.com".into(),
            references: "<abc@example.com>".into(),
        }
    }

    #[test]
    fn splits_address_lists() {
        assert_eq!(
            split_addresses("\"Doe, Jane\" <jane@x.com>, bob@x.com;; carl@x.com,"),
            ["\"Doe, Jane\" <jane@x.com>", "bob@x.com", "carl@x.com"]
        );
        // An escaped quote inside a quoted name does not end the quoting.
        assert_eq!(split_addresses(r#""A \"B, C" <a@x.com>, d@x.com"#).len(), 2);
    }

    #[test]
    fn builds_reply_without_bcc_header() {
        let raw = String::from_utf8(build(sample()).unwrap().formatted()).unwrap();
        assert!(raw.contains("In-Reply-To: <abc@example.com>"), "{raw}");
        assert!(raw.contains("References: <abc@example.com>"), "{raw}");
        assert!(raw.contains("jane@example.com"));
        assert!(
            !raw.contains("secret@example.com"),
            "Bcc must not leak: {raw}"
        );
        assert!(raw.contains("Line 1\r\nLine 2"));
        assert!(raw.contains("Message-ID: <"), "{raw}");
        assert!(
            raw.contains("@example.com>"),
            "Message-ID uses the sender domain: {raw}"
        );
    }

    #[test]
    fn rejects_bad_or_missing_recipients() {
        let mut o = sample();
        o.to = "not an address".into();
        assert!(build(o.clone()).is_err());
        assert!(check_recipients(&o).is_err());
        o.to.clear();
        o.bcc.clear();
        assert!(build(o).unwrap_err().to_string().contains("recipient"));
    }

    #[test]
    fn quoted_display_names_survive_reply_all() {
        // What parse_header stores for `"Doe, Jane"` and `"Al \"Bo\" C"`.
        let raw = b"From: a@b.c\r\nTo: \"Doe, Jane\" <j@x.org>, \"Al \\\"Bo\\\" C\" <al@x.org>, bob@x.org\r\n\r\n";
        let to = crate::mail::mime::parse_header(raw).to;
        let mut o = sample();
        o.to = to.clone();
        o.bcc.clear();
        check_recipients(&o).unwrap();
        // lettre may RFC 2047-encode names with specials; parsing the sent
        // message back must give exactly the same recipients.
        let sent = build(o).unwrap().formatted();
        assert_eq!(crate::mail::mime::parse_header(&sent).to, to);
    }

    #[test]
    fn attachment_reaches_fake_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            s.write_all(b"220 fake\r\n").unwrap();
            let (mut data, mut in_data, mut line) = (String::new(), false, String::new());
            loop {
                line.clear();
                if r.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        s.write_all(b"250 ok\r\n").unwrap();
                    } else {
                        data.push_str(&line);
                    }
                    continue;
                }
                let up = line.to_ascii_uppercase();
                if up.starts_with("DATA") {
                    in_data = true;
                    s.write_all(b"354 go\r\n").unwrap();
                } else if up.starts_with("QUIT") {
                    s.write_all(b"221 bye\r\n").unwrap();
                    break;
                } else {
                    s.write_all(b"250 ok\r\n").unwrap();
                }
            }
            data
        });
        let mut o = sample();
        let bytes = vec![0u8, 159, 146, 150, 255];
        o.attachments = vec![OutAttachment {
            filename: "blob.bin".into(),
            data: bytes.clone(),
        }];
        let msg = build(o).unwrap();
        SmtpTransport::builder_dangerous("127.0.0.1")
            .port(port)
            .build()
            .send(&msg)
            .unwrap();
        let data = server.join().unwrap();
        use base64::Engine as _;
        assert!(data.contains("blob.bin"));
        assert!(data.contains(&base64::engine::general_purpose::STANDARD.encode(&bytes)));
    }
}
