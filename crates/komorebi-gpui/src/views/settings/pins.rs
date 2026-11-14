//! Certificate pinning in the account form (`komorebi::mail::pin`; the
//! Slint app's `src/app/cert.rs`): "Review certificate" after a check
//! failed on an untrusted certificate, the prompt that shows the SHA-256
//! fingerprint, the pin kept in the form until Add or Save succeeds, and
//! "Forget certificate". Nothing is stored before the user confirms, and
//! the pin of a server goes when its host, port or security is edited.

use komorebi::db::Account;
use komorebi::mail::errors::{Class, Special, Step, StepError};
use komorebi::mail::pin::{self, Kind, PinState, Trust};
use komorebi::providers::Server;

use super::hosts::{address, host_for_display};
use super::text;

/// A certificate pinned for one server of the open form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinned {
    /// The saved bytes ([`pin::encode`]): what is trusted and its certificate.
    pub der: Vec<u8>,
    /// The server it was confirmed for; the pin is void for any other.
    pub server: Server,
}

/// The pin of one server in the form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    pinned: Option<Pinned>,
    /// A saved certificate that cannot be used (damaged), and the server it
    /// was saved for.
    damaged: Option<Server>,
    /// The fingerprint of the certificate just forgotten, shown beside the
    /// new one, and the server it was saved for (it goes with that server).
    before: Option<(String, Server)>,
}

/// The pins of the open account form: which account's they are (0: a new
/// one) and one slot per server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormPins {
    account: i64,
    slots: [Slot; 2],
}

/// The certificate a saved pin keeps (the server's, or its CA's).
fn shown(blob: &[u8]) -> &[u8] {
    pin::usable(Some(blob)).map_or(blob, |p| p.der)
}

fn index(kind: Kind) -> usize {
    match kind {
        Kind::Imap => 0,
        Kind::Smtp => 1,
    }
}

impl FormPins {
    /// The pins an account has saved.
    pub fn for_account(a: &Account) -> Self {
        let mut pins = Self {
            account: a.id,
            ..Self::default()
        };
        for (kind, blob, server) in [
            (Kind::Imap, &a.imap_pin, &a.imap),
            (Kind::Smtp, &a.smtp_pin, &a.smtp),
        ] {
            let slot = &mut pins.slots[index(kind)];
            match pin::state(blob.as_deref()) {
                PinState::None => {}
                PinState::Pinned(_) => {
                    slot.pinned = Some(Pinned {
                        der: blob.clone().unwrap_or_default(),
                        server: server.clone(),
                    });
                }
                PinState::Damaged => slot.damaged = Some(server.clone()),
            }
        }
        pins
    }

    /// Whose pins these are (0: a new account's).
    pub fn account(&self) -> i64 {
        self.account
    }

    /// Drop what was saved for a server other than the one now typed
    /// (`None`: not a valid server yet, leave it).
    pub fn retain_for(&mut self, imap: Option<&Server>, smtp: Option<&Server>) {
        for (slot, now) in self.slots.iter_mut().zip([imap, smtp]) {
            let Some(now) = now else { continue };
            if slot.pinned.as_ref().is_some_and(|p| p.server != *now) {
                slot.pinned = None;
            }
            if slot.damaged.as_ref().is_some_and(|s| s != now) {
                slot.damaged = None;
            }
            if slot.before.as_ref().is_some_and(|(_, s)| s != now) {
                slot.before = None;
            }
        }
    }

    /// The certificates to connect and save with.
    pub fn blobs(&self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let der = |i: usize| self.slots[i].pinned.as_ref().map(|p| p.der.clone());
        (der(0), der(1))
    }

    pub fn set(&mut self, kind: Kind, der: Vec<u8>, server: Server) {
        let slot = &mut self.slots[index(kind)];
        slot.pinned = Some(Pinned { der, server });
        slot.damaged = None;
    }

    /// Forget what is saved for `kind`, remembering its fingerprint for the
    /// next review.
    pub fn forget(&mut self, kind: Kind) {
        let slot = &mut self.slots[index(kind)];
        if let Some(p) = slot.pinned.take() {
            slot.before = Some((pin::fingerprint_sha256(shown(&p.der)), p.server));
        }
        slot.damaged = None;
    }

    /// The fingerprint of the certificate just forgotten.
    pub fn before(&self, kind: Kind) -> Option<&str> {
        self.slots[index(kind)]
            .before
            .as_ref()
            .map(|(fp, _)| fp.as_str())
    }

