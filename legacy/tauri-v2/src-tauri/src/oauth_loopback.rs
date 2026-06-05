use std::{
    error::Error,
    fmt,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use url::Url;

use crate::{
    account_management::{
        create_account_from_connection, mail_cache_path, reauth_account_from_connection,
        AccountCommandError, AccountCredentialStore, AccountReauthRequest, AccountSetupRequest,
        AccountSetupResponse,
    },
    auth::ProviderKind,
    credentials::WindowsCredentialManager,
    ipc_validation::{
        validate_bounded_text, validate_entity_id, validate_payload_size, IpcValidationError,
        MAX_TEXT_FIELD_BYTES,
    },
    oauth::{
        build_authorization_request, exchange_authorization_code, OAuthClientConfig, OAuthError,
        OAuthHttpClient, ReqwestOAuthHttpClient,
    },
    schema::{apply_migrations, SchemaMigrationError},
    sync_scheduler::{
        load_account_oauth_client_config, store_account_oauth_client_config,
        BackgroundSyncScheduler,
    },
    token_lifecycle::SecretString,
};

const OAUTH_CALLBACK_PATH: &str = "/";
const DEFAULT_OAUTH_TIMEOUT_SECONDS: u64 = 180;
const MAX_OAUTH_TIMEOUT_SECONDS: u64 = 600;
const CALLBACK_READ_TIMEOUT_SECONDS: u64 = 5;
const MAX_CALLBACK_TARGET_BYTES: usize = 4096;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OAuthAccountSetupRequest {
    pub account_id: Option<String>,
    pub provider_kind: String,
    pub display_email: String,
    pub display_name: Option<String>,
    pub username: String,
    pub client_id: String,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OAuthAccountReauthRequest {
    pub account_id: String,
    pub client_id: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OAuthAccountCommandError {
    pub reason: String,
}

impl OAuthAccountCommandError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for OAuthAccountCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for OAuthAccountCommandError {}

impl From<IpcValidationError> for OAuthAccountCommandError {
    fn from(value: IpcValidationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<AccountCommandError> for OAuthAccountCommandError {
    fn from(value: AccountCommandError) -> Self {
        Self::new(value.reason)
    }
}

impl From<SchemaMigrationError> for OAuthAccountCommandError {
    fn from(value: SchemaMigrationError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<rusqlite::Error> for OAuthAccountCommandError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<std::io::Error> for OAuthAccountCommandError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<tauri::Error> for OAuthAccountCommandError {
    fn from(value: tauri::Error) -> Self {
        Self::new(value.to_string())
    }
}

#[tauri::command]
pub async fn start_oauth_account_setup(
    app: tauri::AppHandle,
    request: OAuthAccountSetupRequest,
) -> Result<AccountSetupResponse, OAuthAccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    let response = tauri::async_runtime::spawn_blocking({
        let cache_path = cache_path.clone();
        move || {
            let receiver = TcpOAuthLoopbackReceiver::bind()?;
            let browser = SystemBrowser;
            let http = ReqwestOAuthHttpClient::default();
            let mut credentials = WindowsCredentialManager;

            start_oauth_account_setup_from_path(
                &cache_path,
                &mut credentials,
                &receiver,
                &browser,
                &http,
                request,
                now_seconds(),
            )
        }
    })
    .await
    .map_err(|error| OAuthAccountCommandError::new(error.to_string()))??;
    let _ = scheduler.refresh_account(cache_path, &response.account.id);
    Ok(response)
}

#[tauri::command]
pub async fn start_oauth_account_reauth(
    app: tauri::AppHandle,
    request: OAuthAccountReauthRequest,
) -> Result<AccountSetupResponse, OAuthAccountCommandError> {
    let cache_path = mail_cache_path(&app)?;
    let scheduler = app.state::<BackgroundSyncScheduler>().inner().clone();
    let response = tauri::async_runtime::spawn_blocking({
        let cache_path = cache_path.clone();
        move || {
            let receiver = TcpOAuthLoopbackReceiver::bind()?;
            let browser = SystemBrowser;
            let http = ReqwestOAuthHttpClient::default();
            let mut credentials = WindowsCredentialManager;

            start_oauth_account_reauth_from_path(
                &cache_path,
                &mut credentials,
                &receiver,
                &browser,
                &http,
                request,
                now_seconds(),
            )
        }
    })
    .await
    .map_err(|error| OAuthAccountCommandError::new(error.to_string()))??;
    let _ = scheduler.refresh_account(cache_path, &response.account.id);
    Ok(response)
}

pub trait OAuthLoopbackReceiver {
    fn redirect_uri(&self) -> &str;
    fn receive_callback(&self, timeout: Duration) -> Result<String, OAuthAccountCommandError>;
}

pub trait OAuthBrowser {
    fn open_authorization_url(&self, url: &str) -> Result<(), OAuthAccountCommandError>;
}

pub fn start_oauth_account_setup_from_path(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    receiver: &impl OAuthLoopbackReceiver,
    browser: &impl OAuthBrowser,
    http: &impl OAuthHttpClient,
    request: OAuthAccountSetupRequest,
    now: u64,
) -> Result<AccountSetupResponse, OAuthAccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    start_oauth_account_setup_from_connection(
        &connection,
        credentials,
        receiver,
        browser,
        http,
        request,
        now,
    )
}

pub fn start_oauth_account_setup_from_connection(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    receiver: &impl OAuthLoopbackReceiver,
    browser: &impl OAuthBrowser,
    http: &impl OAuthHttpClient,
    request: OAuthAccountSetupRequest,
    now: u64,
) -> Result<AccountSetupResponse, OAuthAccountCommandError> {
    let validated = validate_oauth_setup_request(request)?;
    let refresh_token = run_loopback_token_flow(
        receiver,
        browser,
        http,
        validated.provider,
        &validated.client_id,
        validated.timeout,
        now,
    )?;

    let response = create_account_from_connection(
        connection,
        credentials,
        AccountSetupRequest {
            account_id: validated.account_id,
            provider_kind: provider_schema_value(validated.provider).into(),
            display_email: validated.display_email,
            display_name: validated.display_name,
            username: validated.username,
            auth_kind: "oauth2".into(),
            secret: refresh_token.expose_secret().to_string(),
            imap_host: None,
            imap_port: None,
            imap_security: None,
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
        },
    )
    .map_err(OAuthAccountCommandError::from)?;
    store_account_oauth_client_config(
        connection,
        &response.account.id,
        &validated.client_id,
        receiver.redirect_uri(),
    )
    .map_err(|error| OAuthAccountCommandError::new(error.reason))?;
    Ok(response)
}

pub fn start_oauth_account_reauth_from_path(
    cache_path: &Path,
    credentials: &mut impl AccountCredentialStore,
    receiver: &impl OAuthLoopbackReceiver,
    browser: &impl OAuthBrowser,
    http: &impl OAuthHttpClient,
    request: OAuthAccountReauthRequest,
    now: u64,
) -> Result<AccountSetupResponse, OAuthAccountCommandError> {
    let mut connection = Connection::open(cache_path)?;
    apply_migrations(&mut connection)?;
    start_oauth_account_reauth_from_connection(
        &connection,
        credentials,
        receiver,
        browser,
        http,
        request,
        now,
    )
}

pub fn start_oauth_account_reauth_from_connection(
    connection: &Connection,
    credentials: &mut impl AccountCredentialStore,
    receiver: &impl OAuthLoopbackReceiver,
    browser: &impl OAuthBrowser,
    http: &impl OAuthHttpClient,
    request: OAuthAccountReauthRequest,
    now: u64,
) -> Result<AccountSetupResponse, OAuthAccountCommandError> {
    validate_entity_id("account_id", &request.account_id)?;
    let timeout = validate_timeout(request.timeout_seconds)?;
    let account = load_oauth_reauth_context(connection, &request.account_id)?;
    let client_id = resolve_reauth_client_id(connection, &request.account_id, request.client_id)?;
    let refresh_token = run_loopback_token_flow(
        receiver,
        browser,
        http,
        account.provider,
        &client_id,
        timeout,
        now,
    )?;

    let response = reauth_account_from_connection(
        connection,
        credentials,
        AccountReauthRequest {
            account_id: account.account_id.clone(),
            secret: refresh_token.expose_secret().to_string(),
        },
    )
    .map_err(OAuthAccountCommandError::from)?;
    store_account_oauth_client_config(
        connection,
        &account.account_id,
        &client_id,
        receiver.redirect_uri(),
    )
    .map_err(|error| OAuthAccountCommandError::new(error.reason))?;
    Ok(response)
}

fn resolve_reauth_client_id(
    connection: &Connection,
    account_id: &str,
    requested_client_id: Option<String>,
) -> Result<String, OAuthAccountCommandError> {
    if let Some(client_id) = requested_client_id {
        if !client_id.trim().is_empty() {
            return normalize_client_id(client_id);
        }
    }

    let stored = load_account_oauth_client_config(connection, account_id)
        .map_err(|error| OAuthAccountCommandError::new(error.reason))?
        .ok_or_else(|| {
            OAuthAccountCommandError::new(
                "OAuth client configuration is missing; enter the OAuth client ID in settings",
            )
        })?;

    normalize_client_id(stored.client_id)
}

fn run_loopback_token_flow(
    receiver: &impl OAuthLoopbackReceiver,
    browser: &impl OAuthBrowser,
    http: &impl OAuthHttpClient,
    provider: ProviderKind,
    client_id: &str,
    timeout: Duration,
    now: u64,
) -> Result<SecretString, OAuthAccountCommandError> {
    let config = OAuthClientConfig::new(provider, client_id, receiver.redirect_uri(), None)
        .map_err(map_oauth_error)?;
    let authorization = build_authorization_request(&config).map_err(map_oauth_error)?;

    browser.open_authorization_url(&authorization.authorization_url)?;

    let callback_url = receiver.receive_callback(timeout)?;
    let callback = parse_callback(&callback_url)?;
    if callback.state != authorization.state {
        return Err(OAuthAccountCommandError::new(
            "OAuth callback state did not match",
        ));
    }
    if let Some(error) = callback.error {
        return Err(provider_callback_error(&error));
    }

    let code = callback
        .code
        .ok_or_else(|| OAuthAccountCommandError::new("OAuth callback omitted code"))?;
    let tokens = exchange_authorization_code(
        http,
        &config,
        &SecretString::new(code),
        &authorization.pkce_verifier,
        now,
    )
    .map_err(map_oauth_error)?;

    tokens.refresh_token.ok_or_else(|| {
        OAuthAccountCommandError::new("OAuth token response did not include a refresh token")
    })
}

#[derive(Debug, Clone)]
struct ValidatedOAuthSetupRequest {
    account_id: Option<String>,
    provider: ProviderKind,
    display_email: String,
    display_name: Option<String>,
    username: String,
    client_id: String,
    timeout: Duration,
}

fn validate_oauth_setup_request(
    request: OAuthAccountSetupRequest,
) -> Result<ValidatedOAuthSetupRequest, OAuthAccountCommandError> {
    let provider = provider_from_oauth_request(&request.provider_kind)?;
    let account_id = match request.account_id {
        Some(account_id) => {
            validate_entity_id("account_id", &account_id)?;
            Some(account_id)
        }
        None => None,
    };

    Ok(ValidatedOAuthSetupRequest {
        account_id,
        provider,
        display_email: normalize_email("display_email", request.display_email)?,
        display_name: normalize_optional_text("display_name", request.display_name)?,
        username: normalize_email("username", request.username)?,
        client_id: normalize_client_id(request.client_id)?,
        timeout: validate_timeout(request.timeout_seconds)?,
    })
}

#[derive(Debug, Clone)]
struct OAuthReauthContext {
    account_id: String,
    provider: ProviderKind,
}

fn load_oauth_reauth_context(
    connection: &Connection,
    account_id: &str,
) -> Result<OAuthReauthContext, OAuthAccountCommandError> {
    let account = connection
        .query_row(
            "SELECT id, provider_kind, auth_kind
             FROM accounts
             WHERE id = ?1
               AND removed_at IS NULL
               AND auth_status != 'removed'",
            params![account_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| OAuthAccountCommandError::new("account was not found"))?;

    if account.2 != "oauth2" {
        return Err(OAuthAccountCommandError::new(
            "account does not use OAuth2 authentication",
        ));
    }

    Ok(OAuthReauthContext {
        account_id: account.0,
        provider: provider_from_oauth_request(&account.1)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OAuthCallback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

fn parse_callback(value: &str) -> Result<OAuthCallback, OAuthAccountCommandError> {
    validate_payload_size(
        "oauth_callback",
        value.as_bytes(),
        MAX_CALLBACK_TARGET_BYTES,
    )?;
    let url = Url::parse(value)
        .map_err(|_| OAuthAccountCommandError::new("OAuth callback URL could not be parsed"))?;

    if url.path() != OAUTH_CALLBACK_PATH {
        return Err(OAuthAccountCommandError::new(
            "OAuth callback path is invalid",
        ));
    }

    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "state" => state = Some(value.into_owned()),
            "code" => code = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    let state =
        state.ok_or_else(|| OAuthAccountCommandError::new("OAuth callback omitted state"))?;
    validate_bounded_text("oauth_state", &state, MAX_TEXT_FIELD_BYTES)?;
    if let Some(code) = &code {
        validate_bounded_text("authorization_code", code, MAX_TEXT_FIELD_BYTES)?;
    }
    if let Some(error) = &error {
        validate_bounded_text("oauth_error", error, 256)?;
    }

    Ok(OAuthCallback { state, code, error })
}

pub struct TcpOAuthLoopbackReceiver {
    listener: TcpListener,
    redirect_uri: String,
}

impl TcpOAuthLoopbackReceiver {
    pub fn bind() -> Result<Self, OAuthAccountCommandError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();

        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{port}"),
        })
    }
}

impl OAuthLoopbackReceiver for TcpOAuthLoopbackReceiver {
    fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    fn receive_callback(&self, timeout: Duration) -> Result<String, OAuthAccountCommandError> {
        let deadline = Instant::now() + timeout;

        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(
                        CALLBACK_READ_TIMEOUT_SECONDS,
                    )))?;
                    let mut buffer = [0_u8; 8192];
                    let bytes_read = stream.read(&mut buffer)?;
                    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                    let Some(target) = parse_http_get_target(&request) else {
                        write_loopback_response(&mut stream, 400, "Invalid OAuth response")?;
                        continue;
                    };

                    if !is_callback_target(target) {
                        write_loopback_response(&mut stream, 404, "Not found")?;
                        continue;
                    }

                    validate_payload_size(
                        "oauth_callback",
                        target.as_bytes(),
                        MAX_CALLBACK_TARGET_BYTES,
                    )?;
                    write_loopback_response(
                        &mut stream,
                        200,
                        "Authorization received. Return to Komorebi.",
                    )?;
                    let callback_base = self.redirect_uri.as_str();
                    return Ok(format!("{callback_base}{target}"));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(OAuthAccountCommandError::new("OAuth callback timed out"));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn parse_http_get_target(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;

    if method != "GET" || !version.starts_with("HTTP/") {
        return None;
    }

    Some(target)
}

fn is_callback_target(target: &str) -> bool {
    target == OAUTH_CALLBACK_PATH || target.starts_with(&format!("{OAUTH_CALLBACK_PATH}?"))
}

fn write_loopback_response(
    stream: &mut impl Write,
    status: u16,
    body: &str,
) -> Result<(), std::io::Error> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Komorebi OAuth</title><p>{body}</p>"
    );
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    )
}

pub struct SystemBrowser;

impl OAuthBrowser for SystemBrowser {
    fn open_authorization_url(&self, url: &str) -> Result<(), OAuthAccountCommandError> {
        open_system_browser(url)
    }
}

#[cfg(windows)]
fn open_system_browser(url: &str) -> Result<(), OAuthAccountCommandError> {
    use std::ptr;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    validate_bounded_text("authorization_url", url, 4096)?;
    let operation = wide_null("open");
    let file = wide_null(url);
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    if result as isize <= 32 {
        return Err(OAuthAccountCommandError::new(
            "system browser could not be opened",
        ));
    }

    Ok(())
}

#[cfg(not(windows))]
fn open_system_browser(_url: &str) -> Result<(), OAuthAccountCommandError> {
    Err(OAuthAccountCommandError::new(
        "system browser opening is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn provider_from_oauth_request(value: &str) -> Result<ProviderKind, OAuthAccountCommandError> {
    match value {
        "gmail" => Ok(ProviderKind::Gmail),
        "microsoft" => Ok(ProviderKind::Microsoft),
        "generic_imap_smtp" => Err(OAuthAccountCommandError::new(
            "generic IMAP/SMTP does not support OAuth loopback setup",
        )),
        _ => Err(OAuthAccountCommandError::new(
            "provider_kind is unsupported",
        )),
    }
}

fn provider_schema_value(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Gmail => "gmail",
        ProviderKind::Microsoft => "microsoft",
        ProviderKind::GenericImapSmtp => "generic_imap_smtp",
    }
}

fn normalize_client_id(value: String) -> Result<String, OAuthAccountCommandError> {
    let value = value.trim().to_string();
    validate_bounded_text("oauth_client_id", &value, MAX_TEXT_FIELD_BYTES)?;
    Ok(value)
}

fn normalize_email(field: &'static str, value: String) -> Result<String, OAuthAccountCommandError> {
    let value = value.trim().to_string();
    validate_bounded_text(field, &value, 320)?;
    if value.contains(char::is_whitespace)
        || value.contains('<')
        || value.contains('>')
        || value.matches('@').count() != 1
        || value.starts_with('@')
        || value.ends_with('@')
    {
        return Err(OAuthAccountCommandError::new(format!("{field} is invalid")));
    }

    Ok(value)
}

fn normalize_optional_text(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<String>, OAuthAccountCommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validate_payload_size(field, value.as_bytes(), 256)?;
    Ok(Some(value.to_string()))
}

fn validate_timeout(value: Option<u64>) -> Result<Duration, OAuthAccountCommandError> {
    let seconds = value.unwrap_or(DEFAULT_OAUTH_TIMEOUT_SECONDS);
    if seconds == 0 || seconds > MAX_OAUTH_TIMEOUT_SECONDS {
        return Err(OAuthAccountCommandError::new(
            "oauth_timeout_seconds must be between 1 and 600",
        ));
    }

    Ok(Duration::from_secs(seconds))
}

fn provider_callback_error(error: &str) -> OAuthAccountCommandError {
    let code = safe_provider_error_code(error);
    if code == "access_denied" {
        return OAuthAccountCommandError::new("OAuth authorization was cancelled");
    }

    OAuthAccountCommandError::new(format!("OAuth provider returned error: {code}"))
}

fn safe_provider_error_code(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .take(64)
        .collect::<String>();

    if sanitized.is_empty() {
        "provider_error".into()
    } else {
        sanitized
    }
}

fn map_oauth_error(error: OAuthError) -> OAuthAccountCommandError {
    match error {
        OAuthError::InvalidGrant => {
            OAuthAccountCommandError::new("OAuth authorization grant was rejected")
        }
        OAuthError::ProviderRejected { error } => OAuthAccountCommandError::new(format!(
            "OAuth provider returned error: {}",
            safe_provider_error_code(&error)
        )),
        OAuthError::HttpRequestFailed | OAuthError::HttpStatus { .. } => {
            OAuthAccountCommandError::new("OAuth token exchange failed")
        }
        OAuthError::MissingAccessToken
        | OAuthError::MissingExpiry
        | OAuthError::InvalidTokenResponse => {
            OAuthAccountCommandError::new("OAuth token response was invalid")
        }
        OAuthError::UnsupportedProvider { .. } => {
            OAuthAccountCommandError::new("OAuth provider is unsupported")
        }
        OAuthError::IpcValidation(error) => error.into(),
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        credentials::{CredentialError, CredentialSlot},
        oauth::{OAuthHttpError, OAuthHttpResponse},
        schema::apply_migrations,
    };
    use std::{
        cell::RefCell,
        collections::{HashMap, HashSet},
        io::{Read, Write},
        net::TcpStream,
        rc::Rc,
        sync::mpsc,
        thread,
    };

    const NOW: u64 = 1_800_000_000;

    #[derive(Default)]
    struct FakeAccountCredentialStore {
        secrets: HashMap<(String, CredentialSlot), String>,
        deleted: HashSet<String>,
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
            self.secrets.insert(
                (account_id.to_string(), slot),
                secret.expose_secret().to_string(),
            );
            Ok(())
        }

        fn delete_account_secrets(&mut self, account_id: &str) -> Result<(), CredentialError> {
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

    #[derive(Clone)]
    enum FakeCallback {
        Code,
        StateMismatch,
        ProviderError(String),
    }

    struct FakeReceiver {
        redirect_uri: String,
        state: Rc<RefCell<Option<String>>>,
        callback: FakeCallback,
    }

    impl OAuthLoopbackReceiver for FakeReceiver {
        fn redirect_uri(&self) -> &str {
            &self.redirect_uri
        }

        fn receive_callback(&self, _timeout: Duration) -> Result<String, OAuthAccountCommandError> {
            let state = match self.callback {
                FakeCallback::StateMismatch => "bad-state".to_string(),
                _ => self
                    .state
                    .borrow()
                    .clone()
                    .expect("browser opener captured OAuth state"),
            };
            let query = match &self.callback {
                FakeCallback::Code | FakeCallback::StateMismatch => {
                    format!("code=auth-code&state={state}")
                }
                FakeCallback::ProviderError(error) => {
                    format!("error={error}&state={state}")
                }
            };

            Ok(format!("{}?{query}", self.redirect_uri))
        }
    }

    struct TimeoutReceiver {
        redirect_uri: String,
    }

    impl OAuthLoopbackReceiver for TimeoutReceiver {
        fn redirect_uri(&self) -> &str {
            &self.redirect_uri
        }

        fn receive_callback(&self, _timeout: Duration) -> Result<String, OAuthAccountCommandError> {
            Err(OAuthAccountCommandError::new("OAuth callback timed out"))
        }
    }

    #[derive(Clone)]
    struct FakeBrowser {
        state: Rc<RefCell<Option<String>>>,
        opened_urls: Rc<RefCell<Vec<String>>>,
    }

    impl OAuthBrowser for FakeBrowser {
        fn open_authorization_url(&self, url: &str) -> Result<(), OAuthAccountCommandError> {
            let parsed = Url::parse(url).expect("authorization url parses");
            let state = parsed
                .query_pairs()
                .find(|(name, _)| name == "state")
                .map(|(_, value)| value.into_owned())
                .expect("authorization url includes state");
            *self.state.borrow_mut() = Some(state);
            self.opened_urls.borrow_mut().push(url.to_string());
            Ok(())
        }
    }

    struct FakeOAuthHttpClient {
        response: RefCell<Result<OAuthHttpResponse, OAuthHttpError>>,
        requests: RefCell<Vec<Vec<(String, String)>>>,
    }

    impl FakeOAuthHttpClient {
        fn with_json(status: u16, body: &str) -> Self {
            Self {
                response: RefCell::new(Ok(OAuthHttpResponse {
                    status,
                    body: body.to_string(),
                })),
                requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl OAuthHttpClient for FakeOAuthHttpClient {
        fn post_form(
            &self,
            _url: &str,
            fields: &[(&str, String)],
        ) -> Result<OAuthHttpResponse, OAuthHttpError> {
            self.requests.borrow_mut().push(
                fields
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), value.clone()))
                    .collect(),
            );
            self.response.borrow_mut().clone()
        }
    }

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().expect("open db");
        apply_migrations(&mut connection).expect("migrate db");
        connection
    }

    fn setup_request(provider_kind: &str) -> OAuthAccountSetupRequest {
        OAuthAccountSetupRequest {
            account_id: Some(format!("acct_{provider_kind}")),
            provider_kind: provider_kind.into(),
            display_email: format!("{provider_kind}@example.com"),
            display_name: Some("OAuth Account".into()),
            username: format!("{provider_kind}@example.com"),
            client_id: format!("{provider_kind}-client-id"),
            timeout_seconds: Some(10),
        }
    }

    fn fake_flow(callback: FakeCallback) -> (FakeReceiver, FakeBrowser, FakeOAuthHttpClient) {
        let state = Rc::new(RefCell::new(None));
        let opened_urls = Rc::new(RefCell::new(Vec::new()));
        (
            FakeReceiver {
                redirect_uri: "http://127.0.0.1:49152".into(),
                state: Rc::clone(&state),
                callback,
            },
            FakeBrowser { state, opened_urls },
            FakeOAuthHttpClient::with_json(
                200,
                r#"{"access_token":"access-token","refresh_token":"refresh-token","expires_in":3600}"#,
            ),
        )
    }

    #[test]
    fn oauth_setup_opens_browser_exchanges_code_and_stores_refresh_token_only_in_backend() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let (receiver, browser, http) = fake_flow(FakeCallback::Code);

        let response = start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("gmail"),
            NOW,
        )
        .expect("oauth setup succeeds");

        assert_eq!(response.account.id, "acct_gmail");
        assert_eq!(response.account.provider_kind, "gmail");
        assert_eq!(response.account.auth_kind, "oauth2");
        assert_eq!(
            credentials
                .secrets
                .get(&("acct_gmail".into(), CredentialSlot::OAuthRefresh))
                .map(String::as_str),
            Some("refresh-token"),
        );
        let opened = browser.opened_urls.borrow();
        assert_eq!(opened.len(), 1);
        assert!(!opened[0].contains("refresh-token"));
        assert!(!opened[0].contains("access-token"));
        assert!(http.requests.borrow()[0].contains(&("code".into(), "auth-code".into())));
    }

    #[test]
    fn oauth_reauth_rotates_refresh_token_for_existing_oauth_account() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("microsoft"),
            NOW,
        )
        .expect("seed oauth account");

        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        start_oauth_account_reauth_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            OAuthAccountReauthRequest {
                account_id: "acct_microsoft".into(),
                client_id: Some("microsoft-client-id".into()),
                timeout_seconds: Some(10),
            },
            NOW,
        )
        .expect("reauth succeeds");

        assert_eq!(
            credentials
                .secrets
                .get(&("acct_microsoft".into(), CredentialSlot::OAuthRefresh))
                .map(String::as_str),
            Some("refresh-token"),
        );
    }

    #[test]
    fn oauth_reauth_can_reuse_stored_client_id_for_status_reconnect() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("gmail"),
            NOW,
        )
        .expect("seed oauth account");

        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        start_oauth_account_reauth_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            OAuthAccountReauthRequest {
                account_id: "acct_gmail".into(),
                client_id: None,
                timeout_seconds: Some(10),
            },
            NOW,
        )
        .expect("reauth succeeds with stored client id");

        assert_eq!(browser.opened_urls.borrow().len(), 1);
        assert!(browser.opened_urls.borrow()[0].contains("gmail-client-id"));
    }

    #[test]
    fn oauth_setup_rejects_generic_provider_before_opening_browser() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        let error = start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("generic_imap_smtp"),
            NOW,
        )
        .expect_err("generic oauth is rejected");

        assert!(error.reason.contains("generic IMAP/SMTP"));
        assert!(browser.opened_urls.borrow().is_empty());
        assert!(credentials.secrets.is_empty());
    }

    #[test]
    fn oauth_callback_denial_state_mismatch_timeout_and_invalid_grant_do_not_store_credentials() {
        let cases = [
            (
                FakeCallback::ProviderError("access_denied".into()),
                "OAuth authorization was cancelled",
            ),
            (
                FakeCallback::StateMismatch,
                "OAuth callback state did not match",
            ),
        ];

        for (callback, expected) in cases {
            let connection = migrated_connection();
            let mut credentials = FakeAccountCredentialStore::default();
            let (receiver, browser, http) = fake_flow(callback);
            let error = start_oauth_account_setup_from_connection(
                &connection,
                &mut credentials,
                &receiver,
                &browser,
                &http,
                setup_request("gmail"),
                NOW,
            )
            .expect_err("callback denial is rejected");

            assert_eq!(error.reason, expected);
            assert!(credentials.secrets.is_empty());
        }

        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let receiver = TimeoutReceiver {
            redirect_uri: "http://127.0.0.1:49152".into(),
        };
        let state = Rc::new(RefCell::new(None));
        let browser = FakeBrowser {
            state,
            opened_urls: Rc::new(RefCell::new(Vec::new())),
        };
        let http = FakeOAuthHttpClient::with_json(
            200,
            r#"{"access_token":"access-token","refresh_token":"refresh-token","expires_in":3600}"#,
        );
        let timeout_error = start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("gmail"),
            NOW,
        )
        .expect_err("timeout is rejected");
        assert_eq!(timeout_error.reason, "OAuth callback timed out");
        assert!(credentials.secrets.is_empty());

        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        *http.response.borrow_mut() = Ok(OAuthHttpResponse {
            status: 400,
            body: r#"{"error":"invalid_grant"}"#.into(),
        });
        let invalid_grant_error = start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("gmail"),
            NOW,
        )
        .expect_err("invalid grant is rejected");
        assert_eq!(
            invalid_grant_error.reason,
            "OAuth authorization grant was rejected"
        );
        assert!(credentials.secrets.is_empty());
    }

    #[test]
    fn oauth_missing_refresh_token_is_rejected_without_metadata_insert() {
        let connection = migrated_connection();
        let mut credentials = FakeAccountCredentialStore::default();
        let (receiver, browser, http) = fake_flow(FakeCallback::Code);
        *http.response.borrow_mut() = Ok(OAuthHttpResponse {
            status: 200,
            body: r#"{"access_token":"access-token","expires_in":3600}"#.into(),
        });

        let error = start_oauth_account_setup_from_connection(
            &connection,
            &mut credentials,
            &receiver,
            &browser,
            &http,
            setup_request("gmail"),
            NOW,
        )
        .expect_err("refresh token is required");

        assert_eq!(
            error.reason,
            "OAuth token response did not include a refresh token"
        );
        assert!(credentials.secrets.is_empty());
    }

    #[test]
    fn tcp_loopback_receiver_accepts_only_callback_gets_and_returns_callback_url() {
        let receiver = TcpOAuthLoopbackReceiver::bind().expect("bind loopback receiver");
        let redirect_uri = receiver.redirect_uri().to_string();
        let (sender, receiver_done) = mpsc::channel();

        thread::spawn(move || {
            let redirect_url = Url::parse(&redirect_uri).expect("redirect uri parses");
            let address = format!(
                "{}:{}",
                redirect_url.host_str().expect("redirect host"),
                redirect_url.port().expect("redirect port")
            );
            let mut bad_stream =
                TcpStream::connect(&address).expect("connect loopback for bad request");
            bad_stream
                .write_all(
                    b"GET /oauth/callback-extra?code=abc&state=state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
                )
                .expect("write bad request");
            let mut bad_response = String::new();
            bad_stream
                .read_to_string(&mut bad_response)
                .expect("read bad response");
            assert!(bad_response.contains("404 Not Found"));

            let mut stream = TcpStream::connect(address).expect("connect loopback");
            let request = "GET /?code=abc&state=state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
            stream.write_all(request.as_bytes()).expect("write request");
            let mut response = String::new();
            stream.read_to_string(&mut response).expect("read response");
            sender.send(response).expect("send response");
        });

        let callback = receiver
            .receive_callback(Duration::from_secs(5))
            .expect("receive callback");
        let response = receiver_done.recv_timeout(Duration::from_secs(5)).unwrap();

        assert!(callback.contains("/?code=abc&state=state"));
        assert!(response.contains("Authorization received"));
    }
}
