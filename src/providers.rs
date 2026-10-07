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
    pub imap: (&'static str, u16),
    pub smtp: (&'static str, u16),
}

/// Presets in the order of the account form's provider list (Purelymail,
/// Postale.io); "Other (manual)" follows them in the UI.
pub const PRESETS: &[Preset] = &[
    Preset {
        imap: ("imap.purelymail.com", 993),
        smtp: ("smtp.purelymail.com", 465),
    },
    Preset {
        imap: ("mail.postale.io", 993),
        smtp: ("mail.postale.io", 465),
    },
];

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
}
