//! What went wrong, as typed values: a small classifier over worker errors.
//! Each [`Class`] maps to one [`Say`] with a next step; the app shows a class
//! once per account until it clears ([`Problem`]). The library writes no
//! sentences for the screen: the app renders a [`Say`] in the display
//! language (`src/app/text.rs`). The error predicates the workers act on live here
//! too.

use super::imap::{self, Failure};
use super::smtp;
use crate::secrets;

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
    /// What the status line says about the class (none for
    /// [`Class::Other`]: the error's own text says more).
    pub fn say(self) -> Option<Say> {
        match self {
            Class::Other => None,
            class => Some(Say::Class(class)),
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

/// The steps of "Test connection" (and of Add and Save, which share it),
/// in the order they run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// Look up the incoming server's name.
    Resolve,
    /// Open the TCP connection to the incoming server.
    Connect,
    /// TLS, or STARTTLS and then TLS.
    Secure,
    /// IMAP `LOGIN`, then `NOOP`.
    ImapSignIn,
    /// IMAP `LIST`.
    ListFolders,
    /// The sending server: connect, EHLO and TLS.
    SmtpConnect,
    /// The sending server: `AUTH`.
    SmtpSignIn,
}

impl Step {
    pub const ALL: [Step; 7] = [
        Step::Resolve,
        Step::Connect,
        Step::Secure,
        Step::ImapSignIn,
        Step::ListFolders,
        Step::SmtpConnect,
        Step::SmtpSignIn,
    ];

    /// The position in [`Step::ALL`].
    pub fn index(self) -> usize {
        Step::ALL.iter().position(|&s| s == self).unwrap_or(0)
    }

    /// The step's name in the diagnostics log.
    pub fn name(self) -> &'static str {
        match self {
            Step::Resolve => "RESOLVE",
            Step::Connect => "CONNECT",
            Step::Secure => "SECURE",
            Step::ImapSignIn => "IMAP-SIGN-IN",
            Step::ListFolders => "LIST",
            Step::SmtpConnect => "SMTP-CONNECT",
            Step::SmtpSignIn => "SMTP-SIGN-IN",
        }
    }
}

/// A failure with a sentence of its own, beyond what its step and class say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    None,
    /// STARTTLS is not offered or was refused.
    NoStartTls,
    /// A TLS port answered in plain text.
    PlainAnswer,
    /// The sending server offers no sign-in.
    NoAuth,
    /// The sending server's name could not be looked up.
    SmtpResolve,
    /// A server with a saved certificate presented another one.
    CertChanged,
    /// The incoming server's greeting was a goodbye, a sign-in by itself
    /// (PREAUTH) or not IMAP at all.
    BadGreeting,
    /// The incoming server offers no sign-in Komorebi can use over the
    /// secure connection.
    NoLogin,
    /// The incoming server only works in UTF-8 mode (`UTF8=ONLY`).
    Utf8Only,
}

/// What the outbox record says of a send that may have gone through. Never
/// resent automatically. (English by design: the cache is not tied to a
/// display language; the status line renders [`Say::Unconfirmed`].)
pub const UNCONFIRMED: &str =
    "The message may or may not have been sent; check Sent before you retry it from the Outbox.";

/// A sentence for the user, as a value: which one it is and the facts it
/// names (the server, the port, the sign-in name). The app renders it in the
/// language shown; only [`Say::record`] has English, for what the cache and
/// the logs keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Say {
    /// The shared sentence of a class (not [`Class::Other`]).
    Class(Class),
    /// The server's name could not be looked up.
    NotFound { host: String },
    /// The server didn't answer on the port.
    NoAnswer { host: String, port: u16 },
    /// The connection was lost.
    Lost { host: String },
    /// The server rejected the password of `login`.
    Rejected { host: String, login: String },
    /// The certificate isn't trusted or doesn't match the name.
    SecureFailed { host: String },
    /// The server didn't accept the connection on the port.
    NoConnection { host: String, port: u16 },
    /// The secure connection couldn't be set up.
    NoSecure { host: String },
    /// The server wouldn't let `login` sign in.
    NoSignIn { host: String, login: String },
    /// Signed in, but the folder list didn't come.
    NoFolders { host: String },
    /// No STARTTLS, so no sign-in.
    NoStartTls { host: String },
    /// A TLS port answered in plain text.
    PlainAnswer,
    /// The sending server offers no sign-in.
    NoAuth { host: String },
    /// The greeting was not an IMAP server's.
    BadGreeting { host: String },
    /// The server allows no password sign-in over this connection.
    NoLogin { host: String },
    /// The server only works in UTF-8 mode.
    Utf8Only { host: String },
    /// A saved certificate no longer matches the server's.
    CertChanged { host: String },
    /// The sending server refused the From address.
    SenderRefused { host: String, from: String },
    /// Every job fails while the sign-in is rejected.
    SignInLatched,
    /// The send may have gone out; `detail` is the failure's own text.
    Unconfirmed { detail: String },
    /// The server refused the message for its size.
    TooLargeSend { detail: String },
}

