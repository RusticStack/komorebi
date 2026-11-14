//! The sentences the Rust side shows: the GPUI counterpart of the Slint
//! app's `ui/tr.slint` (the `Tr` global), `src/tr.rs` (`t!`) and
//! `src/app/text.rs` (which sentence a typed value means). Each sentence is
//! one function with `tr!`/`trn!` inside, whole, with its names and numbers
//! as arguments; the text is the same msgid (and context) as in
//! `ui/tr.slint`, so both apps share one catalog entry.
//!
//! Views keep typed values (a [`Say`], a count) in their state, never the
//! sentence, and call these functions in `render`: a language switch
//! refreshes every window (`crate::i18n::set_language`), so the text
//! follows. The library and the workers produce no sentences.
//!
//! Phase 15 ports a representative set (docs/GPUI-I18N.md); each view phase
//! moves the sentences it needs from `ui/tr.slint` into this module (or a
//! view's own module) the same way, copying the English text exactly.

// The view phases call these; until then only the tests and the window do.
#![allow(dead_code)]

use komorebi::mail::errors::{Class, Say};
use komorebi_i18n::{tr, trn};

/// A server's name as sentences show it (an IPv6 address in brackets),
/// as the Slint app's `settings::host_for_display`.
pub fn host(name: &str) -> String {
    if name.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{name}]")
    } else {
        name.to_string()
    }
}

/// The window title: the unread count of the inboxes, if any (`Tr.window-title`).
pub fn window_title(unread: usize) -> String {
    if unread > 0 {
        tr!("window title" => "Inbox ({0}) - Komorebi", unread)
    } else {
        "Komorebi".to_string()
    }
}

/// The start-up failure dialog (`Tr.start-failed`).
pub fn start_failed(detail: &str) -> String {
    tr!("Komorebi couldn't start.\n\n{0}", detail)
}

/// The tray menu's first item (`Tr.tray-open`).
pub fn tray_open() -> String {
    tr!("tray menu" => "Open Komorebi")
}

/// The tray icon's tooltip (`Tr.tray-tooltip`).
pub fn tray_tooltip(unread: usize) -> String {
    if unread > 0 {
        trn!(
            "Komorebi: {n} unread message",
            "Komorebi: {n} unread messages",
            unread
        )
    } else {
        "Komorebi".to_string()
    }
}

/// The title of a toast for several new messages (`Tr.toast-burst`).
pub fn toast_burst(n: usize) -> String {
    trn!("{n} new message", "{n} new messages", n)
}

/// The status of a message that went out (`Tr.sent`; the context keeps it
/// apart from the Sent folder's name).
pub fn sent() -> String {
    tr!("status" => "Sent")
}

