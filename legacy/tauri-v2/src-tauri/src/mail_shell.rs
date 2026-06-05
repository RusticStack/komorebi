use std::{collections::HashMap, fs, path::Path};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::ipc_validation::{
    reject_raw_sql, reject_unsanitized_html, validate_bounded_text, validate_entity_id,
    IpcValidationError,
};
use crate::schema::{apply_migrations, SchemaMigrationError};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const MESSAGE_SNAPSHOT_LIMIT: i64 = 500;
const MESSAGE_PAGE_DEFAULT_LIMIT: u32 = 100;
const MESSAGE_PAGE_MAX_LIMIT: u32 = 250;
const MAIL_SEARCH_RESULT_LIMIT: u32 = 100;
const MAX_SEARCH_QUERY_BYTES: usize = 512;
const MAX_SEARCH_TERMS: usize = 12;
const MAX_SENDER_EMAIL_BYTES: usize = 512;
const MAX_PAGE_CURSOR_SORT_KEY_BYTES: usize = 128;
const SEARCH_FILTER_FROM: &str = "from";
const SEARCH_FILTER_SUBJECT: &str = "subject";
const SEARCH_FILTER_BODY: &str = "body";
const SEARCH_FILTER_HAS: &str = "has";
const SEARCH_FILTER_IS: &str = "is";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellSnapshot {
    pub accounts: Vec<MailShellAccount>,
    pub folders: Vec<MailShellFolder>,
    pub messages: Vec<MailShellMessage>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellAccount {
    pub id: String,
    pub label: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider_kind: String,
    pub auth_kind: String,
    pub status: String,
    pub status_updated_at: Option<String>,
    pub last_successful_sync_at: Option<String>,
    pub status_message: Option<String>,
    pub status_action: Option<String>,
    pub unread_count: u32,
    pub signature_html: Option<String>,
    pub local_preferences: MailShellAccountLocalPreferences,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellAccountLocalPreferences {
    pub include_in_unified_inbox: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellFolder {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub kind: String,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellMessage {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub sender: String,
    pub sender_email: String,
    pub subject: String,
    pub preview: String,
    pub received_at: String,
    pub unread: bool,
    pub flagged: bool,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub rfc_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub to_addresses: Vec<MailShellAddress>,
    pub cc_addresses: Vec<MailShellAddress>,
    pub remote_content_allowed: bool,
    pub remote_content_allowed_scope: Option<String>,
    pub attachments: Vec<MailShellAttachment>,
    pub page_cursor: Option<MailMessagePageCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailMessagePageCursor {
    pub sort_key: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellAttachment {
    pub id: String,
    pub display_filename: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub disposition: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailSearchRequest {
    pub query: String,
    pub account_id: Option<String>,
    pub folder_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailMessagePageRequest {
    pub account_id: Option<String>,
    pub folder_id: Option<String>,
    pub unified_inbox: bool,
    pub cursor: Option<MailMessagePageCursor>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailMessagePageResponse {
    pub messages: Vec<MailShellMessage>,
    pub next_cursor: Option<MailMessagePageCursor>,
    pub has_more: bool,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailSearchResponse {
    pub query: String,
    pub index_status: MailSearchIndexStatus,
    pub results: Vec<MailShellMessage>,
    pub parsed_query: MailSearchParsedQuery,
    pub highlight_terms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailSearchParsedQuery {
    pub text_terms: Vec<String>,
    pub filters: Vec<MailSearchParsedFilter>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailSearchParsedFilter {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteContentPreferenceScope {
    Message,
    Sender,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteContentPreferenceRequest {
    pub account_id: String,
    pub folder_id: String,
    pub message_id: String,
    pub scope: RemoteContentPreferenceScope,
    pub user_requested: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteContentPreferenceResponse {
    pub account_id: String,
    pub message_id: String,
    pub scope: RemoteContentPreferenceScope,
    pub sender_email: Option<String>,
    pub remote_content_allowed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MailSearchIndexStatus {
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailShellError {
    pub reason: String,
}

impl MailShellError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl From<rusqlite::Error> for MailShellError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<SchemaMigrationError> for MailShellError {
    fn from(error: SchemaMigrationError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<IpcValidationError> for MailShellError {
    fn from(error: IpcValidationError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<std::io::Error> for MailShellError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<tauri::Error> for MailShellError {
    fn from(error: tauri::Error) -> Self {
        Self::new(error.to_string())
    }
}

#[tauri::command]
pub fn load_mail_shell(app: tauri::AppHandle) -> Result<MailShellSnapshot, MailShellError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);

    load_mail_shell_from_path(&cache_path)
}

pub fn load_mail_shell_from_path(cache_path: &Path) -> Result<MailShellSnapshot, MailShellError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    load_mail_shell_from_connection(&connection).map_err(Into::into)
}

#[tauri::command]
pub fn search_mail(
    app: tauri::AppHandle,
    request: MailSearchRequest,
) -> Result<MailSearchResponse, MailShellError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);

    search_mail_from_path(&cache_path, request)
}

pub fn search_mail_from_path(
    cache_path: &Path,
    request: MailSearchRequest,
) -> Result<MailSearchResponse, MailShellError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    search_mail_from_connection(&connection, request)
}

#[tauri::command]
pub fn load_message_page(
    app: tauri::AppHandle,
    request: MailMessagePageRequest,
) -> Result<MailMessagePageResponse, MailShellError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);

    load_message_page_from_path(&cache_path, request)
}

pub fn load_message_page_from_path(
    cache_path: &Path,
    request: MailMessagePageRequest,
) -> Result<MailMessagePageResponse, MailShellError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    load_message_page_from_connection(&connection, request)
}

#[tauri::command]
pub fn allow_remote_content(
    app: tauri::AppHandle,
    request: RemoteContentPreferenceRequest,
) -> Result<RemoteContentPreferenceResponse, MailShellError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);

    allow_remote_content_from_path(&cache_path, request)
}

pub fn allow_remote_content_from_path(
    cache_path: &Path,
    request: RemoteContentPreferenceRequest,
) -> Result<RemoteContentPreferenceResponse, MailShellError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    allow_remote_content_from_connection(&connection, request)
}

pub fn allow_remote_content_from_connection(
    connection: &Connection,
    request: RemoteContentPreferenceRequest,
) -> Result<RemoteContentPreferenceResponse, MailShellError> {
    let message = validate_remote_content_preference_request(connection, &request)?;
    let sender_email = match request.scope {
        RemoteContentPreferenceScope::Message => None,
        RemoteContentPreferenceScope::Sender => Some(message.sender_email.clone()),
    };
    let preference = RemoteContentPreferenceRecord {
        account_id: request.account_id.clone(),
        message_id: request.message_id.clone(),
        scope: request.scope.clone(),
        sender_email,
    };
    upsert_remote_content_preference(connection, &preference)?;

    Ok(RemoteContentPreferenceResponse {
        account_id: request.account_id,
        message_id: request.message_id,
        scope: request.scope,
        sender_email: preference.sender_email,
        remote_content_allowed: true,
    })
}

pub fn search_mail_from_connection(
    connection: &Connection,
    request: MailSearchRequest,
) -> Result<MailSearchResponse, MailShellError> {
    let validated_request = validate_search_request(connection, request)?;

    if !messages_fts_available(connection)? {
        return Ok(MailSearchResponse {
            query: validated_request.query,
            index_status: MailSearchIndexStatus::Unavailable,
            results: Vec::new(),
            parsed_query: validated_request.parsed_query,
            highlight_terms: validated_request.highlight_terms,
        });
    }

    let mut messages = search_messages(connection, &validated_request)?;
    attach_message_attachments(connection, &mut messages)?;

    Ok(MailSearchResponse {
        query: validated_request.query,
        index_status: MailSearchIndexStatus::Ready,
        results: messages,
        parsed_query: validated_request.parsed_query,
        highlight_terms: validated_request.highlight_terms,
    })
}

pub fn load_message_page_from_connection(
    connection: &Connection,
    request: MailMessagePageRequest,
) -> Result<MailMessagePageResponse, MailShellError> {
    let validated_request = validate_message_page_request(connection, request)?;
    let total_count = count_messages_for_page(connection, &validated_request)?;
    let fetch_limit = validated_request.limit + 1;
    let mut messages = load_messages_page(connection, &validated_request, fetch_limit)?;
    let has_more = messages.len() > validated_request.limit as usize;
    if has_more {
        messages.truncate(validated_request.limit as usize);
    }
    attach_message_attachments(connection, &mut messages)?;
    let next_cursor = if has_more {
        messages
            .last()
            .and_then(|message| message.page_cursor.clone())
    } else {
        None
    };

    Ok(MailMessagePageResponse {
        messages,
        next_cursor,
        has_more,
        total_count,
    })
}

pub fn load_mail_shell_from_connection(
    connection: &Connection,
) -> rusqlite::Result<MailShellSnapshot> {
    Ok(MailShellSnapshot {
        accounts: load_accounts(connection)?,
        folders: load_folders(connection)?,
        messages: load_messages(connection)?,
    })
}

fn load_accounts(connection: &Connection) -> rusqlite::Result<Vec<MailShellAccount>> {
    let mut statement = connection.prepare(
        "SELECT
           accounts.id,
           COALESCE(NULLIF(accounts.display_name, ''), accounts.display_email) AS label,
           accounts.display_email,
           accounts.display_name,
           accounts.provider_kind,
           accounts.auth_kind,
           CASE
             WHEN accounts.auth_status = 'reconnect_needed' THEN 'reconnect-needed'
             WHEN json_extract(sync_worker_status.value_json, '$.state') = 'syncing' THEN 'syncing'
             WHEN json_extract(sync_worker_status.value_json, '$.state') = 'error' THEN 'error'
             WHEN json_extract(sync_worker_status.value_json, '$.state') = 'idle' THEN 'idle'
             WHEN accounts.auth_status = 'connected'
              AND EXISTS (
                SELECT 1
                FROM folders
                WHERE folders.account_id = accounts.id
                  AND folders.sync_state = 'syncing'
              ) THEN 'syncing'
             WHEN accounts.auth_status = 'connected' THEN 'idle'
             ELSE 'offline'
           END AS shell_status,
           CASE
             WHEN json_type(sync_worker_status.value_json, '$.updatedAt') IN ('integer', 'real')
             THEN strftime('%Y-%m-%dT%H:%M:%SZ', json_extract(sync_worker_status.value_json, '$.updatedAt'), 'unixepoch')
             ELSE NULL
           END AS status_updated_at,
           CASE
             WHEN json_type(sync_worker_status.value_json, '$.lastSuccessfulSyncAt') IN ('integer', 'real')
             THEN strftime('%Y-%m-%dT%H:%M:%SZ', json_extract(sync_worker_status.value_json, '$.lastSuccessfulSyncAt'), 'unixepoch')
             ELSE NULL
           END AS last_successful_sync_at,
           json_extract(sync_worker_status.value_json, '$.message') AS status_message,
           json_extract(sync_worker_status.value_json, '$.action') AS status_action,
           COALESCE(
             (
               SELECT SUM(folders.unread_count)
               FROM folders
               WHERE folders.account_id = accounts.id
             ),
             0
           ) AS unread_count,
           accounts.signature_html,
           CASE
             WHEN COALESCE(
               json_extract(account_preferences.value_json, '$.includeInUnifiedInbox'),
               1
             ) = 0 THEN 0
             ELSE 1
           END AS include_in_unified_inbox
         FROM accounts
         LEFT JOIN settings AS account_preferences
           ON account_preferences.key = 'account:' || accounts.id || ':local_preferences'
         LEFT JOIN settings AS sync_worker_status
           ON sync_worker_status.key = 'account:' || accounts.id || ':sync_worker_status'
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
         ORDER BY accounts.display_email COLLATE NOCASE, accounts.id",
    )?;

    let rows = statement.query_map([], |row| {
        Ok(MailShellAccount {
            id: row.get(0)?,
            label: row.get(1)?,
            email: row.get(2)?,
            display_name: row.get(3)?,
            provider_kind: row.get(4)?,
            auth_kind: row.get(5)?,
            status: row.get(6)?,
            status_updated_at: row.get(7)?,
            last_successful_sync_at: row.get(8)?,
            status_message: row.get(9)?,
            status_action: row.get(10)?,
            unread_count: checked_u32(row.get::<_, i64>(11)?, "account unread count")?,
            signature_html: row.get(12)?,
            local_preferences: MailShellAccountLocalPreferences {
                include_in_unified_inbox: row.get::<_, i64>(13)? != 0,
            },
        })
    })?;

    collect_rows(rows)
}

fn load_folders(connection: &Connection) -> rusqlite::Result<Vec<MailShellFolder>> {
    let mut statement = connection.prepare(
        "SELECT
           folders.id,
           folders.account_id,
           folders.display_name,
           folders.kind,
           folders.unread_count,
           folders.total_count
         FROM folders
         JOIN accounts ON accounts.id = folders.account_id
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
         ORDER BY
           folders.account_id,
           CASE folders.kind
             WHEN 'inbox' THEN 0
             WHEN 'sent' THEN 1
             WHEN 'drafts' THEN 2
             WHEN 'archive' THEN 3
             WHEN 'all' THEN 4
             WHEN 'important' THEN 5
             WHEN 'junk' THEN 6
             WHEN 'trash' THEN 7
             ELSE 8
           END,
           folders.display_name COLLATE NOCASE,
           folders.id",
    )?;

    let rows = statement.query_map([], |row| {
        Ok(MailShellFolder {
            id: row.get(0)?,
            account_id: row.get(1)?,
            name: row.get(2)?,
            kind: normalize_folder_kind(row.get::<_, String>(3)?),
            unread_count: checked_u32(row.get::<_, i64>(4)?, "folder unread count")?,
            total_count: checked_u32(row.get::<_, i64>(5)?, "folder total count")?,
        })
    })?;

    collect_rows(rows)
}

fn load_messages(connection: &Connection) -> rusqlite::Result<Vec<MailShellMessage>> {
    let mut statement = connection.prepare(
        "SELECT
           messages.id,
           messages.account_id,
           messages.folder_id,
           COALESCE(NULLIF(messages.sender_name, ''), NULLIF(messages.sender_email, ''), '') AS sender,
           COALESCE(messages.sender_email, '') AS sender_email,
           messages.subject,
           COALESCE(NULLIF(messages.snippet, ''), NULLIF(messages.body_text, ''), '') AS preview,
           COALESCE(messages.date_received, messages.internal_date, '') AS received_at,
           messages.is_read,
           messages.is_flagged,
           messages.body_text,
           messages.body_html,
           messages.rfc_message_id,
           messages.in_reply_to,
           messages.references_json,
           messages.to_addresses_json,
           messages.cc_addresses_json,
           CASE
             WHEN message_remote_content.id IS NOT NULL THEN 1
             WHEN sender_remote_content.id IS NOT NULL THEN 1
             ELSE 0
           END AS remote_content_allowed,
           CASE
             WHEN message_remote_content.id IS NOT NULL THEN 'message'
             WHEN sender_remote_content.id IS NOT NULL THEN 'sender'
             ELSE NULL
           END AS remote_content_allowed_scope,
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) AS page_sort_key
         FROM messages
         JOIN accounts ON accounts.id = messages.account_id
         JOIN folders ON folders.account_id = messages.account_id
          AND folders.id = messages.folder_id
         LEFT JOIN remote_content_preferences AS message_remote_content
           ON message_remote_content.account_id = messages.account_id
          AND message_remote_content.scope = 'message'
          AND message_remote_content.message_id = messages.id
         LEFT JOIN remote_content_preferences AS sender_remote_content
           ON sender_remote_content.account_id = messages.account_id
          AND sender_remote_content.scope = 'sender'
          AND sender_remote_content.normalized_sender_email = LOWER(TRIM(COALESCE(messages.sender_email, '')))
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
         ORDER BY
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) DESC,
           messages.id
         LIMIT ?1",
    )?;

    let rows = statement.query_map(params![MESSAGE_SNAPSHOT_LIMIT], |row| {
        mail_shell_message_from_row(row)
    })?;

    let mut messages = collect_rows(rows)?;
    attach_message_attachments(connection, &mut messages)?;

    Ok(messages)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedSearchRequest {
    query: String,
    fts_query: Option<String>,
    parsed_query: MailSearchParsedQuery,
    highlight_terms: Vec<String>,
    sender_filter: Option<String>,
    subject_filter: Option<String>,
    body_filter: Option<String>,
    has_attachment_filter: Option<bool>,
    read_filter: Option<bool>,
    flagged_filter: Option<bool>,
    account_id: Option<String>,
    folder_id: Option<String>,
    limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchQueryPlan {
    fts_query: Option<String>,
    parsed_query: MailSearchParsedQuery,
    highlight_terms: Vec<String>,
    sender_filter: Option<String>,
    subject_filter: Option<String>,
    body_filter: Option<String>,
    has_attachment_filter: Option<bool>,
    read_filter: Option<bool>,
    flagged_filter: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SearchQueryToken {
    Text(String),
    Filter { kind: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedMessagePageRequest {
    account_id: Option<String>,
    folder_id: Option<String>,
    unified_inbox: bool,
    cursor: Option<MailMessagePageCursor>,
    limit: u32,
}

fn validate_message_page_request(
    connection: &Connection,
    request: MailMessagePageRequest,
) -> Result<ValidatedMessagePageRequest, MailShellError> {
    if request.unified_inbox && (request.account_id.is_some() || request.folder_id.is_some()) {
        return Err(MailShellError::new(
            "unified message pages cannot include account or folder scope",
        ));
    }

    if !request.unified_inbox && request.account_id.is_none() && request.folder_id.is_none() {
        return Err(MailShellError::new("message page scope is required"));
    }

    if let Some(account_id) = request.account_id.as_deref() {
        validate_entity_id("account_id", account_id)?;
        if !active_account_exists(connection, account_id)? {
            return Err(IpcValidationError::UnknownAccount.into());
        }
    }

    if let Some(folder_id) = request.folder_id.as_deref() {
        validate_entity_id("folder_id", folder_id)?;
        if !folder_exists_in_scope(connection, request.account_id.as_deref(), folder_id)? {
            return Err(IpcValidationError::UnknownFolder.into());
        }
    }

    if let Some(cursor) = request.cursor.as_ref() {
        validate_entity_id("cursor.message_id", &cursor.message_id)?;
        validate_bounded_text(
            "cursor.sort_key",
            &cursor.sort_key,
            MAX_PAGE_CURSOR_SORT_KEY_BYTES,
        )?;
        reject_raw_sql("cursor.sort_key", &cursor.sort_key)?;
        reject_unsanitized_html("cursor.sort_key", &cursor.sort_key)?;
        if !page_cursor_matches_scope(connection, &request, cursor)? {
            return Err(MailShellError::new("message page cursor is invalid"));
        }
    }

    Ok(ValidatedMessagePageRequest {
        account_id: request.account_id,
        folder_id: request.folder_id,
        unified_inbox: request.unified_inbox,
        cursor: request.cursor,
        limit: request
            .limit
            .unwrap_or(MESSAGE_PAGE_DEFAULT_LIMIT)
            .clamp(1, MESSAGE_PAGE_MAX_LIMIT),
    })
}

fn page_cursor_matches_scope(
    connection: &Connection,
    request: &MailMessagePageRequest,
    cursor: &MailMessagePageCursor,
) -> rusqlite::Result<bool> {
    let exists = connection.query_row(
        "SELECT EXISTS (
           SELECT 1
           FROM messages
           JOIN accounts ON accounts.id = messages.account_id
           JOIN folders ON folders.account_id = messages.account_id
            AND folders.id = messages.folder_id
           LEFT JOIN settings AS account_preferences
             ON account_preferences.key = 'account:' || accounts.id || ':local_preferences'
           WHERE messages.id = ?1
             AND COALESCE(messages.date_received, messages.internal_date, messages.created_at) = ?2
             AND accounts.removed_at IS NULL
             AND accounts.auth_status != 'removed'
             AND (
               ?3 = 0 OR (
                 folders.kind = 'inbox'
                 AND COALESCE(
                   json_extract(account_preferences.value_json, '$.includeInUnifiedInbox'),
                   1
                 ) != 0
               )
             )
             AND (?4 IS NULL OR messages.account_id = ?4)
             AND (?5 IS NULL OR messages.folder_id = ?5)
         )",
        params![
            cursor.message_id.as_str(),
            cursor.sort_key.as_str(),
            request.unified_inbox as i64,
            request.account_id.as_deref(),
            request.folder_id.as_deref(),
        ],
        |row| row.get::<_, i64>(0),
    )?;

    Ok(exists == 1)
}

fn count_messages_for_page(
    connection: &Connection,
    request: &ValidatedMessagePageRequest,
) -> rusqlite::Result<u32> {
    let count = connection.query_row(
        "SELECT COUNT(*)
         FROM messages
         JOIN accounts ON accounts.id = messages.account_id
         JOIN folders ON folders.account_id = messages.account_id
          AND folders.id = messages.folder_id
         LEFT JOIN settings AS account_preferences
           ON account_preferences.key = 'account:' || accounts.id || ':local_preferences'
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
           AND (
             ?1 = 0 OR (
               folders.kind = 'inbox'
               AND COALESCE(
                 json_extract(account_preferences.value_json, '$.includeInUnifiedInbox'),
                 1
               ) != 0
             )
           )
           AND (?2 IS NULL OR messages.account_id = ?2)
           AND (?3 IS NULL OR messages.folder_id = ?3)",
        params![
            request.unified_inbox as i64,
            request.account_id.as_deref(),
            request.folder_id.as_deref()
        ],
        |row| checked_u32(row.get::<_, i64>(0)?, "message page count"),
    )?;

    Ok(count)
}

fn load_messages_page(
    connection: &Connection,
    request: &ValidatedMessagePageRequest,
    fetch_limit: u32,
) -> rusqlite::Result<Vec<MailShellMessage>> {
    let cursor_sort_key = request
        .cursor
        .as_ref()
        .map(|cursor| cursor.sort_key.as_str());
    let cursor_message_id = request
        .cursor
        .as_ref()
        .map(|cursor| cursor.message_id.as_str());
    let mut statement = connection.prepare(
        "WITH scoped_messages AS (
           SELECT
             messages.id,
             messages.account_id,
             messages.folder_id,
             COALESCE(NULLIF(messages.sender_name, ''), NULLIF(messages.sender_email, ''), '') AS sender,
             COALESCE(messages.sender_email, '') AS sender_email,
             messages.subject,
             COALESCE(NULLIF(messages.snippet, ''), NULLIF(messages.body_text, ''), '') AS preview,
             COALESCE(messages.date_received, messages.internal_date, '') AS received_at,
             messages.is_read,
             messages.is_flagged,
             messages.body_text,
             messages.body_html,
             messages.rfc_message_id,
             messages.in_reply_to,
             messages.references_json,
             messages.to_addresses_json,
             messages.cc_addresses_json,
             CASE
               WHEN message_remote_content.id IS NOT NULL THEN 1
               WHEN sender_remote_content.id IS NOT NULL THEN 1
               ELSE 0
             END AS remote_content_allowed,
             CASE
               WHEN message_remote_content.id IS NOT NULL THEN 'message'
               WHEN sender_remote_content.id IS NOT NULL THEN 'sender'
               ELSE NULL
             END AS remote_content_allowed_scope,
             COALESCE(messages.date_received, messages.internal_date, messages.created_at) AS page_sort_key
           FROM messages
           JOIN accounts ON accounts.id = messages.account_id
           JOIN folders ON folders.account_id = messages.account_id
            AND folders.id = messages.folder_id
           LEFT JOIN settings AS account_preferences
             ON account_preferences.key = 'account:' || accounts.id || ':local_preferences'
           LEFT JOIN remote_content_preferences AS message_remote_content
             ON message_remote_content.account_id = messages.account_id
            AND message_remote_content.scope = 'message'
            AND message_remote_content.message_id = messages.id
           LEFT JOIN remote_content_preferences AS sender_remote_content
             ON sender_remote_content.account_id = messages.account_id
            AND sender_remote_content.scope = 'sender'
            AND sender_remote_content.normalized_sender_email = LOWER(TRIM(COALESCE(messages.sender_email, '')))
           WHERE accounts.removed_at IS NULL
             AND accounts.auth_status != 'removed'
             AND (
               ?1 = 0 OR (
                 folders.kind = 'inbox'
                 AND COALESCE(
                   json_extract(account_preferences.value_json, '$.includeInUnifiedInbox'),
                   1
                 ) != 0
               )
             )
             AND (?2 IS NULL OR messages.account_id = ?2)
             AND (?3 IS NULL OR messages.folder_id = ?3)
         )
         SELECT *
         FROM scoped_messages
         WHERE (
           ?4 IS NULL
           OR page_sort_key < ?4
           OR (page_sort_key = ?4 AND id > ?5)
         )
         ORDER BY page_sort_key DESC, id
         LIMIT ?6",
    )?;

    let rows = statement.query_map(
        params![
            request.unified_inbox as i64,
            request.account_id.as_deref(),
            request.folder_id.as_deref(),
            cursor_sort_key,
            cursor_message_id,
            fetch_limit
        ],
        mail_shell_message_from_row,
    )?;

    collect_rows(rows)
}

fn validate_search_request(
    connection: &Connection,
    request: MailSearchRequest,
) -> Result<ValidatedSearchRequest, MailShellError> {
    validate_bounded_text("query", &request.query, MAX_SEARCH_QUERY_BYTES)?;
    reject_raw_sql("query", &request.query)?;
    reject_unsanitized_html("query", &request.query)?;
    reject_search_urls_and_paths(&request.query)?;

    let query = request.query.trim().to_string();
    let query_plan = parse_search_query(&query)?;

    if let Some(account_id) = request.account_id.as_deref() {
        validate_entity_id("account_id", account_id)?;
        if !active_account_exists(connection, account_id)? {
            return Err(IpcValidationError::UnknownAccount.into());
        }
    }

    if let Some(folder_id) = request.folder_id.as_deref() {
        validate_entity_id("folder_id", folder_id)?;
        if !folder_exists_in_scope(connection, request.account_id.as_deref(), folder_id)? {
            return Err(IpcValidationError::UnknownFolder.into());
        }
    }

    Ok(ValidatedSearchRequest {
        query,
        fts_query: query_plan.fts_query,
        parsed_query: query_plan.parsed_query,
        highlight_terms: query_plan.highlight_terms,
        sender_filter: query_plan.sender_filter,
        subject_filter: query_plan.subject_filter,
        body_filter: query_plan.body_filter,
        has_attachment_filter: query_plan.has_attachment_filter,
        read_filter: query_plan.read_filter,
        flagged_filter: query_plan.flagged_filter,
        account_id: request.account_id,
        folder_id: request.folder_id,
        limit: request
            .limit
            .unwrap_or(MAIL_SEARCH_RESULT_LIMIT)
            .clamp(1, MAIL_SEARCH_RESULT_LIMIT),
    })
}

fn search_messages(
    connection: &Connection,
    request: &ValidatedSearchRequest,
) -> rusqlite::Result<Vec<MailShellMessage>> {
    if request.fts_query.is_none() {
        return search_messages_by_metadata(connection, request);
    }

    let mut statement = connection.prepare(
        "SELECT
           messages.id,
           messages.account_id,
           messages.folder_id,
           COALESCE(NULLIF(messages.sender_name, ''), NULLIF(messages.sender_email, ''), '') AS sender,
           COALESCE(messages.sender_email, '') AS sender_email,
           messages.subject,
           COALESCE(NULLIF(messages.snippet, ''), NULLIF(messages.body_text, ''), '') AS preview,
           COALESCE(messages.date_received, messages.internal_date, '') AS received_at,
           messages.is_read,
           messages.is_flagged,
           messages.body_text,
           messages.body_html,
           messages.rfc_message_id,
           messages.in_reply_to,
           messages.references_json,
           messages.to_addresses_json,
           messages.cc_addresses_json,
           CASE
             WHEN message_remote_content.id IS NOT NULL THEN 1
             WHEN sender_remote_content.id IS NOT NULL THEN 1
             ELSE 0
           END AS remote_content_allowed,
           CASE
             WHEN message_remote_content.id IS NOT NULL THEN 'message'
             WHEN sender_remote_content.id IS NOT NULL THEN 'sender'
             ELSE NULL
           END AS remote_content_allowed_scope,
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) AS page_sort_key,
           bm25(messages_fts) AS rank
         FROM messages_fts
         JOIN messages ON messages.rowid = messages_fts.rowid
         JOIN accounts ON accounts.id = messages.account_id
         JOIN folders ON folders.account_id = messages.account_id
          AND folders.id = messages.folder_id
         LEFT JOIN settings AS account_preferences
           ON account_preferences.key = 'account:' || accounts.id || ':local_preferences'
         LEFT JOIN remote_content_preferences AS message_remote_content
           ON message_remote_content.account_id = messages.account_id
          AND message_remote_content.scope = 'message'
          AND message_remote_content.message_id = messages.id
         LEFT JOIN remote_content_preferences AS sender_remote_content
           ON sender_remote_content.account_id = messages.account_id
          AND sender_remote_content.scope = 'sender'
          AND sender_remote_content.normalized_sender_email = LOWER(TRIM(COALESCE(messages.sender_email, '')))
         WHERE messages_fts MATCH ?1
           AND accounts.removed_at IS NULL
          AND accounts.auth_status != 'removed'
           AND (?2 IS NULL OR messages.account_id = ?2)
           AND (?3 IS NULL OR messages.folder_id = ?3)
           AND (
             ?4 IS NULL
             OR LOWER(COALESCE(messages.sender_name, '')) LIKE ?4 ESCAPE '\\'
             OR LOWER(COALESCE(messages.sender_email, '')) LIKE ?4 ESCAPE '\\'
           )
           AND (
             ?5 IS NULL
             OR LOWER(COALESCE(messages.subject, '')) LIKE ?5 ESCAPE '\\'
           )
           AND (
             ?6 IS NULL
             OR LOWER(COALESCE(messages.body_text, '')) LIKE ?6 ESCAPE '\\'
             OR LOWER(COALESCE(messages.body_html, '')) LIKE ?6 ESCAPE '\\'
           )
           AND (
             ?7 IS NULL
             OR (
               ?7 = 1
               AND EXISTS (
                 SELECT 1 FROM attachments
                 WHERE attachments.account_id = messages.account_id
                   AND attachments.folder_id = messages.folder_id
                   AND attachments.message_id = messages.id
               )
             )
             OR (
               ?7 = 0
               AND NOT EXISTS (
                 SELECT 1 FROM attachments
                 WHERE attachments.account_id = messages.account_id
                   AND attachments.folder_id = messages.folder_id
                   AND attachments.message_id = messages.id
               )
             )
           )
           AND (?8 IS NULL OR messages.is_read = ?8)
           AND (?9 IS NULL OR messages.is_flagged = ?9)
           AND (
             ?2 IS NOT NULL
             OR COALESCE(
               json_extract(account_preferences.value_json, '$.includeInUnifiedInbox'),
               1
             ) != 0
           )
         ORDER BY
           rank,
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) DESC,
           messages.id
         LIMIT ?10",
    )?;

    let rows = statement.query_map(
        params![
            request.fts_query.as_deref(),
            request.account_id.as_deref(),
            request.folder_id.as_deref(),
            request.sender_filter.as_deref(),
            request.subject_filter.as_deref(),
            request.body_filter.as_deref(),
            optional_bool_i64(request.has_attachment_filter),
            optional_bool_i64(request.read_filter),
            optional_bool_i64(request.flagged_filter),
            request.limit
        ],
        mail_shell_message_from_row,
    )?;

    collect_rows(rows)
}

fn search_messages_by_metadata(
    connection: &Connection,
    request: &ValidatedSearchRequest,
) -> rusqlite::Result<Vec<MailShellMessage>> {
    let mut statement = connection.prepare(
        "SELECT
           messages.id,
           messages.account_id,
           messages.folder_id,
           COALESCE(NULLIF(messages.sender_name, ''), NULLIF(messages.sender_email, ''), '') AS sender,
           COALESCE(messages.sender_email, '') AS sender_email,
           messages.subject,
           COALESCE(NULLIF(messages.snippet, ''), NULLIF(messages.body_text, ''), '') AS preview,
           COALESCE(messages.date_received, messages.internal_date, '') AS received_at,
           messages.is_read,
           messages.is_flagged,
           messages.body_text,
           messages.body_html,
           messages.rfc_message_id,
           messages.in_reply_to,
           messages.references_json,
           messages.to_addresses_json,
           messages.cc_addresses_json,
           CASE
             WHEN message_remote_content.id IS NOT NULL THEN 1
             WHEN sender_remote_content.id IS NOT NULL THEN 1
             ELSE 0
           END AS remote_content_allowed,
           CASE
             WHEN message_remote_content.id IS NOT NULL THEN 'message'
             WHEN sender_remote_content.id IS NOT NULL THEN 'sender'
             ELSE NULL
           END AS remote_content_allowed_scope,
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) AS page_sort_key
         FROM messages
         JOIN accounts ON accounts.id = messages.account_id
         JOIN folders ON folders.account_id = messages.account_id
          AND folders.id = messages.folder_id
         LEFT JOIN settings AS account_preferences
           ON account_preferences.key = 'account:' || accounts.id || ':local_preferences'
         LEFT JOIN remote_content_preferences AS message_remote_content
           ON message_remote_content.account_id = messages.account_id
          AND message_remote_content.scope = 'message'
          AND message_remote_content.message_id = messages.id
         LEFT JOIN remote_content_preferences AS sender_remote_content
           ON sender_remote_content.account_id = messages.account_id
          AND sender_remote_content.scope = 'sender'
          AND sender_remote_content.normalized_sender_email = LOWER(TRIM(COALESCE(messages.sender_email, '')))
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
           AND (?1 IS NULL OR messages.account_id = ?1)
           AND (?2 IS NULL OR messages.folder_id = ?2)
           AND (
             ?3 IS NULL
             OR LOWER(COALESCE(messages.sender_name, '')) LIKE ?3 ESCAPE '\\'
             OR LOWER(COALESCE(messages.sender_email, '')) LIKE ?3 ESCAPE '\\'
           )
           AND (
             ?4 IS NULL
             OR LOWER(COALESCE(messages.subject, '')) LIKE ?4 ESCAPE '\\'
           )
           AND (
             ?5 IS NULL
             OR LOWER(COALESCE(messages.body_text, '')) LIKE ?5 ESCAPE '\\'
             OR LOWER(COALESCE(messages.body_html, '')) LIKE ?5 ESCAPE '\\'
           )
           AND (
             ?6 IS NULL
             OR (
               ?6 = 1
               AND EXISTS (
                 SELECT 1 FROM attachments
                 WHERE attachments.account_id = messages.account_id
                   AND attachments.folder_id = messages.folder_id
                   AND attachments.message_id = messages.id
               )
             )
             OR (
               ?6 = 0
               AND NOT EXISTS (
                 SELECT 1 FROM attachments
                 WHERE attachments.account_id = messages.account_id
                   AND attachments.folder_id = messages.folder_id
                   AND attachments.message_id = messages.id
               )
             )
           )
           AND (?7 IS NULL OR messages.is_read = ?7)
           AND (?8 IS NULL OR messages.is_flagged = ?8)
           AND (
             ?1 IS NOT NULL
             OR COALESCE(
               json_extract(account_preferences.value_json, '$.includeInUnifiedInbox'),
               1
             ) != 0
           )
         ORDER BY
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) DESC,
           messages.id
         LIMIT ?9",
    )?;

    let rows = statement.query_map(
        params![
            request.account_id.as_deref(),
            request.folder_id.as_deref(),
            request.sender_filter.as_deref(),
            request.subject_filter.as_deref(),
            request.body_filter.as_deref(),
            optional_bool_i64(request.has_attachment_filter),
            optional_bool_i64(request.read_filter),
            optional_bool_i64(request.flagged_filter),
            request.limit
        ],
        mail_shell_message_from_row,
    )?;

    collect_rows(rows)
}

fn attach_message_attachments(
    connection: &Connection,
    messages: &mut [MailShellMessage],
) -> rusqlite::Result<()> {
    let message_ids = messages
        .iter()
        .map(|message| message.id.clone())
        .collect::<Vec<_>>();
    let mut attachments_by_message = load_message_attachments_by_ids(connection, &message_ids)?;

    for message in messages {
        message.attachments = attachments_by_message
            .remove(&message.id)
            .unwrap_or_default();
    }

    Ok(())
}

fn mail_shell_message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MailShellMessage> {
    let preview = trim_preview(row.get::<_, String>(6)?);
    let id = row.get::<_, String>(0)?;
    let page_sort_key = row.get::<_, String>(19)?;

    Ok(MailShellMessage {
        id: id.clone(),
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        sender: row.get(3)?,
        sender_email: row.get(4)?,
        subject: row.get(5)?,
        preview,
        received_at: row.get(7)?,
        unread: row.get::<_, i64>(8)? == 0,
        flagged: row.get::<_, i64>(9)? == 1,
        body_text: row.get(10)?,
        body_html: row.get(11)?,
        rfc_message_id: row.get(12)?,
        in_reply_to: row.get(13)?,
        references: message_references_from_json(&row.get::<_, String>(14)?),
        to_addresses: mail_addresses_from_json(&row.get::<_, String>(15)?),
        cc_addresses: mail_addresses_from_json(&row.get::<_, String>(16)?),
        remote_content_allowed: row.get::<_, i64>(17)? == 1,
        remote_content_allowed_scope: row.get(18)?,
        attachments: Vec::new(),
        page_cursor: Some(MailMessagePageCursor {
            sort_key: page_sort_key,
            message_id: id,
        }),
    })
}

fn message_references_from_json(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|reference| {
            let trimmed = reference.trim();
            (trimmed.starts_with('<')
                && trimmed.ends_with('>')
                && trimmed.contains('@')
                && !trimmed.chars().any(char::is_control))
            .then(|| trimmed.to_string())
        })
        .collect()
}

fn mail_addresses_from_json(value: &str) -> Vec<MailShellAddress> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(value) else {
        return Vec::new();
    };

    items
        .into_iter()
        .filter_map(|item| {
            let Value::Object(object) = item else {
                return None;
            };
            let address = object
                .get("address")
                .or_else(|| object.get("email"))
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            if address.is_empty() || address.chars().any(char::is_control) {
                return None;
            }
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string);

            Some(MailShellAddress { name, address })
        })
        .collect()
}

#[derive(Debug, Clone)]
struct RemoteContentMessageRow {
    sender_email: String,
}

#[derive(Debug, Clone)]
struct RemoteContentPreferenceRecord {
    account_id: String,
    message_id: String,
    scope: RemoteContentPreferenceScope,
    sender_email: Option<String>,
}

fn validate_remote_content_preference_request(
    connection: &Connection,
    request: &RemoteContentPreferenceRequest,
) -> Result<RemoteContentMessageRow, MailShellError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("folder_id", &request.folder_id)?;
    validate_entity_id("message_id", &request.message_id)?;

    if !request.user_requested {
        return Err(MailShellError::new(
            "remote content preference requires user intent",
        ));
    }

    if !active_account_exists(connection, &request.account_id)? {
        return Err(IpcValidationError::UnknownAccount.into());
    }

    if !folder_exists_in_scope(connection, Some(&request.account_id), &request.folder_id)? {
        return Err(IpcValidationError::UnknownFolder.into());
    }

    let row = connection
        .query_row(
            "SELECT COALESCE(messages.sender_email, '') AS sender_email
             FROM messages
             JOIN accounts ON accounts.id = messages.account_id
             JOIN folders ON folders.account_id = messages.account_id
              AND folders.id = messages.folder_id
             WHERE accounts.removed_at IS NULL
               AND accounts.auth_status != 'removed'
               AND messages.account_id = ?1
               AND messages.folder_id = ?2
               AND messages.id = ?3",
            params![request.account_id, request.folder_id, request.message_id],
            |row| {
                Ok(RemoteContentMessageRow {
                    sender_email: row.get(0)?,
                })
            },
        )
        .optional()?
        .ok_or(IpcValidationError::UnknownMessage)?;

    if matches!(request.scope, RemoteContentPreferenceScope::Sender) {
        let sender_email = row.sender_email.trim();
        validate_bounded_text("sender_email", sender_email, MAX_SENDER_EMAIL_BYTES)?;
        if sender_email.contains(char::is_whitespace)
            || sender_email.contains('<')
            || sender_email.contains('>')
            || sender_email.matches('@').count() != 1
            || sender_email.starts_with('@')
            || sender_email.ends_with('@')
        {
            return Err(MailShellError::new("message sender is unavailable"));
        }
    }

    Ok(row)
}

fn upsert_remote_content_preference(
    connection: &Connection,
    preference: &RemoteContentPreferenceRecord,
) -> Result<(), MailShellError> {
    match preference.scope {
        RemoteContentPreferenceScope::Message => {
            let existing_id = connection
                .query_row(
                    "SELECT id
                     FROM remote_content_preferences
                     WHERE account_id = ?1
                       AND scope = 'message'
                       AND message_id = ?2",
                    params![preference.account_id, preference.message_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            if let Some(existing_id) = existing_id {
                connection.execute(
                    "UPDATE remote_content_preferences
                     SET updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    [existing_id],
                )?;
                return Ok(());
            }

            connection.execute(
                "INSERT INTO remote_content_preferences (
                   id,
                   account_id,
                   scope,
                   message_id
                 )
                 VALUES (?1, ?2, 'message', ?3)",
                params![
                    stable_remote_content_preference_id(
                        &preference.account_id,
                        "message",
                        &preference.message_id
                    ),
                    preference.account_id,
                    preference.message_id
                ],
            )?;
        }
        RemoteContentPreferenceScope::Sender => {
            let sender_email = preference
                .sender_email
                .as_deref()
                .ok_or_else(|| MailShellError::new("message sender is unavailable"))?
                .trim()
                .to_string();
            let normalized_sender_email = sender_email.to_ascii_lowercase();
            let existing_id = connection
                .query_row(
                    "SELECT id
                     FROM remote_content_preferences
                     WHERE account_id = ?1
                       AND scope = 'sender'
                       AND normalized_sender_email = ?2",
                    params![preference.account_id, normalized_sender_email],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            if let Some(existing_id) = existing_id {
                connection.execute(
                    "UPDATE remote_content_preferences
                     SET sender_email = ?2,
                         updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![existing_id, sender_email],
                )?;
                return Ok(());
            }

            connection.execute(
                "INSERT INTO remote_content_preferences (
                   id,
                   account_id,
                   scope,
                   sender_email,
                   normalized_sender_email
                 )
                 VALUES (?1, ?2, 'sender', ?3, ?4)",
                params![
                    stable_remote_content_preference_id(
                        &preference.account_id,
                        "sender",
                        &normalized_sender_email
                    ),
                    preference.account_id,
                    sender_email,
                    normalized_sender_email
                ],
            )?;
        }
    }

    Ok(())
}

fn stable_remote_content_preference_id(account_id: &str, scope: &str, target: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update([0]);
    hasher.update(scope.as_bytes());
    hasher.update([0]);
    hasher.update(target.as_bytes());
    let hash = hasher.finalize();
    let mut id = String::from("rc_");
    for byte in hash.iter().take(16) {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

fn load_message_attachments_by_ids(
    connection: &Connection,
    message_ids: &[String],
) -> rusqlite::Result<HashMap<String, Vec<MailShellAttachment>>> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = (1..=message_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT
           attachments.message_id,
           attachments.id,
           COALESCE(NULLIF(attachments.display_filename, ''), '') AS display_filename,
           COALESCE(
             NULLIF(attachments.canonical_mime, ''),
             NULLIF(attachments.content_type, ''),
             'application/octet-stream'
           ) AS content_type,
           attachments.size_bytes,
           attachments.disposition
         FROM attachments
         WHERE attachments.message_id IN ({placeholders})
         ORDER BY
           attachments.message_id,
           CASE attachments.disposition WHEN 'inline' THEN 0 ELSE 1 END,
           display_filename COLLATE NOCASE,
           attachments.id"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(message_ids.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            MailShellAttachment {
                id: row.get(1)?,
                display_filename: safe_attachment_display_filename(row.get(2)?),
                content_type: safe_attachment_content_type(row.get(3)?),
                size_bytes: checked_u64(row.get::<_, i64>(4)?, "attachment size")?,
                disposition: normalize_attachment_disposition(row.get(5)?),
            },
        ))
    })?;

    collect_attachment_rows(rows)
}