impl Say {
    /// The English line the cache and the logs keep for the sentences a
    /// send leaves with its outbox row (`Outcome`); `None` for the others.
    /// Never shown as a sentence: the status line renders the [`Say`]
    /// (the render test in `src/app/text.rs` keeps the two in step in English).
    pub fn record(&self) -> Option<String> {
        Some(match self {
            Say::Class(Class::TlsCert) => "Couldn't make a secure connection to the server. Check the server name in Settings; its certificate may not be trusted.".to_string(),
            Say::CertChanged { host } => format!(
                "The certificate of {host} changed. If you didn't expect that, don't sign in: review the new certificate and compare its fingerprint before you trust it."
            ),
            Say::SenderRefused { host, from } => format!(
                "{host} refused to send as {from}. Check that this account may send from that address, or set the SMTP username."
            ),
            Say::SignInLatched => {
                "Sign-in rejected. Update the password in Settings, or press Refresh to retry.".to_string()
            }
            Say::Unconfirmed { detail } => format!("{detail}. {UNCONFIRMED}"),
            Say::TooLargeSend { detail } => {
                format!("The server says this message is too large ({detail})")
            }
            _ => return None,
        })
    }
}

/// "Test connection" failed at `step`. `source` is the full chain, for
/// local diagnosis; the user sees [`describe_error`] only.
#[derive(Debug)]
pub struct StepError {
    pub step: Step,
    pub class: Class,
    pub special: Special,
    pub host: String,
    pub port: u16,
    /// The sign-in name used at this step.
    pub login: String,
    pub source: anyhow::Error,
}

impl StepError {
    /// What the form says.
    pub fn say(&self) -> Say {
        describe_error(self)
    }
}

impl std::fmt::Display for StepError {
    /// For the logs: the step and class codes, never the server's words.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} {}", self.step.name(), self.class.name())
    }
}

/// The sentence for a failed step: what happened to `host`, and what to do
/// about it. `login` is the sign-in name of the step.
pub fn describe(step: Step, class: Class, host: &str, port: u16, login: &str) -> Say {
    let host = host.to_string();
    let login = login.to_string();
    match (class, step) {
        (Class::Offline, Step::Resolve) => Say::NotFound { host },
        (Class::Offline, Step::Connect | Step::SmtpConnect) => Say::NoAnswer { host, port },
        (Class::Offline, _) => Say::Lost { host },
        (Class::Auth, _) => Say::Rejected { host, login },
        (Class::TlsCert, _) => Say::SecureFailed { host },
        (Class::TooLarge | Class::Quota | Class::TooManyConnections, _) => Say::Class(class),
        (Class::Other, Step::Resolve | Step::Connect | Step::SmtpConnect) => {
            Say::NoConnection { host, port }
        }
        (Class::Other, Step::Secure) => Say::NoSecure { host },
        (Class::Other, Step::ImapSignIn | Step::SmtpSignIn) => Say::NoSignIn { host, login },
        (Class::Other, Step::ListFolders) => Say::NoFolders { host },
    }
}

