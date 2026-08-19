//! Mail protocols: providers, autodiscovery, OAuth, IMAP, SMTP, MIME.

pub mod discovery;
pub mod imap;
pub mod mime;
pub mod oauth;
pub mod providers;
pub mod smtp;
pub mod types;

use crate::db::{Account, AccountDraft, Database, MessageDraft};
use crate::secrets;
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use std::time::Duration;

/// Fetch a fresh access token for an OAuth account, refreshing if needed.
/// Tokens are read from and written back to the OS credential store.
pub fn oauth_access_token(
    db: &Database,
    account: &Account,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
) -> Result<String> {
    let refresh = secrets::get(&secrets::key::account_oauth_refresh(&account.id))?
        .ok_or_else(|| anyhow!("no OAuth refresh token stored for {}", account.email))?;
    let config = oauth::OAuthConfig::for_kind(
        providers::ProviderKind::from_str(&account.provider_kind),
        client_id,
        client_secret,
        redirect_uri,
    )?;

    // Use the cached access token if it is still valid.
    if let Some(cached) = secrets::get(&secrets::key::account_oauth_access(&account.id))? {
        if let Some(expires) = db.get_setting(&format!("oauth.expires_at.{}", account.id))? {
            if let Ok(exp) = expires.parse::<i64>() {
                if Utc::now().timestamp() < exp - 60 {
                    return Ok(cached);
                }
            }
        }
    }

    let refreshed = oauth::refresh(&config, &refresh)?;
    secrets::set(
        &secrets::key::account_oauth_access(&account.id),
        &refreshed.access_token,
    )?;
    let expires_at = Utc::now().timestamp() + refreshed.expires_in;
    db.set_setting(
        &format!("oauth.expires_at.{}", account.id),
        &expires_at.to_string(),
    )?;
    Ok(refreshed.access_token)
}

/// Full sync of one account: folder discovery plus incremental message fetch.
/// Returns (folders_created, messages_cached).
pub fn sync_account(db: &Database, account: &Account) -> Result<SyncSummary> {
    let imap = imap_settings(account)?;
    let auth = build_imap_auth(db, account)?;

    let folders = imap::ImapConnection::list_folders(&imap, &imap.host, &auth)?;
    let mut summary = SyncSummary::default();
    for folder in folders {
        if folder.special_use.as_deref() == Some("junk") {
            continue; // skip spam by default (privacy + noise)
        }
        let cached = db.upsert_folder(
            &account.id,
            &folder.name,
            Some(folder.name.as_str()),
            folder.special_use.as_deref(),
            Some(&folder.name),
        )?;
        match sync_folder(db, account, &cached.id, &folder.name, &imap, &auth) {
            Ok(r) => {
                summary.fetched += r;
                summary.folders += 1;
            }
            Err(e) => summary.errors.push(format!("{}: {e}", folder.name)),
        }
    }
    Ok(summary)
}

/// Incremental sync of one folder. Returns number of newly cached messages.
pub fn sync_folder(
    db: &Database,
    account: &Account,
    folder_id: &str,
    folder_name: &str,
    settings: &types::ServerSettings,
    auth: &imap::Auth,
) -> Result<usize> {
    let since = db.folder_max_uid(folder_id)?;
    let mut conn = imap::ImapConnection::open(settings, &settings.host, auth, folder_name)?;
    let uid_validity = conn.mailbox.uid_validity;
    let fetched = conn.fetch_new(since, Some(500))?;
    let count = cache_fetched_messages(db, account, folder_id, uid_validity, &fetched)?;
    let total = conn.mailbox.exists as i64;
    let unread = conn.mailbox.unseen.map(|u| (u as i64).max(0)).unwrap_or(0);
    db.set_folder_counts(folder_id, unread, total)?;
    Ok(count)
}

