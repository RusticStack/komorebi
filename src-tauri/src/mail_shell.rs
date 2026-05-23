use std::{collections::HashMap, fs, path::Path};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::Manager;

use crate::schema::{apply_migrations, SchemaMigrationError};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const MESSAGE_SNAPSHOT_LIMIT: i64 = 500;

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
            status: row.get(3)?,
            unread_count: checked_u32(row.get::<_, i64>(4)?, "account unread count")?,
            signature_html: row.get(5)?,
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
    let mut attachments_by_message = load_message_attachments(connection)?;
    let mut statement = connection.prepare(
        "SELECT
           messages.id,
           messages.account_id,
           messages.folder_id,
           COALESCE(NULLIF(messages.sender_name, ''), NULLIF(messages.sender_email, ''), '(unknown sender)') AS sender,
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
        let id: String = row.get(0)?;
        let attachments = attachments_by_message.remove(&id).unwrap_or_default();

        Ok(MailShellMessage {
            id,
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
            attachments,
        })
    })?;

    collect_rows(rows)
}

fn load_message_attachments(
    connection: &Connection,
) -> rusqlite::Result<HashMap<String, Vec<MailShellAttachment>>> {
    let mut statement = connection.prepare(
        "WITH visible_messages AS (
           SELECT
             messages.id,
             messages.account_id,
             messages.folder_id
           FROM messages
           JOIN accounts ON accounts.id = messages.account_id
           JOIN folders ON folders.account_id = messages.account_id
            AND folders.id = messages.folder_id
           WHERE accounts.removed_at IS NULL
             AND accounts.auth_status != 'removed'
           ORDER BY
             COALESCE(messages.date_received, messages.internal_date, messages.created_at) DESC,
             messages.id
           LIMIT ?1
         )
         SELECT
           attachments.message_id,
           attachments.id,
           COALESCE(NULLIF(attachments.display_filename, ''), 'Attachment') AS display_filename,
           COALESCE(
             NULLIF(attachments.canonical_mime, ''),
             NULLIF(attachments.content_type, ''),
             'application/octet-stream'
           ) AS content_type,
           attachments.size_bytes,
           attachments.disposition
         FROM attachments
         JOIN visible_messages ON visible_messages.account_id = attachments.account_id
          AND visible_messages.folder_id = attachments.folder_id
          AND visible_messages.id = attachments.message_id
         ORDER BY
           attachments.message_id,
           CASE attachments.disposition WHEN 'inline' THEN 0 ELSE 1 END,
           display_filename COLLATE NOCASE,
           attachments.id",
    )?;

    let rows = statement.query_map(params![MESSAGE_SNAPSHOT_LIMIT], |row| {
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

    if bounded.is_empty() {
        "Attachment".to_string()
    } else {
        bounded
    }
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
                    status: "connected".to_string(),
                    unread_count: 1,
                    signature_html: Some("<p>Personal signature</p>".to_string()),
                },
                MailShellAccount {
                    id: "acct_work".to_string(),
                    label: "work@example.com".to_string(),
                    email: "work@example.com".to_string(),
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