/// [`describe`] for a [`StepError`], with the sentences only some failures
/// have.
pub fn describe_error(e: &StepError) -> Say {
    let host = e.host.clone();
    match e.special {
        Special::NoStartTls => Say::NoStartTls { host },
        Special::CertChanged => Say::CertChanged { host },
        Special::PlainAnswer => Say::PlainAnswer,
        Special::NoAuth => Say::NoAuth { host },
        Special::BadGreeting => Say::BadGreeting { host },
        Special::NoLogin => Say::NoLogin { host },
        Special::Utf8Only => Say::Utf8Only { host },
        Special::SmtpResolve => describe(Step::Resolve, Class::Offline, &host, e.port, &e.login),
        Special::None => describe(e.step, e.class, &host, e.port, &e.login),
    }
}

/// The sentence for a message the sending server refused because of its
/// From address (553 or 550 "sender not owned by login").
pub fn sender_refused(host: &str, from: &str) -> Say {
    Say::SenderRefused {
        host: host.to_string(),
        from: from.to_string(),
    }
}

/// What a sending (SMTP) failure says about its class, for the ones that
/// are certain: a TLS failure, a refused sign-in (535, 534, 530, 538) and a
/// temporary sign-in failure (454). Everything else is left to the caller.
pub fn classify_smtp(e: &anyhow::Error) -> Option<Class> {
    smtp::step_of(e)
        .map(|(_, class, _)| class)
        .filter(|c| matches!(c, Class::TlsCert | Class::Auth | Class::TooManyConnections))
}

/// A failure as the user sees it: its class and the error's own text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub class: Class,
    /// The full error chain (`{e:#}`).
    pub detail: String,
    /// Set when a saved certificate no longer matches: the server's name.
    pub changed: Option<String>,
}

impl Problem {
    pub fn of(e: &anyhow::Error) -> Self {
        Self {
            class: classify(e),
            detail: format!("{e:#}"),
            changed: changed_in(e).or_else(|| {
                e.chain()
                    .find_map(|c| c.downcast_ref::<CertLatched>())
                    .and_then(|l| l.0.clone())
            }),
        }
    }

    /// What the status line says: the changed certificate's sentence, or the
    /// class's. `None` for [`Class::Other`]: the error itself
    /// ([`Problem::detail`]) is all there is.
    pub fn say(&self) -> Option<Say> {
        match (&self.changed, self.class) {
            (Some(host), _) => Some(Say::CertChanged { host: host.clone() }),
            (None, class) => class.say(),
        }
    }

