//! What went wrong, in the user's words: a small classifier over worker
//! errors. Each [`Class`] maps to one sentence with a next step; the app
//! shows a class once per account until it clears ([`Problem`]).
//! The error predicates the workers act on live here too.

use super::imap::{self, Failure};
use super::smtp;
use crate::secrets;

/// What to do about a rejected sign-in.
pub const SIGN_IN_HINT: &str = "Update the password in Settings, or press Refresh to retry.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// The server can't be reached: no network, DNS, a dropped connection.
    Offline,
    /// The server rejected the sign-in, or no password is saved.
    Auth,
    /// The secure connection couldn't be set up (often an untrusted
    /// certificate, or the wrong server name).
    TlsCert,
    /// The server refused a message for its size.
    TooLarge,
    /// The mailbox is over its quota.
    Quota,
    /// The server refused the sign-in for now: too many connections, or
    /// temporarily unavailable. Never latches the sign-in.
    TooManyConnections,
    Other,
}

impl Class {
    /// The sentence the status line shows (none for [`Class::Other`]: its
    /// own text says more).
    pub fn sentence(self) -> &'static str {
        match self {
            Class::Offline => {
                "Can't reach the server. Komorebi tries again by itself; check your internet connection."
            }
            Class::Auth => {
                "The server rejected the sign-in. Update the password in Settings, or press Refresh to retry."
            }
            Class::TlsCert => {
                "Couldn't make a secure connection to the server. Check the server name in Settings; its certificate may not be trusted."
            }
            Class::TooLarge => {
                "The server says this message is too large. Remove an attachment or share it another way."
            }
            Class::Quota => {
                "The mailbox is full on the server. Delete or archive some mail, then try again."
            }
            Class::TooManyConnections => {
                "The server is busy or has too many connections open. Komorebi tries again shortly; close other mail apps if this keeps happening."
            }
            Class::Other => "",
        }
    }

    /// The class's name in the diagnostics log.
    pub fn name(self) -> &'static str {
        match self {
            Class::Offline => "OFFLINE",
            Class::Auth => "AUTH",
            Class::TlsCert => "TLS",
            Class::TooLarge => "TOO-LARGE",
            Class::Quota => "QUOTA",
            Class::TooManyConnections => "BUSY",
            Class::Other => "ERROR",
        }
    }
}

/// A failure as the user sees it: its class and the error's own text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub class: Class,
    /// The full error chain (`{e:#}`).
    pub detail: String,
}

impl Problem {
    pub fn of(e: &anyhow::Error) -> Self {
        Self {
            class: classify(e),
            detail: format!("{e:#}"),
        }
    }

    /// What the status line says: the class's sentence, or for
    /// [`Class::Other`] the error itself.
    pub fn text(&self) -> &str {
        match self.class {
            Class::Other => &self.detail,
            class => class.sentence(),
        }
    }

    /// What decides whether two problems are "the same" for showing it
    /// once: the class, and for [`Class::Other`] also the text.
    pub fn key(&self) -> (Class, &str) {
        match self.class {
            Class::Other => (Class::Other, &self.detail),
            class => (class, ""),
        }
    }
}

pub fn classify(e: &anyhow::Error) -> Class {
    let refusal = match imap::failure(e) {
        Some(Failure::No(t) | Failure::Bad(t)) => Some(t),
        _ => None,
    };
    let code = refusal.as_deref().and_then(imap::response_code);
    let upper = refusal.as_deref().map(str::to_ascii_uppercase);
    let has_code = |codes: &[&str]| code.as_deref().is_some_and(|c| codes.contains(&c));
    if is_auth_error(e) || e.chain().any(|c| c.is::<SignInLatched>()) {
        Class::Auth
    } else if e.chain().any(|c| c.is::<imap::TlsFailed>()) {
        Class::TlsCert
    } else if has_code(&["OVERQUOTA"]) || upper.as_deref().is_some_and(|t| t.contains("QUOTA")) {
        Class::Quota
    } else if has_code(&["TOOBIG"]) || smtp::too_large(e) {
        Class::TooLarge
    } else if has_code(&["UNAVAILABLE", "LIMIT", "INUSE"])
        || upper.as_deref().is_some_and(imap::transient_text)
    {
        Class::TooManyConnections
    } else if is_unreachable(e) {
        Class::Offline
    } else {
        Class::Other
    }
}

