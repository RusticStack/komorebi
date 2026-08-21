//! Provider registry.
//!
//! Komorebi is built for custom/self-hosted hosting first: generic IMAP/SMTP
//! with sensible presets for common self-hosted stacks, full manual control,
//! and standards-based autodiscovery (`src/mail/discovery.rs`). Gmail and
//! Microsoft are supported through OAuth only - password setup is refused for
//! them, matching the previous implementation's provider policy.

pub use super::types::{AuthMode, ProviderKind, Security, ServerSettings};
use anyhow::{Result, bail};

pub struct ProviderPreset {
    pub kind: ProviderKind,
    pub label: &'static str,
    pub auth_mode: AuthMode,
    /// Suggested IMAP settings (None = discover or ask).
    pub imap: Option<ServerSettings>,
    /// Suggested SMTP settings.
    pub smtp: Option<ServerSettings>,
    pub scopes: &'static [&'static str],
    pub notes: &'static [&'static str],
}

/// All selectable providers, self-hosted first.
pub fn presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            kind: ProviderKind::Custom,
            label: "Custom / self-hosted (any IMAP+SMTP)",
            auth_mode: AuthMode::Password,
            imap: None,
            smtp: None,
            scopes: &[],
            notes: &[
                "Works with any server exposing IMAP and SMTP.",
                "Use autodiscovery or enter server settings manually.",
            ],
        },
        ProviderPreset {
            kind: ProviderKind::Mailcow,
            label: "Mailcow (mailcow-dockerized)",
            auth_mode: AuthMode::Password,
            imap: Some(ServerSettings {
                host: "mail.example.com".into(),
                port: 993,
                security: Security::Tls,
            }),
            smtp: Some(ServerSettings {
                host: "mail.example.com".into(),
                port: 587,
                security: Security::StartTls,
            }),
            scopes: &[],
            notes: &[
                "Use your mailcow domain or hostname.",
                "App passwords are not required; normal mailbox passwords work.",
            ],
        },
        ProviderPreset {
            kind: ProviderKind::Zimbra,
            label: "Zimbra Collaboration",
            auth_mode: AuthMode::Password,
            imap: Some(ServerSettings {
                host: "mail.example.com".into(),
                port: 993,
                security: Security::Tls,
            }),
            smtp: Some(ServerSettings {
                host: "mail.example.com".into(),
                port: 587,
                security: Security::StartTls,
            }),
            scopes: &[],
            notes: &["Zimbra supports standard IMAP and SMTP."],
        },
        ProviderPreset {
            kind: ProviderKind::Posteio,
            label: "Poste.io",
            auth_mode: AuthMode::Password,
            imap: Some(ServerSettings {
                host: "mail.example.com".into(),
                port: 993,
                security: Security::Tls,
            }),
            smtp: Some(ServerSettings {
                host: "mail.example.com".into(),
                port: 587,
                security: Security::StartTls,
            }),
            scopes: &[],
            notes: &["Poste.io exposes standard IMAP/SMTP."],
        },
        ProviderPreset {
            kind: ProviderKind::Gmail,
            label: "Gmail (OAuth)",
            auth_mode: AuthMode::OAuth,
            imap: Some(ServerSettings {
                host: "imap.gmail.com".into(),
                port: 993,
                security: Security::Tls,
            }),
            smtp: Some(ServerSettings {
                host: "smtp.gmail.com".into(),
                port: 587,
                security: Security::StartTls,
            }),
            scopes: &["https://mail.google.com/", "openid", "email"],
            notes: &[
                "Password/app-password setup is refused for Gmail.",
                "You need your own Google Cloud OAuth client (client id/secret).",
            ],
        },
        ProviderPreset {
            kind: ProviderKind::Microsoft,
            label: "Microsoft 365 / Outlook (OAuth)",
            auth_mode: AuthMode::OAuth,
            imap: Some(ServerSettings {
                host: "outlook.office365.com".into(),
                port: 993,
                security: Security::Tls,
            }),
            smtp: Some(ServerSettings {
                host: "smtp.office365.com".into(),
                port: 587,
                security: Security::StartTls,
            }),
            scopes: &[
                "offline_access",
                "https://outlook.office.com/IMAP.AccessAsUser.All",
                "https://outlook.office.com/SMTP.Send",
            ],
            notes: &[
                "Password setup is refused for Microsoft.",
                "Requires an Azure app registration (client id/secret).",
            ],
        },
    ]
}

pub fn preset(kind: ProviderKind) -> Option<ProviderPreset> {
    presets().into_iter().find(|p| p.kind == kind)
}

/// Validate that an account configuration is safe for the provider kind.
/// Mirrors the previous implementation's provider policy enforcement.
pub fn validate_provider_policy(
    kind: ProviderKind,
    auth_mode: AuthMode,
    imap: Option<&ServerSettings>,
    smtp: Option<&ServerSettings>,
) -> Result<()> {
    if kind.requires_oauth() && auth_mode != AuthMode::OAuth {
        bail!(
            "{} only supports OAuth authentication; password setup is refused.",
            kind.label()
        );
    }
    for (label, settings) in [("IMAP", imap), ("SMTP", smtp)] {
        if let Some(s) = settings
            && s.security == Security::None
        {
            bail!(
                "{label} for {} requires an encrypted connection (TLS or STARTTLS).",
                kind.label()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_refuses_gmail_password() {
        let gmail = preset(ProviderKind::Gmail).unwrap();
        let err = validate_provider_policy(
            gmail.kind,
            AuthMode::Password,
            gmail.imap.as_ref(),
            gmail.smtp.as_ref(),
        );
        assert!(err.is_err());
        assert!(
            validate_provider_policy(
                gmail.kind,
                AuthMode::OAuth,
                gmail.imap.as_ref(),
                gmail.smtp.as_ref(),
            )
            .is_ok()
        );
    }

    #[test]
    fn policy_refuses_plaintext_custom() {
        let custom = preset(ProviderKind::Custom).unwrap();
        let plain = ServerSettings {
            host: "imap.example.com".into(),
            port: 143,
            security: Security::None,
        };
        let err = validate_provider_policy(
            custom.kind,
            AuthMode::Password,
            Some(&plain),
            custom.smtp.as_ref(),
        );
        assert!(err.is_err());
    }

    #[test]
    fn presets_are_sorted_self_hosted_first() {
        let list = presets();
        let kinds: Vec<ProviderKind> = list.iter().map(|p| p.kind).collect();
        assert_eq!(kinds[0], ProviderKind::Custom);
        assert_eq!(kinds[1], ProviderKind::Mailcow);
    }
}
