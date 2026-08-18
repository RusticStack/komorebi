//! SMTP sending via `lettre`.
//!
//! Supports TLS (465) and STARTTLS (587), password login and XOAUTH2 for
//! OAuth providers, and multipart messages with text/html bodies and file
//! attachments. Sends are provider-safe: only the account's own configured
//! SMTP server is used.

use super::types::{Security, ServerSettings};
use anyhow::{Context, Result};
use lettre::Transport;
use lettre::message::header::{ContentDisposition, ContentType};
use lettre::message::{Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::SmtpTransport;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};

pub enum SmtpAuth {
    Password {
        username: String,
        password: String,
    },
    XOAuth2 {
        username: String,
        access_token: String,
    },
}

/// A file attachment to include in an outgoing message.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// File name shown to the recipient (e.g. `report.pdf`).
    pub filename: String,
    /// MIME type (e.g. `application/pdf`). `application/octet-stream` is a
    /// safe fallback for unknown types.
    pub content_type: String,
    /// Raw file bytes.
    pub data: Vec<u8>,
}

pub struct SendRequest {
    pub from_name: Option<String>,
    pub from_email: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<Attachment>,
}

/// Send one message through the account's SMTP server.
pub fn send(
    settings: &ServerSettings,
    tls_domain: &str,
    auth: &SmtpAuth,
    req: &SendRequest,
) -> Result<()> {
    let email = build_message(req)?;

    let (credentials, mechanism) = match auth {
        SmtpAuth::Password { username, password } => (
            Credentials::new(username.clone(), password.clone()),
            Mechanism::Login,
        ),
        SmtpAuth::XOAuth2 {
            username,
            access_token,
        } => (
            Credentials::new(username.clone(), access_token.clone()),
            Mechanism::Xoauth2,
        ),
    };

    let mailer = match settings.security {
        Security::Tls => SmtpTransport::relay(tls_domain).context("building TLS SMTP transport")?,
        Security::StartTls => {
            SmtpTransport::starttls_relay(tls_domain).context("building STARTTLS SMTP transport")?
        }
        Security::None => {
            anyhow::bail!("SMTP requires TLS or STARTTLS; refusing a plaintext send")
        }
    }
    .port(settings.port)
    .credentials(credentials)
    .authentication(vec![mechanism])
    .build();

    mailer.send(&email).context("SMTP send failed")?;
    Ok(())
}

/// Build the full RFC 5322 message (headers + MIME body) for a send request.
///
/// Exposed separately from [`send`] so the message structure can be tested
/// without a live network connection.
pub fn build_message(req: &SendRequest) -> Result<Message> {
    let mut builder = Message::builder()
        .subject(req.subject.clone())
        .from(from_mailbox(req.from_name.as_deref(), &req.from_email)?);
    for to in &req.to {
        builder = builder.to(parse_mailbox(to)?);
    }
    for cc in &req.cc {
        builder = builder.cc(parse_mailbox(cc)?);
    }

    let body_text = req.body_text.trim().to_string();

    let email = if req.attachments.is_empty() {
        match &req.body_html {
            Some(html) => builder
                .multipart(MultiPart::alternative_plain_html(
                    body_text.clone(),
                    html.clone(),
                ))
                .context("building multipart message")?,
            None => builder
                .header(ContentType::TEXT_PLAIN)
                .body(body_text)
                .context("building plain message")?,
        }
    } else {
        // Body part (plain or text/html alternative), then one part per file,
        // all wrapped in multipart/mixed.
        let mut mixed = match &req.body_html {
            Some(html) => MultiPart::mixed().multipart(MultiPart::alternative_plain_html(
                body_text.clone(),
                html.clone(),
            )),
            None => MultiPart::mixed().singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(body_text.clone()),
            ),
        };
        for attachment in &req.attachments {
            let content_type = ContentType::parse(&attachment.content_type)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
            mixed = mixed.singlepart(
                SinglePart::builder()
                    .header(content_type)
                    .header(ContentDisposition::attachment(&attachment.filename))
                    .body(attachment.data.clone()),
            );
        }
        builder.multipart(mixed).context("building mixed message")?
    };

    Ok(email)
}

/// A conservative MIME type for a file name, used when attaching local files.
pub fn guess_content_type(filename: &str) -> &'static str {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("txt") | Some("md") | Some("log") => "text/plain",
        Some("html") | Some("htm") => "text/html",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("zip") => "application/zip",
        Some("gz") => "application/gzip",
        Some("tar") => "application/x-tar",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn from_mailbox(name: Option<&str>, email: &str) -> Result<Mailbox> {
    match name {
        Some(n) if !n.trim().is_empty() => parse_mailbox(&format!("{n} <{email}>")),
        _ => parse_mailbox(email),
    }
}

