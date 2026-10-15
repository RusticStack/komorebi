//! Outgoing mail: building RFC 5322 messages and sending them over SMTP.

use super::errors::{Class, Special, Step, StepError};
use crate::db::Importance;
use crate::providers::{Security, Server};
use anyhow::{Context, Result, anyhow, bail};
use lettre::Transport;
use lettre::address::{Address, Envelope};
use lettre::message::header::{self, ContentTransferEncoding, ContentType, Headers};
use lettre::message::{
    Attachment, Body, Mailbox, Mailboxes, Message, MessageBuilder, MultiPart, SinglePart,
};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::Tls;
use lettre::transport::smtp::extension::ClientId;
use lettre::transport::smtp::{SmtpTransport, SmtpTransportBuilder};
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct OutAttachment {
    pub filename: String,
    pub data: Vec<u8>,
}

/// An image the HTML part shows by `cid:` (a signature's logo), sent in a
/// `multipart/related` with the HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// The Content-ID, without angle brackets or `cid:`.
    pub cid: String,
    /// `image/png` or `image/jpeg`.
    pub mime: String,
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
    /// The text/html alternative of `body` (`compose_html::alternative`).
    /// `None`: the message is plain text only.
    pub body_html: Option<String>,
    /// Images `body_html` shows by `cid:`; ignored without `body_html`.
    pub inline: Vec<InlineImage>,
    pub attachments: Vec<OutAttachment>,
    /// Message-ID being replied to, without angle brackets.
    pub in_reply_to: String,
    /// Full References header value for replies.
    pub references: String,
    /// The draft's Message-ID, angle brackets included, so a resend after
    /// a failed or unconfirmed send reuses it. Empty: a new one is minted.
    pub message_id: String,
    /// An iCalendar alternative of `body` (an invitation's REPLY). It
    /// takes the place of `body_html`.
    pub calendar: Option<CalendarPart>,
    /// High or low: sent as `Importance` and `X-Priority` ([`importance`]).
    pub importance: Importance,
}

/// A `text/calendar` part: its iTIP method and the iCalendar text.
#[derive(Debug, Clone)]
pub struct CalendarPart {
    pub method: &'static str,
    pub ics: String,
}

struct Addresses {
    from: Mailbox,
    to: Vec<Mailbox>,
    cc: Vec<Mailbox>,
    bcc: Vec<Mailbox>,
}

fn parse_addresses(out: &Outgoing) -> Result<Addresses> {
    let parsed = Addresses {
        from: sender(out)?,
        to: mailboxes(&out.to)?,
        cc: mailboxes(&out.cc)?,
        bcc: mailboxes(&out.bcc)?,
    };
    if parsed.to.is_empty() && parsed.cc.is_empty() && parsed.bcc.is_empty() {
        bail!("add at least one recipient");
    }
    Ok(parsed)
}

