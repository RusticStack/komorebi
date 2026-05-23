use std::{
    error::Error,
    fmt,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use lettre::{
    message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart},
    transport::smtp::{
        authentication::{Credentials, Mechanism},
        response::Response,
    },
    Address, Message, SmtpTransport, Transport,
};
use rand::RngExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    account_signatures::{
        insert_signature_into_body_html, insert_signature_into_body_text, AccountSignatureError,
    },
    auth::{evaluate_account_setup_auth_policy, AuthKind, AuthPolicy, ProviderKind, SaslMechanism},
    credentials::{CredentialError, CredentialSlot, WindowsCredentialManager},
    draft_persistence::{
        ensure_active_draft, insert_sent_mail_copy_record, mark_draft_sent_after_confirmed_send,
        sent_mail_copy_for_confirmed_send, ConfirmedSentDraft, DraftPersistenceError,
        SentFolderPolicy, SentFolderSyncState,
    },
    ipc_validation::{
        validate_bounded_text, validate_entity_id, validate_payload_size, IpcValidationError,
        MAX_TEXT_FIELD_BYTES,
    },
    token_lifecycle::{
        AccessTokenCache, AccountDataStore, CredentialStore, OAuthTokenClient, ReconnectReason,
        SecretStoreError, SecretString, TokenLifecycle, TokenRefreshResult,
    },
};

pub const MAX_RECIPIENTS: usize = 100;
pub const MAX_OUTBOUND_ATTACHMENTS: usize = 32;
pub const MAX_OUTBOUND_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_OUTBOUND_MESSAGE_BYTES: u64 = 50 * 1024 * 1024;
pub const ATTACHMENT_STREAM_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurity {
    Tls,
    StartTls,
    None,
}

