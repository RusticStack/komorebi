//! "Find settings" in the manual account form (`mail::discover`): one
//! short-lived thread per click looks the email domain up; the answer only
//! prefills the server fields the user has not edited. Nothing is saved and
//! the password is never involved.

use super::*;
use crate::ServerFields;
use crate::mail::discover::{self, Found, Source};

/// The server fields of the form as the lookup sees them (the Slint
/// struct without the Slint types).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Fields {
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

impl From<&ServerFields> for Fields {
    fn from(f: &ServerFields) -> Self {
        Fields {
            imap_host: f.imap_host.to_string(),
            imap_port: f.imap_port.to_string(),
            imap_starttls: f.imap_starttls,
            imap_user: f.imap_user.to_string(),
            smtp_host: f.smtp_host.to_string(),
            smtp_port: f.smtp_port.to_string(),
            smtp_starttls: f.smtp_starttls,
            smtp_user: f.smtp_user.to_string(),
            smtp_own_login: f.smtp_own_login,
        }
    }
}

impl From<&Fields> for ServerFields {
    fn from(f: &Fields) -> Self {
        ServerFields {
            imap_host: f.imap_host.as_str().into(),
            imap_port: f.imap_port.as_str().into(),
            imap_starttls: f.imap_starttls,
            imap_user: f.imap_user.as_str().into(),
            smtp_host: f.smtp_host.as_str().into(),
            smtp_port: f.smtp_port.as_str().into(),
            smtp_starttls: f.smtp_starttls,
            smtp_user: f.smtp_user.as_str().into(),
            smtp_own_login: f.smtp_own_login,
        }
    }
}

/// What a lookup changes in the form.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Merged {
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
pub(super) fn merge(
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

/// The lines under the button after a lookup: where the settings came from,
/// what was not taken and what is left to type, one complete sentence each.
pub(super) fn note(out: &discover::Outcome, kept: bool) -> String {
    let mut lines = Vec::new();
    if out.is_empty() {
        lines.push(t!(invoke_find_none));
    } else {
        let sources = out.sources.as_slice();
        lines.push(match sources {
            [Source::Builtin] => t!(invoke_find_builtin),
            [Source::Srv] => t!(invoke_find_srv),
            [Source::Autoconfig] => t!(invoke_find_autoconfig),
            _ => t!(invoke_find_both),
        });
        if out.found.imap.is_none() {
            lines.push(t!(invoke_find_no_incoming));
        }
        if out.found.smtp.is_none() {
            lines.push(t!(invoke_find_no_sending));
        }
    }
    for host in out.refused.iter().take(2) {
        lines.push(t!(
            invoke_find_refused,
            super::settings::host_for_display(host).into()
        ));
    }
    if kept {
        lines.push(t!(invoke_find_kept));
    }
    lines.join("\n")
}

impl App {
    /// "Find settings": look the email domain up (DNS, then the provider's
    /// autoconfig over HTTPS) on a short-lived thread. Only a press starts
    /// it (Find settings, or Continue on the onboarding email step for a
    /// domain that is not a preset's); offline it refuses.
    pub(super) fn find_settings(&mut self, email: String, fields: ServerFields) {
        let ui = self.ui();
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            ui.set_a_find_note(t!(invoke_err_offline_find).into());
            return;
        }
        if !discover::looks_valid(&email) {
            ui.set_a_find_note(t!(invoke_err_email_first).into());
            return;
        }
        self.find_seq += 1;
        let seq = self.find_seq;
        let from = Fields::from(&fields);
        ui.set_a_finding(true);
        ui.set_a_find_note(SharedString::new());
        let look = move || {
            let out = discover::discover(&email, &discover::system_lookups());
            on_ui(Box::new(move |app| {
                app.finish_find(seq, email, from, out);
            }));
        };
        background("find-settings", Box::new(look)).expect("spawning settings lookup");
    }

    /// The lookup answered: hand the form the fields to fill (it fills them
    /// only if the user has not edited the server fields meanwhile) and the
    /// note.
    pub(super) fn finish_find(
        &mut self,
        seq: u64,
        email: String,
        from: Fields,
        out: discover::Outcome,
    ) {
        if seq != self.find_seq {
            return;
        }
        let ui = self.ui();
        let address = discover::parse_email(&email)
            .map(|(l, d)| format!("{l}@{d}"))
            .unwrap_or_default();
        let merged = merge(&from, &out.found, &address, self.found_last.as_ref());
        if merged.to != from {
            self.found_last = Some(merged.to.clone());
        }
        // The onboarding chip first: a Sign in waiting for this answer
        // reads it when `found-seq` changes.
        self.onboard_found(&out);
        ui.set_a_found_from((&from).into());
        ui.set_a_found_email(email.as_str().into());
        ui.set_a_found_to((&merged.to).into());
        ui.set_a_find_note(note(&out, merged.kept).into());
        ui.set_a_finding(false);
        ui.set_a_found_seq(ui.get_a_found_seq().wrapping_add(1));
    }

    /// The form opened or closed: a lookup still running answers into
    /// nothing, and what it filled is forgotten.
    pub(super) fn find_reset(&mut self) {
        self.find_seq += 1;
        self.found_last = None;
        let ui = self.ui();
        ui.set_a_finding(false);
        ui.set_a_find_note(SharedString::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::discover::{Discovered, Outcome};

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

    #[test]
    fn the_note_names_the_source_and_what_was_left_out() {
        let _ui = crate::tr::tests::window();
        let ok = |sources: Vec<Source>| Outcome {
            found: both(),
            sources,
            refused: Vec::new(),
        };
        assert_eq!(
            note(&ok(vec![Source::Autoconfig]), false),
            "Found by your provider's autoconfig."
        );
        assert_eq!(
            note(&ok(vec![Source::Srv]), false),
            "Found in your domain's DNS records."
        );
        assert_eq!(
            note(&ok(vec![Source::Builtin]), false),
            "Komorebi has settings built in for this provider."
        );
        assert_eq!(
            note(&ok(vec![Source::Srv, Source::Autoconfig]), true),
            "Found in your domain's DNS records and by your provider's autoconfig.\nKept the server settings you typed."
        );
        assert_eq!(
            note(&Outcome::default(), false),
            "No settings found, enter them by hand."
        );
        let foreign = Outcome {
            found: Discovered {
                imap: None,
                smtp: both().smtp,
            },
            sources: vec![Source::Autoconfig],
            refused: vec!["mail.other.net".into()],
        };
        assert_eq!(
            note(&foreign, false),
            "Found by your provider's autoconfig.\nNo incoming server found, enter it by hand.\nThe provider's settings point to mail.other.net, a different domain. Not filled in."
        );
    }
}
