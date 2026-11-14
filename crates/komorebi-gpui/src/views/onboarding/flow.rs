//! The setup flow's plain rules (the Slint app's `src/app/onboarding.rs`
//! and `src/app/find.rs` note): the chip a lookup gives, the check's steps
//! in three stages, the name started from the address, a failure's next
//! step, the From preview's first line and the lookup's note. No window
//! code here, so the rules are tested as plain functions.

use komorebi::mail::discover::{Outcome, Source};
use komorebi::mail::errors::Step;

use crate::views::settings::check::{Kind3, StepState};
use crate::views::settings::text;

/// The chip on the Sign in step (`ob-chip`): how the servers were found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chip {
    None,
    /// The lookup runs.
    Finding,
    /// A preset's domain (`providers::PRESETS` index): no lookup.
    Preset(usize),
    /// Both servers found; the incoming server's name, for display.
    Found(String),
    NotFound,
}

/// The chip a lookup's answer gives: found only when both servers were.
pub fn chip_of(out: &Outcome) -> Chip {
    match (&out.found.imap, &out.found.smtp) {
        (Some(imap), Some(_)) => Chip::Found(crate::text::host(&imap.server.host)),
        _ => Chip::NotFound,
    }
}

/// The steps of the check in three stages: connecting securely (find,
/// connect, secure), checking the password (sign in, folder list) and
/// getting ready to send (the sending server's connection and sign-in).
pub const STAGES: [std::ops::Range<usize>; 3] = [0..3, 3..5, 5..Step::ALL.len()];

/// A stage's state from the steps' states: failed if one step failed,
/// passed if all passed, skipped if all were skipped, running while one
/// runs or part of it passed, else waiting.
pub fn stage(states: &[StepState], group: usize) -> StepState {
    let Some(range) = STAGES.get(group) else {
        return StepState::Waiting;
    };
    let s: Vec<StepState> = range
        .clone()
        .map(|i| states.get(i).copied().unwrap_or(StepState::Waiting))
        .collect();
    let any = |v: StepState| s.contains(&v);
    let all = |v: StepState| s.iter().all(|&x| x == v);
    if any(StepState::Failed) {
        StepState::Failed
    } else if all(StepState::Passed) {
        StepState::Passed
    } else if all(StepState::Skipped) {
        StepState::Skipped
    } else if any(StepState::Running) || (any(StepState::Passed) && any(StepState::Waiting)) {
        StepState::Running
    } else if any(StepState::Skipped) {
        StepState::Skipped
    } else {
        StepState::Waiting
    }
}

/// A name to start "Your name" with, from the address's local part:
/// "ana.silva@example.com" is "Ana Silva" (a `+tag` is left out; a part
/// with no letter gives nothing).
pub fn name_from_address(email: &str) -> String {
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
pub fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim_end().to_string()
}

/// The one next step a failure offers (`err-box.act`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    SetUsername,
    ReviewCertificate,
    TryAgain,
    SetUpManually,
}

/// The next step for a failure of `kind`: Set username after a rejected
/// password (unless the username field shows), Try again when the server
/// can't be reached, Review certificate for a manual server's untrusted
/// one (else Set up manually), and otherwise Try again on a manual setup,
/// Set up manually on a found one.
pub fn act(kind: Kind3, user_open: bool, reviewable: bool, manual: bool) -> Act {
    match kind {
        Kind3::Rejected if user_open => Act::None,
        Kind3::Rejected => Act::SetUsername,
        Kind3::Unreachable => Act::TryAgain,
        Kind3::Certificate if reviewable => Act::ReviewCertificate,
        Kind3::Certificate if manual => Act::None,
        Kind3::Certificate => Act::SetUpManually,
        Kind3::Other if manual => Act::TryAgain,
        Kind3::Other => Act::SetUpManually,
    }
}

/// The lines under the server fields after a lookup: where the settings
/// came from, what was not taken and what is left to type, one complete
/// sentence each (the Slint app's `find::note`).
pub fn note(out: &Outcome, kept: bool) -> String {
    let mut lines = Vec::new();
    if out.is_empty() {
        lines.push(text::find_none());
    } else {
        lines.push(match out.sources.as_slice() {
            [Source::Builtin] => text::find_builtin(),
            [Source::Srv] => text::find_srv(),
            [Source::Autoconfig] => text::find_autoconfig(),
            _ => text::find_both(),
        });
        if out.found.imap.is_none() {
            lines.push(text::find_no_incoming());
        }
        if out.found.smtp.is_none() {
            lines.push(text::find_no_sending());
        }
    }
    for host in out.refused.iter().take(2) {
        lines.push(text::find_refused(&crate::text::host(host)));
    }
    if kept {
        lines.push(text::find_kept());
    }
    lines.join("\n")
}

/// The line under a preset in the provider choice (the Slint flow's
/// `RadioButtons` descriptions).
pub fn preset_note(name: &str) -> String {
    use komorebi_i18n::tr;
    match name {
        "Purelymail" => tr!("Any domain hosted at Purelymail"),
        "Postale.io" => tr!("Any domain hosted at Postale"),
        _ => String::new(),
    }
}

