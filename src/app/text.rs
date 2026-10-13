//! Which sentence a typed value means. The library and the validators hand
//! over values (an error class with its server and port, a failed field of
//! the account form, how a send ended); every `match` from such a value to
//! one of the `Tr` sentences (`ui/tr.slint`, in the language shown) lives
//! here, on the UI thread, so the language can change and the text follows.
//! What stays English on purpose: the failure's own words (`Problem::detail`,
//! the server's replies), logs, codes and protocol text.

use super::*;
use crate::mail::errors::Say;
use crate::mail::{Notice, Reason};
use crate::tr::t;

/// A server's name as the UI shows it (an international name in its own
/// letters), as a sentence argument.
pub(super) fn host(name: &str) -> SharedString {
    settings::host_for_display(name).into()
}

/// A sentence for a failure, in the language shown.
pub(super) fn say(s: &Say) -> String {
    let h = |host: &String| SharedString::from(host.as_str());
    let p = |port: &u16| SharedString::from(port.to_string());
    match s {
        Say::Class(class) => match class {
            Class::Offline => t!(invoke_say_offline),
            Class::Auth => t!(invoke_say_auth),
            Class::TlsCert => t!(invoke_say_tls),
            Class::TooLarge => t!(invoke_say_too_large),
            Class::Quota => t!(invoke_say_quota),
            Class::TooManyConnections => t!(invoke_say_busy),
            // Said in the error's own words (`Problem::detail`).
            Class::Other => String::new(),
        },
        Say::NotFound { host } => t!(invoke_say_not_found, h(host)),
        Say::NoAnswer { host, port } => t!(invoke_say_no_answer, h(host), p(port)),
        Say::Lost { host } => t!(invoke_say_lost, h(host)),
        Say::Rejected { host, login } => t!(invoke_say_rejected, h(host), h(login)),
        Say::SecureFailed { host } => t!(invoke_say_secure_failed, h(host)),
        Say::NoConnection { host, port } => t!(invoke_say_no_connection, h(host), p(port)),
        Say::NoSecure { host } => t!(invoke_say_no_secure, h(host)),
        Say::NoSignIn { host, login } => t!(invoke_say_no_sign_in, h(host), h(login)),
        Say::NoFolders { host } => t!(invoke_say_no_folders, h(host)),
        Say::NoStartTls { host } => t!(invoke_say_no_starttls, h(host)),
        Say::PlainAnswer => t!(invoke_say_plain_answer),
        Say::NoAuth { host } => t!(invoke_say_no_auth, h(host)),
        Say::BadGreeting { host } => t!(invoke_say_bad_greeting, h(host)),
        Say::NoLogin { host } => t!(invoke_say_no_login, h(host)),
        Say::Utf8Only { host } => t!(invoke_say_utf8_only, h(host)),
        Say::CertChanged { host } => t!(invoke_say_cert_changed, h(host)),
        Say::SenderRefused { host, from } => t!(invoke_say_sender_refused, h(host), h(from)),
        Say::SignInLatched => t!(invoke_say_sign_in_latched),
        Say::Unconfirmed { detail } => t!(invoke_say_unconfirmed, h(detail)),
        Say::TooLargeSend { detail } => t!(invoke_say_too_large_send, h(detail)),
    }
}

/// A rejected password in the account form, by provider (`preset`: the
/// index of the one chosen; `None`: Other server, or an edited account): a
/// preset gets the app-password advice only, and, for an address on
/// another domain than its own, a pointer to the other preset; any other
/// server also the username advice.
pub(super) fn rejected(preset: Option<usize>, email: &str, host: &str, login: &str) -> String {
    let (host, login) = (SharedString::from(host), SharedString::from(login));
    let Some(index) = preset.filter(|&i| i < providers::PRESETS.len()) else {
        return t!(invoke_say_rejected, host, login);
    };
    let advice = t!(invoke_say_rejected_preset, host, login);
    if providers::on_own_domain(index, email) {
        return advice;
    }
    match providers::PRESETS
        .iter()
        .enumerate()
        .find(|(i, _)| *i != index)
    {
        Some((_, other)) => format!(
            "{advice} {}",
            t!(invoke_say_other_preset, other.name.into())
        ),
        None => advice,
    }
}

