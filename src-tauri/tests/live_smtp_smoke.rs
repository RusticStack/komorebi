use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use komorebi_lib::{
    composer::{
        autosave_composer_draft_from_path, send_composer_email_from_path, ComposerAddress,
        ComposerDraftSaveRequest, ComposerSendProgress, ComposerSendRequest,
    },
    credentials::{credential_target_name, CredentialSlot, WindowsCredentialManager},
    schema::apply_migrations,
    sync_scheduler::store_account_oauth_client_config,
    token_lifecycle::SecretString,
};
use rusqlite::{params, Connection};

const ENABLE_ENV: &str = "KOMOREBI_LIVE_SMTP_SMOKE";

#[test]
#[ignore = "requires KOMOREBI_LIVE_SMTP_SMOKE=1 and a user-provided test account"]
fn opt_in_live_smtp_send_through_configured_composer_command() {
    if env::var(ENABLE_ENV).ok().as_deref() != Some("1") {
        eprintln!("skipping live SMTP smoke; set {ENABLE_ENV}=1 to opt in");
        return;
    }

    let config = LiveSmtpSmokeConfig::from_env().expect("valid live SMTP smoke configuration");
    let app_data_dir = unique_app_data_dir();
    fs::create_dir_all(&app_data_dir).expect("create temp app-data dir");
    let cache_path = app_data_dir.join("mail-cache.sqlite3");
    let slot = config.credential_slot();
    let _cleanup = SmokeCleanup {
        account_id: config.account_id.clone(),
        slot,
        app_data_dir: app_data_dir.clone(),
    };

    {
        let mut connection = Connection::open(&cache_path).expect("open smoke cache");
        apply_migrations(&mut connection).expect("apply migrations");
        config
            .insert_account(&connection)
            .expect("insert smoke account");
        if let Some(oauth) = &config.oauth {
            store_account_oauth_client_config(
                &connection,
                &config.account_id,
                &oauth.client_id,
                &oauth.redirect_uri,
            )
            .expect("store non-secret OAuth client config");
        }
    }

    WindowsCredentialManager
        .write_secret(
            &config.account_id,
            slot,
            &SecretString::new(config.secret.clone()),
        )
        .expect("write opt-in smoke credential");

    let draft = config.draft_request();
    autosave_composer_draft_from_path(&cache_path, draft.clone()).expect("autosave smoke draft");
    let response = send_composer_email_from_path(
        &cache_path,
        &app_data_dir,
        ComposerSendRequest {
            account_id: draft.account_id,
            draft_id: draft.draft_id,
            subject: draft.subject,
            to: draft.to,
            cc: draft.cc,
            bcc: draft.bcc,
            body_text: draft.body_text,
            body_html: draft.body_html,
            attachment_ids: draft.attachment_ids,
        },
    )
    .expect("send live SMTP smoke through typed composer command");

    assert!(response
        .progress
        .contains(&ComposerSendProgress::SmtpAccepted));
    assert!(response
        .progress
        .contains(&ComposerSendProgress::SentStateRecorded));
    assert_eq!(response.account_id, config.account_id);
    assert_eq!(response.draft_id, "draft_live_smtp_smoke");
}

struct SmokeCleanup {
    account_id: String,
    slot: CredentialSlot,
    app_data_dir: PathBuf,
}

impl Drop for SmokeCleanup {
    fn drop(&mut self) {
        let _ = WindowsCredentialManager.delete_secret(&self.account_id, self.slot);
        let _ = fs::remove_dir_all(&self.app_data_dir);
    }
}

#[derive(Debug)]
struct LiveSmtpSmokeConfig {
    account_id: String,
    provider_kind: String,
    display_email: String,
    display_name: Option<String>,
    username: String,
    auth_kind: String,
    secret: String,
    recipient: String,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_security: Option<String>,
    oauth: Option<LiveOAuthConfig>,
}

#[derive(Debug)]
struct LiveOAuthConfig {
    client_id: String,
    redirect_uri: String,
}

