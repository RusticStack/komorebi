use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use crate::ipc_validation::{validate_entity_id, IpcValidationError};

pub const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_PREVIEW_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_DISPLAY_FILENAME_BYTES: usize = 255;
pub const AUTO_OPEN_ATTACHMENTS: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentContentType {
    PlainText,
    Png,
    Jpeg,
    Gif,
    Webp,
    Pdf,
    Zip,
}

impl AttachmentContentType {
    pub const fn canonical_mime(self) -> &'static str {
        match self {
            Self::PlainText => "text/plain",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Pdf => "application/pdf",
            Self::Zip => "application/zip",
        }
    }

    pub const fn storage_extension(self) -> &'static str {
        match self {
            Self::PlainText => "txt",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Pdf => "pdf",
            Self::Zip => "zip",
        }
    }

    pub const fn preview_kind(self) -> Option<PreviewKind> {
        match self {
            Self::PlainText => Some(PreviewKind::PlainText),
            Self::Png | Self::Jpeg | Self::Gif | Self::Webp => Some(PreviewKind::RasterImage),
            Self::Pdf | Self::Zip => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    PlainText,
    RasterImage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentSafetyError {
    IpcValidation(IpcValidationError),
    EmptyStorageRoot,
    OversizedAttachment {
        max_bytes: u64,
        actual_bytes: u64,
    },
    OversizedPreview {
        max_bytes: u64,
        actual_bytes: u64,
    },
    UnsupportedContentType {
        declared_mime: Option<String>,
        sniffed_mime: Option<String>,
    },
    DeclaredTypeMismatch {
        declared_mime: String,
        sniffed_mime: String,
    },
    UserIntentRequired,
    PreviewUnsupported {
        content_type: AttachmentContentType,
    },
}

impl fmt::Display for AttachmentSafetyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::EmptyStorageRoot => write!(formatter, "attachment storage root is required"),
            Self::OversizedAttachment {
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "attachment exceeds {max_bytes} bytes ({actual_bytes} bytes provided)"
            ),
            Self::OversizedPreview {
                max_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "attachment preview exceeds {max_bytes} bytes ({actual_bytes} bytes provided)"
            ),
            Self::UnsupportedContentType { .. } => {
                write!(formatter, "attachment type is not allowed")
            }
            Self::DeclaredTypeMismatch { .. } => {
                write!(
                    formatter,
                    "attachment declared type does not match detected type"
                )
            }
            Self::UserIntentRequired => {
                write!(formatter, "attachment preview requires user intent")
            }
            Self::PreviewUnsupported { .. } => {
                write!(formatter, "attachment type is not previewable")
            }
        }
    }
}

impl Error for AttachmentSafetyError {}

impl From<IpcValidationError> for AttachmentSafetyError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentTypeEvidence<'a> {
    pub declared_mime: Option<&'a str>,
    pub sniffed_bytes: &'a [u8],
    pub byte_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentStorageKey<'a> {
    pub account_id: &'a str,
    pub folder_id: &'a str,
    pub message_id: &'a str,
    pub attachment_id: &'a str,
    pub content_type: AttachmentContentType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentPreviewRequest {
    pub content_type: AttachmentContentType,
    pub byte_count: u64,
    pub user_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentPreviewDecision {
    pub kind: PreviewKind,
    pub auto_open: bool,
}

pub fn validate_attachment_for_storage(
    evidence: AttachmentTypeEvidence<'_>,
) -> Result<AttachmentContentType, AttachmentSafetyError> {
    if evidence.byte_count > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentSafetyError::OversizedAttachment {
            max_bytes: MAX_ATTACHMENT_BYTES,
            actual_bytes: evidence.byte_count,
        });
    }

    let sniffed_type = sniff_attachment_content_type(evidence.sniffed_bytes);
    let declared_type = normalize_declared_mime(evidence.declared_mime);

    if declared_type
        .as_deref()
        .is_some_and(is_explicitly_dangerous_mime)
    {
        return Err(AttachmentSafetyError::UnsupportedContentType {
            declared_mime: declared_type,
            sniffed_mime: sniffed_type.map(|content_type| content_type.canonical_mime().into()),
        });
    }

    let Some(content_type) = sniffed_type else {
        return Err(AttachmentSafetyError::UnsupportedContentType {
            declared_mime: declared_type,
            sniffed_mime: None,
        });
    };

    if let Some(declared_mime) = declared_type {
        if !declared_type_matches_sniffed(&declared_mime, content_type) {
            return Err(AttachmentSafetyError::DeclaredTypeMismatch {
                declared_mime,
                sniffed_mime: content_type.canonical_mime().into(),
            });
        }
    }

    Ok(content_type)
}