/// What the status line says of a problem: its sentence, or for an error with
/// none of its own, the error's words.
pub(super) fn problem(p: &Problem) -> String {
    p.say().map_or_else(|| p.detail.clone(), |s| say(&s))
}

/// How a failed send reads: its sentence, or the failure's own words.
pub(super) fn reason(r: &Reason) -> String {
    r.say.as_ref().map_or_else(|| r.text.clone(), say)
}

/// A caveat of an action that worked.
pub(super) fn notice(n: &Notice) -> String {
    match n {
        Notice::LeftFlagged => t!(invoke_notice_left_flagged),
        Notice::MoveGone => t!(invoke_notice_move_gone),
        Notice::NoDraftsFolder => t!(invoke_notice_no_drafts_folder),
        Notice::NoSentFolder => t!(invoke_notice_no_sent_folder),
        Notice::NoDraftUid => t!(invoke_notice_no_draft_uid),
        Notice::DraftsNotSaved(detail) => {
            t!(invoke_notice_drafts_not_saved, detail.as_str().into())
        }
    }
}

/// A failure of the cache or the system, behind a lead-in sentence: the
/// error chain as they wrote it (English, or the system's language) stays
/// as the detail. Every such passthrough goes through here.
pub(super) fn detail(e: &anyhow::Error) -> String {
    t!(invoke_error_detail, format!("{e:#}").into())
}

/// A line about one account: its address, then `text`.
pub(super) fn account_line(email: &str, text: &str) -> String {
    t!(invoke_account_line, email.into(), text.into())
}

/// How a send ended, for the status line (`None`: nothing to say, or said
/// by the caller).
pub(super) fn outcome(o: &Outcome) -> Option<String> {
    Some(match o {
        Outcome::Delivered => t!(invoke_sent),
        Outcome::DeliveredNotSaved(detail) => t!(invoke_sent_not_saved, detail.as_str().into()),
        Outcome::Retry(r) => t!(invoke_not_sent_yet, reason(r).into()),
        Outcome::Failed(r) => t!(invoke_not_sent, reason(r).into()),
        Outcome::Unconfirmed(r) => reason(r),
    })
}

/// A field of the account form that can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Proto {
    Imap,
    Smtp,
}

impl Proto {
    fn name(self) -> &'static str {
        match self {
            Proto::Imap => "IMAP",
            Proto::Smtp => "SMTP",
        }
    }
}

/// Why the account form can't be used as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FormError {
    /// The email address has no `@`.
    Email,
    /// No provider was chosen (a new account).
    Provider,
    /// A server name is missing.
    ServerNames,
    /// A server name holds more than the name.
    ServerNameOnly,
    /// An IPv6 zone identifier (`%eth0`).
    ZoneId,
    /// A non-ASCII server name that can't be written in `xn--` form.
    AsciiName,
    Port(Proto),
    Username(Proto),
}

pub(super) fn form_error(e: FormError) -> String {
    match e {
        FormError::Email => t!(invoke_err_email),
        FormError::Provider => t!(invoke_err_provider),
        FormError::ServerNames => t!(invoke_err_server_names),
        FormError::ServerNameOnly => t!(invoke_err_server_name_only),
        FormError::ZoneId => t!(invoke_err_zone_id),
        FormError::AsciiName => t!(invoke_err_ascii_name),
        FormError::Port(p) => t!(invoke_err_port, p.name().into()),
        FormError::Username(p) => t!(invoke_err_username, p.name().into()),
    }
}

/// Why Send is off, under the toolbar.
pub(super) fn send_block(b: composer::SendBlock) -> String {
    use composer::SendBlock;
    match b {
        SendBlock::FixAddress(ComposeField::To) => t!(invoke_fix_address_to),
        SendBlock::FixAddress(ComposeField::Cc) => t!(invoke_fix_address_cc),
        SendBlock::FixAddress(_) => t!(invoke_fix_address_bcc),
        SendBlock::NoRecipient => t!(invoke_add_recipient),
        SendBlock::AttachmentsOver => t!(invoke_attachments_over),
    }
}

