//! The first run and Settings > Accounts > Add account
//! (`ui/onboarding.slint`, docs/WINUI3.md "Onboarding"): Your email (how
//! the servers are found), Sign in (the account check, `run_check`, then
//! `finish_add_account`), Make it yours (the name and signature, one
//! `update_account`) and the finish, with the first download's progress.
//! The lookup (`find_settings`) runs only on Continue, for a domain that is
//! not a preset's, and never sends the password.

use super::*;
use crate::mail::discover::{self, Outcome as Lookup};
use crate::mail::{Say, errors::Step};
use crate::{Onboard, ServerFields};

/// The chip on the Sign in step (`ob-chip`).
pub(super) mod chip {
    pub const NONE: i32 = 0;
    pub const FINDING: i32 = 1;
    pub const PRESET: i32 = 2;
    pub const FOUND: i32 = 3;
    pub const NOT_FOUND: i32 = 4;
}

/// A failure's kind (`a-error-kind`): the short title and sentence the
/// flow shows, the long sentence going under "Show details".
pub(super) mod kind {
    pub const OTHER: i32 = 0;
    pub const REJECTED: i32 = 1;
    pub const UNREACHABLE: i32 = 2;
    pub const CERTIFICATE: i32 = 3;
}

/// The chip a lookup's answer gives, and the incoming server it names:
/// found only when both servers were.
pub(super) fn chip_of(out: &Lookup) -> (i32, String) {
    match (&out.found.imap, &out.found.smtp) {
        (Some(imap), Some(_)) => (chip::FOUND, settings::host_for_display(&imap.server.host)),
        _ => (chip::NOT_FOUND, String::new()),
    }
}

/// The steps of the check in three stages: connecting securely (find,
/// connect, secure), checking the password (sign in, folder list) and
/// getting ready to send (the sending server's connection and sign-in).
const STAGES: [std::ops::Range<usize>; 3] = [0..3, 3..5, 5..Step::ALL.len()];

/// The state of stage `group` from the steps' states (0 waiting, 1
/// running, 2 passed, 3 failed, 4 skipped): failed if one step failed,
/// passed if all passed, skipped if all were skipped, running while one
/// runs or part of it passed, else waiting.
pub(super) fn stage(states: &[i32], group: usize) -> i32 {
    let Some(range) = STAGES.get(group) else {
        return 0;
    };
    let s: Vec<i32> = range
        .clone()
        .map(|i| states.get(i).copied().unwrap_or(0))
        .collect();
    let any = |v: i32| s.contains(&v);
    let all = |v: i32| s.iter().all(|&x| x == v);
    if any(3) {
        3
    } else if all(2) {
        2
    } else if all(4) {
        4
    } else if any(1) || (any(2) && any(0)) {
        1
    } else if any(4) {
        4
    } else {
        0
    }
}

