use std::{
    error::Error,
    ffi::OsStr,
    fmt, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::{
    attachment_safety::{
        decide_attachment_preview, sanitize_display_filename, validate_attachment_for_storage,
        AttachmentContentType, AttachmentPreviewRequest, AttachmentSafetyError,
        AttachmentTypeEvidence, PreviewKind, INBOUND_ATTACHMENT_DIR_NAME, MAX_ATTACHMENT_BYTES,
    },
    ipc_validation::{
        validate_entity_id, validate_payload_size, IpcValidationError, MAX_TEXT_FIELD_BYTES,
    },
    schema::{apply_migrations, SchemaMigrationError},
};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const ATTACHMENT_SAVE_DIR_NAME: &str = "Komorebi";
const MAX_SAVE_COLLISION_ATTEMPTS: usize = 1000;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentCommandRequest {
    pub account_id: String,
    pub folder_id: String,
    pub message_id: String,
    pub attachment_id: String,
    pub user_requested: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AttachmentPreviewKindView {
    PlainText,
    RasterImage,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPreviewResponse {
    pub attachment_id: String,
    pub display_filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub kind: AttachmentPreviewKindView,
    pub text: Option<String>,
    pub bytes: Option<Vec<u8>>,
    pub auto_open: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDownloadResponse {
    pub attachment_id: String,
    pub display_filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSaveResponse {
    pub attachment_id: String,
    pub display_filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub saved_filename: String,
    pub destination_label: String,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentCommandError {
    pub reason: String,
}

impl AttachmentCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for AttachmentCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for AttachmentCommandError {}

impl From<rusqlite::Error> for AttachmentCommandError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SchemaMigrationError> for AttachmentCommandError {
    fn from(value: SchemaMigrationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<IpcValidationError> for AttachmentCommandError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AttachmentSafetyError> for AttachmentCommandError {
    fn from(value: AttachmentSafetyError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<tauri::Error> for AttachmentCommandError {
    fn from(value: tauri::Error) -> Self {
        Self::new(value.to_string())
    }
}

#[tauri::command]
pub fn preview_attachment(
    app: tauri::AppHandle,
    request: AttachmentCommandRequest,
) -> Result<AttachmentPreviewResponse, AttachmentCommandError> {
    let app_data_dir = app_data_dir(&app)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    preview_attachment_from_path(&cache_path, &app_data_dir, request)
}

#[tauri::command]
pub fn download_attachment(
    app: tauri::AppHandle,
    request: AttachmentCommandRequest,
) -> Result<AttachmentDownloadResponse, AttachmentCommandError> {
    let app_data_dir = app_data_dir(&app)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    download_attachment_from_path(&cache_path, &app_data_dir, request)
}

#[tauri::command]
pub fn save_attachment(
    app: tauri::AppHandle,
    request: AttachmentCommandRequest,
) -> Result<AttachmentSaveResponse, AttachmentCommandError> {
    let app_data_dir = app_data_dir(&app)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    let source = load_attachment_source(&connection, &app_data_dir, request)?;
    let destination = selected_save_destination(&app, &source)?;
    save_attachment_source_to_user_path(source, &destination)
}

pub fn preview_attachment_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentPreviewResponse, AttachmentCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    preview_attachment_from_connection(&connection, app_data_dir, request)
}

pub fn preview_attachment_from_connection(
    connection: &Connection,
    app_data_dir: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentPreviewResponse, AttachmentCommandError> {
    let source = load_attachment_source(connection, app_data_dir, request)?;
    let decision = decide_attachment_preview(AttachmentPreviewRequest {
        content_type: source.content_type,
        byte_count: source.size_bytes,
        user_requested: true,
    })?;

    match decision.kind {
        PreviewKind::PlainText => {
            let text = String::from_utf8(source.bytes)
                .map_err(|_| AttachmentCommandError::new("attachment preview is unavailable"))?;
            Ok(AttachmentPreviewResponse {
                attachment_id: source.attachment_id,
                display_filename: source.export_filename,
                content_type: source.content_type.canonical_mime().into(),
                size_bytes: source.size_bytes,
                kind: AttachmentPreviewKindView::PlainText,
                text: Some(text),
                bytes: None,
                auto_open: decision.auto_open,
            })
        }
        PreviewKind::RasterImage => Ok(AttachmentPreviewResponse {
            attachment_id: source.attachment_id,
            display_filename: source.export_filename,
            content_type: source.content_type.canonical_mime().into(),
            size_bytes: source.size_bytes,
            kind: AttachmentPreviewKindView::RasterImage,
            text: None,
            bytes: Some(source.bytes),
            auto_open: decision.auto_open,
        }),
    }
}

pub fn download_attachment_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentDownloadResponse, AttachmentCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    download_attachment_from_connection(&connection, app_data_dir, request)
}

pub fn download_attachment_from_connection(
    connection: &Connection,
    app_data_dir: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentDownloadResponse, AttachmentCommandError> {
    let source = load_attachment_source(connection, app_data_dir, request)?;

    Ok(AttachmentDownloadResponse {
        attachment_id: source.attachment_id,
        display_filename: source.export_filename,
        content_type: source.content_type.canonical_mime().into(),
        size_bytes: source.size_bytes,
        bytes: source.bytes,
    })
}

pub fn save_attachment_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    save_root: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentSaveResponse, AttachmentCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    save_attachment_from_connection(&connection, app_data_dir, save_root, request)
}

pub fn save_attachment_from_connection(
    connection: &Connection,
    app_data_dir: &Path,
    save_root: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentSaveResponse, AttachmentCommandError> {
    let source = load_attachment_source(connection, app_data_dir, request)?;
    fs::create_dir_all(save_root)
        .map_err(|_| AttachmentCommandError::new("attachment save failed"))?;
    let root = save_root
        .canonicalize()
        .map_err(|_| AttachmentCommandError::new("attachment save destination is unavailable"))?;
    let (saved_filename, bytes_written) =
        write_without_overwrite(&root, &source.export_filename, &source.bytes)?;

    Ok(AttachmentSaveResponse {
        attachment_id: source.attachment_id,
        display_filename: source.export_filename,
        content_type: source.content_type.canonical_mime().into(),
        size_bytes: source.size_bytes,
        destination_label: format!("{ATTACHMENT_SAVE_DIR_NAME}/{saved_filename}"),
        saved_filename,
        bytes_written,
    })
}

struct AttachmentSource {
    attachment_id: String,
    export_filename: String,
    content_type: AttachmentContentType,
    size_bytes: u64,
    bytes: Vec<u8>,
}

struct AttachmentRow {
    attachment_id: String,
    display_filename: String,
    canonical_mime: String,
    size_bytes: i64,
    storage_relative_path: Option<String>,
    sha256: Option<String>,
}

fn load_attachment_source(
    connection: &Connection,
    app_data_dir: &Path,
    request: AttachmentCommandRequest,
) -> Result<AttachmentSource, AttachmentCommandError> {
    validate_attachment_request(&request)?;

    let row = attachment_row(connection, &request)?;
    let relative_path = row
        .storage_relative_path
        .as_deref()
        .ok_or_else(|| AttachmentCommandError::new("attachment content is unavailable"))?;
    let expected_bytes = u64::try_from(row.size_bytes)
        .map_err(|_| AttachmentCommandError::new("attachment size is invalid"))?;
    if expected_bytes > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentSafetyError::OversizedAttachment {
            max_bytes: MAX_ATTACHMENT_BYTES,
            actual_bytes: expected_bytes,
        }
        .into());
    }

    let full_path = checked_app_data_path_for_read(app_data_dir, relative_path)?;
    let bytes = read_bounded_file(&full_path)?;
    let actual_bytes = bytes.len() as u64;
    if actual_bytes != expected_bytes {
        return Err(AttachmentCommandError::new("attachment size mismatch"));
    }

    let expected_hash = row
        .sha256
        .as_deref()
        .ok_or_else(|| AttachmentCommandError::new("attachment hash is unavailable"))?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(AttachmentCommandError::new("attachment hash mismatch"));
    }

    let content_type = validate_attachment_for_storage(AttachmentTypeEvidence {
        declared_mime: Some(&row.canonical_mime),
        sniffed_bytes: &bytes,
        byte_count: actual_bytes,
    })?;
    let export_filename = export_filename(&row.display_filename, content_type);

    Ok(AttachmentSource {
        attachment_id: row.attachment_id,
        export_filename,
        content_type,
        size_bytes: actual_bytes,
        bytes,
    })
}

fn validate_attachment_request(
    request: &AttachmentCommandRequest,
) -> Result<(), AttachmentCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("folder_id", &request.folder_id)?;
    validate_entity_id("message_id", &request.message_id)?;
    validate_entity_id("attachment_id", &request.attachment_id)?;
    if !request.user_requested {
        return Err(AttachmentSafetyError::UserIntentRequired.into());
    }
    Ok(())
}

fn attachment_row(
    connection: &Connection,
    request: &AttachmentCommandRequest,
) -> Result<AttachmentRow, AttachmentCommandError> {
    let row = connection
        .query_row(
            "SELECT
               attachments.id,
               attachments.display_filename,
               attachments.canonical_mime,
               attachments.size_bytes,
               attachments.storage_relative_path,
               attachments.sha256
             FROM attachments
             JOIN messages ON messages.account_id = attachments.account_id
              AND messages.folder_id = attachments.folder_id
              AND messages.id = attachments.message_id
             JOIN folders ON folders.account_id = messages.account_id
              AND folders.id = messages.folder_id
             JOIN accounts ON accounts.id = messages.account_id
             WHERE accounts.removed_at IS NULL
               AND accounts.auth_status != 'removed'
               AND attachments.account_id = ?1
               AND attachments.folder_id = ?2
               AND attachments.message_id = ?3
               AND attachments.id = ?4",
            params![
                request.account_id,
                request.folder_id,
                request.message_id,
                request.attachment_id
            ],
            |row| {
                Ok(AttachmentRow {
                    attachment_id: row.get(0)?,
                    display_filename: row.get(1)?,
                    canonical_mime: row.get(2)?,
                    size_bytes: row.get(3)?,
                    storage_relative_path: row.get(4)?,
                    sha256: row.get(5)?,
                })
            },
        )
        .optional()?;

    row.ok_or_else(|| AttachmentCommandError::new("attachment was not found"))
}

fn checked_app_data_path_for_read(
    app_data_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, AttachmentCommandError> {
    ensure_relative_storage_path(relative_path)?;
    let root = app_data_dir
        .canonicalize()
        .map_err(|_| AttachmentCommandError::new("attachment storage is unavailable"))?;
    let path = app_data_dir
        .join(relative_path)
        .canonicalize()
        .map_err(|_| AttachmentCommandError::new("attachment content is unavailable"))?;
    if !path.starts_with(&root) {
        return Err(AttachmentCommandError::new(
            "attachment path is outside app data",
        ));
    }
    Ok(path)
}

fn ensure_relative_storage_path(value: &str) -> Result<(), AttachmentCommandError> {
    validate_payload_size(
        "storage_relative_path",
        value.as_bytes(),
        MAX_TEXT_FIELD_BYTES,
    )?;
    if value.trim().is_empty() {
        return Err(AttachmentCommandError::new(
            "attachment content is unavailable",
        ));
    }

    let mut components = Path::new(value).components();
    let Some(Component::Normal(first)) = components.next() else {
        return Err(AttachmentCommandError::new(
            "attachment path is outside app data",
        ));
    };
    if first != OsStr::new(INBOUND_ATTACHMENT_DIR_NAME) {
        return Err(AttachmentCommandError::new(
            "attachment path is outside app data",
        ));
    }
    if components.any(|component| !matches!(component, Component::Normal(_))) {
        return Err(AttachmentCommandError::new(
            "attachment path is outside app data",
        ));
    }

    Ok(())
}

fn read_bounded_file(path: &Path) -> Result<Vec<u8>, AttachmentCommandError> {
    let file = File::open(path)
        .map_err(|_| AttachmentCommandError::new("attachment content is unavailable"))?;
    let mut reader = file.take(MAX_ATTACHMENT_BYTES + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| AttachmentCommandError::new("attachment content is unavailable"))?;
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentSafetyError::OversizedAttachment {
            max_bytes: MAX_ATTACHMENT_BYTES,
            actual_bytes: bytes.len() as u64,
        }
        .into());
    }
    Ok(bytes)
}

fn write_without_overwrite(
    root: &Path,
    filename: &str,
    bytes: &[u8],
) -> Result<(String, u64), AttachmentCommandError> {
    for attempt in 0..MAX_SAVE_COLLISION_ATTEMPTS {
        let candidate = collision_filename(filename, attempt);
        let path = root.join(&candidate);
        if path.parent() != Some(root) || !path.starts_with(root) {
            return Err(AttachmentCommandError::new(
                "attachment save destination is outside the allowed scope",
            ));
        }

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(_error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    let _ = fs::remove_file(&path);
                    return Err(AttachmentCommandError::new("attachment save failed"));
                }
                return Ok((candidate, bytes.len() as u64));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_error) => return Err(AttachmentCommandError::new("attachment save failed")),
        }
    }

    Err(AttachmentCommandError::new(
        "attachment save destination already exists",
    ))
}

fn selected_save_destination(
    app: &tauri::AppHandle,
    source: &AttachmentSource,
) -> Result<PathBuf, AttachmentCommandError> {
    let extension = source.content_type.storage_extension();
    let selected = app
        .dialog()
        .file()
        .set_title("Save attachment")
        .set_file_name(&source.export_filename)
        .add_filter(source.content_type.canonical_mime(), &[extension])
        .blocking_save_file()
        .ok_or_else(|| AttachmentCommandError::new("attachment save was cancelled"))?;

    selected
        .into_path()
        .map_err(|_| AttachmentCommandError::new("attachment save destination is unavailable"))
}

fn save_attachment_source_to_user_path(
    source: AttachmentSource,
    destination: &Path,
) -> Result<AttachmentSaveResponse, AttachmentCommandError> {
    let path = checked_user_selected_destination(destination, source.content_type)?;
    let saved_filename = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| AttachmentCommandError::new("attachment save destination is unavailable"))?
        .to_string();
    let bytes_written = write_new_file(&path, &source.bytes)?;

    Ok(AttachmentSaveResponse {
        attachment_id: source.attachment_id,
        display_filename: source.export_filename,
        content_type: source.content_type.canonical_mime().into(),
        size_bytes: source.size_bytes,
        destination_label: saved_filename.clone(),
        saved_filename,
        bytes_written,
    })
}

fn checked_user_selected_destination(
    destination: &Path,
    content_type: AttachmentContentType,
) -> Result<PathBuf, AttachmentCommandError> {
    if destination
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AttachmentCommandError::new(
            "attachment save destination is outside the allowed scope",
        ));
    }

    let mut candidate = destination.to_path_buf();
    let canonical_extension = content_type.storage_extension();
    match candidate.extension().and_then(OsStr::to_str) {
        Some(extension) if extension.eq_ignore_ascii_case(canonical_extension) => {}
        Some(_) => {
            return Err(AttachmentCommandError::new(
                "attachment save extension does not match content type",
            ));
        }
        None => {
            candidate.set_extension(canonical_extension);
        }
    }

    let file_name = candidate
        .file_name()
        .ok_or_else(|| AttachmentCommandError::new("attachment save destination is unavailable"))?;
    let parent = candidate
        .parent()
        .ok_or_else(|| AttachmentCommandError::new("attachment save destination is unavailable"))?;
    let root = parent
        .canonicalize()
        .map_err(|_| AttachmentCommandError::new("attachment save destination is unavailable"))?;
    if !root.is_dir() {
        return Err(AttachmentCommandError::new(
            "attachment save destination is unavailable",
        ));
    }

    let checked = root.join(file_name);
    if checked.parent() != Some(root.as_path()) || !checked.starts_with(&root) {
        return Err(AttachmentCommandError::new(
            "attachment save destination is outside the allowed scope",
        ));
    }

    Ok(checked)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<u64, AttachmentCommandError> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            if let Err(_error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                let _ = fs::remove_file(path);
                return Err(AttachmentCommandError::new("attachment save failed"));
            }
            Ok(bytes.len() as u64)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(
            AttachmentCommandError::new("attachment save destination already exists"),
        ),
        Err(_error) => Err(AttachmentCommandError::new("attachment save failed")),
    }
}