/// A sentence for a failure, in the language shown (`src/app/text.rs`
/// `say`). Empty for [`Class::Other`], which is said in the error's own
/// words.
pub fn say(s: &Say) -> String {
    match s {
        Say::Class(class) => match class {
            Class::Offline => tr!(
                "Can't reach the server. Komorebi tries again by itself; check your internet connection."
            ),
            Class::Auth => tr!(
                "The server rejected the sign-in. Update the password in Settings, or press Refresh to retry."
            ),
            Class::TlsCert => tr!(
                "Couldn't make a secure connection to the server. Check the server name in Settings; its certificate may not be trusted."
            ),
            Class::TooLarge => tr!(
                "The server says this message is too large. Remove an attachment or share it another way."
            ),
            Class::Quota => tr!(
                "The mailbox is full on the server. Delete or archive some mail, then try again."
            ),
            Class::TooManyConnections => tr!(
                "The server is busy or has too many connections open. Komorebi tries again shortly; close other mail apps if this keeps happening."
            ),
            Class::Other => String::new(),
        },
        Say::NotFound { host: h } => tr!(
            "Couldn't find {0}. Check the spelling of the server name and your internet connection.",
            host(h)
        ),
        Say::NoAnswer { host: h, port } => tr!(
            "{0} didn't answer on port {1}. SSL/TLS usually uses 993 (IMAP) or 465 (SMTP); STARTTLS uses 143 or 587 (some servers use 25, 2525 or 1143).",
            host(h),
            port
        ),
        Say::Lost { host: h } => tr!(
            "The connection to {0} was lost. Try again, and if it keeps happening, check the server.",
            host(h)
        ),
        Say::Rejected { host: h, login } => tr!(
            "{0} rejected the password for {1}. If sign-in uses two steps, create an app password. If the login isn't your email address, set the username.",
            host(h),
            login
        ),
        Say::SecureFailed { host: h } => tr!(
            "The secure connection to {0} failed. Its certificate isn't trusted or doesn't match the server name. Check the name, or if you run this server yourself, review its certificate.",
            host(h)
        ),
        Say::NoConnection { host: h, port } => tr!(
            "{0} didn't accept the connection on port {1}. Check the server name and port.",
            host(h),
            port
        ),
        Say::NoSecure { host: h } => tr!(
            "Couldn't set up a secure connection to {0}. Check the port and the security setting.",
            host(h)
        ),
        Say::NoSignIn { host: h, login } => tr!(
            "{0} wouldn't let {1} sign in. Check the username, the password and the server's settings.",
            host(h),
            login
        ),
        Say::NoFolders { host: h } => tr!(
            "Signed in, but {0} didn't list its folders. Try again, or check the server.",
            host(h)
        ),
        Say::NoStartTls { host: h } => tr!(
            "{0} doesn't offer STARTTLS, so Komorebi won't sign in (your password would travel unencrypted). Ask the administrator to enable it, or use the SSL/TLS port.",
            host(h)
        ),
        Say::PlainAnswer => tr!(
            "This port answered without encryption. Komorebi only connects securely; choose STARTTLS or the SSL/TLS port."
        ),
        Say::NoAuth { host: h } => tr!(
            "{0} doesn't offer sign-in. Komorebi can't send through servers without sign-in yet.",
            host(h)
        ),
        Say::BadGreeting { host: h } => tr!(
            "{0} didn't greet Komorebi like an IMAP server (it said goodbye, or signed in by itself). Check the server name, the port and the security setting.",
            host(h)
        ),
        Say::NoLogin { host: h } => tr!(
            "{0} doesn't allow signing in with a password over this connection (LOGINDISABLED). Ask the administrator to allow it on secure connections.",
            host(h)
        ),
        Say::Utf8Only { host: h } => tr!(
            "{0} only works in UTF-8 mode (UTF8=ONLY), which Komorebi doesn't support yet. Ask the administrator to also allow the standard mode.",
            host(h)
        ),
        Say::CertChanged { host: h } => tr!(
            "The certificate of {0} changed. If you didn't expect that, don't sign in: review the new certificate and compare its fingerprint before you trust it.",
            host(h)
        ),
        Say::SenderRefused { host: h, from } => tr!(
            "{0} refused to send as {1}. Check that this account may send from that address, or set the SMTP username.",
            host(h),
            from
        ),
        Say::SignInLatched => {
            tr!("Sign-in rejected. Update the password in Settings, or press Refresh to retry.")
        }
        Say::Unconfirmed { detail } => tr!(
            "{0}. The message may or may not have been sent; check Sent before you retry it from the Outbox.",
            detail
        ),
        Say::TooLargeSend { detail } => {
            tr!("The server says this message is too large ({0})", detail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    fn h() -> String {
        "imap.example.com".to_string()
    }

    /// Every `Say` and the English the Slint app renders for it (the same
    /// list as `src/app/text.rs`'s render test): the GPUI text is the
    /// Slint text, word for word.
    #[rustfmt::skip]
    fn english() -> Vec<(Say, &'static str)> {
        vec![
            (Say::Class(Class::Offline), "Can't reach the server. Komorebi tries again by itself; check your internet connection."),
            (Say::Class(Class::Auth), "The server rejected the sign-in. Update the password in Settings, or press Refresh to retry."),
            (Say::Class(Class::TlsCert), "Couldn't make a secure connection to the server. Check the server name in Settings; its certificate may not be trusted."),
            (Say::Class(Class::TooLarge), "The server says this message is too large. Remove an attachment or share it another way."),
            (Say::Class(Class::Quota), "The mailbox is full on the server. Delete or archive some mail, then try again."),
            (Say::Class(Class::TooManyConnections), "The server is busy or has too many connections open. Komorebi tries again shortly; close other mail apps if this keeps happening."),
            (Say::NotFound { host: h() }, "Couldn't find imap.example.com. Check the spelling of the server name and your internet connection."),
            (Say::NoAnswer { host: h(), port: 993 }, "imap.example.com didn't answer on port 993. SSL/TLS usually uses 993 (IMAP) or 465 (SMTP); STARTTLS uses 143 or 587 (some servers use 25, 2525 or 1143)."),
            (Say::Lost { host: h() }, "The connection to imap.example.com was lost. Try again, and if it keeps happening, check the server."),
            (Say::Rejected { host: h(), login: "me@example.com".into() }, "imap.example.com rejected the password for me@example.com. If sign-in uses two steps, create an app password. If the login isn't your email address, set the username."),
            (Say::SecureFailed { host: h() }, "The secure connection to imap.example.com failed. Its certificate isn't trusted or doesn't match the server name. Check the name, or if you run this server yourself, review its certificate."),
            (Say::NoConnection { host: h(), port: 1143 }, "imap.example.com didn't accept the connection on port 1143. Check the server name and port."),
            (Say::NoSecure { host: h() }, "Couldn't set up a secure connection to imap.example.com. Check the port and the security setting."),
            (Say::NoSignIn { host: h(), login: "alice".into() }, "imap.example.com wouldn't let alice sign in. Check the username, the password and the server's settings."),
            (Say::NoFolders { host: h() }, "Signed in, but imap.example.com didn't list its folders. Try again, or check the server."),
            (Say::NoStartTls { host: h() }, "imap.example.com doesn't offer STARTTLS, so Komorebi won't sign in (your password would travel unencrypted). Ask the administrator to enable it, or use the SSL/TLS port."),
            (Say::PlainAnswer, "This port answered without encryption. Komorebi only connects securely; choose STARTTLS or the SSL/TLS port."),
            (Say::NoAuth { host: h() }, "imap.example.com doesn't offer sign-in. Komorebi can't send through servers without sign-in yet."),
            (Say::BadGreeting { host: h() }, "imap.example.com didn't greet Komorebi like an IMAP server (it said goodbye, or signed in by itself). Check the server name, the port and the security setting."),
            (Say::NoLogin { host: h() }, "imap.example.com doesn't allow signing in with a password over this connection (LOGINDISABLED). Ask the administrator to allow it on secure connections."),
            (Say::Utf8Only { host: h() }, "imap.example.com only works in UTF-8 mode (UTF8=ONLY), which Komorebi doesn't support yet. Ask the administrator to also allow the standard mode."),
            (Say::CertChanged { host: h() }, "The certificate of imap.example.com changed. If you didn't expect that, don't sign in: review the new certificate and compare its fingerprint before you trust it."),
            (Say::SenderRefused { host: h(), from: "me@example.com".into() }, "imap.example.com refused to send as me@example.com. Check that this account may send from that address, or set the SMTP username."),
            (Say::SignInLatched, "Sign-in rejected. Update the password in Settings, or press Refresh to retry."),
            (Say::Unconfirmed { detail: "sending failed".into() }, "sending failed. The message may or may not have been sent; check Sent before you retry it from the Outbox."),
            (Say::TooLargeSend { detail: "552".into() }, "The server says this message is too large (552)"),
        ]
    }

    #[test]
    fn failures_read_as_in_the_slint_app() {
        with_lang(Lang::En, || {
            for (s, want) in english() {
                assert_eq!(say(&s), want, "{s:?}");
            }
            assert_eq!(say(&Say::Class(Class::Other)), "");
        });
    }

    #[test]
    fn every_failure_is_translated() {
        with_lang(Lang::PtPt, || {
            for (s, english) in english() {
                let pt = say(&s);
                assert!(
                    !pt.is_empty() && pt != english,
                    "{s:?} is not translated: {pt}"
                );
                // Names and numbers come through in Portuguese too.
                for fact in ["imap.example.com", "me@example.com", "993", "1143", "552"] {
                    if english.contains(fact) {
                        assert!(pt.contains(fact), "{s:?} lost {fact}: {pt}");
                    }
                }
            }
        });
    }

    #[test]
    fn sentences_in_european_portuguese() {
        with_lang(Lang::PtPt, || {
            assert_eq!(
                say(&Say::Class(Class::Offline)),
                "Não foi possível contactar o servidor. O Komorebi tenta novamente de forma automática; verifique a ligação à Internet."
            );
            assert_eq!(window_title(3), "Caixa de entrada (3) - Komorebi");
            assert_eq!(window_title(0), "Komorebi");
            assert_eq!(tray_open(), "Abrir o Komorebi");
            assert_eq!(tray_tooltip(1), "Komorebi: 1 mensagem por ler");
            assert_eq!(tray_tooltip(4), "Komorebi: 4 mensagens por ler");
            assert_eq!(tray_tooltip(0), "Komorebi");
            assert_eq!(toast_burst(2), "2 mensagens novas");
            // The context: the status, not the Sent folder ("Itens enviados").
            assert_eq!(sent(), "Enviada");
            assert_eq!(tr!("Sent"), "Itens enviados");
            assert!(start_failed("x").ends_with("\n\nx"));
        });
    }

    #[test]
    fn sentences_in_english() {
        with_lang(Lang::En, || {
            assert_eq!(window_title(3), "Inbox (3) - Komorebi");
            assert_eq!(tray_tooltip(1), "Komorebi: 1 unread message");
            assert_eq!(toast_burst(5), "5 new messages");
            assert_eq!(sent(), "Sent");
            assert_eq!(
                start_failed("disk full"),
                "Komorebi couldn't start.\n\ndisk full"
            );
            assert_eq!(
                say(&Say::NotFound { host: "::1".into() }),
                "Couldn't find [::1]. Check the spelling of the server name and your internet connection."
            );
        });
    }

    #[test]
    fn the_language_can_change_between_two_calls() {
        let s = Say::Lost { host: h() };
        let en = with_lang(Lang::En, || say(&s));
        let pt = with_lang(Lang::PtPt, || say(&s));
        assert_ne!(en, pt);
        assert_eq!(with_lang(Lang::En, || say(&s)), en);
    }

    // ---- Ports of the Slint app's `src/app/text.rs` tests (GPUI-TEST-PORT.md
    // rows 240, 245 to 248). The sentences live with their views; these
    // check the properties the Slint tests did, on the GPUI functions.

    use crate::views::mail::sync_text;
    use crate::views::reader::trust_notice;
    use crate::views::settings::hosts::{FormError, Proto};
    use crate::views::settings::text as settings_text;
    use komorebi::db::Folder;
    use komorebi::mail::errors::Problem;
    use komorebi::mail::{Notice, Outcome, Reason};
    use komorebi::role::Role;
    use komorebi::trust;

    /// Port of `a_rejected_password_reads_by_provider` (row 240): the
    /// account form's rejected-password sentence is the advice by provider
    /// (`views::settings::text::rejected`), whole, in both languages.
    #[test]
    fn a_rejected_password_reads_by_provider() {
        with_lang(Lang::En, || {
            let (host, login) = ("mail.example.com", "ana");
            let other = settings_text::rejected(None, "ana@example.com", host, login);
            assert_eq!(
                other,
                "mail.example.com rejected the password for ana. If sign-in uses two steps, create an \
                 app password. If the login isn't your email address, set the username."
            );
            // A preset on its own domain: the app-password advice only.
            let own = settings_text::rejected(Some(0), "ana@purelymail.com", host, login);
            assert_eq!(
                own,
                "mail.example.com rejected the password for ana. If sign-in uses two steps, create an \
                 app password."
            );
            // On a custom domain: maybe the other preset hosts it.
            let custom = settings_text::rejected(Some(0), "ana@example.com", host, login);
            assert!(
                custom.ends_with(" Is your mail at Postale.io? Change Provider."),
                "{custom}"
            );
            let custom = settings_text::rejected(Some(1), "ana@example.com", host, login);
            assert!(
                custom.ends_with(" Is your mail at Purelymail? Change Provider."),
                "{custom}"
            );
            // A preset index that is no preset reads as Other server.
            assert_eq!(
                settings_text::rejected(Some(99), "ana@example.com", host, login),
                other
            );
        });
        // Portuguese, whole sentences.
        with_lang(Lang::PtPt, || {
            let pt = settings_text::rejected(Some(1), "ana@example.com", "mail.example.com", "ana");
            assert!(
                pt.ends_with("O seu correio está na Purelymail? Altere o Fornecedor."),
                "{pt}"
            );
        });
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

    /// Port of `typed_values_have_a_sentence_in_both_languages` (row 245):
    /// every typed value the views keep has a whole sentence in English and
    /// in Portuguese.
    #[test]
    fn typed_values_have_a_sentence_in_both_languages() {
        use crate::views::mail::text::folder_label;
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
            with_lang(lang, || {
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
                    assert!(!sync_text::notice(&n).is_empty(), "{lang:?} {n:?}");
                }
                for e in [
                    FormError::Email,
                    FormError::Provider,
                    FormError::ServerNames,
                    FormError::ServerNameOnly,
                    FormError::ZoneId,
                    FormError::AsciiName,
                    FormError::Port(Proto::Imap),
                    FormError::Port(Proto::Smtp),
                    FormError::Username(Proto::Imap),
                    FormError::Username(Proto::Smtp),
                ] {
                    let text = settings_text::form_error(e);
                    assert!(!text.is_empty() && !text.contains('{'), "{lang:?} {e:?}");
                }
                assert!(trust_notice(&[]).is_empty());
                for role in [
                    Role::Inbox,
                    Role::Drafts,
                    Role::Sent,
                    Role::Archive,
                    Role::Junk,
                    Role::Trash,
                    Role::Other,
                ] {
                    assert!(
                        !folder_label(&folder(role, "Server's X")).is_empty(),
                        "{lang:?} {role:?}"
                    );
                }
            });
        }
        // English keeps the server's own spelling of a folder; the other
        // languages name the role.
        let sent = folder(Role::Sent, "Sent Items");
        with_lang(Lang::En, || assert_eq!(folder_label(&sent), "Sent Items"));
        with_lang(Lang::PtPt, || assert_ne!(folder_label(&sent), "Sent Items"));
    }

    /// Port of `a_sender_notice_names_its_reasons_in_english` (row 246).
    #[test]
    fn a_sender_notice_names_its_reasons_in_english() {
        use trust::Reason as R;
        with_lang(Lang::En, || {
            let reasons = [
                R::Dmarc,
                R::Punycode {
                    ascii: "xn--pypal-4ve.com".into(),
                    unicode: "p\u{430}ypal.com".into(),
                },
            ];
            assert_eq!(
                trust_notice(&reasons),
                "Reasons: it failed your server's sender check (DMARC); xn--pypal-4ve.com is the international name \u{201c}p\u{430}ypal.com\u{201d}."
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
        });
        // In Portuguese the script is named in Portuguese too.
        with_lang(Lang::PtPt, || {
            let pt = trust_notice(&[R::MixedScript {
                domain: "x.com".into(),
                script: "Cyrillic",
            }]);
            assert!(pt.contains("x.com") && !pt.contains("Cyrillic"), "{pt}");
        });
    }

    /// Port of `a_problem_says_its_class_or_its_own_words` (row 247).
    #[test]
    fn a_problem_says_its_class_or_its_own_words() {
        with_lang(Lang::En, || {
            let offline = Problem::of(&anyhow::Error::new(std::io::Error::from(
                std::io::ErrorKind::TimedOut,
            )));
            assert!(sync_text::problem(&offline).starts_with("Can't reach the server."));
            let other = Problem::of(&anyhow::anyhow!("opening folder X"));
            assert_eq!(sync_text::problem(&other), "opening folder X");
            let changed =
                Problem::of(&komorebi::mail::pin::PinMismatch("imap.example.com".into()).into());
            assert!(
                sync_text::problem(&changed)
                    .starts_with("The certificate of imap.example.com changed.")
            );
        });
    }

    /// Port of `a_send_reads_by_how_it_ended` (row 248).
    #[test]
    fn a_send_reads_by_how_it_ended() {
        with_lang(Lang::En, || {
            let out = |o| sync_text::outcome(&o);
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
        });
        // Portuguese: the same cases, none left in English.
        with_lang(Lang::PtPt, || {
            assert_eq!(sync_text::outcome(&Outcome::Delivered), "Enviada");
            let failed = sync_text::outcome(&Outcome::Failed("550".into()));
            assert!(
                failed.contains("550") && !failed.contains("Not sent"),
                "{failed}"
            );
        });
    }
}
