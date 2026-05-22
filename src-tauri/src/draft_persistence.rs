use std::{error::Error, fmt};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    auth::ProviderKind,
    ipc_validation::{
        validate_bounded_text, validate_entity_id, validate_payload_size, IpcValidationError,
        MAX_TEXT_FIELD_BYTES,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftState {
    Active,
    Discarded,
    Sent,
}

impl DraftState {
    pub const fn schema_value(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Discarded => "discarded",
            Self::Sent => "sent",
        }
    }

    fn from_schema(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "discarded" => Some(Self::Discarded),
            "sent" => Some(Self::Sent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentFolderPolicy {
    ProviderAutoCopies,
    ImapAppend,
    LocalReceiptOnly,
}

impl SentFolderPolicy {
    pub const fn schema_value(self) -> &'static str {
        match self {
            Self::ProviderAutoCopies => "provider_auto_copies",
            Self::ImapAppend => "imap_append",
            Self::LocalReceiptOnly => "local_receipt_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentFolderSyncState {
    NotRequired,
    PendingUpload,
    Uploaded,
    Unsupported,
    Failed,
}

impl SentFolderSyncState {
    pub const fn schema_value(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::PendingUpload => "pending_upload",
            Self::Uploaded => "uploaded",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftHtml(String);

impl DraftHtml {
    pub fn new_sanitized(value: impl Into<String>) -> Result<Self, DraftPersistenceError> {
        let value = value.into();
        validate_payload_size("body_html", value.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
        if value.trim().is_empty() {
            return Err(DraftPersistenceError::InvalidDraft {
                reason: "body_html is empty",
            });
        }

        let lowered = value.to_ascii_lowercase();
        if lowered.contains("<script")
            || lowered.contains("javascript:")
            || lowered.contains(" onload=")
            || lowered.contains(" onclick=")
            || lowered.contains(" onerror=")
        {
            return Err(DraftPersistenceError::InvalidDraft {
                reason: "body_html is not sanitized",
            });
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAutosaveRequest {
    pub account_id: String,
    pub draft_id: String,
    pub subject: String,
    pub to_addresses_json: String,
    pub cc_addresses_json: String,
    pub bcc_addresses_json: String,
    pub reply_to_addresses_json: String,
    pub body_text: Option<String>,
    pub body_html: Option<DraftHtml>,
    pub attachment_ids_json: String,
    pub in_reply_to: Option<String>,
    pub references_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerDraftRecord {
    pub id: String,
    pub account_id: String,
    pub draft_state: DraftState,
    pub subject: String,
    pub to_addresses_json: String,
    pub cc_addresses_json: String,
    pub bcc_addresses_json: String,
    pub reply_to_addresses_json: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachment_ids_json: String,
    pub in_reply_to: Option<String>,
    pub references_json: String,
    pub autosave_revision: u64,
    pub sent_receipt_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardDraftRequest {
    pub account_id: String,
    pub draft_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedSentDraft {
    pub account_id: String,
    pub draft_id: String,
    pub send_receipt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMailCopyRecord {
    pub id: String,
    pub send_receipt_id: String,
    pub account_id: String,
    pub rfc_message_id: String,
    pub provider_kind: ProviderKind,
    pub sent_folder_policy: SentFolderPolicy,
    pub sync_state: SentFolderSyncState,
    pub sent_folder_id: Option<String>,
    pub sent_folder_remote_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftPersistenceError {
    IpcValidation(IpcValidationError),
    UnknownAccount,
    UnknownDraft { draft_id: String },
    DraftNotActive { draft_id: String },
    InvalidDraft { reason: &'static str },
    Database(String),
}

impl fmt::Display for DraftPersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::UnknownAccount => write!(formatter, "account was not found"),
            Self::UnknownDraft { .. } => write!(formatter, "draft was not found"),
            Self::DraftNotActive { .. } => write!(formatter, "draft is not active"),
            Self::InvalidDraft { reason } => write!(formatter, "draft is invalid: {reason}"),
            Self::Database(error) => write!(formatter, "draft persistence failed: {error}"),
        }
    }
}

impl Error for DraftPersistenceError {}

impl From<IpcValidationError> for DraftPersistenceError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

impl From<rusqlite::Error> for DraftPersistenceError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value.to_string())
    }
}

pub fn autosave_draft(
    connection: &Connection,
    request: DraftAutosaveRequest,
) -> Result<ComposerDraftRecord, DraftPersistenceError> {
    validate_autosave_request(&request)?;
    ensure_account_exists(connection, &request.account_id)?;

    let body_html = request.body_html.as_ref().map(DraftHtml::as_str);
    connection.execute(
        "INSERT INTO composer_drafts (
           id,
           account_id,
           draft_state,
           subject,
           to_addresses_json,
           cc_addresses_json,
           bcc_addresses_json,
           reply_to_addresses_json,
           body_text,
           body_html,
           attachment_ids_json,
           in_reply_to,
           references_json,
           autosave_revision,
           last_autosaved_at
         )
         VALUES (?1, ?2, 'active', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           draft_state = CASE
             WHEN composer_drafts.draft_state = 'active' THEN 'active'
             ELSE composer_drafts.draft_state
           END,
           subject = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.subject
             ELSE composer_drafts.subject
           END,
           to_addresses_json = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.to_addresses_json
             ELSE composer_drafts.to_addresses_json
           END,
           cc_addresses_json = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.cc_addresses_json
             ELSE composer_drafts.cc_addresses_json
           END,
           bcc_addresses_json = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.bcc_addresses_json
             ELSE composer_drafts.bcc_addresses_json
           END,
           reply_to_addresses_json = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.reply_to_addresses_json
             ELSE composer_drafts.reply_to_addresses_json
           END,
           body_text = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.body_text
             ELSE composer_drafts.body_text
           END,
           body_html = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.body_html
             ELSE composer_drafts.body_html
           END,
           attachment_ids_json = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.attachment_ids_json
             ELSE composer_drafts.attachment_ids_json
           END,
           in_reply_to = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.in_reply_to
             ELSE composer_drafts.in_reply_to
           END,
           references_json = CASE
             WHEN composer_drafts.draft_state = 'active' THEN excluded.references_json
             ELSE composer_drafts.references_json
           END,
           autosave_revision = CASE
             WHEN composer_drafts.draft_state = 'active' THEN composer_drafts.autosave_revision + 1
             ELSE composer_drafts.autosave_revision
           END,
           last_autosaved_at = CASE
             WHEN composer_drafts.draft_state = 'active' THEN CURRENT_TIMESTAMP
             ELSE composer_drafts.last_autosaved_at
           END,
           updated_at = CURRENT_TIMESTAMP",
        params![
            &request.draft_id,
            &request.account_id,
            &request.subject,
            &request.to_addresses_json,
            &request.cc_addresses_json,
            &request.bcc_addresses_json,
            &request.reply_to_addresses_json,
            &request.body_text,
            body_html,
            &request.attachment_ids_json,
            &request.in_reply_to,
            &request.references_json,
        ],
    )?;

    let record =
        load_draft(connection, &request.account_id, &request.draft_id)?.ok_or_else(|| {
            DraftPersistenceError::UnknownDraft {
                draft_id: request.draft_id.clone(),
            }
        })?;
    if record.draft_state != DraftState::Active {
        return Err(DraftPersistenceError::DraftNotActive {
            draft_id: request.draft_id,
        });
    }

    Ok(record)
}

pub fn load_draft(
    connection: &Connection,
    account_id: &str,
    draft_id: &str,
) -> Result<Option<ComposerDraftRecord>, DraftPersistenceError> {
    validate_entity_id("account_id", account_id)?;
    validate_entity_id("draft_id", draft_id)?;

    connection
        .query_row(
            "SELECT
               id,
               account_id,
               draft_state,
               subject,
               to_addresses_json,
               cc_addresses_json,
               bcc_addresses_json,
               reply_to_addresses_json,
               body_text,
               body_html,
               attachment_ids_json,
               in_reply_to,
               references_json,
               autosave_revision,
               sent_receipt_id
             FROM composer_drafts
             WHERE account_id = ?1 AND id = ?2",
            params![account_id, draft_id],
            draft_from_row,
        )
        .optional()
        .map_err(DraftPersistenceError::from)
}

pub fn list_active_drafts(
    connection: &Connection,
    account_id: &str,
) -> Result<Vec<ComposerDraftRecord>, DraftPersistenceError> {
    validate_entity_id("account_id", account_id)?;
    ensure_account_exists(connection, account_id)?;

    let mut statement = connection.prepare(
        "SELECT
           id,
           account_id,
           draft_state,
           subject,
           to_addresses_json,
           cc_addresses_json,
           bcc_addresses_json,
           reply_to_addresses_json,
           body_text,
           body_html,
           attachment_ids_json,
           in_reply_to,
           references_json,
           autosave_revision,
           sent_receipt_id
         FROM composer_drafts
         WHERE account_id = ?1 AND draft_state = 'active'
         ORDER BY updated_at DESC, id",
    )?;
    let rows = statement.query_map([account_id], draft_from_row)?;

    let mut drafts = Vec::new();
    for row in rows {
        drafts.push(row?);
    }
    Ok(drafts)
}

pub fn ensure_active_draft(
    connection: &Connection,
    account_id: &str,
    draft_id: &str,
) -> Result<(), DraftPersistenceError> {
    let Some(draft) = load_draft(connection, account_id, draft_id)? else {
        return Err(DraftPersistenceError::UnknownDraft {
            draft_id: draft_id.to_string(),
        });
    };

    if draft.draft_state != DraftState::Active {
        return Err(DraftPersistenceError::DraftNotActive {
            draft_id: draft_id.to_string(),
        });
    }

    Ok(())
}

pub fn discard_draft(
    connection: &Connection,
    request: DiscardDraftRequest,
) -> Result<ComposerDraftRecord, DraftPersistenceError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("draft_id", &request.draft_id)?;
    ensure_active_draft(connection, &request.account_id, &request.draft_id)?;

    connection.execute(
        "UPDATE composer_drafts
         SET
           draft_state = 'discarded',
           subject = '',
           to_addresses_json = '[]',
           cc_addresses_json = '[]',
           bcc_addresses_json = '[]',
           reply_to_addresses_json = '[]',
           body_text = NULL,
           body_html = NULL,
           attachment_ids_json = '[]',
           discarded_at = CURRENT_TIMESTAMP,
           updated_at = CURRENT_TIMESTAMP
         WHERE account_id = ?1 AND id = ?2 AND draft_state = 'active'",
        params![&request.account_id, &request.draft_id],
    )?;

    load_draft(connection, &request.account_id, &request.draft_id)?.ok_or(
        DraftPersistenceError::UnknownDraft {
            draft_id: request.draft_id,
        },
    )
}

pub fn mark_draft_sent_after_confirmed_send(
    transaction: &Transaction<'_>,
    confirmed: ConfirmedSentDraft,
) -> Result<(), DraftPersistenceError> {
    validate_entity_id("account_id", &confirmed.account_id)?;
    validate_entity_id("draft_id", &confirmed.draft_id)?;
    validate_entity_id("send_receipt_id", &confirmed.send_receipt_id)?;

    let changed = transaction.execute(
        "UPDATE composer_drafts
         SET
           draft_state = 'sent',
           subject = '',
           to_addresses_json = '[]',
           cc_addresses_json = '[]',
           bcc_addresses_json = '[]',
           reply_to_addresses_json = '[]',
           body_text = NULL,
           body_html = NULL,
           attachment_ids_json = '[]',
           sent_receipt_id = ?3,
           sent_at = CURRENT_TIMESTAMP,
           updated_at = CURRENT_TIMESTAMP
         WHERE account_id = ?1 AND id = ?2 AND draft_state = 'active'",
        params![
            &confirmed.account_id,
            &confirmed.draft_id,
            &confirmed.send_receipt_id
        ],
    )?;

    if changed != 1 {
        return Err(DraftPersistenceError::DraftNotActive {
            draft_id: confirmed.draft_id,
        });
    }

    Ok(())
}

pub fn sent_mail_copy_for_confirmed_send(
    connection: &Connection,
    account_id: &str,
    provider_kind: ProviderKind,
    send_receipt_id: &str,
    rfc_message_id: &str,
) -> Result<SentMailCopyRecord, DraftPersistenceError> {
    validate_entity_id("account_id", account_id)?;
    validate_entity_id("send_receipt_id", send_receipt_id)?;
    validate_bounded_text("rfc_message_id", rfc_message_id, MAX_TEXT_FIELD_BYTES)?;

    let sent_folder = find_sent_folder(connection, account_id)?;
    let (sent_folder_policy, sync_state, last_error) =
        resolve_sent_folder_policy(provider_kind, sent_folder.as_ref());

    Ok(SentMailCopyRecord {
        id: stable_sent_copy_id(send_receipt_id),
        send_receipt_id: send_receipt_id.to_string(),
        account_id: account_id.to_string(),
        rfc_message_id: rfc_message_id.to_string(),
        provider_kind,
        sent_folder_policy,
        sync_state,
        sent_folder_id: sent_folder
            .as_ref()
            .map(|folder| folder.sent_folder_id.clone()),
        sent_folder_remote_id: sent_folder.map(|folder| folder.sent_folder_remote_id),
        last_error: last_error.map(str::to_string),
    })
}

pub fn insert_sent_mail_copy_record(
    transaction: &Transaction<'_>,
    record: &SentMailCopyRecord,
) -> Result<(), DraftPersistenceError> {
    transaction.execute(
        "INSERT INTO sent_mail_copies (
           id,
           send_receipt_id,
           account_id,
           rfc_message_id,
           provider_kind,
           sent_folder_policy,
           sync_state,
           sent_folder_id,
           sent_folder_remote_id,
           last_error
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            &record.id,
            &record.send_receipt_id,
            &record.account_id,
            &record.rfc_message_id,
            provider_schema_value(record.provider_kind),
            record.sent_folder_policy.schema_value(),
            record.sync_state.schema_value(),
            &record.sent_folder_id,
            &record.sent_folder_remote_id,
            &record.last_error,
        ],
    )?;

    Ok(())
}

fn validate_autosave_request(request: &DraftAutosaveRequest) -> Result<(), DraftPersistenceError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("draft_id", &request.draft_id)?;
    validate_payload_size("subject", request.subject.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    validate_json_array("to_addresses_json", &request.to_addresses_json)?;
    validate_json_array("cc_addresses_json", &request.cc_addresses_json)?;
    validate_json_array("bcc_addresses_json", &request.bcc_addresses_json)?;
    validate_json_array("reply_to_addresses_json", &request.reply_to_addresses_json)?;
    validate_json_array("attachment_ids_json", &request.attachment_ids_json)?;
    validate_json_array("references_json", &request.references_json)?;

    if let Some(body_text) = &request.body_text {
        validate_payload_size("body_text", body_text.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    }
    if let Some(in_reply_to) = &request.in_reply_to {
        validate_bounded_text("in_reply_to", in_reply_to, MAX_TEXT_FIELD_BYTES)?;
    }

    Ok(())
}

fn validate_json_array(field: &'static str, value: &str) -> Result<(), DraftPersistenceError> {
    validate_payload_size(field, value.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    let parsed: Value =
        serde_json::from_str(value).map_err(|_| DraftPersistenceError::InvalidDraft {
            reason: "JSON field is invalid",
        })?;
    if !parsed.is_array() {
        return Err(DraftPersistenceError::InvalidDraft {
            reason: "JSON field must be an array",
        });
    }

    Ok(())
}

fn ensure_account_exists(
    connection: &Connection,
    account_id: &str,
) -> Result<(), DraftPersistenceError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1 AND removed_at IS NULL)",
        [account_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(DraftPersistenceError::UnknownAccount);
    }

    Ok(())
}

fn draft_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ComposerDraftRecord> {
    let draft_state = row.get::<_, String>(2)?;
    let autosave_revision = row.get::<_, i64>(13)?;

    Ok(ComposerDraftRecord {
        id: row.get(0)?,
        account_id: row.get(1)?,
        draft_state: DraftState::from_schema(&draft_state).unwrap_or(DraftState::Discarded),
        subject: row.get(3)?,
        to_addresses_json: row.get(4)?,
        cc_addresses_json: row.get(5)?,
        bcc_addresses_json: row.get(6)?,
        reply_to_addresses_json: row.get(7)?,
        body_text: row.get(8)?,
        body_html: row.get(9)?,
        attachment_ids_json: row.get(10)?,
        in_reply_to: row.get(11)?,
        references_json: row.get(12)?,
        autosave_revision: autosave_revision.max(0) as u64,
        sent_receipt_id: row.get(14)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SentFolderRef {
    sent_folder_id: String,
    sent_folder_remote_id: String,
}

fn find_sent_folder(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<SentFolderRef>, DraftPersistenceError> {
    connection
        .query_row(
            "SELECT id, remote_id
             FROM folders
             WHERE account_id = ?1 AND (kind = 'sent' OR special_use = 'sent')
             ORDER BY
               CASE WHEN special_use = 'sent' THEN 0 ELSE 1 END,
               CASE WHEN kind = 'sent' THEN 0 ELSE 1 END,
               id
             LIMIT 1",
            [account_id],
            |row| {
                Ok(SentFolderRef {
                    sent_folder_id: row.get(0)?,
                    sent_folder_remote_id: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(DraftPersistenceError::from)
}

fn resolve_sent_folder_policy(
    provider_kind: ProviderKind,
    sent_folder: Option<&SentFolderRef>,
) -> (SentFolderPolicy, SentFolderSyncState, Option<&'static str>) {
    match provider_kind {
        ProviderKind::Gmail => (
            SentFolderPolicy::ProviderAutoCopies,
            SentFolderSyncState::NotRequired,
            None,
        ),
        ProviderKind::Microsoft | ProviderKind::GenericImapSmtp => {
            if sent_folder.is_some() {
                (
                    SentFolderPolicy::ImapAppend,
                    SentFolderSyncState::PendingUpload,
                    Some("IMAP APPEND worker is not implemented yet"),
                )
            } else {
                (
                    SentFolderPolicy::LocalReceiptOnly,
                    SentFolderSyncState::Unsupported,
                    Some("no Sent folder has been discovered for this account"),
                )
            }
        }
    }
}

fn stable_sent_copy_id(send_receipt_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"sent-copy");
    digest.update([0]);
    digest.update(send_receipt_id.as_bytes());
    let hash = digest.finalize();
    let mut output = String::from("sentcopy_");
    for byte in hash.iter().take(16) {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn provider_schema_value(provider_kind: ProviderKind) -> &'static str {
    match provider_kind {
        ProviderKind::Gmail => "gmail",
        ProviderKind::Microsoft => "microsoft",
        ProviderKind::GenericImapSmtp => "generic_imap_smtp",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
    }

    fn seed_account(connection: &Connection, account_id: &str, provider_kind: &str) {
        if provider_kind == "generic_imap_smtp" {
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
                       ?1,
                       ?2,
                       'sender@example.com',
                       'sender@example.com',
                       'app_password',
                       ?3,
                       'imap.example.com',
                       993,
                       'tls',
                       'smtp.example.com',
                       587,
                       'starttls'
                     )",
                    params![account_id, provider_kind, format!("cred_{account_id}")],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "INSERT INTO accounts (
                       id,
                       provider_kind,
                       display_email,
                       username,
                       auth_kind,
                       credential_ref
                     )
                     VALUES (?1, ?2, 'sender@example.com', 'sender@example.com', 'oauth2', ?3)",
                    params![account_id, provider_kind, format!("cred_{account_id}")],
                )
                .unwrap();
        }
    }

    fn seed_sent_folder(connection: &Connection, account_id: &str) {
        connection
            .execute(
                "INSERT INTO folders (
                   id,
                   account_id,
                   remote_id,
                   name,
                   display_name,
                   kind,
                   special_use
                 )
                 VALUES ('folder_sent', ?1, 'Sent Items', 'Sent Items', 'Sent Items', 'sent', 'sent')",
                [account_id],
            )
            .unwrap();
    }

    fn autosave_request() -> DraftAutosaveRequest {
        DraftAutosaveRequest {
            account_id: "acct_1".into(),
            draft_id: "draft_1".into(),
            subject: "Draft subject".into(),
            to_addresses_json: r#"[{"address":"recipient@example.com"}]"#.into(),
            cc_addresses_json: "[]".into(),
            bcc_addresses_json: "[]".into(),
            reply_to_addresses_json: "[]".into(),
            body_text: Some("body".into()),
            body_html: Some(DraftHtml::new_sanitized("<p>body</p>").unwrap()),
            attachment_ids_json: r#"["att_1"]"#.into(),
            in_reply_to: Some("<parent@example.com>".into()),
            references_json: r#"["<parent@example.com>"]"#.into(),
        }
    }

    #[test]
    fn autosaves_active_draft_and_survives_reload() {
        let connection = migrated_connection();
        seed_account(&connection, "acct_1", "gmail");

        let first = autosave_draft(&connection, autosave_request()).unwrap();
        let mut request = autosave_request();
        request.subject = "Updated draft".into();
        let second = autosave_draft(&connection, request).unwrap();
        let reloaded = load_draft(&connection, "acct_1", "draft_1")
            .unwrap()
            .unwrap();

        assert_eq!(first.autosave_revision, 1);
        assert_eq!(second.autosave_revision, 2);
        assert_eq!(reloaded.subject, "Updated draft");
        assert_eq!(reloaded.draft_state, DraftState::Active);
        assert_eq!(list_active_drafts(&connection, "acct_1").unwrap().len(), 1);
    }

    #[test]
    fn discard_is_intentional_and_clears_private_content() {
        let connection = migrated_connection();
        seed_account(&connection, "acct_1", "gmail");
        autosave_draft(&connection, autosave_request()).unwrap();

        let discarded = discard_draft(
            &connection,
            DiscardDraftRequest {
                account_id: "acct_1".into(),
                draft_id: "draft_1".into(),
            },
        )
        .unwrap();

        assert_eq!(discarded.draft_state, DraftState::Discarded);
        assert_eq!(discarded.subject, "");
        assert_eq!(discarded.to_addresses_json, "[]");
        assert!(discarded.body_text.is_none());
        assert!(discarded.body_html.is_none());
        assert!(list_active_drafts(&connection, "acct_1")
            .unwrap()
            .is_empty());
        assert!(matches!(
            ensure_active_draft(&connection, "acct_1", "draft_1"),
            Err(DraftPersistenceError::DraftNotActive { .. })
        ));
    }

    #[test]
    fn sent_transition_clears_draft_content_after_confirmed_receipt() {
        let connection = migrated_connection();
        seed_account(&connection, "acct_1", "gmail");
        autosave_draft(&connection, autosave_request()).unwrap();
        connection
            .execute(
                "INSERT INTO smtp_send_receipts (
                   id,
                   account_id,
                   rfc_message_id,
                   subject,
                   from_address,
                   to_addresses_json,
                   smtp_host,
                   smtp_port,
                   smtp_security,
                   auth_mechanism,
                   message_size_bytes
                 )
                 VALUES (
                   'send_1',
                   'acct_1',
                   '<sent@example.com>',
                   'sent',
                   'sender@example.com',
                   '[]',
                   'smtp.example.com',
                   587,
                   'starttls',
                   'xoauth2',
                   256
                 )",
                [],
            )
            .unwrap();

        let transaction = connection.unchecked_transaction().unwrap();
        mark_draft_sent_after_confirmed_send(
            &transaction,
            ConfirmedSentDraft {
                account_id: "acct_1".into(),
                draft_id: "draft_1".into(),
                send_receipt_id: "send_1".into(),
            },
        )
        .unwrap();
        transaction.commit().unwrap();

        let sent = load_draft(&connection, "acct_1", "draft_1")
            .unwrap()
            .unwrap();
        assert_eq!(sent.draft_state, DraftState::Sent);
        assert_eq!(sent.sent_receipt_id, Some("send_1".into()));
        assert_eq!(sent.subject, "");
        assert!(sent.body_text.is_none());
        assert!(sent.body_html.is_none());
    }

    #[test]
    fn sent_folder_policy_is_provider_aware() {
        let connection = migrated_connection();
        seed_account(&connection, "acct_gmail", "gmail");
        seed_account(&connection, "acct_ms", "microsoft");
        seed_sent_folder(&connection, "acct_ms");
        seed_account(&connection, "acct_generic", "generic_imap_smtp");

        let gmail = sent_mail_copy_for_confirmed_send(
            &connection,
            "acct_gmail",
            ProviderKind::Gmail,
            "send_gmail",
            "<gmail@example.com>",
        )
        .unwrap();
        let microsoft = sent_mail_copy_for_confirmed_send(
            &connection,
            "acct_ms",
            ProviderKind::Microsoft,
            "send_ms",
            "<ms@example.com>",
        )
        .unwrap();
        let generic = sent_mail_copy_for_confirmed_send(
            &connection,
            "acct_generic",
            ProviderKind::GenericImapSmtp,
            "send_generic",
            "<generic@example.com>",
        )
        .unwrap();

        assert_eq!(
            gmail.sent_folder_policy,
            SentFolderPolicy::ProviderAutoCopies
        );
        assert_eq!(gmail.sync_state, SentFolderSyncState::NotRequired);
        assert_eq!(microsoft.sent_folder_policy, SentFolderPolicy::ImapAppend);
        assert_eq!(microsoft.sync_state, SentFolderSyncState::PendingUpload);
        assert_eq!(microsoft.sent_folder_id, Some("folder_sent".into()));
        assert_eq!(
            generic.sent_folder_policy,
            SentFolderPolicy::LocalReceiptOnly
        );
        assert_eq!(generic.sync_state, SentFolderSyncState::Unsupported);
    }

    #[test]
    fn rejects_unsanitized_html_and_invalid_json_before_persistence() {
        assert!(matches!(
            DraftHtml::new_sanitized("<img src=x onerror=alert(1)>"),
            Err(DraftPersistenceError::InvalidDraft { .. })
        ));

        let mut request = autosave_request();
        request.to_addresses_json = "{}".into();
        assert!(matches!(
            validate_autosave_request(&request),
            Err(DraftPersistenceError::InvalidDraft { .. })
        ));
    }
}
