//! OAuth 2.0 authorization code + PKCE for Gmail and Microsoft.
//!
//! The flow is backend-owned: the auth URL opens in the user's browser, a
//! tiny loopback listener captures the authorization code, and the token
//! exchange/refresh happens here. Tokens are stored in the OS credential
//! store (`crate::secrets`), never in SQLite or logs.

use super::types::ProviderKind;
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub provider: ProviderKind,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub redirect_uri: String,
}

impl OAuthConfig {
    pub fn gmail(client_id: &str, client_secret: &str, redirect_uri: &str) -> Self {
        Self {
            provider: ProviderKind::Gmail,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scopes: vec![
                "https://mail.google.com/".into(),
                "openid".into(),
                "email".into(),
            ],
            redirect_uri: redirect_uri.into(),
        }
    }

    pub fn microsoft(client_id: &str, client_secret: &str, redirect_uri: &str) -> Self {
        Self {
            provider: ProviderKind::Microsoft,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
            scopes: vec![
                "offline_access".into(),
                "https://outlook.office.com/IMAP.AccessAsUser.All".into(),
                "https://outlook.office.com/SMTP.Send".into(),
            ],
            redirect_uri: redirect_uri.into(),
        }
    }

    pub fn for_kind(
        kind: ProviderKind,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<Self> {
        match kind {
            ProviderKind::Gmail => Ok(Self::gmail(client_id, client_secret, redirect_uri)),
            ProviderKind::Microsoft => Ok(Self::microsoft(client_id, client_secret, redirect_uri)),
            _ => bail!("{kind:?} does not support OAuth"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OAuthSession {
    pub state: String,
    pub verifier: String,
    pub auth_url: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub token_type: String,
}

fn base64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    base64url(&buf)
}

/// Start the PKCE flow: build the authorization URL and keep state locally.
pub fn start(config: &OAuthConfig) -> Result<OAuthSession> {
    let state = random_urlsafe(16);
    let verifier = random_urlsafe(32);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    let scopes = config.scopes.join(" ");
    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        config.auth_url,
        url::form_urlencoded::byte_serialize(config.client_id.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(config.redirect_uri.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(scopes.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(state.as_bytes()).collect::<String>(),
        url::form_urlencoded::byte_serialize(challenge.as_bytes()).collect::<String>(),
    );
    Ok(OAuthSession {
        state,
        verifier,
        auth_url,
        redirect_uri: config.redirect_uri.clone(),
    })
}

/// Listen on the redirect URI's loopback port until the provider redirects
/// back with a code (or error), then return the authorization code.
/// Fails with `CallbackInterrupted` if the user closes the dialog early  - 
/// callers should offer the manual code paste fallback.
pub fn wait_for_code(redirect_uri: &str, timeout: Duration) -> Result<String> {
    let url = url::Url::parse(redirect_uri).context("invalid redirect URI")?;
    if url.host_str() != Some("127.0.0.1") {
        bail!("redirect URI must be a 127.0.0.1 loopback URL");
    }
    let port = url.port().context("redirect URI needs a port")?;
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("binding loopback callback port {port}"))?;
    listener
        .set_nonblocking(true)
        .context("setting nonblocking listener")?;

    let deadline = std::time::Instant::now() + timeout;
    let mut code: Option<String> = None;
    let mut error: Option<String> = None;

    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = handle_callback(stream) {
                    match result {
                        Ok(c) => {
                            code = Some(c);
                            break;
                        }
                        Err(e) => {
                            error = Some(e);
                            break;
                        }
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(anyhow!("callback listener error: {e}")),
        }
    }

    if let Some(c) = code {
        Ok(c)
    } else if let Some(e) = error {
        bail!("authorization failed: {e}")
    } else {
        bail!("timed out waiting for the browser callback")
    }
}

fn handle_callback(mut stream: TcpStream) -> Option<Result<String, String>> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    // Consume headers.
    let mut line = String::new();
    while let Ok(n) = reader.read_line(&mut line) {
        if n <= 2 {
            break;
        }
        line.clear();
    }

    let body = "Komorebi - authorization complete. You can close this window.";
    let html = format!(
        "<!doctype html><html><body style=\"font-family:system-ui;padding:3rem\"><h2>{body}</h2></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );

    let path = request_line.split_whitespace().nth(1)?.to_string();
    let url = url::Url::parse(&format!("http://127.0.0.1{path}")).ok()?;
    let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(e) = query.get("error") {
        return Some(Err(format!(
            "{}: {}",
            e,
            query.get("error_description").cloned().unwrap_or_default()
        )));
    }
    query.get("code").cloned().map(Ok)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: i64,
    token_type: String,
}

fn token_endpoint(config: &OAuthConfig, form: Vec<(&str, String)>) -> Result<TokenSet> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let resp = client
        .post(&config.token_url)
        .form(&form)
        .send()
        .context("token request failed")?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("token endpoint returned {status}: {text}");
    }
    let parsed: TokenResponse = serde_json::from_str(&text).context("parsing token response")?;
    Ok(TokenSet {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_in: parsed.expires_in,
        token_type: parsed.token_type,
    })
}

/// Exchange the authorization code for tokens (PKCE code_verifier included).
pub fn exchange(config: &OAuthConfig, code: &str, verifier: &str) -> Result<TokenSet> {
    token_endpoint(
        config,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.into()),
            ("client_id", config.client_id.clone()),
            ("client_secret", config.client_secret.clone()),
            ("redirect_uri", config.redirect_uri.clone()),
            ("code_verifier", verifier.into()),
        ],
    )
}

/// Refresh an expired access token.
pub fn refresh(config: &OAuthConfig, refresh_token: &str) -> Result<TokenSet> {
    token_endpoint(
        config,
        vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh_token.into()),
            ("client_id", config.client_id.clone()),
            ("client_secret", config.client_secret.clone()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn start_builds_pkce_auth_url() {
        let config = OAuthConfig::gmail(
            "client-id",
            "client-secret",
            "http://127.0.0.1:8765/callback",
        );
        let session = start(&config).unwrap();
        assert!(session.state.len() >= 20);
        assert!(session.verifier.len() >= 40);
        assert!(
            session
                .auth_url
                .starts_with("https://accounts.google.com/o/oauth2/v2/auth")
        );
        assert!(session.auth_url.contains("code_challenge="));
        assert!(session.auth_url.contains("code_challenge_method=S256"));
        assert!(session.auth_url.contains("client_id=client-id"));
    }

    #[test]
    fn loopback_callback_captures_code() {
        // Bind the listener ourselves first, then simulate a browser hitting it.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // free the port for wait_for_code

        let handle = std::thread::spawn(move || {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut stream = stream;
            let req =
                "GET /callback?code=AUTH_CODE_123&state=abc HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(req.as_bytes()).unwrap();
            let mut buf = String::new();
            stream.read_to_string(&mut buf).unwrap();
            assert!(buf.starts_with("HTTP/1.1 200 OK"));
        });

        let uri = format!("http://127.0.0.1:{port}/callback");
        let code = wait_for_code(&uri, Duration::from_secs(5)).unwrap();
        assert_eq!(code, "AUTH_CODE_123");
        handle.join().unwrap();
    }

    #[test]
    fn rejects_non_loopback_redirect() {
        let err = wait_for_code("https://app.example.com/callback", Duration::from_secs(1));
        assert!(err.is_err());
    }

    #[test]
    fn verifier_is_urlsafe() {
        let verifier = random_urlsafe(32);
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        );
        assert_eq!(verifier.len(), 43);
    }
}