fn collect_attachment_rows(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(String, MailShellAttachment)>,
    >,
) -> rusqlite::Result<HashMap<String, Vec<MailShellAttachment>>> {
    let mut attachments_by_message = HashMap::<String, Vec<MailShellAttachment>>::new();
    for row in rows {
        let (message_id, attachment) = row?;
        attachments_by_message
            .entry(message_id)
            .or_default()
            .push(attachment);
    }

    Ok(attachments_by_message)
}

fn messages_fts_available(connection: &Connection) -> rusqlite::Result<bool> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'messages_fts'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    Ok(exists)
}

fn active_account_exists(connection: &Connection, account_id: &str) -> rusqlite::Result<bool> {
    let exists = connection
        .query_row(
            "SELECT 1
             FROM accounts
             WHERE id = ?1
               AND removed_at IS NULL
               AND auth_status != 'removed'",
            [account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    Ok(exists)
}

fn folder_exists_in_scope(
    connection: &Connection,
    account_id: Option<&str>,
    folder_id: &str,
) -> rusqlite::Result<bool> {
    let exists = connection
        .query_row(
            "SELECT 1
             FROM folders
             JOIN accounts ON accounts.id = folders.account_id
             WHERE folders.id = ?1
               AND (?2 IS NULL OR folders.account_id = ?2)
               AND accounts.removed_at IS NULL
               AND accounts.auth_status != 'removed'",
            params![folder_id, account_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    Ok(exists)
}

fn parse_search_query(query: &str) -> Result<SearchQueryPlan, MailShellError> {
    let tokens = tokenize_search_query(query)?;
    let mut text_terms = Vec::new();
    let mut fts_terms = Vec::new();
    let mut filters = Vec::new();
    let mut sender_filter = None;
    let mut subject_filter = None;
    let mut body_filter = None;
    let mut has_attachment_filter = None;
    let mut read_filter = None;
    let mut flagged_filter = None;

    for token in tokens {
        match token {
            SearchQueryToken::Text(value) => {
                let terms = search_terms_from_text(&value);
                push_terms(&mut text_terms, &terms);
                push_terms(&mut fts_terms, &terms);
            }
            SearchQueryToken::Filter { kind, value } => {
                let value = normalize_search_value(&value);
                if value.is_empty() {
                    return Err(MailShellError::new("search filter value is required"));
                }

                match kind.as_str() {
                    SEARCH_FILTER_FROM => {
                        ensure_unique_filter(&sender_filter)?;
                        let terms = search_terms_from_text(&value);
                        ensure_filter_has_terms(&terms)?;
                        push_terms(&mut fts_terms, &terms);
                        sender_filter = Some(sql_like_contains(&value));
                        filters.push(MailSearchParsedFilter { kind, value });
                    }
                    SEARCH_FILTER_SUBJECT => {
                        ensure_unique_filter(&subject_filter)?;
                        let terms = search_terms_from_text(&value);
                        ensure_filter_has_terms(&terms)?;
                        push_terms(&mut fts_terms, &terms);
                        subject_filter = Some(sql_like_contains(&value));
                        filters.push(MailSearchParsedFilter { kind, value });
                    }
                    SEARCH_FILTER_BODY => {
                        ensure_unique_filter(&body_filter)?;
                        let terms = search_terms_from_text(&value);
                        ensure_filter_has_terms(&terms)?;
                        push_terms(&mut fts_terms, &terms);
                        body_filter = Some(sql_like_contains(&value));
                        filters.push(MailSearchParsedFilter { kind, value });
                    }
                    SEARCH_FILTER_HAS => {
                        let canonical = canonical_has_filter(&value)?;
                        if has_attachment_filter.is_some() {
                            return Err(MailShellError::new("duplicate search filter"));
                        }
                        has_attachment_filter = Some(true);
                        filters.push(MailSearchParsedFilter {
                            kind,
                            value: canonical.to_string(),
                        });
                    }
                    SEARCH_FILTER_IS => {
                        let canonical = canonical_is_filter(&value)?;
                        match canonical {
                            "read" => set_read_filter(&mut read_filter, true)?,
                            "unread" => set_read_filter(&mut read_filter, false)?,
                            "flagged" => set_flagged_filter(&mut flagged_filter, true)?,
                            _ => unreachable!("canonical is filter is constrained"),
                        }
                        filters.push(MailSearchParsedFilter {
                            kind,
                            value: canonical.to_string(),
                        });
                    }
                    _ => {
                        let terms = search_terms_from_text(&format!("{kind} {value}"));
                        push_terms(&mut text_terms, &terms);
                        push_terms(&mut fts_terms, &terms);
                    }
                }
            }
        }
    }

    if fts_terms.is_empty()
        && sender_filter.is_none()
        && subject_filter.is_none()
        && body_filter.is_none()
        && has_attachment_filter.is_none()
        && read_filter.is_none()
        && flagged_filter.is_none()
    {
        return Err(IpcValidationError::EmptyField { field: "query" }.into());
    }

    let highlight_terms = fts_terms.clone();
    let fts_query = if fts_terms.is_empty() {
        None
    } else {
        Some(
            fts_terms
                .iter()
                .take(MAX_SEARCH_TERMS)
                .map(|token| format!("{token}*"))
                .collect::<Vec<_>>()
                .join(" "),
        )
    };

    Ok(SearchQueryPlan {
        fts_query,
        parsed_query: MailSearchParsedQuery {
            filters,
            text_terms,
        },
        highlight_terms,
        sender_filter,
        subject_filter,
        body_filter,
        has_attachment_filter,
        read_filter,
        flagged_filter,
    })
}

fn tokenize_search_query(query: &str) -> Result<Vec<SearchQueryToken>, MailShellError> {
    let characters = query.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut tokens = Vec::new();

    while index < characters.len() {
        while index < characters.len() && characters[index].is_whitespace() {
            index += 1;
        }
        if index >= characters.len() {
            break;
        }

        if characters[index] == '"' {
            let (value, next_index) = read_quoted_search_value(&characters, index + 1)?;
            tokens.push(SearchQueryToken::Text(value));
            index = next_index;
            continue;
        }

        let token_start = index;
        while index < characters.len()
            && !characters[index].is_whitespace()
            && characters[index] != ':'
        {
            index += 1;
        }

        if index < characters.len() && characters[index] == ':' {
            let key = characters[token_start..index]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            if is_recognized_search_filter(&key) {
                let key = if key == "sender" {
                    SEARCH_FILTER_FROM.to_string()
                } else {
                    key
                };
                index += 1;
                if index >= characters.len() || characters[index].is_whitespace() {
                    return Err(MailShellError::new("search filter value is required"));
                }

                let (value, next_index) = if characters[index] == '"' {
                    read_quoted_search_value(&characters, index + 1)?
                } else {
                    let value_start = index;
                    while index < characters.len() && !characters[index].is_whitespace() {
                        index += 1;
                    }
                    (
                        characters[value_start..index].iter().collect::<String>(),
                        index,
                    )
                };
                tokens.push(SearchQueryToken::Filter { kind: key, value });
                index = next_index;
                continue;
            }

            while index < characters.len() && !characters[index].is_whitespace() {
                index += 1;
            }
            tokens.push(SearchQueryToken::Text(
                characters[token_start..index].iter().collect(),
            ));
            continue;
        }

        tokens.push(SearchQueryToken::Text(
            characters[token_start..index].iter().collect(),
        ));
    }

    Ok(tokens)
}

fn read_quoted_search_value(
    characters: &[char],
    mut index: usize,
) -> Result<(String, usize), MailShellError> {
    let value_start = index;
    while index < characters.len() && characters[index] != '"' {
        index += 1;
    }

    if index >= characters.len() {
        return Err(MailShellError::new("search query syntax is invalid"));
    }

    Ok((characters[value_start..index].iter().collect(), index + 1))
}

fn is_recognized_search_filter(key: &str) -> bool {
    matches!(
        key,
        SEARCH_FILTER_FROM
            | "sender"
            | SEARCH_FILTER_SUBJECT
            | SEARCH_FILTER_BODY
            | SEARCH_FILTER_HAS
            | SEARCH_FILTER_IS
    )
}

fn normalize_search_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn search_terms_from_text(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn push_terms(target: &mut Vec<String>, terms: &[String]) {
    for term in terms {
        if target.len() >= MAX_SEARCH_TERMS {
            break;
        }
        if !target.iter().any(|existing| existing == term) {
            target.push(term.clone());
        }
    }
}

fn ensure_unique_filter<T>(existing: &Option<T>) -> Result<(), MailShellError> {
    if existing.is_some() {
        return Err(MailShellError::new("duplicate search filter"));
    }

    Ok(())
}

fn ensure_filter_has_terms(terms: &[String]) -> Result<(), MailShellError> {
    if terms.is_empty() {
        return Err(MailShellError::new("search filter value is invalid"));
    }

    Ok(())
}

fn canonical_has_filter(value: &str) -> Result<&'static str, MailShellError> {
    match value.to_ascii_lowercase().as_str() {
        "attachment" | "attachments" => Ok("attachment"),
        _ => Err(MailShellError::new("unsupported search filter")),
    }
}

fn canonical_is_filter(value: &str) -> Result<&'static str, MailShellError> {
    match value.to_ascii_lowercase().as_str() {
        "read" => Ok("read"),
        "unread" => Ok("unread"),
        "flagged" | "starred" => Ok("flagged"),
        _ => Err(MailShellError::new("unsupported search filter")),
    }
}

fn set_read_filter(target: &mut Option<bool>, value: bool) -> Result<(), MailShellError> {
    match target {
        Some(existing) if *existing != value => {
            Err(MailShellError::new("conflicting search filter"))
        }
        Some(_) => Err(MailShellError::new("duplicate search filter")),
        None => {
            *target = Some(value);
            Ok(())
        }
    }
}

fn set_flagged_filter(target: &mut Option<bool>, value: bool) -> Result<(), MailShellError> {
    match target {
        Some(existing) if *existing != value => {
            Err(MailShellError::new("conflicting search filter"))
        }
        Some(_) => Err(MailShellError::new("duplicate search filter")),
        None => {
            *target = Some(value);
            Ok(())
        }
    }
}

fn sql_like_contains(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.to_lowercase().chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }

    format!("%{escaped}%")
}

fn optional_bool_i64(value: Option<bool>) -> Option<i64> {
    value.map(|value| if value { 1 } else { 0 })
}

fn reject_search_urls_and_paths(query: &str) -> Result<(), IpcValidationError> {
    let lowered = query.to_ascii_lowercase();
    if lowered.contains("://")
        || lowered.contains("javascript:")
        || lowered.contains("data:")
        || lowered.contains("file:")
    {
        return Err(IpcValidationError::UnsafeUrl {
            field: "query",
            reason: "URLs are not accepted in search queries",
        });
    }

    let bytes = query.as_bytes();
    let windows_drive_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    if windows_drive_path
        || query.contains("\\\\")
        || query.contains("..\\")
        || query.contains("../")
    {
        return Err(IpcValidationError::ClientSuppliedPath { field: "query" });
    }

    Ok(())
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> rusqlite::Result<Vec<T>> {
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn checked_u32(value: i64, label: &str) -> rusqlite::Result<u32> {
    u32::try_from(value)
        .map_err(|_| rusqlite::Error::InvalidColumnName(format!("{label} is out of range")))
}

fn checked_u64(value: i64, label: &str) -> rusqlite::Result<u64> {
    u64::try_from(value)
        .map_err(|_| rusqlite::Error::InvalidColumnName(format!("{label} is out of range")))
}

fn normalize_folder_kind(kind: String) -> String {
    match kind.as_str() {
        "inbox" | "sent" | "drafts" | "archive" | "trash" | "junk" | "custom" | "all"
        | "important" => kind,
        _ => "custom".to_string(),
    }
}

fn trim_preview(preview: String) -> String {
    let normalized = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.chars().take(180).collect()
}

fn safe_attachment_display_filename(filename: String) -> String {
    let leaf = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    let sanitized = leaf
        .chars()
        .map(|character| match character {
            '\0' | '\r' | '\n' | '\t' => ' ',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect::<String>();
    let sanitized = sanitized
        .trim()
        .trim_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let bounded = sanitized.chars().take(180).collect::<String>();

    bounded
}

fn safe_attachment_content_type(content_type: String) -> String {
    let sanitized = content_type
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let sanitized = sanitized.trim();

    if sanitized.is_empty() {
        "application/octet-stream".to_string()
    } else {
        sanitized.chars().take(120).collect()
    }
}

fn normalize_attachment_disposition(disposition: String) -> String {
    match disposition.as_str() {
        "inline" => "inline".to_string(),
        _ => "attachment".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open temp sqlite");
        apply_migrations(&mut connection).expect("apply migrations");
        connection
    }

    #[test]
    fn empty_cache_loads_empty_shell_snapshot() {
        let connection = migrated_connection();

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();

        assert_eq!(snapshot.accounts, Vec::<MailShellAccount>::new());
        assert_eq!(snapshot.folders, Vec::<MailShellFolder>::new());
        assert_eq!(snapshot.messages, Vec::<MailShellMessage>::new());
    }

    #[test]
    fn reads_accounts_folders_counts_and_recent_messages_from_local_cache() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();

        assert_eq!(
            snapshot.accounts,
            vec![
                MailShellAccount {
                    id: "acct_personal".to_string(),
                    label: "Personal".to_string(),
                    email: "person@example.com".to_string(),
                    display_name: Some("Personal".to_string()),
                    provider_kind: "gmail".to_string(),
                    auth_kind: "oauth2".to_string(),
                    status: "idle".to_string(),
                    status_updated_at: None,
                    last_successful_sync_at: None,
                    status_message: None,
                    status_action: None,
                    unread_count: 1,
                    signature_html: Some("<p>Personal signature</p>".to_string()),
                    local_preferences: MailShellAccountLocalPreferences {
                        include_in_unified_inbox: true,
                    },
                },
                MailShellAccount {
                    id: "acct_work".to_string(),
                    label: "work@example.com".to_string(),
                    email: "work@example.com".to_string(),
                    display_name: None,
                    provider_kind: "microsoft".to_string(),
                    auth_kind: "oauth2".to_string(),
                    status: "syncing".to_string(),
                    status_updated_at: None,
                    last_successful_sync_at: None,
                    status_message: None,
                    status_action: None,
                    unread_count: 2,
                    signature_html: None,
                    local_preferences: MailShellAccountLocalPreferences {
                        include_in_unified_inbox: true,
                    },
                },
            ]
        );
        assert_eq!(snapshot.folders.len(), 3);
        assert_eq!(snapshot.folders[0].id, "folder_personal_inbox");
        assert_eq!(snapshot.folders[0].unread_count, 1);
        assert_eq!(snapshot.folders[2].id, "folder_work_inbox");
        assert_eq!(snapshot.folders[2].unread_count, 2);
        assert_eq!(snapshot.messages.len(), 3);
        assert_eq!(snapshot.messages[0].id, "msg_work");
        assert!(snapshot.messages[0].unread);
        assert!(snapshot.messages[0].flagged);
        assert_eq!(
            snapshot.messages[0].attachments,
            vec![MailShellAttachment {
                id: "att_work_invoice".to_string(),
                display_filename: "invoice_.pdf".to_string(),
                content_type: "application/pdf".to_string(),
                size_bytes: 24576,
                disposition: "attachment".to_string(),
            }]
        );
        assert_eq!(
            snapshot.messages[0].preview,
            "Can you confirm the invoice number?"
        );
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.account_id != "acct_removed"));
    }

    #[test]
    fn account_sync_worker_status_feeds_shell_status_before_folders_exist() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);
        connection
            .execute(
                "INSERT INTO settings (key, value_type, value_json)
                 VALUES (
                   'account:acct_personal:sync_worker_status',
                   'object',
                   '{\"state\":\"syncing\",\"updatedAt\":1779552000}'
                 )",
                [],
            )
            .unwrap();

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let personal = snapshot
            .accounts
            .iter()
            .find(|account| account.id == "acct_personal")
            .unwrap();

        assert_eq!(personal.status, "syncing");
        assert_eq!(
            personal.status_updated_at.as_deref(),
            Some("2026-05-23T16:00:00Z")
        );
    }

    #[test]
    fn account_sync_worker_status_exposes_redacted_detail_and_last_success() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);
        connection
            .execute(
                "INSERT INTO settings (key, value_type, value_json)
                 VALUES (
                   'account:acct_personal:sync_worker_status',
                   'object',
                   '{\"state\":\"error\",\"updatedAt\":1779552060,\"lastSuccessfulSyncAt\":1779548400,\"message\":\"Reauthentication is required for this account.\",\"action\":\"reauthenticate\"}'
                 )",
                [],
            )
            .unwrap();

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let personal = snapshot
            .accounts
            .iter()
            .find(|account| account.id == "acct_personal")
            .unwrap();

        assert_eq!(personal.status, "error");
        assert_eq!(
            personal.status_updated_at.as_deref(),
            Some("2026-05-23T16:01:00Z")
        );
        assert_eq!(
            personal.last_successful_sync_at.as_deref(),
            Some("2026-05-23T15:00:00Z")
        );
        assert_eq!(
            personal.status_message.as_deref(),
            Some("Reauthentication is required for this account.")
        );
        assert_eq!(personal.status_action.as_deref(), Some("reauthenticate"));
    }

    #[test]
    fn serialized_snapshot_does_not_expose_credential_references() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();

        assert!(!json.contains("credential_ref"));
        assert!(!json.contains("Komorebi/mail-account"));
        assert!(!json.contains("storage_relative_path"));
        assert!(!json.contains("att_work_invoice.pdf"));
        assert!(!json.contains("invoice?.pdf"));
        assert!(!json.contains("attachments/acct_work"));
        assert!(json.contains("person@example.com"));
        assert!(json.contains("invoice_.pdf"));
    }

    #[test]
    fn missing_display_labels_are_left_for_frontend_i18n_fallbacks() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);
        connection
            .execute_batch(
                r#"
                INSERT INTO messages (
                  id,
                  account_id,
                  folder_id,
                  imap_uid,
                  uid_validity,
                  subject,
                  sender_name,
                  sender_email,
                  date_received,
                  is_read,
                  is_flagged,
                  body_text,
                  body_html,
                  snippet
                )
                VALUES (
                  'msg_missing_labels',
                  'acct_personal',
                  'folder_personal_inbox',
                  4,
                  10,
                  '',
                  NULL,
                  NULL,
                  '2026-05-23T12:00:00Z',
                  1,
                  0,
                  'Fallback label search text.',
                  NULL,
                  'Fallback label search text.'
                );

                INSERT INTO attachments (
                  id,
                  account_id,
                  folder_id,
                  message_id,
                  content_id,
                  display_filename,
                  canonical_mime,
                  size_bytes,
                  storage_relative_path,
                  disposition
                )
                VALUES (
                  'att_missing_label',
                  'acct_personal',
                  'folder_personal_inbox',
                  'msg_missing_labels',
                  NULL,
                  '...',
                  'text/plain',
                  12,
                  'attachments/acct_personal/folder_personal_inbox/msg_missing_labels/att_missing_label.txt',
                  'attachment'
                );
                "#,
            )
            .unwrap();

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let message = snapshot
            .messages
            .iter()
            .find(|message| message.id == "msg_missing_labels")
            .expect("missing-label message should be loaded");

        assert_eq!(message.sender, "");
        assert_eq!(message.attachments[0].display_filename, "");

        let search = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "fallback label".to_string(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();
        let search_message = search
            .results
            .iter()
            .find(|message| message.id == "msg_missing_labels")
            .expect("missing-label message should be searchable");
        assert_eq!(search_message.sender, "");
        assert_eq!(search_message.attachments[0].display_filename, "");
    }

    #[test]
    fn load_message_page_reads_scoped_cache_incrementally_without_duplicates() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);
        for index in 0..60 {
            connection
                .execute(
                    "INSERT INTO messages (
                       id,
                       account_id,
                       folder_id,
                       imap_uid,
                       uid_validity,
                       subject,
                       sender_name,
                       sender_email,
                       date_received,
                       is_read,
                       is_flagged,
                       body_text,
                       snippet
                     )
                     VALUES (?1, 'acct_personal', 'folder_personal_inbox', ?2, 10, ?3, 'Pager', 'pager@example.com', ?4, 1, 0, ?5, ?5)",
                    params![
                        format!("msg_page_{index:03}"),
                        1000 + index,
                        format!("Paged message {index:03}"),
                        format!("2026-05-24T00:{index:02}:00Z"),
                        format!("Paged body {index:03}")
                    ],
                )
                .unwrap();
        }

        let first_page = load_message_page_from_connection(
            &connection,
            MailMessagePageRequest {
                account_id: Some("acct_personal".to_string()),
                folder_id: Some("folder_personal_inbox".to_string()),
                unified_inbox: false,
                cursor: None,
                limit: Some(25),
            },
        )
        .unwrap();

        assert_eq!(first_page.total_count, 61);
        assert_eq!(first_page.messages.len(), 25);
        assert!(first_page.has_more);
        assert_eq!(first_page.messages[0].id, "msg_page_059");
        assert_eq!(first_page.messages[24].id, "msg_page_035");
        assert!(first_page
            .messages
            .iter()
            .all(|message| message.account_id == "acct_personal"
                && message.folder_id == "folder_personal_inbox"));

        let second_page = load_message_page_from_connection(
            &connection,
            MailMessagePageRequest {
                account_id: Some("acct_personal".to_string()),
                folder_id: Some("folder_personal_inbox".to_string()),
                unified_inbox: false,
                cursor: first_page.next_cursor.clone(),
                limit: Some(25),
            },
        )
        .unwrap();
        let first_ids = first_page
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(second_page.messages.len(), 25);
        assert_eq!(second_page.messages[0].id, "msg_page_034");
        assert!(second_page
            .messages
            .iter()
            .all(|message| !first_ids.contains(message.id.as_str())));
        assert!(second_page.has_more);
    }

    #[test]
    fn load_message_page_validates_scope_and_cursor_before_querying() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let mixed_scope = load_message_page_from_connection(
            &connection,
            MailMessagePageRequest {
                account_id: Some("acct_personal".to_string()),
                folder_id: None,
                unified_inbox: true,
                cursor: None,
                limit: Some(25),
            },
        )
        .unwrap_err();
        assert_eq!(
            mixed_scope.reason,
            "unified message pages cannot include account or folder scope"
        );

        let unowned_folder = load_message_page_from_connection(
            &connection,
            MailMessagePageRequest {
                account_id: Some("acct_work".to_string()),
                folder_id: Some("folder_personal_inbox".to_string()),
                unified_inbox: false,
                cursor: None,
                limit: Some(25),
            },
        )
        .unwrap_err();
        assert_eq!(unowned_folder.reason, "folder was not found for account");

        let bad_cursor = load_message_page_from_connection(
            &connection,
            MailMessagePageRequest {
                account_id: Some("acct_personal".to_string()),
                folder_id: None,
                unified_inbox: false,
                cursor: Some(MailMessagePageCursor {
                    sort_key: "2026-05-24'; DELETE FROM messages; --".to_string(),
                    message_id: "msg_personal".to_string(),
                }),
                limit: Some(25),
            },
        )
        .unwrap_err();
        assert!(!bad_cursor.reason.contains("DELETE FROM messages"));

        let cursor_outside_scope = load_message_page_from_connection(
            &connection,
            MailMessagePageRequest {
                account_id: Some("acct_personal".to_string()),
                folder_id: None,
                unified_inbox: false,
                cursor: Some(MailMessagePageCursor {
                    sort_key: "2026-05-22T10:30:00Z".to_string(),
                    message_id: "msg_work".to_string(),
                }),
                limit: Some(25),
            },
        )
        .unwrap_err();
        assert_eq!(
            cursor_outside_scope.reason,
            "message page cursor is invalid"
        );
    }

    #[test]
    fn search_mail_uses_fts_across_active_accounts_and_returns_safe_messages() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let invoice_search = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice number".to_string(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();

        assert_eq!(invoice_search.index_status, MailSearchIndexStatus::Ready);
        assert_eq!(invoice_search.query, "invoice number");
        assert_eq!(invoice_search.results.len(), 1);
        assert_eq!(invoice_search.results[0].id, "msg_work");
        assert_eq!(invoice_search.results[0].account_id, "acct_work");
        assert_eq!(invoice_search.results[0].folder_id, "folder_work_inbox");
        assert_eq!(invoice_search.results[0].sender, "Finance");
        assert_eq!(
            invoice_search.results[0].attachments,
            vec![MailShellAttachment {
                id: "att_work_invoice".to_string(),
                display_filename: "invoice_.pdf".to_string(),
                content_type: "application/pdf".to_string(),
                size_bytes: 24576,
                disposition: "attachment".to_string(),
            }]
        );

        let account_scoped = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice".to_string(),
                account_id: Some("acct_personal".to_string()),
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert!(account_scoped.results.is_empty());

        let archive_search = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "receipt".to_string(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert_eq!(archive_search.results.len(), 1);
        assert_eq!(archive_search.results[0].id, "msg_archive");
        assert_eq!(
            archive_search.results[0].folder_id,
            "folder_personal_archive"
        );
    }

    #[test]
    fn search_mail_parses_advanced_filters_and_metadata_queries_safely() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let advanced_search = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "from:finance subject:\"Invoice question\" is:unread has:attachment"
                    .to_string(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .expect("advanced search succeeds");

        assert_eq!(advanced_search.index_status, MailSearchIndexStatus::Ready);
        assert_eq!(
            advanced_search.parsed_query.filters,
            vec![
                MailSearchParsedFilter {
                    kind: "from".to_string(),
                    value: "finance".to_string(),
                },
                MailSearchParsedFilter {
                    kind: "subject".to_string(),
                    value: "Invoice question".to_string(),
                },
                MailSearchParsedFilter {
                    kind: "is".to_string(),
                    value: "unread".to_string(),
                },
                MailSearchParsedFilter {
                    kind: "has".to_string(),
                    value: "attachment".to_string(),
                },
            ]
        );
        assert_eq!(
            advanced_search.highlight_terms,
            vec![
                "finance".to_string(),
                "invoice".to_string(),
                "question".to_string()
            ]
        );
        assert_eq!(advanced_search.results.len(), 1);
        assert_eq!(advanced_search.results[0].id, "msg_work");

        let metadata_only = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "has:attachment is:flagged".to_string(),
                account_id: Some("acct_work".to_string()),
                folder_id: None,
                limit: Some(10),
            },
        )
        .expect("metadata-only advanced search succeeds");

        assert_eq!(metadata_only.highlight_terms, Vec::<String>::new());
        assert_eq!(metadata_only.results.len(), 1);
        assert_eq!(metadata_only.results[0].id, "msg_work");

        let scoped_metadata_only = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "has:attachment".to_string(),
                account_id: Some("acct_personal".to_string()),
                folder_id: None,
                limit: Some(10),
            },
        )
        .expect("metadata-only search remains account scoped");

        assert!(scoped_metadata_only.results.is_empty());
    }

    #[test]
    fn account_local_preferences_exclude_accounts_from_unified_search_only() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);
        connection
            .execute(
                "INSERT INTO settings (key, value_json, value_type)
                 VALUES (
                   'account:acct_work:local_preferences',
                   '{\"includeInUnifiedInbox\":false}',
                   'object'
                 )",
                [],
            )
            .unwrap();

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let work_account = snapshot
            .accounts
            .iter()
            .find(|account| account.id == "acct_work")
            .expect("work account loaded");
        assert!(!work_account.local_preferences.include_in_unified_inbox);

        let unified_search = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice".to_string(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert!(unified_search.results.is_empty());

        let account_search = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice".to_string(),
                account_id: Some("acct_work".to_string()),
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();
        assert_eq!(account_search.results.len(), 1);
        assert_eq!(account_search.results[0].id, "msg_work");
    }

    #[test]
    fn remote_content_preferences_are_persisted_for_message_and_sender_scopes() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let message_response = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "acct_work".to_string(),
                folder_id: "folder_work_inbox".to_string(),
                message_id: "msg_work".to_string(),
                scope: RemoteContentPreferenceScope::Message,
                user_requested: true,
            },
        )
        .expect("message preference persists");

        assert_eq!(
            message_response,
            RemoteContentPreferenceResponse {
                account_id: "acct_work".to_string(),
                message_id: "msg_work".to_string(),
                scope: RemoteContentPreferenceScope::Message,
                sender_email: None,
                remote_content_allowed: true,
            }
        );

        let snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let work_message = snapshot
            .messages
            .iter()
            .find(|message| message.id == "msg_work")
            .expect("work message loaded");
        assert!(work_message.remote_content_allowed);
        assert_eq!(
            work_message.remote_content_allowed_scope.as_deref(),
            Some("message")
        );

        let sender_response = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "acct_personal".to_string(),
                folder_id: "folder_personal_inbox".to_string(),
                message_id: "msg_personal".to_string(),
                scope: RemoteContentPreferenceScope::Sender,
                user_requested: true,
            },
        )
        .expect("sender preference persists");
        assert_eq!(
            sender_response.sender_email.as_deref(),
            Some("ada@example.com")
        );

        let sender_snapshot = load_mail_shell_from_connection(&connection).unwrap();
        let personal_message = sender_snapshot
            .messages
            .iter()
            .find(|message| message.id == "msg_personal")
            .expect("personal message loaded");
        assert!(personal_message.remote_content_allowed);
        assert_eq!(
            personal_message.remote_content_allowed_scope.as_deref(),
            Some("sender")
        );

        let sender_row_count: i64 = connection
            .query_row(
                "SELECT COUNT(*)
                 FROM remote_content_preferences
                 WHERE account_id = 'acct_personal'
                   AND scope = 'sender'
                   AND normalized_sender_email = 'ada@example.com'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sender_row_count, 1);
    }

    #[test]
    fn remote_content_preferences_validate_scope_and_intent_before_persistence() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        let no_intent = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "acct_work".to_string(),
                folder_id: "folder_work_inbox".to_string(),
                message_id: "msg_work".to_string(),
                scope: RemoteContentPreferenceScope::Message,
                user_requested: false,
            },
        )
        .expect_err("user intent is required");
        assert_eq!(
            no_intent.reason,
            "remote content preference requires user intent"
        );

        let invalid_account = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "../secret".to_string(),
                folder_id: "folder_work_inbox".to_string(),
                message_id: "msg_work".to_string(),
                scope: RemoteContentPreferenceScope::Message,
                user_requested: true,
            },
        )
        .expect_err("invalid account id is rejected");
        assert!(invalid_account.reason.contains("account_id is invalid"));

        let unowned_folder = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "acct_work".to_string(),
                folder_id: "folder_personal_inbox".to_string(),
                message_id: "msg_work".to_string(),
                scope: RemoteContentPreferenceScope::Message,
                user_requested: true,
            },
        )
        .expect_err("unowned folder is rejected");
        assert_eq!(unowned_folder.reason, "folder was not found for account");

        let missing_message = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "acct_work".to_string(),
                folder_id: "folder_work_inbox".to_string(),
                message_id: "msg_missing".to_string(),
                scope: RemoteContentPreferenceScope::Message,
                user_requested: true,
            },
        )
        .expect_err("missing message is rejected");
        assert_eq!(missing_message.reason, "message was not found for folder");

        connection
            .execute(
                "INSERT INTO messages (
                   id,
                   account_id,
                   folder_id,
                   imap_uid,
                   uid_validity,
                   subject,
                   sender_email
                 )
                 VALUES (
                   'msg_no_sender',
                   'acct_personal',
                   'folder_personal_inbox',
                   4,
                   10,
                   'No sender',
                   ''
                 )",
                [],
            )
            .expect("seed no-sender message");
        let sender_unavailable = allow_remote_content_from_connection(
            &connection,
            RemoteContentPreferenceRequest {
                account_id: "acct_personal".to_string(),
                folder_id: "folder_personal_inbox".to_string(),
                message_id: "msg_no_sender".to_string(),
                scope: RemoteContentPreferenceScope::Sender,
                user_requested: true,
            },
        )
        .expect_err("sender preference requires a usable sender email");
        assert_eq!(sender_unavailable.reason, "sender_email is required");
    }

    #[test]
    fn search_mail_reports_unavailable_when_fts_index_is_missing() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);
        connection.execute("DROP TABLE messages_fts", []).unwrap();

        let response = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice".to_string(),
                account_id: None,
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap();

        assert_eq!(response.index_status, MailSearchIndexStatus::Unavailable);
        assert!(response.results.is_empty());
    }

    #[test]
    fn search_mail_validates_query_and_scope_before_fts_execution() {
        let connection = migrated_connection();
        seed_shell_cache(&connection);

        for query in [
            "SELECT * FROM messages",
            "<img src=x onerror=alert(1)>",
            "subject; DELETE FROM messages",
            "https://example.com/phishing",
            "C:\\Users\\Alice\\mail.db",
            "from:",
            "has:script",
            "is:deleted",
        ] {
            let error = search_mail_from_connection(
                &connection,
                MailSearchRequest {
                    query: query.to_string(),
                    account_id: None,
                    folder_id: None,
                    limit: Some(10),
                },
            )
            .unwrap_err()
            .reason;

            assert!(!error.contains(query));
        }

        let invalid_account = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice".to_string(),
                account_id: Some("acct_work/../../secrets".to_string()),
                folder_id: None,
                limit: Some(10),
            },
        )
        .unwrap_err();
        assert!(invalid_account.reason.contains("account_id is invalid"));

        let unknown_folder = search_mail_from_connection(
            &connection,
            MailSearchRequest {
                query: "invoice".to_string(),
                account_id: Some("acct_work".to_string()),
                folder_id: Some("folder_personal_archive".to_string()),
                limit: Some(10),
            },
        )
        .unwrap_err();
        assert_eq!(unknown_folder.reason, "folder was not found for account");
    }

    fn seed_shell_cache(connection: &Connection) {
        connection
            .execute_batch(
                r#"
                INSERT INTO accounts (
                  id,
                  provider_kind,
                  display_email,
                  display_name,
                  username,
                  auth_kind,
                  credential_ref,
                  auth_status,
                  signature_html,
                  removed_at
                )
                VALUES
                  (
                    'acct_personal',
                    'gmail',
                    'person@example.com',
                    'Personal',
                    'person@example.com',
                    'oauth2',
                    'Komorebi/mail-account/acct_personal/oauth-refresh',
                    'connected',
                    '<p>Personal signature</p>',
                    NULL
                  ),
                  (
                    'acct_work',
                    'microsoft',
                    'work@example.com',
                    NULL,
                    'work@example.com',
                    'oauth2',
                    'Komorebi/mail-account/acct_work/oauth-refresh',
                    'connected',
                    NULL,
                    NULL
                  ),
                  (
                    'acct_removed',
                    'gmail',
                    'removed@example.com',
                    'Removed',
                    'removed@example.com',
                    'oauth2',
                    'Komorebi/mail-account/acct_removed/oauth-refresh',
                    'removed',
                    NULL,
                    CURRENT_TIMESTAMP
                  );

                INSERT INTO folders (
                  id,
                  account_id,
                  remote_id,
                  name,
                  display_name,
                  kind,
                  unread_count,
                  total_count,
                  sync_state
                )
                VALUES
                  (
                    'folder_personal_inbox',
                    'acct_personal',
                    'INBOX',
                    'INBOX',
                    'Inbox',
                    'inbox',
                    1,
                    2,
                    'idle'
                  ),
                  (
                    'folder_personal_archive',
                    'acct_personal',
                    'Archive',
                    'Archive',
                    'Archive',
                    'archive',
                    0,
                    1,
                    'idle'
                  ),
                  (
                    'folder_work_inbox',
                    'acct_work',
                    'INBOX',
                    'INBOX',
                    'Inbox',
                    'inbox',
                    2,
                    2,
                    'syncing'
                  );

                INSERT INTO messages (
                  id,
                  account_id,
                  folder_id,
                  imap_uid,
                  uid_validity,
                  subject,
                  sender_name,
                  sender_email,
                  date_received,
                  is_read,
                  is_flagged,
                  body_text,
                  body_html,
                  snippet
                )
                VALUES
                  (
                    'msg_personal',
                    'acct_personal',
                    'folder_personal_inbox',
                    1,
                    10,
                    'Roadmap review',
                    'Ada',
                    'ada@example.com',
                    '2026-05-22T09:30:00Z',
                    0,
                    0,
                    'Review notes for the local cache work.',
                    NULL,
                    'Review notes for the local cache work.'
                  ),
                  (
                    'msg_archive',
                    'acct_personal',
                    'folder_personal_archive',
                    2,
                    10,
                    'Archived receipt',
                    'Archive Bot',
                    'archive@example.com',
                    '2026-05-21T09:30:00Z',
                    1,
                    0,
                    'Stored outside the unified inbox.',
                    NULL,
                    NULL
                  ),
                  (
                    'msg_work',
                    'acct_work',
                    'folder_work_inbox',
                    3,
                    20,
                    'Invoice question',
                    'Finance',
                    'finance@example.com',
                    '2026-05-22T10:30:00Z',
                    0,
                    1,
                    'Can you confirm the invoice number?',
                    '<p>Can you confirm the invoice number?</p>',
                    'Can you confirm the invoice number?'
                  );

                INSERT INTO attachments (
                  id,
                  account_id,
                  folder_id,
                  message_id,
                  content_id,
                  display_filename,
                  canonical_mime,
                  size_bytes,
                  storage_relative_path,
                  disposition
                )
                VALUES (
                  'att_work_invoice',
                  'acct_work',
                  'folder_work_inbox',
                  'msg_work',
                  NULL,
                  '..\invoice?.pdf',
                  'application/pdf',
                  24576,
                  'attachments/acct_work/folder_work_inbox/msg_work/att_work_invoice.pdf',
                  'attachment'
                );
                "#,
            )
            .unwrap();
    }
}