    /// Whether something is saved that "Forget certificate" removes.
    pub fn is_saved(&self, kind: Kind) -> bool {
        let slot = &self.slots[index(kind)];
        slot.pinned.is_some() || slot.damaged.is_some()
    }

    /// The line under a server in the form.
    pub fn line(&self, kind: Kind) -> String {
        let slot = &self.slots[index(kind)];
        match (&slot.pinned, &slot.damaged) {
            (Some(p), _) => match pin::usable(Some(&p.der)).map(|p| p.trust) {
                Some(Trust::Key) => {
                    text::cert_pinned_key(&pin::key_fingerprint_short(shown(&p.der)))
                }
                Some(Trust::Ca) => text::cert_pinned_ca(
                    &pin::describe(shown(&p.der))
                        .map(|i| i.subject)
                        .unwrap_or_default(),
                ),
                _ => text::cert_pinned(&pin::fingerprint_short(shown(&p.der))),
            },
            (None, Some(s)) => text::cert_damaged(&s.host),
            (None, None) => text::cert_windows(),
        }
    }
}

/// A certificate read from a server, waiting for the user's answer.
#[derive(Clone, Debug)]
pub struct CertReview {
    pub kind: Kind,
    pub server: Server,
    pub der: Vec<u8>,
    /// The certificate of the CA that issued `der`, when the server sent it.
    pub issuer: Option<Vec<u8>>,
}

/// One choice of the trust prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustChoice {
    pub label: String,
    pub description: String,
}

impl CertReview {
    /// What the prompt offers to trust, in its order.
    pub fn choices(&self) -> Vec<Trust> {
        let mut out = vec![Trust::Certificate, Trust::Key];
        if self.issuer.is_some() {
            out.push(Trust::Ca);
        }
        out
    }

    /// The prompt's choices with their words.
    pub fn choice_items(&self) -> Vec<TrustChoice> {
        self.choices()
            .into_iter()
            .map(|trust| match trust {
                Trust::Certificate => TrustChoice {
                    label: text::trust_cert(),
                    description: text::trust_cert_note(),
                },
                Trust::Key => TrustChoice {
                    label: text::trust_key(),
                    description: text::trust_key_note(&pin::key_fingerprint_sha256(&self.der)),
                },
                Trust::Ca => TrustChoice {
                    label: text::trust_ca(
                        &self
                            .issuer
                            .as_deref()
                            .and_then(pin::describe)
                            .map(|i| i.subject)
                            .unwrap_or_default(),
                    ),
                    description: text::trust_ca_note(
                        &self
                            .issuer
                            .as_deref()
                            .map(pin::fingerprint_sha256)
                            .unwrap_or_default(),
                    ),
                },
            })
            .collect()
    }

    /// The bytes to save for the choice `index` of [`Self::choices`].
    pub fn blob(&self, index: usize) -> Vec<u8> {
        let trust = self
            .choices()
            .get(index)
            .copied()
            .unwrap_or(Trust::Certificate);
        match (trust, &self.issuer) {
            (Trust::Ca, Some(ca)) => pin::encode(Trust::Ca, ca),
            (Trust::Key, _) => pin::encode(Trust::Key, &self.der),
            _ => self.der.clone(),
        }
    }
}

/// Which server's certificate changed: the sending server's when it is
/// named and the incoming one has another name, else the incoming one.
pub fn changed_kind(account: &Account, host: Option<&str>) -> Kind {
    match host {
        Some(h) if h == account.smtp.host && h != account.imap.host => Kind::Smtp,
        _ => Kind::Imap,
    }
}

/// The server a failed check lets the user review the certificate of: an
/// untrusted certificate, not a changed one (that is forgotten first) and
/// not a server without STARTTLS.
pub fn review_of(e: &StepError) -> Option<Kind> {
    match (e.step, e.class, e.special) {
        (Step::Secure, Class::TlsCert, Special::None) => Some(Kind::Imap),
        (Step::SmtpConnect, Class::TlsCert, Special::None) => Some(Kind::Smtp),
        _ => None,
    }
}

/// Why the certificate could not be read (what the form says of it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// The server won't do STARTTLS.
    NoStartTls { host: String },
    /// It could not be read; `address` is `host:port`.
    Unread { address: String },
}

