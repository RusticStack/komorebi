//! MIME parsing via `mail-parser`: header rows for the list, readable
//! bodies plus attachments for the reader, and the HTML renderer's input.

use super::html::html_to_text;
use super::ical;
use super::render::{InlinePart, RenderInput};
use crate::db::{Importance, NewHeader};
use mail_parser::{Address, Message, MessageParser, MessagePart, MimeHeaders, PartType};
use std::collections::HashSet;

/// The header fields fetched during sync. Kept small so a first sync of a
/// large folder is mostly envelope data.
/// The three priority fields are a few bytes each, in the same FETCH.
pub const HEADER_FIELDS: &str =
    "FROM TO CC SUBJECT DATE MESSAGE-ID CONTENT-TYPE IMPORTANCE X-PRIORITY X-MSMAIL-PRIORITY";

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
    h.importance = importance(&msg);
    h
}

/// The header fields a draft keeps that the cached row does not: Bcc, and
/// the Message-IDs of In-Reply-To and References (each `<id>`, space
/// separated), as a draft saved elsewhere carries them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DraftFields {
    pub bcc: String,
    pub in_reply_to: String,
    pub references: String,
}

pub fn draft_fields(raw: &[u8]) -> DraftFields {
    let Some(msg) = MessageParser::default().parse_headers(raw) else {
        return DraftFields::default();
    };
    let ids = |v: Option<&[std::borrow::Cow<'_, str>]>| {
        v.unwrap_or_default()
            .iter()
            .map(|id| format!("<{}>", id.trim_matches(['<', '>'])))
            .collect::<Vec<_>>()
            .join(" ")
    };
    DraftFields {
        bcc: addresses(msg.bcc()),
        in_reply_to: ids(msg.in_reply_to().as_text_list()),
        references: ids(msg.references().as_text_list()),
    }
}

/// A priority header's value is read up to this many bytes.
const MAX_PRIORITY_LEN: usize = 64;

/// The importance the sender set: `Importance` (RFC 2156: high, normal,
/// low), else `X-Priority` (its leading digit: 1 and 2 high, 3 normal, 4
/// and 5 low), else `X-MSMail-Priority` (high, normal, low). The first
/// field of each name counts; a value that reads as none of these is
/// normal.
pub fn importance(msg: &Message) -> Importance {
    let value = |field: &str| {
        msg.headers_raw()
            .find(|(name, _)| name.eq_ignore_ascii_case(field))
            .map(|(_, v)| priority_value(v))
    };
    let word = |v: &str| {
        // A trailing comment, as in `high (urgent)`, is ignored.
        let w = v.split(['(', ' ', '\t', ';']).next().unwrap_or_default();
        if w.eq_ignore_ascii_case("high") || w.eq_ignore_ascii_case("urgent") {
            Some(Importance::High)
        } else if w.eq_ignore_ascii_case("low") || w.eq_ignore_ascii_case("non-urgent") {
            Some(Importance::Low)
        } else if w.eq_ignore_ascii_case("normal") {
            Some(Importance::Normal)
        } else {
            None
        }
    };
    if let Some(i) = value("Importance").and_then(|v| word(&v)) {
        return i;
    }
    if let Some(i) = value("X-Priority").and_then(|v| match v.bytes().next() {
        Some(b'1' | b'2') => Some(Importance::High),
        Some(b'3') => Some(Importance::Normal),
        Some(b'4' | b'5') => Some(Importance::Low),
        _ => None,
    }) {
        return i;
    }
    value("X-MSMail-Priority")
        .and_then(|v| word(&v))
        .unwrap_or_default()
}

/// A raw header value, unfolded and trimmed, at most
/// [`MAX_PRIORITY_LEN`] bytes of it read.
fn priority_value(raw: &str) -> String {
    let mut end = raw.len().min(MAX_PRIORITY_LEN);
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw.get(..end)
        .unwrap_or_default()
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
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
    /// The `Authentication-Results` values, topmost first (at most
    /// `trust::MAX_AUTH_HEADERS`, each cut at `trust::MAX_AUTH_LEN`), for
    /// the sender notice (`crate::trust`).
    pub auth: Vec<String>,
    /// The message is encrypted. `text` is then empty, except for inline
    /// PGP, where it is the text around and including the armored block.
    pub encrypted: Option<Encryption>,
    /// Where `List-Unsubscribe` says to unsubscribe: its `mailto:` address
    /// if it has one, else its `https:` one; empty if neither.
    pub unsubscribe: String,
    /// The meeting invitation the message carries ([`calendar_part`]), if
    /// it reads as one.
    pub invite: Option<Box<ical::Event>>,
    /// The importance its sender set ([`importance`]), read from the
    /// headers this parse already has: the reader's mark, and the row's
    /// for mail cached before the column existed.
    pub importance: Importance,
}

