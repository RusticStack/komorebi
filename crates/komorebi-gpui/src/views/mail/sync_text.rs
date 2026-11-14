//! The words of the sync parts, in the language shown: the status line's
//! sentences from their typed values ([`Said`]), the action buttons, "Updated
//! 14:05", the account lines of Settings. Each `tr!` is the Slint app's text
//! and context (`ui/tr.slint`, `src/app/text.rs`), so all are translated
//! already.

use komorebi::mail::errors::Class;
use komorebi::mail::{Notice, Outcome, Reason};
use komorebi_i18n::tr;

use super::sync::{Said, StatusAction};
use crate::text;
use crate::views::composer::Status as ComposerStatus;

/// A sentence for a problem: its own, or for an error with none, the
/// error's words.
pub fn problem(p: &komorebi::mail::errors::Problem) -> String {
    p.say().map_or_else(|| p.detail.clone(), |s| text::say(&s))
}

/// A line about one account: its address, then `text`.
pub fn account_line(email: &str, text: &str) -> String {
    tr!("{0}: {1}", email, text)
}

/// A failure of the cache or the system behind a lead-in sentence.
pub fn detail(detail: &str) -> String {
    tr!("Something went wrong. {0}", detail)
}

/// A caveat of an action that worked.
pub fn notice(n: &Notice) -> String {
    match n {
        Notice::LeftFlagged => tr!("Removed here; the server keeps it flagged as deleted."),
        Notice::MoveGone => tr!("Already gone from the server, so there was nothing to move."),
        Notice::NoDraftsFolder => {
            tr!("There is no Drafts folder on the server, so drafts are only saved on this PC.")
        }
        Notice::NoSentFolder => tr!(
            "This account has no Sent folder on the server, so sent mail isn't saved there. You can choose one in Settings: Accounts, Edit."
        ),
        Notice::NoDraftUid => tr!(
            "A draft saved to the server couldn't be found there, so drafts are only saved on this PC."
        ),
        Notice::DraftsNotSaved(d) => tr!(
            "Couldn't save drafts to the server ({0}); they are kept on this PC.",
            d
        ),
    }
}

/// How a failed send reads: its sentence, or the failure's own words.
fn reason(r: &Reason) -> String {
    r.say.as_ref().map_or_else(|| r.text.clone(), text::say)
}

/// How a send ended, for the status line.
pub fn outcome(o: &Outcome) -> String {
    match o {
        Outcome::Delivered => text::sent(),
        Outcome::DeliveredNotSaved(d) => tr!("Message sent, but {0}", d),
        Outcome::Retry(r) => tr!(
            "Not sent yet: {0}. It stays in the Outbox and is tried again shortly.",
            reason(r)
        ),
        Outcome::Failed(r) => tr!("Not sent: {0}. It is in the Outbox.", reason(r)),
        Outcome::Unconfirmed(r) => reason(r),
    }
}

/// "{email}: downloading {done} of {total} in {folder} …".
pub fn downloading(email: &str, done: usize, total: usize, folder: &str) -> String {
    let lang = komorebi_i18n::current();
    tr!(
        "{0}: downloading {1} of {2} in {3}\u{a0}…",
        email,
        komorebi::format::thousands(lang, done),
        komorebi::format::thousands(lang, total),
        folder
    )
}

/// The status line's sentence.
pub fn said(s: &Said) -> String {
    match s {
        Said::Text(t) => t.clone(),
        Said::Detail(t) => detail(t),
        Said::Problem { email, problem: p } => {
            // A changed certificate in one short sentence; its card in
            // Settings has the whole of it. An account out of reach (the
            // standing line) says it as everywhere.
            let said = match &p.changed {
                Some(host) => tr!(
                    "The certificate of {0} changed. Review it before you connect again.",
                    text::host(host)
                ),
                None => problem(p),
            };
            account_line(email, &said)
        }
        Said::Notice { email, notice: n } => account_line(email, &notice(n)),
        Said::Downloading {
            email,
            done,
            total,
            folder,
        } => downloading(email, *done, *total, folder),
        Said::Outcome(o) => outcome(o),
        Said::Composer(status) => composer(status),
        Said::Sending { secs: 0, .. } => tr!("Sending\u{a0}…"),
        Said::Sending { secs, subject } if subject.is_empty() => tr!("Sending in {0} s", secs),
        Said::Sending { secs, subject } => {
            tr!("Sending \u{201c}{0}\u{201d} in {1} s", subject, secs)
        }
        Said::Discarded => tr!("Message discarded."),
        Said::RebuildFailed => tr!(
            "A message sent as Komorebi closed couldn't be prepared again; it is back in Drafts."
        ),
        Said::WhatsNew { version } => crate::views::updates::text::whats_new(version),
    }
}

