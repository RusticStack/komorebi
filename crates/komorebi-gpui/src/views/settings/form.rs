//! The account form as plain values (the Slint app's `AccountForm` struct
//! and `form_servers`, `form_preset`, `form_logins`, `form_no_auth`): what
//! the Settings > Accounts form and the onboarding Sign in step hold, and
//! the servers and sign-in names they make. No window code here, so the
//! rules are tested as plain functions.

use komorebi::db::{Account, NewAccount};
use komorebi::providers::{self, Security, Server};

use super::hosts::{FormError, Proto, host_for_display, parse_port, validate_host};

/// The provider a form chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// One of [`providers::PRESETS`] whose servers are fixed (not the
    /// Bridge, not a provider Komorebi cannot sign in to).
    Preset(usize),
    /// Other server: the servers are typed.
    Other,
}

/// What the account form holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountForm {
    /// 0: the form adds an account.
    pub id: i64,
    /// `None`: none chosen yet.
    pub provider: Option<Provider>,
    pub name: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_starttls: bool,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_starttls: bool,
    pub save_sent: bool,
    pub signature: String,
    pub imap_user: String,
    pub smtp_user: String,
    pub smtp_own_login: bool,
    pub smtp_no_auth: bool,
}

impl Default for AccountForm {
    /// The first preset chosen, everything else empty (as the Slint
    /// struct's default).
    fn default() -> Self {
        Self {
            id: 0,
            provider: Some(Provider::Preset(0)),
            name: String::new(),
            email: String::new(),
            password: String::new(),
            imap_host: String::new(),
            imap_port: String::new(),
            imap_starttls: false,
            smtp_host: String::new(),
            smtp_port: String::new(),
            smtp_starttls: false,
            save_sent: false,
            signature: String::new(),
            imap_user: String::new(),
            smtp_user: String::new(),
            smtp_own_login: false,
            smtp_no_auth: false,
        }
    }
}

impl AccountForm {
    /// A blank form for a new account, Other server.
    pub fn blank() -> Self {
        Self {
            provider: None,
            save_sent: true,
            imap_port: "993".into(),
            smtp_port: "465".into(),
            ..Self::default()
        }
    }

    /// The form of an account being edited: its settings, never its
    /// password (the Slint app's `edit_account`).
    pub fn of(a: &Account) -> Self {
        Self {
            id: a.id,
            provider: Some(Provider::Other),
            name: a.name.clone(),
            email: a.email.clone(),
            password: String::new(),
            imap_host: host_for_display(&a.imap.host),
            imap_port: a.imap.port.to_string(),
            imap_starttls: a.imap.security == Security::StartTls,
            smtp_host: host_for_display(&a.smtp.host),
            smtp_port: a.smtp.port.to_string(),
            smtp_starttls: a.smtp.security == Security::StartTls,
            save_sent: a.save_sent,
            signature: a.signature.clone(),
            imap_user: a.imap_user.clone(),
            smtp_user: a.smtp_user.clone(),
            smtp_own_login: a.smtp_user != a.imap_user,
            smtp_no_auth: a.smtp_no_auth,
        }
    }

    /// The preset the form chose (`None`: Other server, or none chosen yet).
    pub fn preset(&self) -> Option<usize> {
        match self.provider {
            Some(Provider::Preset(i))
                if providers::PRESETS.get(i).is_some_and(|p| p.is_fixed()) =>
            {
                Some(i)
            }
            _ => None,
        }
    }

    /// A manual form (not one of the presets): its certificates can be
    /// reviewed and pinned.
    pub fn is_manual(&self) -> bool {
        self.preset().is_none()
    }