/// Ingest already-fetched messages into the local cache (deduped by IMAP UID).
///
/// Split out of [`sync_folder`] so the cache-ingest path is testable with
/// synthetic fetches and no live server.
pub fn cache_fetched_messages(
    db: &Database,
    account: &Account,
    folder_id: &str,
    uid_validity: Option<u32>,
    fetched: &[imap::RawFetch],
) -> Result<usize> {
    let mut count = 0;
    for fetch in fetched {
        let uid = fetch.uid;
        if db.find_message_by_uid(folder_id, uid)?.is_some() {
            continue;
        }
        let parsed = mime::parse(&fetch.body)?;
        let row = MessageDraft {
            account_id: account.id.clone(),
            folder_id: folder_id.to_string(),
            imap_uid: Some(uid),
            uid_validity,
            subject: parsed.subject,
            sender_name: parsed.sender_name,
            sender_email: parsed.sender_email,
            to_addresses: serde_json::to_string(&parsed.to).ok(),
            cc_addresses: serde_json::to_string(&parsed.cc).ok(),
            bcc_addresses: None,
            date_received: parsed.date,
            is_read: fetch.seen,
            is_flagged: fetch.flagged,
            snippet: parsed.snippet,
            body_text: parsed.body_text,
            body_html: parsed.body_html,
            size_bytes: Some(fetch.body.len() as i64),
            has_attachments: !parsed.attachments.is_empty(),
            rfc_message_id: parsed.message_id,
            in_reply_to: parsed.in_reply_to,
            references_header: parsed.references,
            thread_id: None,
        };
        let id = db.insert_message(&row)?;
        for att in parsed.attachments {
            db.insert_attachment(&id, &att.display_name, att.mime.as_deref(), &att.data)?;
        }
        count += 1;
    }
    Ok(count)
}

/// Send a message; on success, create a local sent copy in the Sent folder
/// (or the account's first folder if no Sent folder exists yet).
pub fn send_message(
    db: &Database,
    account: &Account,
    to: &[String],
    cc: &[String],
    subject: &str,
    body_text: &str,
    attachments: &[smtp::Attachment],
) -> Result<()> {
    let smtp = smtp_settings(account)?;
    let auth = build_smtp_auth(db, account)?;
    let from_email = account.email.clone();
    let req = smtp::SendRequest {
        from_name: account.display_name.clone(),
        from_email,
        to: to.to_vec(),
        cc: cc.to_vec(),
        subject: subject.into(),
        body_text: body_text.into(),
        body_html: None,
        attachments: attachments.to_vec(),
    };
    smtp::send(&smtp, &smtp.host, &auth, &req)?;

    // Local sent copy (provider-safe: we never touch the server Sent folder
    // in this version; the cache is the local record of what was sent).
    let folders = db.list_folders(&account.id)?;
    let sent = folders
        .iter()
        .find(|f| f.special_use.as_deref() == Some("sent"))
        .or_else(|| folders.first());
    if let Some(sent) = sent {
        let row = MessageDraft {
            account_id: account.id.clone(),
            folder_id: sent.id.clone(),
            subject: Some(subject.into()),
            sender_email: Some(account.email.clone()),
            to_addresses: serde_json::to_string(to).ok(),
            cc_addresses: serde_json::to_string(cc).ok(),
            date_received: Some(Utc::now().timestamp()),
            is_read: true,
            body_text: Some(body_text.into()),
            snippet: Some(mime::make_snippet(body_text)),
            has_attachments: !attachments.is_empty(),
            ..Default::default()
        };
        let message_id = db.insert_message(&row)?;
        for attachment in attachments {
            db.insert_attachment(
                &message_id,
                &attachment.filename,
                Some(&attachment.content_type),
                &attachment.data,
            )?;
        }
    }
    Ok(())
}

/// Discover IMAP/SMTP settings for an address using the real autoconfig HTTP
/// fetch and the system DNS resolver (autoconfig XML → RFC 6186 SRV → presets).
pub fn discover_account(email: &str) -> Result<discovery::DiscoveredSettings> {
    let fetch = |url: &str| -> Result<String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("building autodiscovery HTTP client")?;
        let resp = client
            .get(url)
            .send()
            .with_context(|| format!("autodiscovery request failed: {url}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("autodiscovery endpoint returned {status}"));
        }
        Ok(text)
    };
    discovery::discover(email, &fetch, &discovery::srv_records)
}