/// How an encrypted message is encrypted. Nothing is ever decrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    /// `application/pkcs7-mime` enveloped data.
    Smime,
    /// `multipart/encrypted` (RFC 3156).
    PgpMime,
    /// A `-----BEGIN PGP MESSAGE-----` block in the text.
    PgpInline,
}

impl Encryption {
    pub fn label(self) -> &'static str {
        match self {
            Encryption::Smime => "S/MIME",
            Encryption::PgpMime | Encryption::PgpInline => "PGP",
        }
    }
}

impl Body {
    /// The list preview: a fixed text for encrypted mail (never armor).
    pub fn preview(&self) -> String {
        match self.encrypted {
            Some(_) => "Encrypted message".to_string(),
            None => preview(&self.text),
        }
    }
}

/// Inline PGP is looked for in this many leading bytes of the text.
const PGP_SCAN: usize = 4096;
const PGP_BEGIN: &str = "-----BEGIN PGP MESSAGE-----";
/// The `List-Unsubscribe` value is read up to this many bytes.
const MAX_UNSUBSCRIBE_LEN: usize = 4096;

/// Parse a full RFC 822 message into readable text and attachment metadata.
/// HTML-only messages are converted to text; nothing remote is ever loaded.
pub fn parse_body(raw: &[u8]) -> Body {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Body {
            text: String::from_utf8_lossy(raw).into_owned(),
            ..Default::default()
        };
    };
    let (text, encrypted) = readable(&msg);
    body_of(&msg, text, encrypted)
}

/// Parse a full RFC 822 message into what the HTML renderer needs: the
/// HTML body, every part with a Content-ID (including inline images that
/// [`listed`] hides from the attachment list) and the same text as
/// [`parse_body`] for the fallback view. Nothing remote is loaded.
pub fn parse_render_input(raw: &[u8]) -> RenderInput {
    parse_reader(raw).1
}

/// [`parse_body`] and [`parse_render_input`] from a single parse, for the
/// reader.
pub fn parse_reader(raw: &[u8]) -> (Body, RenderInput) {
    let Some(msg) = MessageParser::default().parse(raw) else {
        let text = String::from_utf8_lossy(raw).into_owned();
        let input = RenderInput {
            text: text.clone(),
            prefers_plain: true,
            ..Default::default()
        };
        let body = Body {
            text,
            ..Default::default()
        };
        return (body, input);
    };
    let (text, encrypted) = readable(&msg);
    let mut input = render_input_of(&msg, text.clone());
    if encrypted.is_some() {
        input.html = None;
        input.prefers_plain = true;
    }
    (body_of(&msg, text, encrypted), input)
}

/// The text the reader shows and how the message is encrypted, if it is:
/// encrypted MIME parts show nothing (their bytes are ciphertext).
fn readable(msg: &Message) -> (String, Option<Encryption>) {
    if let Some(kind) = mime_encryption(msg) {
        return (String::new(), Some(kind));
    }
    let text = body_text(msg);
    let head = text
        .get(..text.floor_char_boundary(PGP_SCAN))
        .unwrap_or_default();
    let inline = head.contains(PGP_BEGIN).then_some(Encryption::PgpInline);
    (text, inline)
}

/// An encrypted message body: the root part is S/MIME enveloped data
/// (`smime-type` enveloped or authEnveloped, or none given: opaque signed
/// data says `signed-data`) or `multipart/encrypted`.
fn mime_encryption(msg: &Message) -> Option<Encryption> {
    let root = msg.parts.first()?;
    if content_type_is(root, "multipart", "encrypted") {
        return Some(Encryption::PgpMime);
    }
    let smime = content_type_is(root, "application", "pkcs7-mime")
        || content_type_is(root, "application", "x-pkcs7-mime");
    let kind = root
        .content_type()
        .and_then(|ct| ct.attribute("smime-type"))
        .map(str::to_ascii_lowercase);
    (smime && kind.is_none_or(|k| k.contains("enveloped"))).then_some(Encryption::Smime)
}

/// An inline PGP block (from its first line to its end line, or to the
/// end of the text) as the file `message.asc`, which the attachment list
/// offers after the message's parts.
fn pgp_block(text: &str) -> Option<(String, Vec<u8>)> {
    const END: &str = "-----END PGP MESSAGE-----";
    let block = text.get(text.find(PGP_BEGIN)?..)?;
    let end = block
        .find(END)
        .map_or(block.len(), |i| i.saturating_add(END.len()));
    let mut out = block.get(..end)?.replace('\n', "\r\n");
    out.push_str("\r\n");
    Some(("message.asc".to_string(), out.into_bytes()))
}