/// What the send guards' bar asks.
pub(super) fn guard(g: composer::Guard) -> String {
    use composer::Guard;
    match g {
        Guard::Both => t!(invoke_guard_both),
        Guard::NoSubject => t!(invoke_guard_no_subject),
        Guard::NoAttachment => t!(invoke_guard_no_attachment),
    }
}

/// The inline error under an address field: `address` is the one at fault.
pub(super) fn address_invalid(address: &str) -> String {
    t!(invoke_address_invalid, address.into())
}

/// One file that could not be attached.
pub(super) fn attach_fail(f: &composer::AttachFail) -> String {
    use composer::AttachWhy;
    match &f.why {
        AttachWhy::Folder => t!(invoke_attach_folder, f.name.as_str().into()),
        AttachWhy::TooBig => t!(invoke_attach_too_big, f.name.as_str().into()),
        AttachWhy::Io(e) => t!(
            invoke_attach_failed,
            f.name.as_str().into(),
            e.as_str().into()
        ),
    }
}

/// The notice under a message from a sender worth a second look: the
/// reasons (at most two) in one sentence; empty without any.
pub(super) fn trust_notice(reasons: &[trust::Reason]) -> String {
    use trust::Reason as R;
    let one = |r: &R| -> SharedString {
        match r {
            R::Dmarc => t!(invoke_trust_dmarc),
            R::SpfDkim => t!(invoke_trust_spf_dkim),
            R::NameDomain { shown, real } => {
                t!(
                    invoke_trust_name_domain,
                    shown.as_str().into(),
                    real.as_str().into()
                )
            }
            R::Punycode { ascii, unicode } => {
                t!(
                    invoke_trust_punycode,
                    ascii.as_str().into(),
                    unicode.as_str().into()
                )
            }
            R::MixedScript { domain, script } => {
                let name: String = match *script {
                    "Cyrillic" => t!(invoke_script_cyrillic),
                    "Greek" => t!(invoke_script_greek),
                    other => other.to_string(),
                };
                t!(
                    invoke_trust_mixed_script,
                    domain.as_str().into(),
                    name.as_str().into()
                )
            }
        }
        .into()
    };
    match reasons {
        [] => String::new(),
        [a] => t!(invoke_trust_notice_one, one(a)),
        [a, b, ..] => t!(invoke_trust_notice_two, one(a), one(b)),
    }
}