/// Create a password-authenticated account, validate provider policy, and
/// store the password in the OS credential store.
#[allow(clippy::too_many_arguments)]
pub fn create_password_account(
    db: &Database,
    email: &str,
    display_name: Option<&str>,
    provider_kind: types::ProviderKind,
    imap: Option<&types::ServerSettings>,
    smtp: Option<&types::ServerSettings>,
    username: Option<&str>,
    password: &str,
) -> Result<Account> {
    providers::validate_provider_policy(provider_kind, types::AuthMode::Password, imap, smtp)?;
    let account = db.insert_account(&account_draft(
        email,
        display_name,
        provider_kind,
        types::AuthMode::Password,
        imap,
        smtp,
        username,
    ))?;
    secrets::set(&secrets::key::account_password(&account.id), password)?;
    Ok(account)
}

/// Run the OAuth PKCE browser flow for an OAuth provider, create the account,
/// and persist the tokens (and client settings) for later refresh.
#[allow(clippy::too_many_arguments)]
pub fn create_oauth_account(
    db: &Database,
    email: &str,
    display_name: Option<&str>,
    provider_kind: types::ProviderKind,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    imap: Option<&types::ServerSettings>,
    smtp: Option<&types::ServerSettings>,
    username: Option<&str>,
) -> Result<Account> {
    providers::validate_provider_policy(provider_kind, types::AuthMode::OAuth, imap, smtp)?;
    let config =
        oauth::OAuthConfig::for_kind(provider_kind, client_id, client_secret, redirect_uri)?;
    let session = oauth::start(&config)?;
    open::that(&session.auth_url).context("opening the authorization URL in your browser")?;
    let code = oauth::wait_for_code(&session.redirect_uri, Duration::from_secs(300))?;
    let tokens = oauth::exchange(&config, &code, &session.verifier)?;

    let account = db.insert_account(&account_draft(
        email,
        display_name,
        provider_kind,
        types::AuthMode::OAuth,
        imap,
        smtp,
        username,
    ))?;

    secrets::set(
        &secrets::key::account_oauth_refresh(&account.id),
        tokens.refresh_token.as_deref().unwrap_or_default(),
    )?;
    secrets::set(
        &secrets::key::account_oauth_access(&account.id),
        &tokens.access_token,
    )?;
    db.set_setting(
        &format!("oauth.expires_at.{}", account.id),
        &(Utc::now().timestamp() + tokens.expires_in).to_string(),
    )?;
    // Client settings needed by build_imap_auth / build_smtp_auth for refresh.
    db.set_setting(&format!("oauth.client_id.{}", account.id), client_id)?;
    db.set_setting(
        &format!("oauth.client_secret.{}", account.id),
        client_secret,
    )?;
    db.set_setting(&format!("oauth.redirect_uri.{}", account.id), redirect_uri)?;
    Ok(account)
}

/// Build the account draft shared by password and OAuth creation paths.
fn account_draft(
    email: &str,
    display_name: Option<&str>,
    provider_kind: types::ProviderKind,
    auth_mode: types::AuthMode,
    imap: Option<&types::ServerSettings>,
    smtp: Option<&types::ServerSettings>,
    username: Option<&str>,
) -> AccountDraft {
    AccountDraft {
        email: email.to_string(),
        display_name: display_name.map(str::to_string),
        provider_kind: provider_kind.as_str().to_string(),
        auth_mode: auth_mode.as_str().to_string(),
        username: username.map(str::to_string),
        imap_host: imap.map(|s| s.host.clone()),
        imap_port: imap.map(|s| s.port as i64),
        imap_security: imap.map(|s| s.security.as_str().to_string()),
        smtp_host: smtp.map(|s| s.host.clone()),
        smtp_port: smtp.map(|s| s.port as i64),
        smtp_security: smtp.map(|s| s.security.as_str().to_string()),
    }
}

