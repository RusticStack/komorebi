//! Built-in provider presets. Picking one fills in every server setting, so
//! adding an account needs only an address and a password.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Implicit TLS (IMAP 993, SMTP 465).
    Tls,
    /// Plain connect upgraded with STARTTLS (SMTP 587).
    StartTls,
}

impl Security {
    pub fn as_i64(self) -> i64 {
        match self {
            Self::Tls => 1,
            Self::StartTls => 0,
        }
    }
    pub fn from_i64(v: i64) -> Self {
        if v == 0 { Self::StartTls } else { Self::Tls }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

pub struct Preset {
    /// The name the account form shows (a brand: not translated).
    pub name: &'static str,
    /// The provider's own mail domain: an address there is its account.
    pub domain: &'static str,
    pub imap: (&'static str, u16),
    pub smtp: (&'static str, u16),
    /// Whether a new account on this preset saves a copy of what it sends
    /// to its Sent folder. SMTP never does that by itself, so it is on
    /// unless the provider is known to save sent mail on the server.
    /// Postale.io is known (from live tests) not to, so it is on;
    /// Purelymail has no documentation either way, so it keeps the
    /// general default (on), and a duplicate is avoided by the Message-ID
    /// lookup of the Sent folder. Existing accounts keep their stored choice.
    pub save_sent: bool,
}

/// Presets in the order of the account form's provider list (Purelymail,
/// Postale.io); "Other server" follows them in the UI.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Purelymail",
        domain: "purelymail.com",
        imap: ("imap.purelymail.com", 993),
        smtp: ("smtp.purelymail.com", 465),
        save_sent: true,
    },
    Preset {
        name: "Postale.io",
        domain: "postale.io",
        imap: ("mail.postale.io", 993),
        smtp: ("mail.postale.io", 465),
        save_sent: true,
    },
];

/// The preset an address's domain names: an address at purelymail.com or
/// postale.io is that provider's (case and spaces aside). Any other domain
/// (a custom one hosted anywhere) names none: the user picks.
pub fn guess(email: &str) -> Option<usize> {
    let (_, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.trim_end_matches('.');
    PRESETS
        .iter()
        .position(|p| p.domain.eq_ignore_ascii_case(domain))
}

/// Whether `email` is on preset `index`'s own domain.
pub fn on_own_domain(index: usize, email: &str) -> bool {
    guess(email) == Some(index)
}

/// The default of "save a copy to Sent" for a new account on preset
/// `index` (`None`: another server, where the general default, on, holds).
pub fn save_sent_default(index: Option<usize>) -> bool {
    index
        .and_then(|i| PRESETS.get(i))
        .is_none_or(|p| p.save_sent)
}

impl Preset {
    pub fn imap_server(&self) -> Server {
        Server {
            host: self.imap.0.into(),
            port: self.imap.1,
            security: Security::Tls,
        }
    }
    pub fn smtp_server(&self) -> Server {
        Server {
            host: self.smtp.0.into(),
            port: self.smtp.1,
            security: Security::Tls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_use_implicit_tls() {
        for p in PRESETS {
            assert_eq!(p.imap_server().security, Security::Tls);
            assert_eq!(p.imap.1, 993);
            assert_eq!(p.smtp.1, 465);
        }
        assert_eq!(PRESETS[0].imap.0, "imap.purelymail.com");
        assert_eq!(PRESETS[0].smtp.0, "smtp.purelymail.com");
        assert_eq!(PRESETS[1].imap.0, "mail.postale.io");
        assert_eq!(PRESETS[1].smtp.0, "mail.postale.io");
    }

    #[test]
    fn sent_copy_defaults_follow_the_preset() {
        // Postale.io does not save sent mail itself (live tests): on.
        assert!(PRESETS[1].save_sent);
        // Purelymail is undocumented: the general default.
        assert!(PRESETS[0].save_sent);
        assert!(save_sent_default(None));
        assert!(save_sent_default(Some(1)));
        assert!(save_sent_default(Some(99)));
    }

    #[test]
    fn the_provider_is_guessed_from_its_own_domain_only() {
        assert_eq!(guess("ana@purelymail.com"), Some(0));
        assert_eq!(guess(" Ana@PurelyMail.COM "), Some(0));
        assert_eq!(guess("ana@postale.io"), Some(1));
        assert_eq!(guess("ana@postale.io."), Some(1));
        // A custom domain can be hosted by either (or anyone): no guess.
        assert_eq!(guess("ana@example.com"), None);
        assert_eq!(guess("ana@mail.purelymail.com"), None);
        assert_eq!(guess("ana@purelymail.com.evil.example"), None);
        assert_eq!(guess("purelymail.com"), None);
        assert_eq!(guess(""), None);
        assert_eq!(guess("ana@"), None);
        assert!(on_own_domain(1, "a@postale.io"));
        assert!(!on_own_domain(0, "a@postale.io"));
    }
}
