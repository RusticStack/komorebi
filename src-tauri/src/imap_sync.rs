use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use crate::mime_normalization::{
    AttachmentDisposition, AttachmentPreviewKind, ParsedAttachmentRecord,
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFolderKind {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Trash,
    Junk,
    Custom,
    All,
    Important,
}

impl RemoteFolderKind {
    fn as_schema_value(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::Junk => "junk",
            Self::Custom => "custom",
            Self::All => "all",
            Self::Important => "important",
        }
    }

    fn from_special_use(special_use: RemoteSpecialUse) -> Self {
        match special_use {
            RemoteSpecialUse::Inbox => Self::Inbox,
            RemoteSpecialUse::Sent => Self::Sent,
            RemoteSpecialUse::Drafts => Self::Drafts,
            RemoteSpecialUse::Archive => Self::Archive,
            RemoteSpecialUse::Trash => Self::Trash,
            RemoteSpecialUse::Junk => Self::Junk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSpecialUse {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Trash,
    Junk,
}

impl RemoteSpecialUse {
    fn as_schema_value(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::Junk => "junk",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    pub remote_id: String,
    pub name: String,
    pub display_name: String,
    pub kind: RemoteFolderKind,
    pub special_use: Option<RemoteSpecialUse>,
    pub delimiter: Option<String>,
    pub attributes: BTreeSet<String>,
}

impl RemoteFolder {
    pub fn new(
        remote_id: impl Into<String>,
        name: impl Into<String>,
        display_name: impl Into<String>,
        kind: RemoteFolderKind,
        special_use: Option<RemoteSpecialUse>,
        delimiter: Option<String>,
    ) -> Self {
        Self {
            remote_id: remote_id.into(),
            name: name.into(),
            display_name: display_name.into(),
            kind,
            special_use,
            delimiter,
            attributes: BTreeSet::new(),
        }
    }

    pub fn from_imap_attributes<I, S>(
        remote_id: impl Into<String>,
        name: impl Into<String>,
        display_name: Option<String>,
        delimiter: Option<String>,
        attributes: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let remote_id = remote_id.into();
        let name = name.into();
        let display_name =
            display_name.unwrap_or_else(|| display_name_from_folder_name(&name, &delimiter));
        let attributes: BTreeSet<String> = attributes
            .into_iter()
            .filter_map(|attribute| normalize_folder_attribute(attribute.as_ref()))
            .collect();
        let special_use = detect_special_use(&name, &attributes);
        let kind = special_use.map_or_else(
            || detect_folder_kind_from_attributes(&attributes),
            RemoteFolderKind::from_special_use,
        );

        Self {
            remote_id,
            name,
            display_name,
            kind,
            special_use,
            delimiter,
            attributes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolderStatus {
    pub uid_validity: u64,
    pub uid_next: u64,
    pub existing_uids: BTreeSet<u64>,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemoteFlag {
    Seen,
    Flagged,
    Answered,
    Draft,
    Deleted,
    Recent,
    Keyword(String),
}

impl RemoteFlag {
    pub fn keyword(value: impl Into<String>) -> Self {
        Self::Keyword(value.into())
    }

    pub fn from_imap_token(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "*" {
            return None;
        }

        match trimmed.to_ascii_lowercase().as_str() {
            "\\seen" => Some(Self::Seen),
            "\\flagged" => Some(Self::Flagged),
            "\\answered" => Some(Self::Answered),
            "\\draft" => Some(Self::Draft),
            "\\deleted" => Some(Self::Deleted),
            "\\recent" => Some(Self::Recent),
            _ => Some(Self::Keyword(trimmed.to_string())),
        }
    }

    pub fn from_imap_tokens<'a>(values: impl IntoIterator<Item = &'a str>) -> BTreeSet<RemoteFlag> {
        values
            .into_iter()
            .filter_map(Self::from_imap_token)
            .collect()
    }

    pub(crate) fn as_imap_string(&self) -> String {
        match self {
            Self::Seen => "\\Seen".into(),
            Self::Flagged => "\\Flagged".into(),
            Self::Answered => "\\Answered".into(),
            Self::Draft => "\\Draft".into(),
            Self::Deleted => "\\Deleted".into(),
            Self::Recent => "\\Recent".into(),
            Self::Keyword(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMessage {
    pub uid: u64,
    pub uid_validity: u64,
    pub rfc_message_id: Option<String>,
    pub provider_message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub subject: String,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to_addresses_json: String,
    pub cc_addresses_json: String,
    pub bcc_addresses_json: String,
    pub reply_to_addresses_json: String,
    pub references_json: String,
    pub date_received: Option<String>,
    pub internal_date: Option<String>,
    pub flags: BTreeSet<RemoteFlag>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub snippet: Option<String>,
    pub size_bytes: Option<u64>,
    pub has_attachments: bool,
    pub parsed_from_raw_mime: bool,
    pub attachment_records: Vec<ParsedAttachmentRecord>,
    pub mime_parse_warnings_json: String,
}

impl RemoteMessage {
    pub fn fixture(uid_validity: u64, uid: u64, subject: impl Into<String>) -> Self {
        Self {
            uid,
            uid_validity,
            rfc_message_id: Some(format!("<uid-{uid}@fixture.komorebi>")),
            provider_message_id: None,
            in_reply_to: None,
            subject: subject.into(),
            sender_name: Some("Fixture Sender".into()),
            sender_email: Some("sender@example.com".into()),
            to_addresses_json: "[]".into(),
            cc_addresses_json: "[]".into(),
            bcc_addresses_json: "[]".into(),
            reply_to_addresses_json: "[]".into(),
            references_json: "[]".into(),
            date_received: Some(format!("2026-05-22T12:{uid:02}:00Z")),
            internal_date: Some(format!("2026-05-22T12:{uid:02}:00Z")),
            flags: BTreeSet::new(),
            body_text: Some(format!("Fixture body for UID {uid}")),
            body_html: None,
            snippet: Some(format!("Fixture body for UID {uid}")),
            size_bytes: Some(128),
            has_attachments: false,
            parsed_from_raw_mime: false,
            attachment_records: Vec::new(),
            mime_parse_warnings_json: "[]".into(),
        }
    }
}

pub(crate) fn stable_imap_message_id(
    account_id: &str,
    folder_id: &str,
    uid_validity: u64,
    uid: u64,
) -> String {
    generated_id(
        "msg",
        &[
            account_id,
            folder_id,
            &uid_validity.to_string(),
            &uid.to_string(),
        ],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFlagUpdate {
    pub uid: u64,
    pub uid_validity: u64,
    pub flags: BTreeSet<RemoteFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFolderState {
    pub folder_id: String,
    pub uid_validity: Option<u64>,
    pub uid_next: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFolderReport {
    pub folder_id: String,
    pub remote_id: String,
    pub discovered: bool,
    pub uid_validity_reset: bool,
    pub messages_fetched: usize,
    pub flags_updated: usize,
    pub expunged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncAccountReport {
    pub folders_seen: usize,
    pub messages_fetched: usize,
    pub flags_updated: usize,
    pub expunged: usize,
    pub uid_validity_resets: usize,
    pub reconnects: usize,
    pub backoff_delays: Vec<Duration>,
    pub folders: Vec<SyncFolderReport>,
}

impl SyncAccountReport {
    fn merge_attempt(&mut self, attempt: Self) {
        self.folders_seen += attempt.folders_seen;
        self.messages_fetched += attempt.messages_fetched;
        self.flags_updated += attempt.flags_updated;
        self.expunged += attempt.expunged;
        self.uid_validity_resets += attempt.uid_validity_resets;
        self.folders.extend(attempt.folders);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    Cancelled,
    UnknownAccount,
    UnknownFolder,
    InvalidRemoteState { reason: &'static str },
    TransientServer { reason: String },
    PermanentServer { reason: String },
    Cache { reason: String },
}

impl SyncError {
    pub fn transient(reason: impl Into<String>) -> Self {
        Self::TransientServer {
            reason: reason.into(),
        }
    }

    pub fn permanent(reason: impl Into<String>) -> Self {
        Self::PermanentServer {
            reason: reason.into(),
        }
    }

    fn cache(error: impl fmt::Display) -> Self {
        Self::Cache {
            reason: error.to_string(),
        }
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(formatter, "sync was cancelled"),
            Self::UnknownAccount => write!(formatter, "account was not found"),
            Self::UnknownFolder => write!(formatter, "folder was not found"),
            Self::InvalidRemoteState { reason } => {
                write!(formatter, "invalid remote state: {reason}")
            }
            Self::TransientServer { .. } => write!(formatter, "mail server operation can retry"),
            Self::PermanentServer { .. } => write!(formatter, "mail server operation failed"),
            Self::Cache { .. } => write!(formatter, "local cache operation failed"),
        }
    }
}

impl Error for SyncError {}

impl From<rusqlite::Error> for SyncError {
    fn from(value: rusqlite::Error) -> Self {
        Self::cache(value)
    }
}

#[derive(Debug, Clone)]
pub struct SyncCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl Default for SyncCancellationToken {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl SyncCancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn ensure_not_cancelled(&self) -> Result<(), SyncError> {
        if self.is_cancelled() {
            return Err(SyncError::Cancelled);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffPolicy {
    pub max_attempts: u8,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(5),
            multiplier: 2,
        }
    }
}

impl BackoffPolicy {
    fn normalized(self) -> Self {
        Self {
            max_attempts: self.max_attempts.max(1),
            initial_delay: self.initial_delay,
            max_delay: self.max_delay.max(self.initial_delay),
            multiplier: self.multiplier.max(1),
        }
    }

    pub fn delay_for_retry(self, retry_index: u8) -> Duration {
        let policy = self.normalized();
        let mut delay = policy.initial_delay;
        for _ in 0..retry_index {
            delay = delay.saturating_mul(policy.multiplier);
            if delay >= policy.max_delay {
                return policy.max_delay;
            }
        }

        delay.min(policy.max_delay)
    }
}

pub trait BackoffWaiter {
    fn wait(
        &mut self,
        delay: Duration,
        cancellation: &SyncCancellationToken,
    ) -> Result<(), SyncError>;
}

#[derive(Debug, Default)]
pub struct NoopBackoffWaiter;

impl BackoffWaiter for NoopBackoffWaiter {
    fn wait(
        &mut self,
        _delay: Duration,
        cancellation: &SyncCancellationToken,
    ) -> Result<(), SyncError> {
        cancellation.ensure_not_cancelled()
    }
}

#[derive(Debug, Default)]
pub struct ThreadBackoffWaiter;

impl BackoffWaiter for ThreadBackoffWaiter {
    fn wait(
        &mut self,
        delay: Duration,
        cancellation: &SyncCancellationToken,
    ) -> Result<(), SyncError> {
        let slice = Duration::from_millis(50);
        let mut remaining = delay;
        while remaining > Duration::ZERO {
            cancellation.ensure_not_cancelled()?;
            let sleep_for = remaining.min(slice);
            thread::sleep(sleep_for);
            remaining = remaining.saturating_sub(sleep_for);
        }

        cancellation.ensure_not_cancelled()
    }
}

pub trait ImapSyncServer {
    fn list_folders(&mut self, account_id: &str) -> Result<Vec<RemoteFolder>, SyncError>;
    fn select_folder(
        &mut self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<RemoteFolderStatus, SyncError>;
    fn fetch_messages(
        &mut self,
        account_id: &str,
        remote_id: &str,
        folder_id: &str,
        uid_validity: u64,
        uids: &[u64],
    ) -> Result<Vec<RemoteMessage>, SyncError>;
    fn fetch_flags(
        &mut self,
        account_id: &str,
        remote_id: &str,
        uid_validity: u64,
        uids: &[u64],
    ) -> Result<Vec<RemoteFlagUpdate>, SyncError>;
    fn reconnect(&mut self, account_id: &str) -> Result<(), SyncError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleCapability {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleWaitOutcome {
    MailboxChanged,
    Timeout,
    IdleUnavailable,
}

pub trait ImapIdleServer: ImapSyncServer {
    fn idle_capability(
        &mut self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<IdleCapability, SyncError>;
    fn idle_wait(
        &mut self,
        account_id: &str,
        remote_id: &str,
        timeout: Duration,
        cancellation: &SyncCancellationToken,
    ) -> Result<IdleWaitOutcome, SyncError>;
}

pub trait SyncCache {
    fn account_exists(&self, account_id: &str) -> Result<bool, SyncError>;
    fn cached_folder_state(
        &self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<Option<CachedFolderState>, SyncError>;
    fn upsert_folder(
        &mut self,
        account_id: &str,
        folder: &RemoteFolder,
        status: &RemoteFolderStatus,
        sync_state: FolderSyncState,
    ) -> Result<String, SyncError>;
    fn set_folder_sync_state(
        &mut self,
        account_id: &str,
        folder_id: &str,
        sync_state: FolderSyncState,
    ) -> Result<(), SyncError>;
    fn clear_folder_messages(&mut self, account_id: &str, folder_id: &str)
        -> Result<(), SyncError>;
    fn message_uids(
        &self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u64,
    ) -> Result<BTreeSet<u64>, SyncError>;
    fn upsert_message(
        &mut self,
        account_id: &str,
        folder_id: &str,
        message: &RemoteMessage,
    ) -> Result<(), SyncError>;
    fn update_message_flags(
        &mut self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u64,
        uid: u64,
        flags: &BTreeSet<RemoteFlag>,
    ) -> Result<bool, SyncError>;
    fn delete_message_by_uid(
        &mut self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u64,
        uid: u64,
    ) -> Result<bool, SyncError>;
    fn finish_folder_sync(
        &mut self,
        account_id: &str,
        folder_id: &str,
        status: &RemoteFolderStatus,
    ) -> Result<(), SyncError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderSyncState {
    Idle,
    Syncing,
    Error,
    Paused,
}

impl FolderSyncState {
    fn as_schema_value(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Syncing => "syncing",
            Self::Error => "error",
            Self::Paused => "paused",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImapSyncEngine {
    pub backoff: BackoffPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderWorkerConfig<'a> {
    pub account_id: &'a str,
    pub remote_id: &'a str,
    pub idle_timeout: Duration,
    pub poll_interval: Duration,
    pub max_cycles: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderWorkerStopReason {
    MaxCyclesReached,
    Cancelled,
    AccountRemoved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderWorkerReport {
    pub stop_reason: FolderWorkerStopReason,
    pub cycles_completed: usize,
    pub sync_runs: usize,
    pub idle_waits: usize,
    pub idle_change_refreshes: usize,
    pub idle_timeout_refreshes: usize,
    pub poll_refreshes: usize,
    pub idle_fallbacks: usize,
    pub reconnects: usize,
    pub backoff_delays: Vec<Duration>,
    pub sync_reports: Vec<SyncFolderReport>,
}

impl FolderWorkerReport {
    fn new() -> Self {
        Self {
            stop_reason: FolderWorkerStopReason::MaxCyclesReached,
            cycles_completed: 0,
            sync_runs: 0,
            idle_waits: 0,
            idle_change_refreshes: 0,
            idle_timeout_refreshes: 0,
            poll_refreshes: 0,
            idle_fallbacks: 0,
            reconnects: 0,
            backoff_delays: Vec::new(),
            sync_reports: Vec::new(),
        }
    }

    fn stopped(mut self, reason: FolderWorkerStopReason) -> Self {
        self.stop_reason = reason;
        self
    }
}

impl ImapSyncEngine {
    pub fn new(backoff: BackoffPolicy) -> Self {
        Self { backoff }
    }

    pub fn sync_account<S, C, W>(
        &self,
        server: &mut S,
        cache: &mut C,
        account_id: &str,
        cancellation: &SyncCancellationToken,
        waiter: &mut W,
    ) -> Result<SyncAccountReport, SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
        W: BackoffWaiter,
    {
        cancellation.ensure_not_cancelled()?;
        if !cache.account_exists(account_id)? {
            return Err(SyncError::UnknownAccount);
        }

        let policy = self.backoff.normalized();
        let mut report = SyncAccountReport::default();

        for attempt_index in 0..policy.max_attempts {
            cancellation.ensure_not_cancelled()?;
            match self.sync_once(server, cache, account_id, cancellation) {
                Ok(attempt_report) => {
                    report.merge_attempt(attempt_report);
                    return Ok(report);
                }
                Err(SyncError::TransientServer { reason })
                    if attempt_index + 1 < policy.max_attempts =>
                {
                    let delay = policy.delay_for_retry(attempt_index);
                    report.reconnects += 1;
                    report.backoff_delays.push(delay);
                    server.reconnect(account_id).map_err(|error| match error {
                        SyncError::TransientServer {
                            reason: reconnect_reason,
                        } => SyncError::TransientServer {
                            reason: format!("{reason}; reconnect failed: {reconnect_reason}"),
                        },
                        other => other,
                    })?;
                    waiter.wait(delay, cancellation)?;
                }
                Err(error) => return Err(error),
            }
        }

        Err(SyncError::TransientServer {
            reason: "sync retry budget exhausted".into(),
        })
    }

    pub fn sync_folder_by_remote_id<S, C, W>(
        &self,
        server: &mut S,
        cache: &mut C,
        account_id: &str,
        remote_id: &str,
        cancellation: &SyncCancellationToken,
        waiter: &mut W,
    ) -> Result<SyncFolderReport, SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
        W: BackoffWaiter,
    {
        cancellation.ensure_not_cancelled()?;
        if !cache.account_exists(account_id)? {
            return Err(SyncError::UnknownAccount);
        }

        let policy = self.backoff.normalized();
        for attempt_index in 0..policy.max_attempts {
            cancellation.ensure_not_cancelled()?;
            match self.sync_folder_once_by_remote_id(
                server,
                cache,
                account_id,
                remote_id,
                cancellation,
            ) {
                Ok(folder_report) => return Ok(folder_report),
                Err(SyncError::TransientServer { reason })
                    if attempt_index + 1 < policy.max_attempts =>
                {
                    server.reconnect(account_id).map_err(|error| match error {
                        SyncError::TransientServer {
                            reason: reconnect_reason,
                        } => SyncError::TransientServer {
                            reason: format!("{reason}; reconnect failed: {reconnect_reason}"),
                        },
                        other => other,
                    })?;
                    waiter.wait(policy.delay_for_retry(attempt_index), cancellation)?;
                }
                Err(error) => return Err(error),
            }
        }

        Err(SyncError::TransientServer {
            reason: "folder sync retry budget exhausted".into(),
        })
    }

    pub fn run_folder_worker<S, C, W>(
        &self,
        server: &mut S,
        cache: &mut C,
        config: FolderWorkerConfig<'_>,
        cancellation: &SyncCancellationToken,
        waiter: &mut W,
    ) -> Result<FolderWorkerReport, SyncError>
    where
        S: ImapIdleServer,
        C: SyncCache,
        W: BackoffWaiter,
    {
        validate_folder_worker_config(config)?;
        let mut report = FolderWorkerReport::new();
        let mut idle_capability = None;

        loop {
            if let Some(max_cycles) = config.max_cycles {
                if report.cycles_completed >= max_cycles {
                    return Ok(report.stopped(FolderWorkerStopReason::MaxCyclesReached));
                }
            }

            if cancellation.is_cancelled() {
                return Ok(report.stopped(FolderWorkerStopReason::Cancelled));
            }

            if !cache.account_exists(config.account_id)? {
                return Ok(report.stopped(FolderWorkerStopReason::AccountRemoved));
            }

            let can_idle = match idle_capability {
                Some(capability) => capability == IdleCapability::Available,
                None => match server.idle_capability(config.account_id, config.remote_id) {
                    Ok(IdleCapability::Available) => {
                        idle_capability = Some(IdleCapability::Available);
                        true
                    }
                    Ok(IdleCapability::Unavailable) => {
                        idle_capability = Some(IdleCapability::Unavailable);
                        false
                    }
                    Err(SyncError::Cancelled) => {
                        return Ok(report.stopped(FolderWorkerStopReason::Cancelled));
                    }
                    Err(SyncError::TransientServer { reason }) => {
                        self.reconnect_after_worker_drop(
                            server,
                            config.account_id,
                            reason,
                            cancellation,
                            waiter,
                            &mut report,
                        )?;
                        idle_capability = None;
                        continue;
                    }
                    Err(error) => return Err(error),
                },
            };

            if can_idle {
                report.idle_waits += 1;
                match server.idle_wait(
                    config.account_id,
                    config.remote_id,
                    config.idle_timeout,
                    cancellation,
                ) {
                    Ok(IdleWaitOutcome::MailboxChanged) => {
                        report.idle_change_refreshes += 1;
                        if let Err(error) = self.worker_sync_folder(
                            server,
                            cache,
                            config,
                            cancellation,
                            waiter,
                            &mut report,
                        ) {
                            return stop_worker_or_error(report, error);
                        }
                        report.cycles_completed += 1;
                    }
                    Ok(IdleWaitOutcome::Timeout) => {
                        report.idle_timeout_refreshes += 1;
                        if let Err(error) = self.worker_sync_folder(
                            server,
                            cache,
                            config,
                            cancellation,
                            waiter,
                            &mut report,
                        ) {
                            return stop_worker_or_error(report, error);
                        }
                        report.cycles_completed += 1;
                    }
                    Ok(IdleWaitOutcome::IdleUnavailable) => {
                        report.idle_fallbacks += 1;
                        idle_capability = Some(IdleCapability::Unavailable);
                        if let Err(error) = self.worker_poll_then_sync(
                            server,
                            cache,
                            config,
                            cancellation,
                            waiter,
                            &mut report,
                        ) {
                            return stop_worker_or_error(report, error);
                        }
                    }
                    Err(SyncError::Cancelled) => {
                        return Ok(report.stopped(FolderWorkerStopReason::Cancelled));
                    }
                    Err(SyncError::TransientServer { reason }) => {
                        self.reconnect_after_worker_drop(
                            server,
                            config.account_id,
                            reason,
                            cancellation,
                            waiter,
                            &mut report,
                        )?;
                        idle_capability = None;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                report.idle_fallbacks += 1;
                if let Err(error) = self.worker_poll_then_sync(
                    server,
                    cache,
                    config,
                    cancellation,
                    waiter,
                    &mut report,
                ) {
                    return stop_worker_or_error(report, error);
                }
            }
        }
    }

    fn sync_once<S, C>(
        &self,
        server: &mut S,
        cache: &mut C,
        account_id: &str,
        cancellation: &SyncCancellationToken,
    ) -> Result<SyncAccountReport, SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
    {
        cancellation.ensure_not_cancelled()?;
        let folders = server.list_folders(account_id)?;
        let mut account_report = SyncAccountReport::default();

        for folder in folders {
            cancellation.ensure_not_cancelled()?;
            let folder_report =
                self.sync_folder(server, cache, account_id, &folder, cancellation)?;
            account_report.folders_seen += 1;
            account_report.messages_fetched += folder_report.messages_fetched;
            account_report.flags_updated += folder_report.flags_updated;
            account_report.expunged += folder_report.expunged;
            if folder_report.uid_validity_reset {
                account_report.uid_validity_resets += 1;
            }
            account_report.folders.push(folder_report);
        }

        Ok(account_report)
    }

    fn sync_folder_once_by_remote_id<S, C>(
        &self,
        server: &mut S,
        cache: &mut C,
        account_id: &str,
        remote_id: &str,
        cancellation: &SyncCancellationToken,
    ) -> Result<SyncFolderReport, SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
    {
        let folder = server
            .list_folders(account_id)?
            .into_iter()
            .find(|folder| folder.remote_id == remote_id)
            .ok_or(SyncError::UnknownFolder)?;

        self.sync_folder(server, cache, account_id, &folder, cancellation)
    }

    fn sync_folder<S, C>(
        &self,
        server: &mut S,
        cache: &mut C,
        account_id: &str,
        folder: &RemoteFolder,
        cancellation: &SyncCancellationToken,
    ) -> Result<SyncFolderReport, SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
    {
        validate_remote_folder(folder)?;
        let cached_before = cache.cached_folder_state(account_id, &folder.remote_id)?;
        let status = server.select_folder(account_id, &folder.remote_id)?;
        validate_remote_status(&status)?;

        let folder_id =
            cache.upsert_folder(account_id, folder, &status, FolderSyncState::Syncing)?;
        let discovered = cached_before.is_none();
        let uid_validity_reset = cached_before
            .as_ref()
            .and_then(|state| state.uid_validity)
            .is_some_and(|cached_uid_validity| cached_uid_validity != status.uid_validity);

        if uid_validity_reset {
            cache.clear_folder_messages(account_id, &folder_id)?;
        }

        let known_uids = if uid_validity_reset {
            BTreeSet::new()
        } else {
            cache.message_uids(account_id, &folder_id, status.uid_validity)?
        };

        let expunged_uids: Vec<u64> = known_uids
            .difference(&status.existing_uids)
            .copied()
            .collect();
        let mut expunged = 0;
        for uid in expunged_uids {
            cancellation.ensure_not_cancelled()?;
            if cache.delete_message_by_uid(account_id, &folder_id, status.uid_validity, uid)? {
                expunged += 1;
            }
        }

        let retained_uids: Vec<u64> = known_uids
            .intersection(&status.existing_uids)
            .copied()
            .collect();
        let mut flags_updated = 0;
        if !retained_uids.is_empty() {
            cancellation.ensure_not_cancelled()?;
            let flag_updates = server.fetch_flags(
                account_id,
                &folder.remote_id,
                status.uid_validity,
                &retained_uids,
            )?;
            let flag_updates = validate_flag_updates(&status, &retained_uids, flag_updates)?;
            for update in flag_updates.values() {
                cancellation.ensure_not_cancelled()?;
                if cache.update_message_flags(
                    account_id,
                    &folder_id,
                    status.uid_validity,
                    update.uid,
                    &update.flags,
                )? {
                    flags_updated += 1;
                }
            }
        }

        let new_uids: Vec<u64> = status
            .existing_uids
            .difference(&known_uids)
            .copied()
            .collect();
        let mut messages_fetched = 0;
        if !new_uids.is_empty() {
            cancellation.ensure_not_cancelled()?;
            let messages = server.fetch_messages(
                account_id,
                &folder.remote_id,
                &folder_id,
                status.uid_validity,
                &new_uids,
            )?;
            let messages = validate_messages(&status, &new_uids, messages)?;
            for message in messages.values() {
                cancellation.ensure_not_cancelled()?;
                cache.upsert_message(account_id, &folder_id, message)?;
                messages_fetched += 1;
            }
        }

        cache.finish_folder_sync(account_id, &folder_id, &status)?;

        Ok(SyncFolderReport {
            folder_id,
            remote_id: folder.remote_id.clone(),
            discovered,
            uid_validity_reset,
            messages_fetched,
            flags_updated,
            expunged,
        })
    }

    fn worker_poll_then_sync<S, C, W>(
        &self,
        server: &mut S,
        cache: &mut C,
        config: FolderWorkerConfig<'_>,
        cancellation: &SyncCancellationToken,
        waiter: &mut W,
        report: &mut FolderWorkerReport,
    ) -> Result<(), SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
        W: BackoffWaiter,
    {
        waiter
            .wait(config.poll_interval, cancellation)
            .map_err(|error| match error {
                SyncError::Cancelled => SyncError::Cancelled,
                other => other,
            })?;
        if cancellation.is_cancelled() {
            return Err(SyncError::Cancelled);
        }

        if !cache.account_exists(config.account_id)? {
            return Err(SyncError::UnknownAccount);
        }

        report.poll_refreshes += 1;
        self.worker_sync_folder(server, cache, config, cancellation, waiter, report)?;
        report.cycles_completed += 1;

        Ok(())
    }

    fn worker_sync_folder<S, C, W>(
        &self,
        server: &mut S,
        cache: &mut C,
        config: FolderWorkerConfig<'_>,
        cancellation: &SyncCancellationToken,
        waiter: &mut W,
        report: &mut FolderWorkerReport,
    ) -> Result<(), SyncError>
    where
        S: ImapSyncServer,
        C: SyncCache,
        W: BackoffWaiter,
    {
        match self.sync_folder_by_remote_id(
            server,
            cache,
            config.account_id,
            config.remote_id,
            cancellation,
            waiter,
        ) {
            Ok(sync_report) => {
                report.sync_runs += 1;
                report.sync_reports.push(sync_report);
                Ok(())
            }
            Err(SyncError::Cancelled) => Err(SyncError::Cancelled),
            Err(SyncError::UnknownAccount) => Err(SyncError::UnknownAccount),
            Err(error) => Err(error),
        }
    }

    fn reconnect_after_worker_drop<S, W>(
        &self,
        server: &mut S,
        account_id: &str,
        reason: String,
        cancellation: &SyncCancellationToken,
        waiter: &mut W,
        report: &mut FolderWorkerReport,
    ) -> Result<(), SyncError>
    where
        S: ImapSyncServer,
        W: BackoffWaiter,
    {
        let policy = self.backoff.normalized();
        report.reconnects += 1;
        server.reconnect(account_id).map_err(|error| match error {
            SyncError::TransientServer {
                reason: reconnect_reason,
            } => SyncError::TransientServer {
                reason: format!("{reason}; reconnect failed: {reconnect_reason}"),
            },
            other => other,
        })?;
        let delay = policy.delay_for_retry(0);
        report.backoff_delays.push(delay);
        waiter.wait(delay, cancellation)
    }
}

fn validate_folder_worker_config(config: FolderWorkerConfig<'_>) -> Result<(), SyncError> {
    if config.account_id.trim().is_empty() || config.remote_id.trim().is_empty() {
        return Err(SyncError::InvalidRemoteState {
            reason: "folder worker account and remote folder ids must be non-empty",
        });
    }

    if config.idle_timeout == Duration::ZERO || config.poll_interval == Duration::ZERO {
        return Err(SyncError::InvalidRemoteState {
            reason: "folder worker intervals must be non-zero",
        });
    }

    Ok(())
}

fn stop_worker_or_error(
    report: FolderWorkerReport,
    error: SyncError,
) -> Result<FolderWorkerReport, SyncError> {
    match error {
        SyncError::Cancelled => Ok(report.stopped(FolderWorkerStopReason::Cancelled)),
        SyncError::UnknownAccount => Ok(report.stopped(FolderWorkerStopReason::AccountRemoved)),
        other => Err(other),
    }
}

fn validate_remote_folder(folder: &RemoteFolder) -> Result<(), SyncError> {
    if folder.remote_id.trim().is_empty()
        || folder.name.trim().is_empty()
        || folder.display_name.trim().is_empty()
    {
        return Err(SyncError::InvalidRemoteState {
            reason: "folder identity fields must be non-empty",
        });
    }

    Ok(())
}

fn display_name_from_folder_name(name: &str, delimiter: &Option<String>) -> String {
    delimiter
        .as_deref()
        .filter(|delimiter| !delimiter.is_empty())
        .and_then(|delimiter| name.rsplit(delimiter).next())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(name)
        .trim()
        .to_string()
}

fn normalize_folder_attribute(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(without_slash) = trimmed.strip_prefix('\\') {
        let without_slash = without_slash.trim();
        if without_slash.is_empty() {
            return None;
        }

        return Some(format!(
            "\\{}",
            canonical_folder_attribute_name(without_slash).unwrap_or(without_slash)
        ));
    }

    Some(trimmed.to_string())
}

fn canonical_folder_attribute_name(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "all" => Some("All"),
        "allmail" => Some("AllMail"),
        "archive" => Some("Archive"),
        "drafts" => Some("Drafts"),
        "flagged" => Some("Flagged"),
        "haschildren" => Some("HasChildren"),
        "hasnochildren" => Some("HasNoChildren"),
        "important" => Some("Important"),
        "inbox" => Some("Inbox"),
        "junk" => Some("Junk"),
        "marked" => Some("Marked"),
        "noinferiors" => Some("Noinferiors"),
        "nonexistent" => Some("NonExistent"),
        "noselect" => Some("Noselect"),
        "sent" => Some("Sent"),
        "spam" => Some("Spam"),
        "starred" => Some("Starred"),
        "trash" => Some("Trash"),
        "unmarked" => Some("Unmarked"),
        _ => None,
    }
}

fn detect_special_use(name: &str, attributes: &BTreeSet<String>) -> Option<RemoteSpecialUse> {
    if folder_has_attribute(attributes, &["inbox"]) || name.eq_ignore_ascii_case("INBOX") {
        return Some(RemoteSpecialUse::Inbox);
    }

    if folder_has_attribute(attributes, &["sent"]) {
        return Some(RemoteSpecialUse::Sent);
    }

    if folder_has_attribute(attributes, &["drafts"]) {
        return Some(RemoteSpecialUse::Drafts);
    }

    if folder_has_attribute(attributes, &["trash"]) {
        return Some(RemoteSpecialUse::Trash);
    }

    if folder_has_attribute(attributes, &["junk", "spam"]) {
        return Some(RemoteSpecialUse::Junk);
    }

    if folder_has_attribute(attributes, &["archive"]) {
        return Some(RemoteSpecialUse::Archive);
    }

    None
}

fn detect_folder_kind_from_attributes(attributes: &BTreeSet<String>) -> RemoteFolderKind {
    if folder_has_attribute(attributes, &["all", "allmail"]) {
        RemoteFolderKind::All
    } else if folder_has_attribute(attributes, &["important"]) {
        RemoteFolderKind::Important
    } else {
        RemoteFolderKind::Custom
    }
}

fn folder_has_attribute(attributes: &BTreeSet<String>, wanted: &[&str]) -> bool {
    attributes.iter().any(|attribute| {
        let key = attribute
            .trim()
            .trim_start_matches('\\')
            .to_ascii_lowercase();
        wanted.iter().any(|wanted| key == *wanted)
    })
}

fn validate_remote_status(status: &RemoteFolderStatus) -> Result<(), SyncError> {
    if status.uid_validity == 0 {
        return Err(SyncError::InvalidRemoteState {
            reason: "UIDVALIDITY must be positive",
        });
    }

    if status.uid_next == 0 {
        return Err(SyncError::InvalidRemoteState {
            reason: "UIDNEXT must be positive",
        });
    }

    if status.existing_uids.iter().any(|uid| *uid == 0) {
        return Err(SyncError::InvalidRemoteState {
            reason: "message UIDs must be positive",
        });
    }

    Ok(())
}

fn validate_messages(
    status: &RemoteFolderStatus,
    requested_uids: &[u64],
    messages: Vec<RemoteMessage>,
) -> Result<BTreeMap<u64, RemoteMessage>, SyncError> {
    let requested: BTreeSet<u64> = requested_uids.iter().copied().collect();
    let mut by_uid = BTreeMap::new();
    for message in messages {
        if message.uid_validity != status.uid_validity {
            return Err(SyncError::InvalidRemoteState {
                reason: "fetched message UIDVALIDITY did not match selected folder",
            });
        }

        if !requested.contains(&message.uid) {
            return Err(SyncError::InvalidRemoteState {
                reason: "server returned an unrequested message UID",
            });
        }

        if by_uid.insert(message.uid, message).is_some() {
            return Err(SyncError::InvalidRemoteState {
                reason: "server returned duplicate message UIDs",
            });
        }
    }

    if by_uid.len() != requested.len() {
        return Err(SyncError::InvalidRemoteState {
            reason: "server did not return all requested message UIDs",
        });
    }

    Ok(by_uid)
}

fn validate_flag_updates(
    status: &RemoteFolderStatus,
    requested_uids: &[u64],
    updates: Vec<RemoteFlagUpdate>,
) -> Result<BTreeMap<u64, RemoteFlagUpdate>, SyncError> {
    let requested: BTreeSet<u64> = requested_uids.iter().copied().collect();
    let mut by_uid = BTreeMap::new();
    for update in updates {
        if update.uid_validity != status.uid_validity {
            return Err(SyncError::InvalidRemoteState {
                reason: "flag update UIDVALIDITY did not match selected folder",
            });
        }

        if !requested.contains(&update.uid) {
            return Err(SyncError::InvalidRemoteState {
                reason: "server returned flags for an unrequested UID",
            });
        }

        if by_uid.insert(update.uid, update).is_some() {
            return Err(SyncError::InvalidRemoteState {
                reason: "server returned duplicate flag updates",
            });
        }
    }

    if by_uid.len() != requested.len() {
        return Err(SyncError::InvalidRemoteState {
            reason: "server did not return all requested flag updates",
        });
    }

    Ok(by_uid)
}

pub struct SqliteSyncCache<'connection> {
    connection: &'connection Connection,
}

impl<'connection> SqliteSyncCache<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    fn replace_message_attachments(
        &self,
        account_id: &str,
        folder_id: &str,
        message_id: &str,
        attachments: &[ParsedAttachmentRecord],
    ) -> Result<(), SyncError> {
        for attachment in attachments {
            if attachment.account_id != account_id
                || attachment.folder_id != folder_id
                || attachment.message_id != message_id
            {
                return Err(SyncError::Cache {
                    reason: "parsed attachment ownership does not match parsed message".into(),
                });
            }
        }

        self.connection.execute(
            "DELETE FROM attachments
             WHERE account_id = ?1
               AND folder_id = ?2
               AND message_id = ?3",
            params![account_id, folder_id, message_id],
        )?;

        for attachment in attachments {
            let preview_kind = attachment.preview_kind.map(attachment_preview_schema_value);
            self.connection.execute(
                "INSERT INTO attachments (
                   id,
                   account_id,
                   folder_id,
                   message_id,
                   content_id,
                   disposition,
                   sender_filename,
                   display_filename,
                   canonical_mime,
                   content_type,
                   size_bytes,
                   storage_relative_path,
                   sha256,
                   is_inline,
                   preview_kind
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13, ?14)",
                params![
                    &attachment.id,
                    &attachment.account_id,
                    &attachment.folder_id,
                    &attachment.message_id,
                    &attachment.content_id,
                    attachment_disposition_schema_value(attachment.disposition),
                    &attachment.sender_filename,
                    &attachment.display_filename,
                    attachment
                        .sniffed_content_type
                        .as_deref()
                        .unwrap_or(&attachment.declared_content_type),
                    &attachment.declared_content_type,
                    u64_to_i64(attachment.size_bytes)?,
                    &attachment.sha256,
                    bool_to_i64(attachment.disposition == AttachmentDisposition::Inline),
                    preview_kind
                ],
            )?;
        }

        Ok(())
    }
}

impl SyncCache for SqliteSyncCache<'_> {
    fn account_exists(&self, account_id: &str) -> Result<bool, SyncError> {
        let count = self.connection.query_row(
            "SELECT COUNT(*) FROM accounts WHERE id = ?1 AND removed_at IS NULL",
            [account_id],
            |row| row.get::<_, i64>(0),
        )?;

        Ok(count > 0)
    }

    fn cached_folder_state(
        &self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<Option<CachedFolderState>, SyncError> {
        self.connection
            .query_row(
                "SELECT id, uid_validity, uid_next
                 FROM folders
                 WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, remote_id],
                |row| {
                    Ok(CachedFolderState {
                        folder_id: row.get(0)?,
                        uid_validity: optional_i64_to_u64(row.get::<_, Option<i64>>(1)?),
                        uid_next: optional_i64_to_u64(row.get::<_, Option<i64>>(2)?),
                    })
                },
            )
            .optional()
            .map_err(SyncError::from)
    }

    fn upsert_folder(
        &mut self,
        account_id: &str,
        folder: &RemoteFolder,
        status: &RemoteFolderStatus,
        sync_state: FolderSyncState,
    ) -> Result<String, SyncError> {
        let folder_id = self
            .cached_folder_state(account_id, &folder.remote_id)?
            .map(|state| state.folder_id)
            .unwrap_or_else(|| generated_id("folder", &[account_id, &folder.remote_id]));

        let uid_validity = u64_to_i64(status.uid_validity)?;
        let uid_next = u64_to_i64(status.uid_next)?;
        let unread_count = u32_to_i64(status.unread_count);
        let total_count = u32_to_i64(status.total_count);
        let special_use = folder
            .special_use
            .map(RemoteSpecialUse::as_schema_value)
            .map(str::to_string);
        let attributes_json = folder_attributes_json(&folder.attributes)?;

        self.connection.execute(
            "INSERT INTO folders (
               id,
               account_id,
               remote_id,
               name,
               display_name,
               kind,
               special_use,
               attributes_json,
               uid_validity,
               uid_next,
               delimiter,
               unread_count,
               total_count,
               sync_state
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(account_id, remote_id) DO UPDATE SET
               name = excluded.name,
               display_name = excluded.display_name,
               kind = excluded.kind,
               special_use = excluded.special_use,
               attributes_json = excluded.attributes_json,
               uid_validity = excluded.uid_validity,
               uid_next = excluded.uid_next,
               delimiter = excluded.delimiter,
               unread_count = excluded.unread_count,
               total_count = excluded.total_count,
               sync_state = excluded.sync_state",
            params![
                &folder_id,
                account_id,
                &folder.remote_id,
                &folder.name,
                &folder.display_name,
                folder.kind.as_schema_value(),
                special_use,
                attributes_json,
                uid_validity,
                uid_next,
                &folder.delimiter,
                unread_count,
                total_count,
                sync_state.as_schema_value()
            ],
        )?;

        Ok(folder_id)
    }

    fn set_folder_sync_state(
        &mut self,
        account_id: &str,
        folder_id: &str,
        sync_state: FolderSyncState,
    ) -> Result<(), SyncError> {
        self.connection.execute(
            "UPDATE folders
             SET sync_state = ?3
             WHERE account_id = ?1 AND id = ?2",
            params![account_id, folder_id, sync_state.as_schema_value()],
        )?;

        Ok(())
    }

    fn clear_folder_messages(
        &mut self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<(), SyncError> {
        self.connection.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND folder_id = ?2",
            params![account_id, folder_id],
        )?;

        Ok(())
    }

    fn message_uids(
        &self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u64,
    ) -> Result<BTreeSet<u64>, SyncError> {
        let mut statement = self.connection.prepare(
            "SELECT imap_uid
             FROM messages
             WHERE account_id = ?1
               AND folder_id = ?2
               AND uid_validity = ?3
               AND imap_uid IS NOT NULL
             ORDER BY imap_uid",
        )?;
        let rows = statement.query_map(
            params![account_id, folder_id, u64_to_i64(uid_validity)?],
            |row| row.get::<_, i64>(0),
        )?;

        let mut uids = BTreeSet::new();
        for row in rows {
            uids.insert(i64_to_u64(row?)?);
        }

        Ok(uids)
    }

    fn upsert_message(
        &mut self,
        account_id: &str,
        folder_id: &str,
        message: &RemoteMessage,
    ) -> Result<(), SyncError> {
        let existing_id = self
            .connection
            .query_row(
                "SELECT id
                 FROM messages
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND uid_validity = ?3
                   AND imap_uid = ?4",
                params![
                    account_id,
                    folder_id,
                    u64_to_i64(message.uid_validity)?,
                    u64_to_i64(message.uid)?
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let message_id = existing_id.unwrap_or_else(|| {
            stable_imap_message_id(account_id, folder_id, message.uid_validity, message.uid)
        });

        let flags_json = flags_json(&message.flags)?;
        let flag_state = FlagState::from_flags(&message.flags);
        self.connection.execute(
            "INSERT INTO messages (
               id,
               account_id,
               folder_id,
               imap_uid,
               uid_validity,
               rfc_message_id,
               provider_message_id,
               in_reply_to,
               references_json,
               subject,
               sender_name,
               sender_email,
               to_addresses_json,
               cc_addresses_json,
               bcc_addresses_json,
               reply_to_addresses_json,
               date_received,
               internal_date,
               is_read,
               is_flagged,
               is_answered,
               is_draft,
               is_deleted,
               has_attachments,
               flags_json,
               mime_parse_warnings_json,
               body_text,
               body_html,
               snippet,
               size_bytes,
               sync_state
             )
             VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
               ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
               ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, 'synced'
             )
             ON CONFLICT(id) DO UPDATE SET
               imap_uid = excluded.imap_uid,
               uid_validity = excluded.uid_validity,
               rfc_message_id = excluded.rfc_message_id,
               provider_message_id = excluded.provider_message_id,
               in_reply_to = excluded.in_reply_to,
               references_json = excluded.references_json,
               subject = excluded.subject,
               sender_name = excluded.sender_name,
               sender_email = excluded.sender_email,
               to_addresses_json = excluded.to_addresses_json,
               cc_addresses_json = excluded.cc_addresses_json,
               bcc_addresses_json = excluded.bcc_addresses_json,
               reply_to_addresses_json = excluded.reply_to_addresses_json,
               date_received = excluded.date_received,
               internal_date = excluded.internal_date,
               is_read = excluded.is_read,
               is_flagged = excluded.is_flagged,
               is_answered = excluded.is_answered,
               is_draft = excluded.is_draft,
               is_deleted = excluded.is_deleted,
               has_attachments = excluded.has_attachments,
               flags_json = excluded.flags_json,
               mime_parse_warnings_json = excluded.mime_parse_warnings_json,
               body_text = excluded.body_text,
               body_html = excluded.body_html,
               snippet = excluded.snippet,
               size_bytes = excluded.size_bytes,
               sync_state = 'synced'",
            params![
                &message_id,
                account_id,
                folder_id,
                u64_to_i64(message.uid)?,
                u64_to_i64(message.uid_validity)?,
                &message.rfc_message_id,
                &message.provider_message_id,
                &message.in_reply_to,
                &message.references_json,
                &message.subject,
                &message.sender_name,
                &message.sender_email,
                &message.to_addresses_json,
                &message.cc_addresses_json,
                &message.bcc_addresses_json,
                &message.reply_to_addresses_json,
                &message.date_received,
                &message.internal_date,
                flag_state.is_read,
                flag_state.is_flagged,
                flag_state.is_answered,
                flag_state.is_draft,
                flag_state.is_deleted,
                bool_to_i64(message.has_attachments),
                flags_json,
                &message.mime_parse_warnings_json,
                &message.body_text,
                &message.body_html,
                &message.snippet,
                optional_u64_to_i64(message.size_bytes)?
            ],
        )?;

        if message.parsed_from_raw_mime {
            self.replace_message_attachments(
                account_id,
                folder_id,
                &message_id,
                &message.attachment_records,
            )?;
        }

        Ok(())
    }

    fn update_message_flags(
        &mut self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u64,
        uid: u64,
        flags: &BTreeSet<RemoteFlag>,
    ) -> Result<bool, SyncError> {
        let existing = self
            .connection
            .query_row(
                "SELECT is_read, is_flagged, is_answered, is_draft, is_deleted, flags_json
                 FROM messages
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND uid_validity = ?3
                   AND imap_uid = ?4",
                params![
                    account_id,
                    folder_id,
                    u64_to_i64(uid_validity)?,
                    u64_to_i64(uid)?
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;

        let Some(existing) = existing else {
            return Ok(false);
        };

        let flags_json = flags_json(flags)?;
        let flag_state = FlagState::from_flags(flags);
        let changed = existing
            != (
                flag_state.is_read,
                flag_state.is_flagged,
                flag_state.is_answered,
                flag_state.is_draft,
                flag_state.is_deleted,
                flags_json.clone(),
            );

        if changed {
            self.connection.execute(
                "UPDATE messages
                 SET is_read = ?5,
                     is_flagged = ?6,
                     is_answered = ?7,
                     is_draft = ?8,
                     is_deleted = ?9,
                     flags_json = ?10,
                     sync_state = 'synced'
                 WHERE account_id = ?1
                   AND folder_id = ?2
                   AND uid_validity = ?3
                   AND imap_uid = ?4",
                params![
                    account_id,
                    folder_id,
                    u64_to_i64(uid_validity)?,
                    u64_to_i64(uid)?,
                    flag_state.is_read,
                    flag_state.is_flagged,
                    flag_state.is_answered,
                    flag_state.is_draft,
                    flag_state.is_deleted,
                    flags_json
                ],
            )?;
        }

        Ok(changed)
    }

    fn delete_message_by_uid(
        &mut self,
        account_id: &str,
        folder_id: &str,
        uid_validity: u64,
        uid: u64,
    ) -> Result<bool, SyncError> {
        let changed = self.connection.execute(
            "DELETE FROM messages
             WHERE account_id = ?1
               AND folder_id = ?2
               AND uid_validity = ?3
               AND imap_uid = ?4",
            params![
                account_id,
                folder_id,
                u64_to_i64(uid_validity)?,
                u64_to_i64(uid)?
            ],
        )?;

        Ok(changed > 0)
    }

    fn finish_folder_sync(
        &mut self,
        account_id: &str,
        folder_id: &str,
        status: &RemoteFolderStatus,
    ) -> Result<(), SyncError> {
        self.connection.execute(
            "UPDATE folders
             SET uid_validity = ?3,
                 uid_next = ?4,
                 unread_count = ?5,
                 total_count = ?6,
                 sync_state = 'idle',
                 last_synced_at = CURRENT_TIMESTAMP
             WHERE account_id = ?1 AND id = ?2",
            params![
                account_id,
                folder_id,
                u64_to_i64(status.uid_validity)?,
                u64_to_i64(status.uid_next)?,
                u32_to_i64(status.unread_count),
                u32_to_i64(status.total_count)
            ],
        )?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FlagState {
    is_read: i64,
    is_flagged: i64,
    is_answered: i64,
    is_draft: i64,
    is_deleted: i64,
}

impl FlagState {
    fn from_flags(flags: &BTreeSet<RemoteFlag>) -> Self {
        Self {
            is_read: bool_to_i64(flags.contains(&RemoteFlag::Seen)),
            is_flagged: bool_to_i64(flags.contains(&RemoteFlag::Flagged)),
            is_answered: bool_to_i64(flags.contains(&RemoteFlag::Answered)),
            is_draft: bool_to_i64(flags.contains(&RemoteFlag::Draft)),
            is_deleted: bool_to_i64(flags.contains(&RemoteFlag::Deleted)),
        }
    }
}

fn generated_id(prefix: &str, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    let hash = digest.finalize();
    let mut output = String::with_capacity(prefix.len() + 1 + 32);
    output.push_str(prefix);
    output.push('_');
    for byte in hash.iter().take(16) {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn flags_json(flags: &BTreeSet<RemoteFlag>) -> Result<String, SyncError> {
    let values: Vec<String> = flags.iter().map(RemoteFlag::as_imap_string).collect();
    serde_json::to_string(&values).map_err(SyncError::cache)
}

fn folder_attributes_json(attributes: &BTreeSet<String>) -> Result<String, SyncError> {
    let values: Vec<&str> = attributes.iter().map(String::as_str).collect();
    serde_json::to_string(&values).map_err(SyncError::cache)
}

fn attachment_disposition_schema_value(disposition: AttachmentDisposition) -> &'static str {
    match disposition {
        AttachmentDisposition::Attachment => "attachment",
        AttachmentDisposition::Inline => "inline",
    }
}

fn attachment_preview_schema_value(preview_kind: AttachmentPreviewKind) -> &'static str {
    match preview_kind {
        AttachmentPreviewKind::PlainText => "plain_text",
        AttachmentPreviewKind::RasterImage => "raster_image",
    }
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

fn u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

fn u64_to_i64(value: u64) -> Result<i64, SyncError> {
    i64::try_from(value).map_err(|_| SyncError::InvalidRemoteState {
        reason: "remote integer exceeds SQLite storage range",
    })
}

fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>, SyncError> {
    value.map(u64_to_i64).transpose()
}

fn i64_to_u64(value: i64) -> Result<u64, SyncError> {
    u64::try_from(value).map_err(|_| SyncError::InvalidRemoteState {
        reason: "cached UID was negative",
    })
}

fn optional_i64_to_u64(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{apply_migrations, search_messages_fts};
    use std::collections::VecDeque;

    const ACCOUNT_ID: &str = "acct_sync";
    const INBOX_REMOTE: &str = "INBOX";

    #[derive(Debug, Clone)]
    struct FakeMailboxFolder {
        folder: RemoteFolder,
        uid_validity: u64,
        messages: BTreeMap<u64, RemoteMessage>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FakeFailure {
        ListTransient(String),
    }

    #[derive(Debug)]
    struct FakeImapServer {
        folders: BTreeMap<String, FakeMailboxFolder>,
        failures: VecDeque<FakeFailure>,
        idle_capability: IdleCapability,
        idle_capability_errors: VecDeque<SyncError>,
        idle_events: VecDeque<Result<IdleWaitOutcome, SyncError>>,
        idle_capability_calls: usize,
        idle_wait_calls: usize,
        reconnects: usize,
        list_calls: usize,
        fetch_message_calls: Vec<Vec<u64>>,
        fetch_flag_calls: Vec<Vec<u64>>,
    }

    impl Default for FakeImapServer {
        fn default() -> Self {
            Self {
                folders: BTreeMap::new(),
                failures: VecDeque::new(),
                idle_capability: IdleCapability::Available,
                idle_capability_errors: VecDeque::new(),
                idle_events: VecDeque::new(),
                idle_capability_calls: 0,
                idle_wait_calls: 0,
                reconnects: 0,
                list_calls: 0,
                fetch_message_calls: Vec::new(),
                fetch_flag_calls: Vec::new(),
            }
        }
    }

    impl FakeImapServer {
        fn with_inbox(messages: Vec<RemoteMessage>) -> Self {
            let mut server = Self::default();
            server.add_folder(
                RemoteFolder::from_imap_attributes(
                    INBOX_REMOTE,
                    "INBOX",
                    Some("Inbox".into()),
                    Some("/".into()),
                    ["\\Inbox"],
                ),
                77,
                messages,
            );
            server
        }

        fn add_folder(
            &mut self,
            folder: RemoteFolder,
            uid_validity: u64,
            messages: Vec<RemoteMessage>,
        ) {
            self.folders.insert(
                folder.remote_id.clone(),
                FakeMailboxFolder {
                    folder,
                    uid_validity,
                    messages: messages
                        .into_iter()
                        .map(|message| (message.uid, message))
                        .collect(),
                },
            );
        }

        fn set_inbox_messages(&mut self, uid_validity: u64, messages: Vec<RemoteMessage>) {
            let folder = self
                .folders
                .get_mut(INBOX_REMOTE)
                .expect("inbox fixture must exist");
            folder.uid_validity = uid_validity;
            folder.messages = messages
                .into_iter()
                .map(|message| (message.uid, message))
                .collect();
        }

        fn fail_next_list_transient(&mut self, reason: impl Into<String>) {
            self.failures
                .push_back(FakeFailure::ListTransient(reason.into()));
        }

        fn set_idle_capability(&mut self, capability: IdleCapability) {
            self.idle_capability = capability;
        }

        fn enqueue_idle_capability_error(&mut self, error: SyncError) {
            self.idle_capability_errors.push_back(error);
        }

        fn enqueue_idle_outcome(&mut self, outcome: IdleWaitOutcome) {
            self.idle_events.push_back(Ok(outcome));
        }

        fn enqueue_idle_error(&mut self, error: SyncError) {
            self.idle_events.push_back(Err(error));
        }
    }

    impl ImapSyncServer for FakeImapServer {
        fn list_folders(&mut self, _account_id: &str) -> Result<Vec<RemoteFolder>, SyncError> {
            self.list_calls += 1;
            if let Some(FakeFailure::ListTransient(reason)) = self.failures.pop_front() {
                return Err(SyncError::transient(reason));
            }

            Ok(self
                .folders
                .values()
                .map(|folder| folder.folder.clone())
                .collect())
        }

        fn select_folder(
            &mut self,
            _account_id: &str,
            remote_id: &str,
        ) -> Result<RemoteFolderStatus, SyncError> {
            let folder = self
                .folders
                .get(remote_id)
                .ok_or_else(|| SyncError::permanent("folder missing"))?;
            let existing_uids: BTreeSet<u64> = folder.messages.keys().copied().collect();
            let unread_count = folder
                .messages
                .values()
                .filter(|message| !message.flags.contains(&RemoteFlag::Seen))
                .count() as u32;
            let uid_next = existing_uids
                .iter()
                .next_back()
                .copied()
                .unwrap_or(0)
                .saturating_add(1)
                .max(1);

            Ok(RemoteFolderStatus {
                uid_validity: folder.uid_validity,
                uid_next,
                existing_uids,
                unread_count,
                total_count: folder.messages.len() as u32,
            })
        }

        fn fetch_messages(
            &mut self,
            _account_id: &str,
            remote_id: &str,
            _folder_id: &str,
            uid_validity: u64,
            uids: &[u64],
        ) -> Result<Vec<RemoteMessage>, SyncError> {
            self.fetch_message_calls.push(uids.to_vec());
            let folder = self
                .folders
                .get(remote_id)
                .ok_or_else(|| SyncError::permanent("folder missing"))?;
            if folder.uid_validity != uid_validity {
                return Err(SyncError::permanent("uidvalidity changed"));
            }

            uids.iter()
                .map(|uid| {
                    folder
                        .messages
                        .get(uid)
                        .cloned()
                        .ok_or_else(|| SyncError::permanent("message missing"))
                })
                .collect::<Result<Vec<_>, _>>()
        }

        fn fetch_flags(
            &mut self,
            _account_id: &str,
            remote_id: &str,
            uid_validity: u64,
            uids: &[u64],
        ) -> Result<Vec<RemoteFlagUpdate>, SyncError> {
            self.fetch_flag_calls.push(uids.to_vec());
            let folder = self
                .folders
                .get(remote_id)
                .ok_or_else(|| SyncError::permanent("folder missing"))?;
            if folder.uid_validity != uid_validity {
                return Err(SyncError::permanent("uidvalidity changed"));
            }

            uids.iter()
                .map(|uid| {
                    let message = folder
                        .messages
                        .get(uid)
                        .ok_or_else(|| SyncError::permanent("message missing"))?;
                    Ok(RemoteFlagUpdate {
                        uid: *uid,
                        uid_validity,
                        flags: message.flags.clone(),
                    })
                })
                .collect::<Result<Vec<_>, SyncError>>()
        }

        fn reconnect(&mut self, _account_id: &str) -> Result<(), SyncError> {
            self.reconnects += 1;
            Ok(())
        }
    }

    impl ImapIdleServer for FakeImapServer {
        fn idle_capability(
            &mut self,
            _account_id: &str,
            _remote_id: &str,
        ) -> Result<IdleCapability, SyncError> {
            self.idle_capability_calls += 1;
            if let Some(error) = self.idle_capability_errors.pop_front() {
                return Err(error);
            }

            Ok(self.idle_capability)
        }

        fn idle_wait(
            &mut self,
            _account_id: &str,
            _remote_id: &str,
            _timeout: Duration,
            cancellation: &SyncCancellationToken,
        ) -> Result<IdleWaitOutcome, SyncError> {
            cancellation.ensure_not_cancelled()?;
            self.idle_wait_calls += 1;
            self.idle_events
                .pop_front()
                .unwrap_or(Ok(IdleWaitOutcome::Timeout))
        }
    }

    #[derive(Debug, Default)]
    struct RecordingBackoffWaiter<'connection> {
        delays: Vec<Duration>,
        cancel_during_wait: Option<SyncCancellationToken>,
        remove_account_on_wait: Option<&'connection Connection>,
    }

    impl BackoffWaiter for RecordingBackoffWaiter<'_> {
        fn wait(
            &mut self,
            delay: Duration,
            cancellation: &SyncCancellationToken,
        ) -> Result<(), SyncError> {
            self.delays.push(delay);
            if let Some(token) = &self.cancel_during_wait {
                token.cancel();
            }
            if let Some(connection) = self.remove_account_on_wait.take() {
                connection
                    .execute("DELETE FROM accounts WHERE id = ?1", [ACCOUNT_ID])
                    .map_err(SyncError::from)?;
            }
            cancellation.ensure_not_cancelled()
        }
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
    }

    fn seed_account(connection: &Connection) {
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
                   'generic_imap_smtp',
                   'sync@example.com',
                   'sync@example.com',
                   'app_password',
                   'komorebi:test:sync',
                   'imap.example.com',
                   993,
                   'tls',
                   'smtp.example.com',
                   465,
                   'tls'
                 )",
                [ACCOUNT_ID],
            )
            .unwrap();
    }

    fn run_sync(
        server: &mut FakeImapServer,
        connection: &Connection,
    ) -> Result<SyncAccountReport, SyncError> {
        let mut cache = SqliteSyncCache::new(connection);
        ImapSyncEngine::default().sync_account(
            server,
            &mut cache,
            ACCOUNT_ID,
            &SyncCancellationToken::new(),
            &mut NoopBackoffWaiter,
        )
    }

    fn message(uid_validity: u64, uid: u64, subject: &str, flags: &[RemoteFlag]) -> RemoteMessage {
        let mut message = RemoteMessage::fixture(uid_validity, uid, subject);
        message.flags = flags.iter().cloned().collect();
        message
    }

    fn message_count(connection: &Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap()
    }

    fn message_flag_state(connection: &Connection, uid: u64) -> (i64, i64, i64) {
        connection
            .query_row(
                "SELECT is_read, is_flagged, is_deleted
                 FROM messages
                 WHERE imap_uid = ?1",
                [u64_to_i64(uid).unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    fn worker_config(max_cycles: Option<usize>) -> FolderWorkerConfig<'static> {
        FolderWorkerConfig {
            account_id: ACCOUNT_ID,
            remote_id: INBOX_REMOTE,
            idle_timeout: Duration::from_millis(30),
            poll_interval: Duration::from_millis(20),
            max_cycles,
        }
    }

    fn run_worker(
        server: &mut FakeImapServer,
        connection: &Connection,
        waiter: &mut RecordingBackoffWaiter<'_>,
        max_cycles: Option<usize>,
    ) -> Result<FolderWorkerReport, SyncError> {
        let mut cache = SqliteSyncCache::new(connection);
        ImapSyncEngine::new(BackoffPolicy {
            max_attempts: 3,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(50),
            multiplier: 2,
        })
        .run_folder_worker(
            server,
            &mut cache,
            worker_config(max_cycles),
            &SyncCancellationToken::new(),
            waiter,
        )
    }

    #[test]
    fn imap_flag_tokens_map_standard_flags_and_preserve_custom_keywords() {
        let flags = RemoteFlag::from_imap_tokens([
            "\\seen",
            "\\FLAGGED",
            "\\Answered",
            "\\Draft",
            "\\Deleted",
            "\\Recent",
            "$Forwarded",
            "\\NonJunk",
            "",
            "*",
        ]);

        assert!(flags.contains(&RemoteFlag::Seen));
        assert!(flags.contains(&RemoteFlag::Flagged));
        assert!(flags.contains(&RemoteFlag::Answered));
        assert!(flags.contains(&RemoteFlag::Draft));
        assert!(flags.contains(&RemoteFlag::Deleted));
        assert!(flags.contains(&RemoteFlag::Recent));
        assert!(flags.contains(&RemoteFlag::keyword("$Forwarded")));
        assert!(flags.contains(&RemoteFlag::keyword("\\NonJunk")));

        let preserved: Vec<String> = serde_json::from_str(&flags_json(&flags).unwrap()).unwrap();
        assert_eq!(
            preserved,
            vec![
                "\\Seen",
                "\\Flagged",
                "\\Answered",
                "\\Draft",
                "\\Deleted",
                "\\Recent",
                "$Forwarded",
                "\\NonJunk"
            ]
        );
    }

    #[test]
    fn folder_attributes_detect_special_use_and_preserve_provider_values() {
        let sent = RemoteFolder::from_imap_attributes(
            "[Gmail]/Sent Mail",
            "[Gmail]/Sent Mail",
            None,
            Some("/".into()),
            ["\\sent", "\\HasNoChildren"],
        );
        assert_eq!(sent.display_name, "Sent Mail");
        assert_eq!(sent.kind, RemoteFolderKind::Sent);
        assert_eq!(sent.special_use, Some(RemoteSpecialUse::Sent));
        assert!(sent.attributes.contains("\\Sent"));
        assert!(sent.attributes.contains("\\HasNoChildren"));

        let junk = RemoteFolder::from_imap_attributes(
            "[Gmail]/Spam",
            "[Gmail]/Spam",
            None,
            Some("/".into()),
            ["\\Spam"],
        );
        assert_eq!(junk.kind, RemoteFolderKind::Junk);
        assert_eq!(junk.special_use, Some(RemoteSpecialUse::Junk));
        assert!(junk.attributes.contains("\\Spam"));

        let all_mail = RemoteFolder::from_imap_attributes(
            "[Gmail]/All Mail",
            "[Gmail]/All Mail",
            None,
            Some("/".into()),
            ["\\AllMail"],
        );
        assert_eq!(all_mail.kind, RemoteFolderKind::All);
        assert_eq!(all_mail.special_use, None);
        assert!(all_mail.attributes.contains("\\AllMail"));

        let unknown = RemoteFolder::from_imap_attributes(
            "Projects/Client",
            "Projects/Client",
            None,
            Some("/".into()),
            ["\\VendorRole"],
        );
        assert_eq!(unknown.display_name, "Client");
        assert_eq!(unknown.kind, RemoteFolderKind::Custom);
        assert_eq!(unknown.special_use, None);
        assert!(unknown.attributes.contains("\\VendorRole"));
    }

    #[test]
    fn initial_sync_discovers_folders_and_fetches_messages_into_sqlite_cache() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![
            message(77, 1, "Welcome to Komorebi", &[RemoteFlag::Seen]),
            message(77, 2, "Unread invoice", &[]),
        ]);
        server.add_folder(
            RemoteFolder::from_imap_attributes(
                "[Gmail]/All Mail",
                "[Gmail]/All Mail",
                None,
                Some("/".into()),
                ["\\All"],
            ),
            88,
            Vec::new(),
        );

        let report = run_sync(&mut server, &connection).unwrap();

        assert_eq!(report.folders_seen, 2);
        assert_eq!(report.messages_fetched, 2);
        assert_eq!(report.flags_updated, 0);
        assert_eq!(message_count(&connection), 2);
        assert_eq!(message_flag_state(&connection, 1), (1, 0, 0));

        let folder_row = connection
            .query_row(
                "SELECT kind, special_use, uid_validity, uid_next, unread_count, total_count, sync_state
                 FROM folders
                 WHERE account_id = ?1 AND remote_id = 'INBOX'",
                [ACCOUNT_ID],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            folder_row,
            (
                "inbox".into(),
                Some("inbox".into()),
                77,
                3,
                1,
                2,
                "idle".into()
            )
        );

        let search = search_messages_fts(&connection, "Komorebi", 10).unwrap();
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].subject, "Welcome to Komorebi");
    }

    #[test]
    fn sync_preserves_full_flags_and_unknown_folder_semantics() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::default();
        let mut custom_message = message(55, 10, "Custom folder message", &[]);
        custom_message.flags = RemoteFlag::from_imap_tokens([
            "\\Seen",
            "\\Flagged",
            "\\Answered",
            "\\Draft",
            "\\Deleted",
            "\\Recent",
            "$Forwarded",
            "\\NonJunk",
        ]);
        server.add_folder(
            RemoteFolder::from_imap_attributes(
                "Projects/Client",
                "Projects/Client",
                None,
                Some("/".into()),
                ["\\VendorRole", "\\HasNoChildren"],
            ),
            55,
            vec![custom_message],
        );

        let report = run_sync(&mut server, &connection).unwrap();

        assert_eq!(report.folders_seen, 1);
        assert_eq!(report.messages_fetched, 1);
        assert_eq!(message_count(&connection), 1);

        let folder_row = connection
            .query_row(
                "SELECT kind, special_use, display_name, attributes_json
                 FROM folders
                 WHERE account_id = ?1 AND remote_id = 'Projects/Client'",
                [ACCOUNT_ID],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(folder_row.0, "custom");
        assert_eq!(folder_row.1, None);
        assert_eq!(folder_row.2, "Client");
        let attributes: Vec<String> = serde_json::from_str(&folder_row.3).unwrap();
        assert_eq!(attributes, vec!["\\HasNoChildren", "\\VendorRole"]);

        let message_row = connection
            .query_row(
                "SELECT
                   is_read,
                   is_flagged,
                   is_answered,
                   is_draft,
                   is_deleted,
                   flags_json
                 FROM messages
                 WHERE imap_uid = 10",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            (
                message_row.0,
                message_row.1,
                message_row.2,
                message_row.3,
                message_row.4
            ),
            (1, 1, 1, 1, 1)
        );
        let flags: Vec<String> = serde_json::from_str(&message_row.5).unwrap();
        assert_eq!(
            flags,
            vec![
                "\\Seen",
                "\\Flagged",
                "\\Answered",
                "\\Draft",
                "\\Deleted",
                "\\Recent",
                "$Forwarded",
                "\\NonJunk"
            ]
        );
    }

    #[test]
    fn incremental_sync_fetches_only_new_uids_and_updates_flags() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![
            message(77, 1, "First", &[]),
            message(77, 2, "Second", &[]),
        ]);

        run_sync(&mut server, &connection).unwrap();
        server.fetch_message_calls.clear();
        server.fetch_flag_calls.clear();
        server.set_inbox_messages(
            77,
            vec![
                message(77, 1, "First", &[]),
                message(77, 2, "Second", &[RemoteFlag::Seen, RemoteFlag::Flagged]),
                message(77, 3, "Third", &[RemoteFlag::Seen]),
            ],
        );

        let report = run_sync(&mut server, &connection).unwrap();

        assert_eq!(report.messages_fetched, 1);
        assert_eq!(report.flags_updated, 1);
        assert_eq!(message_count(&connection), 3);
        assert_eq!(server.fetch_message_calls, vec![vec![3]]);
        assert_eq!(server.fetch_flag_calls, vec![vec![1, 2]]);
        assert_eq!(message_flag_state(&connection, 2), (1, 1, 0));
        assert_eq!(message_flag_state(&connection, 3), (1, 0, 0));
    }

    #[test]
    fn expunge_handling_removes_missing_remote_uids_from_cache() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![
            message(77, 1, "Expunged later", &[]),
            message(77, 2, "Still present", &[]),
        ]);

        run_sync(&mut server, &connection).unwrap();
        server.set_inbox_messages(77, vec![message(77, 2, "Still present", &[])]);

        let report = run_sync(&mut server, &connection).unwrap();

        assert_eq!(report.expunged, 1);
        assert_eq!(message_count(&connection), 1);
        let remaining_uid = connection
            .query_row("SELECT imap_uid FROM messages", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(remaining_uid, 2);
        assert!(search_messages_fts(&connection, "Expunged", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn uidvalidity_change_resets_folder_before_refetching() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![message(77, 1, "Old UIDVALIDITY", &[])]);

        run_sync(&mut server, &connection).unwrap();
        server.set_inbox_messages(
            99,
            vec![message(99, 1, "New UIDVALIDITY", &[RemoteFlag::Seen])],
        );

        let report = run_sync(&mut server, &connection).unwrap();

        assert_eq!(report.uid_validity_resets, 1);
        assert_eq!(report.messages_fetched, 1);
        assert_eq!(message_count(&connection), 1);
        assert!(search_messages_fts(&connection, "Old", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            search_messages_fts(&connection, "New", 10).unwrap().len(),
            1
        );
        assert_eq!(message_flag_state(&connection, 1), (1, 0, 0));
    }

    #[test]
    fn reconnect_and_backoff_after_transient_server_failure() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![message(77, 1, "After reconnect", &[])]);
        server.fail_next_list_transient("network drop");
        let token = SyncCancellationToken::new();
        let mut cache = SqliteSyncCache::new(&connection);
        let mut waiter = RecordingBackoffWaiter::default();
        let engine = ImapSyncEngine::new(BackoffPolicy {
            max_attempts: 2,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
            multiplier: 2,
        });

        let report = engine
            .sync_account(&mut server, &mut cache, ACCOUNT_ID, &token, &mut waiter)
            .unwrap();

        assert_eq!(report.reconnects, 1);
        assert_eq!(report.backoff_delays, vec![Duration::from_millis(10)]);
        assert_eq!(waiter.delays, vec![Duration::from_millis(10)]);
        assert_eq!(server.reconnects, 1);
        assert_eq!(server.list_calls, 2);
        assert_eq!(message_count(&connection), 1);
    }

    #[test]
    fn cancellation_stops_before_retry_after_backoff_is_requested() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![message(77, 1, "Should not sync", &[])]);
        server.fail_next_list_transient("temporary outage");
        let token = SyncCancellationToken::new();
        let mut cache = SqliteSyncCache::new(&connection);
        let mut waiter = RecordingBackoffWaiter {
            cancel_during_wait: Some(token.clone()),
            ..RecordingBackoffWaiter::default()
        };
        let engine = ImapSyncEngine::new(BackoffPolicy {
            max_attempts: 3,
            initial_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
            multiplier: 2,
        });

        let error = engine
            .sync_account(&mut server, &mut cache, ACCOUNT_ID, &token, &mut waiter)
            .unwrap_err();

        assert_eq!(error, SyncError::Cancelled);
        assert_eq!(server.reconnects, 1);
        assert_eq!(server.list_calls, 1);
        assert_eq!(message_count(&connection), 0);
    }

    #[test]
    fn idle_mailbox_change_wakes_folder_worker_and_refreshes_cache() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server =
            FakeImapServer::with_inbox(vec![message(77, 1, "Arrived during IDLE", &[])]);
        server.enqueue_idle_outcome(IdleWaitOutcome::MailboxChanged);
        let mut waiter = RecordingBackoffWaiter::default();

        let report = run_worker(&mut server, &connection, &mut waiter, Some(1)).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::MaxCyclesReached);
        assert_eq!(report.cycles_completed, 1);
        assert_eq!(report.idle_waits, 1);
        assert_eq!(report.idle_change_refreshes, 1);
        assert_eq!(report.poll_refreshes, 0);
        assert_eq!(report.sync_runs, 1);
        assert_eq!(message_count(&connection), 1);
        assert_eq!(
            search_messages_fts(&connection, "Arrived", 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn idle_timeout_periodically_refreshes_folder() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server = FakeImapServer::with_inbox(vec![message(77, 1, "Periodic refresh", &[])]);
        server.enqueue_idle_outcome(IdleWaitOutcome::Timeout);
        let mut waiter = RecordingBackoffWaiter::default();

        let report = run_worker(&mut server, &connection, &mut waiter, Some(1)).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::MaxCyclesReached);
        assert_eq!(report.idle_waits, 1);
        assert_eq!(report.idle_timeout_refreshes, 1);
        assert_eq!(report.sync_runs, 1);
        assert_eq!(message_count(&connection), 1);
    }

    #[test]
    fn polling_fallback_runs_when_idle_is_unavailable() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server =
            FakeImapServer::with_inbox(vec![message(77, 1, "Fetched by polling", &[])]);
        server.set_idle_capability(IdleCapability::Unavailable);
        let mut waiter = RecordingBackoffWaiter::default();

        let report = run_worker(&mut server, &connection, &mut waiter, Some(1)).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::MaxCyclesReached);
        assert_eq!(report.idle_waits, 0);
        assert_eq!(report.idle_fallbacks, 1);
        assert_eq!(report.poll_refreshes, 1);
        assert_eq!(report.sync_runs, 1);
        assert_eq!(waiter.delays, vec![Duration::from_millis(20)]);
        assert_eq!(message_count(&connection), 1);
    }

    #[test]
    fn idle_worker_recovers_from_network_drop_with_reconnect_and_backoff() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server =
            FakeImapServer::with_inbox(vec![message(77, 1, "After idle reconnect", &[])]);
        server.enqueue_idle_error(SyncError::transient("idle connection dropped"));
        server.enqueue_idle_outcome(IdleWaitOutcome::MailboxChanged);
        let mut waiter = RecordingBackoffWaiter::default();

        let report = run_worker(&mut server, &connection, &mut waiter, Some(1)).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::MaxCyclesReached);
        assert_eq!(report.idle_waits, 2);
        assert_eq!(report.reconnects, 1);
        assert_eq!(report.backoff_delays, vec![Duration::from_millis(5)]);
        assert_eq!(waiter.delays, vec![Duration::from_millis(5)]);
        assert_eq!(server.reconnects, 1);
        assert_eq!(report.sync_runs, 1);
        assert_eq!(message_count(&connection), 1);
    }

    #[test]
    fn idle_worker_recovers_when_capability_probe_drops() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server =
            FakeImapServer::with_inbox(vec![message(77, 1, "After capability reconnect", &[])]);
        server.enqueue_idle_capability_error(SyncError::transient("capability probe dropped"));
        server.enqueue_idle_outcome(IdleWaitOutcome::MailboxChanged);
        let mut waiter = RecordingBackoffWaiter::default();

        let report = run_worker(&mut server, &connection, &mut waiter, Some(1)).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::MaxCyclesReached);
        assert_eq!(server.idle_capability_calls, 2);
        assert_eq!(report.idle_waits, 1);
        assert_eq!(report.reconnects, 1);
        assert_eq!(server.reconnects, 1);
        assert_eq!(report.sync_runs, 1);
        assert_eq!(message_count(&connection), 1);
    }

    #[test]
    fn idle_worker_stops_cleanly_on_app_exit_cancellation() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server =
            FakeImapServer::with_inbox(vec![message(77, 1, "Should remain remote", &[])]);
        server.enqueue_idle_error(SyncError::Cancelled);
        let mut waiter = RecordingBackoffWaiter::default();

        let report = run_worker(&mut server, &connection, &mut waiter, None).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::Cancelled);
        assert_eq!(report.idle_waits, 1);
        assert_eq!(report.sync_runs, 0);
        assert_eq!(message_count(&connection), 0);
    }

    #[test]
    fn polling_worker_stops_cleanly_when_account_is_removed() {
        let connection = migrated_connection();
        seed_account(&connection);
        let mut server =
            FakeImapServer::with_inbox(vec![message(77, 1, "Removed account message", &[])]);
        server.set_idle_capability(IdleCapability::Unavailable);
        let mut waiter = RecordingBackoffWaiter {
            remove_account_on_wait: Some(&connection),
            ..RecordingBackoffWaiter::default()
        };

        let report = run_worker(&mut server, &connection, &mut waiter, None).unwrap();

        assert_eq!(report.stop_reason, FolderWorkerStopReason::AccountRemoved);
        assert_eq!(report.idle_waits, 0);
        assert_eq!(report.poll_refreshes, 0);
        assert_eq!(report.sync_runs, 0);
        assert_eq!(message_count(&connection), 0);
    }

    #[test]
    fn server_fixture_validation_rejects_missing_fetch_results() {
        let status = RemoteFolderStatus {
            uid_validity: 77,
            uid_next: 3,
            existing_uids: BTreeSet::from([1, 2]),
            unread_count: 0,
            total_count: 2,
        };

        assert_eq!(
            validate_messages(&status, &[1, 2], vec![message(77, 1, "Only one", &[])]).unwrap_err(),
            SyncError::InvalidRemoteState {
                reason: "server did not return all requested message UIDs"
            }
        );
    }

    #[test]
    fn unknown_account_is_rejected_before_server_calls() {
        let connection = migrated_connection();
        let mut cache = SqliteSyncCache::new(&connection);
        let mut server = FakeImapServer::with_inbox(Vec::new());

        let error = ImapSyncEngine::default()
            .sync_account(
                &mut server,
                &mut cache,
                ACCOUNT_ID,
                &SyncCancellationToken::new(),
                &mut NoopBackoffWaiter,
            )
            .unwrap_err();

        assert_eq!(error, SyncError::UnknownAccount);
        assert_eq!(server.list_calls, 0);
    }
}
