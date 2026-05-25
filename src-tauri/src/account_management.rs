use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::{
    account_connection::{
        AccountConnectionTestError, GenericAccountConnectionConfig, GenericAccountConnectionTester,
        LiveGenericAccountConnectionTester, MailTransportSecurity,
    },
    account_signatures::{AccountSignatureError, AccountSignatureHtml},
    attachment_safety::INBOUND_ATTACHMENT_DIR_NAME,
    auth::{evaluate_account_setup_auth_policy, AuthKind, AuthPolicy, ProviderKind},
    credentials::{
        credential_target_name, CredentialError, CredentialSlot, WindowsCredentialManager,
    },
    ipc_validation::{
        validate_bounded_text, validate_entity_id, validate_payload_size, IpcValidationError,
        MAX_TEXT_FIELD_BYTES,
    },
    oauth::{revoke_refresh_token, OAuthHttpClient, ReqwestOAuthHttpClient},
    schema::{apply_migrations, SchemaMigrationError},
    sync_scheduler::{delete_account_sync_scheduler_settings, BackgroundSyncScheduler},
    token_lifecycle::{RevocationOutcome, SecretString},
};

const MAIL_CACHE_FILE_NAME: &str = "mail-cache.sqlite3";
const MAX_EMAIL_BYTES: usize = 320;
const MAX_HOST_BYTES: usize = 253;
const MAX_DISPLAY_NAME_BYTES: usize = 256;
const REMOVED_ACCOUNT_PREFIX: &str = "removed";
const ACCOUNT_LOCAL_PREFERENCES_PREFIX: &str = "account:";
const ACCOUNT_LOCAL_PREFERENCES_SUFFIX: &str = ":local_preferences";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSetupRequest {
    pub account_id: Option<String>,
    pub provider_kind: String,
    pub display_email: String,
    pub display_name: Option<String>,
    pub username: String,
    pub auth_kind: String,
    pub secret: String,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountReauthRequest {
    pub account_id: String,
    pub secret: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRemovalRequest {
    pub account_id: String,
    pub delete_local_cache: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountAuthStatusRequest {
    pub account_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountLocalPreferences {
    #[serde(default = "default_include_in_unified_inbox")]
    pub include_in_unified_inbox: bool,
}

impl Default for AccountLocalPreferences {
    fn default() -> Self {
        Self {
            include_in_unified_inbox: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSettingsUpdateRequest {
    pub account_id: String,
    pub display_name: Option<String>,
    pub signature_html: String,
    pub local_preferences: AccountLocalPreferences,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSetupResponse {
    pub account: AccountManagementView,
    pub credential_stored: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSettingsUpdateResponse {
    pub account: AccountManagementView,
    pub signature_html: Option<String>,
    pub local_preferences: AccountLocalPreferences,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountConnectionTestResponse {
    pub provider_kind: String,
    pub auth_kind: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountRemovalResponse {
    pub account_id: String,
    pub credentials_deleted: bool,
    pub sync_stopped: bool,
    pub access_cache_cleared: bool,
    pub provider_revocation: AccountProviderRevocationStatus,
    pub local_cache_deleted: bool,
    pub local_cache_retained: bool,
    pub metadata_removed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AccountProviderRevocationStatus {
    Revoked,
    Unsupported,
    Failed,
    NoRefreshTokenPresent,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountAuthStatusView {
    pub account_id: String,
    pub provider_kind: String,
    pub auth_kind: String,
    pub auth_status: String,
    pub credential_present: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountManagementView {
    pub id: String,
    pub provider_kind: String,
    pub display_email: String,
    pub display_name: Option<String>,
    pub username: String,
    pub auth_kind: String,
    pub auth_status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountCommandError {
    pub reason: String,
}

impl AccountCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for AccountCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for AccountCommandError {}

impl From<IpcValidationError> for AccountCommandError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<SchemaMigrationError> for AccountCommandError {
    fn from(value: SchemaMigrationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<rusqlite::Error> for AccountCommandError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<std::io::Error> for AccountCommandError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<tauri::Error> for AccountCommandError {
    fn from(value: tauri::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<CredentialError> for AccountCommandError {
    fn from(value: CredentialError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AccountSignatureError> for AccountCommandError {
    fn from(value: AccountSignatureError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AccountConnectionTestError> for AccountCommandError {
    fn from(value: AccountConnectionTestError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<crate::sync_scheduler::SyncSchedulerError> for AccountCommandError {
    fn from(value: crate::sync_scheduler::SyncSchedulerError) -> Self {
        Self::new(value.reason)
    }
}

impl From<serde_json::Error> for AccountCommandError {
    fn from(value: serde_json::Error) -> Self {
        Self::new(value.to_string())
    }
}

pub trait AccountCredentialStore {
    fn read_account_secret(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<SecretString, CredentialError>;

    fn write_account_secret(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
        secret: &SecretString,
    ) -> Result<(), CredentialError>;

    fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), CredentialError>;

    fn account_secret_exists(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<bool, CredentialError>;
}

impl AccountCredentialStore for WindowsCredentialManager {
    fn read_account_secret(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<SecretString, CredentialError> {
        self.read_secret(account_id, slot)
    }

    fn write_account_secret(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.write_secret(account_id, slot, secret)
    }

    fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), CredentialError> {
        self.delete_all_account_secrets(account_id)
    }

    fn account_secret_exists(
        &mut self,
        account_id: &str,
        slot: CredentialSlot,
    ) -> Result<bool, CredentialError> {
        match self.read_secret(account_id, slot) {
            Ok(_) => Ok(true),
            Err(CredentialError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[tauri::command]
pub async fn create_account(
    app: tauri::AppHandle,
    request: AccountSetupRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    let response = tauri::async_runtime::spawn_blocking({
        let cache_path = cache_path.clone();
        move || {
            reject_direct_major_provider_oauth_setup(&request)?;
            let mut credentials = WindowsCredentialManager;
            let mut connection_tester = LiveGenericAccountConnectionTester::default();

            create_account_from_path_with_connection_tester(
                &cache_path,
                &mut credentials,
                &mut connection_tester,
                request,
            )
        }
    })
    .await
    .map_err(|error| AccountCommandError::new(error.to_string()))??;
    let _ = scheduler.refresh_account(cache_path, &response.account.id);
    Ok(response)
}

#[tauri::command]
pub fn reauth_account(
    app: tauri::AppHandle,
    request: AccountReauthRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    reject_direct_major_provider_oauth_reauth_from_path(&cache_path, &request.account_id)?;
    let mut credentials = WindowsCredentialManager;

    let response = reauth_account_from_path(&cache_path, &mut credentials, request)?;
    let _ = scheduler.refresh_account(cache_path, &response.account.id);
    Ok(response)
}

#[tauri::command]
pub fn remove_account(
    app: tauri::AppHandle,
    request: AccountRemovalRequest,
) -> Result<AccountRemovalResponse, AccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    let account_id = request.account_id.clone();
    let sync_stopped = scheduler.stop_account(&account_id).unwrap_or(false);
    let mut credentials = WindowsCredentialManager;

    let mut response = remove_account_from_path(&cache_path, &mut credentials, request);
    match &mut response {
        Ok(response) => {
            response.sync_stopped = sync_stopped;
            response.access_cache_cleared = true;
        }
        Err(_) => {
            let _ = scheduler.refresh_account(cache_path, &account_id);
        }
    }

    response
}

#[tauri::command]
pub fn update_account_settings(
    app: tauri::AppHandle,
    request: AccountSettingsUpdateRequest,
) -> Result<AccountSettingsUpdateResponse, AccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    let response = update_account_settings_from_path(&cache_path, request)?;
    let _ = scheduler.refresh_account(cache_path, &response.account.id);
    Ok(response)
}

#[tauri::command]
pub async fn test_account_connection(
    request: AccountSetupRequest,
) -> Result<AccountConnectionTestResponse, AccountCommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection_tester = LiveGenericAccountConnectionTester::default();
        test_account_connection_request_with_tester(request, &mut connection_tester)
    })
    .await
    .map_err(|error| AccountCommandError::new(error.to_string()))?
}

fn reject_direct_major_provider_oauth_setup(
    request: &AccountSetupRequest,
) -> Result<(), AccountCommandError> {
    if matches!(request.provider_kind.as_str(), "gmail" | "microsoft") {
        return Err(AccountCommandError::new(
            "Gmail and Microsoft account setup must use the OAuth browser flow",
        ));
    }

    Ok(())
}

fn reject_direct_major_provider_oauth_reauth_from_path(
    cache_path: &Path,
    account_id: &str,
) -> Result<(), AccountCommandError> {
    validate_entity_id("account_id", account_id)?;
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    reject_direct_major_provider_oauth_reauth_from_connection(&connection, account_id)
}

fn reject_direct_major_provider_oauth_reauth_from_connection(
    connection: &Connection,
    account_id: &str,
) -> Result<(), AccountCommandError> {
    let account = connection
        .query_row(
            "SELECT provider_kind, auth_kind
             FROM accounts
             WHERE id = ?1
               AND removed_at IS NULL
               AND auth_status != 'removed'",
            params![account_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    if let Some((provider_kind, auth_kind)) = account {
        if matches!(provider_kind.as_str(), "gmail" | "microsoft") && auth_kind == "oauth2" {
            return Err(AccountCommandError::new(
                "Gmail and Microsoft reauthentication must use the OAuth browser flow",
            ));
        }
    }

    Ok(())
}

#[tauri::command]
pub fn get_account_auth_status(
    app: tauri::AppHandle,
    request: AccountAuthStatusRequest,
) -> Result<AccountAuthStatusView, AccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let mut credentials = WindowsCredentialManager;

    get_account_auth_status_from_path(&cache_path, &mut credentials, request)
}

pub fn create_account_from_path(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    request: AccountSetupRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    create_account_from_connection(&connection, credentials, request)
}

pub fn create_account_from_path_with_connection_tester(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    connection_tester: &mut impl GenericAccountConnectionTester,
    request: AccountSetupRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    create_account_from_connection_with_connection_tester(
        &connection,
        credentials,
        connection_tester,
        request,
    )
}

pub fn create_account_from_connection(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    request: AccountSetupRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let validated = validate_setup_request(request)?;
    let account_id = validated.account_id.clone();
    if account_exists(connection, &account_id)? {
        return Err(AccountCommandError::new("account already exists"));
    }

    credentials.write_account_secret(
        &account_id,
        validated.credential_slot,
        &SecretString::new(validated.secret.clone()),
    )?;

    let credential_ref = credential_target_name(&account_id, validated.credential_slot)?;
    let insert_result = insert_account(connection, &validated, &credential_ref);

    match insert_result {
        Ok(account) => Ok(AccountSetupResponse {
            account,
            credential_stored: true,
        }),
        Err(error) => {
            let _ = credentials.delete_account_secrets(&account_id);
            Err(error)
        }
    }
}

pub fn create_account_from_connection_with_connection_tester(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    connection_tester: &mut impl GenericAccountConnectionTester,
    request: AccountSetupRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let validated = validate_setup_request(request)?;
    let account_id = validated.account_id.clone();
    if account_exists(connection, &account_id)? {
        return Err(AccountCommandError::new("account already exists"));
    }

    test_validated_generic_connection(&validated, connection_tester)?;

    credentials.write_account_secret(
        &account_id,
        validated.credential_slot,
        &SecretString::new(validated.secret.clone()),
    )?;

    let credential_ref = credential_target_name(&account_id, validated.credential_slot)?;
    let insert_result = insert_account(connection, &validated, &credential_ref);

    match insert_result {
        Ok(account) => Ok(AccountSetupResponse {
            account,
            credential_stored: true,
        }),
        Err(error) => {
            let _ = credentials.delete_account_secrets(&account_id);
            Err(error)
        }
    }
}

pub fn reauth_account_from_path(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    request: AccountReauthRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    reauth_account_from_connection(&connection, credentials, request)
}

pub fn reauth_account_from_connection(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    request: AccountReauthRequest,
) -> Result<AccountSetupResponse, AccountCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    validate_secret_text(&request.secret)?;

    let existing = load_account_for_management(connection, &request.account_id)?
        .ok_or_else(|| AccountCommandError::new("account was not found"))?;
    let provider = provider_from_schema(&existing.provider_kind)?;
    let auth_kind = auth_from_schema(&existing.auth_kind)?;
    let credential_slot = allowed_credential_slot(provider, auth_kind)?;

    credentials.write_account_secret(
        &request.account_id,
        credential_slot,
        &SecretString::new(request.secret),
    )?;

    connection.execute(
        "UPDATE accounts
         SET credential_ref = ?2,
             auth_status = 'connected',
             last_auth_at = CURRENT_TIMESTAMP,
             removed_at = NULL
         WHERE id = ?1",
        params![
            request.account_id,
            credential_target_name(&existing.id, credential_slot)?
        ],
    )?;

    let account = load_account_for_management(connection, &existing.id)?
        .ok_or_else(|| AccountCommandError::new("account was not found after reauthentication"))?;

    Ok(AccountSetupResponse {
        account,
        credential_stored: true,
    })
}

pub fn remove_account_from_path(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    request: AccountRemovalRequest,
) -> Result<AccountRemovalResponse, AccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    let revocation_http = ReqwestOAuthHttpClient::default();
    remove_account_from_path_with_revocation_client(
        cache_path,
        &connection,
        credentials,
        &revocation_http,
        request,
    )
}

fn remove_account_from_path_with_revocation_client(
    cache_path: &Path,
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    revocation_http: &impl OAuthHttpClient,
    request: AccountRemovalRequest,
) -> Result<AccountRemovalResponse, AccountCommandError> {
    let account_id = request.account_id.clone();
    let delete_local_cache = request.delete_local_cache;
    let response = remove_account_from_connection_with_revocation_client(
        connection,
        credentials,
        revocation_http,
        request,
    )?;
    if delete_local_cache {
        if let Some(app_data_dir) = cache_path.parent() {
            delete_account_attachment_dir_best_effort(app_data_dir, &account_id);
        }
    }

    Ok(response)
}

pub fn remove_account_from_connection(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    request: AccountRemovalRequest,
) -> Result<AccountRemovalResponse, AccountCommandError> {
    let revocation_http = ReqwestOAuthHttpClient::default();
    remove_account_from_connection_with_revocation_client(
        connection,
        credentials,
        &revocation_http,
        request,
    )
}

fn remove_account_from_connection_with_revocation_client(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    revocation_http: &impl OAuthHttpClient,
    request: AccountRemovalRequest,
) -> Result<AccountRemovalResponse, AccountCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    ensure_active_account(connection, &request.account_id)?;
    let account = load_account_for_management(connection, &request.account_id)?
        .ok_or_else(|| AccountCommandError::new("account was not found"))?;
    let provider = provider_from_schema(&account.provider_kind)?;
    let auth_kind = auth_from_schema(&account.auth_kind)?;
    let provider_revocation = revoke_account_refresh_token(
        credentials,
        revocation_http,
        &request.account_id,
        provider,
        auth_kind,
    );

    credentials.delete_account_secrets(&request.account_id)?;

    if request.delete_local_cache {
        connection.execute(
            "DELETE FROM accounts WHERE id = ?1",
            params![request.account_id],
        )?;
    } else {
        let removed_username = format!("{REMOVED_ACCOUNT_PREFIX}_{}", request.account_id);
        let removed_credential_ref = format!(
            "{REMOVED_ACCOUNT_PREFIX}_{}",
            credential_hash(&request.account_id)
        );
        connection.execute(
            "UPDATE accounts
             SET auth_status = 'removed',
                 removed_at = CURRENT_TIMESTAMP,
                 username = ?2,
                 credential_ref = ?3,
                 last_auth_at = NULL
             WHERE id = ?1",
            params![request.account_id, removed_username, removed_credential_ref],
        )?;
    }
    delete_account_local_preferences(connection, &request.account_id)?;
    delete_account_sync_scheduler_settings(connection, &request.account_id)?;

    Ok(AccountRemovalResponse {
        account_id: request.account_id,
        credentials_deleted: true,
        sync_stopped: false,
        access_cache_cleared: true,
        provider_revocation,
        local_cache_deleted: request.delete_local_cache,
        local_cache_retained: !request.delete_local_cache,
        metadata_removed: true,
    })
}

pub fn update_account_settings_from_path(
    cache_path: &Path,
    request: AccountSettingsUpdateRequest,
) -> Result<AccountSettingsUpdateResponse, AccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    update_account_settings_from_connection(&connection, request)
}

fn delete_account_attachment_dir_best_effort(app_data_dir: &Path, account_id: &str) {
    if validate_entity_id("account_id", account_id).is_err() {
        return;
    }

    let path = app_data_dir
        .join(INBOUND_ATTACHMENT_DIR_NAME)
        .join(account_id);
    let _ = fs::remove_dir_all(path);
}

fn revoke_account_refresh_token(
    credentials: &mut impl AccountCredentialStore,
    revocation_http: &impl OAuthHttpClient,
    account_id: &str,
    provider: ProviderKind,
    auth_kind: AuthKind,
) -> AccountProviderRevocationStatus {
    if provider == ProviderKind::GenericImapSmtp || auth_kind != AuthKind::OAuth2 {
        return AccountProviderRevocationStatus::NotApplicable;
    }

    let refresh_token =
        match credentials.read_account_secret(account_id, CredentialSlot::OAuthRefresh) {
            Ok(secret) => secret,
            Err(CredentialError::NotFound) => {
                return AccountProviderRevocationStatus::NoRefreshTokenPresent;
            }
            Err(_) => return AccountProviderRevocationStatus::Failed,
        };

    match revoke_refresh_token(revocation_http, provider, &refresh_token) {
        RevocationOutcome::Revoked => AccountProviderRevocationStatus::Revoked,
        RevocationOutcome::ProviderRevocationUnsupported => {
            AccountProviderRevocationStatus::Unsupported
        }
        RevocationOutcome::Failed => AccountProviderRevocationStatus::Failed,
        RevocationOutcome::NoRefreshTokenPresent => {
            AccountProviderRevocationStatus::NoRefreshTokenPresent
        }
    }
}

pub fn update_account_settings_from_connection(
    connection: &Connection,
    request: AccountSettingsUpdateRequest,
) -> Result<AccountSettingsUpdateResponse, AccountCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    ensure_active_account(connection, &request.account_id)?;
    let display_name = normalize_optional_text("display_name", request.display_name)?;
    let sanitized = AccountSignatureHtml::sanitize(&request.signature_html)?;
    let signature_html = sanitized
        .as_ref()
        .map(|signature| signature.as_str().to_string());
    let preferences_json = serde_json::to_string(&request.local_preferences)?;
    let settings_key = account_local_preferences_key(&request.account_id);

    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE accounts
         SET display_name = ?2,
             signature_html = ?3,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1
           AND removed_at IS NULL
           AND auth_status != 'removed'",
        params![
            &request.account_id,
            display_name.as_deref(),
            signature_html.as_deref()
        ],
    )?;
    if changed != 1 {
        return Err(AccountCommandError::new("account was not found"));
    }
    transaction.execute(
        "INSERT INTO settings (key, value_json, value_type)
         VALUES (?1, ?2, 'object')
         ON CONFLICT(key) DO UPDATE SET
           value_json = excluded.value_json,
           value_type = excluded.value_type,
           updated_at = CURRENT_TIMESTAMP",
        params![settings_key, preferences_json],
    )?;
    transaction.commit()?;

    let account = load_account_for_management(connection, &request.account_id)?
        .ok_or_else(|| AccountCommandError::new("account was not found after settings update"))?;
    Ok(AccountSettingsUpdateResponse {
        account,
        signature_html,
        local_preferences: request.local_preferences,
    })
}

pub fn test_account_connection_request(
    request: AccountSetupRequest,
) -> Result<AccountConnectionTestResponse, AccountCommandError> {
    let validated = validate_setup_request(request)?;

    Ok(AccountConnectionTestResponse {
        provider_kind: validated.provider_kind_schema().into(),
        auth_kind: validated.auth_kind_schema().into(),
        status: "configuration-ready".into(),
    })
}

pub fn test_account_connection_request_with_tester(
    request: AccountSetupRequest,
    connection_tester: &mut impl GenericAccountConnectionTester,
) -> Result<AccountConnectionTestResponse, AccountCommandError> {
    reject_direct_major_provider_oauth_setup(&request)?;
    let validated = validate_setup_request(request)?;
    let status = if validated.provider_kind == ProviderKind::GenericImapSmtp {
        test_validated_generic_connection(&validated, connection_tester)?;
        "connection-ready"
    } else {
        "configuration-ready"
    };

    Ok(AccountConnectionTestResponse {
        provider_kind: validated.provider_kind_schema().into(),
        auth_kind: validated.auth_kind_schema().into(),
        status: status.into(),
    })
}

pub fn get_account_auth_status_from_path(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    request: AccountAuthStatusRequest,
) -> Result<AccountAuthStatusView, AccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    get_account_auth_status_from_connection(&connection, credentials, request)
}

pub fn get_account_auth_status_from_connection(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    request: AccountAuthStatusRequest,
) -> Result<AccountAuthStatusView, AccountCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    ensure_active_account(connection, &request.account_id)?;
    let account = load_account_for_management(connection, &request.account_id)?
        .ok_or_else(|| AccountCommandError::new("account was not found"))?;
    let provider = provider_from_schema(&account.provider_kind)?;
    let auth_kind = auth_from_schema(&account.auth_kind)?;
    let credential_slot = allowed_credential_slot(provider, auth_kind)?;
    let credential_present = credentials.account_secret_exists(&account.id, credential_slot)?;
    let auth_status = if credential_present {
        account.auth_status
    } else {
        connection.execute(
            "UPDATE accounts
             SET auth_status = 'reconnect_needed',
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1
               AND removed_at IS NULL
               AND auth_status != 'removed'",
            params![account.id],
        )?;
        "reconnect_needed".into()
    };

    Ok(AccountAuthStatusView {
        account_id: account.id,
        provider_kind: account.provider_kind,
        auth_kind: account.auth_kind,
        auth_status,
        credential_present,
    })
}

pub(crate) fn mail_cache_path(app: &tauri::AppHandle) -> Result<PathBuf, AccountCommandError> {
    let app_data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data_dir)?;
    Ok(app_data_dir.join(MAIL_CACHE_FILE_NAME))
}

#[derive(Debug, Clone)]
struct ValidatedAccountSetup {
    account_id: String,
    provider_kind: ProviderKind,
    auth_kind: AuthKind,
    credential_slot: CredentialSlot,
    display_email: String,
    display_name: Option<String>,
    username: String,
    secret: String,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_security: Option<String>,
}

impl ValidatedAccountSetup {
    fn provider_kind_schema(&self) -> &'static str {
        provider_schema_value(self.provider_kind)
    }

    fn auth_kind_schema(&self) -> &'static str {
        auth_schema_value(self.auth_kind)
    }
}

fn validate_setup_request(
    request: AccountSetupRequest,
) -> Result<ValidatedAccountSetup, AccountCommandError> {
    let provider_kind = provider_from_schema(&request.provider_kind)?;
    let auth_kind = auth_from_schema(&request.auth_kind)?;
    let credential_slot = allowed_credential_slot(provider_kind, auth_kind)?;

    let display_email = normalize_email("display_email", request.display_email)?;
    let username = normalize_non_empty("username", request.username, MAX_EMAIL_BYTES)?;
    validate_email_shape("username", &username)?;
    let display_name = normalize_optional_text("display_name", request.display_name)?;
    validate_secret_text(&request.secret)?;

    let account_id = match request.account_id {
        Some(account_id) => {
            validate_entity_id("account_id", &account_id)?;
            account_id
        }
        None => stable_account_id(provider_kind, &username),
    };

    let (imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security) =
        if provider_kind == ProviderKind::GenericImapSmtp {
            (
                Some(validate_host("imap_host", request.imap_host)?),
                Some(validate_required_port("imap_port", request.imap_port)?),
                Some(validate_security("imap_security", request.imap_security)?),
                Some(validate_host("smtp_host", request.smtp_host)?),
                Some(validate_required_port("smtp_port", request.smtp_port)?),
                Some(validate_security("smtp_security", request.smtp_security)?),
            )
        } else {
            (None, None, None, None, None, None)
        };

    Ok(ValidatedAccountSetup {
        account_id,
        provider_kind,
        auth_kind,
        credential_slot,
        display_email,
        display_name,
        username,
        secret: request.secret,
        imap_host,
        imap_port,
        imap_security,
        smtp_host,
        smtp_port,
        smtp_security,
    })
}

fn insert_account(
    connection: &Connection,
    request: &ValidatedAccountSetup,
    credential_ref: &str,
) -> Result<AccountManagementView, AccountCommandError> {
    connection.execute(
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
           last_auth_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'connected', CURRENT_TIMESTAMP)",
        params![
            request.account_id,
            request.provider_kind_schema(),
            request.display_email,
            request.display_name,
            request.username,
            request.auth_kind_schema(),
            credential_ref,
            request.imap_host,
            request.imap_port,
            request.imap_security,
            request.smtp_host,
            request.smtp_port,
            request.smtp_security,
        ],
    )?;

    load_account_for_management(connection, &request.account_id)?
        .ok_or_else(|| AccountCommandError::new("account was not found after insert"))
}

fn test_validated_generic_connection(
    request: &ValidatedAccountSetup,
    connection_tester: &mut impl GenericAccountConnectionTester,
) -> Result<(), AccountCommandError> {
    if request.provider_kind != ProviderKind::GenericImapSmtp {
        return Ok(());
    }

    connection_tester.test_generic_account(&generic_connection_config(request)?)?;
    Ok(())
}

fn generic_connection_config(
    request: &ValidatedAccountSetup,
) -> Result<GenericAccountConnectionConfig, AccountCommandError> {
    Ok(GenericAccountConnectionConfig {
        username: request.username.clone(),
        secret: SecretString::new(request.secret.clone()),
        imap_host: request
            .imap_host
            .clone()
            .ok_or_else(|| AccountCommandError::new("imap_host is required"))?,
        imap_port: request
            .imap_port
            .ok_or_else(|| AccountCommandError::new("imap_port is required"))?,
        imap_security: MailTransportSecurity::from_schema(
            request.imap_security.as_deref().unwrap_or_default(),
        )?,
        smtp_host: request
            .smtp_host
            .clone()
            .ok_or_else(|| AccountCommandError::new("smtp_host is required"))?,
        smtp_port: request
            .smtp_port
            .ok_or_else(|| AccountCommandError::new("smtp_port is required"))?,
        smtp_security: MailTransportSecurity::from_schema(
            request.smtp_security.as_deref().unwrap_or_default(),
        )?,
    })
}

fn load_account_for_management(
    connection: &Connection,
    account_id: &str,
) -> Result<Option<AccountManagementView>, AccountCommandError> {
    let account = connection
        .query_row(
            "SELECT id,
                    provider_kind,
                    display_email,
                    display_name,
                    username,
                    auth_kind,
                    auth_status
             FROM accounts
             WHERE id = ?1",
            params![account_id],
            |row| {
                Ok(AccountManagementView {
                    id: row.get(0)?,
                    provider_kind: row.get(1)?,
                    display_email: row.get(2)?,
                    display_name: row.get(3)?,
                    username: row.get(4)?,
                    auth_kind: row.get(5)?,
                    auth_status: row.get(6)?,
                })
            },
        )
        .optional()?;

    Ok(account)
}

fn account_exists(connection: &Connection, account_id: &str) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
        params![account_id],
        |row| row.get(0),
    )
}

fn ensure_active_account(
    connection: &Connection,
    account_id: &str,
) -> Result<(), AccountCommandError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(
           SELECT 1
           FROM accounts
           WHERE id = ?1
             AND removed_at IS NULL
             AND auth_status != 'removed'
         )",
        params![account_id],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(AccountCommandError::new("account was not found"))
    }
}

pub fn load_account_local_preferences_from_connection(
    connection: &Connection,
    account_id: &str,
) -> Result<AccountLocalPreferences, AccountCommandError> {
    validate_entity_id("account_id", account_id)?;
    let value_json = connection
        .query_row(
            "SELECT value_json
             FROM settings
             WHERE key = ?1",
            params![account_local_preferences_key(account_id)],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    value_json
        .map(|value| serde_json::from_str::<AccountLocalPreferences>(&value))
        .transpose()
        .map_err(Into::into)
        .map(|preferences| preferences.unwrap_or_default())
}

fn delete_account_local_preferences(
    connection: &Connection,
    account_id: &str,
) -> Result<(), AccountCommandError> {
    connection.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![account_local_preferences_key(account_id)],
    )?;
    Ok(())
}

pub fn account_local_preferences_key(account_id: &str) -> String {
    format!("{ACCOUNT_LOCAL_PREFERENCES_PREFIX}{account_id}{ACCOUNT_LOCAL_PREFERENCES_SUFFIX}")
}

fn default_include_in_unified_inbox() -> bool {
    true
}

fn allowed_credential_slot(
    provider: ProviderKind,
    auth_kind: AuthKind,
) -> Result<CredentialSlot, AccountCommandError> {
    match evaluate_account_setup_auth_policy(provider, auth_kind) {
        AuthPolicy::Allowed {
            credential_slot, ..
        } => credential_slot_from_policy(credential_slot),
        AuthPolicy::UnsupportedUntilImplemented { reason, .. }
        | AuthPolicy::Rejected { reason } => Err(AccountCommandError::new(reason)),
    }
}

fn credential_slot_from_policy(value: &str) -> Result<CredentialSlot, AccountCommandError> {
    match value {
        "oauth-refresh" => Ok(CredentialSlot::OAuthRefresh),
        "generic-password" => Ok(CredentialSlot::GenericPassword),
        _ => Err(AccountCommandError::new("unsupported credential slot")),
    }
}

fn provider_from_schema(value: &str) -> Result<ProviderKind, AccountCommandError> {
    match value {
        "gmail" => Ok(ProviderKind::Gmail),
        "microsoft" => Ok(ProviderKind::Microsoft),
        "generic_imap_smtp" => Ok(ProviderKind::GenericImapSmtp),
        _ => Err(AccountCommandError::new("provider_kind is unsupported")),
    }
}

fn provider_schema_value(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Gmail => "gmail",
        ProviderKind::Microsoft => "microsoft",
        ProviderKind::GenericImapSmtp => "generic_imap_smtp",
    }
}

fn auth_from_schema(value: &str) -> Result<AuthKind, AccountCommandError> {
    match value {
        "oauth2" => Ok(AuthKind::OAuth2),
        "app_password" => Ok(AuthKind::AppPassword),
        "manual_password" => Ok(AuthKind::ManualPassword),
        _ => Err(AccountCommandError::new("auth_kind is unsupported")),
    }
}

fn auth_schema_value(auth_kind: AuthKind) -> &'static str {
    match auth_kind {
        AuthKind::OAuth2 => "oauth2",
        AuthKind::AppPassword => "app_password",
        AuthKind::ManualPassword => "manual_password",
    }
}

fn normalize_email(field: &'static str, value: String) -> Result<String, AccountCommandError> {
    let value = normalize_non_empty(field, value, MAX_EMAIL_BYTES)?;
    validate_email_shape(field, &value)?;
    Ok(value)
}

fn normalize_non_empty(
    field: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, AccountCommandError> {
    let value = value.trim().to_string();
    validate_bounded_text(field, &value, max_bytes)?;
    Ok(value)
}

fn normalize_optional_text(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<String>, AccountCommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validate_payload_size(field, value.as_bytes(), MAX_DISPLAY_NAME_BYTES)?;
    Ok(Some(value.to_string()))
}

fn validate_secret_text(value: &str) -> Result<(), AccountCommandError> {
    validate_payload_size("secret", value.as_bytes(), MAX_TEXT_FIELD_BYTES)?;
    if value.trim().is_empty() {
        return Err(AccountCommandError::new("secret is required"));
    }
    if value
        .chars()
        .any(|character| matches!(character, '\r' | '\n' | '\0'))
    {
        return Err(AccountCommandError::new(
            "secret contains unsupported control characters",
        ));
    }

    Ok(())
}

fn validate_email_shape(field: &'static str, value: &str) -> Result<(), AccountCommandError> {
    if value.contains(char::is_whitespace)
        || value.contains('<')
        || value.contains('>')
        || value.matches('@').count() != 1
        || value.starts_with('@')
        || value.ends_with('@')
    {
        return Err(AccountCommandError::new(format!("{field} is invalid")));
    }

    Ok(())
}

fn validate_host(
    field: &'static str,
    value: Option<String>,
) -> Result<String, AccountCommandError> {
    let Some(value) = value else {
        return Err(AccountCommandError::new(format!("{field} is required")));
    };
    let value = normalize_non_empty(field, value, MAX_HOST_BYTES)?;

    if value.contains("://")
        || value.contains('/')
        || value.contains('\\')
        || value.contains('@')
        || value.contains(':')
        || value.starts_with('.')
        || value.ends_with('.')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(AccountCommandError::new(format!("{field} is invalid")));
    }

    for label in value.split('.') {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return Err(AccountCommandError::new(format!("{field} is invalid")));
        }
    }

    Ok(value)
}

fn validate_required_port(
    field: &'static str,
    value: Option<u16>,
) -> Result<u16, AccountCommandError> {
    match value {
        Some(0) | None => Err(AccountCommandError::new(format!("{field} is required"))),
        Some(value) => Ok(value),
    }
}

fn validate_security(
    field: &'static str,
    value: Option<String>,
) -> Result<String, AccountCommandError> {
    let Some(value) = value else {
        return Err(AccountCommandError::new(format!("{field} is required")));
    };
    match value.as_str() {
        "tls" | "starttls" => Ok(value),
        "none" => Err(AccountCommandError::new(format!(
            "{field} must use TLS or STARTTLS"
        ))),
        _ => Err(AccountCommandError::new(format!("{field} is unsupported"))),
    }
}

fn stable_account_id(provider: ProviderKind, username: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_schema_value(provider).as_bytes());
    hasher.update(b":");
    hasher.update(username.to_ascii_lowercase().as_bytes());
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("acct_{hex}")
}

fn credential_hash(account_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(account_id.as_bytes());
    hasher.update(b":removed:");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    hasher.update(now.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::apply_migrations;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct FakeAccountCredentialStore {
        secrets: HashMap<(String, CredentialSlot), String>,
        deleted: HashSet<String>,
        fail_writes: bool,
        fail_deletes: bool,
    }

    impl AccountCredentialStore for FakeAccountCredentialStore {
        fn read_account_secret(
            &mut self,
            account_id: &str,
            slot: CredentialSlot,
        ) -> Result<SecretString, CredentialError> {
            self.secrets
                .get(&(account_id.to_string(), slot))
                .map(|secret| SecretString::new(secret.clone()))
                .ok_or(CredentialError::NotFound)
        }

        fn write_account_secret(
            &mut self,
            account_id: &str,
            slot: CredentialSlot,
            secret: &SecretString,
        ) -> Result<(), CredentialError> {
            if self.fail_writes {
                return Err(CredentialError::WindowsUnavailable {
                    operation: "write",
                    code: 5,
                });
            }
            self.secrets.insert(
                (account_id.to_string(), slot),
                secret.expose_secret().to_string(),
            );
            Ok(())
        }

        fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), CredentialError> {
            if self.fail_deletes {
                return Err(CredentialError::WindowsUnavailable {
                    operation: "delete",
                    code: 5,
                });
            }
            self.secrets
                .retain(|(stored_account_id, _), _| stored_account_id != account_id);
            self.deleted.insert(account_id.to_string());
            Ok(())
        }

        fn account_secret_exists(
            &mut self,
            account_id: &str,
            slot: CredentialSlot,
        ) -> Result<bool, CredentialError> {
            Ok(self.secrets.contains_key(&(account_id.to_string(), slot)))
        }
    }

    #[derive(Default)]
    struct FakeGenericConnectionTester {
        calls: Vec<GenericAccountConnectionConfig>,
        error: Option<AccountConnectionTestError>,
    }

    impl GenericAccountConnectionTester for FakeGenericConnectionTester {
        fn test_generic_account(
            &mut self,
            config: &GenericAccountConnectionConfig,
        ) -> Result<(), AccountConnectionTestError> {
            self.calls.push(config.clone());
            if let Some(error) = &self.error {
                return Err(error.clone());
            }

            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeOAuthRevocationHttp {
        status: u16,
        requests: Vec<(String, Vec<(String, String)>)>,
    }

    impl OAuthHttpClient for std::cell::RefCell<FakeOAuthRevocationHttp> {
        fn post_form(
            &self,
            url: &str,
            fields: &[(&str, String)],
        ) -> Result<crate::oauth::OAuthHttpResponse, crate::oauth::OAuthHttpError> {
            let mut borrowed = self.borrow_mut();
            borrowed.requests.push((
                url.to_string(),
                fields
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), value.clone()))
                    .collect(),
            ));
            Ok(crate::oauth::OAuthHttpResponse {
                status: borrowed.status,
                body: "{}".into(),
            })
        }
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open in-memory db");
        apply_migrations(&mut connection).expect("apply migrations");
        connection
    }

    fn migrated_file_database(label: &str) -> (PathBuf, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let app_data_dir = std::env::temp_dir().join(format!(
            "komorebi-account-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&app_data_dir).unwrap();
        let cache_path = app_data_dir.join(MAIL_CACHE_FILE_NAME);
        let mut connection = Connection::open(&cache_path).expect("open file db");
        apply_migrations(&mut connection).expect("apply migrations");
        drop(connection);
        (cache_path, app_data_dir)
    }

    fn generic_request() -> AccountSetupRequest {
        AccountSetupRequest {
            account_id: Some("acct_generic".into()),
            provider_kind: "generic_imap_smtp".into(),
            display_email: "person@example.com".into(),
            display_name: Some("Personal".into()),
            username: "person@example.com".into(),
            auth_kind: "app_password".into(),
            secret: "app-password".into(),
            imap_host: Some("imap.example.com".into()),
            imap_port: Some(993),
            imap_security: Some("tls".into()),
            smtp_host: Some("smtp.example.com".into()),
            smtp_port: Some(587),
            smtp_security: Some("starttls".into()),
        }
    }

    #[test]
    fn create_generic_account_writes_secret_before_metadata() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();

        let response =
            create_account_from_connection(&connection, &mut credentials, generic_request())
                .expect("create account");

        assert_eq!(response.account.id, "acct_generic");
        assert_eq!(response.account.provider_kind, "generic_imap_smtp");
        assert_eq!(response.account.auth_status, "connected");
        assert_eq!(
            credentials
                .secrets
                .get(&("acct_generic".into(), CredentialSlot::GenericPassword))
                .map(String::as_str),
            Some("app-password"),
        );
        let account_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(account_count, 1);
    }

    #[test]
    fn credential_failure_prevents_metadata_insert() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore {
            fail_writes: true,
            ..Default::default()
        };

        let error =
            create_account_from_connection(&connection, &mut credentials, generic_request())
                .expect_err("credential write fails");

        assert!(error.reason.contains("credential write failed"));
        let account_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(account_count, 0);
    }

    #[test]
    fn duplicate_metadata_rolls_back_stored_secret_best_effort() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create first account");

        let duplicate_error =
            create_account_from_connection(&connection, &mut credentials, generic_request())
                .expect_err("duplicate account rejected");

        assert_eq!(duplicate_error.reason, "account already exists");
        assert!(!credentials.deleted.contains("acct_generic"));
        assert_eq!(
            credentials
                .secrets
                .get(&("acct_generic".into(), CredentialSlot::GenericPassword))
                .map(String::as_str),
            Some("app-password"),
        );
    }

    #[test]
    fn duplicate_username_rolls_back_only_new_secret_best_effort() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create first account");
        let mut duplicate_username = generic_request();
        duplicate_username.account_id = Some("acct_second".into());
        duplicate_username.secret = "second-secret".into();

        create_account_from_connection(&connection, &mut credentials, duplicate_username)
            .expect_err("duplicate username rejected");

        assert!(credentials.deleted.contains("acct_second"));
        assert_eq!(
            credentials
                .secrets
                .get(&("acct_generic".into(), CredentialSlot::GenericPassword))
                .map(String::as_str),
            Some("app-password"),
        );
        assert!(!credentials
            .secrets
            .contains_key(&("acct_second".into(), CredentialSlot::GenericPassword)));
    }

    #[test]
    fn major_provider_passwords_are_rejected_before_secret_write() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let mut request = generic_request();
        request.provider_kind = "gmail".into();
        request.auth_kind = "app_password".into();
        request.imap_host = None;
        request.imap_port = None;
        request.imap_security = None;
        request.smtp_host = None;
        request.smtp_port = None;
        request.smtp_security = None;

        let error = create_account_from_connection(&connection, &mut credentials, request)
            .expect_err("gmail password rejected");

        assert!(error.reason.contains("Gmail requires OAuth2"));
        assert!(credentials.secrets.is_empty());
    }

    #[test]
    fn gmail_oauth_account_uses_refresh_token_slot() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let mut request = generic_request();
        request.account_id = Some("acct_gmail".into());
        request.provider_kind = "gmail".into();
        request.auth_kind = "oauth2".into();
        request.secret = "refresh-token".into();
        request.imap_host = None;
        request.imap_port = None;
        request.imap_security = None;
        request.smtp_host = None;
        request.smtp_port = None;
        request.smtp_security = None;

        create_account_from_connection(&connection, &mut credentials, request)
            .expect("create oauth account");

        assert_eq!(
            credentials
                .secrets
                .get(&("acct_gmail".into(), CredentialSlot::OAuthRefresh))
                .map(String::as_str),
            Some("refresh-token"),
        );
    }

    #[test]
    fn frontend_callable_major_provider_secret_handoff_is_rejected() {
        let mut request = generic_request();
        request.account_id = Some("acct_gmail".into());
        request.provider_kind = "gmail".into();
        request.auth_kind = "oauth2".into();
        request.secret = "refresh-token".into();

        let error = reject_direct_major_provider_oauth_setup(&request)
            .expect_err("direct setup must use loopback");

        assert_eq!(
            error.reason,
            "Gmail and Microsoft account setup must use the OAuth browser flow"
        );
    }

    #[test]
    fn frontend_callable_major_provider_reauth_secret_handoff_is_rejected() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let mut request = generic_request();
        request.account_id = Some("acct_gmail".into());
        request.provider_kind = "gmail".into();
        request.auth_kind = "oauth2".into();
        request.secret = "refresh-token".into();
        request.imap_host = None;
        request.imap_port = None;
        request.imap_security = None;
        request.smtp_host = None;
        request.smtp_port = None;
        request.smtp_security = None;
        create_account_from_connection(&connection, &mut credentials, request)
            .expect("seed oauth account through backend helper");

        let error =
            reject_direct_major_provider_oauth_reauth_from_connection(&connection, "acct_gmail")
                .expect_err("direct reauth must use loopback");

        assert_eq!(
            error.reason,
            "Gmail and Microsoft reauthentication must use the OAuth browser flow"
        );
    }

    #[test]
    fn setup_denies_bad_ids_hosts_and_oversized_fields() {
        let mut bad_id = generic_request();
        bad_id.account_id = Some("../secret".into());
        assert!(validate_setup_request(bad_id)
            .expect_err("bad id")
            .reason
            .contains("account_id is invalid"));

        let mut bad_host = generic_request();
        bad_host.imap_host = Some("https://imap.example.com".into());
        assert!(validate_setup_request(bad_host)
            .expect_err("bad host")
            .reason
            .contains("imap_host is invalid"));

        let mut bad_host_label = generic_request();
        bad_host_label.smtp_host = Some("-smtp.example.com".into());
        assert!(validate_setup_request(bad_host_label)
            .expect_err("bad host label")
            .reason
            .contains("smtp_host is invalid"));

        let mut plaintext_transport = generic_request();
        plaintext_transport.smtp_security = Some("none".into());
        assert!(validate_setup_request(plaintext_transport)
            .expect_err("plaintext transport")
            .reason
            .contains("smtp_security must use TLS or STARTTLS"));

        let mut protocol_injection_secret = generic_request();
        protocol_injection_secret.secret = "secret\r\nA003 LOGOUT".into();
        assert_eq!(
            validate_setup_request(protocol_injection_secret)
                .expect_err("protocol injection secret")
                .reason,
            "secret contains unsupported control characters"
        );

        let mut oversized = generic_request();
        oversized.display_name = Some("x".repeat(MAX_DISPLAY_NAME_BYTES + 1));
        assert!(validate_setup_request(oversized)
            .expect_err("oversized display name")
            .reason
            .contains("display_name exceeds"));
    }

    #[test]
    fn reauth_writes_new_secret_and_keeps_metadata() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create account");

        reauth_account_from_connection(
            &connection,
            &mut credentials,
            AccountReauthRequest {
                account_id: "acct_generic".into(),
                secret: "replacement".into(),
            },
        )
        .expect("reauth account");

        assert_eq!(
            credentials
                .secrets
                .get(&("acct_generic".into(), CredentialSlot::GenericPassword))
                .map(String::as_str),
            Some("replacement"),
        );
    }

    #[test]
    fn remove_account_deletes_credentials_and_optionally_local_cache() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create account");
        update_account_settings_from_connection(
            &connection,
            AccountSettingsUpdateRequest {
                account_id: "acct_generic".into(),
                display_name: Some("Personal".into()),
                signature_html: "<p>Regards</p>".into(),
                local_preferences: AccountLocalPreferences {
                    include_in_unified_inbox: false,
                },
            },
        )
        .expect("persist local preferences");

        let response = remove_account_from_connection(
            &connection,
            &mut credentials,
            AccountRemovalRequest {
                account_id: "acct_generic".into(),
                delete_local_cache: true,
            },
        )
        .expect("remove account");

        assert!(response.credentials_deleted);
        assert!(response.access_cache_cleared);
        assert_eq!(
            response.provider_revocation,
            AccountProviderRevocationStatus::NotApplicable
        );
        assert!(response.local_cache_deleted);
        assert!(!response.local_cache_retained);
        assert!(credentials.deleted.contains("acct_generic"));
        assert!(!account_exists(&connection, "acct_generic").unwrap());
        let preference_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = ?1",
                params![account_local_preferences_key("acct_generic")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preference_count, 0);
    }

    #[test]
    fn remove_account_revokes_supported_oauth_and_reports_retained_cache() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let mut request = generic_request();
        request.account_id = Some("acct_gmail".into());
        request.provider_kind = "gmail".into();
        request.auth_kind = "oauth2".into();
        request.secret = "refresh-token".into();
        request.imap_host = None;
        request.imap_port = None;
        request.imap_security = None;
        request.smtp_host = None;
        request.smtp_port = None;
        request.smtp_security = None;
        create_account_from_connection(&connection, &mut credentials, request)
            .expect("create oauth account");
        let revocation_http = std::cell::RefCell::new(FakeOAuthRevocationHttp {
            status: 200,
            requests: Vec::new(),
        });

        let response = remove_account_from_connection_with_revocation_client(
            &connection,
            &mut credentials,
            &revocation_http,
            AccountRemovalRequest {
                account_id: "acct_gmail".into(),
                delete_local_cache: false,
            },
        )
        .expect("remove oauth account");

        assert!(response.credentials_deleted);
        assert!(response.access_cache_cleared);
        assert_eq!(
            response.provider_revocation,
            AccountProviderRevocationStatus::Revoked
        );
        assert!(!response.local_cache_deleted);
        assert!(response.local_cache_retained);
        assert_eq!(revocation_http.borrow().requests.len(), 1);
        assert_eq!(
            revocation_http.borrow().requests[0].0,
            "https://oauth2.googleapis.com/revoke"
        );
        assert_eq!(
            revocation_http.borrow().requests[0].1,
            vec![("token".to_string(), "refresh-token".to_string())]
        );
        let stored_status: String = connection
            .query_row(
                "SELECT auth_status FROM accounts WHERE id = 'acct_gmail'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_status, "removed");
    }

    #[test]
    fn remove_account_delete_failure_leaves_local_metadata_unchanged() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create account");
        credentials.fail_deletes = true;

        let error = remove_account_from_connection(
            &connection,
            &mut credentials,
            AccountRemovalRequest {
                account_id: "acct_generic".into(),
                delete_local_cache: true,
            },
        )
        .expect_err("credential delete failure blocks metadata mutation");

        assert!(error.reason.contains("credential delete failed"));
        assert!(account_exists(&connection, "acct_generic").unwrap());
        let stored_status: String = connection
            .query_row(
                "SELECT auth_status FROM accounts WHERE id = 'acct_generic'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_status, "connected");
    }

    #[test]
    fn remove_account_from_path_deletes_inbound_attachment_directory_on_cache_delete() {
        let (cache_path, app_data_dir) = migrated_file_database("remove-attachments");
        let mut credentials = FakeAccountCredentialStore::default();
        {
            let connection = Connection::open(&cache_path).unwrap();
            create_account_from_connection(&connection, &mut credentials, generic_request())
                .expect("create account");
        }
        let attachment_dir = app_data_dir
            .join(INBOUND_ATTACHMENT_DIR_NAME)
            .join("acct_generic")
            .join("folder_1")
            .join("message_1");
        fs::create_dir_all(&attachment_dir).unwrap();
        fs::write(attachment_dir.join("att_1.txt"), b"cached attachment").unwrap();

        remove_account_from_path(
            &cache_path,
            &mut credentials,
            AccountRemovalRequest {
                account_id: "acct_generic".into(),
                delete_local_cache: true,
            },
        )
        .expect("remove account");

        assert!(!app_data_dir
            .join(INBOUND_ATTACHMENT_DIR_NAME)
            .join("acct_generic")
            .exists());
        fs::remove_dir_all(app_data_dir).unwrap();
    }

    #[test]
    fn status_reports_missing_credentials_as_reconnect_needed() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create account");
        credentials.secrets.clear();

        let status = get_account_auth_status_from_connection(
            &connection,
            &mut credentials,
            AccountAuthStatusRequest {
                account_id: "acct_generic".into(),
            },
        )
        .expect("status");

        assert_eq!(status.auth_status, "reconnect_needed");
        assert!(!status.credential_present);
        let stored_status: String = connection
            .query_row(
                "SELECT auth_status FROM accounts WHERE id = 'acct_generic'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_status, "reconnect_needed");
    }

    #[test]
    fn update_account_settings_persists_display_name_signature_and_local_preferences() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create account");

        let response = update_account_settings_from_connection(
            &connection,
            AccountSettingsUpdateRequest {
                account_id: "acct_generic".into(),
                display_name: Some("  Updated Personal  ".into()),
                signature_html: r#"<p onclick="alert(1)">Regards <a href="https://example.com">Me</a><img src="https://tracker.example/pixel.gif"></p>"#.into(),
                local_preferences: AccountLocalPreferences {
                    include_in_unified_inbox: false,
                },
            },
        )
        .expect("persist account settings");

        assert_eq!(
            response.account.display_name.as_deref(),
            Some("Updated Personal")
        );
        assert_eq!(
            response.local_preferences,
            AccountLocalPreferences {
                include_in_unified_inbox: false
            }
        );
        let signature_html = response.signature_html.expect("sanitized signature");
        assert!(signature_html.contains("Regards"));
        assert!(signature_html.contains("https://example.com"));
        assert!(!signature_html.contains("onclick"));
        assert!(!signature_html.contains("<img"));
        assert!(!signature_html.contains("tracker.example"));

        let stored: (Option<String>, Option<String>, String) = connection
            .query_row(
                "SELECT accounts.display_name, accounts.signature_html, settings.value_json
                 FROM accounts
                 JOIN settings ON settings.key = ?2
                 WHERE accounts.id = ?1",
                params![
                    "acct_generic",
                    account_local_preferences_key("acct_generic")
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored.0.as_deref(), Some("Updated Personal"));
        assert_eq!(stored.1.as_deref(), Some(signature_html.as_str()));
        assert_eq!(
            serde_json::from_str::<AccountLocalPreferences>(&stored.2).unwrap(),
            AccountLocalPreferences {
                include_in_unified_inbox: false
            }
        );
    }

    #[test]
    fn update_account_settings_rejects_invalid_unknown_removed_and_oversized_values() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        create_account_from_connection(&connection, &mut credentials, generic_request())
            .expect("create account");

        let mut invalid_id = settings_request();
        invalid_id.account_id = "../secret".into();
        assert!(
            update_account_settings_from_connection(&connection, invalid_id)
                .expect_err("invalid account id rejected")
                .reason
                .contains("account_id is invalid")
        );

        let mut unknown_id = settings_request();
        unknown_id.account_id = "acct_missing".into();
        assert_eq!(
            update_account_settings_from_connection(&connection, unknown_id)
                .expect_err("unknown account rejected")
                .reason,
            "account was not found"
        );

        let mut oversized_display = settings_request();
        oversized_display.display_name = Some("x".repeat(MAX_DISPLAY_NAME_BYTES + 1));
        assert!(
            update_account_settings_from_connection(&connection, oversized_display)
                .expect_err("oversized display name rejected")
                .reason
                .contains("display_name exceeds")
        );

        let mut oversized_signature = settings_request();
        oversized_signature.signature_html = "x".repeat(MAX_TEXT_FIELD_BYTES + 1);
        assert!(
            update_account_settings_from_connection(&connection, oversized_signature)
                .expect_err("oversized signature rejected")
                .reason
                .contains("signature_html exceeds")
        );

        remove_account_from_connection(
            &connection,
            &mut credentials,
            AccountRemovalRequest {
                account_id: "acct_generic".into(),
                delete_local_cache: false,
            },
        )
        .expect("soft remove account");
        assert_eq!(
            update_account_settings_from_connection(&connection, settings_request())
                .expect_err("removed account rejected")
                .reason,
            "account was not found"
        );
    }

    #[test]
    fn connection_test_validates_without_persistence() {
        let response =
            test_account_connection_request(generic_request()).expect("configuration validates");

        assert_eq!(response.status, "configuration-ready");
        assert_eq!(response.provider_kind, "generic_imap_smtp");
    }

    #[test]
    fn live_connection_test_attempts_generic_imap_and_smtp_without_persistence() {
        let mut tester = FakeGenericConnectionTester::default();

        let response = test_account_connection_request_with_tester(generic_request(), &mut tester)
            .expect("generic connection validates");

        assert_eq!(response.status, "connection-ready");
        assert_eq!(response.provider_kind, "generic_imap_smtp");
        assert_eq!(tester.calls.len(), 1);
        assert_eq!(tester.calls[0].username, "person@example.com");
        assert_eq!(tester.calls[0].imap_host, "imap.example.com");
        assert_eq!(tester.calls[0].imap_port, 993);
        assert_eq!(tester.calls[0].imap_security, MailTransportSecurity::Tls);
        assert_eq!(tester.calls[0].smtp_host, "smtp.example.com");
        assert_eq!(tester.calls[0].smtp_port, 587);
        assert_eq!(
            tester.calls[0].smtp_security,
            MailTransportSecurity::StartTls
        );
    }

    #[test]
    fn live_connection_test_rejects_major_provider_secret_handoff_before_network() {
        let mut request = generic_request();
        request.account_id = Some("acct_gmail".into());
        request.provider_kind = "gmail".into();
        request.auth_kind = "oauth2".into();
        request.secret = "refresh-token".into();
        let mut tester = FakeGenericConnectionTester::default();

        let error = test_account_connection_request_with_tester(request, &mut tester)
            .expect_err("major provider setup test must use loopback");

        assert_eq!(
            error.reason,
            "Gmail and Microsoft account setup must use the OAuth browser flow"
        );
        assert!(tester.calls.is_empty());
    }

    #[test]
    fn create_generic_account_runs_live_connection_test_before_metadata_insert() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let mut tester = FakeGenericConnectionTester::default();

        let response = create_account_from_connection_with_connection_tester(
            &connection,
            &mut credentials,
            &mut tester,
            generic_request(),
        )
        .expect("create account after connection test");

        assert_eq!(response.account.id, "acct_generic");
        assert_eq!(tester.calls.len(), 1);
        assert_eq!(
            credentials
                .secrets
                .get(&("acct_generic".into(), CredentialSlot::GenericPassword))
                .map(String::as_str),
            Some("app-password"),
        );
        let account_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(account_count, 1);
    }

    #[test]
    fn create_generic_account_does_not_store_secret_when_live_login_fails() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let mut tester = FakeGenericConnectionTester {
            error: Some(AccountConnectionTestError::AuthenticationRejected { protocol: "IMAP" }),
            ..Default::default()
        };

        let error = create_account_from_connection_with_connection_tester(
            &connection,
            &mut credentials,
            &mut tester,
            generic_request(),
        )
        .expect_err("connection failure rejects setup");

        assert_eq!(error.reason, "IMAP authentication failed");
        assert_eq!(tester.calls.len(), 1);
        assert!(!credentials.deleted.contains("acct_generic"));
        assert!(!credentials
            .secrets
            .contains_key(&("acct_generic".into(), CredentialSlot::GenericPassword)));
        let account_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(account_count, 0);
    }

    fn settings_request() -> AccountSettingsUpdateRequest {
        AccountSettingsUpdateRequest {
            account_id: "acct_generic".into(),
            display_name: Some("Personal".into()),
            signature_html: "<p>Regards</p>".into(),
            local_preferences: AccountLocalPreferences::default(),
        }
    }
}