/// A name to start "Your name" with, from the address's local part:
/// "ana.silva@example.com" is "Ana Silva" (a `+tag` is left out; a part
/// with no letter gives nothing).
pub(super) fn name_from_address(email: &str) -> String {
    let local = email.trim().rsplit_once('@').map_or("", |(l, _)| l);
    let local = local.split('+').next().unwrap_or("");
    if !local.chars().any(char::is_alphabetic) {
        return String::new();
    }
    local
        .split(['.', '_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A signature's first line, for the From preview.
pub(super) fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim_end().to_string()
}

/// What kind of failure a check's answer is (see [`kind`]).
pub(super) fn error_kind(text: &CheckText, reviewable: bool) -> i32 {
    match text {
        CheckText::Say(Say::Rejected { .. }) => kind::REJECTED,
        CheckText::Say(
            Say::NotFound { .. }
            | Say::NoAnswer { .. }
            | Say::NoConnection { .. }
            | Say::Lost { .. }
            | Say::Class(Class::Offline),
        ) => kind::UNREACHABLE,
        CheckText::Say(
            Say::SecureFailed { .. } | Say::Class(Class::TlsCert) | Say::CertChanged { .. },
        ) => kind::CERTIFICATE,
        _ if reviewable => kind::CERTIFICATE,
        _ => kind::OTHER,
    }
}

/// The flow's callbacks.
pub(super) fn wire(ui: &AppWindow) {
    ui.on_onboard_continue(|email, fields| {
        with_app(|a| a.onboard_continue(email.to_string(), fields));
    });
    ui.on_onboard_personalise(|save, name, signature| {
        with_app(|a| a.onboard_personalise(save, &name, &signature));
    });
    ui.on_onboard_open_inbox(|| {
        with_app(|a| a.onboard_open_inbox());
    });
    ui.on_onboard_closed(|| {
        with_app(|a| a.onboard_closed());
    });
    let rules = ui.global::<Onboard>();
    rules
        .on_stage(|states, group| stage(&states.iter().collect::<Vec<_>>(), group.max(0) as usize));
    rules.on_first_line(|s| first_line(&s).into());
}

impl App {
    /// Continue on the email step: refuse an address that is not one or is
    /// already here; a preset's domain has its settings (no lookup);
    /// offline, manual setup with its note; otherwise the lookup starts
    /// (only because of this press) and Sign in shows at once.
    pub(super) fn onboard_continue(&mut self, email: String, fields: ServerFields) {
        let ui = self.ui();
        let address = email.trim();
        if !discover::looks_valid(address) {
            ui.set_ob_email_problem(1);
            return;
        }
        if self
            .accounts
            .iter()
            .any(|a| a.email.eq_ignore_ascii_case(address))
        {
            ui.set_ob_email_problem(2);
            return;
        }
        ui.set_ob_email_problem(0);
        self.onboard_closed();
        let domain = discover::parse_email(address).map_or_else(String::new, |(_, d)| d);
        ui.set_ob_domain(domain.as_str().into());
        ui.set_ob_chip_host(SharedString::new());
        ui.set_ob_offline(false);
        if let Some(preset) = providers::guess(address) {
            ui.set_ob_provider(preset as i32);
            ui.set_ob_manual(false);
            ui.set_ob_chip(chip::PRESET);
        } else if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() {
            ui.set_ob_provider(providers::PRESETS.len() as i32);
            ui.set_ob_manual(true);
            ui.set_ob_offline(true);
            ui.set_ob_chip(chip::NONE);
        } else {
            ui.set_ob_provider(-1);
            ui.set_ob_manual(false);
            ui.set_ob_chip(chip::FINDING);
            self.find_settings(email, fields);
        }
        ui.set_ob_step(2);
    }

    /// The lookup answered while the chip waited for it.
    pub(super) fn onboard_found(&self, out: &Lookup) {
        let ui = self.ui();
        if ui.get_ob_chip() == chip::FINDING {
            let (state, host) = chip_of(out);
            ui.set_ob_chip_host(host.into());
            ui.set_ob_chip(state);
        }
    }

    /// The account was saved (the check passed): Make it yours, the name
    /// started from the address, while its first download starts.
    pub(super) fn onboard_added(&mut self, id: i64, email: &str) {
        let ui = self.ui();
        self.ob_account = Some(id);
        self.ob_progress = None;
        ui.set_a_busy(false);
        ui.set_a_test(ModelRc::default());
        ui.set_a_rejected(false);
        ui.set_a_error(SharedString::new());
        ui.set_ob_address(email.into());
        ui.set_ob_name(name_from_address(email).into());
        ui.set_ob_sync(i32::from(self.workers.contains_key(&id)));
        ui.set_ob_sync_value(0.0);
        ui.set_ob_sync_text(SharedString::new());
        ui.set_ob_newest(false);
        ui.set_ob_open(true);
        ui.set_ob_step(3);
    }

    /// Done (`save`) writes the name and signature with one
    /// `update_account`; Skip writes nothing. Then the finish.
    pub(super) fn onboard_personalise(&mut self, save: bool, name: &str, signature: &str) {
        if save
            && let Some(id) = self.ob_account
            && let Some(old) = self.accounts.iter().find(|a| a.id == id).cloned()
        {
            let edited = Account {
                name: name.trim().to_string(),
                signature: signature.to_string(),
                ..old
            };
            match self.db.update_account(&edited) {
                Ok(_) => {
                    // The signature typed here becomes a named signature,
                    // the account's default for new mail and for replies.
                    if let Err(e) = self.db.adopt_signature_text(id, signature) {
                        self.set_status(&text::detail(&e), true);
                    }
                    if let Err(e) = self.load_accounts() {
                        self.set_status(&text::detail(&e), true);
                    }
                    self.show_account_rows();
                    self.rebuild_nav();
                }
                Err(e) => self.set_status(&t!(invoke_not_saved, format!("{e:#}").into()), true),
            }
        }
        self.ui().set_ob_step(4);
    }

    /// Open your inbox: the flow (and Settings under it) closes on the new
    /// account's Inbox, the list focused.
    pub(super) fn onboard_open_inbox(&mut self) {
        let ui = self.ui();
        ui.set_ob_open(false);
        ui.set_ob_step(0);
        self.onboard_closed();
        self.ob_account = None;
        self.ob_progress = None;
        if ui.get_show_settings() {
            ui.set_show_settings(false);
        }
        self.land();
        ui.invoke_focus_list();
    }

    /// Back to the email step, or the flow closed: a lookup or check still
    /// running answers into nothing; the form's certificates and errors go.
    pub(super) fn onboard_closed(&mut self) {
        self.cancel_check();
        self.form_opened();
        let ui = self.ui();
        ui.set_a_error(SharedString::new());
        ui.set_a_error_kind(kind::OTHER);
        ui.set_a_rejected(false);
        ui.set_ob_chip(chip::NONE);
        ui.set_ob_offline(false);
    }

    /// The first download of the account the flow added: an indeterminate
    /// bar until a folder reports numbers, then done of total (holding the
    /// last value between folders, never going back within one); and
    /// whether its Inbox has a message yet.
    pub(super) fn onboard_event(&mut self, event: &Event) {
        let Some(id) = self.ob_account else {
            return;
        };
        let ui = self.ui();
        match *event {
            Event::Syncing {
                account_id, active, ..
            } if account_id == id => {
                if active && ui.get_ob_sync() == 0 {
                    ui.set_ob_sync(1);
                } else if !active {
                    ui.set_ob_sync(3);
                }
            }
            Event::Progress {
                account_id,
                folder_id,
                done,
                total,
            } if account_id == id && total > 0 => {
                let done = match self.ob_progress {
                    Some((f, d)) if f == folder_id => done.max(d),
                    _ => done,
                };
                self.ob_progress = Some((folder_id, done));
                ui.set_ob_sync(2);
                ui.set_ob_sync_value(done as f32 / total as f32);
                ui.set_ob_sync_text(
                    t!(
                        invoke_empty_downloading,
                        format::thousands(self.lang, done).into(),
                        format::thousands(self.lang, total).into()
                    )
                    .into(),
                );
            }
            Event::FolderChanged { .. } => {
                let has_mail = self
                    .folders
                    .iter()
                    .find(|f| f.account_id == id && f.role == Role::Inbox)
                    .is_some_and(|f| self.db.count_messages(f.id).is_ok_and(|n| n > 0));
                if has_mail {
                    ui.set_ob_newest(true);
                }
            }
            _ => {}
        }
    }
}

/// Snapshot QA (`KOMOREBI_SNAPSHOT_ACTION=ob-…`, docs: README "Snapshot
/// options"): the flow in one of its states, made up and offline; nothing is
/// looked up, checked or sent. `personalise`, `done` and `done-progress`
/// add a made-up account to the throwaway cache (only under
/// `KOMOREBI_OFFLINE`); the others write nothing.
pub fn sample_onboard(state: &str) {
    use crate::mail::discover::{Discovered, Found, Source};
    use crate::mail::errors::{Class, describe};
    let found = |host: &str, port: u16, security| Found {
        server: Server {
            host: host.into(),
            port,
            security,
        },
        user: None,
    };
    with_app(|a| {
        let ui = a.ui();
        ui.invoke_open_onboarding();
        let preset = "ana.silva@purelymail.com";
        let custom = "alice@example.com";
        let type_in = |email: &str| ui.set_ob_email(email.into());
        let fields = ServerFields {
            imap_port: "993".into(),
            smtp_port: "465".into(),
            ..Default::default()
        };
        // A lookup under way for the custom address (as Continue starts it).
        let finding = |a: &mut App| {
            type_in(custom);
            ui.set_ob_domain("example.com".into());
            ui.set_ob_provider(-1);
            ui.set_ob_chip(chip::FINDING);
            a.find_seq += 1;
            ui.set_a_finding(true);
            ui.set_ob_step(2);
        };
        // A failed check of the preset address.
        let failed = |a: &mut App, step: Step, say: Say| {
            type_in(preset);
            a.onboard_continue(preset.into(), fields.clone());
            a.check_form = (Some(0), preset.into());
            a.check_seq += 1;
            let seq = a.check_seq;
            a.finish_check(
                seq,
                Err(CheckFailure {
                    step: Some(step.index()),
                    text: CheckText::Say(say),
                    review: None,
                }),
                None,
            );
        };
        match state {
            "email" => {}
            "email-error" => {
                type_in("ana@");
                a.onboard_continue("ana@".into(), fields.clone());
            }
            "finding" => finding(a),
            "found" | "notfound" => {
                finding(a);
                let out = if state == "found" {
                    Lookup {
                        found: Discovered {
                            imap: Some(found("imap.example.com", 993, Security::Tls)),
                            smtp: Some(found("smtp.example.com", 587, Security::StartTls)),
                        },
                        sources: vec![Source::Autoconfig],
                        refused: Vec::new(),
                    }
                } else {
                    Lookup::default()
                };
                let from = find::Fields::from(&fields);
                let seq = a.find_seq;
                a.finish_find(seq, custom.into(), from, out);
            }
            "found-preset" | "password" => {
                type_in(preset);
                a.onboard_continue(preset.into(), fields.clone());
            }
            "wait" | "wait-slow" => {
                type_in(preset);
                a.onboard_continue(preset.into(), fields.clone());
                ui.set_a_busy(true);
                a.show_check_steps(Step::ImapSignIn.index(), None);
                if state == "wait-slow" {
                    ui.set_ob_slow(2);
                }
            }
            "error-rejected" => failed(
                a,
                Step::ImapSignIn,
                describe(
                    Step::ImapSignIn,
                    Class::Auth,
                    "imap.purelymail.com",
                    993,
                    preset,
                ),
            ),
            "error-unreachable" => failed(
                a,
                Step::Connect,
                Say::NoAnswer {
                    host: "imap.purelymail.com".into(),
                    port: 993,
                },
            ),
            "error-cert" => {
                failed(
                    a,
                    Step::Secure,
                    Say::SecureFailed {
                        host: "mail.example.com".into(),
                    },
                );
                ui.set_ob_manual(true);
                ui.set_ob_provider(providers::PRESETS.len() as i32);
                a.show_cert_state(1);
            }
            "manual" => {
                type_in(custom);
                ui.set_ob_domain("example.com".into());
                ui.set_ob_chip(chip::NOT_FOUND);
                ui.set_ob_manual(true);
                ui.set_ob_provider(providers::PRESETS.len() as i32);
                ui.set_ob_step(2);
            }
            "personalise" | "done" | "done-progress"
                if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() =>
            {
                let server = |host: &str| Server {
                    host: host.into(),
                    port: 993,
                    security: Security::Tls,
                };
                let new = NewAccount {
                    email: "ana.silva@example.com".into(),
                    name: String::new(),
                    imap: server("imap.example.com"),
                    smtp: server("smtp.example.com"),
                    save_sent: true,
                    imap_user: String::new(),
                    smtp_user: String::new(),
                    smtp_no_auth: false,
                };
                let Ok(id) = a.db.add_account(&new) else {
                    return;
                };
                let inbox =
                    a.db.upsert_folder(id, "INBOX", "Inbox", 0, Role::Inbox)
                        .ok();
                if a.load_accounts().is_err() {
                    return;
                }
                a.folders = a.db.folders().unwrap_or_default();
                a.rebuild_nav();
                type_in(&new.email);
                a.onboard_added(id, &new.email);
                ui.set_ob_sync(1);
                if state != "personalise" {
                    a.onboard_personalise(false, "", "");
                }
                if state == "done-progress"
                    && let Some(folder) = inbox
                {
                    a.on_event(Event::Progress {
                        account_id: id,
                        folder_id: folder,
                        done: 1234,
                        total: 5000,
                    });
                }
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::discover::{Discovered, Found, Source};

    #[test]
    fn the_name_starts_from_the_local_part() {
        assert_eq!(name_from_address("ana.silva@example.com"), "Ana Silva");
        assert_eq!(name_from_address("JOAO_PEREIRA@x.pt"), "Joao Pereira");
        assert_eq!(name_from_address("ana+news@example.com"), "Ana");
        assert_eq!(name_from_address("mary-jane.o@example.com"), "Mary Jane O");
        assert_eq!(name_from_address("1234@example.com"), "");
        assert_eq!(name_from_address("no-at-sign"), "");
    }

    #[test]
    fn the_chip_says_found_only_with_both_servers() {
        let found = |host: &str| Found {
            server: Server {
                host: host.into(),
                port: 993,
                security: Security::Tls,
            },
            user: None,
        };
        let both = Lookup {
            found: Discovered {
                imap: Some(found("imap.example.com")),
                smtp: Some(found("smtp.example.com")),
            },
            sources: vec![Source::Autoconfig],
            refused: Vec::new(),
        };
        assert_eq!(chip_of(&both), (chip::FOUND, "imap.example.com".into()));
        let half = Lookup {
            found: Discovered {
                imap: None,
                smtp: Some(found("smtp.example.com")),
            },
            ..both
        };
        assert_eq!(chip_of(&half), (chip::NOT_FOUND, String::new()));
        assert_eq!(
            chip_of(&Lookup::default()),
            (chip::NOT_FOUND, String::new())
        );
    }

    #[test]
    fn the_steps_group_into_three_stages() {
        let all = |states: [i32; 7]| (0..3).map(|g| stage(&states, g)).collect::<Vec<_>>();
        // Not started.
        assert_eq!(all([0; 7]), [0, 0, 0]);
        // Connecting: the first stage runs, the others wait.
        assert_eq!(all([2, 1, 0, 0, 0, 0, 0]), [1, 0, 0]);
        // Between stages: the first passed, the next one starting.
        assert_eq!(all([2, 2, 2, 1, 0, 0, 0]), [2, 1, 0]);
        assert_eq!(all([2, 2, 2, 2, 0, 0, 0]), [2, 1, 0]);
        // The password rejected: that stage failed, the last skipped.
        assert_eq!(all([2, 2, 2, 3, 4, 4, 4]), [2, 3, 4]);
        // All passed.
        assert_eq!(all([2; 7]), [2, 2, 2]);
        // Fewer states than steps (none yet): waiting.
        assert_eq!(stage(&[], 1), 0);
        assert_eq!(stage(&[2; 7], 9), 0);
    }

    #[test]
    fn failures_have_a_kind() {
        let say = |s: Say| CheckText::Say(s);
        let host = || "mail.example.net".to_string();
        assert_eq!(
            error_kind(
                &say(Say::Rejected {
                    host: host(),
                    login: "ana".into()
                }),
                false
            ),
            kind::REJECTED
        );
        assert_eq!(
            error_kind(&say(Say::NotFound { host: host() }), false),
            kind::UNREACHABLE
        );
        assert_eq!(
            error_kind(
                &say(Say::NoAnswer {
                    host: host(),
                    port: 993
                }),
                false
            ),
            kind::UNREACHABLE
        );
        assert_eq!(
            error_kind(&say(Say::SecureFailed { host: host() }), false),
            kind::CERTIFICATE
        );
        assert_eq!(
            error_kind(&say(Say::NoFolders { host: host() }), true),
            kind::CERTIFICATE
        );
        assert_eq!(
            error_kind(&say(Say::NoFolders { host: host() }), false),
            kind::OTHER
        );
        assert_eq!(error_kind(&CheckText::NoSavedPassword, false), kind::OTHER);
    }

    #[test]
    fn the_preview_shows_the_first_line() {
        assert_eq!(first_line("Ana Silva\nExample Ltd"), "Ana Silva");
        assert_eq!(first_line("  Ana  \r\nx"), "  Ana");
        assert_eq!(first_line(""), "");
    }
}