    /// What decides whether two problems are "the same" for showing it
    /// once: the class, and for [`Class::Other`] also the text.
    pub fn key(&self) -> (Class, &str) {
        match self.class {
            Class::Other => (Class::Other, &self.detail),
            class => (
                class,
                if self.changed.is_some() {
                    "changed"
                } else {
                    ""
                },
            ),
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
    let smtp_class = classify_smtp(e);
    if is_auth_error(e) || e.chain().any(|c| c.is::<SignInLatched>()) {
        Class::Auth
    } else if e.chain().any(|c| {
        c.is::<imap::TlsFailed>()
            || c.is::<imap::StartTlsRefused>()
            || c.is::<super::pin::PinMismatch>()
            || c.is::<CertLatched>()
    }) {
        Class::TlsCert
    } else if let Some(class) = smtp_class {
        class
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

/// Why an account's workers stopped trying, and the error every job fails
/// with meanwhile. Shared by the account's sync, interactive and IDLE
/// threads: once set, none of them connects (so no password is sent and no
/// server is hammered) until the user changes the password, edits the
/// account or presses Refresh ([`Latch::clear`]).
#[derive(Clone, Default)]
pub struct Latch(std::sync::Arc<std::sync::Mutex<Option<Why>>>);

/// What an account's workers wait for the user to fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    /// The sign-in was rejected (or no password is saved).
    SignIn,
    /// The secure connection failed: a changed or untrusted certificate, or
    /// no STARTTLS.
    Secure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Why {
    /// The server rejected the sign-in, or no password is saved.
    SignIn,
    /// The secure connection failed (an untrusted or changed certificate,
    /// or STARTTLS refused). `changed` is the server whose saved
    /// certificate no longer matches.
    Secure { changed: Option<String> },
}

impl Latch {
    fn state(&self) -> std::sync::MutexGuard<'_, Option<Why>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the workers wait for the user.
    pub fn get(&self) -> bool {
        self.state().is_some()
    }

    /// Whether the wait is about the secure connection, not the sign-in.
    pub fn is_secure(&self) -> bool {
        matches!(*self.state(), Some(Why::Secure { .. }))
    }

    /// What the workers wait for, if they wait.
    pub fn attention(&self) -> Option<Attention> {
        match *self.state() {
            None => None,
            Some(Why::SignIn) => Some(Attention::SignIn),
            Some(Why::Secure { .. }) => Some(Attention::Secure),
        }
    }

    /// Latch for a rejected sign-in.
    pub fn set_sign_in(&self) {
        *self.state() = Some(Why::SignIn);
    }

    /// Latch if `e` is a failure only the user can fix (see
    /// [`needs_attention`]); returns whether it did.
    pub fn set_from(&self, e: &anyhow::Error) -> bool {
        let why = if is_auth_error(e) {
            Why::SignIn
        } else if needs_attention(e) {
            Why::Secure {
                changed: changed_in(e),
            }
        } else {
            return false;
        };
        *self.state() = Some(why);
        true
    }

    /// Let the workers try again.
    pub fn clear(&self) {
        *self.state() = None;
    }

    /// What a job fails with while latched.
    pub fn error(&self) -> anyhow::Error {
        match &*self.state() {
            Some(Why::Secure { changed }) => CertLatched(changed.clone()).into(),
            _ => SignInLatched.into(),
        }
    }
}

/// The server whose changed saved certificate is in `e`'s chain.
fn changed_in(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|c| c.downcast_ref::<super::pin::PinMismatch>())
        .map(|m| m.0.clone())
}

/// A failure that retrying cannot fix and that only the user can: the
/// sign-in was rejected, a saved certificate changed, the certificate is not
/// trusted, or the server will not do STARTTLS. The workers make one
/// attempt and then wait ([`Latch`]).
pub fn needs_attention(e: &anyhow::Error) -> bool {
    is_auth_error(e)
        || e.chain().any(|c| {
            c.is::<super::pin::PinMismatch>()
                || c.is::<imap::TlsFailed>()
                || c.is::<imap::StartTlsRefused>()
        })
}

/// Every account job fails with this while the secure connection is the
/// reason the workers wait.
#[derive(Debug)]
pub struct CertLatched(pub Option<String>);

impl std::fmt::Display for CertLatched {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.0 {
            Some(host) => write!(f, "the certificate of {host} changed; waiting for the user"),
            None => f.write_str("the secure connection failed; waiting for the user"),
        }
    }
}

impl std::error::Error for CertLatched {}

/// Every account job fails with this while the sign-in is rejected: no
/// LOGIN is tried until the password changes or Refresh.
#[derive(Debug)]
pub struct SignInLatched;

impl std::fmt::Display for SignInLatched {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // The outbox record of a send that failed on it.
        f.write_str(&Say::SignInLatched.record().unwrap_or_default())
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
    e.chain()
        .any(|c| c.is::<std::io::Error>() || c.is::<imap::ConnectionClosed>())
        || imap::failure(e) == Some(Failure::Unreachable)
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
        assert_eq!(offline.say(), Some(Say::Class(Class::Offline)));
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
            (other.class, other.say(), other.detail.as_str()),
            (Class::Other, None, "opening folder X")
        );
        // Every class but Other has one sentence of its own.
        for class in [
            Class::Offline,
            Class::Auth,
            Class::TlsCert,
            Class::TooLarge,
            Class::Quota,
            Class::TooManyConnections,
        ] {
            assert_eq!(class.say(), Some(Say::Class(class)));
        }
        assert_eq!(Class::Other.say(), None);
        // Shown once per class, but Other per text.
        assert_eq!(
            offline.key(),
            Problem::of(&io(ErrorKind::ConnectionReset)).key()
        );
        assert_ne!(other.key(), Problem::of(&anyhow::anyhow!("else")).key());
    }