#[derive(Debug, Clone, Default)]
pub struct SyncSummary {
    pub fetched: usize,
    pub folders: usize,
    pub errors: Vec<String>,
}

fn imap_settings(account: &Account) -> Result<types::ServerSettings> {
    Ok(types::ServerSettings {
        host: account
            .imap_host
            .clone()
            .ok_or_else(|| anyhow!("account {} has no IMAP host", account.email))?,
        port: account.imap_port.unwrap_or(993) as u16,
        security: types::Security::from_str(account.imap_security.as_deref().unwrap_or("tls")),
    })
}

fn smtp_settings(account: &Account) -> Result<types::ServerSettings> {
    Ok(types::ServerSettings {
        host: account
            .smtp_host
            .clone()
            .ok_or_else(|| anyhow!("account {} has no SMTP host", account.email))?,
        port: account.smtp_port.unwrap_or(587) as u16,
        security: types::Security::from_str(account.smtp_security.as_deref().unwrap_or("starttls")),
    })
}

fn build_imap_auth(db: &Database, account: &Account) -> Result<imap::Auth> {
    let username = account
        .username
        .clone()
        .unwrap_or_else(|| account.email.clone());
    match account.auth_mode.as_str() {
        "oauth" => {
            let client_id = db
                .get_setting(&format!("oauth.client_id.{}", account.id))?
                .ok_or_else(|| anyhow!("missing OAuth client id for {}", account.email))?;
            let client_secret = db
                .get_setting(&format!("oauth.client_secret.{}", account.id))?
                .unwrap_or_default();
            let redirect_uri = db
                .get_setting(&format!("oauth.redirect_uri.{}", account.id))?
                .unwrap_or_else(|| "http://127.0.0.1:8765/callback".into());
            let token = oauth_access_token(db, account, &client_id, &client_secret, &redirect_uri)?;
            Ok(imap::Auth::XOAuth2 {
                username,
                access_token: token,
            })
        }
        _ => {
            let password = secrets::get(&secrets::key::account_password(&account.id))?
                .ok_or_else(|| anyhow!("no password stored for {}", account.email))?;
            Ok(imap::Auth::Password { username, password })
        }
    }
}

fn build_smtp_auth(db: &Database, account: &Account) -> Result<smtp::SmtpAuth> {
    let username = account
        .username
        .clone()
        .unwrap_or_else(|| account.email.clone());
    match account.auth_mode.as_str() {
        "oauth" => {
            let client_id = db
                .get_setting(&format!("oauth.client_id.{}", account.id))?
                .ok_or_else(|| anyhow!("missing OAuth client id for {}", account.email))?;
            let client_secret = db
                .get_setting(&format!("oauth.client_secret.{}", account.id))?
                .unwrap_or_default();
            let redirect_uri = db
                .get_setting(&format!("oauth.redirect_uri.{}", account.id))?
                .unwrap_or_else(|| "http://127.0.0.1:8765/callback".into());
            let token = oauth_access_token(db, account, &client_id, &client_secret, &redirect_uri)?;
            Ok(smtp::SmtpAuth::XOAuth2 {
                username,
                access_token: token,
            })
        }
        _ => {
            let password = secrets::get(&secrets::key::account_password(&account.id))?
                .ok_or_else(|| anyhow!("no password stored for {}", account.email))?;
            Ok(smtp::SmtpAuth::Password { username, password })
        }
    }
}