fn sender(out: &Outgoing) -> Result<Mailbox> {
    let email = out.from_email.parse().context("invalid sender address")?;
    let name = Some(out.from_name.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    Ok(Mailbox::new(name, email))
}

/// The addresses of a field that are valid so far (a draft's may not be).
fn valid_mailboxes(field: &str) -> Vec<Mailbox> {
    split_addresses(field)
        .iter()
        .filter_map(|a| a.parse().ok())
        .collect()
}

fn mailboxes(field: &str) -> Result<Vec<Mailbox>> {
    split_addresses(field)
        .iter()
        .map(|a| {
            a.parse()
                .with_context(|| format!("“{a}” is not a valid address"))
        })
        .collect()
}

/// Validate one To, Cc or Bcc field as typed (an empty field is valid).
pub fn check_addresses(field: &str) -> Result<()> {
    mailboxes(field).map(drop)
}

/// The first address of a To, Cc or Bcc field as typed that isn't valid
/// (none: all are, or the field is empty).
pub fn first_invalid_address(field: &str) -> Option<String> {
    split_addresses(field)
        .into_iter()
        .find(|a| a.parse::<Mailbox>().is_err())
}

/// Validate sender and recipients without building the message (cheap enough
/// for the UI thread; building base64-encodes every attachment).
pub fn check_recipients(out: &Outgoing) -> Result<()> {
    parse_addresses(out).map(drop)
}

/// A unique Message-ID on the sender's own domain. lettre's default would
/// embed this machine's hostname; this embeds nothing about the machine.
pub fn message_id(from_email: &str) -> String {
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

/// The readable part of a message: text only, or text with HTML.
enum Readable {
    Text(SinglePart),
    Alternative(MultiPart),
}

/// Build the RFC 5322 message, consuming the attachment buffers.
pub fn build(out: Outgoing) -> Result<Message> {
    let addrs = parse_addresses(&out)?;
    build_with(out, addrs, Message::builder())
}

/// The bytes of draft `draft_id` saved to the server's Drafts folder: the
/// message as it would be sent, with its Bcc header and an
/// `X-Komorebi-Draft-Id` header. A draft may be unfinished: no recipient
/// is needed, and an address that isn't valid yet is left out.
pub fn build_draft(out: Outgoing, draft_id: i64) -> Result<Vec<u8>> {
    let from = sender(&out)?;
    let addrs = Addresses {
        to: valid_mailboxes(&out.to),
        cc: valid_mailboxes(&out.cc),
        bcc: valid_mailboxes(&out.bcc),
        from,
    };
    // The envelope is never used (a draft isn't sent), but lettre needs one
    // and would derive it from the recipients, which may be none.
    let envelope = Envelope::new(
        Some(addrs.from.email.clone()),
        vec![addrs.from.email.clone()],
    )?;
    let message = build_with(out, addrs, Message::builder().envelope(envelope).keep_bcc())?;
    let mut raw = format!("X-Komorebi-Draft-Id: {draft_id}\r\n").into_bytes();
    raw.extend_from_slice(&message.formatted());
    Ok(raw)
}

fn build_with(out: Outgoing, addrs: Addresses, builder: MessageBuilder) -> Result<Message> {
    let id = if out.message_id.is_empty() {
        message_id(&out.from_email)
    } else {
        out.message_id
    };
    let mut b = builder
        .message_id(Some(id))
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
    for (name, value) in importance(out.importance) {
        b = b.raw_header(header::HeaderValue::new(
            header::HeaderName::new_from_ascii_str(name),
            value.to_string(),
        ));
    }

    // `Body::new` converts a String's line endings to CRLF.
    let text = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(out.body);
    // With HTML: multipart/alternative (text/plain, then the preferred
    // text/html), inside multipart/mixed when there are attachments. An
    // invitation reply: text/plain, then text/calendar.
    let readable = match (out.calendar, out.body_html) {
        (Some(cal), _) => {
            let ct = format!("text/calendar; method={}; charset=utf-8", cal.method);
            Readable::Alternative(
                MultiPart::alternative().singlepart(text).singlepart(
                    SinglePart::builder()
                        .header(ContentType::parse(&ct)?)
                        .body(cal.ics),
                ),
            )
        }
        (None, Some(html)) => {
            let html = SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(html);
            if out.inline.is_empty() {
                Readable::Alternative(MultiPart::alternative().singlepart(text).singlepart(html))
            } else {
                // text/plain, then multipart/related (the HTML and its images).
                let mut related = MultiPart::related().singlepart(html);
                for img in out.inline {
                    let ct = ContentType::parse(&img.mime)?;
                    related =
                        related.singlepart(Attachment::new_inline(img.cid).body(img.data, ct));
                }
                Readable::Alternative(MultiPart::alternative().singlepart(text).multipart(related))
            }
        }
        (None, None) => Readable::Text(text),
    };
    let msg = if out.attachments.is_empty() {
        match readable {
            Readable::Alternative(alt) => b.multipart(alt)?,
            Readable::Text(text) => b.singlepart(text)?,
        }
    } else {
        let mut mixed = match readable {
            Readable::Alternative(alt) => MultiPart::mixed().multipart(alt),
            Readable::Text(text) => MultiPart::mixed().singlepart(text),
        };
        for a in out.attachments {
            let mime = content_type(&a.filename);
            let ct = ContentType::parse(mime)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
            let part = Attachment::new(a.filename);
            mixed = mixed.singlepart(if mime == RFC822 {
                part.body(message_body(a.data), ct)
            } else {
                part.body(a.data, ct)
            });
        }
        b.multipart(mixed)?
    };
    Ok(msg)
}

const RFC822: &str = "message/rfc822";

/// The headers a message's importance is sent as: `Importance` (RFC 2156,
/// what Outlook and most clients read) and `X-Priority` (1 highest, 5
/// lowest, for the rest). A normal message carries neither, as most mail
/// does.
pub fn importance(i: Importance) -> &'static [(&'static str, &'static str)] {
    match i {
        Importance::Normal => &[],
        Importance::High => &[("Importance", "high"), ("X-Priority", "1")],
        Importance::Low => &[("Importance", "low"), ("X-Priority", "5")],
    }
}

/// The body of an attached message (a forward as attachment, or an `.eml`
/// file): its bytes as they are, marked 7bit or 8bit, when they are valid
/// as such (no NUL, CRLF line ends only, lines of at most 998 octets);
/// otherwise base64. RFC 2046 5.2.1 allows only the first two, but a
/// message that isn't valid mail can't be sent as it is, and base64 keeps
/// every byte.
fn message_body(raw: Vec<u8>) -> Body {
    // Octets on the current line (a CR counts only as part of its CRLF).
    let mut line = 0usize;
    let mut prev = 0u8;
    let valid = raw.iter().all(|&b| {
        let ok = match b {
            0 => false,
            b'\n' => prev == b'\r',
            _ => prev != b'\r',
        };
        line = match b {
            b'\n' => 0,
            b'\r' => line,
            _ => line + 1,
        };
        prev = b;
        ok && line <= 998
    }) && prev != b'\r';
    if !valid {
        return Body::new_with_encoding(raw, ContentTransferEncoding::Base64)
            .unwrap_or_else(Body::new);
    }
    let encoding = if raw.is_ascii() {
        ContentTransferEncoding::SevenBit
    } else {
        ContentTransferEncoding::EightBit
    };
    Body::dangerous_pre_encoded(raw, encoding)
}

/// Build the message once, as the exact bytes sent over SMTP (no Bcc
/// header) and saved to Sent ([`sent_copy`]).
pub fn build_raw(out: Outgoing) -> Result<Vec<u8>> {
    Ok(build(out)?.formatted())
}

/// The SMTP envelope for a built message: the sender's address, then every
/// To, Cc and Bcc address (as lettre derives it from the headers).
pub fn envelope(from_email: &str, to: &str, cc: &str, bcc: &str) -> Result<Envelope> {
    let from: Address = from_email.parse().context("invalid sender address")?;
    let mut rcpt = Vec::new();
    for field in [to, cc, bcc] {
        rcpt.extend(mailboxes(field)?.into_iter().map(|m| m.email));
    }
    if rcpt.is_empty() {
        bail!("add at least one recipient");
    }
    Ok(Envelope::new(Some(from), rcpt)?)
}

/// The bytes saved to the Sent folder: the sent bytes `raw` behind the Bcc
/// header that the transmitted copy leaves out, so the sender still sees
/// who got it.
pub fn sent_copy(raw: &[u8], bcc: &str) -> Result<Vec<u8>> {
    let bcc: Mailboxes = mailboxes(bcc)?.into_iter().collect();
    let mut copy = Vec::new();
    if bcc.iter().next().is_some() {
        let mut headers = Headers::new();
        headers.set(header::Bcc::from(bcc));
        // Display encodes and folds the header and ends it with CRLF.
        copy.extend_from_slice(headers.to_string().as_bytes());
    }
    copy.extend_from_slice(raw);
    Ok(copy)
}

/// Settings shared by every connection. EHLO greets with the address literal
/// `[127.0.0.1]` (RFC 5321 4.1.4) instead of this machine's hostname.
fn configure(builder: SmtpTransportBuilder) -> SmtpTransportBuilder {
    builder
        .hello_name(ClientId::Ipv4(Ipv4Addr::LOCALHOST))
        .timeout(Some(Duration::from_secs(60)))
}

pub fn send(server: &Server, username: &str, password: &str, msg: &Message) -> Result<()> {
    send_raw(
        server,
        None,
        username,
        password,
        msg.envelope(),
        &msg.formatted(),
    )
}

/// Send the built bytes `raw` (see [`build_raw`]) to `envelope`.
pub fn send_raw(
    server: &Server,
    pin: Option<&[u8]>,
    username: &str,
    password: &str,
    envelope: &Envelope,
    raw: &[u8],
) -> Result<()> {
    transport(server, pin, username, password)?
        .send_raw(envelope, raw)
        .map_err(|e| pinned_failure(anyhow::Error::new(e), server, pin))
        .context("sending failed")?;
    Ok(())
}

fn transport(
    server: &Server,
    pin: Option<&[u8]>,
    username: &str,
    password: &str,
) -> Result<SmtpTransport> {
    // SSL/TLS wraps the connection; STARTTLS is required, never optional.
    let params = super::pin::smtp_tls_parameters(&server.host, pin)?;
    let tls = match server.security {
        Security::Tls => Tls::Wrapper(params),
        Security::StartTls => Tls::Required(params),
    };
    let builder = SmtpTransport::builder_dangerous(&server.host).tls(tls);
    Ok(configure(builder)
        .port(server.port)
        .credentials(Credentials::new(username.into(), password.into()))
        .build())
}

/// Check that the SMTP server takes this sign-in before an account is
/// saved: connect (TLS or STARTTLS), EHLO, AUTH, NOOP and QUIT. No message
/// is sent. `report` hears each step that passed; a failure names its step.
pub fn verify_smtp(
    server: &Server,
    pin: Option<&[u8]>,
    username: &str,
    password: &str,
    report: &mut dyn FnMut(Step),
) -> Result<(), StepError> {
    let fail = |special, source: anyhow::Error| StepError {
        step: Step::SmtpConnect,
        class: Class::Offline,
        special,
        host: server.host.clone(),
        port: server.port,
        login: username.to_string(),
        source,
    };
    let found = (server.host.as_str(), server.port)
        .to_socket_addrs()
        .map_err(|e| anyhow::Error::new(e).context(format!("resolving {}", server.host)))
        .and_then(|mut a| a.next().map(drop).ok_or_else(|| anyhow!("no address")));
    if let Err(e) = found {
        return Err(fail(Special::SmtpResolve, e));
    }
    let mailer = transport(server, pin, username, password).map_err(|e| StepError {
        class: Class::Other,
        ..fail(Special::None, e)
    })?;
    check_steps(
        &mailer,
        server,
        super::pin::usable(pin).is_some(),
        username,
        report,
    )
}

fn check_steps(
    mailer: &SmtpTransport,
    server: &Server,
    pinned: bool,
    username: &str,
    report: &mut dyn FnMut(Step),
) -> Result<(), StepError> {
    let (step, class, special, source) = match mailer.test_connection() {
        Ok(true) => {
            report(Step::SmtpConnect);
            report(Step::SmtpSignIn);
            return Ok(());
        }
        Ok(false) => (
            Step::SmtpConnect,
            Class::Offline,
            Special::None,
            anyhow!("SMTP sign-in failed: the server stopped answering"),
        ),
        Err(e) => {
            let source = anyhow::Error::new(e).context("SMTP sign-in failed");
            let (step, class, special) =
                step_of(&source).unwrap_or((Step::SmtpConnect, Class::Other, Special::None));
            // The server speaking plain text where TLS was expected.
            let plain = class == Class::TlsCert
                && special == Special::None
                && server.security == Security::Tls
                && super::imap::answers_plain(&server.host, server.port);
            let special = if plain { Special::PlainAnswer } else { special };
            // A pinned server whose certificate no longer validates: it changed.
            if pinned && class == Class::TlsCert && special == Special::None {
                (
                    step,
                    class,
                    Special::CertChanged,
                    anyhow::Error::new(super::pin::PinMismatch(server.host.clone())),
                )
            } else {
                (step, class, special, source)
            }
        }
    };
    if step == Step::SmtpSignIn {
        report(Step::SmtpConnect);
    }
    Err(StepError {
        step,
        class,
        special,
        host: server.host.clone(),
        port: server.port,
        login: username.to_string(),
        source,
    })
}

/// Where a failed SMTP session stopped, how to call it, and whether it has
/// a sentence of its own; `None` if `e` has no SMTP error in it. Only the
/// sign-in refusals (535, 534, 530, 538), a temporary one (454) and a
/// busy server (421) get a class beyond [`Class::Other`] besides TLS and
/// the network.
pub(crate) fn step_of(e: &anyhow::Error) -> Option<(Step, Class, Special)> {
    let se = smtp_error(e)?;
    let detail = std::error::Error::source(se)
        .map(|s| s.to_string())
        .unwrap_or_default();
    let code = se.status().map(|c| c.to_string());
    Some(if se.is_tls() || tls_in_chain(se) {
        (Step::SmtpConnect, Class::TlsCert, Special::None)
    } else if se.is_client() && detail.contains("STARTTLS is not supported") {
        (Step::SmtpConnect, Class::TlsCert, Special::NoStartTls)
    } else if se.is_client() && detail.contains("No compatible authentication") {
        (Step::SmtpSignIn, Class::Auth, Special::NoAuth)
    } else if se.is_permanent() && matches!(code.as_deref(), Some("535" | "534" | "530" | "538")) {
        (Step::SmtpSignIn, Class::Auth, Special::None)
    } else if se.is_transient() && code.as_deref() == Some("454") {
        (Step::SmtpSignIn, Class::TooManyConnections, Special::None)
    } else if se.is_transient() && code.as_deref() == Some("421") {
        (Step::SmtpConnect, Class::TooManyConnections, Special::None)
    } else if se.is_permanent() || se.is_transient() || se.is_client() || se.is_response() {
        (Step::SmtpConnect, Class::Other, Special::None)
    } else {
        // Connection error, network error, a timeout: nothing answered.
        (Step::SmtpConnect, Class::Offline, Special::None)
    })
}

/// Whether a TLS error is behind `e`: on an SSL/TLS port a failed handshake
/// comes back as a connection error that wraps it.
fn tls_in_chain(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(e);
    while let Some(err) = next {
        if err.is::<native_tls::Error>() {
            return true;
        }
        next = err.source();
    }
    false
}

/// [`verify_smtp`]'s check over plain TCP to a local fake server.
#[cfg(test)]
pub(crate) fn verify_plain(port: u16, username: &str, password: &str) -> Result<()> {
    verify_plain_steps(port, username, password, &mut |_| {}).map_err(|e| e.source)
}

/// [`verify_plain`] with the step that failed.
#[cfg(test)]
pub(crate) fn verify_plain_steps(
    port: u16,
    username: &str,
    password: &str,
    report: &mut dyn FnMut(Step),
) -> Result<(), StepError> {
    let server = Server {
        host: "127.0.0.1".into(),
        port,
        // Plain TCP: no TLS port to take for a plain answer.
        security: Security::StartTls,
    };
    check_steps(
        &configure(SmtpTransport::builder_dangerous("127.0.0.1"))
            .port(port)
            .credentials(Credentials::new(username.into(), password.into()))
            .build(),
        &server,
        false,
        username,
        report,
    )
}

/// `e`, a failed send or check, marked as the changed certificate of a pinned
/// server when TLS is what failed.
fn pinned_failure(e: anyhow::Error, server: &Server, pin: Option<&[u8]>) -> anyhow::Error {
    let changed = super::pin::usable(pin).is_some()
        && matches!(step_of(&e), Some((_, Class::TlsCert, Special::None)));
    if changed {
        anyhow::Error::new(super::pin::PinMismatch(server.host.clone()))
    } else {
        e
    }
}

/// Whether a failed [`send`] may still have delivered the message: the
/// connection broke, timed out or gave an unreadable reply once the session
/// was up, possibly after the server took the data. A refusal (4xx/5xx), a
/// failed connect or TLS handshake, or an error before any SMTP (no lettre
/// error at all) means it was not sent.
pub fn may_have_sent(e: &anyhow::Error) -> bool {
    let Some(e) = smtp_error(e) else {
        return false;
    };
    // lettre has no `is_connection()`; its Display names the kind
    // ("Connection error" for a failed connect, "network error" after).
    let connect_failed = e.to_string().starts_with("Connection error");
    !(e.is_permanent() || e.is_transient() || e.is_client() || e.is_tls() || connect_failed)
}

/// Whether a failed [`send`] surely did not deliver and may be tried again
/// later by itself: the connection or TLS handshake failed, or the server
/// answered with a temporary (4xx) refusal, which means it did not take
/// the message. Not for a size refusal, which a retry can't fix.
pub fn retryable(e: &anyhow::Error) -> bool {
    let Some(se) = smtp_error(e) else {
        return false;
    };
    let connect_failed = se.to_string().starts_with("Connection error");
    !too_large(e) && (se.is_transient() || se.is_tls() || connect_failed)
}

/// The server refused the sender: 553 or 550 with a reply that speaks of
/// the sender or the From address ("Sender address rejected: not owned by
/// user"), not of a recipient.
pub fn sender_refused(e: &anyhow::Error) -> bool {
    let Some(se) = smtp_error(e) else {
        return false;
    };
    let code = se.status().map(|c| c.to_string());
    let text = std::error::Error::source(se)
        .map(|s| s.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    se.is_permanent()
        && matches!(code.as_deref(), Some("550" | "553"))
        && (text.contains("sender") || text.contains("not owned") || text.contains("from address"))
        && !text.contains("recipient")
}

/// The server refused the message for its size (552, or 452 for storage).
pub fn too_large(e: &anyhow::Error) -> bool {
    smtp_error(e)
        .and_then(|e| e.status())
        .is_some_and(|code| matches!(code.to_string().as_str(), "552" | "452"))
}

/// What kind of failure an Outbox row's record is, for the plain
/// sentence the Outbox shows (the record itself sits behind Details).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendClass {
    /// 5.1.1 / 5.1.10, or 550 that speaks of an unknown user or recipient.
    NoSuchRecipient,
    /// 5.1.3, or 501/553 about a recipient's syntax.
    BadAddress,
    /// 5.2.2: the recipient's mailbox is full.
    MailboxFull,
    /// 552 or 5.3.4 / 5.2.3: the message is too large.
    TooLarge,
    /// The server refused the sender (553, 5.7.1 about the sender, 5.1.7,
    /// 5.1.8, or the [`super::errors::Say::SenderRefused`] record).
    SenderRefused,
    /// Another 5.7.x: policy, spam or relaying refused.
    Policy,
    /// 530/534/535/538 or 5.7.8, or the rejected sign-in record.
    SignIn,
    /// A temporary refusal (4xx): the server will take it later.
    Busy,
    /// Nothing answered: no connection, a timeout, a name not found.
    Unreachable,
    /// The secure connection failed (an untrusted or changed certificate).
    Certificate,
    /// Anything else, said as it is behind Details.
    Other,
}

/// An Outbox record read back: its class, the SMTP reply code and the
/// enhanced status code if the server gave them, and the address the
/// reply names, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendFailure {
    pub class: SendClass,
    pub code: Option<u16>,
    pub enhanced: Option<String>,
    pub address: Option<String>,
}