fn composer(status: &ComposerStatus) -> String {
    status.text()
}

/// The label of the button a status action puts on the line.
pub fn action_label(action: StatusAction) -> Option<String> {
    Some(match action {
        StatusAction::None => return None,
        StatusAction::EditAccount(_) => tr!("Update password"),
        StatusAction::RetryAccount(_) => tr!("Retry"),
        StatusAction::ReviewCertificate(_) => tr!("Review certificate"),
        StatusAction::TryNow(_) => tr!("Try now"),
        StatusAction::WhatsNew => tr!("What's new"),
        StatusAction::UndoSend(_) => tr!("Undo"),
    })
}

/// The loader's name while an account syncs.
pub fn syncing() -> String {
    tr!("Syncing\u{a0}…")
}

/// "Updated 14:05" beside Refresh.
pub fn updated_at(time: &str) -> String {
    tr!("Updated {0}", time)
}

/// The Refresh button's name and tooltip: "Refresh (F5). Updated 14:05",
/// or "Refresh" before the first sync pass.
pub fn refresh(updated: Option<&str>) -> String {
    match updated {
        Some(time) => tr!("Refresh (F5). {0}", updated_at(time)),
        None => tr!("Refresh"),
    }
}

/// Whether a problem is the kind that stays said (a server out of reach).
pub fn stays(class: Class) -> bool {
    matches!(class, Class::Offline | Class::TooManyConnections)
}

/// An account's line in Settings (empty before its first sync).
pub fn account_status(
    state: Option<&super::sync::State>,
    attention: Option<komorebi::mail::errors::Attention>,
    host: &str,
) -> String {
    use komorebi::mail::errors::Attention;
    match attention {
        Some(Attention::SignIn) => return tr!("Sign-in rejected"),
        Some(Attention::Secure) => return tr!("Secure connection failed"),
        None => {}
    }
    match state {
        None => String::new(),
        Some(super::sync::State::Syncing) => tr!("Syncing\u{a0}…"),
        Some(super::sync::State::Checked(at)) => {
            match chrono::TimeZone::timestamp_opt(&chrono::Local, *at, 0).single() {
                Some(time) => tr!("Up to date, checked {0}", time.format("%H:%M")),
                None => tr!("Up to date"),
            }
        }
        Some(super::sync::State::Unreachable) => tr!("Can't reach {0}", host),
        Some(super::sync::State::Failed(p)) => tr!("Last sync failed: {0}", problem(p)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    /// The Slint app's sentences, word for word, in English and
    /// Portuguese.
    #[test]
    fn the_status_words_are_the_slint_apps() {
        with_lang(Lang::En, || {
            assert_eq!(
                said(&Said::Notice {
                    email: "me@example.com".into(),
                    notice: Notice::MoveGone
                }),
                "me@example.com: Already gone from the server, so there was nothing to move."
            );
            assert_eq!(
                said(&Said::Downloading {
                    email: "me@example.com".into(),
                    done: 1234,
                    total: 5000,
                    folder: "Inbox".into()
                }),
                "me@example.com: downloading 1,234 of 5,000 in Inbox\u{a0}…"
            );
            assert_eq!(said(&Said::Outcome(Outcome::Delivered)), "Sent");
            let sending = |secs, subject: &str| Said::Sending {
                secs,
                subject: subject.into(),
            };
            assert_eq!(said(&sending(10, "")), "Sending in 10 s");
            // The Slint `count_down`: the subject, then "Sending …" while
            // the message is still being built.
            assert_eq!(
                said(&sending(9, "Plans")),
                "Sending \u{201c}Plans\u{201d} in 9 s"
            );
            assert_eq!(said(&sending(0, "Plans")), "Sending\u{a0}…");
            assert_eq!(said(&Said::Discarded), "Message discarded.");
            assert_eq!(updated_at("14:05"), "Updated 14:05");
            assert_eq!(
                action_label(StatusAction::TryNow(1)).as_deref(),
                Some("Try now")
            );
            assert_eq!(action_label(StatusAction::None), None);
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(said(&Said::Discarded), "Mensagem descartada.");
            assert_eq!(
                action_label(StatusAction::EditAccount(1)).as_deref(),
                Some("Atualizar palavra-passe")
            );
            assert_eq!(
                action_label(StatusAction::UndoSend(1)).as_deref(),
                Some("Desfazer")
            );
        });
    }
}
