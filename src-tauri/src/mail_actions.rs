use std::{error::Error, fmt};

use crate::ipc_validation::{validate_entity_id, IpcValidationError};

pub const MAX_MAIL_ACTION_ATTEMPTS: u8 = 3;

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
}