/// Every account job fails with this while the sign-in is rejected: no
/// LOGIN is tried until the password changes or Refresh.
#[derive(Debug)]
pub struct SignInLatched;

impl std::fmt::Display for SignInLatched {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Sign-in rejected. {SIGN_IN_HINT}")
    }
}

impl std::error::Error for SignInLatched {}

/// The folder is no longer in the cache (the server's folder list dropped
/// it).
#[derive(Debug)]
pub struct FolderGone;

impl std::fmt::Display for FolderGone {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("folder no longer exists")
    }
}

impl std::error::Error for FolderGone {}

/// The connection is unusable: I/O failed, or a response couldn't be parsed
/// (the stream is out of step). A tagged BAD leaves it in sync, so it's not
/// one of these.
pub fn is_connection_error(e: &anyhow::Error) -> bool {
    is_unreachable(e) || is_parse_error(e)
}

/// The server could not be reached, or the connection dropped: a network
/// or socket error rather than an answer.
pub fn is_unreachable(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<std::io::Error>()) || imap::failure(e) == Some(Failure::Unreachable)
}

/// A reply the parser can't read (or one the stream refused): the
/// command may have run, so it is never retried on the same connection.
pub fn is_parse_error(e: &anyhow::Error) -> bool {
    imap::failure(e) == Some(Failure::Protocol)
}

/// Signing in again can't work until the password changes.
pub fn is_auth_error(e: &anyhow::Error) -> bool {
    e.chain()
        .any(|c| c.is::<imap::AuthFailed>() || c.is::<secrets::NoPassword>())
}

/// A tagged NO or BAD (to LOGIN: a refusal other than a wrong password).
pub fn is_refusal(e: &anyhow::Error) -> bool {
    matches!(imap::failure(e), Some(Failure::No(_) | Failure::Bad(_)))
}

/// The message is gone: expunged by another client (`[EXPUNGEISSUED]`,
/// Dovecot's "no longer exist"), or with its folder (`[NONEXISTENT]`, or
/// no longer cached: [`FolderGone`]). Flags and deletes on such a message
/// have nothing left to do.
pub fn message_gone(e: &anyhow::Error) -> bool {
    if e.chain().any(|c| c.is::<FolderGone>()) {
        return true;
    }
    let Some(Failure::No(text)) = imap::failure(e) else {
        return false;
    };
    let upper = text.to_ascii_uppercase();
    matches!(
        imap::response_code(&text).as_deref(),
        Some("NONEXISTENT" | "EXPUNGEISSUED")
    ) || upper.contains("NO LONGER EXIST")
        || upper.contains("NO SUCH MESSAGE")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(kind: std::io::ErrorKind) -> anyhow::Error {
        anyhow::Error::new(std::io::Error::from(kind)).context("connecting to imap.example.org:993")
    }

    #[test]
    fn classes_and_their_sentences() {
        use std::io::ErrorKind;
        let offline = Problem::of(&io(ErrorKind::TimedOut));
        assert_eq!(offline.class, Class::Offline);
        assert!(offline.text().starts_with("Can't reach the server."));
        assert!(offline.detail.contains("imap.example.org"));
        let auth = anyhow::Error::new(imap::AuthFailed("Invalid".into()));
        assert_eq!(classify(&auth), Class::Auth);
        assert_eq!(classify(&anyhow::Error::new(SignInLatched)), Class::Auth);
        assert_eq!(
            classify(&anyhow::Error::new(secrets::NoPassword)),
            Class::Auth
        );
        let tls = anyhow::Error::new(imap::TlsFailed("imap.example.org".into()));
        assert_eq!(classify(&tls), Class::TlsCert);
        // A plain error keeps its own words.
        let other = Problem::of(&anyhow::anyhow!("opening folder X"));
        assert_eq!(
            (other.class, other.text()),
            (Class::Other, "opening folder X")
        );
        // Every class but Other has one sentence with a next step.
        for class in [
            Class::Offline,
            Class::Auth,
            Class::TlsCert,
            Class::TooLarge,
            Class::Quota,
            Class::TooManyConnections,
        ] {
            let s = class.sentence();
            assert!(s.ends_with('.') && s.matches(". ").count() >= 1, "{s}");
        }
        // Shown once per class, but Other per text.
        assert_eq!(
            offline.key(),
            Problem::of(&io(ErrorKind::ConnectionReset)).key()
        );
        assert_ne!(other.key(), Problem::of(&anyhow::anyhow!("else")).key());
    }
}
