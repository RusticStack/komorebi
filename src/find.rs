//! "Find settings" in the manual account form: what a lookup of the email
//! domain (`mail::discover`) may fill in. The answer only prefills the
//! server fields the user has not edited; nothing is saved and the password
//! is never involved. Both apps call this (the Slint app from
//! `src/app/find.rs`, which keeps the lookup thread and the note's words).

use crate::mail::discover::{self, Found};
use crate::providers::Security;

/// The server fields of the form as the lookup sees them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub imap_host: String,
    pub imap_port: String,
    pub imap_starttls: bool,
    pub imap_user: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_starttls: bool,
    pub smtp_user: String,
    pub smtp_own_login: bool,
}

/// What a lookup changes in the form.
#[derive(Debug, PartialEq, Eq)]
pub struct Merged {
    pub to: Fields,
    /// Something found was left out because the user had typed there.
    pub kept: bool,
}

/// A field the lookup may fill: empty, or still what the last lookup put
/// there.
fn untouched(now: &str, last: Option<&str>) -> bool {
    now.trim().is_empty() || last == Some(now)
}

/// A port the lookup may replace: empty, the last one it set, or the
/// default of the security now chosen.
fn port_untouched(now: &str, last: Option<&str>, imap: bool, starttls: bool) -> bool {
    let default = match (imap, starttls) {
        (true, false) => "993",
        (true, true) => "143",
        (false, false) => "465",
        (false, true) => "587",
    };
    untouched(now, last) || now.trim() == default
}