impl SendFailure {
    /// A refusal that sending again unchanged can't fix (the user has to
    /// edit the message or the account): Edit is the main action then,
    /// and Retry the main one otherwise.
    pub fn permanent(&self) -> bool {
        !matches!(
            self.class,
            SendClass::Busy | SendClass::Unreachable | SendClass::Other
        )
    }
}

/// Read an Outbox row's `last_error` record (lettre's words kept as they
/// came, `permanent error (550): 5.1.1 <a@b> ...`, or one of the English
/// records of `Say::record`) back into a [`SendFailure`]. Pure; the
/// record is server text, so it is only scanned, never trusted for more.
pub fn classify(record: &str) -> SendFailure {
    let lower = record.to_ascii_lowercase();
    let code = reply_code(&lower);
    let enhanced = enhanced_code(record);
    let address = named_address(record);
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let en = enhanced.as_deref().unwrap_or("");
    let class = if has(&["certificate", "secure connection", "tls error"]) {
        SendClass::Certificate
    } else if has(&["refused to send as"]) {
        SendClass::SenderRefused
    } else if has(&["sign-in rejected"])
        || matches!(code, Some(530 | 534 | 535 | 538))
        || en == "5.7.8"
    {
        SendClass::SignIn
    } else if code == Some(552) || matches!(en, "5.3.4" | "5.2.3") || has(&["too large"]) {
        if en == "5.2.2" {
            SendClass::MailboxFull
        } else {
            SendClass::TooLarge
        }
    } else if en == "5.2.2"
        || (code.is_some_and(|c| c >= 500)
            && has(&["mailbox full", "over quota", "quota exceeded"]))
    {
        SendClass::MailboxFull
    } else if matches!(en, "5.1.1" | "5.1.10" | "5.1.6")
        || (code == Some(550)
            && has(&[
                "no such user",
                "user unknown",
                "unknown user",
                "does not exist",
                "doesn't exist",
                "recipient not found",
                "no mailbox",
                "mailbox unavailable",
                "invalid recipient",
                "recipient rejected",
            ]))
    {
        SendClass::NoSuchRecipient
    } else if matches!(en, "5.1.3" | "5.1.2")
        || (matches!(code, Some(501 | 553)) && has(&["syntax", "invalid address", "bad address"]))
    {
        SendClass::BadAddress
    } else if matches!(en, "5.1.7" | "5.1.8")
        || code == Some(553)
        || (code == Some(550) && has(&["sender", "not owned", "from address"]))
    {
        SendClass::SenderRefused
    } else if en.starts_with("5.7.")
        || (code.is_some_and(|c| c >= 500) && has(&["spam", "policy", "relay", "blocked"]))
    {
        SendClass::Policy
    } else if code.is_some_and(|c| (400..500).contains(&c)) || en.starts_with("4.") {
        SendClass::Busy
    } else if code.is_none()
        && has(&[
            "connection error",
            "network error",
            "timed out",
            "timeout",
            "resolving",
            "connection refused",
            "connection reset",
            "unreachable",
        ])
    {
        SendClass::Unreachable
    } else {
        SendClass::Other
    };
    SendFailure {
        class,
        code,
        enhanced,
        address,
    }
}

