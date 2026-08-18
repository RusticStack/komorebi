//! IMAP client wrapper (sync, on OS threads).
//!
//! Supports TLS and STARTTLS, password login and XOAUTH2 SASL for OAuth
//! providers, folder listing with special-use attributes, incremental UID
//! sync, and flag updates. Provider-safe posture: no destructive mutations
//! (expunge, move, delete) are exposed yet.

use super::types::{Security, ServerSettings};
use anyhow::{Context, Result, anyhow};
use imap::types::{Flag, Mailbox, Name, NameAttribute};
use imap::{Client, Session};
use native_tls::TlsConnector;
use std::net::TcpStream;

#[derive(Debug)]
pub enum Auth {
    Password {
        username: String,
        password: String,
    },
    XOAuth2 {
        username: String,
        access_token: String,
    },
}

/// Minimal XOAUTH2 SASL authenticator for the `imap` crate's generic
/// `AUTHENTICATE` handshake.
struct XOAuth2Authenticator {
    username: String,
    access_token: String,
}

impl imap::Authenticator for XOAuth2Authenticator {
    type Response = Vec<u8>;

    fn process(&self, _challenge: &[u8]) -> Vec<u8> {
        format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.username, self.access_token
        )
        .into_bytes()
    }
}

/// A live IMAP connection bound to one mailbox folder.
pub struct ImapConnection {
    session: Session<native_tls::TlsStream<TcpStream>>,
    pub folder: String,
    pub mailbox: Mailbox,
}

pub fn connect(
    settings: &ServerSettings,
    tls_domain: &str,
) -> Result<Client<native_tls::TlsStream<TcpStream>>> {
    let tls = TlsConnector::builder()
        .build()
        .context("building TLS connector")?;
    let addr = (settings.host.as_str(), settings.port);
    match settings.security {
        Security::Tls => imap::connect(addr, tls_domain, &tls).context("IMAP TLS connect failed"),
        Security::StartTls => {
            imap::connect_starttls(addr, tls_domain, &tls).context("IMAP STARTTLS connect failed")
        }
        Security::None => Err(anyhow!(
            "IMAP requires TLS or STARTTLS; refusing a plaintext connection"
        )),
    }
}

pub fn authenticate(
    client: Client<native_tls::TlsStream<TcpStream>>,
    auth: &Auth,
) -> Result<Session<native_tls::TlsStream<TcpStream>>> {
    match auth {
        Auth::Password { username, password } => client
            .login(username, password)
            .map_err(|(e, _)| e)
            .context("IMAP LOGIN failed"),
        Auth::XOAuth2 {
            username,
            access_token,
        } => client
            .authenticate(
                "XOAUTH2",
                &XOAuth2Authenticator {
                    username: username.clone(),
                    access_token: access_token.clone(),
                },
            )
            .map_err(|(e, _)| e)
            .context("IMAP XOAUTH2 failed"),
    }
}

impl ImapConnection {
    pub fn open(
        settings: &ServerSettings,
        tls_domain: &str,
        auth: &Auth,
        folder: &str,
    ) -> Result<Self> {
        let client = connect(settings, tls_domain)?;
        let mut session = authenticate(client, auth)?;
        let mailbox = session
            .select(folder)
            .with_context(|| format!("selecting folder {folder}"))?;
        Ok(Self {
            session,
            folder: folder.to_string(),
            mailbox,
        })
    }

    /// List all selectable folders with RFC 6154 special-use attributes.
    pub fn list_folders(
        settings: &ServerSettings,
        tls_domain: &str,
        auth: &Auth,
    ) -> Result<Vec<FolderInfo>> {
        let client = connect(settings, tls_domain)?;
        let mut session = authenticate(client, auth)?;
        let names = session
            .list(Some(""), Some("*"))
            .context("IMAP LIST failed")?;
        let mut out = Vec::new();
        for name in names.iter() {
            if name
                .attributes()
                .iter()
                .any(|a| matches!(a, NameAttribute::NoSelect))
            {
                continue;
            }
            out.push(FolderInfo {
                name: name.name().to_string(),
                delimiter: name.delimiter().map(|d| d.to_string()),
                special_use: special_use(&name),
            });
        }
        session.logout().ok();
        Ok(out)
    }

    /// Fetch messages with UID > `since_uid`, extracting the data we cache.
    /// Pass `since_uid = 0` for a full initial fetch (capped by `limit`).
    pub fn fetch_new(&mut self, since_uid: u32, limit: Option<u32>) -> Result<Vec<RawFetch>> {
        let start = since_uid.saturating_add(1);
        let range = match limit {
            Some(n) if n > 0 => format!("{start}:{}", start.saturating_add(n).saturating_sub(1)),
            _ => format!("{start}:*"),
        };
        let result = self
            .session
            .uid_fetch(range, "(UID FLAGS BODY.PEEK[])")
            .map_err(|e| anyhow!("IMAP UID FETCH failed: {e}"))?;
        Ok(result.iter().map(RawFetch::from).collect())
    }