/// A step of the indicator as Narrator reads it: its name, and whether it
/// is done or the current one.
pub fn step_name(label: &str, done: bool, current: bool) -> String {
    use komorebi_i18n::tr;
    if done {
        tr!("{0}, done", label)
    } else if current {
        tr!("{0}, current step", label)
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use komorebi::i18n::Lang;
    use komorebi::mail::discover::{Discovered, Found};
    use komorebi::providers::{Security, Server};

    use super::*;

    fn found(host: &str, port: u16, security: Security) -> Found {
        Found {
            server: Server {
                host: host.into(),
                port,
                security,
            },
            user: None,
        }
    }

    fn both() -> Discovered {
        Discovered {
            imap: Some(found("imap.example.com", 993, Security::Tls)),
            smtp: Some(found("smtp.example.com", 587, Security::StartTls)),
        }
    }

    /// Ports `the_name_starts_from_the_local_part` (src/app/onboarding.rs).
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
    fn steps_say_done_and_current_and_presets_their_line() {
        komorebi_i18n::with_lang(Lang::En, || {
            assert_eq!(step_name("Sign in", true, false), "Sign in, done");
            assert_eq!(step_name("Sign in", false, true), "Sign in, current step");
            assert_eq!(step_name("Sign in", false, false), "Sign in");
            assert_eq!(preset_note("Postale.io"), "Any domain hosted at Postale");
            assert_eq!(preset_note("Other"), "");
        });
        komorebi_i18n::with_lang(Lang::PtPt, || {
            assert_eq!(step_name("Entrar", true, false), "Entrar, concluído");
        });
    }

    /// Ports `the_chip_says_found_only_with_both_servers`.
    #[test]
    fn the_chip_says_found_only_with_both_servers() {
        let out = Outcome {
            found: both(),
            sources: vec![Source::Autoconfig],
            refused: Vec::new(),
        };
        assert_eq!(chip_of(&out), Chip::Found("imap.example.com".into()));
        let half = Outcome {
            found: Discovered {
                imap: both().imap,
                smtp: None,
            },
            ..out.clone()
        };
        assert_eq!(chip_of(&half), Chip::NotFound);
        assert_eq!(chip_of(&Outcome::default()), Chip::NotFound);
    }

    /// Ports `the_steps_group_into_three_stages`.
    #[test]
    fn the_steps_group_into_three_stages() {
        use StepState::{Failed as F, Passed as P, Running as R, Skipped as S, Waiting as W};
        let all = |states: [StepState; 7]| (0..3).map(|g| stage(&states, g)).collect::<Vec<_>>();
        // Not started.
        assert_eq!(all([W; 7]), [W, W, W]);
        // Connecting: the first stage runs, the others wait.
        assert_eq!(all([P, R, W, W, W, W, W]), [R, W, W]);
        // Between stages: the first passed, the next one starting.
        assert_eq!(all([P, P, P, R, W, W, W]), [P, R, W]);
        assert_eq!(all([P, P, P, P, W, W, W]), [P, R, W]);
        // The password rejected: that stage failed, the last skipped.
        assert_eq!(all([P, P, P, F, S, S, S]), [P, F, S]);
        // All passed.
        assert_eq!(all([P; 7]), [P, P, P]);
        // Fewer states than steps (none yet): waiting.
        assert_eq!(stage(&[], 1), W);
        assert_eq!(stage(&[P; 7], 9), W);
    }

    /// Ports `failures_have_a_kind` (the kind itself is
    /// `settings::check::error_kind`), with the next step each offers.
    #[test]
    fn failures_have_a_kind() {
        use crate::views::settings::check::{CheckText, error_kind};
        use komorebi::mail::errors::{Class, Say};
        let rejected = CheckText::Say(Say::Rejected {
            host: "imap.example.com".into(),
            login: "ana".into(),
        });
        assert_eq!(error_kind(&rejected, false), Kind3::Rejected);
        let gone = CheckText::Say(Say::NoAnswer {
            host: "imap.example.com".into(),
            port: 993,
        });
        assert_eq!(error_kind(&gone, false), Kind3::Unreachable);
        assert_eq!(
            error_kind(&CheckText::Say(Say::Class(Class::Offline)), false),
            Kind3::Unreachable
        );
        let cert = CheckText::Say(Say::SecureFailed {
            host: "mail.example.com".into(),
        });
        assert_eq!(error_kind(&cert, false), Kind3::Certificate);
        assert_eq!(
            error_kind(&CheckText::Say(Say::Class(Class::Other)), true),
            Kind3::Certificate
        );
        assert_eq!(error_kind(&CheckText::NoSavedPassword, false), Kind3::Other);

        assert_eq!(act(Kind3::Rejected, false, false, false), Act::SetUsername);
        assert_eq!(act(Kind3::Rejected, true, false, false), Act::None);
        assert_eq!(act(Kind3::Unreachable, false, false, true), Act::TryAgain);
        assert_eq!(
            act(Kind3::Certificate, false, true, true),
            Act::ReviewCertificate
        );
        assert_eq!(
            act(Kind3::Certificate, false, false, false),
            Act::SetUpManually
        );
        assert_eq!(act(Kind3::Other, false, false, true), Act::TryAgain);
        assert_eq!(act(Kind3::Other, false, false, false), Act::SetUpManually);
    }

    /// Ports `the_preview_shows_the_first_line`.
    #[test]
    fn the_preview_shows_the_first_line() {
        assert_eq!(first_line("Ana Silva  \nExample Ltd"), "Ana Silva");
        assert_eq!(first_line(""), "");
        assert_eq!(first_line("\nsecond"), "");
    }

    /// Ports `the_note_names_the_source_and_what_was_left_out`
    /// (src/app/find.rs).
    #[test]
    fn the_note_names_the_source_and_what_was_left_out() {
        komorebi_i18n::with_lang(Lang::En, || {
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
        });
    }
}
