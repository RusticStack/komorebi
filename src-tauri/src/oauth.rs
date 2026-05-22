use std::{error::Error, fmt};

use base64::{engine::general_purpose, Engine as _};
use rand::RngExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    auth::{ProviderKind, SaslMechanism},
    ipc_validation::{
        validate_bounded_text, validate_checked_url, IpcValidationError, MAX_TEXT_FIELD_BYTES,
    },
    token_lifecycle::{
        OAuthRefreshResponse, OAuthTokenClient, RefreshFailure, RevocationOutcome, SecretString,
    },
};

pub const GMAIL_MAIL_SCOPE: &str = "https://mail.google.com/";
pub const MICROSOFT_OFFLINE_SCOPE: &str = "offline_access";
pub const MICROSOFT_IMAP_SCOPE: &str = "https://outlook.office.com/IMAP.AccessAsUser.All";
pub const MICROSOFT_SMTP_SCOPE: &str = "https://outlook.office.com/SMTP.Send";

const PKCE_RANDOM_BYTES: usize = 32;
const STATE_RANDOM_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClientConfig {
    pub provider: ProviderKind,
    pub client_id: String,
    pub redirect_uri: String,
    pub client_secret: Option<SecretString>,
}

impl OAuthClientConfig {
    pub fn new(
        provider: ProviderKind,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        client_secret: Option<SecretString>,
    ) -> Result<Self, OAuthError> {
        let client_id = client_id.into();
        let redirect_uri = redirect_uri.into();

        provider_oauth_profile(provider)?;
        validate_bounded_text("client_id", &client_id, MAX_TEXT_FIELD_BYTES)?;
        validate_checked_url("redirect_uri", &redirect_uri)?;

        Ok(Self {
            provider,
            client_id,
            redirect_uri,
            client_secret,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderOAuthProfile {
    pub authorization_endpoint: &'static str,
    pub token_endpoint: &'static str,
    pub revocation_endpoint: Option<&'static str>,
    pub scopes: &'static [&'static str],
    pub sasl_mechanism: SaslMechanism,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthAuthorizationRequest {
    pub authorization_url: String,
    pub state: String,
    pub pkce_verifier: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthTokenSet {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub access_expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OAuthError {
    IpcValidation(IpcValidationError),
    UnsupportedProvider { provider: ProviderKind },
    HttpRequestFailed,
    HttpStatus { status: u16 },
    InvalidGrant,
    ProviderRejected { error: String },
    MissingAccessToken,
    MissingExpiry,
    InvalidTokenResponse,
}

impl fmt::Display for OAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IpcValidation(error) => write!(formatter, "{error}"),
            Self::UnsupportedProvider { provider } => {
                write!(formatter, "{provider:?} does not have an OAuth profile")
            }
            Self::HttpRequestFailed => write!(formatter, "OAuth HTTP request failed"),
            Self::HttpStatus { status } => {
                write!(formatter, "OAuth provider returned HTTP {status}")
            }
            Self::InvalidGrant => write!(formatter, "OAuth refresh grant is invalid or revoked"),
            Self::ProviderRejected { error } => {
                write!(formatter, "OAuth provider rejected request: {error}")
            }
            Self::MissingAccessToken => write!(formatter, "OAuth response omitted access_token"),
            Self::MissingExpiry => write!(formatter, "OAuth response omitted expires_in"),
            Self::InvalidTokenResponse => write!(formatter, "OAuth response could not be parsed"),
        }
    }
}

impl Error for OAuthError {}

impl From<IpcValidationError> for OAuthError {
    fn from(value: IpcValidationError) -> Self {
        Self::IpcValidation(value)
    }
}

pub fn provider_oauth_profile(provider: ProviderKind) -> Result<ProviderOAuthProfile, OAuthError> {
    match provider {
        ProviderKind::Gmail => Ok(ProviderOAuthProfile {
            authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth",
            token_endpoint: "https://oauth2.googleapis.com/token",
            revocation_endpoint: Some("https://oauth2.googleapis.com/revoke"),
            scopes: &[GMAIL_MAIL_SCOPE],
            sasl_mechanism: SaslMechanism::XOAuth2,
        }),
        ProviderKind::Microsoft => Ok(ProviderOAuthProfile {
            authorization_endpoint:
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            revocation_endpoint: None,
            scopes: &[
                MICROSOFT_OFFLINE_SCOPE,
                MICROSOFT_IMAP_SCOPE,
                MICROSOFT_SMTP_SCOPE,
            ],
            sasl_mechanism: SaslMechanism::XOAuth2,
        }),
        ProviderKind::GenericImapSmtp => Err(OAuthError::UnsupportedProvider { provider }),
    }
}

pub fn build_authorization_request(
    config: &OAuthClientConfig,
) -> Result<OAuthAuthorizationRequest, OAuthError> {
    let pkce = generate_pkce_pair();
    let state = random_urlsafe_token(STATE_RANDOM_BYTES);

    build_authorization_request_with_pkce(config, state, pkce)
}

pub fn build_authorization_request_with_pkce(
    config: &OAuthClientConfig,
    state: String,
    pkce: PkcePair,
) -> Result<OAuthAuthorizationRequest, OAuthError> {
    validate_bounded_text("oauth_state", &state, MAX_TEXT_FIELD_BYTES)?;
    validate_bounded_text("pkce_verifier", &pkce.verifier, MAX_TEXT_FIELD_BYTES)?;
    validate_bounded_text("pkce_challenge", &pkce.challenge, MAX_TEXT_FIELD_BYTES)?;

    let profile = provider_oauth_profile(config.provider)?;
    let scope = profile.scopes.join(" ");
    let mut url = Url::parse(profile.authorization_endpoint).map_err(|_| {
        OAuthError::UnsupportedProvider {
            provider: config.provider,
        }
    })?;

    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &config.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scope)
            .append_pair("state", &state)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256");

        match config.provider {
            ProviderKind::Gmail => {
                query
                    .append_pair("access_type", "offline")
                    .append_pair("prompt", "consent")
                    .append_pair("include_granted_scopes", "true");
            }
            ProviderKind::Microsoft => {
                query
                    .append_pair("response_mode", "query")
                    .append_pair("prompt", "select_account");
            }
            ProviderKind::GenericImapSmtp => {}
        }
    }

    Ok(OAuthAuthorizationRequest {
        authorization_url: url.to_string(),
        state,
        pkce_verifier: pkce.verifier,
        scopes: profile
            .scopes
            .iter()
            .map(|scope| scope.to_string())
            .collect(),
    })
}

pub fn generate_pkce_pair() -> PkcePair {
    let verifier = random_urlsafe_token(PKCE_RANDOM_BYTES);
    let challenge = pkce_challenge(&verifier);

    PkcePair {
        verifier,
        challenge,
    }
}

pub fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());

    general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub fn build_xoauth2_initial_response(
    username: &str,
    access_token: &SecretString,
) -> Result<String, OAuthError> {
    validate_bounded_text("username", username, MAX_TEXT_FIELD_BYTES)?;
    validate_bounded_text(
        "access_token",
        access_token.expose_secret(),
        MAX_TEXT_FIELD_BYTES,
    )?;

    if username.contains(['\0', '\x01']) || access_token.expose_secret().contains(['\0', '\x01']) {
        return Err(OAuthError::InvalidTokenResponse);
    }

    let response = format!(
        "user={username}\x01auth=Bearer {}\x01\x01",
        access_token.expose_secret()
    );

    Ok(general_purpose::STANDARD.encode(response.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthHttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthHttpError;

pub trait OAuthHttpClient {
    fn post_form(
        &self,
        url: &str,
        fields: &[(&str, String)],
    ) -> Result<OAuthHttpResponse, OAuthHttpError>;
}

#[derive(Debug, Clone)]
pub struct ReqwestOAuthHttpClient {
    client: reqwest::blocking::Client,
}

impl Default for ReqwestOAuthHttpClient {
    fn default() -> Self {
        Self {
            client: reqwest::blocking::Client::new(),
        }
    }
}

impl OAuthHttpClient for ReqwestOAuthHttpClient {
    fn post_form(
        &self,
        url: &str,
        fields: &[(&str, String)],
    ) -> Result<OAuthHttpResponse, OAuthHttpError> {
        let borrowed_fields = fields
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect::<Vec<_>>();
        let response = self
            .client
            .post(url)
            .form(&borrowed_fields)
            .send()
            .map_err(|_| OAuthHttpError)?;
        let status = response.status().as_u16();
        let body = response.text().map_err(|_| OAuthHttpError)?;

        Ok(OAuthHttpResponse { status, body })
    }
}

pub fn exchange_authorization_code(
    http: &impl OAuthHttpClient,
    config: &OAuthClientConfig,
    authorization_code: &SecretString,
    pkce_verifier: &str,
    now: u64,
) -> Result<OAuthTokenSet, OAuthError> {
    validate_bounded_text(
        "authorization_code",
        authorization_code.expose_secret(),
        MAX_TEXT_FIELD_BYTES,
    )?;
    validate_bounded_text("pkce_verifier", pkce_verifier, MAX_TEXT_FIELD_BYTES)?;

    let mut fields = base_token_fields(config)?;
    fields.push(("grant_type", "authorization_code".to_string()));
    fields.push(("code", authorization_code.expose_secret().to_string()));
    fields.push(("code_verifier", pkce_verifier.to_string()));

    post_token_request(http, config, &fields, now)
}

pub fn refresh_access_token(
    http: &impl OAuthHttpClient,
    config: &OAuthClientConfig,
    refresh_token: &SecretString,
    now: u64,
) -> Result<OAuthTokenSet, OAuthError> {
    validate_bounded_text(
        "refresh_token",
        refresh_token.expose_secret(),
        MAX_TEXT_FIELD_BYTES,
    )?;

    let mut fields = base_token_fields(config)?;
    fields.push(("grant_type", "refresh_token".to_string()));
    fields.push(("refresh_token", refresh_token.expose_secret().to_string()));
    fields.push((
        "scope",
        provider_oauth_profile(config.provider)?.scopes.join(" "),
    ));

    post_token_request(http, config, &fields, now)
}

pub fn revoke_refresh_token(
    http: &impl OAuthHttpClient,
    provider: ProviderKind,
    refresh_token: &SecretString,
) -> RevocationOutcome {
    let Ok(profile) = provider_oauth_profile(provider) else {
        return RevocationOutcome::ProviderRevocationUnsupported;
    };
    let Some(endpoint) = profile.revocation_endpoint else {
        return RevocationOutcome::ProviderRevocationUnsupported;
    };

    let fields = [("token", refresh_token.expose_secret().to_string())];

    match http.post_form(endpoint, &fields) {
        Ok(response) if (200..300).contains(&response.status) => RevocationOutcome::Revoked,
        Ok(_) | Err(_) => RevocationOutcome::Failed,
    }
}

pub struct ProviderOAuthTokenClient<H = ReqwestOAuthHttpClient> {
    configs: Vec<OAuthClientConfig>,
    http: H,
}

impl ProviderOAuthTokenClient<ReqwestOAuthHttpClient> {
    pub fn new(configs: Vec<OAuthClientConfig>) -> Self {
        Self {
            configs,
            http: ReqwestOAuthHttpClient::default(),
        }
    }
}

impl<H> ProviderOAuthTokenClient<H>
where
    H: OAuthHttpClient,
{
    pub fn with_http(configs: Vec<OAuthClientConfig>, http: H) -> Self {
        Self { configs, http }
    }

    fn config_for(&self, provider: ProviderKind) -> Option<&OAuthClientConfig> {
        self.configs
            .iter()
            .find(|config| config.provider == provider)
    }
}

impl<H> OAuthTokenClient for ProviderOAuthTokenClient<H>
where
    H: OAuthHttpClient,
{
    fn refresh_access_token(
        &mut self,
        provider: ProviderKind,
        refresh_token: &SecretString,
        now: u64,
    ) -> Result<OAuthRefreshResponse, RefreshFailure> {
        let config = self
            .config_for(provider)
            .ok_or(RefreshFailure::ProviderUnavailable)?;

        let token_set = refresh_access_token(&self.http, config, refresh_token, now)
            .map_err(refresh_failure)?;

        Ok(OAuthRefreshResponse {
            access_token: token_set.access_token,
            access_expires_at: token_set.access_expires_at,
            replacement_refresh_token: token_set.refresh_token,
        })
    }

    fn revoke_refresh_token(
        &mut self,
        provider: ProviderKind,
        refresh_token: &SecretString,
    ) -> RevocationOutcome {
        revoke_refresh_token(&self.http, provider, refresh_token)
    }
}

fn base_token_fields(
    config: &OAuthClientConfig,
) -> Result<Vec<(&'static str, String)>, OAuthError> {
    validate_bounded_text("client_id", &config.client_id, MAX_TEXT_FIELD_BYTES)?;
    validate_checked_url("redirect_uri", &config.redirect_uri)?;

    let mut fields = vec![
        ("client_id", config.client_id.clone()),
        ("redirect_uri", config.redirect_uri.clone()),
    ];

    if let Some(secret) = &config.client_secret {
        validate_bounded_text(
            "client_secret",
            secret.expose_secret(),
            MAX_TEXT_FIELD_BYTES,
        )?;
        fields.push(("client_secret", secret.expose_secret().to_string()));
    }

    Ok(fields)
}

fn post_token_request(
    http: &impl OAuthHttpClient,
    config: &OAuthClientConfig,
    fields: &[(&str, String)],
    now: u64,
) -> Result<OAuthTokenSet, OAuthError> {
    let profile = provider_oauth_profile(config.provider)?;
    let response = http
        .post_form(profile.token_endpoint, fields)
        .map_err(|_| OAuthError::HttpRequestFailed)?;

    parse_token_response(response, now)
}

fn parse_token_response(
    response: OAuthHttpResponse,
    now: u64,
) -> Result<OAuthTokenSet, OAuthError> {
    let payload: OAuthTokenPayload =
        serde_json::from_str(&response.body).map_err(|_| OAuthError::InvalidTokenResponse)?;

    if let Some(error) = payload.error {
        if error == "invalid_grant" {
            return Err(OAuthError::InvalidGrant);
        }

        return Err(OAuthError::ProviderRejected { error });
    }

    if !(200..300).contains(&response.status) {
        return Err(OAuthError::HttpStatus {
            status: response.status,
        });
    }

    let access_token = payload.access_token.ok_or(OAuthError::MissingAccessToken)?;
    let expires_in = payload.expires_in.ok_or(OAuthError::MissingExpiry)?;

    Ok(OAuthTokenSet {
        access_token: SecretString::new(access_token),
        refresh_token: payload.refresh_token.map(SecretString::new),
        access_expires_at: now.saturating_add(expires_in),
    })
}

fn refresh_failure(error: OAuthError) -> RefreshFailure {
    match error {
        OAuthError::InvalidGrant => RefreshFailure::InvalidGrant,
        OAuthError::HttpRequestFailed => RefreshFailure::Network,
        OAuthError::HttpStatus { .. }
        | OAuthError::IpcValidation(_)
        | OAuthError::UnsupportedProvider { .. }
        | OAuthError::ProviderRejected { .. }
        | OAuthError::MissingAccessToken
        | OAuthError::MissingExpiry
        | OAuthError::InvalidTokenResponse => RefreshFailure::ProviderUnavailable,
    }
}

fn random_urlsafe_token(byte_count: usize) -> String {
    let mut bytes = vec![0_u8; byte_count];
    rand::rng().fill(&mut bytes);

    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Deserialize)]
struct OAuthTokenPayload {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const NOW: u64 = 1_800_000_000;
    type RecordedRequest = (String, Vec<(String, String)>);

    fn gmail_config() -> OAuthClientConfig {
        OAuthClientConfig::new(
            ProviderKind::Gmail,
            "gmail-client-id",
            "http://127.0.0.1:49152/oauth/callback",
            None,
        )
        .unwrap()
    }

    fn microsoft_config() -> OAuthClientConfig {
        OAuthClientConfig::new(
            ProviderKind::Microsoft,
            "microsoft-client-id",
            "http://127.0.0.1:49153/oauth/callback",
            None,
        )
        .unwrap()
    }

    #[derive(Default)]
    struct FakeOAuthHttpClient {
        responses: RefCell<Vec<Result<OAuthHttpResponse, OAuthHttpError>>>,
        requests: RefCell<Vec<RecordedRequest>>,
    }

    impl FakeOAuthHttpClient {
        fn with_json(status: u16, body: &str) -> Self {
            Self {
                responses: RefCell::new(vec![Ok(OAuthHttpResponse {
                    status,
                    body: body.to_string(),
                })]),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.borrow().clone()
        }
    }

    impl OAuthHttpClient for FakeOAuthHttpClient {
        fn post_form(
            &self,
            url: &str,
            fields: &[(&str, String)],
        ) -> Result<OAuthHttpResponse, OAuthHttpError> {
            self.requests.borrow_mut().push((
                url.to_string(),
                fields
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), value.clone()))
                    .collect(),
            ));

            self.responses.borrow_mut().remove(0)
        }
    }

    #[test]
    fn gmail_authorization_request_uses_pkce_offline_and_mail_scope() {
        let request = build_authorization_request_with_pkce(
            &gmail_config(),
            "state_value".into(),
            PkcePair {
                verifier: "verifier_value".into(),
                challenge: "challenge_value".into(),
            },
        )
        .unwrap();
        let url = Url::parse(&request.authorization_url).unwrap();
        let query = url.query_pairs().collect::<Vec<_>>();

        assert_eq!(
            url.as_str().split('?').next().unwrap(),
            "https://accounts.google.com/o/oauth2/v2/auth"
        );
        assert!(query
            .iter()
            .any(|(name, value)| name == "scope" && value == GMAIL_MAIL_SCOPE));
        assert!(query
            .iter()
            .any(|(name, value)| name == "access_type" && value == "offline"));
        assert!(query
            .iter()
            .any(|(name, value)| name == "prompt" && value == "consent"));
        assert_eq!(request.pkce_verifier, "verifier_value");
    }

    #[test]
    fn microsoft_authorization_request_uses_offline_imap_and_smtp_scopes() {
        let request = build_authorization_request_with_pkce(
            &microsoft_config(),
            "state_value".into(),
            PkcePair {
                verifier: "verifier_value".into(),
                challenge: "challenge_value".into(),
            },
        )
        .unwrap();
        let url = Url::parse(&request.authorization_url).unwrap();
        let scope = url
            .query_pairs()
            .find(|(name, _)| name == "scope")
            .map(|(_, value)| value.into_owned())
            .unwrap();

        assert!(url
            .as_str()
            .starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"));
        assert!(scope.contains(MICROSOFT_OFFLINE_SCOPE));
        assert!(scope.contains(MICROSOFT_IMAP_SCOPE));
        assert!(scope.contains(MICROSOFT_SMTP_SCOPE));
    }

    #[test]
    fn generated_pkce_pair_is_urlsafe_and_uses_s256_challenge() {
        let pair = generate_pkce_pair();

        assert_eq!(pair.verifier.len(), 43);
        assert!(!pair.verifier.contains('='));
        assert_eq!(pair.challenge, pkce_challenge(&pair.verifier));
    }

    #[test]
    fn xoauth2_initial_response_matches_sasl_format() {
        let encoded = build_xoauth2_initial_response(
            "person@example.com",
            &SecretString::new("access-token"),
        )
        .unwrap();
        let decoded =
            String::from_utf8(general_purpose::STANDARD.decode(encoded).unwrap()).unwrap();

        assert_eq!(
            decoded,
            "user=person@example.com\x01auth=Bearer access-token\x01\x01"
        );
    }

    #[test]
    fn exchanges_authorization_code_with_pkce_and_parses_tokens() {
        let http = FakeOAuthHttpClient::with_json(
            200,
            r#"{"access_token":"access-1","refresh_token":"refresh-1","expires_in":3600}"#,
        );

        let token_set = exchange_authorization_code(
            &http,
            &gmail_config(),
            &SecretString::new("auth-code"),
            "pkce-verifier",
            NOW,
        )
        .unwrap();

        assert_eq!(token_set.access_token.expose_secret(), "access-1");
        assert_eq!(
            token_set.refresh_token.unwrap().expose_secret(),
            "refresh-1"
        );
        assert_eq!(token_set.access_expires_at, NOW + 3600);

        let requests = http.requests();
        assert_eq!(requests[0].0, "https://oauth2.googleapis.com/token");
        assert!(requests[0]
            .1
            .contains(&("grant_type".into(), "authorization_code".into())));
        assert!(requests[0]
            .1
            .contains(&("code_verifier".into(), "pkce-verifier".into())));
    }

    #[test]
    fn refresh_posts_refresh_token_and_maps_replacement_token() {
        let http = FakeOAuthHttpClient::with_json(
            200,
            r#"{"access_token":"access-2","refresh_token":"refresh-2","expires_in":900}"#,
        );
        let mut client = ProviderOAuthTokenClient::with_http(vec![microsoft_config()], http);

        let response = client
            .refresh_access_token(
                ProviderKind::Microsoft,
                &SecretString::new("refresh-1"),
                NOW,
            )
            .unwrap();

        assert_eq!(response.access_token.expose_secret(), "access-2");
        assert_eq!(
            response.replacement_refresh_token.unwrap().expose_secret(),
            "refresh-2"
        );
        assert_eq!(response.access_expires_at, NOW + 900);

        let requests = client.http.requests();
        assert_eq!(
            requests[0].0,
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        assert!(requests[0]
            .1
            .contains(&("grant_type".into(), "refresh_token".into())));
        assert!(requests[0]
            .1
            .contains(&("refresh_token".into(), "refresh-1".into())));
    }

    #[test]
    fn invalid_grant_maps_to_lifecycle_invalid_grant() {
        let http = FakeOAuthHttpClient::with_json(400, r#"{"error":"invalid_grant"}"#);
        let mut client = ProviderOAuthTokenClient::with_http(vec![gmail_config()], http);

        assert_eq!(
            client.refresh_access_token(
                ProviderKind::Gmail,
                &SecretString::new("revoked-refresh"),
                NOW,
            ),
            Err(RefreshFailure::InvalidGrant)
        );
    }

    #[test]
    fn google_revocation_posts_token_and_microsoft_revocation_is_unsupported() {
        let http = FakeOAuthHttpClient::with_json(200, "{}");

        assert_eq!(
            revoke_refresh_token(&http, ProviderKind::Gmail, &SecretString::new("refresh-1")),
            RevocationOutcome::Revoked
        );
        assert_eq!(
            revoke_refresh_token(
                &http,
                ProviderKind::Microsoft,
                &SecretString::new("refresh-1")
            ),
            RevocationOutcome::ProviderRevocationUnsupported
        );

        let requests = http.requests();
        assert_eq!(requests[0].0, "https://oauth2.googleapis.com/revoke");
        assert!(requests[0]
            .1
            .contains(&("token".into(), "refresh-1".into())));
    }
}