/// The `Authentication-Results` values, topmost first, capped.
fn auth_headers(msg: &Message) -> Vec<String> {
    msg.headers_raw()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Authentication-Results"))
        .take(crate::trust::MAX_AUTH_HEADERS)
        .map(|(_, v)| {
            let cut = v.floor_char_boundary(crate::trust::MAX_AUTH_LEN);
            v.get(..cut).unwrap_or_default().trim().to_string()
        })
        .collect()
}

/// The address to unsubscribe with from a `List-Unsubscribe` value (RFC
/// 2369: URIs in angle brackets, whitespace inside ignored): the first
/// `mailto:` one, else the first `https:` one. Other schemes, plain `http:`
/// among them, are not used.
pub fn unsubscribe_uri(value: &str) -> Option<String> {
    let value = value
        .get(..value.floor_char_boundary(MAX_UNSUBSCRIBE_LEN))
        .unwrap_or_default();
    let uris: Vec<String> = value
        .split('<')
        .skip(1)
        .filter_map(|s| s.split_once('>'))
        .map(|(uri, _)| uri.chars().filter(|c| !c.is_whitespace()).collect())
        .collect();
    let first = |scheme: &str| {
        uris.iter()
            .filter(|u| {
                u.get(..scheme.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
            })
            .find_map(|u| super::render::links::sanitize(u))
    };
    first("mailto:").or_else(|| first("https:"))
}

fn body_of(msg: &Message, text: String, encrypted: Option<Encryption>) -> Body {
    let mut attachments: Vec<AttachmentInfo> = listed(msg)
        .enumerate()
        .map(|(index, part)| AttachmentInfo {
            index,
            name: attachment_name(part, index),
            size: part.contents().len(),
        })
        .collect();
    if encrypted == Some(Encryption::PgpInline)
        && let Some((name, data)) = pgp_block(&text)
    {
        attachments.push(AttachmentInfo {
            index: attachments.len(),
            name,
            size: data.len(),
        });
    }
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
    let unsubscribe = msg
        .headers_raw()
        .find(|(name, _)| name.eq_ignore_ascii_case("List-Unsubscribe"))
        .and_then(|(_, v)| unsubscribe_uri(v))
        .unwrap_or_default();
    Body {
        text,
        attachments,
        references,
        reply_to,
        auth: auth_headers(msg),
        importance: importance(msg),
        invite: calendar_part(msg)
            .filter(|_| encrypted.is_none())
            .and_then(|p| ical::parse(p.contents()).map(Box::new)),
        encrypted,
        unsubscribe,
    }
}

/// The part holding a message's invitation: its first `text/calendar`
/// part (the alternative an invite comes as), else its first `.ics` file.
fn calendar_part<'a>(msg: &'a Message<'a>) -> Option<&'a MessagePart<'a>> {
    let single =
        |p: &&MessagePart| !matches!(p.body, PartType::Multipart(_) | PartType::Message(_));
    let mut parts = msg.parts.iter().filter(single);
    parts
        .clone()
        .find(|p| content_type_is(p, "text", "calendar"))
        .or_else(|| parts.find(|p| is_ics_file(p)))
}

/// The bytes of a message's invitation ([`calendar_part`]), for Save .ics.
pub fn calendar_file(raw: &[u8]) -> Option<Vec<u8>> {
    let msg = MessageParser::default().parse(raw)?;
    calendar_part(&msg).map(|p| p.contents().to_vec())
}

fn render_input_of(msg: &Message, text: String) -> RenderInput {
    // mail-parser lists a text/plain part among the HTML bodies too (it
    // synthesizes HTML from it), so only a real text/html part counts. In a
    // multipart/alternative every HTML alternative is listed; the first wins.
    let html_part = html_part(msg);
    let prefers_plain = prefers_plain(msg, html_part);
    let inline = msg
        .parts
        .iter()
        .filter(|p| !matches!(p.body, PartType::Multipart(_) | PartType::Message(_)))
        .filter_map(|p| {
            let cid = p.content_id()?.trim().trim_matches(['<', '>']).trim();
            let mime = p
                .content_type()
                .map(|ct| {
                    let mut m = format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or_default());
                    m.make_ascii_lowercase();
                    m
                })
                .unwrap_or_default();
            (!cid.is_empty()).then(|| InlinePart {
                cid: cid.to_string(),
                mime,
                bytes: p.contents().into(),
            })
        })
        .collect();
    RenderInput {
        html: html_part
            .and_then(|_| msg.body_html(0))
            .map(|h| h.into_owned()),
        inline,
        text,
        prefers_plain,
    }
}

