use std::{collections::HashMap, fs, path::Path};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::ipc_validation::{
    reject_raw_sql, reject_unsanitized_html, validate_bounded_text, validate_entity_id,
    IpcValidationError,
};
use crate::schema::{apply_migrations, SchemaMigrationError};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const MESSAGE_SNAPSHOT_LIMIT: i64 = 500;
const MAIL_SEARCH_RESULT_LIMIT: u32 = 100;
const MAX_SEARCH_QUERY_BYTES: usize = 512;
const MAX_SEARCH_TERMS: usize = 12;

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
    pub provider_kind: String,
    pub auth_kind: String,
    pub status: String,
    pub unread_count: u32,
    pub signature_html: Option<String>,
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
    pub attachments: Vec<MailShellAttachment>,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailSearchResponse {
    pub query: String,
    pub index_status: MailSearchIndexStatus,
    pub results: Vec<MailShellMessage>,
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
        });
    }

    let mut messages = search_messages(connection, &validated_request)?;
    attach_message_attachments(connection, &mut messages)?;

    Ok(MailSearchResponse {
        query: validated_request.query,
        index_status: MailSearchIndexStatus::Ready,
        results: messages,
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
           accounts.provider_kind,
           accounts.auth_kind,
           CASE
             WHEN accounts.auth_status = 'reconnect_needed' THEN 'reconnect-needed'
             WHEN accounts.auth_status = 'connected'
              AND EXISTS (
                SELECT 1
                FROM folders
                WHERE folders.account_id = accounts.id
                  AND folders.sync_state = 'syncing'
              ) THEN 'syncing'
             WHEN accounts.auth_status = 'connected' THEN 'connected'
             ELSE 'offline'
           END AS shell_status,
           COALESCE(
             (
               SELECT SUM(folders.unread_count)
               FROM folders
               WHERE folders.account_id = accounts.id
             ),
             0
           ) AS unread_count,
           accounts.signature_html
         FROM accounts
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
         ORDER BY accounts.display_email COLLATE NOCASE, accounts.id",
    )?;

    let rows = statement.query_map([], |row| {
        Ok(MailShellAccount {
            id: row.get(0)?,
            label: row.get(1)?,
            email: row.get(2)?,
            provider_kind: row.get(3)?,
            auth_kind: row.get(4)?,
            status: row.get(5)?,
            unread_count: checked_u32(row.get::<_, i64>(6)?, "account unread count")?,
            signature_html: row.get(7)?,
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
           messages.body_html
         FROM messages
         JOIN accounts ON accounts.id = messages.account_id
         JOIN folders ON folders.account_id = messages.account_id
          AND folders.id = messages.folder_id
         WHERE accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
         ORDER BY
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) DESC,
           messages.id
         LIMIT ?1",
    )?;

    let rows = statement.query_map(params![MESSAGE_SNAPSHOT_LIMIT], |row| {
        let preview = trim_preview(row.get::<_, String>(6)?);

        Ok(MailShellMessage {
            id: row.get(0)?,
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
            attachments: Vec::new(),
        })
    })?;

    let mut messages = collect_rows(rows)?;
    attach_message_attachments(connection, &mut messages)?;

    Ok(messages)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedSearchRequest {
    query: String,
    fts_query: String,
    account_id: Option<String>,
    folder_id: Option<String>,
    limit: u32,
}

fn validate_search_request(
    connection: &Connection,
    request: MailSearchRequest,
) -> Result<ValidatedSearchRequest, MailShellError> {
    validate_bounded_text("query", &request.query, MAX_SEARCH_QUERY_BYTES)?;
    reject_raw_sql("query", &request.query)?;
    reject_unsanitized_html("query", &request.query)?;

    let query = request.query.trim().to_string();
    let fts_query = fts_query_from_user_input(&query)?;

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
        fts_query,
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
           bm25(messages_fts) AS rank
         FROM messages_fts
         JOIN messages ON messages.rowid = messages_fts.rowid
         JOIN accounts ON accounts.id = messages.account_id
         JOIN folders ON folders.account_id = messages.account_id
          AND folders.id = messages.folder_id
         WHERE messages_fts MATCH ?1
           AND accounts.removed_at IS NULL
           AND accounts.auth_status != 'removed'
           AND (?2 IS NULL OR messages.account_id = ?2)
           AND (?3 IS NULL OR messages.folder_id = ?3)
         ORDER BY
           rank,
           COALESCE(messages.date_received, messages.internal_date, messages.created_at) DESC,
           messages.id
         LIMIT ?4",
    )?;

    let rows = statement.query_map(
        params![
            request.fts_query.as_str(),
            request.account_id.as_deref(),
            request.folder_id.as_deref(),
            request.limit
        ],
        |row| {
            let preview = trim_preview(row.get::<_, String>(6)?);

            Ok(MailShellMessage {
                id: row.get(0)?,
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
                attachments: Vec::new(),
            })
        },
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

fn fts_query_from_user_input(query: &str) -> Result<String, IpcValidationError> {
    let tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .take(MAX_SEARCH_TERMS)
        .map(|token| format!("{}*", token.to_lowercase()))
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return Err(IpcValidationError::EmptyField { field: "query" });
    }

    Ok(tokens.join(" "))
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
                    provider_kind: "gmail".to_string(),
                    auth_kind: "oauth2".to_string(),
                    status: "connected".to_string(),
                    unread_count: 1,
                    signature_html: Some("<p>Personal signature</p>".to_string()),
                },
                MailShellAccount {
                    id: "acct_work".to_string(),
                    label: "work@example.com".to_string(),
                    email: "work@example.com".to_string(),
                    provider_kind: "microsoft".to_string(),
                    auth_kind: "oauth2".to_string(),
                    status: "syncing".to_string(),
                    unread_count: 2,
                    signature_html: None,
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
