use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{
    imap_live::{LiveImapAdapter, LiveImapStoreMode},
    imap_sync::{RemoteFlag, SyncError},
    ipc_validation::{validate_entity_id, IpcValidationError},
    schema::{apply_migrations, SchemaMigrationError},
    sync_scheduler::{self, BackgroundSyncScheduler, SyncSchedulerError},
};

pub const MAX_MAIL_ACTION_ATTEMPTS: u8 = 3;
const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserConfirmation {
    NotConfirmed,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageActionKind<'a> {
    MarkRead { read: bool },
    Move { target_folder_id: &'a str },
    Archive { archive_folder_id: &'a str },
    DeleteToTrash { trash_folder_id: &'a str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailActionTargetFolderKind {
    Archive,
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderActionKind {
    ExpungeDeleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailAction<'a> {
    Message {
        folder_id: &'a str,
        message_id: &'a str,
        kind: MessageActionKind<'a>,
    },
    Folder {
        folder_id: &'a str,
        kind: FolderActionKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailActionCommand<'a> {
    pub account_id: &'a str,
    pub action: MailAction<'a>,
    pub confirmation: UserConfirmation,
    pub attempt: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionReversibility {
    Reversible,
    Irreversible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationRequirement {
    NotRequired,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailActionPlan {
    pub reversibility: ActionReversibility,
    pub confirmation: ConfirmationRequirement,
    pub max_attempts: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoMailAction {
    MarkRead {
        account_id: String,
        folder_id: String,
        message_id: String,
        read: bool,
    },
    Move {
        account_id: String,
        from_folder_id: String,
        to_folder_id: String,
        message_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteMutation {
    FlagsUpdated {
        read: bool,
    },
    Moved {
        from_folder_id: String,
        to_folder_id: String,
    },
    Expunged {
        folder_id: String,
        removed_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailActionReceipt {
    pub mutation: RemoteMutation,
    pub undo: Option<UndoMailAction>,
    pub retry_policy: RetryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    Retryable { max_attempts: u8 },
    NotRetryable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    RetryNow { next_attempt: u8, max_attempts: u8 },
    Exhausted,
    NotRetryable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailActionError {
    IpcValidation(IpcValidationError),
    ConfirmationRequired,
    UnknownAccount,
    UnknownFolder {
        folder_id: String,
    },
    UnknownMessage {
        folder_id: String,
        message_id: String,
    },
    TransientServerFailure(String),
    PermanentServerFailure(String),
}

impl fmt::Display for MailActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::ConfirmationRequired => write!(formatter, "mail action requires confirmation"),
            Self::UnknownAccount => write!(formatter, "account was not found"),
            Self::UnknownFolder { .. } => write!(formatter, "folder was not found"),
            Self::UnknownMessage { .. } => write!(formatter, "message was not found"),
            Self::TransientServerFailure(_) => write!(formatter, "mail server operation can retry"),
            Self::PermanentServerFailure(_) => write!(formatter, "mail server operation failed"),
        }
    }
}

impl Error for MailActionError {}

impl From<IpcValidationError> for MailActionError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailActionFailure {
    pub error: MailActionError,
    pub retry: RetryDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailActionStoreError {
    Transient(String),
    Permanent(String),
}

pub trait MailActionStore {
    fn account_exists(&self, account_id: &str) -> bool;
    fn folder_exists(&self, account_id: &str, folder_id: &str) -> bool;
    fn message_exists(&self, account_id: &str, folder_id: &str, message_id: &str) -> bool;
    fn folder_id_for_kind(
        &self,
        _account_id: &str,
        _kind: MailActionTargetFolderKind,
    ) -> Option<String> {
        None
    }
    fn set_seen(
        &mut self,
        account_id: &str,
        folder_id: &str,
        message_id: &str,
        read: bool,
    ) -> Result<(), MailActionStoreError>;
    fn move_message(
        &mut self,
        account_id: &str,
        from_folder_id: &str,
        message_id: &str,
        to_folder_id: &str,
    ) -> Result<(), MailActionStoreError>;
    fn expunge_deleted(
        &mut self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<usize, MailActionStoreError>;
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PerformMailActionRequest {
    pub account_id: String,
    pub folder_id: String,
    pub message_id: Option<String>,
    pub action: PerformMailActionKind,
    pub confirmed: bool,
    pub user_requested: bool,
    pub attempt: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PerformMailActionKind {
    MarkRead {
        read: bool,
    },
    Move {
        #[serde(rename = "targetFolderId")]
        target_folder_id: String,
    },
    Archive,
    DeleteToTrash,
    ExpungeDeleted,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailActionCommandResponse {
    pub account_id: String,
    pub action: PerformMailActionKind,
    pub mutation: MailActionMutationView,
    pub undo: Option<MailActionUndoView>,
    pub retry: MailActionRetryPolicyView,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MailActionMutationView {
    FlagsUpdated {
        read: bool,
    },
    Moved {
        #[serde(rename = "fromFolderId")]
        from_folder_id: String,
        #[serde(rename = "toFolderId")]
        to_folder_id: String,
    },
    Expunged {
        #[serde(rename = "folderId")]
        folder_id: String,
        #[serde(rename = "removedCount")]
        removed_count: usize,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailActionUndoView {
    pub account_id: String,
    pub folder_id: String,
    pub message_id: String,
    pub action: PerformMailActionKind,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MailActionRetryPolicyView {
    Retryable {
        #[serde(rename = "maxAttempts")]
        max_attempts: u8,
    },
    NotRetryable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MailActionRetryDecisionView {
    RetryNow {
        #[serde(rename = "nextAttempt")]
        next_attempt: u8,
        #[serde(rename = "maxAttempts")]
        max_attempts: u8,
    },
    Exhausted,
    NotRetryable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MailActionCommandError {
    pub reason: String,
    pub retry: MailActionRetryDecisionView,
}

impl MailActionCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            retry: MailActionRetryDecisionView::NotRetryable,
        }
    }
}

impl From<IpcValidationError> for MailActionCommandError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<rusqlite::Error> for MailActionCommandError {
    fn from(_value: rusqlite::Error) -> Self {
        Self::new("mail action cache access failed")
    }
}

impl From<SchemaMigrationError> for MailActionCommandError {
    fn from(_value: SchemaMigrationError) -> Self {
        Self::new("mail action cache migration failed")
    }
}

impl From<SyncSchedulerError> for MailActionCommandError {
    fn from(value: SyncSchedulerError) -> Self {
        Self::new(value.reason)
    }
}

impl From<tauri::Error> for MailActionCommandError {
    fn from(_value: tauri::Error) -> Self {
        Self::new("mail action app data unavailable")
    }
}

impl From<std::io::Error> for MailActionCommandError {
    fn from(_value: std::io::Error) -> Self {
        Self::new("mail action filesystem operation failed")
    }
}

impl From<MailActionFailure> for MailActionCommandError {
    fn from(value: MailActionFailure) -> Self {
        Self {
            reason: value.error.to_string(),
            retry: retry_decision_view(value.retry),
        }
    }
}

#[tauri::command]
pub async fn perform_mail_action(
    app: tauri::AppHandle,
    request: PerformMailActionRequest,
) -> Result<MailActionCommandResponse, MailActionCommandError> {
    if !request.user_requested {
        return Err(MailActionCommandError::new(
            "mail action requires explicit user intent",
        ));
    }
    validate_entity_id("account_id", &request.account_id)?;

    let account_id = request.account_id.clone();
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    let stopped = scheduler.stop_account(&account_id).unwrap_or(false);
    let refresh_path = cache_path.clone();
    let refresh_account_id = account_id.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        perform_mail_action_from_path(&cache_path, &app_data_dir, request)
    })
    .await
    .map_err(|_| MailActionCommandError::new("mail action worker failed"))?;

    if stopped {
        let _ = scheduler.refresh_account(refresh_path, &refresh_account_id);
    }

    result
}

pub fn perform_mail_action_from_path(
    cache_path: &Path,
    app_data_dir: &Path,
    request: PerformMailActionRequest,
) -> Result<MailActionCommandResponse, MailActionCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    let config = sync_scheduler::load_live_imap_config(&connection, &request.account_id)?;
    let mut server = LiveImapAdapter::connect(config).map_err(command_error_from_sync_error)?;
    let result =
        perform_mail_action_from_connection(&mut connection, app_data_dir, &mut server, request);
    let _ = server.logout();

    result
}

pub fn perform_mail_action_from_connection(
    connection: &mut Connection,
    app_data_dir: &Path,
    server: &mut LiveImapAdapter,
    request: PerformMailActionRequest,
) -> Result<MailActionCommandResponse, MailActionCommandError> {
    let mut store = SqliteLiveMailActionStore::new(connection, app_data_dir, server);
    perform_mail_action_with_store(&mut store, request)
}

pub fn perform_mail_action_with_store(
    store: &mut impl MailActionStore,
    request: PerformMailActionRequest,
) -> Result<MailActionCommandResponse, MailActionCommandError> {
    if !request.user_requested {
        return Err(MailActionCommandError::new(
            "mail action requires explicit user intent",
        ));
    }
    validate_entity_id("account_id", &request.account_id)?;
    validate_entity_id("folder_id", &request.folder_id)?;
    if let Some(message_id) = request.message_id.as_deref() {
        validate_entity_id("message_id", message_id)?;
    }

    let receipt = execute_request_with_store(store, &request)?;

    Ok(MailActionCommandResponse {
        account_id: request.account_id,
        action: request.action,
        mutation: mutation_view(&receipt.mutation),
        undo: receipt.undo.as_ref().map(undo_view),
        retry: retry_policy_view(receipt.retry_policy),
    })
}

fn execute_request_with_store(
    store: &mut impl MailActionStore,
    request: &PerformMailActionRequest,
) -> Result<MailActionReceipt, MailActionCommandError> {
    let confirmation = if request.confirmed {
        UserConfirmation::Confirmed
    } else {
        UserConfirmation::NotConfirmed
    };
    let attempt = request
        .attempt
        .unwrap_or(1)
        .clamp(1, MAX_MAIL_ACTION_ATTEMPTS);
    let action = match &request.action {
        PerformMailActionKind::MarkRead { read } => MailAction::Message {
            folder_id: &request.folder_id,
            message_id: required_message_id(request)?,
            kind: MessageActionKind::MarkRead { read: *read },
        },
        PerformMailActionKind::Move { target_folder_id } => {
            validate_entity_id("target_folder_id", target_folder_id)?;
            MailAction::Message {
                folder_id: &request.folder_id,
                message_id: required_message_id(request)?,
                kind: MessageActionKind::Move { target_folder_id },
            }
        }
        PerformMailActionKind::Archive => {
            let archive_folder_id = store
                .folder_id_for_kind(&request.account_id, MailActionTargetFolderKind::Archive)
                .ok_or_else(|| MailActionCommandError::new("archive folder was not found"))?;
            let command = MailActionCommand {
                account_id: &request.account_id,
                action: MailAction::Message {
                    folder_id: &request.folder_id,
                    message_id: required_message_id(request)?,
                    kind: MessageActionKind::Archive {
                        archive_folder_id: &archive_folder_id,
                    },
                },
                confirmation,
                attempt,
            };
            return execute_mail_action(store, command).map_err(Into::into);
        }
        PerformMailActionKind::DeleteToTrash => {
            let trash_folder_id = store
                .folder_id_for_kind(&request.account_id, MailActionTargetFolderKind::Trash)
                .ok_or_else(|| MailActionCommandError::new("trash folder was not found"))?;
            let command = MailActionCommand {
                account_id: &request.account_id,
                action: MailAction::Message {
                    folder_id: &request.folder_id,
                    message_id: required_message_id(request)?,
                    kind: MessageActionKind::DeleteToTrash {
                        trash_folder_id: &trash_folder_id,
                    },
                },
                confirmation,
                attempt,
            };
            return execute_mail_action(store, command).map_err(Into::into);
        }
        PerformMailActionKind::ExpungeDeleted => MailAction::Folder {
            folder_id: &request.folder_id,
            kind: FolderActionKind::ExpungeDeleted,
        },
    };

    execute_mail_action(
        store,
        MailActionCommand {
            account_id: &request.account_id,
            action,
            confirmation,
            attempt,
        },
    )
    .map_err(Into::into)
}

fn required_message_id(request: &PerformMailActionRequest) -> Result<&str, MailActionCommandError> {
    request
        .message_id
        .as_deref()
        .ok_or_else(|| MailActionCommandError::new("message_id is required"))
}

fn mutation_view(mutation: &RemoteMutation) -> MailActionMutationView {
    match mutation {
        RemoteMutation::FlagsUpdated { read } => {
            MailActionMutationView::FlagsUpdated { read: *read }
        }
        RemoteMutation::Moved {
            from_folder_id,
            to_folder_id,
        } => MailActionMutationView::Moved {
            from_folder_id: from_folder_id.clone(),
            to_folder_id: to_folder_id.clone(),
        },
        RemoteMutation::Expunged {
            folder_id,
            removed_count,
        } => MailActionMutationView::Expunged {
            folder_id: folder_id.clone(),
            removed_count: *removed_count,
        },
    }
}

fn undo_view(undo: &UndoMailAction) -> MailActionUndoView {
    match undo {
        UndoMailAction::MarkRead {
            account_id,
            folder_id,
            message_id,
            read,
        } => MailActionUndoView {
            account_id: account_id.clone(),
            folder_id: folder_id.clone(),
            message_id: message_id.clone(),
            action: PerformMailActionKind::MarkRead { read: *read },
        },
        UndoMailAction::Move {
            account_id,
            from_folder_id,
            to_folder_id,
            message_id,
        } => MailActionUndoView {
            account_id: account_id.clone(),
            folder_id: from_folder_id.clone(),
            message_id: message_id.clone(),
            action: PerformMailActionKind::Move {
                target_folder_id: to_folder_id.clone(),
            },
        },
    }
}

fn retry_policy_view(policy: RetryPolicy) -> MailActionRetryPolicyView {
    match policy {
        RetryPolicy::Retryable { max_attempts } => {
            MailActionRetryPolicyView::Retryable { max_attempts }
        }
        RetryPolicy::NotRetryable => MailActionRetryPolicyView::NotRetryable,
    }
}

fn retry_decision_view(decision: RetryDecision) -> MailActionRetryDecisionView {
    match decision {
        RetryDecision::RetryNow {
            next_attempt,
            max_attempts,
        } => MailActionRetryDecisionView::RetryNow {
            next_attempt,
            max_attempts,
        },
        RetryDecision::Exhausted => MailActionRetryDecisionView::Exhausted,
        RetryDecision::NotRetryable => MailActionRetryDecisionView::NotRetryable,
    }
}

#[derive(Debug, Clone)]
struct MessageActionTarget {
    folder_remote_id: String,
    uid_validity: u64,
    uid: u64,
    rfc_message_id: Option<String>,
    flags: BTreeSet<RemoteFlag>,
}

#[derive(Debug, Clone)]
struct FolderActionTarget {
    remote_id: String,
}

struct SqliteLiveMailActionStore<'connection, 'server> {
    connection: &'connection mut Connection,
    app_data_dir: PathBuf,
    server: &'server mut LiveImapAdapter,
}

impl<'connection, 'server> SqliteLiveMailActionStore<'connection, 'server> {
    fn new(
        connection: &'connection mut Connection,
        app_data_dir: impl Into<PathBuf>,
        server: &'server mut LiveImapAdapter,
    ) -> Self {
        Self {
            connection,
            app_data_dir: app_data_dir.into(),
            server,
        }
    }

    fn load_message_target(
        &self,
        account_id: &str,
        folder_id: &str,
        message_id: &str,
    ) -> Result<MessageActionTarget, MailActionStoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT folders.remote_id,
                        messages.uid_validity,
                        messages.imap_uid,
                        messages.rfc_message_id,
                        messages.flags_json
                 FROM messages
                 JOIN folders ON folders.account_id = messages.account_id
                  AND folders.id = messages.folder_id
                 JOIN accounts ON accounts.id = messages.account_id
                 WHERE accounts.removed_at IS NULL
                   AND accounts.auth_status = 'connected'
                   AND messages.account_id = ?1
                   AND messages.folder_id = ?2
                   AND messages.id = ?3
                   AND messages.uid_validity IS NOT NULL
                   AND messages.imap_uid IS NOT NULL",
                params![account_id, folder_id, message_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error_from_sql)?
            .ok_or_else(|| MailActionStoreError::Permanent("message was not found".into()))?;

        Ok(MessageActionTarget {
            folder_remote_id: row.0,
            uid_validity: i64_to_u64(row.1)?,
            uid: i64_to_u64(row.2)?,
            rfc_message_id: row.3,
            flags: flags_from_json(&row.4)?,
        })
    }

    fn load_folder_target(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<FolderActionTarget, MailActionStoreError> {
        self.connection
            .query_row(
                "SELECT folders.remote_id
                 FROM folders
                 JOIN accounts ON accounts.id = folders.account_id
                 WHERE accounts.removed_at IS NULL
                   AND accounts.auth_status = 'connected'
                   AND folders.account_id = ?1
                   AND folders.id = ?2",
                params![account_id, folder_id],
                |row| {
                    Ok(FolderActionTarget {
                        remote_id: row.get(0)?,
                    })
                },
            )
            .optional()
            .map_err(store_error_from_sql)?
            .ok_or_else(|| MailActionStoreError::Permanent("folder was not found".into()))
    }

    fn update_message_flags(
        &mut self,
        account_id: &str,
        folder_id: &str,
        message_id: &str,
        flags: &BTreeSet<RemoteFlag>,
    ) -> Result<(), MailActionStoreError> {
        let flags_json = flags_json(flags)?;
        self.connection
            .execute(
                "UPDATE messages
                 SET is_read = ?4,
                     is_flagged = ?5,
                     is_answered = ?6,
                     is_draft = ?7,
                     is_deleted = ?8,
                     flags_json = ?9,
                     sync_state = 'synced'
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND id = ?3",
                params![
                    account_id,
                    folder_id,
                    message_id,
                    bool_to_i64(flags.contains(&RemoteFlag::Seen)),
                    bool_to_i64(flags.contains(&RemoteFlag::Flagged)),
                    bool_to_i64(flags.contains(&RemoteFlag::Answered)),
                    bool_to_i64(flags.contains(&RemoteFlag::Draft)),
                    bool_to_i64(flags.contains(&RemoteFlag::Deleted)),
                    flags_json
                ],
            )
            .map_err(store_error_from_sql)?;
        self.refresh_folder_counts(account_id, folder_id)
    }

    fn move_cached_message(
        &mut self,
        account_id: &str,
        source_folder_id: &str,
        message_id: &str,
        target_folder_id: &str,
        destination_uid_validity: u64,
        destination_uid: u64,
    ) -> Result<(), MailActionStoreError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(store_error_from_sql)?;
        transaction
            .execute_batch("PRAGMA defer_foreign_keys = ON;")
            .map_err(store_error_from_sql)?;
        transaction
            .execute(
                "UPDATE messages
                 SET folder_id = ?4,
                     uid_validity = ?5,
                     imap_uid = ?6,
                     sync_state = 'synced'
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND id = ?3",
                params![
                    account_id,
                    source_folder_id,
                    message_id,
                    target_folder_id,
                    u64_to_i64(destination_uid_validity)?,
                    u64_to_i64(destination_uid)?
                ],
            )
            .map_err(store_error_from_sql)?;
        transaction
            .execute(
                "UPDATE attachments
                 SET folder_id = ?4
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND message_id = ?3",
                params![account_id, source_folder_id, message_id, target_folder_id],
            )
            .map_err(store_error_from_sql)?;
        refresh_folder_counts_in_connection(&transaction, account_id, source_folder_id)?;
        refresh_folder_counts_in_connection(&transaction, account_id, target_folder_id)?;
        transaction.commit().map_err(store_error_from_sql)
    }

    fn delete_cached_deleted_messages(
        &mut self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<usize, MailActionStoreError> {
        let storage_paths = self
            .storage_paths_for_deleted_messages(account_id, folder_id)
            .map_err(store_error_from_sql)?;
        let removed = self
            .connection
            .execute(
                "DELETE FROM messages
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND is_deleted = 1",
                params![account_id, folder_id],
            )
            .map_err(store_error_from_sql)?;
        self.delete_attachment_files_best_effort(&storage_paths);
        self.refresh_folder_counts(account_id, folder_id)?;
        Ok(removed)
    }

    fn storage_paths_for_deleted_messages(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> rusqlite::Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT attachments.storage_relative_path
             FROM attachments
             JOIN messages ON messages.account_id = attachments.account_id
              AND messages.folder_id = attachments.folder_id
              AND messages.id = attachments.message_id
             WHERE messages.account_id = ?1
               AND messages.folder_id = ?2
               AND messages.is_deleted = 1
               AND attachments.storage_relative_path IS NOT NULL",
        )?;
        let rows = statement.query_map(params![account_id, folder_id], |row| {
            row.get::<_, String>(0)
        })?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(row?);
        }
        Ok(paths)
    }

    fn refresh_folder_counts(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<(), MailActionStoreError> {
        refresh_folder_counts_in_connection(self.connection, account_id, folder_id)
    }

    fn delete_attachment_files_best_effort(&self, relative_paths: &[String]) {
        let Ok(root) = self.app_data_dir.canonicalize() else {
            return;
        };
        for relative_path in relative_paths {
            let Ok(path) = self.app_data_dir.join(relative_path).canonicalize() else {
                continue;
            };
            if path.starts_with(&root) && fs::remove_file(&path).is_ok() {
                remove_empty_dirs_best_effort(&root, path.parent());
            }
        }
    }
}

impl MailActionStore for SqliteLiveMailActionStore<'_, '_> {
    fn account_exists(&self, account_id: &str) -> bool {
        self.connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1
                   FROM accounts
                   WHERE id = ?1
                     AND removed_at IS NULL
                     AND auth_status = 'connected'
                 )",
                params![account_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    fn folder_exists(&self, account_id: &str, folder_id: &str) -> bool {
        self.connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1
                   FROM folders
                   JOIN accounts ON accounts.id = folders.account_id
                   WHERE accounts.removed_at IS NULL
                     AND accounts.auth_status = 'connected'
                     AND folders.account_id = ?1
                     AND folders.id = ?2
                 )",
                params![account_id, folder_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    fn message_exists(&self, account_id: &str, folder_id: &str, message_id: &str) -> bool {
        self.connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1
                   FROM messages
                   JOIN accounts ON accounts.id = messages.account_id
                   WHERE accounts.removed_at IS NULL
                     AND accounts.auth_status = 'connected'
                     AND messages.account_id = ?1
                     AND messages.folder_id = ?2
                     AND messages.id = ?3
                     AND messages.uid_validity IS NOT NULL
                     AND messages.imap_uid IS NOT NULL
                 )",
                params![account_id, folder_id, message_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    fn folder_id_for_kind(
        &self,
        account_id: &str,
        kind: MailActionTargetFolderKind,
    ) -> Option<String> {
        let folder_kind = match kind {
            MailActionTargetFolderKind::Archive => "archive",
            MailActionTargetFolderKind::Trash => "trash",
        };
        self.connection
            .query_row(
                "SELECT id
                 FROM folders
                 WHERE account_id = ?1
                   AND kind = ?2
                 ORDER BY
                   CASE WHEN special_use = ?2 THEN 0 ELSE 1 END,
                   display_name COLLATE NOCASE,
                   id
                 LIMIT 1",
                params![account_id, folder_kind],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    fn set_seen(
        &mut self,
        account_id: &str,
        folder_id: &str,
        message_id: &str,
        read: bool,
    ) -> Result<(), MailActionStoreError> {
        let target = self.load_message_target(account_id, folder_id, message_id)?;
        let mut flag = BTreeSet::new();
        flag.insert(RemoteFlag::Seen);
        let updates = self
            .server
            .store_flags(
                account_id,
                &target.folder_remote_id,
                &[target.uid],
                if read {
                    LiveImapStoreMode::Add
                } else {
                    LiveImapStoreMode::Remove
                },
                &flag,
            )
            .map_err(store_error_from_sync)?;
        let flags = updates
            .into_iter()
            .find(|update| update.uid_validity == target.uid_validity && update.uid == target.uid)
            .map(|update| update.flags)
            .unwrap_or_else(|| {
                let mut flags = target.flags;
                if read {
                    flags.insert(RemoteFlag::Seen);
                } else {
                    flags.remove(&RemoteFlag::Seen);
                }
                flags
            });
        self.update_message_flags(account_id, folder_id, message_id, &flags)
    }

    fn move_message(
        &mut self,
        account_id: &str,
        from_folder_id: &str,
        message_id: &str,
        to_folder_id: &str,
    ) -> Result<(), MailActionStoreError> {
        let source = self.load_message_target(account_id, from_folder_id, message_id)?;
        let target = self.load_folder_target(account_id, to_folder_id)?;
        let result = self
            .server
            .move_messages(
                account_id,
                &source.folder_remote_id,
                &target.remote_id,
                &[source.uid],
            )
            .map_err(store_error_from_sync)?;
        let destination_uid_validity = result
            .destination_uid_validity
            .unwrap_or(source.uid_validity);
        let destination_uid = result
            .destination_uids
            .first()
            .copied()
            .or_else(|| {
                source.rfc_message_id.as_ref().and_then(|message_id| {
                    self.server
                        .find_message_uids_by_message_id(account_id, &target.remote_id, message_id)
                        .ok()
                        .and_then(|uids| uids.into_iter().next())
                })
            })
            .ok_or_else(|| {
                MailActionStoreError::Transient("IMAP MOVE did not return a destination UID".into())
            })?;
        self.move_cached_message(
            account_id,
            from_folder_id,
            message_id,
            to_folder_id,
            destination_uid_validity,
            destination_uid,
        )
    }

    fn expunge_deleted(
        &mut self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<usize, MailActionStoreError> {
        let target = self.load_folder_target(account_id, folder_id)?;
        let remote_removed = self
            .server
            .expunge_folder(account_id, &target.remote_id)
            .map_err(store_error_from_sync)?;
        let local_removed = self.delete_cached_deleted_messages(account_id, folder_id)?;

        Ok(remote_removed.max(local_removed))
    }
}

fn refresh_folder_counts_in_connection(
    connection: &Connection,
    account_id: &str,
    folder_id: &str,
) -> Result<(), MailActionStoreError> {
    connection
        .execute(
            "UPDATE folders
             SET unread_count = (
                   SELECT COUNT(*)
                   FROM messages
                   WHERE account_id = ?1
                     AND folder_id = ?2
                     AND is_read = 0
                 ),
                 total_count = (
                   SELECT COUNT(*)
                   FROM messages
                   WHERE account_id = ?1
                     AND folder_id = ?2
                 )
             WHERE account_id = ?1
               AND id = ?2",
            params![account_id, folder_id],
        )
        .map_err(store_error_from_sql)?;
    Ok(())
}

fn flags_from_json(value: &str) -> Result<BTreeSet<RemoteFlag>, MailActionStoreError> {
    let values: Vec<String> = serde_json::from_str(value)
        .map_err(|error| MailActionStoreError::Permanent(error.to_string()))?;
    Ok(RemoteFlag::from_imap_tokens(
        values.iter().map(String::as_str),
    ))
}

fn flags_json(flags: &BTreeSet<RemoteFlag>) -> Result<String, MailActionStoreError> {
    let values: Vec<String> = flags.iter().map(RemoteFlag::as_imap_string).collect();
    serde_json::to_string(&values)
        .map_err(|error| MailActionStoreError::Permanent(error.to_string()))
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn i64_to_u64(value: i64) -> Result<u64, MailActionStoreError> {
    u64::try_from(value).map_err(|_| MailActionStoreError::Permanent("negative UID".into()))
}

fn u64_to_i64(value: u64) -> Result<i64, MailActionStoreError> {
    i64::try_from(value).map_err(|_| MailActionStoreError::Permanent("UID overflow".into()))
}

fn store_error_from_sql(_error: rusqlite::Error) -> MailActionStoreError {
    MailActionStoreError::Permanent("mail action cache update failed".into())
}

fn store_error_from_sync(error: SyncError) -> MailActionStoreError {
    match error {
        SyncError::TransientServer { reason } => MailActionStoreError::Transient(reason),
        SyncError::Cancelled => MailActionStoreError::Transient("mail action was cancelled".into()),
        SyncError::PermanentServer { reason } | SyncError::Cache { reason } => {
            MailActionStoreError::Permanent(reason)
        }
        SyncError::UnknownAccount => {
            MailActionStoreError::Permanent("account was not found".into())
        }
        SyncError::UnknownFolder => MailActionStoreError::Permanent("folder was not found".into()),
        SyncError::InvalidRemoteState { reason } => {
            MailActionStoreError::Permanent(reason.to_string())
        }
    }
}

fn command_error_from_sync_error(error: SyncError) -> MailActionCommandError {
    let failure = store_failure(store_error_from_sync(error), 1, MAX_MAIL_ACTION_ATTEMPTS);
    failure.into()
}

fn remove_empty_dirs_best_effort(app_data_dir: &Path, mut current: Option<&Path>) {
    while let Some(directory) = current {
        if directory == app_data_dir {
            break;
        }
        if fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}

pub fn mail_action_plan(command: &MailActionCommand<'_>) -> MailActionPlan {
    let (reversibility, confirmation) = match command.action {
        MailAction::Message { kind, .. } => match kind {
            MessageActionKind::MarkRead { .. }
            | MessageActionKind::Move { .. }
            | MessageActionKind::Archive { .. } => (
                ActionReversibility::Reversible,
                ConfirmationRequirement::NotRequired,
            ),
            MessageActionKind::DeleteToTrash { .. } => (
                ActionReversibility::Reversible,
                ConfirmationRequirement::Required,
            ),
        },
        MailAction::Folder {
            kind: FolderActionKind::ExpungeDeleted,
            ..
        } => (
            ActionReversibility::Irreversible,
            ConfirmationRequirement::Required,
        ),
    };

    MailActionPlan {
        reversibility,
        confirmation,
        max_attempts: MAX_MAIL_ACTION_ATTEMPTS,
    }
}

pub fn execute_mail_action(
    store: &mut impl MailActionStore,
    command: MailActionCommand<'_>,
) -> Result<MailActionReceipt, MailActionFailure> {
    let plan = mail_action_plan(&command);

    validate_command(&command, store)?;

    if plan.confirmation == ConfirmationRequirement::Required
        && command.confirmation != UserConfirmation::Confirmed
    {
        return Err(non_retryable(MailActionError::ConfirmationRequired));
    }

    let retry_policy = RetryPolicy::Retryable {
        max_attempts: plan.max_attempts,
    };

    match command.action {
        MailAction::Message {
            folder_id,
            message_id,
            kind,
        } => execute_message_action(store, command.account_id, folder_id, message_id, kind)
            .map(|(mutation, undo)| MailActionReceipt {
                mutation,
                undo,
                retry_policy,
            })
            .map_err(|error| store_failure(error, command.attempt, plan.max_attempts)),
        MailAction::Folder {
            folder_id,
            kind: FolderActionKind::ExpungeDeleted,
        } => store
            .expunge_deleted(command.account_id, folder_id)
            .map(|removed_count| MailActionReceipt {
                mutation: RemoteMutation::Expunged {
                    folder_id: folder_id.to_string(),
                    removed_count,
                },
                undo: None,
                retry_policy,
            })
            .map_err(|error| store_failure(error, command.attempt, plan.max_attempts)),
    }
}

fn execute_message_action(
    store: &mut impl MailActionStore,
    account_id: &str,
    folder_id: &str,
    message_id: &str,
    kind: MessageActionKind<'_>,
) -> Result<(RemoteMutation, Option<UndoMailAction>), MailActionStoreError> {
    match kind {
        MessageActionKind::MarkRead { read } => {
            store.set_seen(account_id, folder_id, message_id, read)?;
            Ok((
                RemoteMutation::FlagsUpdated { read },
                Some(UndoMailAction::MarkRead {
                    account_id: account_id.to_string(),
                    folder_id: folder_id.to_string(),
                    message_id: message_id.to_string(),
                    read: !read,
                }),
            ))
        }
        MessageActionKind::Move { target_folder_id } => {
            move_with_undo(store, account_id, folder_id, message_id, target_folder_id)
        }
        MessageActionKind::Archive { archive_folder_id } => {
            move_with_undo(store, account_id, folder_id, message_id, archive_folder_id)
        }
        MessageActionKind::DeleteToTrash { trash_folder_id } => {
            move_with_undo(store, account_id, folder_id, message_id, trash_folder_id)
        }
    }
}

fn move_with_undo(
    store: &mut impl MailActionStore,
    account_id: &str,
    from_folder_id: &str,
    message_id: &str,
    to_folder_id: &str,
) -> Result<(RemoteMutation, Option<UndoMailAction>), MailActionStoreError> {
    store.move_message(account_id, from_folder_id, message_id, to_folder_id)?;

    Ok((
        RemoteMutation::Moved {
            from_folder_id: from_folder_id.to_string(),
            to_folder_id: to_folder_id.to_string(),
        },
        Some(UndoMailAction::Move {
            account_id: account_id.to_string(),
            from_folder_id: to_folder_id.to_string(),
            to_folder_id: from_folder_id.to_string(),
            message_id: message_id.to_string(),
        }),
    ))
}

fn validate_command(
    command: &MailActionCommand<'_>,
    store: &impl MailActionStore,
) -> Result<(), MailActionFailure> {
    validate_entity_id("account_id", command.account_id)?;

    if !store.account_exists(command.account_id) {
        return Err(non_retryable(MailActionError::UnknownAccount));
    }

    match command.action {
        MailAction::Message {
            folder_id,
            message_id,
            kind,
        } => {
            validate_entity_id("folder_id", folder_id)?;
            validate_entity_id("message_id", message_id)?;
            validate_target_folder_ids(kind).map_err(non_retryable)?;
            ensure_folder_exists(store, command.account_id, folder_id)?;

            if let Some(target_folder_id) = target_folder_id(kind) {
                ensure_folder_exists(store, command.account_id, target_folder_id)?;
            }

            if !store.message_exists(command.account_id, folder_id, message_id) {
                return Err(non_retryable(MailActionError::UnknownMessage {
                    folder_id: folder_id.to_string(),
                    message_id: message_id.to_string(),
                }));
            }
        }
        MailAction::Folder { folder_id, .. } => {
            validate_entity_id("folder_id", folder_id)?;
            ensure_folder_exists(store, command.account_id, folder_id)?;
        }
    }

    Ok(())
}

fn validate_target_folder_ids(kind: MessageActionKind<'_>) -> Result<(), MailActionError> {
    if let Some(folder_id) = target_folder_id(kind) {
        validate_entity_id("target_folder_id", folder_id)?;
    }

    Ok(())
}

fn target_folder_id(kind: MessageActionKind<'_>) -> Option<&str> {
    match kind {
        MessageActionKind::MarkRead { .. } => None,
        MessageActionKind::Move { target_folder_id } => Some(target_folder_id),
        MessageActionKind::Archive { archive_folder_id } => Some(archive_folder_id),
        MessageActionKind::DeleteToTrash { trash_folder_id } => Some(trash_folder_id),
    }
}

fn ensure_folder_exists(
    store: &impl MailActionStore,
    account_id: &str,
    folder_id: &str,
) -> Result<(), MailActionFailure> {
    if store.folder_exists(account_id, folder_id) {
        Ok(())
    } else {
        Err(non_retryable(MailActionError::UnknownFolder {
            folder_id: folder_id.to_string(),
        }))
    }
}

fn store_failure(error: MailActionStoreError, attempt: u8, max_attempts: u8) -> MailActionFailure {
    match error {
        MailActionStoreError::Transient(message) => MailActionFailure {
            error: MailActionError::TransientServerFailure(message),
            retry: if attempt < max_attempts {
                RetryDecision::RetryNow {
                    next_attempt: attempt.saturating_add(1),
                    max_attempts,
                }
            } else {
                RetryDecision::Exhausted
            },
        },
        MailActionStoreError::Permanent(message) => MailActionFailure {
            error: MailActionError::PermanentServerFailure(message),
            retry: RetryDecision::NotRetryable,
        },
    }
}

fn non_retryable(error: MailActionError) -> MailActionFailure {
    MailActionFailure {
        error,
        retry: RetryDecision::NotRetryable,
    }
}

impl From<IpcValidationError> for MailActionFailure {
    fn from(value: IpcValidationError) -> Self {
        non_retryable(MailActionError::IpcValidation(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    const ACCOUNT_ID: &str = "acct_1";
    const INBOX: &str = "inbox";
    const PROJECTS: &str = "projects";
    const ARCHIVE: &str = "archive";
    const TRASH: &str = "trash";
    const MESSAGE_ID: &str = "msg_1";
    const DELETED_MESSAGE_ID: &str = "msg_deleted";

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeMessage {
        read: bool,
        deleted: bool,
    }

    #[derive(Debug, Default)]
    struct FakeMailboxStore {
        accounts: HashSet<String>,
        folders: HashMap<(String, String), HashMap<String, FakeMessage>>,
        fail_next: Option<MailActionStoreError>,
    }

    impl FakeMailboxStore {
        fn seeded() -> Self {
            let mut store = Self::default();
            store.accounts.insert(ACCOUNT_ID.to_string());
            store.add_folder(INBOX);
            store.add_folder(PROJECTS);
            store.add_folder(ARCHIVE);
            store.add_folder(TRASH);
            store.add_message(INBOX, MESSAGE_ID, false, false);
            store.add_message(TRASH, DELETED_MESSAGE_ID, true, true);
            store
        }

        fn add_folder(&mut self, folder_id: &str) {
            self.folders
                .entry((ACCOUNT_ID.to_string(), folder_id.to_string()))
                .or_default();
        }

        fn add_message(&mut self, folder_id: &str, message_id: &str, read: bool, deleted: bool) {
            self.folders
                .entry((ACCOUNT_ID.to_string(), folder_id.to_string()))
                .or_default()
                .insert(message_id.to_string(), FakeMessage { read, deleted });
        }

        fn message(&self, folder_id: &str, message_id: &str) -> Option<&FakeMessage> {
            self.folders
                .get(&(ACCOUNT_ID.to_string(), folder_id.to_string()))
                .and_then(|folder| folder.get(message_id))
        }

        fn fail_next_transient(&mut self) {
            self.fail_next = Some(MailActionStoreError::Transient("network dropped".into()));
        }

        fn take_failure(&mut self) -> Result<(), MailActionStoreError> {
            if let Some(error) = self.fail_next.take() {
                Err(error)
            } else {
                Ok(())
            }
        }
    }

    impl MailActionStore for FakeMailboxStore {
        fn account_exists(&self, account_id: &str) -> bool {
            self.accounts.contains(account_id)
        }

        fn folder_exists(&self, account_id: &str, folder_id: &str) -> bool {
            self.folders
                .contains_key(&(account_id.to_string(), folder_id.to_string()))
        }

        fn message_exists(&self, account_id: &str, folder_id: &str, message_id: &str) -> bool {
            self.folders
                .get(&(account_id.to_string(), folder_id.to_string()))
                .is_some_and(|folder| folder.contains_key(message_id))
        }

        fn folder_id_for_kind(
            &self,
            account_id: &str,
            kind: MailActionTargetFolderKind,
        ) -> Option<String> {
            if account_id != ACCOUNT_ID {
                return None;
            }

            Some(match kind {
                MailActionTargetFolderKind::Archive => ARCHIVE.to_string(),
                MailActionTargetFolderKind::Trash => TRASH.to_string(),
            })
            .filter(|folder_id| self.folder_exists(account_id, folder_id))
        }

        fn set_seen(
            &mut self,
            account_id: &str,
            folder_id: &str,
            message_id: &str,
            read: bool,
        ) -> Result<(), MailActionStoreError> {
            self.take_failure()?;
            let folder = self
                .folders
                .get_mut(&(account_id.to_string(), folder_id.to_string()))
                .ok_or_else(|| MailActionStoreError::Permanent("folder missing".into()))?;
            let message = folder
                .get_mut(message_id)
                .ok_or_else(|| MailActionStoreError::Permanent("message missing".into()))?;
            message.read = read;
            Ok(())
        }

        fn move_message(
            &mut self,
            account_id: &str,
            from_folder_id: &str,
            message_id: &str,
            to_folder_id: &str,
        ) -> Result<(), MailActionStoreError> {
            self.take_failure()?;
            let message = self
                .folders
                .get_mut(&(account_id.to_string(), from_folder_id.to_string()))
                .and_then(|folder| folder.remove(message_id))
                .ok_or_else(|| MailActionStoreError::Permanent("message missing".into()))?;
            self.folders
                .get_mut(&(account_id.to_string(), to_folder_id.to_string()))
                .ok_or_else(|| MailActionStoreError::Permanent("target folder missing".into()))?
                .insert(message_id.to_string(), message);
            Ok(())
        }

        fn expunge_deleted(
            &mut self,
            account_id: &str,
            folder_id: &str,
        ) -> Result<usize, MailActionStoreError> {
            self.take_failure()?;
            let folder = self
                .folders
                .get_mut(&(account_id.to_string(), folder_id.to_string()))
                .ok_or_else(|| MailActionStoreError::Permanent("folder missing".into()))?;
            let before = folder.len();
            folder.retain(|_, message| !message.deleted);
            Ok(before - folder.len())
        }
    }

    fn command(action: MailAction<'_>) -> MailActionCommand<'_> {
        MailActionCommand {
            account_id: ACCOUNT_ID,
            action,
            confirmation: UserConfirmation::NotConfirmed,
            attempt: 1,
        }
    }

    fn confirmed_command(action: MailAction<'_>) -> MailActionCommand<'_> {
        MailActionCommand {
            confirmation: UserConfirmation::Confirmed,
            ..command(action)
        }
    }

    fn message_action(kind: MessageActionKind<'_>) -> MailAction<'_> {
        MailAction::Message {
            folder_id: INBOX,
            message_id: MESSAGE_ID,
            kind,
        }
    }

    #[test]
    fn mark_read_is_typed_reversible_and_updates_fake_state() {
        let mut store = FakeMailboxStore::seeded();
        let command = command(message_action(MessageActionKind::MarkRead { read: true }));

        assert_eq!(
            mail_action_plan(&command),
            MailActionPlan {
                reversibility: ActionReversibility::Reversible,
                confirmation: ConfirmationRequirement::NotRequired,
                max_attempts: MAX_MAIL_ACTION_ATTEMPTS
            }
        );

        let receipt = execute_mail_action(&mut store, command).unwrap();

        assert!(store.message(INBOX, MESSAGE_ID).unwrap().read);
        assert_eq!(
            receipt.undo,
            Some(UndoMailAction::MarkRead {
                account_id: ACCOUNT_ID.into(),
                folder_id: INBOX.into(),
                message_id: MESSAGE_ID.into(),
                read: false
            })
        );
    }

    #[test]
    fn move_is_typed_reversible_and_moves_fake_state() {
        let mut store = FakeMailboxStore::seeded();

        let receipt = execute_mail_action(
            &mut store,
            command(message_action(MessageActionKind::Move {
                target_folder_id: PROJECTS,
            })),
        )
        .unwrap();

        assert!(store.message(INBOX, MESSAGE_ID).is_none());
        assert!(store.message(PROJECTS, MESSAGE_ID).is_some());
        assert_eq!(
            receipt.undo,
            Some(UndoMailAction::Move {
                account_id: ACCOUNT_ID.into(),
                from_folder_id: PROJECTS.into(),
                to_folder_id: INBOX.into(),
                message_id: MESSAGE_ID.into()
            })
        );
    }

    #[test]
    fn archive_moves_to_archive_and_can_be_undone() {
        let mut store = FakeMailboxStore::seeded();

        let receipt = execute_mail_action(
            &mut store,
            command(message_action(MessageActionKind::Archive {
                archive_folder_id: ARCHIVE,
            })),
        )
        .unwrap();

        assert!(store.message(INBOX, MESSAGE_ID).is_none());
        assert!(store.message(ARCHIVE, MESSAGE_ID).is_some());
        assert_eq!(
            receipt.mutation,
            RemoteMutation::Moved {
                from_folder_id: INBOX.into(),
                to_folder_id: ARCHIVE.into()
            }
        );
        assert!(receipt.undo.is_some());
    }

    #[test]
    fn delete_to_trash_requires_confirmation_and_is_reversible() {
        let mut store = FakeMailboxStore::seeded();
        let delete = message_action(MessageActionKind::DeleteToTrash {
            trash_folder_id: TRASH,
        });

        assert_eq!(
            execute_mail_action(&mut store, command(delete)),
            Err(MailActionFailure {
                error: MailActionError::ConfirmationRequired,
                retry: RetryDecision::NotRetryable
            })
        );

        let receipt = execute_mail_action(&mut store, confirmed_command(delete)).unwrap();

        assert!(store.message(INBOX, MESSAGE_ID).is_none());
        assert!(store.message(TRASH, MESSAGE_ID).is_some());
        assert_eq!(
            receipt.undo,
            Some(UndoMailAction::Move {
                account_id: ACCOUNT_ID.into(),
                from_folder_id: TRASH.into(),
                to_folder_id: INBOX.into(),
                message_id: MESSAGE_ID.into()
            })
        );
    }

    #[test]
    fn expunge_requires_confirmation_and_is_irreversible() {
        let mut store = FakeMailboxStore::seeded();
        let expunge = MailAction::Folder {
            folder_id: TRASH,
            kind: FolderActionKind::ExpungeDeleted,
        };

        assert_eq!(
            mail_action_plan(&command(expunge)),
            MailActionPlan {
                reversibility: ActionReversibility::Irreversible,
                confirmation: ConfirmationRequirement::Required,
                max_attempts: MAX_MAIL_ACTION_ATTEMPTS
            }
        );
        assert_eq!(
            execute_mail_action(&mut store, command(expunge)),
            Err(MailActionFailure {
                error: MailActionError::ConfirmationRequired,
                retry: RetryDecision::NotRetryable
            })
        );

        let receipt = execute_mail_action(&mut store, confirmed_command(expunge)).unwrap();

        assert!(store.message(TRASH, DELETED_MESSAGE_ID).is_none());
        assert_eq!(
            receipt,
            MailActionReceipt {
                mutation: RemoteMutation::Expunged {
                    folder_id: TRASH.into(),
                    removed_count: 1
                },
                undo: None,
                retry_policy: RetryPolicy::Retryable {
                    max_attempts: MAX_MAIL_ACTION_ATTEMPTS
                }
            }
        );
    }

    #[test]
    fn transient_server_failure_is_retryable_and_does_not_mutate_state() {
        let mut store = FakeMailboxStore::seeded();
        store.fail_next_transient();

        let move_command = command(message_action(MessageActionKind::Move {
            target_folder_id: PROJECTS,
        }));

        assert_eq!(
            execute_mail_action(&mut store, move_command),
            Err(MailActionFailure {
                error: MailActionError::TransientServerFailure("network dropped".into()),
                retry: RetryDecision::RetryNow {
                    next_attempt: 2,
                    max_attempts: MAX_MAIL_ACTION_ATTEMPTS
                }
            })
        );
        assert!(store.message(INBOX, MESSAGE_ID).is_some());
        assert!(store.message(PROJECTS, MESSAGE_ID).is_none());

        let receipt = execute_mail_action(
            &mut store,
            MailActionCommand {
                attempt: 2,
                ..move_command
            },
        )
        .unwrap();

        assert!(receipt.undo.is_some());
        assert!(store.message(PROJECTS, MESSAGE_ID).is_some());
    }

    #[test]
    fn validation_rejects_unknown_and_invalid_targets_before_mutation() {
        let mut store = FakeMailboxStore::seeded();
        let before = store.message(INBOX, MESSAGE_ID).cloned();

        assert_eq!(
            execute_mail_action(
                &mut store,
                command(MailAction::Message {
                    folder_id: "missing",
                    message_id: MESSAGE_ID,
                    kind: MessageActionKind::MarkRead { read: true }
                })
            ),
            Err(MailActionFailure {
                error: MailActionError::UnknownFolder {
                    folder_id: "missing".into()
                },
                retry: RetryDecision::NotRetryable
            })
        );
        assert_eq!(
            execute_mail_action(
                &mut store,
                command(message_action(MessageActionKind::Move {
                    target_folder_id: "..\\trash",
                }))
            ),
            Err(MailActionFailure {
                error: MailActionError::IpcValidation(IpcValidationError::InvalidId {
                    field: "target_folder_id",
                    reason: "only ASCII letters, digits, hyphen, and underscore are allowed"
                }),
                retry: RetryDecision::NotRetryable
            })
        );
        assert_eq!(store.message(INBOX, MESSAGE_ID), before.as_ref());
    }

    #[test]
    fn command_request_resolves_special_folders_and_returns_undo_metadata() {
        let mut store = FakeMailboxStore::seeded();

        let archive = perform_mail_action_with_store(
            &mut store,
            PerformMailActionRequest {
                account_id: ACCOUNT_ID.into(),
                folder_id: INBOX.into(),
                message_id: Some(MESSAGE_ID.into()),
                action: PerformMailActionKind::Archive,
                confirmed: false,
                user_requested: true,
                attempt: None,
            },
        )
        .expect("archive command succeeds");

        assert!(store.message(ARCHIVE, MESSAGE_ID).is_some());
        assert_eq!(
            archive.mutation,
            MailActionMutationView::Moved {
                from_folder_id: INBOX.into(),
                to_folder_id: ARCHIVE.into(),
            }
        );
        assert_eq!(
            archive.undo,
            Some(MailActionUndoView {
                account_id: ACCOUNT_ID.into(),
                folder_id: ARCHIVE.into(),
                message_id: MESSAGE_ID.into(),
                action: PerformMailActionKind::Move {
                    target_folder_id: INBOX.into(),
                },
            })
        );

        let undo = archive.undo.expect("archive exposes undo");
        perform_mail_action_with_store(
            &mut store,
            PerformMailActionRequest {
                account_id: undo.account_id,
                folder_id: undo.folder_id,
                message_id: Some(undo.message_id),
                action: undo.action,
                confirmed: false,
                user_requested: true,
                attempt: None,
            },
        )
        .expect("undo command succeeds");
        assert!(store.message(INBOX, MESSAGE_ID).is_some());
    }

    #[test]
    fn command_request_requires_confirmation_for_delete_and_expunge() {
        let mut store = FakeMailboxStore::seeded();

        let delete = PerformMailActionRequest {
            account_id: ACCOUNT_ID.into(),
            folder_id: INBOX.into(),
            message_id: Some(MESSAGE_ID.into()),
            action: PerformMailActionKind::DeleteToTrash,
            confirmed: false,
            user_requested: true,
            attempt: None,
        };
        let denial = perform_mail_action_with_store(&mut store, delete.clone())
            .expect_err("delete requires confirmation");
        assert_eq!(denial.reason, "mail action requires confirmation");

        let receipt = perform_mail_action_with_store(
            &mut store,
            PerformMailActionRequest {
                confirmed: true,
                ..delete
            },
        )
        .expect("confirmed delete moves to trash");
        assert_eq!(
            receipt.mutation,
            MailActionMutationView::Moved {
                from_folder_id: INBOX.into(),
                to_folder_id: TRASH.into(),
            }
        );
        assert!(store.message(TRASH, MESSAGE_ID).is_some());

        let expunge = perform_mail_action_with_store(
            &mut store,
            PerformMailActionRequest {
                account_id: ACCOUNT_ID.into(),
                folder_id: TRASH.into(),
                message_id: None,
                action: PerformMailActionKind::ExpungeDeleted,
                confirmed: false,
                user_requested: true,
                attempt: None,
            },
        )
        .expect_err("expunge requires confirmation");
        assert_eq!(expunge.reason, "mail action requires confirmation");
    }

    #[test]
    fn command_request_exposes_retry_for_transient_failures() {
        let mut store = FakeMailboxStore::seeded();
        store.fail_next_transient();

        let error = perform_mail_action_with_store(
            &mut store,
            PerformMailActionRequest {
                account_id: ACCOUNT_ID.into(),
                folder_id: INBOX.into(),
                message_id: Some(MESSAGE_ID.into()),
                action: PerformMailActionKind::Move {
                    target_folder_id: PROJECTS.into(),
                },
                confirmed: false,
                user_requested: true,
                attempt: Some(1),
            },
        )
        .expect_err("transient move failure surfaces retry metadata");

        assert_eq!(error.reason, "mail server operation can retry");
        assert_eq!(
            error.retry,
            MailActionRetryDecisionView::RetryNow {
                next_attempt: 2,
                max_attempts: MAX_MAIL_ACTION_ATTEMPTS,
            }
        );
        assert!(store.message(INBOX, MESSAGE_ID).is_some());
    }
}
