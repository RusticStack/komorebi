use crate::attachment_safety::{
    sanitize_display_filename, sniff_attachment_content_type, validate_attachment_for_storage,
    AttachmentContentType, AttachmentTypeEvidence, PreviewKind,
};
use crate::imap_sync::{
    stable_imap_message_id, RemoteFlag, RemoteMessage, SqliteSyncCache, SyncCache, SyncError,
};
use crate::ipc_validation::{validate_entity_id, IpcValidationError};
use mail_parser::{
    Addr, Address, HeaderValue, Message, MessageParser, MessagePart, MimeHeaders, PartType,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

const SNIPPET_LEN: usize = 240;
const DEFAULT_ATTACHMENT_FILENAME: &str = "attachment.bin";

#[derive(Debug, Clone, Copy)]
pub struct ParseInboundMimeRequest<'a> {
    pub account_id: &'a str,
    pub folder_id: &'a str,
    pub uid_validity: u64,
    pub uid: u64,
    pub raw_message: &'a [u8],
    pub internal_date: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ParseRemoteMessageRequest<'a> {
    pub uid_validity: u64,
    pub uid: u64,
    pub raw_message: &'a [u8],
    pub internal_date: Option<&'a str>,
    pub flags: &'a BTreeSet<RemoteFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInboundMime {
    pub account_id: String,
    pub folder_id: String,
    pub message_id: String,
    pub message: RemoteMessage,
    pub attachments: Vec<ParsedAttachmentRecord>,
    pub warnings: Vec<MimeParseWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAttachmentRecord {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub message_id: String,
    pub uid_validity: u64,
    pub message_uid: u64,
    pub part_index: usize,
    pub content_id: Option<String>,
    pub disposition: AttachmentDisposition,
    pub sender_filename: Option<String>,
    pub display_filename: String,
    pub declared_content_type: String,
    pub sniffed_content_type: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub preview_kind: Option<AttachmentPreviewKind>,
    pub storage: Option<ParsedAttachmentStorage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAttachmentStorage {
    pub content_type: AttachmentContentType,
    pub bytes: Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentDisposition {
    Attachment,
    Inline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentPreviewKind {
    PlainText,
    RasterImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MimeParseWarning {
    EncodingProblem {
        part_index: usize,
    },
    AttachmentNotStorageSafe {
        attachment_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MimeParseError {
    InvalidRecordId(String),
    InvalidUid,
    EmptyMessage,
    Unparseable,
    Serialization(String),
    SizeOverflow,
}

impl fmt::Display for MimeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecordId(message) => write!(f, "invalid MIME record id: {message}"),
            Self::InvalidUid => write!(f, "IMAP UID and UIDVALIDITY must be non-zero"),
            Self::EmptyMessage => write!(f, "raw MIME message is empty"),
            Self::Unparseable => write!(f, "raw MIME message could not be parsed"),
            Self::Serialization(message) => {
                write!(f, "MIME record serialization failed: {message}")
            }
            Self::SizeOverflow => write!(f, "MIME part size does not fit in u64"),
        }
    }
}

impl Error for MimeParseError {}

impl From<IpcValidationError> for MimeParseError {
    fn from(error: IpcValidationError) -> Self {
        Self::InvalidRecordId(error.to_string())
    }
}

impl From<serde_json::Error> for MimeParseError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MailAddressRecord {
    name: Option<String>,
    address: Option<String>,
}

pub fn parse_inbound_mime(
    request: ParseInboundMimeRequest<'_>,
) -> Result<ParsedInboundMime, MimeParseError> {
    validate_entity_id("account", request.account_id)?;
    validate_entity_id("folder", request.folder_id)?;

    if request.uid_validity == 0 || request.uid == 0 {
        return Err(MimeParseError::InvalidUid);
    }

    if request.raw_message.is_empty() {
        return Err(MimeParseError::EmptyMessage);
    }

    let message = MessageParser::default()
        .parse(request.raw_message)
        .ok_or(MimeParseError::Unparseable)?;

    normalize_message(request, &message)
}

pub fn parse_remote_message_from_raw_mime(
    request: ParseRemoteMessageRequest<'_>,
) -> Result<RemoteMessage, MimeParseError> {
    if request.uid_validity == 0 || request.uid == 0 {
        return Err(MimeParseError::InvalidUid);
    }

    if request.raw_message.is_empty() {
        return Err(MimeParseError::EmptyMessage);
    }

    let message = MessageParser::default()
        .parse(request.raw_message)
        .ok_or(MimeParseError::Unparseable)?;
    let has_attachments = message.attachments().next().is_some();

    normalize_remote_message(
        request.uid_validity,
        request.uid,
        request.raw_message.len(),
        request.internal_date,
        request.flags,
        &message,
        has_attachments,
    )
}

fn normalize_message(
    request: ParseInboundMimeRequest<'_>,
    message: &Message<'_>,
) -> Result<ParsedInboundMime, MimeParseError> {
    let mut warnings = collect_encoding_warnings(message);
    let attachments = normalize_attachments(request, message, &mut warnings)?;
    let mut remote_message = normalize_remote_message(
        request.uid_validity,
        request.uid,
        request.raw_message.len(),
        request.internal_date,
        &BTreeSet::new(),
        message,
        !attachments.is_empty(),
    )?;
    remote_message.parsed_from_raw_mime = true;
    remote_message.attachment_records = attachments.clone();
    remote_message.mime_parse_warnings_json = serialize_mime_warnings(&warnings)?;

    Ok(ParsedInboundMime {
        account_id: request.account_id.to_string(),
        folder_id: request.folder_id.to_string(),
        message_id: stable_imap_message_id(
            request.account_id,
            request.folder_id,
            request.uid_validity,
            request.uid,
        ),
        message: remote_message,
        attachments,
        warnings,
    })
}

fn normalize_remote_message(
    uid_validity: u64,
    uid: u64,
    raw_message_len: usize,
    internal_date: Option<&str>,
    flags: &BTreeSet<RemoteFlag>,
    message: &Message<'_>,
    has_attachments: bool,
) -> Result<RemoteMessage, MimeParseError> {
    let from = collect_addresses(message.from());
    let sender = from.first();
    let body_text = message.body_text(0).map(cow_to_string);
    let body_html = message.body_html(0).map(cow_to_string);
    let snippet = message.body_preview(SNIPPET_LEN).map(cow_to_string);
    let date_received = message
        .date()
        .map(|date| date.to_rfc3339())
        .or_else(|| internal_date.map(str::to_string));

    Ok(RemoteMessage {
        uid_validity,
        uid,
        provider_message_id: None,
        rfc_message_id: message.message_id().map(str::to_string),
        subject: message.subject().map_or_else(String::new, str::to_string),
        sender_name: sender.and_then(|address| address.name.clone()),
        sender_email: sender.and_then(|address| address.address.clone()),
        to_addresses_json: serialize_addresses(&collect_addresses(message.to()))?,
        cc_addresses_json: serialize_addresses(&collect_addresses(message.cc()))?,
        bcc_addresses_json: serialize_addresses(&collect_addresses(message.bcc()))?,
        reply_to_addresses_json: serialize_addresses(&collect_addresses(message.reply_to()))?,
        date_received,
        internal_date: internal_date.map(str::to_string),
        flags: flags.clone(),
        body_text,
        body_html,
        snippet,
        size_bytes: Some(
            raw_message_len
                .try_into()
                .map_err(|_| MimeParseError::SizeOverflow)?,
        ),
        has_attachments,
        in_reply_to: first_header_text(message.in_reply_to()),
        references_json: serialize_text_list(&header_texts(message.references()))?,
        parsed_from_raw_mime: false,
        attachment_records: Vec::new(),
        mime_parse_warnings_json: "[]".into(),
    })
}

pub fn persist_parsed_inbound_mime(
    connection: &mut Connection,
    parsed: &ParsedInboundMime,
) -> Result<(), SyncError> {
    let transaction = connection.transaction()?;

    {
        let mut cache = SqliteSyncCache::new(&transaction);
        cache.upsert_message(&parsed.account_id, &parsed.folder_id, &parsed.message)?;
    }

    transaction.commit()?;
    Ok(())
}

fn normalize_attachments(
    request: ParseInboundMimeRequest<'_>,
    message: &Message<'_>,
    warnings: &mut Vec<MimeParseWarning>,
) -> Result<Vec<ParsedAttachmentRecord>, MimeParseError> {
    let mut attachments = Vec::new();

    for (attachment_index, part) in message.attachments().enumerate() {
        let id = stable_attachment_id(request, attachment_index, part);
        let message_id = stable_imap_message_id(
            request.account_id,
            request.folder_id,
            request.uid_validity,
            request.uid,
        );
        let bytes = part.contents();
        let declared_content_type = declared_content_type(part);
        let sniffed = sniff_attachment_content_type(bytes);
        let sender_filename = part.attachment_name().map(str::to_string);
        let display_filename = display_filename(sender_filename.as_deref(), sniffed);
        let storage_content_type = match validate_attachment_for_storage(AttachmentTypeEvidence {
            declared_mime: Some(&declared_content_type),
            sniffed_bytes: bytes,
            byte_count: bytes
                .len()
                .try_into()
                .map_err(|_| MimeParseError::SizeOverflow)?,
        }) {
            Ok(content_type) => Some(content_type),
            Err(error) => {
                warnings.push(MimeParseWarning::AttachmentNotStorageSafe {
                    attachment_id: id.clone(),
                    reason: error.to_string(),
                });
                None
            }
        };
        let preview_kind = storage_content_type
            .and_then(AttachmentContentType::preview_kind)
            .map(AttachmentPreviewKind::from);
        let storage = storage_content_type.map(|content_type| ParsedAttachmentStorage {
            content_type,
            bytes: Arc::from(bytes),
        });

        attachments.push(ParsedAttachmentRecord {
            id,
            account_id: request.account_id.to_string(),
            folder_id: request.folder_id.to_string(),
            message_id,
            uid_validity: request.uid_validity,
            message_uid: request.uid,
            part_index: attachment_index,
            content_id: normalized_content_id(part),
            disposition: attachment_disposition(part),
            sender_filename,
            display_filename,
            declared_content_type,
            sniffed_content_type: sniffed.map(canonical_content_type),
            size_bytes: bytes
                .len()
                .try_into()
                .map_err(|_| MimeParseError::SizeOverflow)?,
            sha256: sha256_hex(bytes),
            preview_kind,
            storage,
        });
    }

    Ok(attachments)
}

fn collect_encoding_warnings(message: &Message<'_>) -> Vec<MimeParseWarning> {
    message
        .parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            part.is_encoding_problem
                .then_some(MimeParseWarning::EncodingProblem { part_index: index })
        })
        .collect()
}

fn collect_addresses(address: Option<&Address<'_>>) -> Vec<MailAddressRecord> {
    address
        .map(|address| match address {
            Address::List(addresses) => addresses.iter().filter_map(address_record).collect(),
            Address::Group(groups) => groups
                .iter()
                .flat_map(|group| group.addresses.iter())
                .filter_map(address_record)
                .collect(),
        })
        .unwrap_or_default()
}

fn address_record(addr: &Addr<'_>) -> Option<MailAddressRecord> {
    let record = MailAddressRecord {
        name: addr.name.as_ref().map(|value| value.to_string()),
        address: addr.address.as_ref().map(|value| value.to_string()),
    };
    (record.name.is_some() || record.address.is_some()).then_some(record)
}

fn serialize_addresses(addresses: &[MailAddressRecord]) -> Result<String, MimeParseError> {
    Ok(serde_json::to_string(addresses)?)
}

fn serialize_text_list(values: &[String]) -> Result<String, MimeParseError> {
    Ok(serde_json::to_string(values)?)
}

fn serialize_mime_warnings(warnings: &[MimeParseWarning]) -> Result<String, MimeParseError> {
    Ok(serde_json::to_string(warnings)?)
}

fn first_header_text(value: &HeaderValue<'_>) -> Option<String> {
    header_texts(value).into_iter().next()
}

fn header_texts(value: &HeaderValue<'_>) -> Vec<String> {
    match value {
        HeaderValue::Text(text) => vec![text.to_string()],
        HeaderValue::TextList(values) => values.iter().map(|value| value.to_string()).collect(),
        HeaderValue::Address(address) => collect_addresses(Some(address))
            .into_iter()
            .filter_map(|address| address.address.or(address.name))
            .collect(),
        HeaderValue::Empty => Vec::new(),
        HeaderValue::DateTime(value) => vec![value.to_rfc3339()],
        HeaderValue::ContentType(value) => vec![format!(
            "{}/{}",
            value.ctype(),
            value.subtype().unwrap_or("octet-stream")
        )],
        HeaderValue::Received(_) => Vec::new(),
    }
}

fn cow_to_string(value: Cow<'_, str>) -> String {
    value.into_owned()
}

fn normalized_content_id(part: &MessagePart<'_>) -> Option<String> {
    part.content_id().map(|content_id| {
        content_id
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_string()
    })
}

fn attachment_disposition(part: &MessagePart<'_>) -> AttachmentDisposition {
    let inline_disposition = part
        .content_disposition()
        .is_some_and(|disposition| disposition.is_inline());
    let inline_body = matches!(&part.body, PartType::InlineBinary(_));

    if inline_disposition || inline_body || part.content_id().is_some() {
        AttachmentDisposition::Inline
    } else {
        AttachmentDisposition::Attachment
    }
}

fn declared_content_type(part: &MessagePart<'_>) -> String {
    part.content_type()
        .map(|content_type| {
            let ctype = content_type.ctype().to_ascii_lowercase();
            let subtype = content_type
                .subtype()
                .unwrap_or("octet-stream")
                .to_ascii_lowercase();
            format!("{ctype}/{subtype}")
        })
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn display_filename(
    sender_filename: Option<&str>,
    sniffed_content_type: Option<AttachmentContentType>,
) -> String {
    if let Some(content_type) = sniffed_content_type {
        sanitize_display_filename(sender_filename, content_type)
    } else {
        sanitize_display_filename_fallback(sender_filename)
    }
}

fn sanitize_display_filename_fallback(sender_filename: Option<&str>) -> String {
    let Some(sender_filename) = sender_filename else {
        return DEFAULT_ATTACHMENT_FILENAME.to_string();
    };

    let last_path_segment = sender_filename
        .rsplit(['/', '\\'])
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(DEFAULT_ATTACHMENT_FILENAME);
    let mut sanitized = String::with_capacity(last_path_segment.len());

    for ch in last_path_segment.chars() {
        match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => sanitized.push('_'),
            ch if ch.is_control() => sanitized.push('_'),
            ch => sanitized.push(ch),
        }
    }

    let trimmed = sanitized.trim().trim_matches('.').trim().to_string();
    if trimmed.is_empty() {
        DEFAULT_ATTACHMENT_FILENAME.to_string()
    } else {
        trimmed
    }
}

fn stable_attachment_id(
    request: ParseInboundMimeRequest<'_>,
    attachment_index: usize,
    part: &MessagePart<'_>,
) -> String {
    let mut digest = Sha256::new();
    digest.update(request.account_id.as_bytes());
    digest.update([0]);
    digest.update(request.folder_id.as_bytes());
    digest.update([0]);
    digest.update(request.uid_validity.to_be_bytes());
    digest.update(request.uid.to_be_bytes());
    digest.update(
        u64::try_from(attachment_index)
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    if let Some(content_id) = normalized_content_id(part) {
        digest.update(content_id.as_bytes());
    }
    if let Some(filename) = part.attachment_name() {
        digest.update(filename.as_bytes());
    }
    digest.update(declared_content_type(part).as_bytes());
    digest.update(sha256_hex(part.contents()).as_bytes());

    let hex = hex_digest(digest.finalize().as_slice());
    format!("att_{}", &hex[..32])
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex_digest(digest.finalize().as_slice())
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn canonical_content_type(content_type: AttachmentContentType) -> String {
    match content_type {
        AttachmentContentType::PlainText => "text/plain",
        AttachmentContentType::Png => "image/png",
        AttachmentContentType::Jpeg => "image/jpeg",
        AttachmentContentType::Gif => "image/gif",
        AttachmentContentType::Webp => "image/webp",
        AttachmentContentType::Pdf => "application/pdf",
        AttachmentContentType::Zip => "application/zip",
    }
    .to_string()
}

impl From<PreviewKind> for AttachmentPreviewKind {
    fn from(value: PreviewKind) -> Self {
        match value {
            PreviewKind::PlainText => Self::PlainText,
            PreviewKind::RasterImage => Self::RasterImage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;
    use rusqlite::Connection;
    use std::{fs, path::PathBuf};

    fn request(raw_message: &[u8]) -> ParseInboundMimeRequest<'_> {
        ParseInboundMimeRequest {
            account_id: "account_01",
            folder_id: "folder_inbox",
            uid_validity: 42,
            uid: 7,
            raw_message,
            internal_date: Some("2026-05-22T10:30:00+01:00"),
        }
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        apply_migrations(&mut connection).expect("apply migrations");
        connection
            .execute(
                "INSERT INTO accounts (
                   id,
                   provider_kind,
                   display_email,
                   username,
                   auth_kind,
                   credential_ref,
                   imap_host,
                   imap_port,
                   imap_security,
                   smtp_host,
                   smtp_port,
                   smtp_security
                 )
                 VALUES (
                   'account_01',
                   'generic_imap_smtp',
                   'reader@example.com',
                   'reader@example.com',
                   'app_password',
                   'cred_account_01',
                   'imap.example.com',
                   993,
                   'tls',
                   'smtp.example.com',
                   465,
                   'tls'
                 )",
                [],
            )
            .expect("seed account");
        connection
            .execute(
                "INSERT INTO folders (
                   id,
                   account_id,
                   remote_id,
                   name,
                   display_name,
                   kind
                 )
                 VALUES (
                   'folder_inbox',
                   'account_01',
                   'INBOX',
                   'INBOX',
                   'Inbox',
                   'inbox'
                 )",
                [],
            )
            .expect("seed folder");
        connection
    }

    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("mime")
            .join(name);
        fs::read(&path)
            .unwrap_or_else(|error| panic!("read MIME fixture {}: {error}", path.display()))
    }

    #[test]
    fn reusable_mime_fixture_corpus_covers_parser_boundaries() {
        let encoded = fixture_bytes("encoded-portuguese-japanese-alternative.eml");
        let encoded = parse_inbound_mime(request(&encoded)).expect("encoded fixture parses");

        assert_eq!(
            encoded.message.subject,
            "Relat\u{00f3}rio \u{2014} ol\u{00e1} e \u{65e5}\u{672c}\u{8a9e}"
        );
        assert_eq!(
            encoded.message.sender_name.as_deref(),
            Some("Jos\u{00e9} Silva")
        );
        assert_eq!(
            encoded.message.body_text.as_deref().map(str::trim),
            Some("Ol\u{00e1}, o relat\u{00f3}rio de seguran\u{00e7}a est\u{00e1} pronto.")
        );
        assert!(encoded
            .message
            .body_html
            .as_deref()
            .is_some_and(|body| body.contains("relat\u{00f3}rio de seguran\u{00e7}a")));
        let recipients: Vec<MailAddressRecord> =
            serde_json::from_str(&encoded.message.to_addresses_json).expect("to JSON");
        assert!(recipients.iter().any(
            |recipient| recipient.name.as_deref() == Some("\u{5c71}\u{7530} \u{592a}\u{90ce}")
        ));
        assert!(recipients.iter().any(|recipient| {
            recipient.name.as_deref() == Some("Ana Cora\u{00e7}\u{00e3}o")
                && recipient.address.as_deref() == Some("ana@example.pt")
        }));

        let nested = fixture_bytes("nested-related-attachments.eml");
        let nested = parse_inbound_mime(request(&nested)).expect("nested fixture parses");

        assert!(nested
            .message
            .body_html
            .as_deref()
            .is_some_and(|body| body.contains("cid:logo.fixture@example.test")));
        assert_eq!(nested.attachments.len(), 2);
        let inline = &nested.attachments[0];
        assert_eq!(inline.disposition, AttachmentDisposition::Inline);
        assert_eq!(
            inline.content_id.as_deref(),
            Some("logo.fixture@example.test")
        );
        assert_eq!(
            inline.preview_kind,
            Some(AttachmentPreviewKind::RasterImage)
        );
        let attachment = &nested.attachments[1];
        assert_eq!(attachment.disposition, AttachmentDisposition::Attachment);
        assert_eq!(
            attachment.sender_filename.as_deref(),
            Some("../../reports/quarterly?.pdf")
        );
        assert_eq!(attachment.display_filename, "quarterly_.pdf");
        assert_eq!(
            attachment.sniffed_content_type.as_deref(),
            Some("application/pdf")
        );

        let malformed = fixture_bytes("malformed-boundary.eml");
        let malformed =
            parse_inbound_mime(request(&malformed)).expect("malformed fixture still parses");

        assert_eq!(malformed.message.subject, "Missing closing boundary");
        assert!(malformed
            .message
            .body_text
            .as_deref()
            .is_some_and(|body| body.contains("final boundary is absent")));
        assert!(!malformed.attachments.is_empty());

        let hostile = fixture_bytes("phishing-xss-html-alternative.eml");
        let hostile = parse_inbound_mime(request(&hostile)).expect("hostile HTML fixture parses");

        assert_eq!(
            hostile.message.subject,
            "A\u{00e7}\u{00e3}o necess\u{00e1}ria"
        );
        assert!(hostile
            .message
            .body_text
            .as_deref()
            .is_some_and(|body| body.contains("verifique o portal manualmente")));
        let html = hostile
            .message
            .body_html
            .as_deref()
            .expect("hostile fixture keeps HTML for rendering-boundary tests");
        assert!(html.contains("https://evil.example.test/login"));
        assert!(html.contains("javascript:alert"));
        assert!(html.contains("<script>"));
        assert!(html.contains("onerror="));
    }

    #[test]
    fn parses_encoded_headers_non_ascii_addresses_and_charset_bodies() {
        let raw = br#"From: =?UTF-8?Q?Jos=C3=A9_Silva?= <jose@example.com>
To: =?UTF-8?B?5pel5pys6Kqe?= <nihon@example.jp>, Ana <ana@example.pt>
Cc: "Team" <team@example.com>
Reply-To: Responder <reply@example.com>
Date: Fri, 22 May 2026 10:00:00 +0100
Message-ID: <msg-1@example.com>
In-Reply-To: <prev@example.com>
References: <root@example.com> <prev@example.com>
Subject: =?UTF-8?Q?Relat=C3=B3rio_=E2=80=94_ol=C3=A1?=
Content-Type: multipart/alternative; boundary="alt"

--alt
Content-Type: text/plain; charset=iso-8859-1
Content-Transfer-Encoding: quoted-printable

Ol=E1 em texto
--alt
Content-Type: text/html; charset=utf-8
Content-Transfer-Encoding: quoted-printable

<p>Ol=C3=A1 <strong>HTML</strong></p>
--alt--
"#;

        let parsed = parse_inbound_mime(request(raw)).expect("message parses");

        assert_eq!(
            parsed.message.rfc_message_id.as_deref(),
            Some("msg-1@example.com")
        );
        assert_eq!(
            parsed.message.subject,
            "Relat\u{00f3}rio \u{2014} ol\u{00e1}"
        );
        assert_eq!(
            parsed.message.sender_name.as_deref(),
            Some("Jos\u{00e9} Silva")
        );
        assert_eq!(
            parsed.message.sender_email.as_deref(),
            Some("jose@example.com")
        );
        assert_eq!(
            parsed.message.body_text.as_deref(),
            Some("Ol\u{00e1} em texto")
        );
        assert_eq!(
            parsed.message.body_html.as_deref(),
            Some("<p>Ol\u{00e1} <strong>HTML</strong></p>")
        );
        assert_eq!(
            parsed.message.in_reply_to.as_deref(),
            Some("prev@example.com")
        );
        assert!(!parsed.message.has_attachments);

        let to: Vec<MailAddressRecord> =
            serde_json::from_str(&parsed.message.to_addresses_json).expect("to JSON");
        assert_eq!(
            to,
            vec![
                MailAddressRecord {
                    name: Some("\u{65e5}\u{672c}\u{8a9e}".to_string()),
                    address: Some("nihon@example.jp".to_string())
                },
                MailAddressRecord {
                    name: Some("Ana".to_string()),
                    address: Some("ana@example.pt".to_string())
                }
            ]
        );

        let references: Vec<String> =
            serde_json::from_str(&parsed.message.references_json).expect("references JSON");
        assert_eq!(references, vec!["root@example.com", "prev@example.com"]);
    }

    #[test]
    fn parses_inline_images_and_attachments_into_stable_records() {
        let raw = br#"From: Sender <sender@example.com>
To: Reader <reader@example.com>
Subject: Attachments
Message-ID: <msg-attachments@example.com>
Content-Type: multipart/mixed; boundary="mix"

--mix
Content-Type: multipart/related; boundary="rel"

--rel
Content-Type: text/html; charset=utf-8

<p><img src="cid:logo@example"></p>
--rel
Content-Type: image/png; name="logo.png"
Content-Disposition: inline; filename="logo.png"
Content-ID: <logo@example>
Content-Transfer-Encoding: base64

iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=
--rel--
--mix
Content-Type: application/pdf
Content-Disposition: attachment; filename="../../report?.pdf"
Content-Transfer-Encoding: base64

JVBERi0xLjQKJUVPRgo=
--mix--
"#;

        let first = parse_inbound_mime(request(raw)).expect("message parses");
        let second = parse_inbound_mime(request(raw)).expect("message parses again");

        assert!(first.message.has_attachments);
        assert!(first.message.parsed_from_raw_mime);
        assert_eq!(first.message.attachment_records, first.attachments);
        assert_eq!(first.attachments, second.attachments);
        assert_eq!(first.attachments.len(), 2);

        let inline = &first.attachments[0];
        assert!(inline.message_id.starts_with("msg_"));
        assert_eq!(inline.disposition, AttachmentDisposition::Inline);
        assert_eq!(inline.content_id.as_deref(), Some("logo@example"));
        assert_eq!(inline.sender_filename.as_deref(), Some("logo.png"));
        assert_eq!(inline.display_filename, "logo.png");
        assert_eq!(inline.declared_content_type, "image/png");
        assert_eq!(inline.sniffed_content_type.as_deref(), Some("image/png"));
        assert_eq!(
            inline.preview_kind,
            Some(AttachmentPreviewKind::RasterImage)
        );
        assert_eq!(inline.sha256.len(), 64);
        assert!(inline.id.starts_with("att_"));

        let attachment = &first.attachments[1];
        assert_eq!(attachment.disposition, AttachmentDisposition::Attachment);
        assert_eq!(
            attachment.sender_filename.as_deref(),
            Some("../../report?.pdf")
        );
        assert_eq!(attachment.display_filename, "report_.pdf");
        assert_eq!(attachment.declared_content_type, "application/pdf");
        assert_eq!(
            attachment.sniffed_content_type.as_deref(),
            Some("application/pdf")
        );
        assert_eq!(attachment.preview_kind, None);
        assert_eq!(attachment.size_bytes, 14);
    }

    #[test]
    fn malformed_multipart_still_produces_stable_body_record() {
        let raw = br#"From: Broken <broken@example.com>
To: Reader <reader@example.com>
Subject: Missing closing boundary
Message-ID: <broken@example.com>
Content-Type: multipart/mixed; boundary="broken"

--broken
Content-Type: text/plain; charset=utf-8

Body survives even when the final boundary is absent.
--broken
Content-Type: application/octet-stream
Content-Transfer-Encoding: base64

@@not-base64@@
"#;

        let parsed = parse_inbound_mime(request(raw)).expect("message parses despite defects");

        assert_eq!(
            parsed.message.rfc_message_id.as_deref(),
            Some("broken@example.com")
        );
        assert_eq!(
            parsed.message.body_text.as_deref(),
            Some("Body survives even when the final boundary is absent.")
        );
        assert!(!parsed.attachments.is_empty());
        assert_eq!(parsed.message.attachment_records, parsed.attachments);
        assert!(!parsed.warnings.is_empty());
    }

    #[test]
    fn persists_normalized_message_and_attachment_metadata_to_sqlite() {
        let raw = br#"From: Sender <sender@example.com>
To: Reader <reader@example.com>
Subject: Persisted MIME
Message-ID: <persisted@example.com>
In-Reply-To: <previous@example.com>
Content-Type: multipart/mixed; boundary="mix"

--mix
Content-Type: text/plain; charset=utf-8

Persisted body
--mix
Content-Type: text/plain; name="notes.txt"
Content-Disposition: attachment; filename="../../notes.txt"
Content-Transfer-Encoding: base64

Tm90ZSBib2R5
--mix--
"#;
        let parsed = parse_inbound_mime(request(raw)).expect("message parses");
        let mut connection = migrated_connection();

        persist_parsed_inbound_mime(&mut connection, &parsed).expect("persist parsed MIME");
        persist_parsed_inbound_mime(&mut connection, &parsed).expect("persist is idempotent");

        let message: (String, Option<String>, Option<String>, i64, String) = connection
            .query_row(
                "SELECT id, in_reply_to, body_text, has_attachments, mime_parse_warnings_json
                 FROM messages
                 WHERE account_id = 'account_01'
                   AND folder_id = 'folder_inbox'
                   AND imap_uid = 7",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("message row");

        assert_eq!(message.0, parsed.message_id);
        assert_eq!(message.1.as_deref(), Some("previous@example.com"));
        assert_eq!(message.2.as_deref(), Some("Persisted body"));
        assert_eq!(message.3, 1);
        assert_eq!(message.4, parsed.message.mime_parse_warnings_json);

        let attachment: (i64, String, String, String, i64, i64, Option<String>) = connection
            .query_row(
                "SELECT
                   COUNT(*),
                   message_id,
                   display_filename,
                   canonical_mime,
                   size_bytes,
                   is_inline,
                   preview_kind
                 FROM attachments
                 WHERE account_id = 'account_01'
                   AND folder_id = 'folder_inbox'
                 GROUP BY message_id, display_filename, canonical_mime, size_bytes, is_inline, preview_kind",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .expect("attachment row");

        assert_eq!(attachment.0, 1);
        assert_eq!(attachment.1, parsed.message_id);
        assert_eq!(attachment.2, "notes.txt");
        assert_eq!(attachment.3, "text/plain");
        assert_eq!(attachment.4, 9);
        assert_eq!(attachment.5, 0);
        assert_eq!(attachment.6.as_deref(), Some("plain_text"));
    }

    #[test]
    fn rejects_empty_or_invalid_identity_without_panicking() {
        let raw = b"Subject: no body";

        let mut invalid = request(raw);
        invalid.uid = 0;
        assert_eq!(parse_inbound_mime(invalid), Err(MimeParseError::InvalidUid));

        let mut invalid = request(raw);
        invalid.account_id = "../account";
        assert!(matches!(
            parse_inbound_mime(invalid),
            Err(MimeParseError::InvalidRecordId(_))
        ));

        let empty = ParseInboundMimeRequest {
            raw_message: b"",
            ..request(raw)
        };
        assert_eq!(parse_inbound_mime(empty), Err(MimeParseError::EmptyMessage));
    }
}