impl LiveSmtpSmokeConfig {
    fn from_env() -> Result<Self, String> {
        let provider_kind = env_required("KOMOREBI_LIVE_SMTP_PROVIDER")?.to_ascii_lowercase();
        let display_email = env_required("KOMOREBI_LIVE_SMTP_FROM")?;
        let username = env::var("KOMOREBI_LIVE_SMTP_USERNAME").unwrap_or(display_email.clone());
        let recipient = env_required("KOMOREBI_LIVE_SMTP_TO")?;
        let account_id = env::var("KOMOREBI_LIVE_SMTP_ACCOUNT_ID")
            .unwrap_or_else(|_| format!("acct_live_smtp_smoke_{}", process::id()));
        let display_name = env_optional("KOMOREBI_LIVE_SMTP_DISPLAY_NAME");

        match provider_kind.as_str() {
            "gmail" | "microsoft" => Ok(Self {
                account_id,
                provider_kind,
                display_email,
                display_name,
                username,
                auth_kind: "oauth2".to_string(),
                secret: env_required("KOMOREBI_LIVE_SMTP_REFRESH_TOKEN")?,
                recipient,
                imap_host: None,
                imap_port: None,
                imap_security: None,
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
                oauth: Some(LiveOAuthConfig {
                    client_id: env_required("KOMOREBI_LIVE_SMTP_OAUTH_CLIENT_ID")?,
                    redirect_uri: env_required("KOMOREBI_LIVE_SMTP_OAUTH_REDIRECT_URI")?,
                }),
            }),
            "generic_imap_smtp" => Ok(Self {
                account_id,
                provider_kind,
                display_email,
                display_name,
                username,
                auth_kind: env::var("KOMOREBI_LIVE_SMTP_AUTH_KIND")
                    .unwrap_or_else(|_| "app_password".to_string()),
                secret: env_required("KOMOREBI_LIVE_SMTP_SECRET")?,
                recipient,
                imap_host: Some(env_required("KOMOREBI_LIVE_SMTP_IMAP_HOST")?),
                imap_port: Some(env_u16("KOMOREBI_LIVE_SMTP_IMAP_PORT")?),
                imap_security: Some(env_security("KOMOREBI_LIVE_SMTP_IMAP_SECURITY")?),
                smtp_host: Some(env_required("KOMOREBI_LIVE_SMTP_HOST")?),
                smtp_port: Some(env_u16("KOMOREBI_LIVE_SMTP_PORT")?),
                smtp_security: Some(env_security("KOMOREBI_LIVE_SMTP_SECURITY")?),
                oauth: None,
            }),
            _ => Err(
                "KOMOREBI_LIVE_SMTP_PROVIDER must be gmail, microsoft, or generic_imap_smtp"
                    .to_string(),
            ),
        }
    }

    fn credential_slot(&self) -> CredentialSlot {
        if self.auth_kind == "oauth2" {
            CredentialSlot::OAuthRefresh
        } else {
            CredentialSlot::GenericPassword
        }
    }

    fn insert_account(&self, connection: &Connection) -> Result<(), rusqlite::Error> {
        let credential_ref = credential_target_name(&self.account_id, self.credential_slot())
            .expect("valid smoke credential target");
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
                &self.account_id,
                &self.provider_kind,
                &self.display_email,
                &self.display_name,
                &self.username,
                &self.auth_kind,
                credential_ref,
                &self.imap_host,
                &self.imap_port,
                &self.imap_security,
                &self.smtp_host,
                &self.smtp_port,
                &self.smtp_security,
            ],
        )?;
        Ok(())
    }

    fn draft_request(&self) -> ComposerDraftSaveRequest {
        ComposerDraftSaveRequest {
            account_id: self.account_id.clone(),
            draft_id: "draft_live_smtp_smoke".to_string(),
            subject: format!("Komorebi live SMTP smoke {}", unix_now()),
            to: vec![ComposerAddress {
                name: Some("Komorebi SMTP Smoke Recipient".to_string()),
                address: self.recipient.clone(),
            }],
            cc: Vec::new(),
            bcc: Vec::new(),
            body_text: Some(
                "This opt-in Komorebi smoke message was sent from a user-provided test account."
                    .to_string(),
            ),
            body_html: Some(
                "<p>This opt-in Komorebi smoke message was sent from a user-provided test account.</p>"
                    .to_string(),
            ),
            attachment_ids: Vec::new(),
        }
    }
}

fn env_required(name: &str) -> Result<String, String> {
    env::var(name)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

fn env_optional(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u16(name: &str) -> Result<u16, String> {
    env_required(name)?
        .parse::<u16>()
        .map_err(|_| format!("{name} must be a port between 1 and 65535"))
        .and_then(|port| {
            if port == 0 {
                Err(format!("{name} must be a port between 1 and 65535"))
            } else {
                Ok(port)
            }
        })
}

fn env_security(name: &str) -> Result<String, String> {
    let value = env_required(name)?.to_ascii_lowercase();
    match value.as_str() {
        "tls" | "starttls" => Ok(value),
        _ => Err(format!("{name} must be tls or starttls")),
    }
}

fn unique_app_data_dir() -> PathBuf {
    env::temp_dir().join(format!(
        "komorebi-live-smtp-smoke-{}-{}",
        process::id(),
        unix_now()
    ))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