    #[test]
    fn every_step_and_class_has_a_sentence_with_its_facts() {
        let classes = [
            Class::Offline,
            Class::Auth,
            Class::TlsCert,
            Class::TooLarge,
            Class::Quota,
            Class::TooManyConnections,
            Class::Other,
        ];
        let host = || "mail.example.com".to_string();
        for step in Step::ALL {
            for class in classes {
                let say = describe(step, class, "mail.example.com", 1143, "alice");
                match class {
                    // The busy, size and quota sentences are the shared ones.
                    Class::TooLarge | Class::Quota | Class::TooManyConnections => {
                        assert_eq!(say, Say::Class(class), "{step:?} {class:?}");
                    }
                    Class::Auth => assert_eq!(
                        say,
                        Say::Rejected {
                            host: host(),
                            login: "alice".into()
                        },
                        "{step:?}"
                    ),
                    _ => assert!(!matches!(say, Say::Class(_)), "{step:?} {class:?}: {say:?}"),
                }
            }
        }
        let say = |s, c| describe(s, c, "imap.example.com", 993, "me@example.com");
        let host = || "imap.example.com".to_string();
        assert_eq!(
            say(Step::Connect, Class::Offline),
            Say::NoAnswer {
                host: host(),
                port: 993
            }
        );
        assert_eq!(
            say(Step::Resolve, Class::Offline),
            Say::NotFound { host: host() }
        );
        assert_eq!(
            say(Step::Secure, Class::Offline),
            Say::Lost { host: host() }
        );
        assert_eq!(
            say(Step::Secure, Class::TlsCert),
            Say::SecureFailed { host: host() }
        );
        assert_eq!(
            say(Step::SmtpConnect, Class::Other),
            Say::NoConnection {
                host: host(),
                port: 993
            }
        );
        assert_eq!(
            say(Step::Secure, Class::Other),
            Say::NoSecure { host: host() }
        );
        assert_eq!(
            say(Step::SmtpSignIn, Class::Auth),
            Say::Rejected {
                host: host(),
                login: "me@example.com".into()
            }
        );
        assert_eq!(
            say(Step::ImapSignIn, Class::Other),
            Say::NoSignIn {
                host: host(),
                login: "me@example.com".into()
            }
        );
        assert_eq!(
            say(Step::ListFolders, Class::Other),
            Say::NoFolders { host: host() }
        );
    }

    #[test]
    fn special_failures_have_their_own_sentences() {
        let error = |special| StepError {
            step: Step::SmtpSignIn,
            class: Class::Auth,
            special,
            host: "smtp.example.com".into(),
            port: 587,
            login: "me".into(),
            source: anyhow::anyhow!("raw chain"),
        };
        let host = || "smtp.example.com".to_string();
        assert_eq!(error(Special::NoAuth).say(), Say::NoAuth { host: host() });
        assert_eq!(
            error(Special::NoStartTls).say(),
            Say::NoStartTls { host: host() }
        );
        assert_eq!(error(Special::PlainAnswer).say(), Say::PlainAnswer);
        assert_eq!(
            error(Special::SmtpResolve).say(),
            Say::NotFound { host: host() }
        );
        assert_eq!(
            error(Special::CertChanged).say(),
            Say::CertChanged { host: host() }
        );
        assert_eq!(
            error(Special::BadGreeting).say(),
            Say::BadGreeting { host: host() }
        );
        assert_eq!(error(Special::NoLogin).say(), Say::NoLogin { host: host() });
        assert_eq!(
            error(Special::Utf8Only).say(),
            Say::Utf8Only { host: host() }
        );
        assert_eq!(
            error(Special::None).say(),
            Say::Rejected {
                host: host(),
                login: "me".into()
            }
        );
        // The logs get the codes, never the chain or the server's words.
        assert_eq!(error(Special::None).to_string(), "SMTP-SIGN-IN AUTH");
    }

    #[test]
    fn only_the_sentences_an_outbox_row_keeps_have_a_record() {
        assert!(
            Say::SignInLatched
                .record()
                .unwrap()
                .starts_with("Sign-in rejected.")
        );
        assert!(
            sender_refused("smtp.example.com", "me@example.com")
                .record()
                .unwrap()
                .starts_with("smtp.example.com refused to send as me@example.com.")
        );
        assert!(Say::Class(Class::Offline).record().is_none());
        assert!(Say::NotFound { host: "x".into() }.record().is_none());
    }
}