fn collision_filename(filename: &str, attempt: usize) -> String {
    if attempt == 0 {
        return filename.to_string();
    }

    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("attachment");
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or_default();
    if extension.is_empty() {
        format!("{stem} ({attempt})")
    } else {
        format!("{stem} ({attempt}).{extension}")
    }
}

fn export_filename(display_filename: &str, content_type: AttachmentContentType) -> String {
    let sanitized = sanitize_display_filename(Some(display_filename), content_type);
    let stem = Path::new(&sanitized)
        .file_stem()
        .and_then(OsStr::to_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("attachment");
    let bounded_stem = truncate_utf8(stem, 180);

    format!("{}.{}", bounded_stem, content_type.storage_extension())
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::new();
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, AttachmentCommandError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)
        .map_err(|_| AttachmentCommandError::new("app data directory is unavailable"))?;
    Ok(app_data_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, process};

    const ACCOUNT_ID: &str = "acct_attach";
    const OTHER_ACCOUNT_ID: &str = "acct_other";
    const FOLDER_ID: &str = "folder_inbox";
    const OTHER_FOLDER_ID: &str = "folder_other";
    const MESSAGE_ID: &str = "msg_attach";
    const ATTACHMENT_ID: &str = "att_notes";

    #[test]
    fn previews_plain_text_attachment_after_ownership_hash_and_intent_checks() {
        let connection = migrated_connection();
        let app_data_dir = temp_app_data_dir("preview");
        seed_attachment(
            &connection,
            &app_data_dir,
            AttachmentSeed {
                bytes: b"stored attachment notes".to_vec(),
                canonical_mime: "text/plain",
                display_filename: "..\\notes.exe",
                storage_relative_path: None,
                sha256: None,
            },
        );

        let preview = preview_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect("text preview succeeds");

        assert_eq!(preview.attachment_id, ATTACHMENT_ID);
        assert_eq!(preview.display_filename, "notes.txt");
        assert_eq!(preview.content_type, "text/plain");
        assert_eq!(preview.text.as_deref(), Some("stored attachment notes"));
        assert!(preview.bytes.is_none());
        assert!(!preview.auto_open);

        let no_intent = preview_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(false),
        )
        .expect_err("preview requires explicit user intent");
        assert_eq!(no_intent.reason, "attachment preview requires user intent");

        let mut wrong_folder = attachment_request(true);
        wrong_folder.folder_id = OTHER_FOLDER_ID.into();
        let unowned = preview_attachment_from_connection(&connection, &app_data_dir, wrong_folder)
            .expect_err("unowned folder scope is rejected");
        assert_eq!(unowned.reason, "attachment was not found");

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn rejects_corrupt_hash_mismatched_type_unavailable_content_and_path_traversal() {
        let connection = migrated_connection();
        let app_data_dir = temp_app_data_dir("reject");
        seed_attachment(
            &connection,
            &app_data_dir,
            AttachmentSeed {
                bytes: b"safe notes".to_vec(),
                canonical_mime: "text/plain",
                display_filename: "notes.txt",
                storage_relative_path: None,
                sha256: Some("0000000000000000000000000000000000000000000000000000000000000000"),
            },
        );
        let corrupt = download_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect_err("corrupt hash is rejected");
        assert_eq!(corrupt.reason, "attachment hash mismatch");

        connection
            .execute(
                "UPDATE attachments
                 SET sha256 = ?1, canonical_mime = 'image/png'
                 WHERE id = ?2",
                params![sha256_hex(b"safe notes"), ATTACHMENT_ID],
            )
            .unwrap();
        let mismatch = download_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect_err("declared/sniffed mismatch is rejected");
        assert_eq!(
            mismatch.reason,
            "attachment declared type does not match detected type"
        );

        connection
            .execute(
                "UPDATE attachments
                 SET canonical_mime = 'text/plain',
                     storage_relative_path = '../outside.txt'
                 WHERE id = ?1",
                [ATTACHMENT_ID],
            )
            .unwrap();
        let traversal = download_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect_err("path traversal is rejected");
        assert_eq!(traversal.reason, "attachment path is outside app data");

        connection
            .execute(
                "UPDATE attachments
                 SET storage_relative_path = NULL
                 WHERE id = ?1",
                [ATTACHMENT_ID],
            )
            .unwrap();
        let unavailable = download_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect_err("metadata-only attachments are not exported");
        assert_eq!(unavailable.reason, "attachment content is unavailable");

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn rejects_unpreviewable_attachment_types() {
        let connection = migrated_connection();
        let app_data_dir = temp_app_data_dir("pdf-preview");
        seed_attachment(
            &connection,
            &app_data_dir,
            AttachmentSeed {
                bytes: b"%PDF-1.7\nbody".to_vec(),
                canonical_mime: "application/pdf",
                display_filename: "invoice.pdf",
                storage_relative_path: None,
                sha256: None,
            },
        );

        let error = preview_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect_err("PDF preview is denied");
        assert_eq!(error.reason, "attachment type is not previewable");

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn downloads_attachment_bytes_without_exposing_storage_paths() {
        let connection = migrated_connection();
        let app_data_dir = temp_app_data_dir("download");
        seed_attachment(
            &connection,
            &app_data_dir,
            AttachmentSeed {
                bytes: b"\x89PNG\r\n\x1A\nimage".to_vec(),
                canonical_mime: "image/png",
                display_filename: "..\\calc.exe",
                storage_relative_path: None,
                sha256: None,
            },
        );

        let response = download_attachment_from_connection(
            &connection,
            &app_data_dir,
            attachment_request(true),
        )
        .expect("download returns bytes");
        assert_eq!(response.display_filename, "calc.png");
        assert_eq!(response.content_type, "image/png");
        assert_eq!(response.bytes, b"\x89PNG\r\n\x1A\nimage");

        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("storage_relative_path"));
        assert!(!json.contains("attachments/acct_attach"));
        assert!(!json.contains(&app_data_dir.to_string_lossy().to_string()));

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn saves_to_download_scope_without_overwrite_or_sender_filename_paths() {
        let connection = migrated_connection();
        let app_data_dir = temp_app_data_dir("save-source");
        let save_root = temp_app_data_dir("save-destination");
        let outside_sentinel = save_root.with_file_name(format!(
            "{}-outside.txt",
            save_root.file_name().unwrap().to_string_lossy()
        ));
        fs::write(&outside_sentinel, b"do not touch").unwrap();
        seed_attachment(
            &connection,
            &app_data_dir,
            AttachmentSeed {
                bytes: b"safe notes".to_vec(),
                canonical_mime: "text/plain",
                display_filename: "..\\..\\report.exe",
                storage_relative_path: None,
                sha256: None,
            },
        );

        let first = save_attachment_from_connection(
            &connection,
            &app_data_dir,
            &save_root,
            attachment_request(true),
        )
        .expect("first save succeeds");
        let second = save_attachment_from_connection(
            &connection,
            &app_data_dir,
            &save_root,
            attachment_request(true),
        )
        .expect("second save picks a collision filename");

        assert_eq!(first.saved_filename, "report.txt");
        assert_eq!(first.destination_label, "Komorebi/report.txt");
        assert_eq!(first.bytes_written, 10);
        assert_eq!(second.saved_filename, "report (1).txt");
        assert_eq!(
            fs::read(save_root.join("report.txt")).unwrap(),
            b"safe notes"
        );
        assert_eq!(
            fs::read(save_root.join("report (1).txt")).unwrap(),
            b"safe notes"
        );
        assert_eq!(fs::read(&outside_sentinel).unwrap(), b"do not touch");

        fs::remove_file(outside_sentinel).unwrap();
        fs::remove_dir_all(app_data_dir).unwrap();
        fs::remove_dir_all(save_root).unwrap();
    }

    #[test]
    fn saves_to_user_selected_destination_without_overwrite_or_extension_spoofing() {
        let connection = migrated_connection();
        let app_data_dir = temp_app_data_dir("selected-save-source");
        let save_root = temp_app_data_dir("selected-save-destination");
        seed_attachment(
            &connection,
            &app_data_dir,
            AttachmentSeed {
                bytes: b"selected notes".to_vec(),
                canonical_mime: "text/plain",
                display_filename: "..\\..\\report.exe",
                storage_relative_path: None,
                sha256: None,
            },
        );
        let source = load_attachment_source(&connection, &app_data_dir, attachment_request(true))
            .expect("source loads");

        let response = save_attachment_source_to_user_path(source, &save_root.join("chosen-name"))
            .expect("missing extension gets canonical extension");
        assert_eq!(response.saved_filename, "chosen-name.txt");
        assert_eq!(response.destination_label, "chosen-name.txt");
        assert_eq!(
            fs::read(save_root.join("chosen-name.txt")).unwrap(),
            b"selected notes"
        );

        let source = load_attachment_source(&connection, &app_data_dir, attachment_request(true))
            .expect("source reloads");
        let overwrite =
            save_attachment_source_to_user_path(source, &save_root.join("chosen-name.txt"))
                .expect_err("selected save never overwrites");
        assert_eq!(
            overwrite.reason,
            "attachment save destination already exists"
        );

        let source = load_attachment_source(&connection, &app_data_dir, attachment_request(true))
            .expect("source reloads");
        let spoofed =
            save_attachment_source_to_user_path(source, &save_root.join("chosen-name.exe"))
                .expect_err("spoofed extension is rejected");
        assert_eq!(
            spoofed.reason,
            "attachment save extension does not match content type"
        );

        fs::remove_dir_all(app_data_dir).unwrap();
        fs::remove_dir_all(save_root).unwrap();
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
    }

    struct AttachmentSeed<'a> {
        bytes: Vec<u8>,
        canonical_mime: &'a str,
        display_filename: &'a str,
        storage_relative_path: Option<&'a str>,
        sha256: Option<&'a str>,
    }

    fn seed_attachment(connection: &Connection, app_data_dir: &Path, seed: AttachmentSeed<'_>) {
        seed_accounts_folders_and_message(connection);
        let relative_path = seed
            .storage_relative_path
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{INBOUND_ATTACHMENT_DIR_NAME}/{ACCOUNT_ID}/{FOLDER_ID}/{MESSAGE_ID}/{ATTACHMENT_ID}.{}",
                    validate_attachment_for_storage(AttachmentTypeEvidence {
                        declared_mime: Some(seed.canonical_mime),
                        sniffed_bytes: &seed.bytes,
                        byte_count: seed.bytes.len() as u64,
                    })
                    .unwrap()
                    .storage_extension()
                )
            });
        let full_path = app_data_dir.join(&relative_path);
        fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        fs::write(&full_path, &seed.bytes).unwrap();
        let sha256 = seed
            .sha256
            .map(str::to_string)
            .unwrap_or_else(|| sha256_hex(&seed.bytes));

        connection
            .execute(
                "INSERT INTO attachments (
                   id,
                   account_id,
                   folder_id,
                   message_id,
                   content_id,
                   display_filename,
                   canonical_mime,
                   content_type,
                   size_bytes,
                   storage_relative_path,
                   sha256,
                   disposition
                 )
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?6, ?7, ?8, ?9, 'attachment')",
                params![
                    ATTACHMENT_ID,
                    ACCOUNT_ID,
                    FOLDER_ID,
                    MESSAGE_ID,
                    seed.display_filename,
                    seed.canonical_mime,
                    seed.bytes.len() as i64,
                    relative_path,
                    sha256
                ],
            )
            .unwrap();
    }

    fn seed_accounts_folders_and_message(connection: &Connection) {
        for (account_id, email) in [
            (ACCOUNT_ID, "attachments@example.com"),
            (OTHER_ACCOUNT_ID, "other@example.com"),
        ] {
            connection
                .execute(
                    "INSERT OR IGNORE INTO accounts (
                       id,
                       provider_kind,
                       display_email,
                       display_name,
                       username,
                       auth_kind,
                       credential_ref,
                       imap_host,
                       imap_port,
                       imap_security,
                       smtp_host,
                       smtp_port,
                       smtp_security,
                       auth_status
                     )
                     VALUES (?1, 'generic_imap_smtp', ?2, ?2, ?2, 'app_password', ?3, 'imap.example.com', 993, 'tls', 'smtp.example.com', 587, 'starttls', 'connected')",
                    params![account_id, email, format!("cred_{account_id}")],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT OR IGNORE INTO folders (
                   id,
                   account_id,
                   remote_id,
                   name,
                   display_name,
                   kind,
                   uid_validity,
                   uid_next,
                   sync_state
                 )
                 VALUES (?1, ?2, ?1, ?1, ?1, 'inbox', 1, 2, 'idle')",
                params![FOLDER_ID, ACCOUNT_ID],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO folders (
                   id,
                   account_id,
                   remote_id,
                   name,
                   display_name,
                   kind,
                   uid_validity,
                   uid_next,
                   sync_state
                 )
                 VALUES (?1, ?2, ?1, ?1, ?1, 'inbox', 1, 2, 'idle')",
                params![OTHER_FOLDER_ID, OTHER_ACCOUNT_ID],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO messages (
                   id,
                   account_id,
                   folder_id,
                   imap_uid,
                   uid_validity,
                   subject,
                   sender_email,
                   body_text
                 )
                 VALUES (?1, ?2, ?3, 1, 1, 'Attachment message', 'sender@example.com', 'Body')",
                params![MESSAGE_ID, ACCOUNT_ID, FOLDER_ID],
            )
            .unwrap();
    }

    fn attachment_request(user_requested: bool) -> AttachmentCommandRequest {
        AttachmentCommandRequest {
            account_id: ACCOUNT_ID.into(),
            folder_id: FOLDER_ID.into(),
            message_id: MESSAGE_ID.into(),
            attachment_id: ATTACHMENT_ID.into(),
            user_requested,
        }
    }

    fn temp_app_data_dir(label: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("komorebi-attachment-{label}-{}", process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