impl SmtpSecurity {
    fn schema_value(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
            Self::None => "none",
        }
    }

    fn from_schema(value: &str) -> Option<Self> {
        match value {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::StartTls),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpAccountConfig {
    pub account_id: String,
    pub provider_kind: ProviderKind,
    pub auth_kind: AuthKind,
    pub display_email: String,
    pub display_name: Option<String>,
    pub username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    pub signature_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutboundEmailAddress {
    pub name: Option<String>,
    pub address: String,
}

impl OutboundEmailAddress {
    pub fn new(
        name: Option<impl Into<String>>,
        address: impl Into<String>,
    ) -> Result<Self, SendEmailError> {
        let name = name
            .map(Into::into)
            .filter(|value| !value.trim().is_empty());
        let address = address.into();
        validate_email_address("email_address", name.as_deref(), &address)?;

        Ok(Self { name, address })
    }

    fn mailbox(&self) -> Result<Mailbox, SendEmailError> {
        Ok(Mailbox::new(
            self.name.clone(),
            self.address
                .parse::<Address>()
                .map_err(|_| SendEmailError::InvalidAddress {
                    field: "email_address",
                })?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedHtml(String);

impl SanitizedHtml {
    pub fn new_sanitized(value: impl Into<String>) -> Result<Self, SendEmailError> {
        let value = value.into();
        validate_payload_size("body_html", value.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
        if value.trim().is_empty() {
            return Err(SendEmailError::InvalidRequest {
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
            return Err(SendEmailError::InvalidRequest {
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
pub struct OutboundAttachmentRef {
    pub attachment_id: String,
}

impl OutboundAttachmentRef {
    pub fn new(attachment_id: impl Into<String>) -> Result<Self, SendEmailError> {
        let attachment_id = attachment_id.into();
        validate_entity_id("attachment_id", &attachment_id)?;
        Ok(Self { attachment_id })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundAttachmentDisposition {
    Attachment,
    Inline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundAttachmentMetadata {
    pub attachment_id: String,
    pub display_filename: String,
    pub content_type: String,
    pub disposition: OutboundAttachmentDisposition,
    pub content_id: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendEmailRequest {
    pub account_id: String,
    pub draft_id: Option<String>,
    pub to: Vec<OutboundEmailAddress>,
    pub cc: Vec<OutboundEmailAddress>,
    pub bcc: Vec<OutboundEmailAddress>,
    pub reply_to: Vec<OutboundEmailAddress>,
    pub subject: String,
    pub body_text: Option<String>,
    pub body_html: Option<SanitizedHtml>,
    pub attachments: Vec<OutboundAttachmentRef>,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtpAuth {
    Plain {
        username: String,
        password: SecretString,
    },
    XOAuth2 {
        username: String,
        access_token: SecretString,
    },
}

impl SmtpAuth {
    fn mechanism(&self) -> SaslMechanism {
        match self {
            Self::Plain { .. } => SaslMechanism::Plain,
            Self::XOAuth2 { .. } => SaslMechanism::XOAuth2,
        }
    }

    fn mechanism_schema_value(&self) -> &'static str {
        match self {
            Self::Plain { .. } => "plain",
            Self::XOAuth2 { .. } => "xoauth2",
        }
    }

    fn username(&self) -> &str {
        match self {
            Self::Plain { username, .. } | Self::XOAuth2 { username, .. } => username,
        }
    }

    fn secret(&self) -> &SecretString {
        match self {
            Self::Plain { password, .. } => password,
            Self::XOAuth2 { access_token, .. } => access_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendProgressEvent {
    AccountResolved,
    DraftPrepared { draft_id: String },
    AuthResolved { mechanism: SaslMechanism },
    AttachmentStreamed { attachment_id: String, bytes: u64 },
    MimeComposed { message_size_bytes: u64 },
    SmtpAccepted,
    SentStateRecorded,
    DraftFinalized { draft_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpDelivery {
    pub provider_response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendEmailReport {
    pub account_id: String,
    pub source_draft_id: Option<String>,
    pub rfc_message_id: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    pub sent_folder_policy: SentFolderPolicy,
    pub sent_folder_sync_state: SentFolderSyncState,
    pub provider_response: Option<String>,
    pub message_size_bytes: u64,
    pub attachment_bytes_streamed: u64,
    pub progress: Vec<SendProgressEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedSendRecord {
    pub id: String,
    pub account_id: String,
    pub provider_kind: ProviderKind,
    pub source_draft_id: Option<String>,
    pub rfc_message_id: String,
    pub subject: String,
    pub from_address: String,
    pub to_addresses_json: String,
    pub cc_addresses_json: String,
    pub bcc_addresses_json: String,
    pub attachment_ids_json: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    pub auth_mechanism: &'static str,
    pub message_size_bytes: u64,
    pub provider_response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendEmailError {
    IpcValidation(IpcValidationError),
    UnknownAccount,
    UnknownAttachment,
    InvalidAddress {
        field: &'static str,
    },
    InvalidRequest {
        reason: &'static str,
    },
    DraftPersistence(String),
    AccountSignature(String),
    UnsupportedAuth {
        reason: &'static str,
    },
    ReconnectNeeded {
        reason: ReconnectReason,
    },
    CredentialUnavailable,
    AttachmentUnavailable {
        attachment_id: String,
    },
    AttachmentPathRejected {
        attachment_id: String,
    },
    AttachmentTooLarge {
        attachment_id: String,
        max_bytes: u64,
        actual_bytes: u64,
    },
    AttachmentSizeMismatch {
        attachment_id: String,
        expected_bytes: u64,
        actual_bytes: u64,
    },
    InvalidContentType {
        attachment_id: String,
    },
    MimeBuild(String),
    SmtpTransport(String),
    Database(String),
}

impl fmt::Display for SendEmailError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::UnknownAccount => write!(formatter, "sender account was not found"),
            Self::UnknownAttachment => write!(formatter, "attachment was not found"),
            Self::InvalidAddress { field } => write!(formatter, "{field} is not a valid address"),
            Self::InvalidRequest { reason } => {
                write!(formatter, "send request is invalid: {reason}")
            }
            Self::DraftPersistence(message) => {
                write!(formatter, "draft persistence failed: {message}")
            }
            Self::AccountSignature(message) => {
                write!(formatter, "account signature failed: {message}")
            }
            Self::UnsupportedAuth { reason } => {
                write!(formatter, "SMTP auth is unsupported: {reason}")
            }
            Self::ReconnectNeeded { .. } => write!(formatter, "sender account needs reconnect"),
            Self::CredentialUnavailable => write!(formatter, "SMTP credential is unavailable"),
            Self::AttachmentUnavailable { .. } => {
                write!(formatter, "attachment content is unavailable")
            }
            Self::AttachmentPathRejected { .. } => {
                write!(formatter, "attachment path is outside app storage")
            }
            Self::AttachmentTooLarge {
                max_bytes,
                actual_bytes,
                ..
            } => {
                write!(
                    formatter,
                    "attachment exceeds {max_bytes} bytes ({actual_bytes} bytes provided)"
                )
            }
            Self::AttachmentSizeMismatch {
                expected_bytes,
                actual_bytes,
                ..
            } => {
                write!(
                    formatter,
                    "attachment size mismatch: expected {expected_bytes}, read {actual_bytes}"
                )
            }
            Self::InvalidContentType { .. } => {
                write!(formatter, "attachment content type is invalid")
            }
            Self::MimeBuild(message) => write!(formatter, "MIME composition failed: {message}"),
            Self::SmtpTransport(message) => write!(formatter, "SMTP send failed: {message}"),
            Self::Database(message) => {
                write!(formatter, "send state persistence failed: {message}")
            }
        }
    }
}

impl Error for SendEmailError {}

impl From<IpcValidationError> for SendEmailError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

impl From<rusqlite::Error> for SendEmailError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value.to_string())
    }
}

impl From<DraftPersistenceError> for SendEmailError {
    fn from(value: DraftPersistenceError) -> Self {
        Self::DraftPersistence(value.to_string())
    }
}

impl From<AccountSignatureError> for SendEmailError {
    fn from(value: AccountSignatureError) -> Self {
        Self::AccountSignature(value.to_string())
    }
}

pub trait SmtpAccountStore {
    fn load_smtp_account(&mut self, account_id: &str) -> Result<SmtpAccountConfig, SendEmailError>;
    fn ensure_active_draft_for_send(
        &mut self,
        _account_id: &str,
        _draft_id: &str,
    ) -> Result<(), SendEmailError> {
        Ok(())
    }
    fn record_confirmed_send(
        &mut self,
        record: &ConfirmedSendRecord,
    ) -> Result<ConfirmedSendPersistence, SendEmailError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedSendPersistence {
    pub sent_folder_policy: SentFolderPolicy,
    pub sent_folder_sync_state: SentFolderSyncState,
}

pub trait SmtpAuthResolver {
    fn resolve_smtp_auth(
        &mut self,
        account: &SmtpAccountConfig,
    ) -> Result<SmtpAuth, SendEmailError>;
}

pub trait AttachmentContentStore {
    fn stream_attachment(
        &mut self,
        account_id: &str,
        attachment_id: &str,
        max_bytes: u64,
        chunk_size: usize,
        on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), SendEmailError>,
    ) -> Result<OutboundAttachmentMetadata, SendEmailError>;
}

pub trait SmtpSender {
    fn send(
        &mut self,
        account: &SmtpAccountConfig,
        auth: &SmtpAuth,
        message: &Message,
    ) -> Result<SmtpDelivery, SendEmailError>;
}

pub struct SqliteSmtpAccountStore<'connection> {
    connection: &'connection Connection,
}

impl<'connection> SqliteSmtpAccountStore<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }
}

impl SmtpAccountStore for SqliteSmtpAccountStore<'_> {
    fn load_smtp_account(&mut self, account_id: &str) -> Result<SmtpAccountConfig, SendEmailError> {
        validate_entity_id("account_id", account_id)?;

        let row = self
            .connection
            .query_row(
                "SELECT
                   provider_kind,
                   display_email,
                   display_name,
                   username,
                   auth_kind,
                   smtp_host,
                   smtp_port,
                   smtp_security,
                   signature_html
                 FROM accounts
                 WHERE id = ?1 AND removed_at IS NULL",
                [account_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            provider_kind,
            display_email,
            display_name,
            username,
            auth_kind,
            smtp_host,
            smtp_port,
            smtp_security,
            signature_html,
        )) = row
        else {
            return Err(SendEmailError::UnknownAccount);
        };

        let provider_kind = provider_from_schema(&provider_kind)?;
        let auth_kind = auth_from_schema(&auth_kind)?;
        let (smtp_host, smtp_port, smtp_security) =
            resolve_smtp_endpoint(provider_kind, smtp_host, smtp_port, smtp_security)?;

        validate_email_address("display_email", display_name.as_deref(), &display_email)?;
        validate_smtp_host(&smtp_host)?;
        validate_bounded_text("smtp_username", &username, MAX_TEXT_FIELD_BYTES)?;

        Ok(SmtpAccountConfig {
            account_id: account_id.to_string(),
            provider_kind,
            auth_kind,
            display_email,
            display_name,
            username,
            smtp_host,
            smtp_port,
            smtp_security,
            signature_html,
        })
    }

    fn ensure_active_draft_for_send(
        &mut self,
        account_id: &str,
        draft_id: &str,
    ) -> Result<(), SendEmailError> {
        ensure_active_draft(self.connection, account_id, draft_id)?;
        Ok(())
    }

    fn record_confirmed_send(
        &mut self,
        record: &ConfirmedSendRecord,
    ) -> Result<ConfirmedSendPersistence, SendEmailError> {
        let sent_copy = sent_mail_copy_for_confirmed_send(
            self.connection,
            &record.account_id,
            record.provider_kind,
            &record.id,
            &record.rfc_message_id,
        )?;
        let transaction = self.connection.unchecked_transaction()?;

        transaction.execute(
            "INSERT INTO smtp_send_receipts (
               id,
               account_id,
               source_draft_id,
               rfc_message_id,
               subject,
               from_address,
               to_addresses_json,
               cc_addresses_json,
               bcc_addresses_json,
               attachment_ids_json,
               smtp_host,
               smtp_port,
               smtp_security,
               auth_mechanism,
               message_size_bytes,
               provider_response
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                &record.id,
                &record.account_id,
                &record.source_draft_id,
                &record.rfc_message_id,
                &record.subject,
                &record.from_address,
                &record.to_addresses_json,
                &record.cc_addresses_json,
                &record.bcc_addresses_json,
                &record.attachment_ids_json,
                &record.smtp_host,
                i64::from(record.smtp_port),
                record.smtp_security.schema_value(),
                record.auth_mechanism,
                u64_to_i64(record.message_size_bytes, "message_size_bytes")?,
                &record.provider_response
            ],
        )?;
        if let Some(draft_id) = &record.source_draft_id {
            mark_draft_sent_after_confirmed_send(
                &transaction,
                ConfirmedSentDraft {
                    account_id: record.account_id.clone(),
                    draft_id: draft_id.clone(),
                    send_receipt_id: record.id.clone(),
                },
            )?;
        }
        insert_sent_mail_copy_record(&transaction, &sent_copy)?;
        transaction.commit()?;

        Ok(ConfirmedSendPersistence {
            sent_folder_policy: sent_copy.sent_folder_policy,
            sent_folder_sync_state: sent_copy.sync_state,
        })
    }
}

pub trait PasswordCredentialStore {
    fn load_generic_password(
        &mut self,
        account_id: &str,
    ) -> Result<Option<SecretString>, SecretStoreError>;
}

impl PasswordCredentialStore for WindowsCredentialManager {
    fn load_generic_password(
        &mut self,
        account_id: &str,
    ) -> Result<Option<SecretString>, SecretStoreError> {
        match self.read_secret(account_id, CredentialSlot::GenericPassword) {
            Ok(secret) => Ok(Some(secret)),
            Err(CredentialError::NotFound) => Ok(None),
            Err(_) => Err(SecretStoreError::Unavailable),
        }
    }
}

pub struct BackendSmtpAuthResolver<P, C, A, O, D> {
    password_store: P,
    token_lifecycle: TokenLifecycle<C, A, O, D>,
    now: u64,
}

impl<P, C, A, O, D> BackendSmtpAuthResolver<P, C, A, O, D> {
    pub fn new(password_store: P, token_lifecycle: TokenLifecycle<C, A, O, D>, now: u64) -> Self {
        Self {
            password_store,
            token_lifecycle,
            now,
        }
    }
}

impl<P, C, A, O, D> SmtpAuthResolver for BackendSmtpAuthResolver<P, C, A, O, D>
where
    P: PasswordCredentialStore,
    C: CredentialStore,
    A: AccessTokenCache,
    O: OAuthTokenClient,
    D: AccountDataStore,
{
    fn resolve_smtp_auth(
        &mut self,
        account: &SmtpAccountConfig,
    ) -> Result<SmtpAuth, SendEmailError> {
        match evaluate_account_setup_auth_policy(account.provider_kind, account.auth_kind) {
            AuthPolicy::Allowed {
                mechanism: SaslMechanism::Plain,
                ..
            } => {
                let Some(password) = self
                    .password_store
                    .load_generic_password(&account.account_id)
                    .map_err(|_| SendEmailError::CredentialUnavailable)?
                else {
                    return Err(SendEmailError::CredentialUnavailable);
                };

                Ok(SmtpAuth::Plain {
                    username: account.username.clone(),
                    password,
                })
            }
            AuthPolicy::Allowed {
                mechanism: SaslMechanism::XOAuth2,
                ..
            } => match self.token_lifecycle.ensure_access_token(
                &account.account_id,
                account.provider_kind,
                self.now,
            ) {
                TokenRefreshResult::Ready { grant, .. } => Ok(SmtpAuth::XOAuth2 {
                    username: account.username.clone(),
                    access_token: grant.access_token().clone(),
                }),
                TokenRefreshResult::ReconnectNeeded { view } => match view {
                    crate::token_lifecycle::AccountAuthView::ReconnectNeeded { reason, .. } => {
                        Err(SendEmailError::ReconnectNeeded { reason })
                    }
                    _ => Err(SendEmailError::CredentialUnavailable),
                },
            },
            AuthPolicy::UnsupportedUntilImplemented { reason, .. }
            | AuthPolicy::Rejected { reason } => Err(SendEmailError::UnsupportedAuth { reason }),
        }
    }
}

pub struct FilesystemAttachmentStore<'connection> {
    connection: &'connection Connection,
    storage_root: PathBuf,
}

impl<'connection> FilesystemAttachmentStore<'connection> {
    pub fn new(connection: &'connection Connection, storage_root: impl Into<PathBuf>) -> Self {
        Self {
            connection,
            storage_root: storage_root.into(),
        }
    }
}

impl AttachmentContentStore for FilesystemAttachmentStore<'_> {
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
                   is_inline,
                   content_id
                 FROM attachments
                 WHERE account_id = ?1 AND id = ?2",
                params![account_id, attachment_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
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
            is_inline,
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
        let relative_path =
            storage_relative_path.ok_or_else(|| SendEmailError::AttachmentUnavailable {
                attachment_id: attachment_id.to_string(),
            })?;
        let full_path = safe_attachment_path(&self.storage_root, &relative_path, attachment_id)?;

        let mut file =
            File::open(&full_path).map_err(|_| SendEmailError::AttachmentUnavailable {
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
            disposition: if is_inline == 1 {
                OutboundAttachmentDisposition::Inline
            } else {
                OutboundAttachmentDisposition::Attachment
            },
            content_id,
            size_bytes: expected_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LettreSmtpSender {
    #[cfg(test)]
    allow_plaintext_loopback: bool,
}

impl LettreSmtpSender {
    #[cfg(test)]
    fn plaintext_loopback_for_tests() -> Self {
        Self {
            allow_plaintext_loopback: true,
        }
    }
}

impl SmtpSender for LettreSmtpSender {
    fn send(
        &mut self,
        account: &SmtpAccountConfig,
        auth: &SmtpAuth,
        message: &Message,
    ) -> Result<SmtpDelivery, SendEmailError> {
        if account.smtp_security == SmtpSecurity::None
            && !self.allows_plaintext_loopback_for_tests(&account.smtp_host)
        {
            return Err(SendEmailError::UnsupportedAuth {
                reason: "authenticated SMTP requires TLS or STARTTLS",
            });
        }

        let credentials = Credentials::new(
            auth.username().to_string(),
            auth.secret().expose_secret().to_string(),
        );
        let mechanism = match auth {
            SmtpAuth::Plain { .. } => Mechanism::Plain,
            SmtpAuth::XOAuth2 { .. } => Mechanism::Xoauth2,
        };
        let builder = match account.smtp_security {
            SmtpSecurity::Tls => SmtpTransport::relay(&account.smtp_host),
            SmtpSecurity::StartTls => SmtpTransport::starttls_relay(&account.smtp_host),
            SmtpSecurity::None => Ok(SmtpTransport::builder_dangerous(&account.smtp_host)),
        }
        .map_err(|_| SendEmailError::SmtpTransport("SMTP transport setup failed".to_string()))?
        .port(account.smtp_port)
        .credentials(credentials)
        .authentication(vec![mechanism])
        .timeout(Some(Duration::from_secs(60)));

        let response = builder
            .build()
            .send(message)
            .map_err(redacted_smtp_transport_error)?;

        Ok(SmtpDelivery {
            provider_response: Some(response_summary(&response)),
        })
    }
}

impl LettreSmtpSender {
    fn allows_plaintext_loopback_for_tests(&self, host: &str) -> bool {
        #[cfg(test)]
        {
            self.allow_plaintext_loopback
                && (host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1")
        }

        #[cfg(not(test))]
        {
            let _ = host;
            false
        }
    }
}

pub fn send_email<A, R, C, T>(
    request: SendEmailRequest,
    account_store: &mut A,
    auth_resolver: &mut R,
    attachment_store: &mut C,
    smtp_sender: &mut T,
) -> Result<SendEmailReport, SendEmailError>
where
    A: SmtpAccountStore,
    R: SmtpAuthResolver,
    C: AttachmentContentStore,
    T: SmtpSender,
{
    validate_send_request(&request)?;
    let mut progress = Vec::new();

    let account = account_store.load_smtp_account(&request.account_id)?;
    progress.push(SendProgressEvent::AccountResolved);

    if let Some(draft_id) = &request.draft_id {
        account_store.ensure_active_draft_for_send(&account.account_id, draft_id)?;
        progress.push(SendProgressEvent::DraftPrepared {
            draft_id: draft_id.clone(),
        });
    }

    let auth = auth_resolver.resolve_smtp_auth(&account)?;
    progress.push(SendProgressEvent::AuthResolved {
        mechanism: auth.mechanism(),
    });

    let rfc_message_id = request
        .message_id
        .clone()
        .map(validate_message_id)
        .transpose()?
        .unwrap_or_else(|| generate_message_id(&account.display_email));
    let composed = compose_message(
        &account,
        &request,
        &rfc_message_id,
        attachment_store,
        &mut progress,
    )?;

    if composed.message_size_bytes > MAX_OUTBOUND_MESSAGE_BYTES {
        return Err(SendEmailError::InvalidRequest {
            reason: "composed message exceeds outbound size limit",
        });
    }

    progress.push(SendProgressEvent::MimeComposed {
        message_size_bytes: composed.message_size_bytes,
    });

    let delivery = smtp_sender.send(&account, &auth, &composed.message)?;
    progress.push(SendProgressEvent::SmtpAccepted);

    let record = ConfirmedSendRecord {
        id: stable_send_receipt_id(&account.account_id, &rfc_message_id),
        account_id: account.account_id.clone(),
        provider_kind: account.provider_kind,
        source_draft_id: request.draft_id.clone(),
        rfc_message_id: rfc_message_id.clone(),
        subject: request.subject.clone(),
        from_address: account.display_email.clone(),
        to_addresses_json: serialize_addresses(&request.to)?,
        cc_addresses_json: serialize_addresses(&request.cc)?,
        bcc_addresses_json: serialize_addresses(&request.bcc)?,
        attachment_ids_json: serialize_attachment_ids(&request.attachments)?,
        smtp_host: account.smtp_host.clone(),
        smtp_port: account.smtp_port,
        smtp_security: account.smtp_security,
        auth_mechanism: auth.mechanism_schema_value(),
        message_size_bytes: composed.message_size_bytes,
        provider_response: delivery.provider_response.clone(),
    };
    let persistence = account_store.record_confirmed_send(&record)?;
    progress.push(SendProgressEvent::SentStateRecorded);
    if let Some(draft_id) = &request.draft_id {
        progress.push(SendProgressEvent::DraftFinalized {
            draft_id: draft_id.clone(),
        });
    }

    Ok(SendEmailReport {
        account_id: account.account_id,
        source_draft_id: request.draft_id,
        rfc_message_id,
        smtp_host: account.smtp_host,
        smtp_port: account.smtp_port,
        smtp_security: account.smtp_security,
        sent_folder_policy: persistence.sent_folder_policy,
        sent_folder_sync_state: persistence.sent_folder_sync_state,
        provider_response: delivery.provider_response,
        message_size_bytes: composed.message_size_bytes,
        attachment_bytes_streamed: composed.attachment_bytes_streamed,
        progress,
    })
}

struct ComposedMessage {
    message: Message,
    message_size_bytes: u64,
    attachment_bytes_streamed: u64,
}

enum BodyPart {
    Single(SinglePart),
    Multi(MultiPart),
}

fn compose_message(
    account: &SmtpAccountConfig,
    request: &SendEmailRequest,
    rfc_message_id: &str,
    attachment_store: &mut impl AttachmentContentStore,
    progress: &mut Vec<SendProgressEvent>,
) -> Result<ComposedMessage, SendEmailError> {
    let mut builder = Message::builder()
        .from(account_mailbox(account)?)
        .subject(request.subject.clone())
        .message_id(Some(rfc_message_id.to_string()))
        .user_agent("Komorebi/0.1".into());

    for recipient in &request.to {
        builder = builder.to(recipient.mailbox()?);
    }
    for recipient in &request.cc {
        builder = builder.cc(recipient.mailbox()?);
    }
    for recipient in &request.bcc {
        builder = builder.bcc(recipient.mailbox()?);
    }
    for recipient in &request.reply_to {
        builder = builder.reply_to(recipient.mailbox()?);
    }

    let body = compose_body_part(account, request)?;
    let mut attachment_bytes_streamed = 0_u64;
    let mut attachment_parts = Vec::new();
    for attachment in &request.attachments {
        let mut bytes = Vec::new();
        let starting_attachment_bytes = attachment_bytes_streamed;
        let metadata = attachment_store.stream_attachment(
            &account.account_id,
            &attachment.attachment_id,
            MAX_OUTBOUND_ATTACHMENT_BYTES,
            ATTACHMENT_STREAM_CHUNK_BYTES,
            &mut |chunk| {
                if starting_attachment_bytes
                    .saturating_add(bytes.len() as u64)
                    .saturating_add(chunk.len() as u64)
                    > MAX_OUTBOUND_MESSAGE_BYTES
                {
                    return Err(SendEmailError::InvalidRequest {
                        reason: "attachments exceed outbound size limit",
                    });
                }
                bytes.extend_from_slice(chunk);
                Ok(())
            },
        )?;
        attachment_bytes_streamed = attachment_bytes_streamed.saturating_add(bytes.len() as u64);
        progress.push(SendProgressEvent::AttachmentStreamed {
            attachment_id: attachment.attachment_id.clone(),
            bytes: bytes.len() as u64,
        });
        if bytes.len() as u64 != metadata.size_bytes {
            return Err(SendEmailError::AttachmentSizeMismatch {
                attachment_id: metadata.attachment_id,
                expected_bytes: metadata.size_bytes,
                actual_bytes: bytes.len() as u64,
            });
        }
        let content_type = ContentType::parse(&metadata.content_type).map_err(|_| {
            SendEmailError::InvalidContentType {
                attachment_id: metadata.attachment_id.clone(),
            }
        })?;
        let part = match metadata.disposition {
            OutboundAttachmentDisposition::Attachment => {
                Attachment::new(metadata.display_filename).body(bytes, content_type)
            }
            OutboundAttachmentDisposition::Inline => {
                let content_id = metadata
                    .content_id
                    .unwrap_or_else(|| metadata.attachment_id.clone());
                Attachment::new_inline_with_name(content_id, metadata.display_filename)
                    .body(bytes, content_type)
            }
        };
        attachment_parts.push(part);
    }

    let message = if attachment_parts.is_empty() {
        match body {
            BodyPart::Single(part) => builder.singlepart(part),
            BodyPart::Multi(part) => builder.multipart(part),
        }
    } else {
        let mut mixed = MultiPart::mixed().build();
        mixed = match body {
            BodyPart::Single(part) => mixed.singlepart(part),
            BodyPart::Multi(part) => mixed.multipart(part),
        };
        for attachment in attachment_parts {
            mixed = mixed.singlepart(attachment);
        }
        builder.multipart(mixed)
    }
    .map_err(|error| SendEmailError::MimeBuild(error.to_string()))?;

    let message_size_bytes =
        u64::try_from(message.formatted().len()).map_err(|_| SendEmailError::InvalidRequest {
            reason: "composed message size overflowed",
        })?;

    Ok(ComposedMessage {
        message,
        message_size_bytes,
        attachment_bytes_streamed,
    })
}

fn compose_body_part(
    account: &SmtpAccountConfig,
    request: &SendEmailRequest,
) -> Result<BodyPart, SendEmailError> {
    let body_text = if request.body_html.is_none() {
        request
            .body_text
            .as_deref()
            .map(|text| insert_signature_into_body_text(text, account.signature_html.as_deref()))
            .transpose()?
    } else {
        request.body_text.clone()
    };
    let body_html = request
        .body_html
        .as_ref()
        .map(|html| {
            insert_signature_into_body_html(html.as_str(), account.signature_html.as_deref())
        })
        .transpose()?;

    match (&body_text, &body_html) {
        (Some(text), Some(html)) => Ok(BodyPart::Multi(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(text.clone()))
                .singlepart(SinglePart::html(html.clone())),
        )),
        (Some(text), None) => Ok(BodyPart::Single(SinglePart::plain(text.clone()))),
        (None, Some(html)) => Ok(BodyPart::Single(SinglePart::html(html.clone()))),
        (None, None) => Err(SendEmailError::InvalidRequest {
            reason: "message body is required",
        }),
    }
}

fn validate_send_request(request: &SendEmailRequest) -> Result<(), SendEmailError> {
    validate_entity_id("account_id", &request.account_id)?;
    if let Some(draft_id) = &request.draft_id {
        validate_entity_id("draft_id", draft_id)?;
    }
    if request.to.len() + request.cc.len() + request.bcc.len() == 0 {
        return Err(SendEmailError::InvalidRequest {
            reason: "at least one recipient is required",
        });
    }
    if request.to.len() + request.cc.len() + request.bcc.len() + request.reply_to.len()
        > MAX_RECIPIENTS
    {
        return Err(SendEmailError::InvalidRequest {
            reason: "too many recipients",
        });
    }
    if request.attachments.len() > MAX_OUTBOUND_ATTACHMENTS {
        return Err(SendEmailError::InvalidRequest {
            reason: "too many attachments",
        });
    }
    validate_payload_size("subject", request.subject.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    if let Some(body_text) = &request.body_text {
        validate_payload_size("body_text", body_text.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
        if body_text.trim().is_empty() && request.body_html.is_none() {
            return Err(SendEmailError::InvalidRequest {
                reason: "message body is required",
            });
        }
    }
    if request.body_text.is_none() && request.body_html.is_none() {
        return Err(SendEmailError::InvalidRequest {
            reason: "message body is required",
        });
    }
    for attachment in &request.attachments {
        validate_entity_id("attachment_id", &attachment.attachment_id)?;
    }
    for address in request
        .to
        .iter()
        .chain(request.cc.iter())
        .chain(request.bcc.iter())
        .chain(request.reply_to.iter())
    {
        validate_email_address("email_address", address.name.as_deref(), &address.address)?;
    }
    if let Some(message_id) = &request.message_id {
        validate_message_id(message_id.clone())?;
    }

    Ok(())
}

fn validate_email_address(
    field: &'static str,
    name: Option<&str>,
    address: &str,
) -> Result<(), SendEmailError> {
    if let Some(name) = name {
        validate_payload_size(field, name.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    }
    validate_bounded_text(field, address, MAX_TEXT_FIELD_BYTES)?;
    address
        .parse::<Address>()
        .map_err(|_| SendEmailError::InvalidAddress { field })?;
    Ok(())
}

fn validate_smtp_host(host: &str) -> Result<(), SendEmailError> {
    validate_bounded_text("smtp_host", host, MAX_TEXT_FIELD_BYTES)?;
    if host != host.trim()
        || host
            .chars()
            .any(|character| character.is_ascii_whitespace())
        || host.contains(['/', '\\', '@', ':', '?', '#'])
    {
        return Err(SendEmailError::InvalidRequest {
            reason: "smtp_host must be a host name, not a URL or path",
        });
    }

    Ok(())
}

fn validate_message_id(value: String) -> Result<String, SendEmailError> {
    validate_bounded_text("message_id", &value, MAX_TEXT_FIELD_BYTES)?;
    let trimmed = value.trim();
    if !(trimmed.starts_with('<') && trimmed.ends_with('>') && trimmed.contains('@')) {
        return Err(SendEmailError::InvalidRequest {
            reason: "message_id must be an RFC Message-ID",
        });
    }

    Ok(trimmed.to_string())
}

fn account_mailbox(account: &SmtpAccountConfig) -> Result<Mailbox, SendEmailError> {
    Ok(Mailbox::new(
        account.display_name.clone(),
        account
            .display_email
            .parse::<Address>()
            .map_err(|_| SendEmailError::InvalidAddress {
                field: "display_email",
            })?,
    ))
}

fn provider_from_schema(value: &str) -> Result<ProviderKind, SendEmailError> {
    match value {
        "gmail" => Ok(ProviderKind::Gmail),
        "microsoft" => Ok(ProviderKind::Microsoft),
        "generic_imap_smtp" => Ok(ProviderKind::GenericImapSmtp),
        _ => Err(SendEmailError::InvalidRequest {
            reason: "unknown provider_kind",
        }),
    }
}

fn auth_from_schema(value: &str) -> Result<AuthKind, SendEmailError> {
    match value {
        "oauth2" => Ok(AuthKind::OAuth2),
        "app_password" => Ok(AuthKind::AppPassword),
        "manual_password" => Ok(AuthKind::ManualPassword),
        _ => Err(SendEmailError::InvalidRequest {
            reason: "unknown auth_kind",
        }),
    }
}

fn resolve_smtp_endpoint(
    provider: ProviderKind,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
    smtp_security: Option<String>,
) -> Result<(String, u16, SmtpSecurity), SendEmailError> {
    match provider {
        ProviderKind::Gmail => Ok((
            smtp_host.unwrap_or_else(|| "smtp.gmail.com".into()),
            optional_port(smtp_port).unwrap_or(587),
            smtp_security
                .as_deref()
                .and_then(SmtpSecurity::from_schema)
                .unwrap_or(SmtpSecurity::StartTls),
        )),
        ProviderKind::Microsoft => Ok((
            smtp_host.unwrap_or_else(|| "smtp.office365.com".into()),
            optional_port(smtp_port).unwrap_or(587),
            smtp_security
                .as_deref()
                .and_then(SmtpSecurity::from_schema)
                .unwrap_or(SmtpSecurity::StartTls),
        )),
        ProviderKind::GenericImapSmtp => {
            let Some(host) = smtp_host else {
                return Err(SendEmailError::InvalidRequest {
                    reason: "generic SMTP host is required",
                });
            };
            let Some(port) = optional_port(smtp_port) else {
                return Err(SendEmailError::InvalidRequest {
                    reason: "generic SMTP port is required",
                });
            };
            let Some(security) = smtp_security.as_deref().and_then(SmtpSecurity::from_schema)
            else {
                return Err(SendEmailError::InvalidRequest {
                    reason: "generic SMTP security is required",
                });
            };

            Ok((host, port, security))
        }
    }
}

fn optional_port(value: Option<i64>) -> Option<u16> {
    value
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn serialize_addresses(addresses: &[OutboundEmailAddress]) -> Result<String, SendEmailError> {
    serde_json::to_string(addresses).map_err(|error| SendEmailError::Database(error.to_string()))
}

fn serialize_attachment_ids(
    attachments: &[OutboundAttachmentRef],
) -> Result<String, SendEmailError> {
    let ids: Vec<&str> = attachments
        .iter()
        .map(|attachment| attachment.attachment_id.as_str())
        .collect();
    serde_json::to_string(&ids).map_err(|error| SendEmailError::Database(error.to_string()))
}

fn stable_send_receipt_id(account_id: &str, rfc_message_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(account_id.as_bytes());
    digest.update([0]);
    digest.update(rfc_message_id.as_bytes());
    let hash = digest.finalize();
    let mut output = String::from("send_");
    for byte in hash.iter().take(16) {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn generate_message_id(from_address: &str) -> String {
    let domain = from_address
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| {
            !domain.trim().is_empty()
                && domain
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        })
        .unwrap_or("komorebi.local");
    let mut bytes = [0_u8; 16];
    rand::rng().fill(&mut bytes);
    let mut local = String::from("komorebi-");
    for byte in bytes {
        local.push_str(&format!("{byte:02x}"));
    }
    format!("<{local}@{domain}>")
}

fn safe_attachment_path(
    storage_root: &Path,
    relative_path: &str,
    attachment_id: &str,
) -> Result<PathBuf, SendEmailError> {
    if relative_path.trim().is_empty() {
        return Err(SendEmailError::AttachmentPathRejected {
            attachment_id: attachment_id.to_string(),
        });
    }
    let relative = Path::new(relative_path);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SendEmailError::AttachmentPathRejected {
            attachment_id: attachment_id.to_string(),
        });
    }

    let root = storage_root
        .canonicalize()
        .map_err(|_| SendEmailError::AttachmentPathRejected {
            attachment_id: attachment_id.to_string(),
        })?;
    let full = root.join(relative);
    let canonical = full
        .canonicalize()
        .map_err(|_| SendEmailError::AttachmentUnavailable {
            attachment_id: attachment_id.to_string(),
        })?;
    if !canonical.starts_with(&root) {
        return Err(SendEmailError::AttachmentPathRejected {
            attachment_id: attachment_id.to_string(),
        });
    }

    Ok(canonical)
}

fn response_summary(response: &Response) -> String {
    let message = response.message().collect::<Vec<_>>().join(" | ");
    if message.is_empty() {
        response.code().to_string()
    } else {
        format!("{} {message}", response.code())
    }
}

fn redacted_smtp_transport_error(error: lettre::transport::smtp::Error) -> SendEmailError {
    let message = error.to_string().to_ascii_lowercase();
    let reason = if message.contains("auth")
        || message.contains("credential")
        || message.contains("535")
        || message.contains("534")
    {
        "SMTP authentication failed"
    } else if message.contains("starttls") || message.contains("tls") {
        "SMTP TLS negotiation failed"
    } else if message.contains("timeout") || message.contains("timed out") {
        "SMTP connection timed out"
    } else {
        "SMTP transport failed"
    };

    SendEmailError::SmtpTransport(reason.to_string())
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64, SendEmailError> {
    i64::try_from(value).map_err(|_| SendEmailError::Database(format!("{field} overflows i64")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;
    use std::{
        collections::{BTreeMap, VecDeque},
        fs,
        io::{self, BufRead, BufReader, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        sync::{Arc, Mutex},
        thread::{self, JoinHandle},
        time::Duration,
    };

    fn address(value: &str) -> OutboundEmailAddress {
        OutboundEmailAddress::new(None::<String>, value).unwrap()
    }

    fn request() -> SendEmailRequest {
        SendEmailRequest {
            account_id: "acct_send".into(),
            draft_id: None,
            to: vec![address("recipient@example.com")],
            cc: vec![OutboundEmailAddress::new(Some("Copy"), "copy@example.com").unwrap()],
            bcc: vec![address("hidden@example.com")],
            reply_to: Vec::new(),
            subject: "Quarterly update".into(),
            body_text: Some("Plain update".into()),
            body_html: Some(SanitizedHtml::new_sanitized("<p>Plain update</p>").unwrap()),
            attachments: vec![OutboundAttachmentRef::new("att_report").unwrap()],
            message_id: Some("<send-1@example.com>".into()),
        }
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
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
                   smtp_security
                 )
                 VALUES (
                   'acct_send',
                   'generic_imap_smtp',
                   'sender@example.com',
                   'Sender',
                   'sender@example.com',
                   'app_password',
                   'cred_acct_send',
                   'imap.example.com',
                   993,
                   'tls',
                   'smtp.example.com',
                   587,
                   'starttls'
                 )",
                [],
            )
            .unwrap();
    }

    fn seed_gmail_account(connection: &Connection) {
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
                 VALUES (
                   'acct_send',
                   'gmail',
                   'sender@gmail.com',
                   'sender@gmail.com',
                   'oauth2',
                   'cred_acct_send'
                 )",
                [],
            )
            .unwrap();
    }

    fn seed_active_draft(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO composer_drafts (
                   id,
                   account_id,
                   subject,
                   to_addresses_json,
                   body_text,
                   body_html,
                   attachment_ids_json
                 )
                 VALUES (
                   'draft_1',
                   'acct_send',
                   'Draft subject',
                   '[{\"address\":\"recipient@example.com\"}]',
                   'Draft body',
                   '<p>Draft body</p>',
                   '[\"att_report\"]'
                 )",
                [],
            )
            .unwrap();
    }

    fn seed_account_signature(connection: &Connection, signature_html: &str) {
        connection
            .execute(
                "UPDATE accounts SET signature_html = ?1 WHERE id = 'acct_send'",
                [signature_html],
            )
            .unwrap();
    }

    #[derive(Debug, Clone)]
    struct FakeAuthResolver {
        auth: SmtpAuth,
    }

    impl SmtpAuthResolver for FakeAuthResolver {
        fn resolve_smtp_auth(
            &mut self,
            _account: &SmtpAccountConfig,
        ) -> Result<SmtpAuth, SendEmailError> {
            Ok(self.auth.clone())
        }
    }

    #[derive(Debug, Default)]
    struct FakeAttachmentStore {
        attachments: BTreeMap<String, (OutboundAttachmentMetadata, Vec<u8>)>,
        chunk_sizes: Vec<usize>,
    }

    impl FakeAttachmentStore {
        fn with_text_attachment() -> Self {
            let mut store = Self::default();
            store.attachments.insert(
                "att_report".into(),
                (
                    OutboundAttachmentMetadata {
                        attachment_id: "att_report".into(),
                        display_filename: "report.txt".into(),
                        content_type: "text/plain".into(),
                        disposition: OutboundAttachmentDisposition::Attachment,
                        content_id: None,
                        size_bytes: 11,
                    },
                    b"hello world".to_vec(),
                ),
            );
            store
        }
    }

    impl AttachmentContentStore for FakeAttachmentStore {
        fn stream_attachment(
            &mut self,
            _account_id: &str,
            attachment_id: &str,
            max_bytes: u64,
            chunk_size: usize,
            on_chunk: &mut dyn FnMut(&[u8]) -> Result<(), SendEmailError>,
        ) -> Result<OutboundAttachmentMetadata, SendEmailError> {
            let (metadata, bytes) = self
                .attachments
                .get(attachment_id)
                .cloned()
                .ok_or(SendEmailError::UnknownAttachment)?;
            if bytes.len() as u64 > max_bytes {
                return Err(SendEmailError::AttachmentTooLarge {
                    attachment_id: attachment_id.into(),
                    max_bytes,
                    actual_bytes: bytes.len() as u64,
                });
            }
            for chunk in bytes.chunks(chunk_size.min(4)) {
                self.chunk_sizes.push(chunk.len());
                on_chunk(chunk)?;
            }

            Ok(metadata)
        }
    }

    #[derive(Debug, Default)]
    struct FakeSmtpSender {
        fail: bool,
        sent_messages: Vec<String>,
        seen_host: Option<String>,
        seen_port: Option<u16>,
        seen_security: Option<SmtpSecurity>,
        seen_mechanism: Option<SaslMechanism>,
    }

    impl SmtpSender for FakeSmtpSender {
        fn send(
            &mut self,
            account: &SmtpAccountConfig,
            auth: &SmtpAuth,
            message: &Message,
        ) -> Result<SmtpDelivery, SendEmailError> {
            self.seen_host = Some(account.smtp_host.clone());
            self.seen_port = Some(account.smtp_port);
            self.seen_security = Some(account.smtp_security);
            self.seen_mechanism = Some(auth.mechanism());
            self.sent_messages
                .push(String::from_utf8_lossy(&message.formatted()).into_owned());
            if self.fail {
                return Err(SendEmailError::SmtpTransport("fixture failure".into()));
            }

            Ok(SmtpDelivery {
                provider_response: Some("250 accepted".into()),
            })
        }
    }

    #[test]
    fn sends_generic_message_with_account_transport_and_records_after_confirmation() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver {
            auth: SmtpAuth::Plain {
                username: "sender@example.com".into(),
                password: SecretString::new("app-password"),
            },
        };
        let mut attachments = FakeAttachmentStore::with_text_attachment();
        let mut smtp = FakeSmtpSender::default();

        let report = send_email(
            request(),
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .unwrap();

        assert_eq!(report.rfc_message_id, "<send-1@example.com>");
        assert_eq!(report.smtp_host, "smtp.example.com");
        assert_eq!(report.smtp_port, 587);
        assert_eq!(report.smtp_security, SmtpSecurity::StartTls);
        assert_eq!(
            report.sent_folder_policy,
            SentFolderPolicy::LocalReceiptOnly
        );
        assert_eq!(
            report.sent_folder_sync_state,
            SentFolderSyncState::Unsupported
        );
        assert_eq!(report.attachment_bytes_streamed, 11);
        assert_eq!(smtp.seen_mechanism, Some(SaslMechanism::Plain));
        assert_eq!(smtp.sent_messages.len(), 1);
        assert!(smtp.sent_messages[0].contains("multipart/mixed"));
        assert!(smtp.sent_messages[0].contains("report.txt"));
        assert!(!smtp.sent_messages[0].contains("Bcc:"));
        assert!(attachments.chunk_sizes.iter().all(|size| *size <= 4));

        let receipt = connection
            .query_row(
                "SELECT
                   rfc_message_id,
                   smtp_host,
                   smtp_security,
                   auth_mechanism,
                   provider_response
                 FROM smtp_send_receipts
                 WHERE account_id = 'acct_send'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            receipt,
            (
                "<send-1@example.com>".into(),
                "smtp.example.com".into(),
                "starttls".into(),
                "plain".into(),
                Some("250 accepted".into())
            )
        );
        let sent_copy = connection
            .query_row(
                "SELECT sent_folder_policy, sync_state
                 FROM sent_mail_copies
                 WHERE send_receipt_id = (
                   SELECT id FROM smtp_send_receipts WHERE account_id = 'acct_send'
                 )",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(
            sent_copy,
            ("local_receipt_only".into(), "unsupported".into())
        );
        assert!(matches!(
            report.progress.as_slice(),
            [
                SendProgressEvent::AccountResolved,
                SendProgressEvent::AuthResolved { .. },
                SendProgressEvent::AttachmentStreamed { .. },
                SendProgressEvent::MimeComposed { .. },
                SendProgressEvent::SmtpAccepted,
                SendProgressEvent::SentStateRecorded
            ]
        ));
    }

    #[test]
    fn failed_smtp_delivery_does_not_record_sent_state() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        seed_active_draft(&connection);
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver {
            auth: SmtpAuth::Plain {
                username: "sender@example.com".into(),
                password: SecretString::new("app-password"),
            },
        };
        let mut attachments = FakeAttachmentStore::with_text_attachment();
        let mut smtp = FakeSmtpSender {
            fail: true,
            ..Default::default()
        };

        let mut request = request();
        request.draft_id = Some("draft_1".into());
        let error = send_email(
            request,
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .unwrap_err();

        assert!(matches!(error, SendEmailError::SmtpTransport(_)));
        let receipt_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM smtp_send_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(receipt_count, 0);
        let draft_state = connection
            .query_row(
                "SELECT draft_state FROM composer_drafts WHERE id = 'draft_1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(draft_state, "active");
    }

    #[test]
    fn gmail_account_uses_default_xoauth2_transport_metadata() {
        let connection = migrated_connection();
        seed_gmail_account(&connection);
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver {
            auth: SmtpAuth::XOAuth2 {
                username: "sender@gmail.com".into(),
                access_token: SecretString::new("access-token"),
            },
        };
        let mut attachments = FakeAttachmentStore::default();
        let mut smtp = FakeSmtpSender::default();
        let mut request = request();
        request.attachments.clear();
        request.message_id = Some("<gmail-send@gmail.com>".into());

        let report = send_email(
            request,
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .unwrap();

        assert_eq!(report.smtp_host, "smtp.gmail.com");
        assert_eq!(report.smtp_port, 587);
        assert_eq!(report.smtp_security, SmtpSecurity::StartTls);
        assert_eq!(
            report.sent_folder_policy,
            SentFolderPolicy::ProviderAutoCopies
        );
        assert_eq!(
            report.sent_folder_sync_state,
            SentFolderSyncState::NotRequired
        );
        assert_eq!(smtp.seen_mechanism, Some(SaslMechanism::XOAuth2));
        let auth_mechanism = connection
            .query_row("SELECT auth_mechanism FROM smtp_send_receipts", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(auth_mechanism, "xoauth2");
    }

    #[test]
    fn smtp_send_inserts_sanitized_account_signature_into_html_body() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        seed_account_signature(
            &connection,
            r#"<p onclick="alert(1)">Regards<br><strong>Sender</strong><img src="https://tracker.example/pixel.gif"></p>"#,
        );
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver {
            auth: SmtpAuth::Plain {
                username: "sender@example.com".into(),
                password: SecretString::new("app-password"),
            },
        };
        let mut attachments = FakeAttachmentStore::default();
        let mut smtp = FakeSmtpSender::default();
        let mut request = request();
        request.attachments.clear();

        send_email(
            request,
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .unwrap();

        let sent = &smtp.sent_messages[0];
        assert_eq!(
            sent.matches(crate::account_signatures::SIGNATURE_HTML_BLOCK_START)
                .count(),
            1
        );
        assert!(sent.contains("komorebi-account-signature"));
        assert!(sent.contains("Regards"));
        assert!(sent.contains("Sender"));
        assert!(!sent.contains("onclick"));
        assert!(!sent.contains("<img"));
        assert!(!sent.contains("tracker.example"));
    }

    #[test]
    fn rich_composer_html_serializes_to_outgoing_mime() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth_resolver = FakeAuthResolver {
            auth: SmtpAuth::Plain {
                username: "sender@example.com".into(),
                password: SecretString::new("app-password"),
            },
        };
        let mut attachments = FakeAttachmentStore::default();
        let mut smtp = FakeSmtpSender::default();
        let mut request = request();
        request.attachments.clear();
        request.body_text = Some("Rich formatting fallback".into());
        request.body_html = Some(
            SanitizedHtml::new_sanitized(
                r#"<p style="text-align: center"><s><span style="font-family: Georgia; font-size: 18px; color: #9a624f; background-color: #fff2a8">Rich formatting</span></s></p><table><tbody><tr><th><p>Item</p></th><th><p>Status</p></th></tr><tr><td><p>Sync</p></td><td><p>Ready</p></td></tr></tbody></table><pre><code>const ok = true;</code></pre>"#,
            )
            .unwrap(),
        );

        send_email(
            request,
            &mut account_store,
            &mut auth_resolver,
            &mut attachments,
            &mut smtp,
        )
        .expect("send rich html message");

        let sent = smtp.sent_messages[0].clone();
        let normalized_sent = sent
            .replace("=\r\n", "")
            .replace("=\n", "")
            .replace("=3D", "=");
        assert!(normalized_sent.contains("<table>"), "{sent}");
        assert!(normalized_sent.contains("<s>"), "{sent}");
        assert!(normalized_sent.contains("<pre><code>"), "{sent}");
        assert!(normalized_sent.contains("</code></pre>"), "{sent}");
        assert!(normalized_sent.contains("font-family: Georgia"), "{sent}");
        assert!(normalized_sent.contains("background-color:"), "{sent}");
    }

    #[test]
    fn confirmed_send_marks_source_draft_sent_and_clears_draft_content() {
        let connection = migrated_connection();
        seed_generic_account(&connection);
        seed_active_draft(&connection);
        let mut account_store = SqliteSmtpAccountStore::new(&connection);
        let mut auth = FakeAuthResolver {
            auth: SmtpAuth::Plain {
                username: "sender@example.com".into(),
                password: SecretString::new("app-password"),
            },
        };
        let mut attachments = FakeAttachmentStore::with_text_attachment();
        let mut smtp = FakeSmtpSender::default();
        let mut request = request();
        request.draft_id = Some("draft_1".into());
        request.message_id = Some("<draft-send@example.com>".into());

        let report = send_email(
            request,
            &mut account_store,
            &mut auth,
            &mut attachments,
            &mut smtp,
        )
        .unwrap();

        assert_eq!(report.source_draft_id, Some("draft_1".into()));
        let draft = connection
            .query_row(
                "SELECT draft_state, subject, body_text, body_html, sent_receipt_id
                 FROM composer_drafts
                 WHERE id = 'draft_1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(draft.0, "sent");
        assert_eq!(draft.1, "");
        assert!(draft.2.is_none());
        assert!(draft.3.is_none());
        assert!(draft.4.is_some());
        let source_draft_id = connection
            .query_row(
                "SELECT source_draft_id FROM smtp_send_receipts WHERE rfc_message_id = '<draft-send@example.com>'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap();
        assert_eq!(source_draft_id, Some("draft_1".into()));
        assert!(report.progress.iter().any(|event| {
            matches!(event, SendProgressEvent::DraftPrepared { draft_id } if draft_id == "draft_1")
        }));
        assert!(report.progress.iter().any(|event| {
            matches!(event, SendProgressEvent::DraftFinalized { draft_id } if draft_id == "draft_1")
        }));
    }

    #[test]
    fn rejects_unsafe_html_and_invalid_recipients_before_transport() {
        assert!(matches!(
            SanitizedHtml::new_sanitized("<script>alert(1)</script>"),
            Err(SendEmailError::InvalidRequest { .. })
        ));
        assert!(matches!(
            OutboundEmailAddress::new(None::<String>, "not an address"),
            Err(SendEmailError::InvalidAddress { .. })
        ));
    }

    #[test]
    fn request_requires_recipient_body_and_backend_attachment_ids() {
        let mut no_recipient_request = request();
        no_recipient_request.to.clear();
        no_recipient_request.cc.clear();
        no_recipient_request.bcc.clear();
        assert!(matches!(
            validate_send_request(&no_recipient_request),
            Err(SendEmailError::InvalidRequest { .. })
        ));

        let mut no_body_request = request();
        no_body_request.body_text = None;
        no_body_request.body_html = None;
        assert!(matches!(
            validate_send_request(&no_body_request),
            Err(SendEmailError::InvalidRequest { .. })
        ));

        assert!(matches!(
            OutboundAttachmentRef::new("../report"),
            Err(SendEmailError::IpcValidation(_))
        ));
    }

    #[test]
    fn lettre_sender_rejects_authenticated_plaintext_smtp_before_network() {
        let account = SmtpAccountConfig {
            account_id: "acct_send".into(),
            provider_kind: ProviderKind::GenericImapSmtp,
            auth_kind: AuthKind::AppPassword,
            display_email: "sender@example.com".into(),
            display_name: None,
            username: "sender@example.com".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: 25,
            smtp_security: SmtpSecurity::None,
            signature_html: None,
        };
        let auth = SmtpAuth::Plain {
            username: "sender@example.com".into(),
            password: SecretString::new("app-password"),
        };
        let message = Message::builder()
            .from(account_mailbox(&account).unwrap())
            .to(address("recipient@example.com").mailbox().unwrap())
            .subject("No plaintext auth")
            .singlepart(SinglePart::plain("body".to_string()))
            .unwrap();
        let mut sender = LettreSmtpSender::default();

        let error = sender.send(&account, &auth, &message).unwrap_err();

        assert_eq!(
            error,
            SendEmailError::UnsupportedAuth {
                reason: "authenticated SMTP requires TLS or STARTTLS"
            }
        );
    }

    #[test]
    fn lettre_sender_delivers_plain_auth_to_local_loopback_fixture_for_adapter_verification() {
        let server = LoopbackSmtpServer::start("PLAIN LOGIN", false);
        let account = loopback_account(server.addr().port(), AuthKind::AppPassword);
        let auth = SmtpAuth::Plain {
            username: "sender@example.com".into(),
            password: SecretString::new("app-password"),
        };
        let message = loopback_message("Plain adapter verification");
        let mut sender = LettreSmtpSender::plaintext_loopback_for_tests();

        let delivery = sender.send(&account, &auth, &message).unwrap();
        let fixture = server.join();

        assert!(delivery
            .provider_response
            .as_deref()
            .unwrap_or_default()
            .contains("250"));
        assert!(fixture
            .commands
            .iter()
            .any(|line| line.starts_with("AUTH PLAIN ")));
        assert_eq!(fixture.messages.len(), 1);
        assert!(fixture.messages[0].contains("Subject: Plain adapter verification"));
        assert!(!fixture.messages[0].contains("app-password"));
    }

    #[test]
    fn lettre_sender_uses_xoauth2_for_oauth_accounts_on_loopback_fixture() {
        let server = LoopbackSmtpServer::start("XOAUTH2 PLAIN", false);
        let mut account = loopback_account(server.addr().port(), AuthKind::OAuth2);
        account.provider_kind = ProviderKind::Gmail;
        account.display_email = "sender@gmail.com".into();
        account.username = "sender@gmail.com".into();
        let auth = SmtpAuth::XOAuth2 {
            username: "sender@gmail.com".into(),
            access_token: SecretString::new("access-token"),
        };
        let message = loopback_message("XOAUTH2 adapter verification");
        let mut sender = LettreSmtpSender::plaintext_loopback_for_tests();

        sender.send(&account, &auth, &message).unwrap();
        let fixture = server.join();

        assert!(fixture
            .commands
            .iter()
            .any(|line| line.starts_with("AUTH XOAUTH2 ")));
        assert_eq!(fixture.messages.len(), 1);
        assert!(fixture.messages[0].contains("Subject: XOAUTH2 adapter verification"));
        assert!(!fixture.messages[0].contains("access-token"));
    }

    #[test]
    fn lettre_sender_redacts_loopback_auth_failures() {
        let server = LoopbackSmtpServer::start("PLAIN LOGIN", true);
        let account = loopback_account(server.addr().port(), AuthKind::AppPassword);
        let auth = SmtpAuth::Plain {
            username: "sender@example.com".into(),
            password: SecretString::new("super-secret-password"),
        };
        let message = loopback_message("Rejected adapter verification");
        let mut sender = LettreSmtpSender::plaintext_loopback_for_tests();

        let error = sender.send(&account, &auth, &message).unwrap_err();
        let fixture = server.join();

        assert_eq!(
            error,
            SendEmailError::SmtpTransport("SMTP authentication failed".to_string())
        );
        assert!(fixture
            .commands
            .iter()
            .any(|line| line.starts_with("AUTH PLAIN ")));
        assert!(fixture.messages.is_empty());
        assert!(!error.to_string().contains("super-secret-password"));
    }

    #[test]
    fn sqlite_account_store_rejects_url_like_smtp_hosts() {
        let connection = migrated_connection();
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
                   'acct_send',
                   'generic_imap_smtp',
                   'sender@example.com',
                   'sender@example.com',
                   'app_password',
                   'cred_acct_send',
                   'imap.example.com',
                   993,
                   'tls',
                   'https://smtp.example.com',
                   587,
                   'starttls'
                 )",
                [],
            )
            .unwrap();
        let mut account_store = SqliteSmtpAccountStore::new(&connection);

        let error = account_store.load_smtp_account("acct_send").unwrap_err();

        assert!(matches!(error, SendEmailError::InvalidRequest { .. }));
    }

    #[test]
    fn attachment_storage_paths_must_stay_under_app_generated_root() {
        let root = std::env::temp_dir().join(format!(
            "komorebi-smtp-path-{}",
            stable_send_receipt_id("acct_send", "<path@example.com>")
        ));
        let nested = root.join("acct_send").join("folder_1").join("msg_1");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("att_report.txt"), b"hello").unwrap();

        assert!(safe_attachment_path(
            &root,
            "acct_send/folder_1/msg_1/att_report.txt",
            "att_report"
        )
        .unwrap()
        .starts_with(root.canonicalize().unwrap()));
        assert!(matches!(
            safe_attachment_path(&root, "../evil.txt", "att_report"),
            Err(SendEmailError::AttachmentPathRejected { .. })
        ));

        fs::remove_dir_all(root).unwrap();
    }

    fn loopback_account(port: u16, auth_kind: AuthKind) -> SmtpAccountConfig {
        SmtpAccountConfig {
            account_id: "acct_send".into(),
            provider_kind: ProviderKind::GenericImapSmtp,
            auth_kind,
            display_email: "sender@example.com".into(),
            display_name: Some("Sender".into()),
            username: "sender@example.com".into(),
            smtp_host: "127.0.0.1".into(),
            smtp_port: port,
            smtp_security: SmtpSecurity::None,
            signature_html: None,
        }
    }

    fn loopback_message(subject: &str) -> Message {
        Message::builder()
            .from("Sender <sender@example.com>".parse().unwrap())
            .to("Recipient <recipient@example.com>".parse().unwrap())
            .subject(subject)
            .singlepart(SinglePart::plain(
                "This message stays on the loopback SMTP fixture.".to_string(),
            ))
            .unwrap()
    }

    struct LoopbackSmtpServer {
        addr: SocketAddr,
        commands: Arc<Mutex<Vec<String>>>,
        messages: Arc<Mutex<Vec<String>>>,
        handle: Option<JoinHandle<()>>,
    }

    struct LoopbackSmtpResult {
        commands: Vec<String>,
        messages: Vec<String>,
    }

    impl LoopbackSmtpServer {
        fn start(mechanisms: &'static str, reject_auth: bool) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind SMTP fixture");
            let addr = listener.local_addr().expect("SMTP fixture addr");
            let commands = Arc::new(Mutex::new(Vec::new()));
            let messages = Arc::new(Mutex::new(Vec::new()));
            let thread_commands = Arc::clone(&commands);
            let thread_messages = Arc::clone(&messages);

            let handle = thread::spawn(move || {
                let (stream, _) = listener.accept().expect("accept SMTP fixture");
                handle_loopback_smtp(
                    stream,
                    mechanisms,
                    reject_auth,
                    thread_commands,
                    thread_messages,
                )
                .expect("serve SMTP fixture");
            });

            Self {
                addr,
                commands,
                messages,
                handle: Some(handle),
            }
        }

        fn addr(&self) -> SocketAddr {
            self.addr
        }

        fn join(mut self) -> LoopbackSmtpResult {
            self.handle
                .take()
                .expect("SMTP fixture handle")
                .join()
                .expect("SMTP fixture exits");
            LoopbackSmtpResult {
                commands: self.commands.lock().expect("commands").clone(),
                messages: self.messages.lock().expect("messages").clone(),
            }
        }
    }

    fn handle_loopback_smtp(
        stream: TcpStream,
        mechanisms: &str,
        reject_auth: bool,
        commands: Arc<Mutex<Vec<String>>>,
        messages: Arc<Mutex<Vec<String>>>,
    ) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut writer = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        write_smtp_line(&mut writer, "220 komorebi.test loopback SMTP ready")?;

        loop {
            let Some(line) = read_smtp_line(&mut reader)? else {
                return Ok(());
            };
            commands.lock().expect("commands").push(line.clone());
            let upper = line.to_ascii_uppercase();

            if upper.starts_with("EHLO ") || upper.starts_with("HELO ") {
                write_smtp_line(&mut writer, "250-komorebi.test")?;
                write_smtp_line(&mut writer, &format!("250-AUTH {mechanisms}"))?;
                write_smtp_line(&mut writer, "250 SIZE 52428800")?;
            } else if upper.starts_with("AUTH ") {
                if reject_auth {
                    write_smtp_line(&mut writer, "535 5.7.8 Authentication credentials invalid")?;
                    return Ok(());
                }
                write_smtp_line(&mut writer, "235 2.7.0 Authentication successful")?;
            } else if upper.starts_with("MAIL FROM:") {
                write_smtp_line(&mut writer, "250 2.1.0 OK")?;
            } else if upper.starts_with("RCPT TO:") {
                write_smtp_line(&mut writer, "250 2.1.5 OK")?;
            } else if upper == "DATA" {
                write_smtp_line(&mut writer, "354 End data with <CR><LF>.<CR><LF>")?;
                let mut message = String::new();
                loop {
                    let Some(data_line) = read_smtp_line(&mut reader)? else {
                        return Ok(());
                    };
                    if data_line == "." {
                        break;
                    }
                    message.push_str(&data_line);
                    message.push_str("\r\n");
                }
                messages.lock().expect("messages").push(message);
                write_smtp_line(&mut writer, "250 2.0.0 queued")?;
            } else if upper == "QUIT" {
                write_smtp_line(&mut writer, "221 2.0.0 Bye")?;
                return Ok(());
            } else {
                write_smtp_line(&mut writer, "502 5.5.2 Command not recognized")?;
            }
        }
    }

    fn read_smtp_line(reader: &mut BufReader<TcpStream>) -> io::Result<Option<String>> {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
    }

    fn write_smtp_line(writer: &mut TcpStream, line: &str) -> io::Result<()> {
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\r\n")?;
        writer.flush()
    }

    #[derive(Default)]
    struct FakePasswordStore {
        password: Option<SecretString>,
    }

    impl PasswordCredentialStore for FakePasswordStore {
        fn load_generic_password(
            &mut self,
            _account_id: &str,
        ) -> Result<Option<SecretString>, SecretStoreError> {
            Ok(self.password.clone())
        }
    }

    #[derive(Default)]
    struct FakeCredentialStore {
        refresh: Option<SecretString>,
    }

    impl CredentialStore for FakeCredentialStore {
        fn load_refresh_token(
            &mut self,
            _account_id: &str,
        ) -> Result<Option<SecretString>, SecretStoreError> {
            Ok(self.refresh.clone())
        }

        fn store_refresh_token(
            &mut self,
            _account_id: &str,
            token: SecretString,
        ) -> Result<(), SecretStoreError> {
            self.refresh = Some(token);
            Ok(())
        }

        fn delete_account_secrets(&mut self, _account_id: &str) -> Result<(), SecretStoreError> {
            self.refresh = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeAccessCache;

    impl AccessTokenCache for FakeAccessCache {
        fn load_access_token(
            &mut self,
            _account_id: &str,
        ) -> Option<crate::token_lifecycle::CachedAccessToken> {
            None
        }

        fn store_access_token(
            &mut self,
            _account_id: &str,
            _token: crate::token_lifecycle::CachedAccessToken,
        ) {
        }

        fn clear_access_token(&mut self, _account_id: &str) {}
    }

    #[derive(Default)]
    struct FakeOAuthClient;

    impl OAuthTokenClient for FakeOAuthClient {
        fn refresh_access_token(
            &mut self,
            _provider: ProviderKind,
            _refresh_token: &SecretString,
            _now: u64,
        ) -> Result<
            crate::token_lifecycle::OAuthRefreshResponse,
            crate::token_lifecycle::RefreshFailure,
        > {
            Ok(crate::token_lifecycle::OAuthRefreshResponse {
                access_token: SecretString::new("fresh-access-token"),
                access_expires_at: 10_000,
                replacement_refresh_token: None,
            })
        }

        fn revoke_refresh_token(
            &mut self,
            _provider: ProviderKind,
            _refresh_token: &SecretString,
        ) -> crate::token_lifecycle::RevocationOutcome {
            crate::token_lifecycle::RevocationOutcome::ProviderRevocationUnsupported
        }
    }

    #[derive(Default)]
    struct FakeAccountData {
        reconnects: VecDeque<ReconnectReason>,
    }

    impl AccountDataStore for FakeAccountData {
        fn mark_reconnect_needed(
            &mut self,
            _account_id: &str,
            reason: ReconnectReason,
        ) -> Result<(), crate::token_lifecycle::AccountDataError> {
            self.reconnects.push_back(reason);
            Ok(())
        }

        fn delete_local_cache(
            &mut self,
            _account_id: &str,
        ) -> Result<(), crate::token_lifecycle::AccountDataError> {
            Ok(())
        }

        fn delete_account_metadata(
            &mut self,
            _account_id: &str,
        ) -> Result<(), crate::token_lifecycle::AccountDataError> {
            Ok(())
        }
    }

    #[test]
    fn backend_auth_resolver_uses_password_or_refreshes_oauth_without_returning_secret_to_frontend()
    {
        let password_store = FakePasswordStore {
            password: Some(SecretString::new("generic-secret")),
        };
        let lifecycle = TokenLifecycle::new(
            FakeCredentialStore::default(),
            FakeAccessCache,
            FakeOAuthClient,
            FakeAccountData::default(),
        );
        let mut resolver = BackendSmtpAuthResolver::new(password_store, lifecycle, 1_000);
        let generic = SmtpAccountConfig {
            account_id: "acct_send".into(),
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

        let auth = resolver.resolve_smtp_auth(&generic).unwrap();
        assert_eq!(auth.mechanism(), SaslMechanism::Plain);
        assert_eq!(
            format!("{auth:?}"),
            "Plain { username: \"sender@example.com\", password: [redacted] }"
        );

        let password_store = FakePasswordStore::default();
        let lifecycle = TokenLifecycle::new(
            FakeCredentialStore {
                refresh: Some(SecretString::new("refresh")),
            },
            FakeAccessCache,
            FakeOAuthClient,
            FakeAccountData::default(),
        );
        let mut resolver = BackendSmtpAuthResolver::new(password_store, lifecycle, 1_000);
        let gmail = SmtpAccountConfig {
            provider_kind: ProviderKind::Gmail,
            auth_kind: AuthKind::OAuth2,
            display_email: "sender@gmail.com".into(),
            username: "sender@gmail.com".into(),
            smtp_host: "smtp.gmail.com".into(),
            ..generic
        };
        let auth = resolver.resolve_smtp_auth(&gmail).unwrap();
        assert_eq!(auth.mechanism(), SaslMechanism::XOAuth2);
        assert_eq!(
            format!("{auth:?}"),
            "XOAuth2 { username: \"sender@gmail.com\", access_token: [redacted] }"
        );
    }
}