    /// Fetch a single message by UID.
    pub fn fetch_one(&mut self, uid: u32) -> Result<RawFetch> {
        let result = self
            .session
            .uid_fetch(uid.to_string(), "(UID FLAGS BODY.PEEK[])")
            .map_err(|e| anyhow!("IMAP UID FETCH failed: {e}"))?;
        let mut it = result.iter();
        let fetch = it
            .next()
            .ok_or_else(|| anyhow!("no message with UID {uid}"))?;
        Ok(RawFetch::from(fetch))
    }

    pub fn set_seen(&mut self, uid: u32, seen: bool) -> Result<()> {
        let op = if seen {
            "+FLAGS.SILENT"
        } else {
            "-FLAGS.SILENT"
        };
        self.session
            .uid_store(uid.to_string(), format!("({op} (\\Seen))"))
            .map_err(|e| anyhow!("IMAP STORE \\Seen failed: {e}"))?;
        Ok(())
    }

    pub fn set_flagged(&mut self, uid: u32, flagged: bool) -> Result<()> {
        let op = if flagged {
            "+FLAGS.SILENT"
        } else {
            "-FLAGS.SILENT"
        };
        self.session
            .uid_store(uid.to_string(), format!("({op} (\\Flagged))"))
            .map_err(|e| anyhow!("IMAP STORE \\Flagged failed: {e}"))?;
        Ok(())
    }

    pub fn logout(mut self) {
        self.session.logout().ok();
    }
}

#[derive(Debug, Clone)]
pub struct FolderInfo {
    pub name: String,
    pub delimiter: Option<String>,
    pub special_use: Option<String>,
}

fn special_use(name: &Name) -> Option<String> {
    for attr in name.attributes() {
        if let NameAttribute::Custom(value) = attr {
            let lower = value.to_ascii_lowercase();
            let special = match lower.as_str() {
                "\\sent" => Some("sent"),
                "\\drafts" => Some("drafts"),
                "\\trash" => Some("trash"),
                "\\junk" => Some("junk"),
                "\\archive" => Some("archive"),
                "\\all" => Some("all"),
                "\\flagged" => Some("flagged"),
                _ => None,
            };
            if let Some(s) = special {
                return Some(s.into());
            }
        }
    }
    special_use_from_name(name.name())
}

/// Name-based special-use heuristic fallback (INBOX handled by callers).
fn special_use_from_name(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    if lower.ends_with("/sent") || lower.ends_with("sent items") || lower == "sent" {
        return Some("sent".into());
    }
    if lower.ends_with("/drafts") || lower == "drafts" {
        return Some("drafts".into());
    }
    if lower.ends_with("/trash") || lower == "trash" || lower == "deleted items" {
        return Some("trash".into());
    }
    if lower.ends_with("/junk") || lower == "junk" || lower == "spam" || lower == "junk email" {
        return Some("junk".into());
    }
    if lower.ends_with("/archive") || lower == "archive" {
        return Some("archive".into());
    }
    None
}

/// The subset of a fetched message we persist to the cache.
#[derive(Debug, Clone)]
pub struct RawFetch {
    pub uid: u32,
    pub body: Vec<u8>,
    pub seen: bool,
    pub flagged: bool,
}

impl RawFetch {
    pub fn from(fetch: &imap::types::Fetch) -> Self {
        let flags = fetch.flags();
        Self {
            uid: fetch.uid.unwrap_or(0),
            body: fetch.body().unwrap_or_default().to_vec(),
            seen: flags.iter().any(|f| *f == Flag::Seen),
            flagged: flags.iter().any(|f| *f == Flag::Flagged),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoauth2_payload_format() {
        use imap::Authenticator as _;
        let auth = XOAuth2Authenticator {
            username: "me@example.com".into(),
            access_token: "tok123".into(),
        };
        let payload = String::from_utf8(auth.process(b"")).unwrap();
        assert_eq!(payload, "user=me@example.com\x01auth=Bearer tok123\x01\x01");
    }

    #[test]
    fn special_use_name_heuristics() {
        assert_eq!(special_use_from_name("Sent Items"), Some("sent".into()));
        assert_eq!(special_use_from_name("Archive"), Some("archive".into()));
        assert_eq!(special_use_from_name("Deleted Items"), Some("trash".into()));
        assert_eq!(special_use_from_name("Junk Email"), Some("junk".into()));
        assert_eq!(special_use_from_name("Projects/Team"), None);
    }

    #[test]
    fn refuses_plaintext_imap() {
        let settings = ServerSettings {
            host: "imap.example.com".into(),
            port: 143,
            security: Security::None,
        };
        let err = connect(&settings, "imap.example.com").unwrap_err();
        assert!(err.to_string().contains("plaintext"));
    }
}
