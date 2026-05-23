use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::{
    attachment_safety::{
        sanitize_display_filename, validate_attachment_for_storage, AttachmentContentType,
        AttachmentSafetyError, AttachmentTypeEvidence, PreviewKind,
    },
    credentials::WindowsCredentialManager,
    draft_persistence::{
        autosave_draft, discard_draft, ensure_active_draft, ComposerDraftRecord,
        DiscardDraftRequest, DraftAutosaveRequest, DraftHtml, DraftPersistenceError,
        SentFolderPolicy, SentFolderSyncState,
    },
    ipc_validation::{
        validate_bounded_text, validate_entity_id, validate_payload_size, IpcValidationError,
        MAX_TEXT_FIELD_BYTES,
    },
    oauth::ProviderOAuthTokenClient,
    schema::{apply_migrations, SchemaMigrationError},
    smtp_send::{
        send_email, AttachmentContentStore, BackendSmtpAuthResolver, LettreSmtpSender,
        OutboundAttachmentDisposition, OutboundAttachmentMetadata, OutboundAttachmentRef,
        OutboundEmailAddress, SendEmailError, SendEmailReport, SendEmailRequest, SendProgressEvent,
        SmtpAccountStore, SmtpAuthResolver, SmtpSender, SqliteSmtpAccountStore,
        ATTACHMENT_STREAM_CHUNK_BYTES,
    },
    token_lifecycle::{
        AccessTokenCache, AccountDataError, AccountDataStore, CachedAccessToken, ReconnectReason,
        TokenLifecycle,
    },
};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const COMPOSER_ATTACHMENT_DIR_NAME: &str = "composer-attachments";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerDraftSaveRequest {
    pub account_id: String,
    pub draft_id: String,
    pub subject: String,
    pub to: Vec<ComposerAddress>,
    pub cc: Vec<ComposerAddress>,
    pub bcc: Vec<ComposerAddress>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerDraftDiscardRequest {
    pub account_id: String,
    pub draft_id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerAttachmentStageRequest {
    pub account_id: String,
    pub draft_id: String,
    pub display_filename: String,
    pub content_type: Option<String>,
    pub disposition: ComposerAttachmentDisposition,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ComposerAttachmentDisposition {
    Attachment,
    Inline,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerSendRequest {
    pub account_id: String,
    pub draft_id: String,
    pub subject: String,
    pub to: Vec<ComposerAddress>,
    pub cc: Vec<ComposerAddress>,
    pub bcc: Vec<ComposerAddress>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerDraftView {
    pub id: String,
    pub account_id: String,
    pub autosave_revision: u64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerAttachmentView {
    pub id: String,
    pub account_id: String,
    pub draft_id: String,
    pub display_filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub disposition: ComposerAttachmentDisposition,
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ComposerSendProgress {
    AccountResolved,
    DraftPrepared,
    AuthResolved,
    AttachmentStreamed,
    MimeComposed,
    SmtpAccepted,
    SentStateRecorded,
    DraftFinalized,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerSendResponse {
    pub account_id: String,
    pub draft_id: String,
    pub rfc_message_id: String,
    pub sent_folder_policy: String,
    pub sent_folder_sync_state: String,
    pub provider_response: Option<String>,
    pub message_size_bytes: u64,
    pub attachment_bytes_streamed: u64,
    pub progress: Vec<ComposerSendProgress>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentRecipientSearchRequest {
    pub account_id: String,
    pub query: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentRecipientView {
    pub id: String,
    pub account_id: String,
    pub name: Option<String>,
    pub address: String,
    pub use_count: u32,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComposerCommandError {
    pub reason: String,
}

impl ComposerCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ComposerCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for ComposerCommandError {}

impl From<rusqlite::Error> for ComposerCommandError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<std::io::Error> for ComposerCommandError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<tauri::Error> for ComposerCommandError {
    fn from(value: tauri::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SchemaMigrationError> for ComposerCommandError {
    fn from(value: SchemaMigrationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<IpcValidationError> for ComposerCommandError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<DraftPersistenceError> for ComposerCommandError {
    fn from(value: DraftPersistenceError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AttachmentSafetyError> for ComposerCommandError {
    fn from(value: AttachmentSafetyError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SendEmailError> for ComposerCommandError {
    fn from(value: SendEmailError) -> Self {
        Self::new(value.to_string())
    }
}

#[tauri::command]
pub fn autosave_composer_draft(
    app: tauri::AppHandle,
    request: ComposerDraftSaveRequest,
) -> Result<ComposerDraftView, ComposerCommandError> {
    let cache_path = mail_cache_path(&app)?;
    autosave_composer_draft_from_path(&cache_path, request)
}

#[tauri::command]
pub fn discard_composer_draft(
    app: tauri::AppHandle,
    request: ComposerDraftDiscardRequest,
) -> Result<ComposerDraftView, ComposerCommandError> {
    let app_data_dir = app_data_dir(&app)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    discard_composer_draft_from_path(&cache_path, &app_data_dir, request)
}

#[tauri::command]
pub fn stage_composer_attachment(
    app: tauri::AppHandle,
    request: ComposerAttachmentStageRequest,
) -> Result<ComposerAttachmentView, ComposerCommandError> {
    let app_data_dir = app_data_dir(&app)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    stage_composer_attachment_from_path(&cache_path, &app_data_dir, request)
}

#[tauri::command]
pub async fn send_composer_email(
    app: tauri::AppHandle,
    request: ComposerSendRequest,
) -> Result<ComposerSendResponse, ComposerCommandError> {
    let app_data_dir = app_data_dir(&app)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);

    tauri::async_runtime::spawn_blocking(move || {
        send_composer_email_from_path(&cache_path, &app_data_dir, request)
    })
    .await
    .map_err(|error| ComposerCommandError::new(error.to_string()))?
}

#[tauri::command]
pub fn list_recent_recipients(
    app: tauri::AppHandle,
    request: RecentRecipientSearchRequest,
) -> Result<Vec<RecentRecipientView>, ComposerCommandError> {
    let cache_path = mail_cache_path(&app)?;
    list_recent_recipients_from_path(&cache_path, request)
}

pub fn autosave_composer_draft_from_path(
    cache_path: &Path,
    request: ComposerDraftSaveRequest,
) -> Result<ComposerDraftView, ComposerCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    autosave_composer_draft_from_connection(&connection, request)
}

pub fn autosave_composer_draft_from_connection(
    connection: &Connection,
    request: ComposerDraftSaveRequest,
) -> Result<ComposerDraftView, ComposerCommandError> {
    ensure_composer_attachments_exist(
        connection,
        &request.account_id,
        &request.draft_id,
        &request.attachment_ids,
    )?;
    let record = autosave_draft(connection, draft_autosave_request(request)?)?;
    Ok(draft_view(record))
}

pub fn discard_composer_draft_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    request: ComposerDraftDiscardRequest,
) -> Result<ComposerDraftView, ComposerCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    discard_composer_draft_from_connection(&connection, app_data_dir, request)
}

pub fn discard_composer_draft_from_connection(
    connection: &Connection,
    app_data_dir: &Path,
    request: ComposerDraftDiscardRequest,
) -> Result<ComposerDraftView, ComposerCommandError> {
    let staged_files = staged_attachment_paths(
        connection,
        app_data_dir,
        &request.account_id,
        &request.draft_id,
    )?;
    let record = discard_draft(
        connection,
        DiscardDraftRequest {
            account_id: request.account_id.clone(),
            draft_id: request.draft_id.clone(),
        },
    )?;
    delete_staged_attachment_rows(connection, &request.account_id, &request.draft_id)?;
    delete_files_best_effort(staged_files);

    Ok(draft_view(record))
}

pub fn stage_composer_attachment_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    request: ComposerAttachmentStageRequest,
) -> Result<ComposerAttachmentView, ComposerCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    stage_composer_attachment_from_connection(&connection, app_data_dir, request)
}

pub fn stage_composer_attachment_from_connection(
    connection: &Connection,
    app_data_dir: &Path,
    request: ComposerAttachmentStageRequest,
) -> Result<ComposerAttachmentView, ComposerCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("draft_id", &request.draft_id)?;
    ensure_active_draft(connection, &request.account_id, &request.draft_id)?;
    validate_payload_size("display_filename", request.display_filename.as_bytes(), 255)?;

    let byte_count = request.bytes.len() as u64;
    let content_type = validate_attachment_for_storage(AttachmentTypeEvidence {
        declared_mime: request.content_type.as_deref(),
        sniffed_bytes: &request.bytes,
        byte_count,
    })?;
    if request.disposition == ComposerAttachmentDisposition::Inline
        && content_type.preview_kind() != Some(PreviewKind::RasterImage)
    {
        return Err(ComposerCommandError::new(
            "inline composer attachments must be raster images",
        ));
    }

    let display_filename = sanitize_display_filename(Some(&request.display_filename), content_type);
    let digest = sha256_hex(&request.bytes);
    let attachment_id = stable_composer_attachment_id(
        &request.account_id,
        &request.draft_id,
        &display_filename,
        &digest,
        request.disposition,
    );
    let content_id = (request.disposition == ComposerAttachmentDisposition::Inline)
        .then(|| format!("{}@komorebi.local", attachment_id));
    let relative_path = composer_attachment_relative_path(
        &request.account_id,
        &request.draft_id,
        &attachment_id,
        content_type,
    )?;
    let storage_path = checked_app_data_path_for_write(app_data_dir, &relative_path)?;
    if let Some(parent) = storage_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&storage_path, &request.bytes)?;

    connection.execute(
        "INSERT INTO composer_attachments (
           id,
           account_id,
           draft_id,
           disposition,
           content_id,
           display_filename,
           canonical_mime,
           size_bytes,
           storage_relative_path,
           sha256
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(account_id, draft_id, id) DO UPDATE SET
           disposition = excluded.disposition,
           content_id = excluded.content_id,
           display_filename = excluded.display_filename,
           canonical_mime = excluded.canonical_mime,
           size_bytes = excluded.size_bytes,
           storage_relative_path = excluded.storage_relative_path,
           sha256 = excluded.sha256,
           updated_at = CURRENT_TIMESTAMP",
        params![
            &attachment_id,
            &request.account_id,
            &request.draft_id,
            composer_disposition_schema_value(request.disposition),
            &content_id,
            &display_filename,
            content_type.canonical_mime(),
            u64_to_i64(byte_count, "attachment size")?,
            relative_path,
            digest,
        ],
    )?;

    Ok(ComposerAttachmentView {
        id: attachment_id,
        account_id: request.account_id,
        draft_id: request.draft_id,
        display_filename,
        content_type: content_type.canonical_mime().to_string(),
        size_bytes: byte_count,
        disposition: request.disposition,
        content_id,
    })
}

pub fn send_composer_email_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    request: ComposerSendRequest,
) -> Result<ComposerSendResponse, ComposerCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    let mut account_store = SqliteSmtpAccountStore::new(&connection);
    let token_lifecycle = TokenLifecycle::new(
        WindowsCredentialManager,
        InMemoryAccessTokenCache::default(),
        ProviderOAuthTokenClient::new(Vec::new()),
        SqliteTokenAccountDataStore::new(&connection),
    );
    let mut auth_resolver =
        BackendSmtpAuthResolver::new(WindowsCredentialManager, token_lifecycle, unix_now());
    let mut attachments = ComposerAttachmentStore::new(&connection, app_data_dir);
    let mut smtp_sender = LettreSmtpSender;

    send_composer_email_with_runtime(
        &connection,
        app_data_dir,
        request,
        &mut account_store,
        &mut auth_resolver,
        &mut attachments,
        &mut smtp_sender,
    )
}

pub fn list_recent_recipients_from_path(
    cache_path: &Path,
    request: RecentRecipientSearchRequest,
) -> Result<Vec<RecentRecipientView>, ComposerCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    list_recent_recipients_from_connection(&connection, request)
}

pub fn send_composer_email_with_runtime<A, R, C, T>(
    connection: &Connection,
    app_data_dir: &Path,
    request: ComposerSendRequest,
    account_store: &mut A,
    auth_resolver: &mut R,
    attachment_store: &mut C,
    smtp_sender: &mut T,
) -> Result<ComposerSendResponse, ComposerCommandError>
where
    A: SmtpAccountStore,
    R: SmtpAuthResolver,
    C: AttachmentContentStore,
    T: SmtpSender,
{
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("draft_id", &request.draft_id)?;
    ensure_active_draft(connection, &request.account_id, &request.draft_id)?;
    ensure_composer_attachments_exist(
        connection,
        &request.account_id,
        &request.draft_id,
        &request.attachment_ids,
    )?;
    autosave_composer_draft_from_connection(
        connection,
        ComposerDraftSaveRequest {
            account_id: request.account_id.clone(),
            draft_id: request.draft_id.clone(),
            subject: request.subject.clone(),
            to: request.to.clone(),
            cc: request.cc.clone(),
            bcc: request.bcc.clone(),
            body_text: request.body_text.clone(),
            body_html: request.body_html.clone(),
            attachment_ids: request.attachment_ids.clone(),
        },
    )?;

    let staged_files = staged_attachment_paths(
        connection,
        app_data_dir,
        &request.account_id,
        &request.draft_id,
    )?;
    let draft_id = request.draft_id.clone();
    let account_id = request.account_id.clone();
    let outbound_request = send_email_request(request)?;
    let recent_recipients = outbound_request
        .to
        .iter()
        .chain(outbound_request.cc.iter())
        .chain(outbound_request.bcc.iter())
        .cloned()
        .collect::<Vec<_>>();
    let report = send_email(
        outbound_request,
        account_store,
        auth_resolver,
        attachment_store,
        smtp_sender,
    )?;
    let _ = upsert_recent_recipients(connection, &account_id, &recent_recipients);
    delete_staged_attachment_rows(connection, &account_id, &draft_id)?;
    delete_files_best_effort(staged_files);

    Ok(send_response(draft_id, report))
}

pub fn list_recent_recipients_from_connection(
    connection: &Connection,
    request: RecentRecipientSearchRequest,
) -> Result<Vec<RecentRecipientView>, ComposerCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_payload_size("recipient_query", request.query.as_bytes(), 256)?;
    ensure_account_exists(connection, &request.account_id)?;

    let normalized_query = request.query.trim().to_ascii_lowercase();
    let escaped_query = escape_like_pattern(&normalized_query);
    let like_pattern = format!("%{escaped_query}%");
    let limit = request.limit.unwrap_or(8).clamp(1, 20);
    let mut statement = connection.prepare(
        "SELECT
           id,
           account_id,
           display_name,
           email,
           use_count,
           last_used_at
         FROM recent_recipients
         WHERE account_id = ?1
           AND (
             ?2 = ''
             OR normalized_email LIKE ?3 ESCAPE '\\'
             OR lower(coalesce(display_name, '')) LIKE ?3 ESCAPE '\\'
           )
         ORDER BY last_used_at DESC, use_count DESC, normalized_email
         LIMIT ?4",
    )?;
    let rows = statement.query_map(
        params![
            &request.account_id,
            normalized_query,
            like_pattern,
            i64::from(limit)
        ],
        |row| {
            let use_count = row.get::<_, i64>(4)?;
            Ok(RecentRecipientView {
                id: row.get(0)?,
                account_id: row.get(1)?,
                name: row.get(2)?,
                address: row.get(3)?,
                use_count: u32::try_from(use_count).unwrap_or(u32::MAX),
                last_used_at: row.get(5)?,
            })
        },
    )?;

    let mut recipients = Vec::new();
    for row in rows {
        recipients.push(row?);
    }
    Ok(recipients)
}

fn mail_cache_path(app: &tauri::AppHandle) -> Result<PathBuf, ComposerCommandError> {
    Ok(app_data_dir(app)?.join(MAIL_CACHE_FILE_NAME))
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, ComposerCommandError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    Ok(app_data_dir)
}

fn draft_autosave_request(
    request: ComposerDraftSaveRequest,
) -> Result<DraftAutosaveRequest, ComposerCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("draft_id", &request.draft_id)?;
    validate_payload_size("subject", request.subject.as_bytes(), MAX_TEXT_FIELD_BYTES)?;

    let body_text = clean_optional_text(request.body_text)?;
    let body_html = clean_optional_html_for_draft(request.body_html)?;
    let recipients = deduplicate_recipient_fields(request.to, request.cc, request.bcc);

    Ok(DraftAutosaveRequest {
        account_id: request.account_id,
        draft_id: request.draft_id,
        subject: request.subject,
        to_addresses_json: serialize_composer_addresses(&recipients.to)?,
        cc_addresses_json: serialize_composer_addresses(&recipients.cc)?,
        bcc_addresses_json: serialize_composer_addresses(&recipients.bcc)?,
        reply_to_addresses_json: "[]".to_string(),
        body_text,
        body_html,
        attachment_ids_json: serialize_attachment_ids(&request.attachment_ids)?,
        in_reply_to: None,
        references_json: "[]".to_string(),
    })
}

fn send_email_request(
    request: ComposerSendRequest,
) -> Result<SendEmailRequest, ComposerCommandError> {
    let recipients = deduplicate_recipient_fields(request.to, request.cc, request.bcc);

    Ok(SendEmailRequest {
        account_id: request.account_id,
        draft_id: Some(request.draft_id),
        to: outbound_addresses(recipients.to)?,
        cc: outbound_addresses(recipients.cc)?,
        bcc: outbound_addresses(recipients.bcc)?,
        reply_to: Vec::new(),
        subject: request.subject,
        body_text: clean_optional_text(request.body_text)?,
        body_html: clean_optional_html_for_send(request.body_html)?,
        attachments: outbound_attachment_refs(request.attachment_ids)?,
        message_id: None,
    })
}

fn clean_optional_text(value: Option<String>) -> Result<Option<String>, ComposerCommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    validate_payload_size("body_text", value.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn clean_optional_html_for_draft(
    value: Option<String>,
) -> Result<Option<DraftHtml>, ComposerCommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if html_is_empty(&value) {
        return Ok(None);
    }
    Ok(Some(DraftHtml::new_sanitized(value)?))
}

fn clean_optional_html_for_send(
    value: Option<String>,
) -> Result<Option<crate::smtp_send::SanitizedHtml>, ComposerCommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if html_is_empty(&value) {
        return Ok(None);
    }
    Ok(Some(crate::smtp_send::SanitizedHtml::new_sanitized(value)?))
}

fn html_is_empty(value: &str) -> bool {
    let without_tags = value
        .replace("<p></p>", "")
        .replace("<p><br></p>", "")
        .replace("<br>", "")
        .replace("&nbsp;", " ");
    without_tags.trim().is_empty()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RecipientFields {
    to: Vec<ComposerAddress>,
    cc: Vec<ComposerAddress>,
    bcc: Vec<ComposerAddress>,
}

fn deduplicate_recipient_fields(
    to: Vec<ComposerAddress>,
    cc: Vec<ComposerAddress>,
    bcc: Vec<ComposerAddress>,
) -> RecipientFields {
    let mut seen = HashSet::new();

    RecipientFields {
        to: deduplicate_recipient_list(to, &mut seen),
        cc: deduplicate_recipient_list(cc, &mut seen),
        bcc: deduplicate_recipient_list(bcc, &mut seen),
    }
}

fn deduplicate_recipient_list(
    addresses: Vec<ComposerAddress>,
    seen: &mut HashSet<String>,
) -> Vec<ComposerAddress> {
    let mut deduplicated = Vec::new();
    for address in addresses {
        let normalized = normalize_email_for_recipient_store(&address.address);
        if seen.insert(normalized) {
            deduplicated.push(address);
        }
    }
    deduplicated
}

fn serialize_composer_addresses(
    addresses: &[ComposerAddress],
) -> Result<String, ComposerCommandError> {
    for address in addresses {
        validate_payload_size(
            "email_address",
            address.address.as_bytes(),
            MAX_TEXT_FIELD_BYTES,
        )?;
        if let Some(name) = &address.name {
            validate_payload_size("email_name", name.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
        }
    }
    serde_json::to_string(addresses).map_err(|error| ComposerCommandError::new(error.to_string()))
}

fn outbound_addresses(
    addresses: Vec<ComposerAddress>,
) -> Result<Vec<OutboundEmailAddress>, ComposerCommandError> {
    addresses
        .into_iter()
        .map(|address| OutboundEmailAddress::new(address.name, address.address).map_err(Into::into))
        .collect()
}

fn outbound_attachment_refs(
    attachment_ids: Vec<String>,
) -> Result<Vec<OutboundAttachmentRef>, ComposerCommandError> {
    attachment_ids
        .into_iter()
        .map(|attachment_id| OutboundAttachmentRef::new(attachment_id).map_err(Into::into))
        .collect()
}

fn serialize_attachment_ids(ids: &[String]) -> Result<String, ComposerCommandError> {
    for id in ids {
        validate_entity_id("attachment_id", id)?;
    }
    serde_json::to_string(ids).map_err(|error| ComposerCommandError::new(error.to_string()))
}

fn ensure_composer_attachments_exist(
    connection: &Connection,
    account_id: &str,
    draft_id: &str,
    attachment_ids: &[String],
) -> Result<(), ComposerCommandError> {
    validate_entity_id("account_id", account_id)?;
    validate_entity_id("draft_id", draft_id)?;
    for attachment_id in attachment_ids {
        validate_entity_id("attachment_id", attachment_id)?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(
               SELECT 1
               FROM composer_attachments
               WHERE account_id = ?1 AND draft_id = ?2 AND id = ?3
             )",
            params![account_id, draft_id, attachment_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(ComposerCommandError::new(
                "composer attachment was not found",
            ));
        }
    }
    Ok(())
}

fn composer_attachment_relative_path(
    account_id: &str,
    draft_id: &str,
    attachment_id: &str,
    content_type: AttachmentContentType,
) -> Result<String, ComposerCommandError> {
    validate_entity_id("account_id", account_id)?;
    validate_entity_id("draft_id", draft_id)?;
    validate_entity_id("attachment_id", attachment_id)?;

    Ok(format!(
        "{COMPOSER_ATTACHMENT_DIR_NAME}/{account_id}/{draft_id}/{attachment_id}.{}",
        content_type.storage_extension()
    ))
}

fn checked_app_data_path_for_write(
    app_data_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, ComposerCommandError> {
    ensure_relative_storage_path(relative_path)?;
    Ok(app_data_dir.join(relative_path))
}

fn checked_app_data_path_for_read(
    app_data_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, ComposerCommandError> {
    ensure_relative_storage_path(relative_path)?;
    let root = app_data_dir.canonicalize()?;
    let path = app_data_dir.join(relative_path).canonicalize()?;
    if !path.starts_with(&root) {
        return Err(ComposerCommandError::new(
            "composer attachment path is outside app data",
        ));
    }
    Ok(path)
}

fn ensure_relative_storage_path(value: &str) -> Result<(), ComposerCommandError> {
    validate_bounded_text("storage_relative_path", value, MAX_TEXT_FIELD_BYTES)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ComposerCommandError::new(
            "composer attachment path is outside app data",
        ));
    }
    Ok(())
}

fn staged_attachment_paths(
    connection: &Connection,
    app_data_dir: &Path,
    account_id: &str,
    draft_id: &str,
) -> Result<Vec<PathBuf>, ComposerCommandError> {
    validate_entity_id("account_id", account_id)?;
    validate_entity_id("draft_id", draft_id)?;
    let mut statement = connection.prepare(
        "SELECT storage_relative_path
         FROM composer_attachments
         WHERE account_id = ?1 AND draft_id = ?2",
    )?;
    let rows = statement.query_map(params![account_id, draft_id], |row| row.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for row in rows {
        let relative_path = row?;
        if let Ok(path) = checked_app_data_path_for_read(app_data_dir, &relative_path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn delete_staged_attachment_rows(
    connection: &Connection,
    account_id: &str,
    draft_id: &str,
) -> Result<(), ComposerCommandError> {
    connection.execute(
        "DELETE FROM composer_attachments WHERE account_id = ?1 AND draft_id = ?2",
        params![account_id, draft_id],
    )?;
    Ok(())
}

fn delete_files_best_effort(paths: Vec<PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn stable_composer_attachment_id(
    account_id: &str,
    draft_id: &str,
    display_filename: &str,
    digest: &str,
    disposition: ComposerAttachmentDisposition,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update([0]);
    hasher.update(draft_id.as_bytes());
    hasher.update([0]);
    hasher.update(display_filename.as_bytes());
    hasher.update([0]);
    hasher.update(digest.as_bytes());
    hasher.update([0]);
    hasher.update(composer_disposition_schema_value(disposition).as_bytes());
    let hash = hasher.finalize();
    let mut id = String::from("catt_");
    for byte in hash.iter().take(16) {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::new();
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn composer_disposition_schema_value(disposition: ComposerAttachmentDisposition) -> &'static str {
    match disposition {
        ComposerAttachmentDisposition::Attachment => "attachment",
        ComposerAttachmentDisposition::Inline => "inline",
    }
}

fn composer_disposition_from_schema(value: &str) -> ComposerAttachmentDisposition {
    match value {
        "inline" => ComposerAttachmentDisposition::Inline,
        _ => ComposerAttachmentDisposition::Attachment,
    }
}

fn outbound_disposition(
    disposition: ComposerAttachmentDisposition,
) -> OutboundAttachmentDisposition {
    match disposition {
        ComposerAttachmentDisposition::Attachment => OutboundAttachmentDisposition::Attachment,
        ComposerAttachmentDisposition::Inline => OutboundAttachmentDisposition::Inline,
    }
}

fn ensure_account_exists(
    connection: &Connection,
    account_id: &str,
) -> Result<(), ComposerCommandError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(
           SELECT 1
           FROM accounts
           WHERE id = ?1 AND removed_at IS NULL
         )",
        [account_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(ComposerCommandError::from(
            IpcValidationError::UnknownAccount,
        ));
    }

    Ok(())
}

fn upsert_recent_recipients(
    connection: &Connection,
    account_id: &str,
    recipients: &[OutboundEmailAddress],
) -> Result<(), ComposerCommandError> {
    validate_entity_id("account_id", account_id)?;
    let mut seen = HashSet::new();

    for recipient in recipients {
        let normalized_email = normalize_email_for_recipient_store(&recipient.address);
        if !seen.insert(normalized_email.clone()) {
            continue;
        }
        let recipient_id = stable_recent_recipient_id(account_id, &normalized_email);
        let display_name = recipient
            .name
            .as_ref()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty());
        validate_payload_size(
            "email_address",
            recipient.address.as_bytes(),
            MAX_TEXT_FIELD_BYTES,
        )?;
        if let Some(name) = display_name {
            validate_payload_size("email_name", name.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
        }

        connection.execute(
            "INSERT INTO recent_recipients (
               id,
               account_id,
               display_name,
               email,
               normalized_email,
               source
             )
             VALUES (?1, ?2, ?3, ?4, ?5, 'composer_send')
             ON CONFLICT(account_id, normalized_email) DO UPDATE SET
               display_name = coalesce(excluded.display_name, recent_recipients.display_name),
               email = excluded.email,
               source = excluded.source,
               use_count = recent_recipients.use_count + 1,
               last_used_at = CURRENT_TIMESTAMP,
               updated_at = CURRENT_TIMESTAMP",
            params![
                recipient_id,
                account_id,
                display_name,
                &recipient.address,
                normalized_email,
            ],
        )?;
    }

    Ok(())
}

fn normalize_email_for_recipient_store(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn stable_recent_recipient_id(account_id: &str, normalized_email: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update([0]);
    hasher.update(normalized_email.as_bytes());
    let hash = hasher.finalize();
    let mut id = String::from("rr_");
    for byte in hash.iter().take(16) {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn u64_to_i64(value: u64, label: &str) -> Result<i64, ComposerCommandError> {
    i64::try_from(value).map_err(|_| ComposerCommandError::new(format!("{label} is out of range")))
}

fn draft_view(record: ComposerDraftRecord) -> ComposerDraftView {
    ComposerDraftView {
        id: record.id,
        account_id: record.account_id,
        autosave_revision: record.autosave_revision,
        state: record.draft_state.schema_value().to_string(),
    }
}

fn send_response(draft_id: String, report: SendEmailReport) -> ComposerSendResponse {
    ComposerSendResponse {
        account_id: report.account_id,
        draft_id,
        rfc_message_id: report.rfc_message_id,
        sent_folder_policy: sent_policy_schema_value(report.sent_folder_policy).to_string(),
        sent_folder_sync_state: sent_sync_schema_value(report.sent_folder_sync_state).to_string(),
        provider_response: report.provider_response,
        message_size_bytes: report.message_size_bytes,
        attachment_bytes_streamed: report.attachment_bytes_streamed,
        progress: report
            .progress
            .into_iter()
            .map(composer_progress_from_send_progress)
            .collect(),
    }
}

fn composer_progress_from_send_progress(event: SendProgressEvent) -> ComposerSendProgress {
    match event {
        SendProgressEvent::AccountResolved => ComposerSendProgress::AccountResolved,
        SendProgressEvent::DraftPrepared { .. } => ComposerSendProgress::DraftPrepared,
        SendProgressEvent::AuthResolved { .. } => ComposerSendProgress::AuthResolved,
        SendProgressEvent::AttachmentStreamed { .. } => ComposerSendProgress::AttachmentStreamed,
        SendProgressEvent::MimeComposed { .. } => ComposerSendProgress::MimeComposed,
        SendProgressEvent::SmtpAccepted => ComposerSendProgress::SmtpAccepted,
        SendProgressEvent::SentStateRecorded => ComposerSendProgress::SentStateRecorded,
        SendProgressEvent::DraftFinalized { .. } => ComposerSendProgress::DraftFinalized,
    }
}

fn sent_policy_schema_value(policy: SentFolderPolicy) -> &'static str {
    match policy {
        SentFolderPolicy::ProviderAutoCopies => "provider_auto_copies",
        SentFolderPolicy::ImapAppend => "imap_append",
        SentFolderPolicy::LocalReceiptOnly => "local_receipt_only",
    }
}

fn sent_sync_schema_value(sync_state: SentFolderSyncState) -> &'static str {
    match sync_state {
        SentFolderSyncState::NotRequired => "not_required",
        SentFolderSyncState::PendingUpload => "pending_upload",
        SentFolderSyncState::Uploaded => "uploaded",
        SentFolderSyncState::Unsupported => "unsupported",
        SentFolderSyncState::Failed => "failed",
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Default)]
struct InMemoryAccessTokenCache {
    tokens: HashMap<String, CachedAccessToken>,
}

impl AccessTokenCache for InMemoryAccessTokenCache {
    fn load_access_token(&mut self, account_id: &str) -> Option<CachedAccessToken> {
        self.tokens.get(account_id).cloned()
    }

    fn store_access_token(&mut self, account_id: &str, token: CachedAccessToken) {
        self.tokens.insert(account_id.to_string(), token);
    }

    fn clear_access_token(&mut self, account_id: &str) {
        self.tokens.remove(account_id);
    }
}

struct SqliteTokenAccountDataStore<'connection> {
    connection: &'connection Connection,
}

impl<'connection> SqliteTokenAccountDataStore<'connection> {
    fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }
}

impl AccountDataStore for SqliteTokenAccountDataStore<'_> {
    fn mark_reconnect_needed(
        &mut self,
        account_id: &str,
        _reason: ReconnectReason,
    ) -> Result<(), AccountDataError> {
        self.connection
            .execute(
                "UPDATE accounts
                 SET auth_status = 'reconnect_needed', updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1 AND removed_at IS NULL",
                [account_id],
            )
            .map(|_| ())
            .map_err(|_| AccountDataError::Unavailable)
    }

    fn delete_local_cache(&mut self, _account_id: &str) -> Result<(), AccountDataError> {
        Err(AccountDataError::Unavailable)
    }

    fn delete_account_metadata(&mut self, _account_id: &str) -> Result<(), AccountDataError> {
        Err(AccountDataError::Unavailable)
    }
}

pub struct ComposerAttachmentStore<'connection> {
    connection: &'connection Connection,
    app_data_dir: PathBuf,
}

impl<'connection> ComposerAttachmentStore<'connection> {
    pub fn new(connection: &'connection Connection, app_data_dir: impl Into<PathBuf>) -> Self {
        Self {
            connection,
            app_data_dir: app_data_dir.into(),
        }
    }
}

impl AttachmentContentStore for ComposerAttachmentStore<'_> {
    fn stream_attachment(
        &mut self,
        account_id: &str,
        attachment_id: &str,
        max_bytes: u64,
        chunk_size: usize,
        on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), SendEmailError>,
    ) -> Result<OutboundAttachmentMetadata, SendEmailError> {
        validate_entity_id("account_id", account_id)?;
        validate_entity_id("attachment_id", attachment_id)?;
        let row = self
            .connection
            .query_row(
                "SELECT
                   display_filename,
                   canonical_mime,
                   size_bytes,
                   storage_relative_path,
                   disposition,
                   content_id
                 FROM composer_attachments
                 WHERE account_id = ?1 AND id = ?2",
                params![account_id, attachment_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            display_filename,
            content_type,
            size_bytes,
            storage_relative_path,
            disposition,
            content_id,
        )) = row
        else {
            return Err(SendEmailError::UnknownAttachment);
        };
        let expected_bytes =
            u64::try_from(size_bytes).map_err(|_| SendEmailError::AttachmentTooLarge {
                attachment_id: attachment_id.to_string(),
                max_bytes,
                actual_bytes: u64::MAX,
            })?;
        if expected_bytes > max_bytes {
            return Err(SendEmailError::AttachmentTooLarge {
                attachment_id: attachment_id.to_string(),
                max_bytes,
                actual_bytes: expected_bytes,
            });
        }

        let full_path = checked_app_data_path_for_read(&self.app_data_dir, &storage_relative_path)
            .map_err(|_| SendEmailError::AttachmentPathRejected {
                attachment_id: attachment_id.to_string(),
            })?;
        let mut file =
            File::open(full_path).map_err(|_| SendEmailError::AttachmentUnavailable {
                attachment_id: attachment_id.to_string(),
            })?;
        let mut total = 0_u64;
        let mut buffer = vec![0_u8; chunk_size.clamp(1, ATTACHMENT_STREAM_CHUNK_BYTES)];
        loop {
            let read =
                file.read(&mut buffer)
                    .map_err(|_| SendEmailError::AttachmentUnavailable {
                        attachment_id: attachment_id.to_string(),
                    })?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > max_bytes {
                return Err(SendEmailError::AttachmentTooLarge {
                    attachment_id: attachment_id.to_string(),
                    max_bytes,
                    actual_bytes: total,
                });
            }
            on_chunk(&buffer[..read])?;
        }
        if total != expected_bytes {
            return Err(SendEmailError::AttachmentSizeMismatch {
                attachment_id: attachment_id.to_string(),
                expected_bytes,
                actual_bytes: total,
            });
        }

        Ok(OutboundAttachmentMetadata {
            attachment_id: attachment_id.to_string(),
            display_filename,
            content_type,
            disposition: outbound_disposition(composer_disposition_from_schema(&disposition)),
            content_id,
            size_bytes: expected_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{AuthKind, ProviderKind},
        schema::apply_migrations,
        smtp_send::{
            ConfirmedSendPersistence, SmtpAccountConfig, SmtpAuth, SmtpDelivery, SmtpSecurity,
        },
        token_lifecycle::SecretString,
    };
    use lettre::Message;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
    }

    fn temp_app_data_dir(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("komorebi-composer-{label}-{}", unix_now()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn seed_generic_account(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO accounts (
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
                   auth_status,
                   signature_html
                 )
                 VALUES (
                   'acct_composer',
                   'generic_imap_smtp',
                   'sender@example.com',
                   'Sender',
                   'sender@example.com',
                   'app_password',
                   'cred_acct_composer',
                   'imap.example.com',
                   993,
                   'tls',
                   'smtp.example.com',
                   587,
                   'starttls',
                   'connected',
                   '<p>Regards,<br><strong>Sender</strong></p>'
                 )",
                [],
            )
            .unwrap();
    }

    fn draft_save_request() -> ComposerDraftSaveRequest {
        ComposerDraftSaveRequest {
            account_id: "acct_composer".into(),
            draft_id: "draft_composer".into(),
            subject: "Composer subject".into(),
            to: vec![ComposerAddress {
                name: Some("Recipient".into()),
                address: "recipient@example.com".into(),
            }],
            cc: Vec::new(),
            bcc: Vec::new(),
            body_text: Some("Hello from composer".into()),
            body_html: Some("<p>Hello from <strong>composer</strong></p>".into()),
            attachment_ids: Vec::new(),
        }
    }

    #[test]
    fn autosave_command_persists_composer_draft() {
        let connection = migrated_connection();
        seed_generic_account(&connection);

        let saved = autosave_composer_draft_from_connection(&connection, draft_save_request())
            .expect("save composer draft");

        assert_eq!(saved.id, "draft_composer");
        assert_eq!(saved.autosave_revision, 1);
        let subject: String = connection
            .query_row(
                "SELECT subject FROM composer_drafts WHERE id = 'draft_composer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(subject, "Composer subject");
    }

    #[test]
    fn autosave_command_persists_rich_composer_html() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        let mut request = draft_save_request();
        request.body_html = Some(
            r#"<p style="text-align: center"><s><span style="font-family: Georgia; font-size: 18px; color: #9a624f">Rich</span></s></p><table><tbody><tr><th><p>Item</p></th><th><p>Status</p></th></tr><tr><td><p>Sync</p></td><td><p>Ready</p></td></tr></tbody></table><pre><code>const ok = true;</code></pre>"#
                .into(),
        );

        autosave_composer_draft_from_connection(&connection, request).unwrap();

        let body_html: String = connection
            .query_row(
                "SELECT body_html FROM composer_drafts WHERE id = 'draft_composer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(body_html.contains("<table>"));
        assert!(body_html.contains("<s>"));
        assert!(body_html.contains("<pre><code>const ok = true;</code></pre>"));
        assert!(body_html.contains("font-family: Georgia"));
    }

    #[test]
    fn stages_composer_attachment_under_generated_app_data_path() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        autosave_composer_draft_from_connection(&connection, draft_save_request()).unwrap();
        let app_data_dir = temp_app_data_dir("stage");

        let staged = stage_composer_attachment_from_connection(
            &connection,
            &app_data_dir,
            ComposerAttachmentStageRequest {
                account_id: "acct_composer".into(),
                draft_id: "draft_composer".into(),
                display_filename: "..\\report?.txt".into(),
                content_type: Some("text/plain".into()),
                disposition: ComposerAttachmentDisposition::Attachment,
                bytes: b"hello attachment".to_vec(),
            },
        )
        .expect("stage attachment");

        assert_eq!(staged.display_filename, "report_.txt");
        assert_eq!(staged.content_type, "text/plain");
        assert!(staged.id.starts_with("catt_"));
        let stored_path: String = connection
            .query_row(
                "SELECT storage_relative_path
                 FROM composer_attachments
                 WHERE id = ?1",
                [&staged.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored_path.starts_with("composer-attachments/acct_composer/draft_composer/"));
        assert!(checked_app_data_path_for_read(&app_data_dir, &stored_path)
            .unwrap()
            .exists());

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn rejects_unsafe_composer_attachment_content() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        autosave_composer_draft_from_connection(&connection, draft_save_request()).unwrap();
        let app_data_dir = temp_app_data_dir("reject");

        let error = stage_composer_attachment_from_connection(
            &connection,
            &app_data_dir,
            ComposerAttachmentStageRequest {
                account_id: "acct_composer".into(),
                draft_id: "draft_composer".into(),
                display_filename: "invoice.svg".into(),
                content_type: Some("image/svg+xml".into()),
                disposition: ComposerAttachmentDisposition::Inline,
                bytes: br#"<svg onload="alert(1)"></svg>"#.to_vec(),
            },
        )
        .unwrap_err();

        assert!(error.reason.contains("attachment type is not allowed"));
        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn sends_composer_request_with_staged_attachment_and_cleans_private_files() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        autosave_composer_draft_from_connection(&connection, draft_save_request()).unwrap();
        let app_data_dir = temp_app_data_dir("send");
        let staged = stage_composer_attachment_from_connection(
            &connection,
            &app_data_dir,
            ComposerAttachmentStageRequest {
                account_id: "acct_composer".into(),
                draft_id: "draft_composer".into(),
                display_filename: "report.txt".into(),
                content_type: Some("text/plain".into()),
                disposition: ComposerAttachmentDisposition::Attachment,
                bytes: b"hello attachment".to_vec(),
            },
        )
        .unwrap();
        let stored_path: String = connection
            .query_row(
                "SELECT storage_relative_path FROM composer_attachments WHERE id = ?1",
                [&staged.id],
                |row| row.get(0),
            )
            .unwrap();
        let full_path = checked_app_data_path_for_read(&app_data_dir, &stored_path).unwrap();
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver;
        let mut attachments = ComposerAttachmentStore::new(&connection, &app_data_dir);
        let mut smtp = FakeSmtpSender::default();

        let response = send_composer_email_with_runtime(
            &connection,
            &app_data_dir,
            ComposerSendRequest {
                attachment_ids: vec![staged.id.clone()],
                ..ComposerSendRequest::from(draft_save_request())
            },
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .expect("send composer email");

        assert_eq!(response.account_id, "acct_composer");
        assert_eq!(response.draft_id, "draft_composer");
        assert!(response
            .progress
            .contains(&ComposerSendProgress::SmtpAccepted));
        assert_eq!(response.attachment_bytes_streamed, 16);
        assert!(!full_path.exists());
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM composer_attachments", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
        let draft_state: String = connection
            .query_row(
                "SELECT draft_state FROM composer_drafts WHERE id = 'draft_composer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(draft_state, "sent");
        assert!(smtp.sent_message.contains("komorebi-account-signature"));

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    impl From<ComposerDraftSaveRequest> for ComposerSendRequest {
        fn from(value: ComposerDraftSaveRequest) -> Self {
            Self {
                account_id: value.account_id,
                draft_id: value.draft_id,
                subject: value.subject,
                to: value.to,
                cc: value.cc,
                bcc: value.bcc,
                body_text: value.body_text,
                body_html: value.body_html,
                attachment_ids: value.attachment_ids,
            }
        }
    }

    struct FakeAuthResolver;

    impl SmtpAuthResolver for FakeAuthResolver {
        fn resolve_smtp_auth(
            &mut self,
            account: &SmtpAccountConfig,
        ) -> Result<SmtpAuth, SendEmailError> {
            Ok(SmtpAuth::Plain {
                username: account.username.clone(),
                password: SecretString::new("app-password"),
            })
        }
    }

    #[derive(Default)]
    struct FakeSmtpSender {
        sent_message: String,
    }

    impl SmtpSender for FakeSmtpSender {
        fn send(
            &mut self,
            _account: &SmtpAccountConfig,
            _auth: &SmtpAuth,
            message: &Message,
        ) -> Result<SmtpDelivery, SendEmailError> {
            self.sent_message = String::from_utf8_lossy(&message.formatted()).to_string();
            Ok(SmtpDelivery {
                provider_response: Some("250 accepted".into()),
            })
        }
    }

    #[test]
    fn command_mapping_rejects_invalid_send_addresses_before_smtp() {
        let request = ComposerSendRequest {
            to: vec![ComposerAddress {
                name: None,
                address: "not-an-address".into(),
            }],
            ..ComposerSendRequest::from(draft_save_request())
        };

        let error = send_email_request(request).unwrap_err();

        assert!(error
            .reason
            .contains("email_address is not a valid address"));
    }

    #[test]
    fn autosave_deduplicates_recipients_across_to_cc_and_bcc() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        let mut request = draft_save_request();
        request.to = vec![
            ComposerAddress {
                name: Some("Primary".into()),
                address: "recipient@example.com".into(),
            },
            ComposerAddress {
                name: None,
                address: "RECIPIENT@example.com".into(),
            },
        ];
        request.cc = vec![
            ComposerAddress {
                name: None,
                address: "copy@example.com".into(),
            },
            ComposerAddress {
                name: None,
                address: "recipient@example.com".into(),
            },
        ];
        request.bcc = vec![ComposerAddress {
            name: None,
            address: "COPY@example.com".into(),
        }];

        autosave_composer_draft_from_connection(&connection, request).unwrap();

        let stored = connection
            .query_row(
                "SELECT to_addresses_json, cc_addresses_json, bcc_addresses_json
                 FROM composer_drafts
                 WHERE id = 'draft_composer'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            stored.0,
            r#"[{"name":"Primary","address":"recipient@example.com"}]"#
        );
        assert_eq!(stored.1, r#"[{"name":null,"address":"copy@example.com"}]"#);
        assert_eq!(stored.2, "[]");
    }

    #[test]
    fn confirmed_send_records_local_recent_recipients_for_autocomplete() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        autosave_composer_draft_from_connection(&connection, draft_save_request()).unwrap();
        let app_data_dir = temp_app_data_dir("recent");
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver;
        let mut attachments = ComposerAttachmentStore::new(&connection, &app_data_dir);
        let mut smtp = FakeSmtpSender::default();
        let mut request = ComposerSendRequest::from(draft_save_request());
        request.to = vec![ComposerAddress {
            name: Some("Recent Person".into()),
            address: "recent@example.com".into(),
        }];
        request.cc = vec![
            ComposerAddress {
                name: None,
                address: "RECENT@example.com".into(),
            },
            ComposerAddress {
                name: Some("Copy Person".into()),
                address: "copy@example.com".into(),
            },
        ];
        request.bcc = vec![ComposerAddress {
            name: None,
            address: "copy@example.com".into(),
        }];

        send_composer_email_with_runtime(
            &connection,
            &app_data_dir,
            request,
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .expect("send composer email");

        let recent_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM recent_recipients", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(recent_count, 2);
        let suggestions = list_recent_recipients_from_connection(
            &connection,
            RecentRecipientSearchRequest {
                account_id: "acct_composer".into(),
                query: "copy".into(),
                limit: Some(5),
            },
        )
        .unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].name.as_deref(), Some("Copy Person"));
        assert_eq!(suggestions[0].address, "copy@example.com");

        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[allow(dead_code)]
    fn _assert_public_send_types() {
        let _ = SmtpAccountConfig {
            account_id: "acct".into(),
            provider_kind: ProviderKind::GenericImapSmtp,
            auth_kind: AuthKind::AppPassword,
            display_email: "sender@example.com".into(),
            display_name: None,
            username: "sender@example.com".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_security: SmtpSecurity::StartTls,
            signature_html: None,
        };
        let _ = ConfirmedSendPersistence {
            sent_folder_policy: SentFolderPolicy::LocalReceiptOnly,
            sent_folder_sync_state: SentFolderSyncState::Unsupported,
        };
    }
}