/// Whether the reader shows the text of a message whose real text/html
/// part is `html_part`: it has none, or the sender prefers plain text (RFC
/// 2046 5.1.4: alternatives go from least to most preferred, so a
/// text/plain part after the HTML means plain text is preferred).
fn prefers_plain(msg: &Message, html_part: Option<u32>) -> bool {
    let plain_part = msg
        .text_body
        .first()
        .copied()
        .filter(|&i| matches!(part_body(msg, i), Some(PartType::Text(_))));
    match (html_part, plain_part) {
        (Some(h), Some(t)) => t > h,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// The HTML a reply or forward quotes: the decoded text/html body the
/// reader renders. `None` for a message the reader shows as text. Not
/// sanitized (see `compose_html::sanitize`).
pub fn html_body(raw: &[u8]) -> Option<String> {
    let msg = MessageParser::default().parse(raw)?;
    let part = html_part(&msg);
    if prefers_plain(&msg, part) {
        return None;
    }
    msg.body_html(0).map(|h| h.into_owned())
}

/// The readable text of a parsed message, shared by [`parse_body`] and
/// [`parse_render_input`] so the reader and the fallback view agree.
fn body_text(msg: &Message) -> String {
    match (msg.text_body.first(), msg.html_body.first()) {
        // Prefer a real text/plain part; mail-parser synthesizes text from
        // HTML when only HTML exists, so check the part type.
        (Some(&i), _) if matches!(part_body(msg, i), Some(PartType::Text(_))) => {
            tidy(&msg.body_text(0).unwrap_or_default())
        }
        (_, Some(_)) => tidy(&html_to_text(&msg.body_html(0).unwrap_or_default())),
        _ => String::new(),
    }
}

fn part_body<'a>(msg: &'a Message, index: u32) -> Option<&'a PartType<'a>> {
    msg.parts.get(usize::try_from(index).ok()?).map(|p| &p.body)
}

/// Attachments shown to the user: everything mail-parser classifies as an
/// attachment except
/// - a part with a Content-ID that is not explicitly an attachment, when the
///   HTML body references it (`cid:`) or it has no file name (an inline image
///   the page draws);
/// - a `text/calendar` alternative when the message also carries the same
///   event as an `.ics` file (dedupe by UID);
/// - the signature part of a top-level `multipart/signed` (not verified);
/// - the version part (`Version: 1`) of a PGP/MIME encrypted message (its
///   ciphertext part is listed, to save for a program with the key).
///
/// An inline PGP block follows these as `message.asc` ([`pgp_block`]).
/// Every function numbering attachments goes through this filter, so the
/// indices stay consistent.
fn listed<'a>(msg: &'a Message<'a>) -> impl Iterator<Item = &'a MessagePart<'a>> {
    let cids = html_cids(msg);
    let children = |p: Option<&'a MessagePart<'a>>| match p.map(|p| &p.body) {
        Some(PartType::Multipart(ids)) => ids.as_slice(),
        _ => &[],
    };
    let signed = msg
        .parts
        .first()
        .filter(|p| content_type_is(p, "multipart", "signed"));
    let signature_ids = children(signed);
    let pgp_ids = match msg.parts.first() {
        root @ Some(p) if content_type_is(p, "multipart", "encrypted") => children(root),
        _ => &[],
    };
    // Children of every multipart/alternative: a text/calendar part there is
    // an alternative view of the message, not a file the sender attached.
    let alternative_ids: Vec<u32> = msg
        .parts
        .iter()
        .filter(|p| content_type_is(p, "multipart", "alternative"))
        .flat_map(|p| children(Some(p)).iter().copied())
        .collect();
    let ics_uids: Option<Vec<String>> = {
        let files: Vec<_> = msg.attachments().filter(|p| is_ics_file(p)).collect();
        (!files.is_empty()).then(|| files.iter().flat_map(|p| uids(p)).collect())
    };
    msg.attachments
        .iter()
        .filter_map(|&id| Some((id, msg.parts.get(usize::try_from(id).ok()?)?)))
        .filter(move |&(id, part)| {
            if pgp_ids.contains(&id) && content_type_is(part, "application", "pgp-encrypted") {
                return false;
            }
            let disposition_attachment = part
                .content_disposition()
                .is_some_and(|d| d.ctype().eq_ignore_ascii_case("attachment"));
            let inline_image = !disposition_attachment
                && part
                    .content_id()
                    .map(normalize_cid)
                    .is_some_and(|cid| part.attachment_name().is_none() || cids.contains(&cid));
            let signature = signature_ids.contains(&id)
                && (content_type_is(part, "application", "pkcs7-signature")
                    || content_type_is(part, "application", "x-pkcs7-signature")
                    || content_type_is(part, "application", "pgp-signature"));
            let calendar_twin = alternative_ids.contains(&id)
                && content_type_is(part, "text", "calendar")
                && ics_uids.as_ref().is_some_and(|ics| {
                    // Hide unless both name events and they differ.
                    let own = uids(part);
                    ics.is_empty() || own.is_empty() || own.iter().all(|u| ics.contains(u))
                });
            !(inline_image || signature || calendar_twin)
        })
        .map(|(_, part)| part)
}