/// The name a folder is shown by: the server's own, or in a language other
/// than English the name of its role (the server's spelling is English
/// mostly, and the same folder reads differently on every server).
pub(super) fn folder_label(lang: komorebi::i18n::Lang, f: &Folder) -> String {
    if lang == komorebi::i18n::Lang::En {
        return f.display.clone();
    }
    match f.role {
        Role::Inbox => t!(invoke_role_inbox),
        Role::Drafts => t!(invoke_role_drafts),
        Role::Sent => t!(invoke_role_sent),
        Role::Archive => t!(invoke_role_archive),
        Role::Junk => t!(invoke_role_junk),
        Role::Trash => t!(invoke_role_trash),
        Role::Other => f.display.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::text_calls::CALLS;
    use super::*;
    use crate::Tr;
    use crate::tr::tests::window;
    use komorebi::i18n::{Lang, select};
    use std::collections::BTreeSet;

    #[test]
    fn a_rejected_password_reads_by_provider() {
        let _ui = window();
        let (host, login) = ("mail.example.com", "ana");
        let other = rejected(None, "ana@example.com", host, login);
        assert_eq!(
            other,
            "mail.example.com rejected the password for ana. If sign-in uses two steps, create an \
             app password. If the login isn't your email address, set the username."
        );
        // A preset on its own domain: the app-password advice only.
        let own = rejected(Some(0), "ana@purelymail.com", host, login);
        assert_eq!(
            own,
            "mail.example.com rejected the password for ana. If sign-in uses two steps, create an \
             app password."
        );
        // On a custom domain: maybe the other preset hosts it.
        let custom = rejected(Some(0), "ana@example.com", host, login);
        assert!(
            custom.ends_with(" Is your mail at Postale.io? Change Provider."),
            "{custom}"
        );
        let custom = rejected(Some(1), "ana@example.com", host, login);
        assert!(
            custom.ends_with(" Is your mail at Purelymail? Change Provider."),
            "{custom}"
        );
        // Portuguese, whole sentences.
        assert!(select(Lang::PtPt));
        let pt = rejected(Some(1), "ana@example.com", host, login);
        assert!(
            pt.ends_with("O seu correio está na Purelymail? Altere o Fornecedor."),
            "{pt}"
        );
        assert!(select(Lang::En));
    }

    /// The functions of `ui/tr.slint` with the number of string parameters.
    fn declared() -> Vec<(String, usize)> {
        let src = include_str!("../../ui/tr.slint");
        src.split("pure public function ")
            .skip(1)
            .map(|rest| {
                let name = rest.split('(').next().unwrap_or_default().to_string();
                let params = rest.split(')').next().unwrap_or_default();
                let strings = params.matches(": string").count();
                (name, strings)
            })
            .collect()
    }

    #[test]
    fn the_calls_list_every_sentence_of_the_catalog_file() {
        let declared: BTreeSet<String> = declared().into_iter().map(|(n, _)| n).collect();
        let listed: BTreeSet<String> = CALLS.iter().map(|(n, _)| n.to_string()).collect();
        let missing: Vec<_> = declared.difference(&listed).collect();
        let extra: Vec<_> = listed.difference(&declared).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "ui/tr.slint and src/app/text_calls.rs differ: missing {missing:?}, extra {extra:?}"
        );
        assert_eq!(CALLS.len(), declared.len(), "a name listed twice");
    }

    #[test]
    fn every_sentence_renders_in_both_languages() {
        let ui = window();
        let strings: std::collections::HashMap<String, usize> = declared().into_iter().collect();
        for lang in [Lang::En, Lang::PtPt] {
            assert!(select(lang), "{lang:?} is bundled");
            let tr = ui.global::<Tr>();
            for (name, call) in CALLS {
                let text = call(&tr).to_string();
                assert!(!text.trim().is_empty(), "{lang:?} {name} is empty");
                assert!(
                    !text.contains(['{', '}']),
                    "{lang:?} {name} has an unfilled placeholder: {text}"
                );
                for i in 0..strings[*name] {
                    assert!(
                        text.contains(&format!("t{i}")),
                        "{lang:?} {name} lost its argument t{i}: {text}"
                    );
                }
            }
        }
        select(Lang::En);
    }

    #[test]
    fn the_language_reaches_the_sentences() {
        let ui = window();
        let title = || ui.global::<Tr>().invoke_window_title(3).to_string();
        assert_eq!(title(), "Inbox (3) - Komorebi");
        assert!(select(Lang::PtPt));
        assert_eq!(title(), "Caixa de entrada (3) - Komorebi");
        assert!(select(Lang::En));
        assert_eq!(title(), "Inbox (3) - Komorebi");
    }

    fn folder(role: Role, display: &str) -> Folder {
        Folder {
            id: 1,
            account_id: 1,
            name: "X".into(),
            display: display.into(),
            depth: 0,
            role,
            uidvalidity: 0,
        }
    }

    fn h() -> String {
        "imap.example.com".into()
    }

    #[test]
    fn failures_read_as_they_always_did_in_english() {
        let _ui = window();
        let said = |s: Say| say(&s);
        let cases: Vec<(Say, String)> = vec![
            (Say::Class(Class::Offline), "Can't reach the server. Komorebi tries again by itself; check your internet connection.".into()),
            (Say::Class(Class::Auth), "The server rejected the sign-in. Update the password in Settings, or press Refresh to retry.".into()),
            (Say::Class(Class::TlsCert), "Couldn't make a secure connection to the server. Check the server name in Settings; its certificate may not be trusted.".into()),
            (Say::Class(Class::TooLarge), "The server says this message is too large. Remove an attachment or share it another way.".into()),
            (Say::Class(Class::Quota), "The mailbox is full on the server. Delete or archive some mail, then try again.".into()),
            (Say::Class(Class::TooManyConnections), "The server is busy or has too many connections open. Komorebi tries again shortly; close other mail apps if this keeps happening.".into()),
            (Say::NotFound { host: h() }, "Couldn't find imap.example.com. Check the spelling of the server name and your internet connection.".into()),
            (Say::NoAnswer { host: h(), port: 993 }, "imap.example.com didn't answer on port 993. SSL/TLS usually uses 993 (IMAP) or 465 (SMTP); STARTTLS uses 143 or 587 (some servers use 25, 2525 or 1143).".into()),
            (Say::Lost { host: h() }, "The connection to imap.example.com was lost. Try again, and if it keeps happening, check the server.".into()),
            (Say::Rejected { host: h(), login: "me@example.com".into() }, "imap.example.com rejected the password for me@example.com. If sign-in uses two steps, create an app password. If the login isn't your email address, set the username.".into()),
            (Say::SecureFailed { host: h() }, "The secure connection to imap.example.com failed. Its certificate isn't trusted or doesn't match the server name. Check the name, or if you run this server yourself, review its certificate.".into()),
            (Say::NoConnection { host: h(), port: 1143 }, "imap.example.com didn't accept the connection on port 1143. Check the server name and port.".into()),
            (Say::NoSecure { host: h() }, "Couldn't set up a secure connection to imap.example.com. Check the port and the security setting.".into()),
            (Say::NoSignIn { host: h(), login: "alice".into() }, "imap.example.com wouldn't let alice sign in. Check the username, the password and the server's settings.".into()),
            (Say::NoFolders { host: h() }, "Signed in, but imap.example.com didn't list its folders. Try again, or check the server.".into()),
            (Say::NoStartTls { host: h() }, "imap.example.com doesn't offer STARTTLS, so Komorebi won't sign in (your password would travel unencrypted). Ask the administrator to enable it, or use the SSL/TLS port.".into()),
            (Say::PlainAnswer, "This port answered without encryption. Komorebi only connects securely; choose STARTTLS or the SSL/TLS port.".into()),
            (Say::NoAuth { host: h() }, "imap.example.com doesn't offer sign-in. Komorebi can't send through servers without sign-in yet.".into()),
            (Say::BadGreeting { host: h() }, "imap.example.com didn't greet Komorebi like an IMAP server (it said goodbye, or signed in by itself). Check the server name, the port and the security setting.".into()),
            (Say::NoLogin { host: h() }, "imap.example.com doesn't allow signing in with a password over this connection (LOGINDISABLED). Ask the administrator to allow it on secure connections.".into()),
            (Say::Utf8Only { host: h() }, "imap.example.com only works in UTF-8 mode (UTF8=ONLY), which Komorebi doesn't support yet. Ask the administrator to also allow the standard mode.".into()),
            (Say::CertChanged { host: h() }, "The certificate of imap.example.com changed. If you didn't expect that, don't sign in: review the new certificate and compare its fingerprint before you trust it.".into()),
            (Say::SenderRefused { host: h(), from: "me@example.com".into() }, "imap.example.com refused to send as me@example.com. Check that this account may send from that address, or set the SMTP username.".into()),
            (Say::SignInLatched, "Sign-in rejected. Update the password in Settings, or press Refresh to retry.".into()),
            (Say::Unconfirmed { detail: "sending failed".into() }, "sending failed. The message may or may not have been sent; check Sent before you retry it from the Outbox.".into()),
            (Say::TooLargeSend { detail: "552".into() }, "The server says this message is too large (552)".into()),
        ];
        for (s, english) in cases {
            assert_eq!(said(s.clone()), english, "{s:?}");
            // What the outbox row keeps is the same English line.
            if let Some(record) = s.record() {
                assert_eq!(record, english, "{s:?}");
            }
        }
    }

    #[test]
    fn typed_values_have_a_sentence_in_both_languages() {
        let ui = window();
        let all_says = [
            Say::Class(Class::Offline),
            Say::NotFound { host: "h".into() },
            Say::NoAnswer {
                host: "h".into(),
                port: 25,
            },
            Say::SignInLatched,
        ];
        for lang in [Lang::En, Lang::PtPt] {
            select(lang);
            for s in &all_says {
                assert!(!say(s).is_empty(), "{lang:?} {s:?}");
            }
            for n in [
                Notice::LeftFlagged,
                Notice::MoveGone,
                Notice::NoDraftsFolder,
                Notice::NoSentFolder,
                Notice::NoDraftUid,
                Notice::DraftsNotSaved("x".into()),
            ] {
                assert!(!notice(&n).is_empty(), "{lang:?} {n:?}");
            }
            for e in [
                FormError::Email,
                FormError::ServerNames,
                FormError::ServerNameOnly,
                FormError::ZoneId,
                FormError::AsciiName,
                FormError::Port(Proto::Imap),
                FormError::Port(Proto::Smtp),
                FormError::Username(Proto::Imap),
                FormError::Username(Proto::Smtp),
            ] {
                let text = form_error(e);
                assert!(!text.is_empty() && !text.contains('{'), "{lang:?} {e:?}");
            }
            assert!(trust_notice(&[]).is_empty());
            let name = |role| folder(role, "Server's X");
            for role in [
                Role::Inbox,
                Role::Drafts,
                Role::Sent,
                Role::Archive,
                Role::Junk,
                Role::Trash,
                Role::Other,
            ] {
                assert!(!folder_label(lang, &name(role)).is_empty());
            }
        }
        // English keeps the server's own spelling of a folder; the other
        // languages name the role.
        select(Lang::En);
        let sent = folder(Role::Sent, "Sent Items");
        assert_eq!(folder_label(Lang::En, &sent), "Sent Items");
        select(Lang::PtPt);
        assert_ne!(folder_label(Lang::PtPt, &sent), "Sent Items");
        select(Lang::En);
        drop(ui);
    }

    #[test]
    fn a_sender_notice_names_its_reasons_in_english() {
        use trust::Reason as R;
        let _ui = window();
        let reasons = [
            R::Dmarc,
            R::Punycode {
                ascii: "xn--pypal-4ve.com".into(),
                unicode: "pаypal.com".into(),
            },
        ];
        assert_eq!(
            trust_notice(&reasons),
            "Reasons: it failed your server's sender check (DMARC); xn--pypal-4ve.com is the international name \u{201c}pаypal.com\u{201d}."
        );
        assert_eq!(
            trust_notice(&[R::SpfDkim]),
            "Reason: it failed your server's sender checks (SPF and DKIM)."
        );
        assert_eq!(
            trust_notice(&[R::NameDomain {
                shown: "paypal.com".into(),
                real: "evil.example".into()
            }]),
            "Reason: the name shows paypal.com, but the address is at evil.example."
        );
        assert_eq!(
            trust_notice(&[R::MixedScript {
                domain: "x.com".into(),
                script: "Cyrillic"
            }]),
            "Reason: x.com mixes Latin and Cyrillic letters."
        );
    }

    #[test]
    fn a_problem_says_its_class_or_its_own_words() {
        let _ui = window();
        let offline = Problem::of(&anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::TimedOut,
        )));
        assert!(problem(&offline).starts_with("Can't reach the server."));
        let other = Problem::of(&anyhow::anyhow!("opening folder X"));
        assert_eq!(problem(&other), "opening folder X");
        let changed = Problem::of(&mail::pin::PinMismatch("imap.example.com".into()).into());
        assert!(problem(&changed).starts_with("The certificate of imap.example.com changed."));
    }

    #[test]
    fn a_send_reads_by_how_it_ended() {
        let _ui = window();
        let out = |o| outcome(&o).unwrap_or_default();
        assert_eq!(out(Outcome::Delivered), "Sent");
        assert_eq!(
            out(Outcome::DeliveredNotSaved("no Sent folder".into())),
            "Message sent, but no Sent folder"
        );
        assert_eq!(
            out(Outcome::Retry("busy".into())),
            "Not sent yet: busy. It stays in the Outbox and is tried again shortly."
        );
        assert_eq!(
            out(Outcome::Failed("550".into())),
            "Not sent: 550. It is in the Outbox."
        );
        assert_eq!(
            out(Outcome::Failed(Reason::said(Say::SenderRefused {
                host: "smtp.example.com".into(),
                from: "me@example.com".into()
            }))),
            "Not sent: smtp.example.com refused to send as me@example.com. Check that this account may send from that address, or set the SMTP username.. It is in the Outbox."
        );
        assert_eq!(
            out(Outcome::Unconfirmed(Reason::said(Say::Unconfirmed {
                detail: "connection lost".into()
            }))),
            "connection lost. The message may or may not have been sent; check Sent before you retry it from the Outbox."
        );
    }
}