/// `from` (the form when the button was pressed) with what `found` supplies
/// for the fields that are untouched: empty, or what the previous lookup
/// filled (`last`). A server whose host or port the user typed is left
/// alone as a whole, so the host, port and security always fit together.
pub fn merge(
    from: &Fields,
    found: &discover::Discovered,
    email: &str,
    last: Option<&Fields>,
) -> Merged {
    let mut to = from.clone();
    let mut kept = false;

    if let Some(Found { server, user }) = &found.imap {
        if untouched(&from.imap_host, last.map(|l| l.imap_host.as_str()))
            && port_untouched(
                &from.imap_port,
                last.map(|l| l.imap_port.as_str()),
                true,
                from.imap_starttls,
            )
        {
            to.imap_host = server.host.clone();
            to.imap_port = server.port.to_string();
            to.imap_starttls = server.security == Security::StartTls;
            if untouched(&from.imap_user, last.map(|l| l.imap_user.as_str())) {
                to.imap_user = user.clone().unwrap_or_default();
            } else {
                kept = true;
            }
        } else {
            kept = true;
        }
    }

    if let Some(Found { server, user }) = &found.smtp {
        if untouched(&from.smtp_host, last.map(|l| l.smtp_host.as_str()))
            && port_untouched(
                &from.smtp_port,
                last.map(|l| l.smtp_port.as_str()),
                false,
                from.smtp_starttls,
            )
        {
            to.smtp_host = server.host.clone();
            to.smtp_port = server.port.to_string();
            to.smtp_starttls = server.security == Security::StartTls;
            // The sending sign-in follows: the IMAP one unless the provider
            // names another.
            if untouched(&from.smtp_user, last.map(|l| l.smtp_user.as_str())) {
                let login = |s: &str| {
                    if s.trim().is_empty() {
                        email.to_string()
                    } else {
                        s.trim().to_string()
                    }
                };
                let wanted = user.clone().unwrap_or_default();
                if login(&wanted) == login(&to.imap_user) {
                    to.smtp_own_login = false;
                    to.smtp_user = String::new();
                } else {
                    to.smtp_own_login = true;
                    to.smtp_user = wanted;
                }
            } else {
                kept = true;
            }
        } else {
            kept = true;
        }
    }
    Merged { to, kept }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::discover::Discovered;
    use crate::providers::Server;

    fn found(host: &str, port: u16, security: Security, user: Option<&str>) -> Found {
        Found {
            server: Server {
                host: host.into(),
                port,
                security,
            },
            user: user.map(String::from),
        }
    }

    fn empty_form() -> Fields {
        Fields {
            imap_port: "993".into(),
            smtp_port: "465".into(),
            ..Fields::default()
        }
    }

    fn both() -> Discovered {
        Discovered {
            imap: Some(found("imap.example.com", 993, Security::Tls, None)),
            smtp: Some(found("smtp.example.com", 587, Security::StartTls, None)),
        }
    }

    #[test]
    fn an_untouched_form_is_filled() {
        let m = merge(&empty_form(), &both(), "me@example.com", None);
        assert!(!m.kept);
        assert_eq!(
            m.to,
            Fields {
                imap_host: "imap.example.com".into(),
                imap_port: "993".into(),
                smtp_host: "smtp.example.com".into(),
                smtp_port: "587".into(),
                smtp_starttls: true,
                ..Fields::default()
            }
        );
    }

    #[test]
    fn typed_servers_are_never_overwritten() {
        let typed = Fields {
            imap_host: "mail.mine.org".into(),
            imap_port: "993".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: "2525".into(),
            ..empty_form()
        };
        let m = merge(&typed, &both(), "me@example.com", None);
        // The typed IMAP host stays with its port; the SMTP host was
        // already the same but its port is the user's.
        assert!(m.kept);
        assert_eq!(m.to.imap_host, "mail.mine.org");
        assert_eq!(m.to.smtp_port, "2525");
        assert_eq!(m.to.smtp_host, "smtp.example.com");
        assert!(!m.to.smtp_starttls);
    }

    #[test]
    fn a_second_lookup_replaces_only_what_the_first_filled() {
        let first = merge(&empty_form(), &both(), "me@example.com", None).to;
        // The user edits the SMTP host only.
        let mut edited = first.clone();
        edited.smtp_host = "relay.mine.org".into();
        let other = Discovered {
            imap: Some(found("imap2.example.com", 143, Security::StartTls, None)),
            smtp: Some(found("smtp2.example.com", 465, Security::Tls, None)),
        };
        let m = merge(&edited, &other, "me@example.com", Some(&first));
        assert!(m.kept);
        assert_eq!(m.to.imap_host, "imap2.example.com");
        assert_eq!(m.to.imap_port, "143");
        assert!(m.to.imap_starttls);
        assert_eq!(m.to.smtp_host, "relay.mine.org");
        assert_eq!(m.to.smtp_port, "587");
    }

    #[test]
    fn sign_in_names_follow_the_provider() {
        // The IMAP name differs from the address; SMTP signs in with the
        // address: so SMTP gets its own (empty = the address) sign-in.
        let d = Discovered {
            imap: Some(found("imap.example.com", 993, Security::Tls, Some("alice"))),
            smtp: Some(found("smtp.example.com", 465, Security::Tls, None)),
        };
        let m = merge(&empty_form(), &d, "alice@example.com", None).to;
        assert_eq!(m.imap_user, "alice");
        assert!(m.smtp_own_login);
        assert_eq!(m.smtp_user, "");
        // The same name for both: one sign-in.
        let d = Discovered {
            imap: Some(found("imap.example.com", 993, Security::Tls, Some("alice"))),
            smtp: Some(found("smtp.example.com", 465, Security::Tls, Some("alice"))),
        };
        let m = merge(&empty_form(), &d, "alice@example.com", None).to;
        assert_eq!((m.imap_user.as_str(), m.smtp_own_login), ("alice", false));
        // A name the user typed stays.
        let typed = Fields {
            imap_user: "bob".into(),
            ..empty_form()
        };
        let m = merge(&typed, &d, "alice@example.com", None);
        assert!(m.kept);
        assert_eq!(m.to.imap_user, "bob");
    }

    #[test]
    fn a_part_of_the_answer_leaves_the_rest_alone() {
        let only_imap = Discovered {
            imap: both().imap,
            smtp: None,
        };
        let typed = Fields {
            smtp_host: "mine".into(),
            smtp_port: "25".into(),
            ..empty_form()
        };
        let m = merge(&typed, &only_imap, "me@example.com", None);
        assert!(!m.kept);
        assert_eq!(m.to.smtp_host, "mine");
        assert_eq!(m.to.smtp_port, "25");
        assert_eq!(m.to.imap_host, "imap.example.com");
        // Nothing found: nothing changes.
        let m = merge(&typed, &Discovered::default(), "me@example.com", None);
        assert_eq!((m.to, m.kept), (typed, false));
    }
}