/// The first real text/html part, as the renderer uses it.
fn html_part(msg: &Message) -> Option<u32> {
    msg.html_body
        .first()
        .copied()
        .filter(|&i| matches!(part_body(msg, i), Some(PartType::Html(_))))
}

/// The Content-IDs the HTML body references with `cid:` URLs, normalized as
/// [`normalize_cid`] does (the renderer matches them the same way).
fn html_cids(msg: &Message) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(PartType::Html(html)) = html_part(msg).and_then(|i| part_body(msg, i)) else {
        return out;
    };
    let bytes = html.as_bytes();
    let mut at = 0;
    while let Some(off) = bytes.get(at..).and_then(|rest| {
        rest.windows(4)
            .position(|w| w.eq_ignore_ascii_case(b"cid:"))
    }) {
        let start = at + off + 4;
        let len = bytes
            .get(start..)
            .unwrap_or_default()
            .iter()
            .position(|&b| {
                matches!(b, b'"' | b'\'' | b')' | b'>' | b'<') || b.is_ascii_whitespace()
            })
            .unwrap_or(bytes.len() - start);
        if let Some(id) = html.get(start..start + len) {
            out.insert(normalize_cid(id));
        }
        at = start + len;
    }
    out
}

/// A Content-ID or `cid:` URL body as compared: without angle brackets or
/// surrounding space, ASCII case folded (as the renderer resolves them).
fn normalize_cid(id: &str) -> String {
    id.trim()
        .trim_matches(['<', '>'])
        .trim()
        .to_ascii_lowercase()
}

fn content_type_is(part: &MessagePart, ctype: &str, subtype: &str) -> bool {
    part.content_type().is_some_and(|ct| {
        ct.ctype().eq_ignore_ascii_case(ctype)
            && ct
                .subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case(subtype))
    })
}

/// An iCalendar file the sender attached: `application/ics` or a `.ics` name.
fn is_ics_file(part: &MessagePart) -> bool {
    content_type_is(part, "application", "ics")
        || part.attachment_name().is_some_and(|n| {
            n.trim()
                .rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("ics"))
        })
}

/// The `UID:` values of an iCalendar part (unfolded lines are not needed:
/// only equality between two copies of the same invite matters).
fn uids(part: &MessagePart) -> Vec<String> {
    let text = String::from_utf8_lossy(part.contents());
    text.lines()
        .take(10_000)
        .filter_map(|l| {
            let (key, value) = l.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("UID")
                .then(|| value.trim().to_string())
        })
        .filter(|u| !u.is_empty())
        .collect()
}

