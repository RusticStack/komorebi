use std::{error::Error, fmt, time::Duration};

use quick_xml::{events::BytesStart, events::Event, Reader};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

use crate::ipc_validation::{validate_bounded_text, validate_payload_size};

const MAX_EMAIL_BYTES: usize = 320;
const MAX_HOST_BYTES: usize = 253;
const MAX_AUTOCONFIG_XML_BYTES: usize = 256 * 1024;
const AUTOCONFIG_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiscoveryRequest {
    pub display_email: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiscoveryResponse {
    pub provider_kind: String,
    pub auth_kind: String,
    pub username: String,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_security: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,
    pub source: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountDiscoveryError {
    pub reason: String,
}

impl AccountDiscoveryError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for AccountDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl Error for AccountDiscoveryError {}

#[tauri::command]
pub async fn discover_account_settings(
    request: AccountDiscoveryRequest,
) -> Result<AccountDiscoveryResponse, AccountDiscoveryError> {
    tauri::async_runtime::spawn_blocking(move || {
        let client = Client::builder()
            .timeout(AUTOCONFIG_TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent("Komorebi/0.1 account-autoconfig")
            .build()
            .map_err(|_| AccountDiscoveryError::new("account discovery is unavailable"))?;

        discover_account_settings_with_fetcher(request, |url| fetch_autoconfig_xml(&client, url))
    })
    .await
    .map_err(|error| AccountDiscoveryError::new(error.to_string()))?
}

pub fn discover_account_settings_request(
    request: AccountDiscoveryRequest,
) -> Result<AccountDiscoveryResponse, AccountDiscoveryError> {
    discover_account_settings_with_fetcher(request, |_url| Ok(None))
}

fn discover_account_settings_with_fetcher(
    request: AccountDiscoveryRequest,
    mut fetcher: impl FnMut(&str) -> Result<Option<String>, AccountDiscoveryError>,
) -> Result<AccountDiscoveryResponse, AccountDiscoveryError> {
    let email = normalize_email(request.display_email)?;
    let (local_part, domain) = split_email_domain(&email)?;

    if let Some(response) = major_provider_response(&email, domain) {
        return Ok(response);
    }

    for (source, url) in autoconfig_urls(&email, domain) {
        let Some(xml) = fetcher(&url)? else {
            continue;
        };
        if let Some(response) = parse_autoconfig_xml(&xml, &email, local_part, domain, source)? {
            return Ok(response);
        }
    }

    if let Some(response) = known_provider_response(&email, domain) {
        return Ok(response);
    }

    Ok(AccountDiscoveryResponse {
        provider_kind: "generic_imap_smtp".into(),
        auth_kind: "app_password".into(),
        username: email,
        imap_host: None,
        imap_port: None,
        imap_security: None,
        smtp_host: None,
        smtp_port: None,
        smtp_security: None,
        source: "manual".into(),
        confidence: "manual".into(),
    })
}

fn fetch_autoconfig_xml(
    client: &Client,
    url: &str,
) -> Result<Option<String>, AccountDiscoveryError> {
    let response = match client.get(url).send() {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };

    if !response.status().is_success() {
        return Ok(None);
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_AUTOCONFIG_XML_BYTES as u64)
    {
        return Ok(None);
    }

    let text = response
        .text()
        .map_err(|_| AccountDiscoveryError::new("account discovery response could not be read"))?;
    if text.len() > MAX_AUTOCONFIG_XML_BYTES {
        return Ok(None);
    }

    Ok(Some(text))
}

fn autoconfig_urls(email: &str, domain: &str) -> Vec<(&'static str, String)> {
    let encoded_email = form_urlencoded::byte_serialize(email.as_bytes()).collect::<String>();
    vec![
        (
            "domain-autoconfig",
            format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={encoded_email}"),
        ),
        (
            "well-known-autoconfig",
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={encoded_email}"),
        ),
        (
            "thunderbird-ispdb",
            format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
        ),
    ]
}

fn major_provider_response(email: &str, domain: &str) -> Option<AccountDiscoveryResponse> {
    let provider_kind = if matches!(domain, "gmail.com" | "googlemail.com") {
        "gmail"
    } else if matches!(
        domain,
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "office365.com"
    ) {
        "microsoft"
    } else {
        return None;
    };

    Some(AccountDiscoveryResponse {
        provider_kind: provider_kind.into(),
        auth_kind: "oauth2".into(),
        username: email.into(),
        imap_host: None,
        imap_port: None,
        imap_security: None,
        smtp_host: None,
        smtp_port: None,
        smtp_security: None,
        source: "major-provider".into(),
        confidence: "high".into(),
    })
}

fn known_provider_response(email: &str, domain: &str) -> Option<AccountDiscoveryResponse> {
    let (imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security) = match domain {
        "fastmail.com" => (
            "imap.fastmail.com",
            993,
            "tls",
            "smtp.fastmail.com",
            465,
            "tls",
        ),
        "icloud.com" | "me.com" => (
            "imap.mail.me.com",
            993,
            "tls",
            "smtp.mail.me.com",
            587,
            "starttls",
        ),
        "yahoo.com" => (
            "imap.mail.yahoo.com",
            993,
            "tls",
            "smtp.mail.yahoo.com",
            465,
            "tls",
        ),
        "zoho.com" => ("imap.zoho.com", 993, "tls", "smtp.zoho.com", 465, "tls"),
        "gmx.com" => ("imap.gmx.com", 993, "tls", "mail.gmx.com", 587, "starttls"),
        "gmx.net" => ("imap.gmx.net", 993, "tls", "mail.gmx.net", 587, "starttls"),
        "mail.com" => (
            "imap.mail.com",
            993,
            "tls",
            "smtp.mail.com",
            587,
            "starttls",
        ),
        _ => return None,
    };

    Some(generic_response(GenericServerSettings {
        username: email,
        imap_host,
        imap_port,
        imap_security,
        smtp_host,
        smtp_port,
        smtp_security,
        source: "known-provider",
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerSection {
    IncomingImap,
    OutgoingSmtp,
    Ignored,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ServerConfig {
    hostname: Option<String>,
    port: Option<u16>,
    socket_type: Option<String>,
    username: Option<String>,
}

fn parse_autoconfig_xml(
    xml: &str,
    email: &str,
    local_part: &str,
    domain: &str,
    source: &'static str,
) -> Result<Option<AccountDiscoveryResponse>, AccountDiscoveryError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut current_section: Option<ServerSection> = None;
    let mut current_server = ServerConfig::default();
    let mut incoming: Option<ServerConfig> = None;
    let mut outgoing: Option<ServerConfig> = None;
    let mut current_field: Option<Vec<u8>> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"incomingServer" => {
                    current_section = Some(match attr_value(&event, b"type").as_deref() {
                        Some("imap") => ServerSection::IncomingImap,
                        _ => ServerSection::Ignored,
                    });
                    current_server = ServerConfig::default();
                    current_field = None;
                }
                b"outgoingServer" => {
                    current_section = Some(match attr_value(&event, b"type").as_deref() {
                        Some("smtp") => ServerSection::OutgoingSmtp,
                        _ => ServerSection::Ignored,
                    });
                    current_server = ServerConfig::default();
                    current_field = None;
                }
                b"hostname" | b"port" | b"socketType" | b"username"
                    if current_section.is_some() =>
                {
                    current_field = Some(event.name().as_ref().to_vec());
                }
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if matches!(
                    current_section,
                    Some(ServerSection::IncomingImap | ServerSection::OutgoingSmtp)
                ) {
                    if let Some(field) = current_field.as_deref() {
                        let value = String::from_utf8_lossy(text.as_ref()).trim().to_string();
                        match field {
                            b"hostname" => current_server.hostname = Some(value),
                            b"port" => current_server.port = value.parse::<u16>().ok(),
                            b"socketType" => current_server.socket_type = Some(value),
                            b"username" => current_server.username = Some(value),
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"incomingServer" => {
                    if current_section == Some(ServerSection::IncomingImap) && incoming.is_none() {
                        incoming = Some(current_server.clone());
                    }
                    current_section = None;
                    current_field = None;
                }
                b"outgoingServer" => {
                    if current_section == Some(ServerSection::OutgoingSmtp) && outgoing.is_none() {
                        outgoing = Some(current_server.clone());
                    }
                    current_section = None;
                    current_field = None;
                }
                b"hostname" | b"port" | b"socketType" | b"username" => {
                    current_field = None;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => return Ok(None),
            _ => {}
        }
    }

    let Some(incoming) = incoming else {
        return Ok(None);
    };
    let Some(outgoing) = outgoing else {
        return Ok(None);
    };

    let Some(imap_host) = incoming
        .hostname
        .and_then(|value| normalize_host(&value).ok())
    else {
        return Ok(None);
    };
    let Some(smtp_host) = outgoing
        .hostname
        .and_then(|value| normalize_host(&value).ok())
    else {
        return Ok(None);
    };
    let Some(imap_port) = incoming.port.filter(|port| *port > 0) else {
        return Ok(None);
    };
    let Some(smtp_port) = outgoing.port.filter(|port| *port > 0) else {
        return Ok(None);
    };
    let Some(imap_security) = incoming
        .socket_type
        .and_then(|value| map_socket_type(&value))
    else {
        return Ok(None);
    };
    let Some(smtp_security) = outgoing
        .socket_type
        .and_then(|value| map_socket_type(&value))
    else {
        return Ok(None);
    };

    let username = incoming
        .username
        .or(outgoing.username)
        .map(|value| replace_username_placeholders(&value, email, local_part, domain))
        .filter(|value| value.contains('@') || !value.trim().is_empty())
        .unwrap_or_else(|| email.to_string());

    Ok(Some(generic_response(GenericServerSettings {
        username: &username,
        imap_host: &imap_host,
        imap_port,
        imap_security: &imap_security,
        smtp_host: &smtp_host,
        smtp_port,
        smtp_security: &smtp_security,
        source,
    })))
}

struct GenericServerSettings<'a> {
    username: &'a str,
    imap_host: &'a str,
    imap_port: u16,
    imap_security: &'a str,
    smtp_host: &'a str,
    smtp_port: u16,
    smtp_security: &'a str,
    source: &'a str,
}

fn generic_response(settings: GenericServerSettings<'_>) -> AccountDiscoveryResponse {
    AccountDiscoveryResponse {
        provider_kind: "generic_imap_smtp".into(),
        auth_kind: "app_password".into(),
        username: settings.username.into(),
        imap_host: Some(settings.imap_host.into()),
        imap_port: Some(settings.imap_port),
        imap_security: Some(settings.imap_security.into()),
        smtp_host: Some(settings.smtp_host.into()),
        smtp_port: Some(settings.smtp_port),
        smtp_security: Some(settings.smtp_security.into()),
        source: settings.source.into(),
        confidence: "high".into(),
    }
}

fn attr_value(event: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    event
        .attributes()
        .with_checks(false)
        .filter_map(Result::ok)
        .find(|attribute| attribute.key.as_ref() == name)
        .map(|attribute| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
}

fn map_socket_type(value: &str) -> Option<String> {
    match value.trim().to_ascii_uppercase().as_str() {
        "SSL" | "SSL/TLS" | "TLS" => Some("tls".into()),
        "STARTTLS" => Some("starttls".into()),
        _ => None,
    }
}

fn replace_username_placeholders(
    value: &str,
    email: &str,
    local_part: &str,
    domain: &str,
) -> String {
    value
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local_part)
        .replace("%EMAILDOMAIN%", domain)
        .trim()
        .to_string()
}

fn normalize_email(value: String) -> Result<String, AccountDiscoveryError> {
    let value = value.trim().to_ascii_lowercase();
    validate_bounded_text("display_email", &value, MAX_EMAIL_BYTES)
        .map_err(|error| AccountDiscoveryError::new(error.to_string()))?;
    if value.contains(char::is_whitespace)
        || value.contains('<')
        || value.contains('>')
        || value.matches('@').count() != 1
        || value.starts_with('@')
        || value.ends_with('@')
    {
        return Err(AccountDiscoveryError::new("email address is invalid"));
    }

    let (_, domain) = split_email_domain(&value)?;
    normalize_host(domain)?;

    Ok(value)
}

fn split_email_domain(email: &str) -> Result<(&str, &str), AccountDiscoveryError> {
    email
        .split_once('@')
        .ok_or_else(|| AccountDiscoveryError::new("email address is invalid"))
}

fn normalize_host(value: &str) -> Result<String, AccountDiscoveryError> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    validate_payload_size("host", value.as_bytes(), MAX_HOST_BYTES)
        .map_err(|error| AccountDiscoveryError::new(error.to_string()))?;

    if value.is_empty()
        || value.contains("://")
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
        return Err(AccountDiscoveryError::new("host is invalid"));
    }

    for label in value.split('.') {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return Err(AccountDiscoveryError::new("host is invalid"));
        }
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_major_provider_without_server_guessing() {
        let response = discover_account_settings_request(AccountDiscoveryRequest {
            display_email: "Person@Gmail.com".into(),
        })
        .expect("gmail discovery");

        assert_eq!(response.provider_kind, "gmail");
        assert_eq!(response.auth_kind, "oauth2");
        assert_eq!(response.username, "person@gmail.com");
        assert_eq!(response.imap_host, None);
        assert_eq!(response.source, "major-provider");
    }

    #[test]
    fn unknown_domain_returns_manual_state_without_imap_or_smtp_guess() {
        let response = discover_account_settings_request(AccountDiscoveryRequest {
            display_email: "person@example.invalid".into(),
        })
        .expect("manual response");

        assert_eq!(response.provider_kind, "generic_imap_smtp");
        assert_eq!(response.source, "manual");
        assert_eq!(response.imap_host, None);
        assert_eq!(response.smtp_host, None);
    }

    #[test]
    fn parses_thunderbird_autoconfig_xml() {
        let xml = r#"
          <clientConfig version="1.1">
            <emailProvider id="example.com">
              <incomingServer type="imap">
                <hostname>mail.example.com</hostname>
                <port>993</port>
                <socketType>SSL</socketType>
                <username>%EMAILADDRESS%</username>
              </incomingServer>
              <outgoingServer type="smtp">
                <hostname>submit.example.com</hostname>
                <port>587</port>
                <socketType>STARTTLS</socketType>
                <username>%EMAILLOCALPART%</username>
              </outgoingServer>
            </emailProvider>
          </clientConfig>
        "#;

        let response = parse_autoconfig_xml(
            xml,
            "person@example.com",
            "person",
            "example.com",
            "domain-autoconfig",
        )
        .expect("parse succeeds")
        .expect("config discovered");

        assert_eq!(response.provider_kind, "generic_imap_smtp");
        assert_eq!(response.username, "person@example.com");
        assert_eq!(response.imap_host.as_deref(), Some("mail.example.com"));
        assert_eq!(response.imap_port, Some(993));
        assert_eq!(response.imap_security.as_deref(), Some("tls"));
        assert_eq!(response.smtp_host.as_deref(), Some("submit.example.com"));
        assert_eq!(response.smtp_port, Some(587));
        assert_eq!(response.smtp_security.as_deref(), Some("starttls"));
        assert_eq!(response.source, "domain-autoconfig");
    }

    #[test]
    fn discovers_from_autoconfig_before_known_provider_preset() {
        let mut requested_urls = Vec::new();
        let response = discover_account_settings_with_fetcher(
            AccountDiscoveryRequest {
                display_email: "person@fastmail.com".into(),
            },
            |url| {
                requested_urls.push(url.to_string());
                Ok(Some(
                    r#"
                    <clientConfig version="1.1">
                      <emailProvider id="fastmail.com">
                        <incomingServer type="imap">
                          <hostname>imap.mailhost.test</hostname>
                          <port>993</port>
                          <socketType>SSL</socketType>
                          <username>%EMAILADDRESS%</username>
                        </incomingServer>
                        <outgoingServer type="smtp">
                          <hostname>smtp.mailhost.test</hostname>
                          <port>465</port>
                          <socketType>SSL</socketType>
                          <username>%EMAILADDRESS%</username>
                        </outgoingServer>
                      </emailProvider>
                    </clientConfig>
                    "#
                    .into(),
                ))
            },
        )
        .expect("autoconfig discovery");

        assert_eq!(requested_urls.len(), 1);
        assert_eq!(response.source, "domain-autoconfig");
        assert_eq!(response.imap_host.as_deref(), Some("imap.mailhost.test"));
    }

    #[test]
    fn skips_plaintext_autoconfig_servers() {
        let response = parse_autoconfig_xml(
            r#"
            <clientConfig version="1.1">
              <emailProvider id="example.com">
                <incomingServer type="imap">
                  <hostname>mail.example.com</hostname>
                  <port>143</port>
                  <socketType>plain</socketType>
                </incomingServer>
                <outgoingServer type="smtp">
                  <hostname>submit.example.com</hostname>
                  <port>587</port>
                  <socketType>STARTTLS</socketType>
                </outgoingServer>
              </emailProvider>
            </clientConfig>
            "#,
            "person@example.com",
            "person",
            "example.com",
            "domain-autoconfig",
        )
        .expect("parse succeeds");

        assert_eq!(response, None);
    }

    #[test]
    fn rejects_path_like_email_domain() {
        let error = discover_account_settings_request(AccountDiscoveryRequest {
            display_email: "person@example.com/path".into(),
        })
        .expect_err("invalid domain rejected");

        assert!(error.reason.contains("host is invalid"));
    }
}