    /// The servers the form names: a preset's, or the ones typed.
    pub fn servers(&self) -> Result<(Server, Server), FormError> {
        let Some(provider) = self.provider else {
            return Err(FormError::Provider);
        };
        if matches!(provider, Provider::Preset(_))
            && let Some(p) = self.preset().and_then(|i| providers::PRESETS.get(i))
        {
            return Ok((p.imap_server(), p.smtp_server()));
        }
        let port = |s: &str, proto| parse_port(s).ok_or(FormError::Port(proto));
        let security = |starttls: bool| {
            if starttls {
                Security::StartTls
            } else {
                Security::Tls
            }
        };
        let (imap_host, smtp_host) = (self.imap_host.trim(), self.smtp_host.trim());
        if imap_host.is_empty() || smtp_host.is_empty() {
            return Err(FormError::ServerNames);
        }
        let (imap_host, smtp_host) = (validate_host(imap_host)?, validate_host(smtp_host)?);
        Ok((
            Server {
                host: imap_host,
                port: port(&self.imap_port, Proto::Imap)?,
                security: security(self.imap_starttls),
            },
            Server {
                host: smtp_host,
                port: port(&self.smtp_port, Proto::Smtp)?,
                security: security(self.smtp_starttls),
            },
        ))
    }

    /// The sign-in names the form holds, as stored: a name equal to the
    /// address is empty, and the SMTP one follows the IMAP one unless the
    /// form has its own. A preset keeps the username typed under the
    /// password ("Set username"), for both servers: its separate SMTP field
    /// is not shown.
    pub fn logins(&self, email: &str) -> Result<(String, String), FormError> {
        let preset = self.preset().is_some();
        let clean = |raw: &str, what: Proto| -> Result<String, FormError> {
            let user = raw.trim();
            if user.chars().any(char::is_control) || user.len() > 254 {
                return Err(FormError::Username(what));
            }
            Ok(if user == email {
                String::new()
            } else {
                user.into()
            })
        };
        let imap = clean(&self.imap_user, Proto::Imap)?;
        let smtp = if self.smtp_own_login && !preset {
            clean(&self.smtp_user, Proto::Smtp)?
        } else {
            imap.clone()
        };
        Ok((imap, smtp))
    }

    /// Whether the form's SMTP server takes mail without a sign-in: only a
    /// manual server can (a preset always signs in).
    pub fn no_auth(&self) -> bool {
        self.smtp_no_auth && self.preset().is_none()
    }

