//! Mail domain types shared across providers, sync, send and AI features.

/// Provider kinds. Custom/self-hosted hosting is the primary focus; Gmail and
/// Microsoft are supported through OAuth (password auth is refused for them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Generic IMAP/SMTP - any self-hosted or custom-hosting server.
    Custom,
    /// Mailcow / mailcow-dockerized.
    Mailcow,
    /// Zimbra Collaboration Suite.
    Zimbra,
    /// Poste.io.
    Posteio,
    /// Gmail (OAuth only).
    Gmail,
    /// Microsoft 365 / Outlook (OAuth only).
    Microsoft,
    /// Local demo account (no network).
    Local,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Mailcow => "mailcow",
            Self::Zimbra => "zimbra",
            Self::Posteio => "posteio",
            Self::Gmail => "gmail",
            Self::Microsoft => "microsoft",
            Self::Local => "local",
        }
    }

    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "mailcow" => Self::Mailcow,
            "zimbra" => Self::Zimbra,
            "posteio" => Self::Posteio,
            "gmail" => Self::Gmail,
            "microsoft" => Self::Microsoft,
            "local" => Self::Local,
            _ => Self::Custom,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Custom => "Custom / self-hosted",
            Self::Mailcow => "Mailcow",
            Self::Zimbra => "Zimbra",
            Self::Posteio => "Poste.io",
            Self::Gmail => "Gmail",
            Self::Microsoft => "Microsoft 365",
            Self::Local => "Local demo",
        }
    }

    pub fn requires_oauth(&self) -> bool {
        matches!(self, Self::Gmail | Self::Microsoft)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Password,
    OAuth,
}

impl AuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::OAuth => "oauth",
        }
    }
    pub fn parse_or_default(s: &str) -> Self {
        if s == "oauth" {
            Self::OAuth
        } else {
            Self::Password
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    None,
    StartTls,
    Tls,
}

impl Security {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::StartTls => "starttls",
            Self::Tls => "tls",
        }
    }
    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "starttls" => Self::StartTls,
            "tls" => Self::Tls,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// Parsed envelope of an incoming message (from MIME parsing or sync).
#[derive(Debug, Clone, Default)]
pub struct ParsedMessage {
    pub subject: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub snippet: Option<String>,
    pub attachments: Vec<ParsedAttachment>,
}

#[derive(Debug, Clone)]
pub struct ParsedAttachment {
    pub display_name: String,
    pub mime: Option<String>,
    pub data: Vec<u8>,
}

/// Result of a folder sync pass.
#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub folder: String,
    pub fetched: usize,
    pub updated: usize,
    pub errors: Vec<String>,
}
