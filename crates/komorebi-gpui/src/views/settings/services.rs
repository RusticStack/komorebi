//! What Settings and onboarding reach outside the window: the account check
//! (the servers), the settings lookup (DNS and the provider's autoconfig),
//! the certificate probe, and Windows Credential Manager. A GPUI global, so
//! the tests put fakes in its place and nothing reaches the network or the
//! real credential store; with none set, the app gets the real ones (and a
//! test build gets fakes that refuse, so a forgotten set-up never touches
//! a real password).

use gpui::{App, Global};
use komorebi::mail::discover::Outcome;
use komorebi::mail::pin::Kind;
use komorebi::providers::Server;

use super::check::{ReadPassword, Verifier};
use super::pins::Unreadable;

/// Reads the certificate a server presents (no sign-in): the server's own
/// and, when sent, its CA's.
pub type Probe = fn(&Server, Kind) -> Result<(Vec<u8>, Option<Vec<u8>>), Unreadable>;

/// Looks an address's domain up (`mail::discover::discover`).
pub type Lookup = fn(&str) -> Outcome;

/// Saves an account's password.
pub type StorePassword = fn(i64, &str) -> anyhow::Result<()>;

/// Forgets an account's password.
pub type ForgetPassword = fn(i64) -> anyhow::Result<()>;

/// Reads what classic Outlook keeps on this PC (Import from Outlook).
pub type FindOutlook = fn() -> komorebi::import::outlook::Found;

/// The outside world of the account forms.
#[derive(Clone, Copy)]
pub struct Services {
    pub verify: Verifier,
    pub read_password: ReadPassword,
    pub store_password: StorePassword,
    pub forget_password: ForgetPassword,
    pub probe: Probe,
    pub lookup: Lookup,
    pub outlook: FindOutlook,
    /// `KOMOREBI_OFFLINE`: no check, lookup or probe is tried.
    pub offline: bool,
}

impl Global for Services {}

impl Default for Services {
    fn default() -> Self {
        if cfg!(test) { refusing() } else { real() }
    }
}

/// The ones the app uses.
pub fn real() -> Services {
    Services {
        verify: super::check::VERIFY,
        read_password: komorebi::secrets::get_password,
        store_password: komorebi::secrets::set_password,
        forget_password: komorebi::secrets::delete_password,
        probe: real_probe,
        lookup: real_lookup,
        outlook: real_outlook,
        offline: komorebi::dev_var("KOMOREBI_OFFLINE").is_some(),
    }
}

fn real_probe(server: &Server, kind: Kind) -> Result<(Vec<u8>, Option<Vec<u8>>), Unreadable> {
    komorebi::mail::pin::probe_certificate(&server.host, server.port, server.security, kind)
        .map(|p| (p.leaf, p.issuer))
        .map_err(|e| super::pins::unreadable(server, &e))
}

fn real_lookup(email: &str) -> Outcome {
    komorebi::mail::discover::discover(email, &komorebi::mail::discover::system_lookups())
}

/// Outlook's accounts and signatures. Nothing during a snapshot: it shows
/// made-up ones (`ob-import`), never this PC's.
fn real_outlook() -> komorebi::import::outlook::Found {
    if komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some() {
        return komorebi::import::outlook::Found::default();
    }
    komorebi::import::outlook::discover()
}

/// Fakes that touch nothing and refuse what would need the outside world.
pub fn refusing() -> Services {
    fn verify(
        imap: &Server,
        _: &Server,
        _: komorebi::mail::Pins,
        _: (&str, &str),
        _: &str,
        _: bool,
        _: &mut dyn FnMut(komorebi::mail::errors::Step),
    ) -> Result<(), komorebi::mail::errors::StepError> {
        Err(komorebi::mail::errors::StepError {
            step: komorebi::mail::errors::Step::Resolve,
            class: komorebi::mail::errors::Class::Offline,
            special: komorebi::mail::errors::Special::None,
            host: imap.host.clone(),
            port: imap.port,
            login: String::new(),
            source: anyhow::anyhow!("no network in this build"),
        })
    }
    Services {
        verify,
        read_password: |_| anyhow::bail!("no credential store in this build"),
        store_password: |_, _| Ok(()),
        forget_password: |_| Ok(()),
        probe: |server, _| {
            Err(Unreadable::Unread {
                address: super::hosts::address(server),
            })
        },
        lookup: |_| Outcome::default(),
        outlook: komorebi::import::outlook::Found::default,
        offline: false,
    }
}

/// The services in use.
pub fn get(cx: &App) -> Services {
    cx.try_global::<Services>().copied().unwrap_or_default()
}