    /// The new account the form makes (the Slint app's `add_account`).
    pub fn new_account(&self) -> Result<NewAccount, FormError> {
        let email = self.email.trim().to_string();
        if !email.contains('@') {
            return Err(FormError::Email);
        }
        let (imap, smtp) = self.servers()?;
        let (imap_user, smtp_user) = self.logins(&email)?;
        Ok(NewAccount {
            email,
            name: self.name.trim().to_string(),
            imap,
            smtp,
            // A preset's own default can only turn the copy off.
            save_sent: self.save_sent && providers::save_sent_default(self.preset()),
            imap_user,
            smtp_user,
            smtp_no_auth: self.no_auth(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual() -> AccountForm {
        AccountForm {
            provider: Some(Provider::Other),
            imap_host: " mail.example.net ".into(),
            imap_port: "143".into(),
            imap_starttls: true,
            smtp_host: "mail.example.net".into(),
            smtp_port: "465".into(),
            ..Default::default()
        }
    }

    /// Ports `account_forms_name_their_servers` (src/app.rs).
    #[test]
    fn account_forms_name_their_servers() {
        let manual = manual();
        let (imap, smtp) = manual.servers().unwrap();
        assert_eq!(
            (imap.host.as_str(), imap.port, imap.security),
            ("mail.example.net", 143, Security::StartTls)
        );
        assert_eq!((smtp.port, smtp.security), (465, Security::Tls));
        let preset = AccountForm::default().servers().unwrap();
        assert_eq!(preset.0, providers::PRESETS[0].imap_server());
        for port in ["0", "x"] {
            let bad = AccountForm {
                imap_port: port.into(),
                ..manual.clone()
            };
            assert_eq!(bad.servers().unwrap_err(), FormError::Port(Proto::Imap));
        }
        let no_host = AccountForm {
            smtp_host: " ".into(),
            ..manual.clone()
        };
        assert!(no_host.servers().is_err());
        // No provider chosen yet.
        let none = AccountForm {
            provider: None,
            ..manual.clone()
        };
        assert_eq!(none.servers().unwrap_err(), FormError::Provider);

        // Host names are checked: the name only, no scheme, port or path.
        for host in [
            "imap://mail.example.net",
            "mail.example.net:993",
            "a/b",
            "me@x",
        ] {
            let bad = AccountForm {
                imap_host: host.into(),
                ..manual.clone()
            };
            assert_eq!(bad.servers().unwrap_err(), FormError::ServerNameOnly);
        }
        // Normalised on the way in: case, a trailing dot, an IPv6 literal.
        let odd = AccountForm {
            imap_host: "[::1]".into(),
            smtp_host: "Mail.Example.NET.".into(),
            ..manual.clone()
        };
        let (imap, smtp) = odd.servers().unwrap();
        assert_eq!(
            (imap.host.as_str(), smtp.host.as_str()),
            ("::1", "mail.example.net")
        );
        // Custom ports are fine for either security (Proton Bridge, 2525).
        let bridge = AccountForm {
            imap_port: "1143".into(),
            smtp_port: "2525".into(),
            smtp_starttls: true,
            ..manual
        };
        let (imap, smtp) = bridge.servers().unwrap();
        assert_eq!(
            (imap.port, smtp.port, smtp.security),
            (1143, 2525, Security::StartTls)
        );
    }

    /// Ports `form_logins_follow_the_form` (src/app/settings.rs).
    #[test]
    fn form_logins_follow_the_form() {
        let manual = AccountForm {
            imap_user: " alice ".into(),
            ..manual()
        };
        let logins = |f: &AccountForm| f.logins("alice@example.com");
        // The SMTP login follows the IMAP one unless the form has its own.
        assert_eq!(logins(&manual), Ok(("alice".into(), "alice".into())));
        let own = AccountForm {
            smtp_own_login: true,
            smtp_user: "relay".into(),
            ..manual.clone()
        };
        assert_eq!(logins(&own), Ok(("alice".into(), "relay".into())));
        // An own SMTP login left empty is the address.
        let blank = AccountForm {
            smtp_own_login: true,
            smtp_user: " ".into(),
            ..manual.clone()
        };
        assert_eq!(logins(&blank), Ok(("alice".into(), String::new())));
        // Empty, or the address itself, stores nothing.
        let none = AccountForm {
            imap_user: "alice@example.com".into(),
            ..manual.clone()
        };
        assert_eq!(logins(&none), Ok((String::new(), String::new())));
        // A preset keeps a username typed under the password (Set
        // username), for both servers; the SMTP field it doesn't show is
        // ignored.
        let preset = AccountForm {
            provider: Some(Provider::Preset(0)),
            ..own.clone()
        };
        assert_eq!(logins(&preset), Ok(("alice".into(), "alice".into())));
        let preset_plain = AccountForm {
            provider: Some(Provider::Preset(1)),
            imap_user: String::new(),
            ..own.clone()
        };
        assert_eq!(logins(&preset_plain), Ok((String::new(), String::new())));
        let bad = AccountForm {
            imap_user: "a\nb".into(),
            ..manual
        };
        assert_eq!(logins(&bad), Err(FormError::Username(Proto::Imap)));
    }

    #[test]
    fn a_new_account_needs_an_address_and_only_a_manual_server_can_skip_sign_in() {
        let mut form = AccountForm {
            email: " ana@example.net ".into(),
            smtp_no_auth: true,
            save_sent: true,
            ..manual()
        };
        let new = form.new_account().unwrap();
        assert_eq!(new.email, "ana@example.net");
        assert!(new.smtp_no_auth);
        form.provider = Some(Provider::Preset(0));
        assert!(!form.new_account().unwrap().smtp_no_auth);
        form.email = "ana".into();
        assert_eq!(form.new_account().unwrap_err(), FormError::Email);
    }
}