/// The 3-digit reply code of `permanent error (550)` / `transient error
/// (421)` in a lowercased record.
fn reply_code(lower: &str) -> Option<u16> {
    for key in ["permanent error (", "transient error ("] {
        if let Some(at) = lower.find(key) {
            let rest = lower.get(at + key.len()..)?;
            let digits = rest.get(..3)?;
            if rest.get(3..4) == Some(")") {
                return digits.parse().ok().filter(|c| (200..600).contains(c));
            }
        }
    }
    None
}

/// The first enhanced status code (RFC 3463, `5.1.1`) in `text`: class 2,
/// 4 or 5, a subject of 1-3 digits and a detail of 1-3, standing alone.
fn enhanced_code(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let digits = |from: usize| {
        b.iter()
            .skip(from)
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let alone = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.');
        if alone && matches!(c, b'2' | b'4' | b'5') && b.get(i + 1) == Some(&b'.') {
            let s = digits(i + 2);
            if (1..=3).contains(&s) && b.get(i + 2 + s) == Some(&b'.') {
                let d = digits(i + 3 + s);
                let end = i + 3 + s + d;
                let after = b.get(end).copied().unwrap_or(b' ');
                if (1..=3).contains(&d) && !after.is_ascii_alphanumeric() && after != b'.' {
                    return text.get(i..end).map(str::to_string);
                }
            }
        }
        i += 1;
    }
    None
}

