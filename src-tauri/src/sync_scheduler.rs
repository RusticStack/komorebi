use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    account_connection::{AccountConnectionTestError, MailTransportSecurity},
    auth::{AuthKind, ProviderKind},
    credentials::{CredentialError, CredentialSlot, WindowsCredentialManager},
    imap_live::{LiveImapAdapter, LiveImapAuth, LiveImapConfig},
    imap_sync::{
        BackoffPolicy, FolderWorkerConfig, ImapSyncEngine, SqliteSyncCache, SyncCancellationToken,
        SyncError, ThreadBackoffWaiter,
    },
    ipc_validation::{validate_bounded_text, validate_entity_id, IpcValidationError},
    oauth::{OAuthClientConfig, OAuthError, ProviderOAuthTokenClient},
    schema::{apply_migrations, SchemaMigrationError},
    token_lifecycle::{
        AccessTokenCache, AccountDataError, AccountDataStore, CachedAccessToken, ReconnectReason,
        SecretString, TokenLifecycle, TokenRefreshResult,
    },
};

const ACCOUNT_SETTING_PREFIX: &str = "account:";
const SYNC_WORKER_STATUS_SUFFIX: &str = ":sync_worker_status";
const OAUTH_CLIENT_CONFIG_SUFFIX: &str = ":oauth_client";
const MAX_OAUTH_CLIENT_ID_BYTES: usize = 2048;
const MAX_OAUTH_REDIRECT_URI_BYTES: usize = 2048;
const DEFAULT_MAX_ACCOUNT_WORKERS: usize = 2;
const DEFAULT_MAX_FOLDER_WORKERS_PER_ACCOUNT: usize = 6;
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(29 * 60);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const WORKER_JOIN_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy)]
pub struct SyncSchedulerOptions {
    pub max_account_workers: usize,
    pub max_folder_workers_per_account: usize,
    pub idle_timeout: Duration,
    pub poll_interval: Duration,
    pub backoff: BackoffPolicy,
}