/// Create a local demo account so the shell works with zero configuration
/// (mirrors the previous implementation's seed behavior).
pub fn seed_local_demo(db: &Database) -> Result<()> {
    let account = match db
        .list_accounts()?
        .into_iter()
        .find(|a| a.provider_kind == "local")
    {
        Some(account) => account,
        None => db.insert_account(&crate::db::AccountDraft {
            email: "haruto.sato@komorebi.com".into(),
            display_name: Some("Haruto Sato".into()),
            provider_kind: "local".into(),
            auth_mode: "password".into(),
            ..Default::default()
        })?,
    };

    // Standard folders are upserted idempotently so existing demo databases
    // gain any newly introduced folders on the next launch.
    let inbox = db.upsert_folder(&account.id, "INBOX", Some("Inbox"), Some("inbox"), None)?;
    let sent = db.upsert_folder(&account.id, "Sent", Some("Sent"), Some("sent"), None)?;
    let drafts = db.upsert_folder(&account.id, "Drafts", Some("Drafts"), Some("drafts"), None)?;
    let archive = db.upsert_folder(
        &account.id,
        "Archive",
        Some("Archive"),
        Some("archive"),
        None,
    )?;
    let trash = db.upsert_folder(&account.id, "Trash", Some("Trash"), Some("trash"), None)?;
    let _junk = db.upsert_folder(&account.id, "Junk", Some("Junk"), Some("junk"), None)?;

    // Upgrade path: the original minimal seed is replaced with the rich demo
    // set (matches the intended product experience). Once present, it is stable.
    let existing = db.list_messages(&inbox.id, 1000, 0)?;
    let has_rich = existing
        .iter()
        .any(|m| m.subject.as_deref() == Some("Re: Partnership Proposal"));
    // Re-seed older demo databases that predate threaded conversations.
    let sent_existing = db.list_messages(&sent.id, 1000, 0)?;
    let has_thread_roots = sent_existing
        .iter()
        .any(|m| m.subject.as_deref() == Some("Partnership Proposal"));
    if has_rich && has_thread_roots {
        return Ok(());
    }
    for folder in [&inbox, &sent, &drafts, &archive, &trash] {
        db.delete_all_messages(&folder.id)?;
    }

    struct DemoMsg<'a> {
        sender_name: &'a str,
        sender_email: &'a str,
        subject: &'a str,
        body: &'a str,
        label: &'a str,
        flagged: bool,
        unread: bool,
        seconds_ago: i64,
        attachments: &'a [(&'a str, &'a str)],
        message_id: &'a str,
        in_reply_to: Option<&'a str>,
    }

    let now = Utc::now().timestamp();

    // Sent roots so the demo has real multi-message conversations.
    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        rfc_message_id: Some("partnership-root@komorebi.mail".into()),
        subject: Some("Partnership Proposal".into()),
        sender_email: Some(account.email.clone()),
        body_text: Some(
            "Hi Isabella, please find the partnership proposal attached. I'd welcome your feedback on scope and timeline."
                .into(),
        ),
        date_received: Some(now - 2 * 3600),
        is_read: true,
        ..Default::default()
    })?;
    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        rfc_message_id: Some("landing-root@komorebi.mail".into()),
        subject: Some("Feedback on Landing Page".into()),
        sender_email: Some(account.email.clone()),
        body_text: Some(
            "Hi Ava, here's the latest landing page. Keen to hear your thoughts on the palette and hero."
                .into(),
        ),
        date_received: Some(now - (2 * 86400 + 3600)),
        is_read: true,
        ..Default::default()
    })?;

    let samples: [DemoMsg; 9] = [
        DemoMsg {
            sender_name: "Isabella Martinez",
            sender_email: "isabella.martinez@company.com",
            subject: "Re: Partnership Proposal",
            message_id: "isabella-partnership@company.com",
            in_reply_to: Some("partnership-root@komorebi.mail"),
            body: "Hi Haruto,\n\nThanks for the detailed proposal. I'm excited about the potential partnership with Komorebi.\n\nA few questions before we move forward:\n\n\u{2022} Can you share more about the integration timeline?\n\u{2022} What kind of support will be provided during onboarding?\n\u{2022} Are there any case studies from similar implementations?\n\nLooking forward to your thoughts!\n\nBest regards,\nIsabella",
            label: "Clients",
            flagged: true,
            unread: true,
            seconds_ago: 12 * 60,
            attachments: &[
                ("Partnership Proposal.pdf", "application/pdf"),
                (
                    "Pricing Overview.xlsx",
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                ),
            ],
        },
        DemoMsg {
            sender_name: "Sophia Bennett",
            sender_email: "sophia.bennett@company.com",
            subject: "Q2 Roadmap Alignment",
            message_id: "sophia-q2@company.com",
            in_reply_to: None,
            body: "Hey team, please find the latest Q2 roadmap attached. I'd love your input before we lock the timeline on Friday.",
            label: "Work",
            flagged: false,
            unread: true,
            seconds_ago: 38 * 60,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Liam O'Connor",
            sender_email: "liam.oconnor@company.com",
            subject: "Design System Update",
            message_id: "liam-design@company.com",
            in_reply_to: None,
            body: "We've updated the component library with the new Japandi tokens - softer borders, warmer neutrals, and a calmer focus ring.",
            label: "Work",
            flagged: false,
            unread: false,
            seconds_ago: 66 * 60,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Noah Williams",
            sender_email: "noah.williams@company.com",
            subject: "User Research Insights",
            message_id: "noah-research@company.com",
            in_reply_to: None,
            body: "Here are the key takeaways from this week's interviews. Most people want email to feel quieter, not gamified.",
            label: "Product",
            flagged: false,
            unread: false,
            seconds_ago: 86400,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Emma Johnson",
            sender_email: "emma.johnson@company.com",
            subject: "Meeting Notes - May 20",
            message_id: "emma-notes@company.com",
            in_reply_to: None,
            body: "Thanks everyone for joining. Notes and action items are below - please review and flag anything I missed.",
            label: "Work",
            flagged: false,
            unread: false,
            seconds_ago: 86400 + 7200,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Olivia Davis",
            sender_email: "olivia.davis@company.com",
            subject: "Invoice INV-00234",
            message_id: "olivia-invoice@company.com",
            in_reply_to: None,
            body: "Please find attached the invoice for the March engagement. Let me know if you need any changes before payment.",
            label: "Finance",
            flagged: false,
            unread: false,
            seconds_ago: 86400 + 14400,
            attachments: &[("Invoice INV-00234.pdf", "application/pdf")],
        },
        DemoMsg {
            sender_name: "Ava Thompson",
            sender_email: "ava.thompson@company.com",
            subject: "Re: Feedback on Landing Page",
            message_id: "ava-landing@company.com",
            in_reply_to: Some("landing-root@komorebi.mail"),
            body: "The latest version looks great! The softer palette reads much calmer. One small nit on the hero spacing.",
            label: "Design",
            flagged: false,
            unread: false,
            seconds_ago: 2 * 86400,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "William Anderson",
            sender_email: "william.anderson@company.com",
            subject: "Offsite Agenda",
            message_id: "william-offsite@company.com",
            in_reply_to: None,
            body: "Sharing the agenda for our spring offsite. Tuesday is all-hands, Wednesday is breakout workshops.",
            label: "Work",
            flagged: true,
            unread: false,
            seconds_ago: 2 * 86400 + 3600,
            attachments: &[],
        },
        DemoMsg {
            sender_name: "Mia Garcia",
            sender_email: "mia.garcia@company.com",
            subject: "Project Aurora Brief",
            message_id: "mia-aurora@company.com",
            in_reply_to: None,
            body: "Attached is the project brief for Aurora. The scope and success criteria are on pages 4-6.",
            label: "Product",
            flagged: false,
            unread: false,
            seconds_ago: 3 * 86400,
            attachments: &[("Project Aurora Brief.pdf", "application/pdf")],
        },
    ];

    let mut unread = 0i64;
    for m in samples {
        let snippet = mime::make_snippet(m.body);
        let message_id = db.insert_message(&MessageDraft {
            account_id: account.id.clone(),
            folder_id: inbox.id.clone(),
            subject: Some(m.subject.into()),
            sender_name: Some(m.sender_name.into()),
            sender_email: Some(m.sender_email.into()),
            snippet: Some(snippet),
            body_text: Some(m.body.into()),
            date_received: Some(now - m.seconds_ago),
            is_read: !m.unread,
            is_flagged: m.flagged,
            has_attachments: !m.attachments.is_empty(),
            rfc_message_id: Some(m.message_id.into()),
            in_reply_to: m.in_reply_to.map(str::to_string),
            references_header: m.in_reply_to.map(str::to_string),
            ..Default::default()
        })?;
        if m.unread {
            unread += 1;
        }
        db.set_ai_fields(&message_id, Some(m.label), None)?;
        for (name, mime_type) in m.attachments {
            let data = demo_attachment_bytes(mime_type);
            db.insert_attachment(&message_id, name, Some(mime_type), &data)?;
        }
    }

    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: sent.id.clone(),
        rfc_message_id: Some("haruto-q2-reply@komorebi.mail".into()),
        in_reply_to: Some("sophia-q2@company.com".into()),
        references_header: Some("sophia-q2@company.com".into()),
        subject: Some("Re: Q2 Roadmap Alignment".into()),
        sender_email: Some(account.email.clone()),
        body_text: Some("Thanks Sophia - reviewed the roadmap and left a few comments. The timeline works for me.".into()),
        date_received: Some(now - 20 * 60),
        is_read: true,
        ..Default::default()
    })?;
    db.insert_message(&MessageDraft {
        account_id: account.id.clone(),
        folder_id: drafts.id.clone(),
        subject: Some("Draft: Weekly sync notes".into()),
        body_text: Some("Quick notes for this week's sync...".into()),
        date_received: Some(now - 1800),
        is_read: true,
        ..Default::default()
    })?;

    db.set_folder_counts(&inbox.id, unread, 9)?;
    db.set_folder_counts(&sent.id, 0, 3)?;
    db.set_folder_counts(&drafts.id, 0, 1)?;
    Ok(())
}