pub fn unreadable(server: &Server, e: &anyhow::Error) -> Unreadable {
    let refused = e
        .chain()
        .any(|c| c.is::<komorebi::mail::imap::StartTlsRefused>());
    if refused {
        return Unreadable::NoStartTls {
            host: server.host.clone(),
        };
    }
    Unreadable::Unread {
        address: address(server),
    }
}

/// The prompt for a certificate: title, text, button, details, fingerprint
/// and the fingerprint it replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustPrompt {
    pub title: String,
    pub text: String,
    pub ok: String,
    pub detail: String,
    /// The fingerprint in fixed byte groups ([`group_fingerprint`]) and the
    /// line above it.
    pub code: String,
    pub code_label: String,
    /// The fingerprint it replaces, grouped the same way, its line and the
    /// caution under both (all empty when none is replaced).
    pub old: String,
    pub old_label: String,
    pub caution: String,
    /// How to check it ("How do I check this?").
    pub note: String,
    /// The fingerprint alone, for Copy.
    pub fingerprint: String,
}

/// Bytes per line of a fingerprint in the prompt: a SHA-256 (32 bytes)
/// makes 4 lines of 8.
const BYTES_PER_LINE: usize = 8;

/// A `AA:BB:...` fingerprint in lines of [`BYTES_PER_LINE`] bytes, so a
/// line never ends inside a byte or on a colon and the bytes line up in
/// columns for comparing them.
pub fn group_fingerprint(fingerprint: &str) -> String {
    let bytes: Vec<&str> = fingerprint.split(':').filter(|b| !b.is_empty()).collect();
    bytes
        .chunks(BYTES_PER_LINE)
        .map(|line| line.join(":"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The prompt's words for `review`; `before`: the fingerprint of the
/// certificate it replaces.
pub fn trust_prompt(review: &CertReview, before: Option<&str>) -> TrustPrompt {
    let host = &review.server.host;
    let mut lines = Vec::new();
    if let Some(info) = pin::describe(&review.der) {
        if !info.names.is_empty() {
            lines.push(text::cert_names(&info.names.join(", ")));
        } else if !info.subject.is_empty() {
            lines.push(text::cert_subject(&info.subject));
        }
        if info.self_signed {
            lines.push(text::cert_self_signed());
        } else if !info.issuer.is_empty() {
            lines.push(text::cert_issued_by(&info.issuer));
        }
        lines.push(text::cert_valid(&info.not_before, &info.not_after));
    }
    let fingerprint = pin::fingerprint_sha256(&review.der);
    TrustPrompt {
        title: text::q_trust_title(),
        text: text::q_trust_text(&address(&review.server), &host_for_display(host)),
        ok: text::q_trust_ok(),
        detail: lines.join("\n"),
        code: group_fingerprint(&fingerprint),
        code_label: match before {
            Some(_) => text::cert_fp_new(),
            None => text::cert_fp_label(),
        },
        old: before.map(group_fingerprint).unwrap_or_default(),
        old_label: before.map(|_| text::cert_fp_old()).unwrap_or_default(),
        caution: before.map(|_| text::cert_fp_caution()).unwrap_or_default(),
        note: text::q_trust_note(),
        fingerprint,
    }
}

#[cfg(test)]
mod tests {
    use komorebi::providers::Security;

    use super::super::hosts::typed_server;
    use super::*;

    fn server(host: &str, port: u16, security: Security) -> Server {
        Server {
            host: host.into(),
            port,
            security,
        }
    }

    fn der() -> Vec<u8> {
        native_tls::Certificate::from_pem(include_bytes!(
            "../../../../../tests/fixtures/tls/test-cert.pem"
        ))
        .unwrap()
        .to_der()
        .unwrap()
    }

    fn account(imap_pin: Option<Vec<u8>>) -> Account {
        Account {
            id: 7,
            email: "me@example.org".into(),
            name: String::new(),
            imap: server("mail.example.org", 993, Security::Tls),
            smtp: server("mail.example.org", 587, Security::StartTls),
            save_sent: true,
            signature: String::new(),
            enabled: true,
            imap_user: String::new(),
            smtp_user: String::new(),
            imap_pin,
            smtp_pin: None,
            smtp_no_auth: false,
        }
    }

    /// Ports `saved_pins_seed_the_form` (src/app/cert.rs).
    #[test]
    fn saved_pins_seed_the_form() {
        let pins = FormPins::for_account(&account(Some(der())));
        assert_eq!(pins.blobs(), (Some(der()), None));
        assert!(pins.is_saved(Kind::Imap) && !pins.is_saved(Kind::Smtp));
        assert!(
            pins.line(Kind::Imap)
                .starts_with("Certificate: pinned (SHA-256 F7:AE:3F:F2")
        );
        assert_eq!(pins.line(Kind::Smtp), "Certificate: checked by Windows");
    }

    /// Ports `a_damaged_pin_is_shown_and_trusted_for_nothing`.
    #[test]
    fn a_damaged_pin_is_shown_and_trusted_for_nothing() {
        let pins = FormPins::for_account(&account(Some(vec![1, 2, 3])));
        assert_eq!(pins.blobs(), (None, None));
        assert!(pins.is_saved(Kind::Imap));
        assert!(
            pins.line(Kind::Imap).contains("damaged"),
            "{}",
            pins.line(Kind::Imap)
        );
    }

    /// Ports `a_pin_goes_when_host_port_or_security_change`.
    #[test]
    fn a_pin_goes_when_host_port_or_security_change() {
        let a = account(Some(der()));
        let (imap, smtp) = (a.imap.clone(), a.smtp.clone());
        let changed = [
            server("other.example.org", 993, Security::Tls),
            server("mail.example.org", 994, Security::Tls),
            server("mail.example.org", 993, Security::StartTls),
        ];
        for new in changed {
            let mut pins = FormPins::for_account(&a);
            pins.retain_for(Some(&new), Some(&smtp));
            assert_eq!(pins.blobs(), (None, None), "{new:?}");
            assert_eq!(pins.line(Kind::Imap), "Certificate: checked by Windows");
        }
        // The same servers (a login or password edit) keep it.
        let mut pins = FormPins::for_account(&a);
        pins.retain_for(Some(&imap), Some(&smtp));
        assert_eq!(pins.blobs().0, Some(der()));
        // A half-typed server is left alone until it is valid.
        pins.retain_for(None, None);
        assert_eq!(pins.blobs().0, Some(der()));
        // The SMTP pin is bound to the SMTP server only.
        pins.set(Kind::Smtp, der(), smtp.clone());
        pins.retain_for(
            Some(&imap),
            Some(&server("mail.example.org", 465, Security::Tls)),
        );
        assert_eq!(pins.blobs(), (Some(der()), None));
    }

    /// Ports `forgetting_remembers_the_old_fingerprint_for_the_next_review`.
    #[test]
    fn forgetting_remembers_the_old_fingerprint_for_the_next_review() {
        let mut pins = FormPins::for_account(&account(Some(der())));
        pins.forget(Kind::Imap);
        assert_eq!(pins.blobs(), (None, None));
        assert_eq!(
            pins.before(Kind::Imap),
            Some(pin::fingerprint_sha256(&der()).as_str())
        );
        let review = CertReview {
            kind: Kind::Imap,
            server: server("mail.example.org", 993, Security::Tls),
            der: der(),
            issuer: None,
        };
        let prompt = trust_prompt(&review, pins.before(Kind::Imap));
        assert_eq!(
            prompt.old,
            group_fingerprint(&pin::fingerprint_sha256(&der()))
        );
        assert_eq!(prompt.old_label, "Saved before (SHA-256)");
        assert_eq!(prompt.code_label, "New certificate (SHA-256)");
        assert!(!prompt.caution.is_empty());
        // Pinning again clears the damaged state and keeps the server.
        pins.set(Kind::Imap, der(), review.server.clone());
        assert!(pins.is_saved(Kind::Imap));
    }

    /// Ports `the_prompt_names_the_server_and_shows_the_whole_fingerprint`.
    #[test]
    fn the_prompt_names_the_server_and_shows_the_whole_fingerprint() {
        let review = CertReview {
            kind: Kind::Imap,
            server: server("mail.example.org", 993, Security::Tls),
            der: der(),
            issuer: None,
        };
        let p = trust_prompt(&review, None);
        assert_eq!(p.title, "Trust this certificate?");
        assert_eq!(
            p.text,
            "mail.example.org:993 presented a certificate that Windows doesn't trust. Trust this certificate for mail.example.org only?"
        );
        assert_eq!(p.ok, "Trust this certificate");
        assert_eq!(p.fingerprint.len(), 95);
        assert!(!p.fingerprint.contains('…') && !p.fingerprint.contains("..."));
        assert!(
            p.detail.contains("Names: 127.0.0.1, localhost"),
            "{}",
            p.detail
        );
        assert!(
            p.detail.contains("Self-signed") && p.detail.contains("Valid from "),
            "{}",
            p.detail
        );
        assert!(p.old.is_empty() && p.old_label.is_empty() && p.caution.is_empty());
        assert_eq!(p.code, group_fingerprint(&p.fingerprint));
        assert_eq!(p.code_label, "SHA-256 fingerprint");
        // An unreadable certificate still shows its fingerprint.
        assert_eq!(review.choices(), vec![Trust::Certificate, Trust::Key]);
        assert_eq!(review.blob(0), der());
        assert_eq!(review.blob(1), pin::encode(Trust::Key, &der()));
        assert_eq!(review.blob(7), der());
        let broken = CertReview {
            der: vec![0x30, 0x03, 1, 2, 3],
            ..review
        };
        let p = trust_prompt(&broken, None);
        assert_eq!(p.detail, "");
        assert_eq!(p.fingerprint.len(), 95);
    }

    /// Ports `a_fingerprint_is_shown_in_whole_bytes_eight_a_line`.
    #[test]
    fn a_fingerprint_is_shown_in_whole_bytes_eight_a_line() {
        let fp = pin::fingerprint_sha256(&der());
        let grouped = group_fingerprint(&fp);
        let lines: Vec<&str> = grouped.lines().collect();
        assert_eq!(lines.len(), 4, "{grouped}");
        for line in &lines {
            // 8 bytes, never cut inside one, no colon at either end.
            assert_eq!(line.len(), 8 * 2 + 7, "{line}");
            assert!(!line.starts_with(':') && !line.ends_with(':'));
            assert!(line.split(':').all(|b| b.len() == 2));
        }
        // Nothing is lost or reordered: joined again it is the fingerprint.
        assert_eq!(lines.join(":"), fp);
        // A shorter one ends with a short line; an empty one stays empty.
        assert_eq!(group_fingerprint("AA:BB:CC"), "AA:BB:CC");
        assert_eq!(
            group_fingerprint("01:02:03:04:05:06:07:08:09"),
            "01:02:03:04:05:06:07:08\n09"
        );
        assert_eq!(group_fingerprint(""), "");
    }

    /// Ports `a_changed_certificate_is_the_server_it_names`.
    #[test]
    fn a_changed_certificate_is_the_server_it_names() {
        let mut a = account(None);
        a.smtp = server("smtp.example.org", 465, Security::Tls);
        assert_eq!(changed_kind(&a, Some("smtp.example.org")), Kind::Smtp);
        assert_eq!(changed_kind(&a, Some(&a.imap.host.clone())), Kind::Imap);
        assert_eq!(changed_kind(&a, None), Kind::Imap);
        // One host for both: the incoming server is reviewed first.
        a.smtp.host = a.imap.host.clone();
        assert_eq!(changed_kind(&a, Some(&a.imap.host.clone())), Kind::Imap);
    }

    /// Ports `only_an_untrusted_certificate_can_be_reviewed`.
    #[test]
    fn only_an_untrusted_certificate_can_be_reviewed() {
        let e = |step, class, special| StepError {
            step,
            class,
            special,
            host: "h".into(),
            port: 1,
            login: String::new(),
            source: anyhow::anyhow!("x"),
        };
        assert_eq!(
            review_of(&e(Step::Secure, Class::TlsCert, Special::None)),
            Some(Kind::Imap)
        );
        assert_eq!(
            review_of(&e(Step::SmtpConnect, Class::TlsCert, Special::None)),
            Some(Kind::Smtp)
        );
        for special in [
            Special::CertChanged,
            Special::NoStartTls,
            Special::PlainAnswer,
        ] {
            assert_eq!(review_of(&e(Step::Secure, Class::TlsCert, special)), None);
        }
        assert_eq!(
            review_of(&e(Step::Secure, Class::Offline, Special::None)),
            None
        );
        assert_eq!(
            review_of(&e(Step::ImapSignIn, Class::Auth, Special::None)),
            None
        );
    }

    /// Ports `typed_servers_are_checked_like_the_form_does`.
    #[test]
    fn typed_servers_are_checked_like_the_form_does() {
        assert_eq!(
            typed_server(" Mail.Example.org. ", "993", false),
            Some(server("mail.example.org", 993, Security::Tls))
        );
        assert_eq!(typed_server("", "993", false), None);
        assert_eq!(typed_server("mail.example.org", "0", true), None);
        assert_eq!(typed_server("mail.example.org", "x", true), None);
        assert_eq!(typed_server("https://mail.example.org", "993", true), None);
    }
}