fn parse_mailbox(s: &str) -> Result<Mailbox> {
    s.parse::<Mailbox>()
        .with_context(|| format!("invalid address: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    fn sample_request() -> SendRequest {
        SendRequest {
            from_name: None,
            from_email: "a@b.c".into(),
            to: vec!["d@e.f".into()],
            cc: vec![],
            subject: "with file".into(),
            body_text: "see attached".into(),
            body_html: None,
            attachments: vec![Attachment {
                filename: "notes.txt".into(),
                content_type: "text/plain".into(),
                data: b"hello attachment".to_vec(),
            }],
        }
    }

    /// Minimal single-connection SMTP server that captures the DATA payload.
    /// Returns the listening port and a receiver for the captured message.
    fn fake_smtp_server() -> (u16, mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut respond = |code: &str| {
                stream.write_all(code.as_bytes()).unwrap();
                stream.flush().unwrap();
            };

            respond("220 fake ESMTP ready\r\n");
            let mut in_data = false;
            let mut data_lines: Vec<String> = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let cmd = line.trim_end_matches(['\r', '\n']);
                if in_data {
                    if cmd == "." {
                        in_data = false;
                        let _ = tx.send(data_lines.join("\r\n"));
                        data_lines.clear();
                        respond("250 OK\r\n");
                    } else {
                        // Undo dot-stuffing so the captured body is faithful.
                        let unstuffed = cmd.strip_prefix("..").unwrap_or(cmd);
                        data_lines.push(unstuffed.to_string());
                    }
                    continue;
                }
                let upper = cmd.to_ascii_uppercase();
                if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    respond("250 fake\r\n");
                } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                    respond("250 OK\r\n");
                } else if upper.starts_with("DATA") {
                    respond("354 End data with <CR><LF>.<CR><LF>\r\n");
                    in_data = true;
                } else if upper.starts_with("RSET") || upper.starts_with("NOOP") {
                    respond("250 OK\r\n");
                } else if upper.starts_with("QUIT") {
                    respond("221 Bye\r\n");
                    break;
                } else {
                    respond("250 OK\r\n");
                }
            }
        });

        (port, rx)
    }

    #[test]
    fn parses_mailboxes() {
        let m = parse_mailbox("Jane Doe <jane@example.com>").unwrap();
        assert_eq!(m.email.to_string(), "jane@example.com");
        assert_eq!(m.name.as_deref(), Some("Jane Doe"));
        let plain = parse_mailbox("me@example.org").unwrap();
        assert_eq!(plain.email.to_string(), "me@example.org");
        assert!(parse_mailbox("not an address").is_err());
    }

    #[test]
    fn refuses_plaintext_smtp() {
        let settings = ServerSettings {
            host: "smtp.example.com".into(),
            port: 25,
            security: Security::None,
        };
        let auth = SmtpAuth::Password {
            username: "u".into(),
            password: "p".into(),
        };
        assert!(send(&settings, "smtp.example.com", &auth, &sample_request()).is_err());
    }

    #[test]
    fn build_message_with_attachment_is_multipart_mixed() {
        let msg = build_message(&sample_request()).unwrap();
        let raw = String::from_utf8_lossy(&msg.formatted()).into_owned();
        assert!(raw.contains("multipart/mixed"), "raw: {raw}");
        assert!(raw.contains("notes.txt"), "raw: {raw}");
        assert!(raw.contains("see attached"), "raw: {raw}");
        assert!(
            raw.contains("hello attachment") || raw.contains("aGVsbG8"),
            "raw: {raw}"
        );
    }

    #[test]
    fn build_message_without_attachments_is_plain_text() {
        let mut req = sample_request();
        req.attachments.clear();
        let msg = build_message(&req).unwrap();
        let raw = String::from_utf8_lossy(&msg.formatted()).into_owned();
        assert!(!raw.contains("multipart/mixed"));
        assert!(raw.contains("see attached"));
    }

    #[test]
    fn fake_server_receives_attachment() {
        let (port, rx) = fake_smtp_server();

        // Non-ASCII bytes force base64 transport encoding, which proves binary
        // file data survives the full send path intact.
        let data: Vec<u8> = vec![0x00, 0x9F, 0x92, 0x96, 0xFF, 0x01, 0xFE, 0x80, 0x7F];
        let mut req = sample_request();
        req.attachments = vec![Attachment {
            filename: "blob.bin".into(),
            content_type: "application/octet-stream".into(),
            data: data.clone(),
        }];
        let msg = build_message(&req).unwrap();

        let mailer = SmtpTransport::builder_dangerous("127.0.0.1")
            .port(port)
            .build();
        mailer.send(&msg).expect("send to fake server");

        let captured = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("server captured data");
        assert!(captured.contains("blob.bin"), "captured: {captured}");
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
        assert!(captured.contains(&b64), "captured: {captured}");
    }

    #[test]
    fn guesses_common_content_types() {
        assert_eq!(guess_content_type("report.pdf"), "application/pdf");
        assert_eq!(guess_content_type("photo.JPG"), "image/jpeg");
        assert_eq!(guess_content_type("notes.txt"), "text/plain");
        assert_eq!(guess_content_type("data.json"), "application/json");
        assert_eq!(
            guess_content_type("unknown.xyz"),
            "application/octet-stream"
        );
    }
}