/// File names and bytes of the attachments whose index satisfies `want`,
/// from a single parse of the message.
pub fn attachments(raw: &[u8], want: impl Fn(usize) -> bool) -> Vec<(String, Vec<u8>)> {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    let mut out: Vec<_> = listed(&msg)
        .enumerate()
        .filter(|(i, _)| want(*i))
        .map(|(i, part)| (attachment_name(part, i), part.contents().to_vec()))
        .collect();
    if want(listed(&msg).count())
        && let (text, Some(Encryption::PgpInline)) = readable(&msg)
    {
        out.extend(pgp_block(&text));
    }
    out
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
        // An unnamed invite (a text/calendar part with no .ics copy).
        if content_type_is(part, "text", "calendar") {
            return "invite.ics".to_string();
        }
        if content_type_is(part, "application", "pkcs7-mime")
            || content_type_is(part, "application", "x-pkcs7-mime")
        {
            return "smime.p7m".to_string();
        }
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

    // Windows ignores spaces before the extension here ("NUL .txt"), and
    // superscript digits count ("COM¹").
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let digit = |s: &str| {
        let mut c = s.chars();
        matches!(c.next(), Some('0'..='9' | '¹' | '²' | '³')) && c.next().is_none()
    };
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem.get(..3).zip(stem.get(3..)).is_some_and(|(p, n)| {
        (p.eq_ignore_ascii_case("COM") || p.eq_ignore_ascii_case("LPT")) && digit(n)
    });
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
        out.push_str(&super::smtp::mailbox(a.name().unwrap_or_default(), email));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn draft_fields_read_bcc_and_reply_ids() {
        let raw = b"From: a@x.org\r\nTo: b@x.org\r\nBcc: Hidden <h@x.org>, i@x.org\r\n\
            In-Reply-To: <p2@x>\r\nReferences: <p1@x> <p2@x>\r\nSubject: s\r\n\r\nbody";
        let f = draft_fields(raw);
        assert_eq!(f.bcc, "Hidden <h@x.org>, i@x.org");
        assert_eq!(f.in_reply_to, "<p2@x>");
        assert_eq!(f.references, "<p1@x> <p2@x>");
        assert_eq!(draft_fields(b"Subject: s\r\n\r\nx"), DraftFields::default());
    }

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
    fn importance_headers() {
        use crate::db::Importance::{High, Low, Normal};
        let of = |headers: &str| {
            parse_header(format!("From: a@b.c\r\n{headers}Subject: x\r\n\r\n").as_bytes())
                .importance
        };
        // None set, and the plain row has none.
        assert_eq!(of(""), Normal);
        assert_eq!(parse_header(HEADER).importance, Normal);
        // Importance (RFC 2156), any case, with a comment or extra space.
        assert_eq!(of("Importance: high\r\n"), High);
        assert_eq!(of("importance:   HIGH (urgent)\r\n"), High);
        assert_eq!(of("Importance: Low\r\n"), Low);
        assert_eq!(of("Importance: normal\r\n"), Normal);
        // X-Priority: its leading digit.
        assert_eq!(of("X-Priority: 1\r\n"), High);
        assert_eq!(of("X-Priority: 2 (High)\r\n"), High);
        assert_eq!(of("X-Priority: 3 (Normal)\r\n"), Normal);
        assert_eq!(of("X-Priority: 4\r\n"), Low);
        assert_eq!(of("X-Priority: 5 (Lowest)\r\n"), Low);
        // X-MSMail-Priority.
        assert_eq!(of("X-MSMail-Priority: High\r\n"), High);
        assert_eq!(of("X-MSMail-Priority: Low\r\n"), Low);
        // A folded value.
        assert_eq!(of("Importance:\r\n high\r\n"), High);
        // Importance wins, then X-Priority, then X-MSMail-Priority.
        assert_eq!(of("X-Priority: 5\r\nImportance: high\r\n"), High);
        assert_eq!(of("X-MSMail-Priority: Low\r\nX-Priority: 1\r\n"), High);
        assert_eq!(of("Importance: normal\r\nX-Priority: 1\r\n"), Normal);
        // A value that reads as none falls through to the next field.
        assert_eq!(of("Importance: whenever\r\nX-Priority: 1\r\n"), High);
        // The first field of a name counts.
        assert_eq!(of("Importance: low\r\nImportance: high\r\n"), Low);
        // Junk is normal.
        for junk in [
            "Importance: \r\n",
            "Importance: highest\r\n",
            "X-Priority: 0\r\n",
            "X-Priority: 9\r\n",
            "X-Priority: urgent\r\n",
            "X-MSMail-Priority: 1\r\n",
            "Importance: \u{fe0f}\u{fe0f}\r\n",
        ] {
            assert_eq!(of(junk), Normal, "{junk:?}");
        }
        // A long value is read only to its cap (cut inside a character
        // too), and its start still counts.
        let long = format!("Importance: high{}\r\n", " (é)".repeat(5000));
        assert_eq!(of(&long), High);
        let long = format!("X-Priority: {}1\r\n", " ".repeat(5000));
        assert_eq!(of(&long), Normal);
        assert_eq!(
            priority_value(&format!("x{}", "é".repeat(100))).len(),
            MAX_PRIORITY_LEN - 1
        );
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
        assert_eq!(sanitize_filename("NUL .txt"), "_NUL .txt");
        assert_eq!(sanitize_filename("lpt¹.doc"), "_lpt¹.doc");
        assert_eq!(sanitize_filename("conout$"), "_conout$");
        assert_eq!(sanitize_filename("console.txt"), "console.txt");
        assert_eq!(sanitize_filename("report. . ."), "report");
        assert_eq!(sanitize_filename("COM⁴.txt"), "COM⁴.txt");
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
        // logo: unnamed and referenced; banner: named and referenced (as
        // Outlook and Apple Mail send them); photo: named, not referenced;
        // report: referenced but explicitly an attachment.
        let raw = b"From: a@b.c\r\nContent-Type: multipart/related; boundary=R\r\n\r\n--R\r\nContent-Type: text/html\r\n\r\n<p>Hi <img src=\"cid:logo\"><img src='CID:Banner@X'><a href=cid:report>r</a></p>\r\n--R\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R\r\nContent-Type: image/png; name=banner.png\r\nContent-Disposition: inline; filename=banner.png\r\nContent-ID: <banner@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R\r\nContent-Type: image/png; name=photo.png\r\nContent-ID: <photo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R\r\nContent-Type: application/pdf; name=report.pdf\r\nContent-Disposition: attachment; filename=report.pdf\r\nContent-ID: <report>\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--R--\r\n";
        let b = parse_body(raw);
        let names: Vec<_> = b.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["photo.png", "report.pdf"]);
        assert_eq!(attachment(raw, 0).unwrap().0, "photo.png");
        assert_eq!(attachment(raw, 1).unwrap().0, "report.pdf");
        // The renderer still gets every Content-ID part.
        assert_eq!(parse_render_input(raw).inline.len(), 4);
    }

    #[test]
    fn calendar_alternative_dedupes_against_the_ics_file() {
        let msg = |alt_uid: &str, ics: bool| {
            let file = if ics {
                "--M\r\nContent-Type: application/ics; name=invite.ics\r\nContent-Disposition: attachment; filename=invite.ics\r\n\r\nBEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:one@x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            } else {
                ""
            };
            format!(
                "From: a@b.c\r\nContent-Type: multipart/mixed; boundary=M\r\n\r\n--M\r\nContent-Type: multipart/alternative; boundary=A\r\n\r\n--A\r\nContent-Type: text/plain\r\n\r\nInvite\r\n--A\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{alt_uid}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--A--\r\n{file}--M--\r\n"
            )
        };
        let names = |raw: String| -> Vec<String> {
            parse_body(raw.as_bytes())
                .attachments
                .into_iter()
                .map(|a| a.name)
                .collect()
        };
        assert_eq!(names(msg("one@x", true)), ["invite.ics"]);
        // A different event in the alternative is kept, under its own name.
        assert_eq!(names(msg("two@x", true)), ["invite.ics", "invite.ics"]);
        // No .ics file: the alternative is the invite.
        let raw = msg("one@x", false);
        assert_eq!(names(raw.clone()), ["invite.ics"]);
        let (name, data) = attachment(raw.as_bytes(), 0).unwrap();
        assert_eq!(name, "invite.ics");
        assert!(data.starts_with(b"BEGIN:VCALENDAR"));
    }

    #[test]
    fn cid_scan_survives_odd_html() {
        for html in [
            "cid:",
            "x cid:",
            "cid:\u{e9}\u{e9}",
            "<img src=\"cid:a\" cid:b>cid:",
        ] {
            let raw =
                format!("From: a@b.c\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}");
            parse_body(raw.as_bytes());
        }
    }

    #[test]
    fn render_input_keeps_hidden_inline_parts() {
        let raw = b"From: a@b.c\r\nContent-Type: multipart/related; boundary=R\r\n\r\n--R\r\nContent-Type: text/html\r\n\r\n<p>Hi <img src=\"cid:logo\"></p>\r\n--R\r\nContent-Type: IMAGE/PNG\r\nContent-ID: < logo >\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R--\r\n";
        let r = parse_render_input(raw);
        assert_eq!(r.html.as_deref(), Some("<p>Hi <img src=\"cid:logo\"></p>"));
        assert!(!r.prefers_plain);
        assert_eq!(r.text, "Hi");
        assert_eq!(r.inline.len(), 1);
        assert_eq!(r.inline[0].cid, "logo");
        assert_eq!(r.inline[0].mime, "image/png");
        assert_eq!(&r.inline[0].bytes[..], b"\x89PNG\r\n\x1a\n");
        assert!(parse_body(raw).attachments.is_empty());
    }

    #[test]
    fn render_input_plain_only_has_no_html() {
        let r = parse_render_input(FULL);
        assert_eq!(r.html, None);
        assert!(r.prefers_plain);
        assert_eq!(r.text, parse_body(FULL).text);
        assert!(r.inline.is_empty());
    }

    #[test]
    fn render_input_honours_alternative_order() {
        let alt = |first: &str, second: &str| {
            format!(
                "From: a@b.c\r\nContent-Type: multipart/alternative; boundary=A\r\n\r\n--A\r\n{first}\r\n--A\r\n{second}\r\n--A--\r\n"
            )
        };
        let plain = "Content-Type: text/plain\r\n\r\nplain";
        let html = "Content-Type: text/html\r\n\r\n<b>html</b>";
        let r = parse_render_input(alt(plain, html).as_bytes());
        assert_eq!(r.html.as_deref(), Some("<b>html</b>"));
        assert!(!r.prefers_plain);
        assert_eq!(r.text, "plain");
        let r = parse_render_input(alt(html, plain).as_bytes());
        assert_eq!(r.html.as_deref(), Some("<b>html</b>"));
        assert!(r.prefers_plain);
        // multipart/mixed led by text/plain: the plain body is the message,
        // as the reader shows it today.
        let mixed = alt(plain, html).replace("alternative", "mixed");
        let r = parse_render_input(mixed.as_bytes());
        assert_eq!(r.html, None);
        assert!(r.prefers_plain);
    }

    #[test]
    fn legacy_cjk_charsets_decode() {
        use base64::Engine as _;
        let word = |charset: &str, bytes: &[u8]| {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            format!("From: a@b.c\r\nSubject: =?{charset}?B?{b64}?=\r\n\r\n")
        };
        // 日本 in ISO-2022-JP, 中文 in Big5.
        let jp = word("ISO-2022-JP", b"\x1b$BF|K\\\x1b(B");
        assert_eq!(parse_header(jp.as_bytes()).subject, "日本");
        let big5 = word("Big5", b"\xa4\xa4\xa4\xe5");
        assert_eq!(parse_header(big5.as_bytes()).subject, "中文");
        // 中文 as a GB2312 body.
        let mut raw = b"From: a@b.c\r\nContent-Type: text/plain; charset=gb2312\r\n\r\n".to_vec();
        raw.extend_from_slice(b"\xd6\xd0\xce\xc4");
        let text = parse_body(&raw).text;
        assert!(!text.contains('\u{FFFD}'), "{text}");
        assert_eq!(text, "中文");
    }

    #[test]
    fn unsubscribe_uris_are_parsed() {
        let u = |v: &str| unsubscribe_uri(v);
        assert_eq!(
            u("<mailto:leave@list.example?subject=Unsubscribe%20me>").as_deref(),
            Some("mailto:leave@list.example?subject=Unsubscribe%20me")
        );
        // Several URIs: mailto first, else https; whitespace inside a URI
        // (folding) is ignored; text outside brackets is not a URI.
        assert_eq!(
            u("<https://l.example/u?a=1>, <mailto:u@l.example>").as_deref(),
            Some("mailto:u@l.example")
        );
        assert_eq!(
            u("<http://l.example/u>,\r\n <https://l.example/\r\n u>").as_deref(),
            Some("https://l.example/u")
        );
        assert_eq!(u("mailto:bare@l.example, https://l.example/u"), None);
        assert_eq!(u("<ftp://l.example/u>, <javascript:alert(1)>"), None);
        assert_eq!(u("<mailto:>, <https:>"), None);
        assert_eq!(u(""), None);
        assert_eq!(u("<<<>>>"), None);
        assert_eq!(u(&"<".repeat(100_000)), None);
        // The composer gets the subject and body from the URI.
        let m = crate::mail::render::links::mailto(
            &u("<MAILTO:leave@list.example?subject=Stop&body=Remove%20me%0D%0Aplease>").unwrap(),
        )
        .unwrap();
        assert_eq!(
            (m.to.as_str(), m.subject.as_str(), m.body.as_str()),
            ("leave@list.example", "Stop", "Remove me\nplease")
        );
        let raw = b"From: a@b.c\r\nList-Unsubscribe: <https://l.example/x>\r\n\r\nhi";
        assert_eq!(parse_body(raw).unsubscribe, "https://l.example/x");
    }

    #[test]
    fn auth_headers_are_kept_topmost_first_and_capped() {
        let mut raw = String::new();
        for i in 0..15 {
            raw.push_str(&format!(
                "Authentication-Results: mx{i}.example; dmarc=pass\r\n"
            ));
        }
        raw.push_str(&format!(
            "From: a@b.c\r\nauthentication-results: x; {}\r\n\r\nhi",
            "é".repeat(10_000)
        ));
        let auth = parse_body(raw.as_bytes()).auth;
        assert_eq!(auth.len(), crate::trust::MAX_AUTH_HEADERS);
        assert_eq!(auth[0], "mx0.example; dmarc=pass");
        assert_eq!(parse_body(b"From: a@b.c\r\n\r\nhi").auth.len(), 0);
        // A long value is cut on a character boundary.
        let raw = format!(
            "authentication-results: x; {}\r\nFrom: a@b.c\r\n\r\nhi",
            "é".repeat(10_000)
        );
        let auth = parse_body(raw.as_bytes()).auth;
        assert!(auth[0].len() <= crate::trust::MAX_AUTH_LEN);
    }

    #[test]
    fn inline_pgp_is_found_only_near_the_start() {
        let body =
            |text: &str| parse_body(format!("From: a@b.c\r\n\r\n{text}").as_bytes()).encrypted;
        assert_eq!(
            body("Hi\r\n-----BEGIN PGP MESSAGE-----\r\nx\r\n"),
            Some(Encryption::PgpInline)
        );
        assert_eq!(body("-----BEGIN PGP SIGNED MESSAGE-----\r\nx"), None);
        let late = format!("{}\r\n-----BEGIN PGP MESSAGE-----\r\n", "a ".repeat(3000));
        assert_eq!(body(&late), None);
        // An opaque S/MIME signature is not encryption.
        let raw =
            b"From: a@b.c\r\nContent-Type: application/pkcs7-mime; smime-type=signed-data\r\n\r\nx";
        assert_eq!(parse_body(raw).encrypted, None);
    }

    #[test]
    fn tidy_squeezes_and_trims() {
        assert_eq!(tidy("\r\n\r\n  a  \r\n\r\n\r\nb\r\n\r\n"), "a\n\nb");
    }
}