/// A tiny placeholder payload so the demo's attachment cards render with real
/// (but non-sensitive) bytes. Not a real document.
fn demo_attachment_bytes(mime_type: &str) -> Vec<u8> {
    if mime_type.contains("spreadsheet") || mime_type.contains("excel") {
        b"PK\x03\x04 demo xlsx placeholder".to_vec()
    } else {
        b"%PDF-1.4 demo pdf placeholder".to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_seed_is_idempotent() {
        let db = Database::in_memory().unwrap();
        seed_local_demo(&db).unwrap();
        seed_local_demo(&db).unwrap();
        assert_eq!(db.list_accounts().unwrap().len(), 1);
        let inbox = db.list_folders(&db.list_accounts().unwrap()[0].id).unwrap();
        assert!(
            inbox
                .iter()
                .any(|f| f.special_use.as_deref() == Some("inbox"))
        );
    }

    #[test]
    fn demo_seed_creates_threads() {
        let db = Database::in_memory().unwrap();
        seed_local_demo(&db).unwrap();
        let account = db.list_accounts().unwrap().remove(0);
        let inbox = db
            .list_folders(&account.id)
            .unwrap()
            .into_iter()
            .find(|f| f.special_use.as_deref() == Some("inbox"))
            .unwrap();
        let reply = db
            .list_messages(&inbox.id, 1000, 0)
            .unwrap()
            .into_iter()
            .find(|m| m.subject.as_deref() == Some("Re: Partnership Proposal"))
            .unwrap();
        let detail = db.get_message_detail(&reply.id).unwrap().unwrap();
        let tid = detail.thread_id.clone().unwrap();
        let thread = db.list_thread_messages(&tid).unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].subject.as_deref(), Some("Partnership Proposal"));
        assert_eq!(
            thread[1].subject.as_deref(),
            Some("Re: Partnership Proposal")
        );
    }

    #[test]
    fn imap_auth_requires_stored_secret() {
        let db = Database::in_memory().unwrap();
        let account = db
            .insert_account(&crate::db::AccountDraft {
                email: "a@b.test".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        // No secret stored → clear error, never a panic.
        let err = build_imap_auth(&db, &account).unwrap_err();
        assert!(err.to_string().contains("no password stored"));
    }

    /// A minimal RFC822 message with one attachment, used to exercise the
    /// cache-ingest path without a live IMAP server.
    const FETCH_BODY: &[u8] = b"From: Jane Doe <jane@example.com>\r\nTo: me@example.org\r\nSubject: Sync ingest\r\nDate: Tue, 4 Jun 2026 10:00:00 +0200\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody from a live fetch.\r\n--B\r\nContent-Type: text/plain; name=\"notes.txt\"\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nfile bytes\r\n--B--\r\n";

    #[test]
    fn cache_fetched_messages_ingests_and_dedupes() {
        let db = Database::in_memory().unwrap();
        let account = db
            .insert_account(&AccountDraft {
                email: "me@example.org".into(),
                provider_kind: "custom".into(),
                auth_mode: "password".into(),
                ..Default::default()
            })
            .unwrap();
        let folder = db
            .upsert_folder(&account.id, "INBOX", None, Some("inbox"), None)
            .unwrap();

        let fetched = [imap::RawFetch {
            uid: 7,
            body: FETCH_BODY.to_vec(),
            seen: false,
            flagged: true,
        }];
        let inserted =
            cache_fetched_messages(&db, &account, &folder.id, Some(42), &fetched).unwrap();
        assert_eq!(inserted, 1);

        let rows = db.list_messages(&folder.id, 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject.as_deref(), Some("Sync ingest"));
        assert_eq!(rows[0].sender_email.as_deref(), Some("jane@example.com"));
        assert!(!rows[0].is_read);
        assert!(rows[0].is_flagged);

        let attachments = db.list_attachments(&rows[0].id).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].display_name, "notes.txt");
        assert_eq!(
            db.get_attachment_data(&attachments[0].id).unwrap().unwrap(),
            b"file bytes"
        );

        // Re-ingesting the same UID is a no-op.
        let again = cache_fetched_messages(&db, &account, &folder.id, Some(42), &fetched).unwrap();
        assert_eq!(again, 0);
        assert_eq!(db.list_messages(&folder.id, 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn account_draft_maps_server_settings() {
        let draft = account_draft(
            "me@example.org",
            Some("Me"),
            types::ProviderKind::Custom,
            types::AuthMode::Password,
            Some(&types::ServerSettings {
                host: "imap.example.org".into(),
                port: 993,
                security: types::Security::Tls,
            }),
            Some(&types::ServerSettings {
                host: "smtp.example.org".into(),
                port: 587,
                security: types::Security::StartTls,
            }),
            None,
        );
        assert_eq!(draft.email, "me@example.org");
        assert_eq!(draft.provider_kind, "custom");
        assert_eq!(draft.auth_mode, "password");
        assert_eq!(draft.imap_host.as_deref(), Some("imap.example.org"));
        assert_eq!(draft.imap_security.as_deref(), Some("tls"));
        assert_eq!(draft.smtp_host.as_deref(), Some("smtp.example.org"));
        assert_eq!(draft.smtp_security.as_deref(), Some("starttls"));
    }

    #[test]
    fn password_account_policy_refuses_gmail() {
        // Policy check runs before any secret is written, so this can be
        // tested with an in-memory DB and no keyring.
        let db = Database::in_memory().unwrap();
        let gmail = providers::preset(types::ProviderKind::Gmail).unwrap();
        let err = create_password_account(
            &db,
            "me@gmail.com",
            None,
            types::ProviderKind::Gmail,
            gmail.imap.as_ref(),
            gmail.smtp.as_ref(),
            None,
            "secret",
        )
        .unwrap_err();
        assert!(err.to_string().contains("OAuth"));
    }
}