pub fn attachment_storage_path(
    storage_root: &Path,
    key: AttachmentStorageKey<'_>,
) -> Result<PathBuf, AttachmentSafetyError> {
    if storage_root.as_os_str().is_empty() {
        return Err(AttachmentSafetyError::EmptyStorageRoot);
    }

    validate_entity_id("account_id", key.account_id)?;
    validate_entity_id("folder_id", key.folder_id)?;
    validate_entity_id("message_id", key.message_id)?;
    validate_entity_id("attachment_id", key.attachment_id)?;

    Ok(storage_root
        .join(key.account_id)
        .join(key.folder_id)
        .join(key.message_id)
        .join(format!(
            "{}.{}",
            key.attachment_id,
            key.content_type.storage_extension()
        )))
}

pub fn sanitize_display_filename(
    sender_filename: Option<&str>,
    content_type: AttachmentContentType,
) -> String {
    let fallback = format!("attachment.{}", content_type.storage_extension());
    let Some(sender_filename) = sender_filename else {
        return fallback;
    };

    let leaf = sender_filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    let sanitized: String = leaf
        .chars()
        .map(|character| match character {
            '\0' | '\r' | '\n' | '\t' => ' ',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect();

    let sanitized = sanitized
        .trim()
        .trim_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let sanitized = truncate_utf8(&sanitized, MAX_DISPLAY_FILENAME_BYTES);

    if sanitized.is_empty() {
        fallback
    } else {
        sanitized
    }
}

pub fn decide_attachment_preview(
    request: AttachmentPreviewRequest,
) -> Result<AttachmentPreviewDecision, AttachmentSafetyError> {
    if !request.user_requested {
        return Err(AttachmentSafetyError::UserIntentRequired);
    }

    if request.byte_count > MAX_PREVIEW_BYTES {
        return Err(AttachmentSafetyError::OversizedPreview {
            max_bytes: MAX_PREVIEW_BYTES,
            actual_bytes: request.byte_count,
        });
    }

    let Some(kind) = request.content_type.preview_kind() else {
        return Err(AttachmentSafetyError::PreviewUnsupported {
            content_type: request.content_type,
        });
    };

    Ok(AttachmentPreviewDecision {
        kind,
        auto_open: AUTO_OPEN_ATTACHMENTS,
    })
}

pub fn sniff_attachment_content_type(bytes: &[u8]) -> Option<AttachmentContentType> {
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        return Some(AttachmentContentType::Png);
    }

    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(AttachmentContentType::Jpeg);
    }

    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(AttachmentContentType::Gif);
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(AttachmentContentType::Webp);
    }

    if bytes.starts_with(b"%PDF-") {
        return Some(AttachmentContentType::Pdf);
    }

    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        return Some(AttachmentContentType::Zip);
    }

    if !bytes.is_empty() && std::str::from_utf8(bytes).is_ok() && !bytes.contains(&0) {
        return Some(AttachmentContentType::PlainText);
    }

    None
}