/// The first `<local@domain>` the reply names (or a bare `local@domain`
/// word), for "{address} doesn't exist".
fn named_address(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let after = rest.get(open + 1..)?;
        let close = after.find('>')?;
        let inner = after.get(..close)?.trim();
        if inner.contains('@') && !inner.contains(char::is_whitespace) {
            return Some(inner.to_string());
        }
        rest = after.get(close..)?;
    }
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '"' | '\''))
        .map(|w| w.trim_end_matches(['.', ':']))
        .find(|w| {
            w.split_once('@')
                .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.'))
        })
        .map(str::to_string)
}

fn smtp_error(e: &anyhow::Error) -> Option<&lettre::transport::smtp::Error> {
    e.chain().find_map(|c| c.downcast_ref())
}

/// [`send_raw`] over plain TCP to a local fake server.
#[cfg(test)]
pub(crate) fn send_plain(port: u16, envelope: &Envelope, raw: &[u8]) -> Result<()> {
    configure(SmtpTransport::builder_dangerous("127.0.0.1"))
        .port(port)
        .build()
        .send_raw(envelope, raw)
        .context("sending failed")?;
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

/// One entry of [`split_addresses`] as (display name, address): `Name
/// <a@x>`, `"Doe, Jane" <j@x>` (unquoted) or a bare `a@x`. Neither part is
/// checked.
pub fn mailbox_parts(entry: &str) -> (String, &str) {
    let (name, addr) = match entry.rsplit_once('<') {
        Some((name, addr)) => (name.trim(), addr.trim_end_matches('>').trim()),
        None => ("", entry.trim()),
    };
    let name = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        Some(quoted) => {
            let mut unescaped = String::with_capacity(quoted.len());
            let mut chars = quoted.chars();
            while let Some(c) = chars.next() {
                unescaped.push(if c == '\\' {
                    chars.next().unwrap_or(c)
                } else {
                    c
                });
            }
            unescaped
        }
        None => name.to_string(),
    };
    (name, addr)
}

/// `Name <a@x>` for an address field, the name quoted when it holds
/// specials, so `"Doe, Jane" <j@x>` survives [`split_addresses`]; the bare
/// address without a name.
pub fn mailbox(name: &str, addr: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return addr.to_string();
    }
    if !name.contains([
        ',', ';', '<', '>', '"', '(', ')', '@', ':', '[', ']', '\\', '.',
    ]) {
        return format!("{name} <{addr}>");
    }
    let mut out = String::with_capacity(name.len() + addr.len() + 6);
    out.push('"');
    for c in name.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push_str("\" <");
    out.push_str(addr);
    out.push('>');
    out
}