impl Default for SyncSchedulerOptions {
    fn default() -> Self {
        Self {
            max_account_workers: DEFAULT_MAX_ACCOUNT_WORKERS,
            max_folder_workers_per_account: DEFAULT_MAX_FOLDER_WORKERS_PER_ACCOUNT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            poll_interval: DEFAULT_POLL_INTERVAL,
            backoff: BackoffPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSchedulerStartReport {
    pub accounts_started: usize,
    pub accounts_skipped: usize,
    pub accounts_stopped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredSyncWorkerStatus {
    pub state: String,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncWorkerState {
    Syncing,
    Idle,
    Error,
    Stopped,
}

impl SyncWorkerState {
    fn schema_value(self) -> &'static str {
        match self {
            Self::Syncing => "syncing",
            Self::Idle => "idle",
            Self::Error => "error",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOAuthClientConfig {
    pub client_id: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncSchedulerError {
    pub reason: String,
}

impl SyncSchedulerError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for SyncSchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for SyncSchedulerError {}

impl From<IpcValidationError> for SyncSchedulerError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<rusqlite::Error> for SyncSchedulerError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SchemaMigrationError> for SyncSchedulerError {
    fn from(value: SchemaMigrationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<serde_json::Error> for SyncSchedulerError {
    fn from(value: serde_json::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AccountConnectionTestError> for SyncSchedulerError {
    fn from(value: AccountConnectionTestError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<CredentialError> for SyncSchedulerError {
    fn from(value: CredentialError) -> Self {
        Self::new(match value {
            CredentialError::NotFound => "account credentials are missing".to_string(),
            _ => "credential store is unavailable".to_string(),
        })
    }
}

impl From<OAuthError> for SyncSchedulerError {
    fn from(value: OAuthError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SyncError> for SyncSchedulerError {
    fn from(value: SyncError) -> Self {
        Self::new(value.to_string())
    }
}

#[derive(Clone)]
pub struct BackgroundSyncScheduler {
    inner: Arc<Mutex<SchedulerState>>,
    options: SyncSchedulerOptions,
    account_limiter: Arc<ConcurrencyLimiter>,
}

impl Default for BackgroundSyncScheduler {
    fn default() -> Self {
        Self::new(SyncSchedulerOptions::default())
    }
}

impl BackgroundSyncScheduler {
    pub fn new(options: SyncSchedulerOptions) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SchedulerState::default())),
            options,
            account_limiter: Arc::new(ConcurrencyLimiter::new(options.max_account_workers)),
        }
    }

    pub fn start_all_configured_accounts(
        &self,
        cache_path: impl Into<PathBuf>,
    ) -> Result<SyncSchedulerStartReport, SyncSchedulerError> {
        let cache_path = cache_path.into();
        self.start_all_configured_accounts_with_launcher(cache_path, &LiveAccountWorkerLauncher)
    }

    pub fn refresh_account(
        &self,
        cache_path: impl Into<PathBuf>,
        account_id: &str,
    ) -> Result<bool, SyncSchedulerError> {
        let cache_path = cache_path.into();
        self.refresh_account_with_launcher(cache_path, account_id, &LiveAccountWorkerLauncher)
    }

    pub fn stop_account(&self, account_id: &str) -> Result<bool, SyncSchedulerError> {
        validate_entity_id("account_id", account_id)?;
        if let Some(worker) = self.take_worker(account_id) {
            stop_worker(worker);
            return Ok(true);
        }

        Ok(false)
    }

    pub fn stop_all(&self) {
        let workers = {
            let mut state = self.inner.lock().expect("sync scheduler lock poisoned");
            state.shutting_down = true;
            state
                .workers
                .drain()
                .map(|(_, worker)| worker)
                .collect::<Vec<_>>()
        };

        for worker in workers {
            stop_worker(worker);
        }
    }

    fn start_all_configured_accounts_with_launcher(
        &self,
        cache_path: PathBuf,
        launcher: &dyn AccountWorkerLauncher,
    ) -> Result<SyncSchedulerStartReport, SyncSchedulerError> {
        let mut connection = Connection::open(&cache_path)?;
        apply_migrations(&mut connection)?;
        let active_accounts = load_active_account_ids(&connection)?;
        let active_set = active_accounts.iter().cloned().collect::<HashSet<_>>();
        let stopped = self.stop_workers_not_in(&active_set);

        let mut started = 0;
        let mut skipped = 0;
        for account_id in active_accounts {
            if self.refresh_account_with_launcher(cache_path.clone(), &account_id, launcher)? {
                started += 1;
            } else {
                skipped += 1;
            }
        }

        Ok(SyncSchedulerStartReport {
            accounts_started: started,
            accounts_skipped: skipped,
            accounts_stopped: stopped,
        })
    }

    fn refresh_account_with_launcher(
        &self,
        cache_path: PathBuf,
        account_id: &str,
        launcher: &dyn AccountWorkerLauncher,
    ) -> Result<bool, SyncSchedulerError> {
        validate_entity_id("account_id", account_id)?;
        if let Some(worker) = self.take_worker(account_id) {
            stop_worker(worker);
        }

        let mut connection = Connection::open(&cache_path)?;
        apply_migrations(&mut connection)?;
        if !account_is_sync_eligible(&connection, account_id)? {
            write_sync_worker_status(
                &connection,
                account_id,
                SyncWorkerState::Stopped,
                Some("account is not active"),
            )?;
            return Ok(false);
        }

        let cancellation = SyncCancellationToken::new();
        let job = AccountWorkerJob {
            cache_path,
            account_id: account_id.to_string(),
            cancellation: cancellation.clone(),
            options: self.options,
            account_limiter: Arc::clone(&self.account_limiter),
        };
        let handle = launcher.spawn(job);

        let mut state = self.inner.lock().expect("sync scheduler lock poisoned");
        if state.shutting_down {
            drop(state);
            cancellation.cancel();
            let _ = handle.join();
            return Ok(false);
        }
        state.workers.insert(
            account_id.to_string(),
            RunningAccountWorker {
                cancellation,
                handle,
            },
        );

        Ok(true)
    }

    fn take_worker(&self, account_id: &str) -> Option<RunningAccountWorker> {
        self.inner
            .lock()
            .expect("sync scheduler lock poisoned")
            .workers
            .remove(account_id)
    }

    fn stop_workers_not_in(&self, active_accounts: &HashSet<String>) -> usize {
        let workers = {
            let mut state = self.inner.lock().expect("sync scheduler lock poisoned");
            let stale_ids = state
                .workers
                .keys()
                .filter(|account_id| !active_accounts.contains(*account_id))
                .cloned()
                .collect::<Vec<_>>();
            stale_ids
                .into_iter()
                .filter_map(|account_id| state.workers.remove(&account_id))
                .collect::<Vec<_>>()
        };
        let stopped = workers.len();
        for worker in workers {
            stop_worker(worker);
        }

        stopped
    }

    #[cfg(test)]
    fn running_account_count(&self) -> usize {
        self.inner
            .lock()
            .expect("sync scheduler lock poisoned")
            .workers
            .len()
    }
}

#[derive(Default)]
struct SchedulerState {
    workers: HashMap<String, RunningAccountWorker>,
    shutting_down: bool,
}

struct RunningAccountWorker {
    cancellation: SyncCancellationToken,
    handle: JoinHandle<()>,
}

fn stop_worker(worker: RunningAccountWorker) {
    worker.cancellation.cancel();
    let _ = worker.handle.join();
}

trait AccountWorkerLauncher: Send + Sync {
    fn spawn(&self, job: AccountWorkerJob) -> JoinHandle<()>;
}

struct LiveAccountWorkerLauncher;

impl AccountWorkerLauncher for LiveAccountWorkerLauncher {
    fn spawn(&self, job: AccountWorkerJob) -> JoinHandle<()> {
        thread::spawn(move || run_live_account_worker(job))
    }
}

struct AccountWorkerJob {
    cache_path: PathBuf,
    account_id: String,
    cancellation: SyncCancellationToken,
    options: SyncSchedulerOptions,
    account_limiter: Arc<ConcurrencyLimiter>,
}

fn run_live_account_worker(job: AccountWorkerJob) {
    let permit = match job.account_limiter.acquire(&job.cancellation) {
        Ok(permit) => permit,
        Err(SyncError::Cancelled) => {
            let _ = write_sync_status_for_path(
                &job.cache_path,
                &job.account_id,
                SyncWorkerState::Stopped,
                Some("sync worker was cancelled before start"),
            );
            return;
        }
        Err(error) => {
            let _ = write_sync_status_for_path(
                &job.cache_path,
                &job.account_id,
                SyncWorkerState::Error,
                Some(&error.to_string()),
            );
            return;
        }
    };

    let _permit = permit;
    let _ = write_sync_status_for_path(
        &job.cache_path,
        &job.account_id,
        SyncWorkerState::Syncing,
        None,
    );

    let result = run_live_account_worker_inner(&job);
    match result {
        Ok(()) => {
            let _ = write_sync_status_for_path(
                &job.cache_path,
                &job.account_id,
                SyncWorkerState::Idle,
                None,
            );
        }
        Err(SyncSchedulerError { reason }) if job.cancellation.is_cancelled() => {
            let _ = write_sync_status_for_path(
                &job.cache_path,
                &job.account_id,
                SyncWorkerState::Stopped,
                Some(&reason),
            );
        }
        Err(error) => {
            let _ = write_sync_status_for_path(
                &job.cache_path,
                &job.account_id,
                SyncWorkerState::Error,
                Some(&error.reason),
            );
        }
    }
}

fn run_live_account_worker_inner(job: &AccountWorkerJob) -> Result<(), SyncSchedulerError> {
    job.cancellation
        .ensure_not_cancelled()
        .map_err(SyncSchedulerError::from)?;

    let mut connection = Connection::open(&job.cache_path)?;
    apply_migrations(&mut connection)?;
    if !account_is_sync_eligible(&connection, &job.account_id)? {
        return Err(SyncSchedulerError::new("account is not active"));
    }

    {
        let config = load_live_imap_config(&connection, &job.account_id)?;
        let mut server = LiveImapAdapter::connect(config)?;
        let mut cache = SqliteSyncCache::new(&connection);
        let mut waiter = ThreadBackoffWaiter;
        let engine = ImapSyncEngine::new(job.options.backoff);
        engine.sync_account(
            &mut server,
            &mut cache,
            &job.account_id,
            &job.cancellation,
            &mut waiter,
        )?;
    }

    if job.cancellation.is_cancelled() {
        return Ok(());
    }

    let folder_remote_ids = load_background_folder_remote_ids(
        &connection,
        &job.account_id,
        job.options.max_folder_workers_per_account,
    )?;
    write_sync_worker_status(&connection, &job.account_id, SyncWorkerState::Idle, None)?;
    drop(connection);

    let mut handles = folder_remote_ids
        .into_iter()
        .map(|remote_id| spawn_live_folder_worker(job, remote_id))
        .collect::<Vec<_>>();

    while !job.cancellation.is_cancelled() {
        let mut index = 0;
        while index < handles.len() {
            if handles[index].is_finished() {
                let handle = handles.swap_remove(index);
                let _ = handle.join();
            } else {
                index += 1;
            }
        }

        if handles.is_empty() {
            return Ok(());
        }

        thread::sleep(WORKER_JOIN_POLL);
    }

    for handle in handles {
        let _ = handle.join();
    }

    Ok(())
}

fn spawn_live_folder_worker(job: &AccountWorkerJob, remote_id: String) -> JoinHandle<()> {
    let cache_path = job.cache_path.clone();
    let account_id = job.account_id.clone();
    let cancellation = job.cancellation.clone();
    let options = job.options;

    thread::Builder::new()
        .name(format!("komorebi-sync-{}-{}", account_id, remote_id))
        .spawn(move || {
            if let Err(error) =
                run_live_folder_worker(&cache_path, &account_id, &remote_id, &cancellation, options)
            {
                let _ = mark_folder_sync_error(&cache_path, &account_id, &remote_id);
                if !cancellation.is_cancelled() {
                    let _ = write_sync_status_for_path(
                        &cache_path,
                        &account_id,
                        SyncWorkerState::Error,
                        Some(&error.reason),
                    );
                }
            }
        })
        .unwrap_or_else(|_| thread::spawn(|| {}))
}

fn run_live_folder_worker(
    cache_path: &Path,
    account_id: &str,
    remote_id: &str,
    cancellation: &SyncCancellationToken,
    options: SyncSchedulerOptions,
) -> Result<(), SyncSchedulerError> {
    cancellation
        .ensure_not_cancelled()
        .map_err(SyncSchedulerError::from)?;
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    if !account_is_sync_eligible(&connection, account_id)? {
        return Ok(());
    }

    let config = load_live_imap_config(&connection, account_id)?;
    let mut server = LiveImapAdapter::connect(config)?;
    let mut cache = SqliteSyncCache::new(&connection);
    let mut waiter = ThreadBackoffWaiter;
    let engine = ImapSyncEngine::new(options.backoff);
    engine.run_folder_worker(
        &mut server,
        &mut cache,
        FolderWorkerConfig {
            account_id,
            remote_id,
            idle_timeout: options.idle_timeout,
            poll_interval: options.poll_interval,
            max_cycles: None,
        },
        cancellation,
        &mut waiter,
    )?;

    Ok(())
}

fn load_active_account_ids(connection: &Connection) -> Result<Vec<String>, SyncSchedulerError> {
    let mut statement = connection.prepare(
        "SELECT id
         FROM accounts
         WHERE removed_at IS NULL
           AND auth_status = 'connected'
         ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut account_ids = Vec::new();
    for row in rows {
        account_ids.push(row?);
    }

    Ok(account_ids)
}

fn account_is_sync_eligible(
    connection: &Connection,
    account_id: &str,
) -> Result<bool, SyncSchedulerError> {
    let exists = connection.query_row(
        "SELECT EXISTS(
           SELECT 1
           FROM accounts
           WHERE id = ?1
             AND removed_at IS NULL
             AND auth_status = 'connected'
         )",
        params![account_id],
        |row| row.get::<_, bool>(0),
    )?;

    Ok(exists)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SyncAccountConfig {
    account_id: String,
    provider: ProviderKind,
    auth_kind: AuthKind,
    username: String,
    imap_host: String,
    imap_port: u16,
    imap_security: MailTransportSecurity,
}

fn load_live_imap_config(
    connection: &Connection,
    account_id: &str,
) -> Result<LiveImapConfig, SyncSchedulerError> {
    let account = load_sync_account_config(connection, account_id)?;
    let auth = match account.auth_kind {
        AuthKind::OAuth2 => {
            let access_token =
                refresh_oauth_access_token(connection, &account.account_id, account.provider)?;
            LiveImapAuth::XOAuth2 {
                username: account.username.clone(),
                access_token,
            }
        }
        AuthKind::AppPassword | AuthKind::ManualPassword => {
            let credentials = WindowsCredentialManager;
            let secret = match credentials.read_secret(account_id, CredentialSlot::GenericPassword)
            {
                Ok(secret) => secret,
                Err(CredentialError::NotFound) => {
                    mark_reconnect_needed(connection, account_id)?;
                    return Err(SyncSchedulerError::new("account credentials are missing"));
                }
                Err(error) => return Err(error.into()),
            };
            LiveImapAuth::Password {
                username: account.username.clone(),
                secret,
            }
        }
    };

    Ok(LiveImapConfig::new(
        account.account_id,
        account.imap_host,
        account.imap_port,
        account.imap_security,
        auth,
    ))
}

fn load_sync_account_config(
    connection: &Connection,
    account_id: &str,
) -> Result<SyncAccountConfig, SyncSchedulerError> {
    validate_entity_id("account_id", account_id)?;
    let row = connection
        .query_row(
            "SELECT id,
                    provider_kind,
                    auth_kind,
                    username,
                    imap_host,
                    imap_port,
                    imap_security
             FROM accounts
             WHERE id = ?1
               AND removed_at IS NULL
               AND auth_status = 'connected'",
            params![account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<u16>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| SyncSchedulerError::new("account is not active"))?;

    let provider = provider_from_schema(&row.1)?;
    let auth_kind = auth_from_schema(&row.2)?;
    let (imap_host, imap_port, imap_security) = match provider {
        ProviderKind::Gmail => (
            "imap.gmail.com".to_string(),
            993,
            MailTransportSecurity::Tls,
        ),
        ProviderKind::Microsoft => (
            "outlook.office365.com".to_string(),
            993,
            MailTransportSecurity::Tls,
        ),
        ProviderKind::GenericImapSmtp => (
            row.4
                .ok_or_else(|| SyncSchedulerError::new("IMAP host is missing"))?,
            row.5
                .ok_or_else(|| SyncSchedulerError::new("IMAP port is missing"))?,
            MailTransportSecurity::from_schema(row.6.as_deref().unwrap_or_default())?,
        ),
    };

    Ok(SyncAccountConfig {
        account_id: row.0,
        provider,
        auth_kind,
        username: row.3,
        imap_host,
        imap_port,
        imap_security,
    })
}

fn refresh_oauth_access_token(
    connection: &Connection,
    account_id: &str,
    provider: ProviderKind,
) -> Result<SecretString, SyncSchedulerError> {
    let stored = load_account_oauth_client_config(connection, account_id)?.ok_or_else(|| {
        SyncSchedulerError::new(
            "OAuth client configuration is missing; reauthentication is required",
        )
    })?;
    let config = OAuthClientConfig::new(provider, stored.client_id, stored.redirect_uri, None)?;
    let mut lifecycle = TokenLifecycle::new(
        WindowsCredentialManager,
        InMemoryAccessTokenCache::default(),
        ProviderOAuthTokenClient::new(vec![config]),
        SqliteSchedulerAccountDataStore { connection },
    );

    match lifecycle.ensure_access_token(account_id, provider, unix_now()) {
        TokenRefreshResult::Ready { grant, .. } => Ok(grant.access_token().clone()),
        TokenRefreshResult::ReconnectNeeded { .. } => Err(SyncSchedulerError::new(
            "OAuth refresh failed; reauthentication is required",
        )),
    }
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

struct SqliteSchedulerAccountDataStore<'connection> {
    connection: &'connection Connection,
}

impl AccountDataStore for SqliteSchedulerAccountDataStore<'_> {
    fn mark_reconnect_needed(
        &mut self,
        account_id: &str,
        _reason: ReconnectReason,
    ) -> Result<(), AccountDataError> {
        mark_reconnect_needed(self.connection, account_id)
            .map_err(|_| AccountDataError::Unavailable)
    }

    fn delete_local_cache(&mut self, _account_id: &str) -> Result<(), AccountDataError> {
        Err(AccountDataError::Unavailable)
    }

    fn delete_account_metadata(&mut self, _account_id: &str) -> Result<(), AccountDataError> {
        Err(AccountDataError::Unavailable)
    }
}

fn mark_reconnect_needed(
    connection: &Connection,
    account_id: &str,
) -> Result<(), SyncSchedulerError> {
    connection.execute(
        "UPDATE accounts
         SET auth_status = 'reconnect_needed',
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1
           AND removed_at IS NULL",
        params![account_id],
    )?;
    Ok(())
}

fn load_background_folder_remote_ids(
    connection: &Connection,
    account_id: &str,
    limit: usize,
) -> Result<Vec<String>, SyncSchedulerError> {
    let limit = limit.max(1);
    let mut statement = connection.prepare(
        "SELECT remote_id
         FROM folders
         WHERE account_id = ?1
         ORDER BY
           CASE kind
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
           display_name COLLATE NOCASE,
           remote_id
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![account_id, limit as i64], |row| {
        row.get::<_, String>(0)
    })?;
    let mut remote_ids = Vec::new();
    for row in rows {
        remote_ids.push(row?);
    }

    Ok(remote_ids)
}

fn mark_folder_sync_error(
    cache_path: &Path,
    account_id: &str,
    remote_id: &str,
) -> Result<(), SyncSchedulerError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    connection.execute(
        "UPDATE folders
         SET sync_state = 'error',
             updated_at = CURRENT_TIMESTAMP
         WHERE account_id = ?1
           AND remote_id = ?2",
        params![account_id, remote_id],
    )?;
    Ok(())
}

pub fn account_sync_worker_status_key(account_id: &str) -> String {
    format!("{ACCOUNT_SETTING_PREFIX}{account_id}{SYNC_WORKER_STATUS_SUFFIX}")
}

pub fn account_oauth_client_config_key(account_id: &str) -> String {
    format!("{ACCOUNT_SETTING_PREFIX}{account_id}{OAUTH_CLIENT_CONFIG_SUFFIX}")
}

pub fn store_account_oauth_client_config(
    connection: &Connection,
    account_id: &str,
    client_id: &str,
    redirect_uri: &str,
) -> Result<(), SyncSchedulerError> {
    validate_entity_id("account_id", account_id)?;
    validate_bounded_text("oauth_client_id", client_id, MAX_OAUTH_CLIENT_ID_BYTES)?;
    let client_id = client_id.to_string();
    validate_bounded_text(
        "oauth_redirect_uri",
        redirect_uri,
        MAX_OAUTH_REDIRECT_URI_BYTES,
    )?;
    let redirect_uri = redirect_uri.to_string();
    let value_json = serde_json::to_string(&StoredOAuthClientConfig {
        client_id,
        redirect_uri,
    })?;
    upsert_setting(
        connection,
        &account_oauth_client_config_key(account_id),
        "object",
        &value_json,
    )?;
    Ok(())
}

fn load_account_oauth_client_config(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<StoredOAuthClientConfig>, SyncSchedulerError> {
    let value_json = connection
        .query_row(
            "SELECT value_json
             FROM settings
             WHERE key = ?1",
            params![account_oauth_client_config_key(account_id)],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    value_json
        .map(|value| serde_json::from_str::<StoredOAuthClientConfig>(&value))
        .transpose()
        .map_err(Into::into)
}

pub fn delete_account_sync_scheduler_settings(
    connection: &Connection,
    account_id: &str,
) -> Result<(), SyncSchedulerError> {
    validate_entity_id("account_id", account_id)?;
    connection.execute(
        "DELETE FROM settings WHERE key IN (?1, ?2)",
        params![
            account_sync_worker_status_key(account_id),
            account_oauth_client_config_key(account_id)
        ],
    )?;
    Ok(())
}

fn write_sync_status_for_path(
    cache_path: &Path,
    account_id: &str,
    state: SyncWorkerState,
    message: Option<&str>,
) -> Result<(), SyncSchedulerError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    write_sync_worker_status(&connection, account_id, state, message)
}

fn write_sync_worker_status(
    connection: &Connection,
    account_id: &str,
    state: SyncWorkerState,
    message: Option<&str>,
) -> Result<(), SyncSchedulerError> {
    validate_entity_id("account_id", account_id)?;
    let status = StoredSyncWorkerStatus {
        state: state.schema_value().to_string(),
        updated_at: unix_now(),
        message: message.map(redact_status_message),
    };
    let value_json = serde_json::to_string(&status)?;
    upsert_setting(
        connection,
        &account_sync_worker_status_key(account_id),
        "object",
        &value_json,
    )?;
    Ok(())
}

fn upsert_setting(
    connection: &Connection,
    key: &str,
    value_type: &str,
    value_json: &str,
) -> Result<(), SyncSchedulerError> {
    connection.execute(
        "INSERT INTO settings (key, value_type, value_json, updated_at)
         VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET
           value_type = excluded.value_type,
           value_json = excluded.value_json,
           updated_at = CURRENT_TIMESTAMP",
        params![key, value_type, value_json],
    )?;
    Ok(())
}

fn redact_status_message(value: &str) -> String {
    const MAX_STATUS_MESSAGE_BYTES: usize = 240;
    let mut redacted = value.replace(['\r', '\n', '\0'], " ");
    if redacted.len() > MAX_STATUS_MESSAGE_BYTES {
        redacted.truncate(MAX_STATUS_MESSAGE_BYTES);
    }
    redacted
}

fn provider_from_schema(value: &str) -> Result<ProviderKind, SyncSchedulerError> {
    match value {
        "gmail" => Ok(ProviderKind::Gmail),
        "microsoft" => Ok(ProviderKind::Microsoft),
        "generic_imap_smtp" => Ok(ProviderKind::GenericImapSmtp),
        _ => Err(SyncSchedulerError::new("provider_kind is unsupported")),
    }
}

fn auth_from_schema(value: &str) -> Result<AuthKind, SyncSchedulerError> {
    match value {
        "oauth2" => Ok(AuthKind::OAuth2),
        "app_password" => Ok(AuthKind::AppPassword),
        "manual_password" => Ok(AuthKind::ManualPassword),
        _ => Err(SyncSchedulerError::new("auth_kind is unsupported")),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug)]
struct ConcurrencyLimiter {
    max: usize,
    state: Mutex<usize>,
    condvar: Condvar,
}

impl ConcurrencyLimiter {
    fn new(max: usize) -> Self {
        Self {
            max: max.max(1),
            state: Mutex::new(0),
            condvar: Condvar::new(),
        }
    }

    fn acquire(
        self: &Arc<Self>,
        cancellation: &SyncCancellationToken,
    ) -> Result<ConcurrencyPermit, SyncError> {
        let mut active = self.state.lock().expect("sync limiter lock poisoned");
        loop {
            cancellation.ensure_not_cancelled()?;
            if *active < self.max {
                *active += 1;
                return Ok(ConcurrencyPermit {
                    limiter: Arc::clone(self),
                });
            }

            let (next_active, _) = self
                .condvar
                .wait_timeout(active, Duration::from_millis(100))
                .expect("sync limiter wait poisoned");
            active = next_active;
        }
    }
}

struct ConcurrencyPermit {
    limiter: Arc<ConcurrencyLimiter>,
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        let mut active = self
            .limiter
            .state
            .lock()
            .expect("sync limiter lock poisoned");
        *active = active.saturating_sub(1);
        self.limiter.condvar.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Default)]
    struct FakeLauncher {
        spawned: Arc<Mutex<Vec<String>>>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl AccountWorkerLauncher for FakeLauncher {
        fn spawn(&self, job: AccountWorkerJob) -> JoinHandle<()> {
            self.spawned
                .lock()
                .expect("spawned lock")
                .push(job.account_id.clone());
            let active = Arc::clone(&self.active);
            let max_active = Arc::clone(&self.max_active);
            let release = Arc::clone(&self.release);
            thread::spawn(move || {
                let Ok(_permit) = job.account_limiter.acquire(&job.cancellation) else {
                    return;
                };
                let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                loop {
                    let previous = max_active.load(Ordering::SeqCst);
                    if now_active <= previous
                        || max_active
                            .compare_exchange(
                                previous,
                                now_active,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            )
                            .is_ok()
                    {
                        break;
                    }
                }

                let (lock, condvar) = &*release;
                let mut released = lock.lock().expect("release lock");
                while !*released && !job.cancellation.is_cancelled() {
                    let wait = condvar
                        .wait_timeout(released, Duration::from_millis(25))
                        .expect("release wait");
                    released = wait.0;
                }
                active.fetch_sub(1, Ordering::SeqCst);
            })
        }
    }

    impl FakeLauncher {
        fn spawned_accounts(&self) -> Vec<String> {
            self.spawned.lock().expect("spawned lock").clone()
        }

        fn release_all(&self) {
            let (lock, condvar) = &*self.release;
            *lock.lock().expect("release lock") = true;
            condvar.notify_all();
        }
    }

    #[test]
    fn startup_loads_only_active_accounts() {
        let cache_path = migrated_file_database();
        {
            let connection = Connection::open(&cache_path).unwrap();
            insert_account(&connection, "acct_active", "connected", None);
            insert_account(&connection, "acct_reconnect", "reconnect_needed", None);
            insert_account(
                &connection,
                "acct_removed",
                "connected",
                Some("2026-05-23 00:00:00"),
            );
        }

        let scheduler = BackgroundSyncScheduler::new(test_options(2));
        let launcher = FakeLauncher::default();
        let report = scheduler
            .start_all_configured_accounts_with_launcher(cache_path.clone(), &launcher)
            .unwrap();

        assert_eq!(report.accounts_started, 1);
        assert_eq!(launcher.spawned_accounts(), vec!["acct_active"]);
        scheduler.stop_all();
        launcher.release_all();
        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn refresh_stops_previous_worker_before_restart() {
        let cache_path = migrated_file_database();
        {
            let connection = Connection::open(&cache_path).unwrap();
            insert_account(&connection, "acct_active", "connected", None);
        }

        let scheduler = BackgroundSyncScheduler::new(test_options(1));
        let launcher = FakeLauncher::default();
        assert!(scheduler
            .refresh_account_with_launcher(cache_path.clone(), "acct_active", &launcher)
            .unwrap());
        assert_eq!(scheduler.running_account_count(), 1);

        assert!(scheduler
            .refresh_account_with_launcher(cache_path.clone(), "acct_active", &launcher)
            .unwrap());
        assert_eq!(scheduler.running_account_count(), 1);
        assert_eq!(
            launcher.spawned_accounts(),
            vec!["acct_active".to_string(), "acct_active".to_string()]
        );

        scheduler.stop_all();
        launcher.release_all();
        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn scheduler_stops_workers_for_removed_accounts() {
        let cache_path = migrated_file_database();
        {
            let connection = Connection::open(&cache_path).unwrap();
            insert_account(&connection, "acct_active", "connected", None);
            insert_account(&connection, "acct_removed_later", "connected", None);
        }
        let scheduler = BackgroundSyncScheduler::new(test_options(2));
        let launcher = FakeLauncher::default();
        scheduler
            .start_all_configured_accounts_with_launcher(cache_path.clone(), &launcher)
            .unwrap();

        {
            let connection = Connection::open(&cache_path).unwrap();
            connection
                .execute(
                    "UPDATE accounts SET removed_at = CURRENT_TIMESTAMP WHERE id = 'acct_removed_later'",
                    [],
                )
                .unwrap();
        }

        let report = scheduler
            .start_all_configured_accounts_with_launcher(cache_path.clone(), &launcher)
            .unwrap();
        assert_eq!(report.accounts_stopped, 1);
        assert_eq!(scheduler.running_account_count(), 1);

        scheduler.stop_all();
        launcher.release_all();
        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn account_worker_concurrency_is_bounded() {
        let cache_path = migrated_file_database();
        {
            let connection = Connection::open(&cache_path).unwrap();
            insert_account(&connection, "acct_a", "connected", None);
            insert_account(&connection, "acct_b", "connected", None);
            insert_account(&connection, "acct_c", "connected", None);
        }

        let scheduler = BackgroundSyncScheduler::new(test_options(1));
        let launcher = FakeLauncher::default();
        scheduler
            .start_all_configured_accounts_with_launcher(cache_path.clone(), &launcher)
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(launcher.max_active.load(Ordering::SeqCst), 1);

        scheduler.stop_all();
        launcher.release_all();
        let _ = fs::remove_file(cache_path);
    }

    #[test]
    fn sync_status_and_oauth_config_are_persisted_and_deleted() {
        let connection = migrated_connection();
        insert_account(&connection, "acct_oauth", "connected", None);

        store_account_oauth_client_config(
            &connection,
            "acct_oauth",
            "client-id",
            "http://127.0.0.1:49152",
        )
        .unwrap();
        write_sync_worker_status(
            &connection,
            "acct_oauth",
            SyncWorkerState::Syncing,
            Some("syncing now"),
        )
        .unwrap();

        let config = load_account_oauth_client_config(&connection, "acct_oauth")
            .unwrap()
            .unwrap();
        assert_eq!(config.client_id, "client-id");
        let status_json: String = connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [account_sync_worker_status_key("acct_oauth")],
                |row| row.get(0),
            )
            .unwrap();
        assert!(status_json.contains("\"state\":\"syncing\""));

        delete_account_sync_scheduler_settings(&connection, "acct_oauth").unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    fn test_options(max_account_workers: usize) -> SyncSchedulerOptions {
        SyncSchedulerOptions {
            max_account_workers,
            max_folder_workers_per_account: 1,
            idle_timeout: Duration::from_millis(50),
            poll_interval: Duration::from_millis(50),
            backoff: BackoffPolicy {
                max_attempts: 1,
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(1),
                multiplier: 1,
            },
        }
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection).unwrap();
        connection
    }

    fn migrated_file_database() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "komorebi-sync-scheduler-{}-{}.sqlite3",
            std::process::id(),
            nanos
        ));
        let mut connection = Connection::open(&path).unwrap();
        apply_migrations(&mut connection).unwrap();
        drop(connection);
        path
    }

    fn insert_account(
        connection: &Connection,
        account_id: &str,
        status: &str,
        removed_at: Option<&str>,
    ) {
        connection
            .execute(
                "INSERT INTO accounts (
                   id,
                   provider_kind,
                   display_email,
                   username,
                   auth_kind,
                   credential_ref,
                   auth_status,
                   imap_host,
                   imap_port,
                   imap_security,
                   smtp_host,
                   smtp_port,
                   smtp_security,
                   removed_at
                 )
                 VALUES (?1, 'generic_imap_smtp', ?2, ?2, 'manual_password', ?3, ?4,
                         'imap.example.com', 993, 'tls', 'smtp.example.com', 587, 'starttls', ?5)",
                params![
                    account_id,
                    format!("{account_id}@example.com"),
                    format!("Komorebi/mail-account/{account_id}/generic-password"),
                    status,
                    removed_at
                ],
            )
            .unwrap();
    }
}