fn normalize_declared_mime(value: Option<&str>) -> Option<String> {
    value
        .map(|mime| {
            mime.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|mime| !mime.is_empty())
}

fn declared_type_matches_sniffed(declared_mime: &str, sniffed_type: AttachmentContentType) -> bool {
    if declared_mime == "application/octet-stream" {
        return true;
    }

    matches!(
        (declared_mime, sniffed_type),
        ("text/plain", AttachmentContentType::PlainText)
            | ("image/png", AttachmentContentType::Png)
            | ("image/jpeg", AttachmentContentType::Jpeg)
            | ("image/jpg", AttachmentContentType::Jpeg)
            | ("image/gif", AttachmentContentType::Gif)
            | ("image/webp", AttachmentContentType::Webp)
            | ("application/pdf", AttachmentContentType::Pdf)
            | ("application/zip", AttachmentContentType::Zip)
            | ("application/x-zip-compressed", AttachmentContentType::Zip)
    )
}

fn is_explicitly_dangerous_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/hta"
            | "application/javascript"
            | "application/msword"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-powerpoint"
            | "application/x-bat"
            | "application/x-cmd"
            | "application/x-dosexec"
            | "application/x-msdownload"
            | "application/x-msi"
            | "application/x-sh"
            | "image/svg+xml"
            | "text/html"
            | "text/javascript"
    )
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1A\nrest";
    const JPG_BYTES: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00];
    const PDF_BYTES: &[u8] = b"%PDF-1.7\nbody";
    const ZIP_BYTES: &[u8] = b"PK\x03\x04zip";

    fn storage_key(content_type: AttachmentContentType) -> AttachmentStorageKey<'static> {
        AttachmentStorageKey {
            account_id: "acct_1",
            folder_id: "folder_1",
            message_id: "msg_1",
            attachment_id: "att_1",
            content_type,
        }
    }

    #[test]
    fn validates_storage_type_with_sniffed_bytes_and_declared_type() {
        assert_eq!(
            validate_attachment_for_storage(AttachmentTypeEvidence {
                declared_mime: Some("image/png; name=image.png"),
                sniffed_bytes: PNG_BYTES,
                byte_count: PNG_BYTES.len() as u64,
            }),
            Ok(AttachmentContentType::Png)
        );

        assert_eq!(
            validate_attachment_for_storage(AttachmentTypeEvidence {
                declared_mime: Some("application/octet-stream"),
                sniffed_bytes: PDF_BYTES,
                byte_count: PDF_BYTES.len() as u64,
            }),
            Ok(AttachmentContentType::Pdf)
        );
    }

    #[test]
    fn rejects_oversized_attachments_and_unsupported_types() {
        assert_eq!(
            validate_attachment_for_storage(AttachmentTypeEvidence {
                declared_mime: Some("image/jpeg"),
                sniffed_bytes: JPG_BYTES,
                byte_count: MAX_ATTACHMENT_BYTES + 1,
            }),
            Err(AttachmentSafetyError::OversizedAttachment {
                max_bytes: MAX_ATTACHMENT_BYTES,
                actual_bytes: MAX_ATTACHMENT_BYTES + 1
            })
        );

        assert_eq!(
            validate_attachment_for_storage(AttachmentTypeEvidence {
                declared_mime: Some("application/x-msdownload"),
                sniffed_bytes: b"MZ executable",
                byte_count: 13,
            }),
            Err(AttachmentSafetyError::UnsupportedContentType {
                declared_mime: Some("application/x-msdownload".into()),
                sniffed_mime: Some("text/plain".into())
            })
        );
    }

    #[test]
    fn rejects_declared_type_mismatches() {
        assert_eq!(
            validate_attachment_for_storage(AttachmentTypeEvidence {
                declared_mime: Some("image/png"),
                sniffed_bytes: PDF_BYTES,
                byte_count: PDF_BYTES.len() as u64,
            }),
            Err(AttachmentSafetyError::DeclaredTypeMismatch {
                declared_mime: "image/png".into(),
                sniffed_mime: "application/pdf".into()
            })
        );
    }

    #[test]
    fn generates_storage_path_from_ids_not_sender_filename() {
        let root = Path::new("C:\\Users\\Alice\\AppData\\Local\\Komorebi\\attachments");
        let display_name = sanitize_display_filename(
            Some("..\\..\\Windows\\system32\\calc.exe"),
            AttachmentContentType::Png,
        );
        let path = attachment_storage_path(root, storage_key(AttachmentContentType::Png)).unwrap();
        let path_text = path.to_string_lossy();

        assert_eq!(display_name, "calc.exe");
        assert!(path.ends_with(Path::new("acct_1/folder_1/msg_1/att_1.png")));
        assert!(!path_text.contains("calc.exe"));
        assert!(!path_text.contains(".."));
        assert!(!path_text.contains("system32"));
    }

    #[test]
    fn rejects_path_traversal_in_storage_ids() {
        let root = Path::new("attachments");
        let result = attachment_storage_path(
            root,
            AttachmentStorageKey {
                attachment_id: "..\\evil",
                ..storage_key(AttachmentContentType::Pdf)
            },
        );

        assert_eq!(
            result,
            Err(AttachmentSafetyError::IpcValidation(
                IpcValidationError::InvalidId {
                    field: "attachment_id",
                    reason: "only ASCII letters, digits, hyphen, and underscore are allowed"
                }
            ))
        );
    }

    #[test]
    fn sanitizes_sender_filename_for_display_only() {
        assert_eq!(
            sanitize_display_filename(
                Some("C:\\Users\\Alice\\Desktop\\invoice:Q1?.pdf"),
                AttachmentContentType::Pdf,
            ),
            "invoice_Q1_.pdf"
        );
        assert_eq!(
            sanitize_display_filename(Some("..\0\r\n"), AttachmentContentType::Zip),
            "attachment.zip"
        );
        assert_eq!(
            sanitize_display_filename(None, AttachmentContentType::PlainText),
            "attachment.txt"
        );
    }

    #[test]
    fn preview_requires_user_intent_type_allowlist_and_size_limit() {
        assert_eq!(
            decide_attachment_preview(AttachmentPreviewRequest {
                content_type: AttachmentContentType::Png,
                byte_count: PNG_BYTES.len() as u64,
                user_requested: false,
            }),
            Err(AttachmentSafetyError::UserIntentRequired)
        );

        assert_eq!(
            decide_attachment_preview(AttachmentPreviewRequest {
                content_type: AttachmentContentType::Zip,
                byte_count: ZIP_BYTES.len() as u64,
                user_requested: true,
            }),
            Err(AttachmentSafetyError::PreviewUnsupported {
                content_type: AttachmentContentType::Zip
            })
        );

        assert_eq!(
            decide_attachment_preview(AttachmentPreviewRequest {
                content_type: AttachmentContentType::Jpeg,
                byte_count: MAX_PREVIEW_BYTES + 1,
                user_requested: true,
            }),
            Err(AttachmentSafetyError::OversizedPreview {
                max_bytes: MAX_PREVIEW_BYTES,
                actual_bytes: MAX_PREVIEW_BYTES + 1
            })
        );

        assert_eq!(
            decide_attachment_preview(AttachmentPreviewRequest {
                content_type: AttachmentContentType::PlainText,
                byte_count: 128,
                user_requested: true,
            }),
            Ok(AttachmentPreviewDecision {
                kind: PreviewKind::PlainText,
                auto_open: false
            })
        );
    }

    #[test]
    fn auto_open_is_forbidden() {
        let decision = decide_attachment_preview(AttachmentPreviewRequest {
            content_type: AttachmentContentType::Png,
            byte_count: PNG_BYTES.len() as u64,
            user_requested: true,
        })
        .unwrap();

        assert!(!decision.auto_open);
    }
}