/// The MIME type an attachment is sent with, from its file name.
pub fn content_type(filename: &str) -> &'static str {
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
        "eml" => RFC822,
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::fake::{FakeSmtp, SmtpEnd};

    /// A server that offers AUTH but not STARTTLS never sees a password: the
    /// transport requires TLS before it signs in.
    #[test]
    fn auth_offered_without_starttls_gets_no_sign_in() {
        let smtp = FakeSmtp::start(SmtpEnd::Accept);
        let params = crate::mail::pin::smtp_tls_parameters("127.0.0.1", None).unwrap();
        let transport = configure(SmtpTransport::builder_dangerous("127.0.0.1"))
            .tls(Tls::Required(params))
            .port(smtp.port)
            .credentials(Credentials::new("alice".into(), "secret".into()))
            .build();
        assert!(transport.test_connection().is_err());
        assert!(smtp.finish_with_auth().is_empty());
    }

    fn sample() -> Outgoing {
        Outgoing {
            from_name: "Me".into(),
            from_email: "me@example.com".into(),
            to: "\"Doe, Jane\" <jane@example.com>; bob@example.com".into(),
            cc: String::new(),
            bcc: "secret@example.com".into(),
            subject: "Re: Plans".into(),
            body: "Line 1\nLine 2".into(),
            body_html: None,
            inline: vec![],
            attachments: vec![],
            in_reply_to: "abc@example.com".into(),
            references: "<abc@example.com>".into(),
            message_id: String::new(),
            calendar: None,
            importance: Importance::Normal,
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
    fn mailboxes_round_trip() {
        for (name, addr) in [
            ("Ana", "ana@x.pt"),
            ("Doe, Jane", "jane@x.com"),
            (r#"A "B" \ C"#, "a@x.com"),
            ("", "bob@x.com"),
        ] {
            let field = format!("{}, z@x.com", mailbox(name, addr));
            let entries = split_addresses(&field);
            assert_eq!(entries.len(), 2, "{field}");
            assert_eq!(mailbox_parts(&entries[0]), (name.to_string(), addr));
        }
        assert_eq!(mailbox("Doe, Jane", "j@x"), r#""Doe, Jane" <j@x>"#);
        assert_eq!(mailbox_parts("<b@x.org>"), (String::new(), "b@x.org"));
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
    fn sent_copy_folds_and_encodes_long_bcc() {
        let mut o = sample();
        o.bcc.clear();
        let msg = build(o).unwrap();
        assert_eq!(sent_copy(&msg.formatted(), "").unwrap(), msg.formatted());
        let bcc: Vec<String> = (0..12)
            .map(|i| format!("Zoë Number {i} <person{i}@example.com>"))
            .collect();
        let copy =
            String::from_utf8(sent_copy(&msg.formatted(), &bcc.join(", ")).unwrap()).unwrap();
        let lines: Vec<&str> = copy
            .split("\r\n")
            .enumerate()
            .take_while(|(i, l)| *i == 0 || l.starts_with(' '))
            .map(|(_, l)| l)
            .collect();
        assert!(
            lines[0].starts_with("Bcc: ") && lines.len() > 1,
            "{lines:?}"
        );
        assert!(lines.iter().all(|l| l.is_ascii() && l.len() < 998));
        assert_eq!(lines.concat().matches("@example.com").count(), 12);
    }

    #[test]
    fn checks_one_address_field() {
        assert!(check_addresses("").is_ok());
        assert!(check_addresses("João Silva <j@x.pt>").is_ok());
        assert!(check_addresses("\"Doe, Jane\" <jane@x.com>; bob@x.com").is_ok());
        let err = check_addresses("bob@x.com, bob@").unwrap_err();
        assert_eq!(err.to_string(), "“bob@” is not a valid address");
        assert!(check_addresses("João Silva j@x.pt").is_err());
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

    /// Sends `msg` through [`send_plain`] the way [`send`] would.
    fn send_msg(port: u16, msg: &Message) -> Result<()> {
        send_plain(port, msg.envelope(), &msg.formatted())
    }

    /// Sends `sample()` to a fake server that ends as `end`.
    fn send_to_fake(end: SmtpEnd) -> Result<()> {
        let server = FakeSmtp::start(end);
        let result = send_msg(server.port, &build(sample()).unwrap());
        server.finish();
        result
    }

    #[test]
    fn classifies_send_failures() {
        let refused = send_to_fake(SmtpEnd::RejectRcpt).unwrap_err();
        assert!(
            !may_have_sent(&refused) && !too_large(&refused) && !retryable(&refused),
            "{refused:#}"
        );
        // A temporary refusal: not taken, so it may be tried again.
        let busy = send_to_fake(SmtpEnd::TempRejectRcpt).unwrap_err();
        assert!(!may_have_sent(&busy) && retryable(&busy), "{busy:#}");
        let big = send_to_fake(SmtpEnd::TooLarge).unwrap_err();
        assert!(
            !may_have_sent(&big) && too_large(&big) && !retryable(&big),
            "{big:#}"
        );
        // The server may have taken the message before the line dropped.
        let dropped = send_to_fake(SmtpEnd::DropAfterData).unwrap_err();
        assert!(
            may_have_sent(&dropped) && !retryable(&dropped),
            "{dropped:#}"
        );
        // Nothing listens on a just-freed port: the connect fails.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let unreachable = send_msg(port, &build(sample()).unwrap()).unwrap_err();
        assert!(
            !may_have_sent(&unreachable) && retryable(&unreachable),
            "{unreachable:#}"
        );
        // No SMTP at all (e.g. no saved password).
        let no_password = anyhow::anyhow!("no password");
        assert!(!may_have_sent(&no_password) && !retryable(&no_password));
    }

    #[test]
    fn outbox_records_map_to_their_smtp_class() {
        // What the fake server's refusals leave in the row, as stored.
        let record = |end| format!("{:#}", send_to_fake(end).unwrap_err());
        let refused = classify(&record(SmtpEnd::RejectRcpt));
        assert_eq!(
            (refused.class, refused.code),
            (SendClass::NoSuchRecipient, Some(550))
        );
        assert!(refused.permanent());
        let busy = classify(&record(SmtpEnd::TempRejectRcpt));
        assert_eq!(busy.class, SendClass::Busy);
        assert!(!busy.permanent());
        assert_eq!(
            classify(&record(SmtpEnd::TooLarge)).class,
            SendClass::TooLarge
        );
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let unreachable = format!(
            "{:#}",
            send_msg(port, &build(sample()).unwrap()).unwrap_err()
        );
        assert_eq!(classify(&unreachable).class, SendClass::Unreachable);

        // Real-world replies, by their codes first and their words second.
        let cases = [
            (
                "permanent error (550): 5.1.1 <bob@example.org>: Recipient address rejected: User unknown",
                SendClass::NoSuchRecipient,
                Some("5.1.1"),
                Some("bob@example.org"),
            ),
            (
                "permanent error (550): 5.2.2 mailbox full",
                SendClass::MailboxFull,
                Some("5.2.2"),
                None,
            ),
            (
                "permanent error (552): 5.3.4 Message size exceeds fixed limit",
                SendClass::TooLarge,
                Some("5.3.4"),
                None,
            ),
            (
                "permanent error (553): 5.7.1 <me@x.org>: Sender address rejected: not owned by user",
                SendClass::SenderRefused,
                Some("5.7.1"),
                Some("me@x.org"),
            ),
            (
                "permanent error (554): 5.7.1 Message rejected as spam",
                SendClass::Policy,
                Some("5.7.1"),
                None,
            ),
            (
                "permanent error (535): 5.7.8 Authentication credentials invalid",
                SendClass::SignIn,
                Some("5.7.8"),
                None,
            ),
            (
                "permanent error (501): 5.1.3 Bad recipient address syntax",
                SendClass::BadAddress,
                Some("5.1.3"),
                None,
            ),
            (
                "transient error (421): 4.7.0 Try again later",
                SendClass::Busy,
                Some("4.7.0"),
                None,
            ),
            (
                "transient error (452): 4.2.2 Over quota, try later",
                SendClass::Busy,
                Some("4.2.2"),
                None,
            ),
            (
                "Connection error: Connection refused (os error 10061)",
                SendClass::Unreachable,
                None,
                None,
            ),
            (
                "Couldn't make a secure connection to the server. Check the server name in Settings; its certificate may not be trusted.",
                SendClass::Certificate,
                None,
                None,
            ),
            (
                "smtp.example.org refused to send as me@x.org. Check that this account may send from that address, or set the SMTP username.",
                SendClass::SenderRefused,
                None,
                Some("me@x.org"),
            ),
            (
                "Sign-in rejected. Update the password in Settings, or press Refresh to retry.",
                SendClass::SignIn,
                None,
                None,
            ),
            (
                "permanent error (554): Transaction failed",
                SendClass::Other,
                None,
                None,
            ),
            // Version numbers and IPs are not status codes.
            (
                "permanent error (554): server 5.10.2.3 said no in v4.1.2b",
                SendClass::Other,
                None,
                None,
            ),
        ];
        for (record, class, enhanced, address) in cases {
            let f = classify(record);
            assert_eq!(f.class, class, "{record}");
            assert_eq!(f.enhanced.as_deref(), enhanced, "{record}");
            assert_eq!(f.address.as_deref(), address, "{record}");
        }
        assert!(!classify("").permanent());
    }

    #[test]
    fn envelope_matches_the_built_message() {
        let o = sample();
        let env = envelope(&o.from_email, &o.to, &o.cc, &o.bcc).unwrap();
        assert_eq!(&env, build(o.clone()).unwrap().envelope());
        assert_eq!(env.to().len(), 3, "Bcc is in the envelope");
        assert!(envelope(&o.from_email, "", " ", "").is_err());
        assert!(envelope(&o.from_email, "bob@", "", "").is_err());
    }

    #[test]
    fn verify_smtp_signs_in_and_sends_nothing() {
        let server = FakeSmtp::start(SmtpEnd::Accept);
        verify_plain(server.port, "me@example.com", "secret").unwrap();
        let (first, data) = server.finish();
        assert_eq!(first, "EHLO [127.0.0.1]\r\n");
        assert!(data.is_empty(), "{data}");

        let server = FakeSmtp::start(SmtpEnd::RejectAuth);
        let err = verify_plain(server.port, "me@example.com", "wrong").unwrap_err();
        server.finish();
        let text = format!("{err:#}");
        assert!(text.starts_with("SMTP sign-in failed"), "{text}");
        assert!(text.contains("535"), "{text}");
    }

    #[test]
    fn attachment_reaches_fake_server() {
        let server = FakeSmtp::start(SmtpEnd::Accept);
        let mut o = sample();
        let bytes = vec![0u8, 159, 146, 150, 255];
        o.attachments = vec![OutAttachment {
            filename: "blob.bin".into(),
            data: bytes.clone(),
        }];
        let msg = build(o).unwrap();
        send_msg(server.port, &msg).unwrap();
        let (first, data) = server.finish();
        use base64::Engine as _;
        assert_eq!(first, "EHLO [127.0.0.1]\r\n");
        assert!(data.contains("blob.bin"));
        assert!(data.contains(&base64::engine::general_purpose::STANDARD.encode(&bytes)));
        // The transmitted copy hides Bcc; only the saved copy names it.
        assert!(!data.contains("secret@example.com"), "{data}");
        let copy =
            String::from_utf8(sent_copy(&msg.formatted(), "secret@example.com").unwrap()).unwrap();
        assert!(copy.starts_with("Bcc: secret@example.com\r\n"), "{copy}");
        assert!(copy.ends_with(&String::from_utf8(msg.formatted()).unwrap()));
    }

    /// `type/subtype` and charset of a parsed part.
    fn kind(part: &mail_parser::MessagePart) -> (String, Option<String>) {
        use mail_parser::MimeHeaders;
        let ct = part.content_type().unwrap();
        (
            format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or_default()).to_lowercase(),
            ct.attribute("charset").map(str::to_lowercase),
        )
    }

    /// The tree of content types of a built message, depth first.
    fn structure(raw: &[u8]) -> Vec<(String, Option<String>)> {
        let msg = mail_parser::MessageParser::default().parse(raw).unwrap();
        msg.parts.iter().map(kind).collect()
    }

    fn with_html(mut o: Outgoing, attachment: bool) -> Vec<u8> {
        o.body = "**Hi**".into();
        o.body_html = Some("<p><b>Hi</b></p>".into());
        if attachment {
            o.attachments = vec![OutAttachment {
                filename: "notes.txt".into(),
                data: b"notes".to_vec(),
            }];
        }
        build_raw(o).unwrap()
    }

    #[test]
    fn mime_structure_per_case() {
        let utf8 = || Some("utf-8".to_string());
        let t = |s: &str| s.to_string();
        // Text only: one text/plain part, as before.
        assert_eq!(
            structure(&build_raw(sample()).unwrap()),
            [(t("text/plain"), utf8())]
        );
        let alt = with_html(sample(), false);
        assert_eq!(
            structure(&alt),
            [
                (t("multipart/alternative"), None),
                (t("text/plain"), utf8()),
                (t("text/html"), utf8()),
            ]
        );
        let mixed = with_html(sample(), true);
        assert_eq!(
            structure(&mixed),
            [
                (t("multipart/mixed"), None),
                (t("multipart/alternative"), None),
                (t("text/plain"), utf8()),
                (t("text/html"), utf8()),
                (t("text/plain"), None),
            ]
        );
        let msg = mail_parser::MessageParser::default()
            .parse(&mixed[..])
            .unwrap();
        assert_eq!(msg.body_text(0).unwrap(), "**Hi**");
        assert_eq!(msg.body_html(0).unwrap(), "<p><b>Hi</b></p>");
        assert_eq!(msg.attachments().count(), 1);
        // Each multipart has its own boundary.
        let text = String::from_utf8(mixed).unwrap();
        let boundaries: std::collections::HashSet<&str> = text
            .split("boundary=\"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
            .collect();
        assert_eq!(boundaries.len(), 2, "{text}");
        for b in boundaries {
            assert!(text.contains(&format!("\r\n--{b}--")), "{b}");
        }
    }

    fn logo_png() -> InlineImage {
        InlineImage {
            cid: "sig-0123456789abcdef@komorebi".into(),
            mime: "image/png".into(),
            data: b"\x89PNG\r\n\x1a\nlogo".to_vec(),
        }
    }

    #[test]
    fn a_logo_travels_as_a_related_part_of_the_html() {
        let utf8 = || Some("utf-8".to_string());
        let t = |s: &str| s.to_string();
        let build = |attachment: bool| {
            let mut o = sample();
            o.body = "Hi\n\n-- \nAna".into();
            o.body_html = Some(
                "<div>Hi</div><div class=\"sig\"><img src=\"cid:sig-0123456789abcdef@komorebi\"></div>"
                    .into(),
            );
            o.inline = vec![logo_png()];
            if attachment {
                o.attachments = vec![OutAttachment {
                    filename: "notes.txt".into(),
                    data: b"notes".to_vec(),
                }];
            }
            build_raw(o).unwrap()
        };
        let alt = build(false);
        assert_eq!(
            structure(&alt),
            [
                (t("multipart/alternative"), None),
                (t("text/plain"), utf8()),
                (t("multipart/related"), None),
                (t("text/html"), utf8()),
                (t("image/png"), None),
            ]
        );
        let mixed = build(true);
        assert_eq!(
            structure(&mixed),
            [
                (t("multipart/mixed"), None),
                (t("multipart/alternative"), None),
                (t("text/plain"), utf8()),
                (t("multipart/related"), None),
                (t("text/html"), utf8()),
                (t("image/png"), None),
                (t("text/plain"), None),
            ]
        );
        let text = String::from_utf8(alt.clone()).unwrap();
        assert!(
            text.contains("Content-ID: <sig-0123456789abcdef@komorebi>"),
            "{text}"
        );
        assert!(text.contains("Content-Disposition: inline"), "{text}");
        let msg = mail_parser::MessageParser::default()
            .parse(&alt[..])
            .unwrap();
        assert_eq!(msg.body_text(0).unwrap(), "Hi\r\n\r\n-- \r\nAna");
        assert!(
            msg.body_html(0)
                .unwrap()
                .contains("cid:sig-0123456789abcdef@komorebi")
        );
        // The picture is not listed as an attachment of its own.
        use mail_parser::MimeHeaders;
        let image = msg
            .parts
            .iter()
            .find(|p| p.content_type().is_some_and(|c| c.ctype() == "image"))
            .unwrap();
        assert_eq!(image.contents(), &logo_png().data[..]);
        // Without HTML the images are ignored: plain text only.
        let mut o = sample();
        o.inline = vec![logo_png()];
        assert_eq!(
            structure(&build_raw(o).unwrap()),
            [(t("text/plain"), utf8())]
        );
    }

    #[test]
    fn a_rich_message_without_formatting_is_built_exactly_as_typed() {
        let without_date = |raw: Vec<u8>| -> String {
            String::from_utf8(raw)
                .unwrap()
                .split("\r\n")
                .filter(|l| !l.starts_with("Date: "))
                .collect::<Vec<_>>()
                .join("\r\n")
        };
        let mut o = sample();
        o.message_id = "<fixed@example.com>".into();
        o.body = "Hi Ana,\n\n5 * 3 and snake_case, see https://example.com.\n\n-- \nMe".into();
        let before = build_raw(o.clone()).unwrap();
        let doc = crate::mail::rich::Doc::from_plain(&o.body);
        let (body, html) = crate::mail::rich::parts(&doc);
        assert_eq!(html, None);
        assert_eq!(body, o.body);
        let after = build_raw(Outgoing { body, ..o }).unwrap();
        assert_eq!(without_date(after), without_date(before));
    }

    #[test]
    fn unformatted_text_is_built_exactly_as_before() {
        let without_date = |raw: Vec<u8>| -> String {
            String::from_utf8(raw)
                .unwrap()
                .split("\r\n")
                .filter(|l| !l.starts_with("Date: "))
                .collect::<Vec<_>>()
                .join("\r\n")
        };
        let mut o = sample();
        o.message_id = "<fixed@example.com>".into();
        o.body = "Hi Ana,\n\n5 * 3 and snake_case, see https://example.com.\n\n-- \nMe".into();
        let before = build_raw(o.clone()).unwrap();
        let (body, body_html) = crate::mail::compose_html::alternative(&o.body, None);
        assert_eq!(body_html, None);
        let after = build_raw(Outgoing { body, ..o }).unwrap();
        assert_eq!(without_date(after), without_date(before));
    }

    #[test]
    fn importance_is_sent_as_two_headers() {
        let text = |raw: Vec<u8>| String::from_utf8(raw).unwrap();
        let read = |raw: &[u8]| crate::mail::mime::parse_header(raw).importance;
        // Normal: neither header, as most mail.
        let normal = text(build_raw(sample()).unwrap());
        assert!(!normal.contains("Importance:") && !normal.contains("X-Priority:"));
        let mut o = sample();
        o.importance = Importance::High;
        let high = text(build_raw(o.clone()).unwrap());
        assert!(high.contains("\r\nImportance: high\r\n"), "{high}");
        assert!(high.contains("\r\nX-Priority: 1\r\n"), "{high}");
        assert_eq!(read(high.as_bytes()), Importance::High);
        // The server's Drafts copy keeps it too.
        let draft = build_draft(o.clone(), 7).unwrap();
        assert_eq!(read(&draft), Importance::High);
        o.importance = Importance::Low;
        let low = text(build_raw(o.clone()).unwrap());
        assert!(low.contains("\r\nImportance: low\r\n") && low.contains("\r\nX-Priority: 5\r\n"));
        assert_eq!(read(low.as_bytes()), Importance::Low);
        // Once each, in the top-level header only (not in the parts).
        o.attachments = vec![OutAttachment {
            filename: "a.txt".into(),
            data: b"hi".to_vec(),
        }];
        let mixed = text(build_raw(o).unwrap());
        assert_eq!(mixed.matches("Importance:").count(), 1);
        let head = &mixed[..mixed.find("\r\n\r\n").unwrap()];
        assert!(head.contains("Importance: low"));
    }

    #[test]
    fn a_draft_keeps_bcc_and_needs_no_valid_recipient() {
        let text = |raw: Vec<u8>| String::from_utf8(raw).unwrap();
        let mut o = sample();
        o.message_id = "<d1@example.com>".into();
        let draft = text(build_draft(o.clone(), 42).unwrap());
        assert!(draft.contains("X-Komorebi-Draft-Id: 42\r\n"));
        assert!(draft.contains("Bcc: secret@example.com\r\n"));
        assert!(draft.contains("Message-ID: <d1@example.com>\r\n"));
        // The message sent never carries the draft header (nor Bcc).
        let sent = text(build_raw(o.clone()).unwrap());
        assert!(!sent.contains("X-Komorebi") && !sent.contains("Bcc:"));
        // Unfinished: no recipient yet, or one half typed.
        o.to = "jane@".into();
        o.bcc = String::new();
        let draft = text(build_draft(o, 42).unwrap());
        assert!(!draft.contains("\r\nTo:") && draft.contains("Subject: Re: Plans"));
    }

    #[test]
    fn non_ascii_names_and_subjects_round_trip() {
        let mut o = sample();
        o.from_name = "João Silva".into();
        o.to = "Zé Ninguém <ze@example.pt>".into();
        o.bcc.clear();
        o.subject = "Reunião às 15h".into();
        o.body = "Olá, **até já**!".into();
        o.body_html = Some("<p>Olá, <b>até já</b>!</p>".into());
        let raw = build_raw(o).unwrap();
        let head = crate::mail::mime::parse_header(&raw);
        assert_eq!(head.from_name, "João Silva");
        assert_eq!(head.subject, "Reunião às 15h");
        assert_eq!(head.to, "Zé Ninguém <ze@example.pt>");
        let msg = mail_parser::MessageParser::default()
            .parse(&raw[..])
            .unwrap();
        assert_eq!(msg.body_text(0).unwrap(), "Olá, **até já**!");
        assert_eq!(msg.body_html(0).unwrap(), "<p>Olá, <b>até já</b>!</p>");
    }

    #[test]
    fn html_reply_reaches_fake_server_as_built() {
        let server = FakeSmtp::start(SmtpEnd::Accept);
        let raw = with_html(sample(), false);
        let o = sample();
        send_plain(
            server.port,
            &envelope(&o.from_email, &o.to, &o.cc, &o.bcc).unwrap(),
            &raw,
        )
        .unwrap();
        let (_, data) = server.finish();
        // The transport ends the data with one more CRLF before the dot.
        assert_eq!(
            data.trim_end(),
            std::str::from_utf8(&raw).unwrap().trim_end()
        );
        let msg = mail_parser::MessageParser::default()
            .parse(data.as_bytes())
            .unwrap();
        assert_eq!(msg.body_html(0).unwrap(), "<p><b>Hi</b></p>");
    }
    #[test]
    fn forwarded_message_is_attached_intact() {
        use mail_parser::{MimeHeaders, PartType};
        let plain = b"From: Bob <bob@example.org>\r\nSubject: Lunch\r\n\
            Message-ID: <m1@example.org>\r\n\r\nNoon?\r\n"
            .to_vec();
        let mut utf8 = plain.clone();
        utf8.extend_from_slice("Até já\r\n".as_bytes());
        let mut long = plain.clone();
        long.extend(std::iter::repeat_n(b'x', 999));
        let mut bare_lf = plain.clone();
        bare_lf.extend_from_slice(b"one\ntwo\r\n");
        let mut nul = plain.clone();
        nul.extend_from_slice(b"a\0b\r\n");
        let mut edge = plain.clone();
        edge.extend(std::iter::repeat_n(b'x', 998));
        edge.extend_from_slice(b"\r\n");
        for (raw, cte) in [
            (plain, "7bit"),
            (utf8, "8bit"),
            (edge, "7bit"),
            (long, "base64"),
            (bare_lf, "base64"),
            (nul, "base64"),
        ] {
            let mut o = sample();
            o.subject = "Fwd: Lunch".into();
            o.attachments = vec![OutAttachment {
                filename: "Lunch.eml".into(),
                data: raw.clone(),
            }];
            let built = build_raw(o).unwrap();
            let msg = mail_parser::MessageParser::default()
                .parse(&built[..])
                .unwrap();
            let part = msg
                .parts
                .iter()
                .find(|p| matches!(p.body, PartType::Message(_)))
                .expect("an attached message");
            assert_eq!(kind(part).0, "message/rfc822");
            assert_eq!(part.attachment_name(), Some("Lunch.eml"));
            assert_eq!(part.content_transfer_encoding(), Some(cte), "{cte}");
            let PartType::Message(nested) = &part.body else {
                unreachable!()
            };
            assert_eq!(nested.raw_message(), &raw[..], "{cte}");
            assert_eq!(nested.subject(), Some("Lunch"));
            assert_eq!(nested.message_id(), Some("m1@example.org"));
        }
    }
}
