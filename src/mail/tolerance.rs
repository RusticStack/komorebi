//! More of what servers do wrongly or less: no password before the server
//! has earned it (STARTTLS lists and answers, hang-ups), sending (the EHLO
//! name, awkward passwords, a refused sender), and the workers stopping
//! where only the user can help (a changed certificate, a rejected
//! password) instead of trying again and again. The capability profiles are
//! in `profiles`.

use super::tests::{queued, setup, worker_with};
use super::*;
use crate::providers::Security;

// ---- the secured paths

mod secured {
    use super::super::fake_tls::{CERT, KEY, Mode, Proto, TlsFake, der_of};
    use super::*;
    use crate::mail::errors::{Class, Special, Step as Check};

    fn server(fake: &TlsFake, security: Security) -> Server {
        Server {
            host: "localhost".into(),
            port: fake.addr.port(),
            security,
        }
    }

    #[test]
    fn starttls_lists_the_capabilities_again_before_the_password() {
        let der = der_of(CERT);
        let fake = TlsFake::start(Proto::Imap, Mode::StartTls, CERT, KEY);
        let imap = Imap::connect(
            &server(&fake, Security::StartTls),
            Some(&der),
            "alice",
            "pw",
        )
        .expect("signs in after STARTTLS");
        imap.logout();
        let seen = fake.finish();
        // Asked for in the clear, asked for again under TLS, and only then
        // the password.
        assert_eq!(
            &seen[..4],
            [
                "a0 CAPABILITY",
                "a1 STARTTLS",
                "c0 CAPABILITY",
                "a1 LOGIN \"alice\" \"pw\""
            ],
            "{seen:?}"
        );
    }

    #[test]
    fn a_refused_starttls_sends_no_password() {
        let der = der_of(CERT);
        for mode in [Mode::StartTlsRefused, Mode::NoStartTls] {
            for pin in [None, Some(&der[..])] {
                let fake = TlsFake::start(Proto::Imap, mode, CERT, KEY);
                let err = Imap::connect(&server(&fake, Security::StartTls), pin, "alice", "pw")
                    .err()
                    .expect("refused");
                assert!(
                    err.chain().any(|c| c.is::<imap::StartTlsRefused>()),
                    "{err:#}"
                );
                // The workers stop on it rather than ask again.
                assert!(errors::needs_attention(&err));
                let seen = fake.finish();
                assert!(!seen.iter().any(|l| l.contains("LOGIN")), "{seen:?}");
            }
        }
    }

    #[test]
    fn awkward_passwords_are_sent_as_typed_under_tls() {
        // Quoted: the quote and the backslash escaped, the rest as typed
        // (UTF-8), nothing asked for first.
        let der = der_of(CERT);
        for (password, quoted) in [("p\"w\\ x", "p\\\"w\\\\ x"), ("pässwörd ✓", "pässwörd ✓")]
        {
            // A loaded machine can drop one connect or handshake of the
            // throwaway fake; a fresh fake gets a few tries (a failed one is
            // left to end with the process, since it may still be waiting).
            let mut last = None;
            let mut done = None;
            for _ in 0..4 {
                let fake = TlsFake::start(Proto::Imap, Mode::Implicit, CERT, KEY);
                match Imap::connect(&server(&fake, Security::Tls), Some(&der), "alice", password) {
                    Ok(imap) => {
                        imap.logout();
                        done = Some(fake.finish());
                        break;
                    }
                    Err(e) => {
                        last = Some(e);
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                }
            }
            let seen = done.unwrap_or_else(|| panic!("signs in: {:#}", last.unwrap()));
            assert_eq!(
                seen[0],
                format!("a1 LOGIN \"alice\" \"{quoted}\""),
                "{seen:?}"
            );
        }
    }

    /// A port that accepts and hangs up at once.
    fn hangs_up() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            while let Ok((stream, _)) = listener.accept() {
                drop(stream);
            }
        });
        port
    }

    #[test]
    fn a_server_that_hangs_up_was_not_reached_and_is_not_an_untrusted_one() {
        let host = |port, security| Server {
            host: "127.0.0.1".into(),
            port,
            security,
        };
        // STARTTLS: it never greeted.
        let (step, e) = Imap::connect_steps(
            &host(hangs_up(), Security::StartTls),
            None,
            "alice",
            "pw",
            Some(&mut |_| {}),
        )
        .err()
        .expect("hung up");
        assert_eq!(step, Check::Connect, "{e:#}");
        let shown = imap_step_error(step, e, &host(0, Security::StartTls), false, "alice");
        assert_eq!(
            (shown.class, shown.special),
            (Class::Offline, Special::None)
        );
        // SSL/TLS: the handshake was cut off, which says nothing about the
        // certificate, so no worker waits for the user on it.
        let (_, e) =
            Imap::connect_steps(&host(hangs_up(), Security::Tls), None, "alice", "pw", None)
                .err()
                .expect("hung up");
        assert!(!errors::needs_attention(&e), "{e:#}");
        assert_eq!(errors::classify(&e), Class::Offline, "{e:#}");
    }

    #[test]
    fn bad_greetings_and_logins_have_their_own_sentences() {
        let host = Server {
            host: "mail.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        let say = |step, e: anyhow::Error| imap_step_error(step, e, &host, false, "alice").say();
        for greeting in [
            imap::GreetingRefused::PreAuth,
            imap::GreetingRefused::Bye,
            imap::GreetingRefused::Other,
        ] {
            let said = say(Check::ImapSignIn, greeting.into());
            assert_eq!(
                said,
                Say::BadGreeting {
                    host: "mail.example.com".into()
                }
            );
        }
        let said = say(Check::ImapSignIn, imap::NoSignIn.into());
        assert_eq!(
            said,
            Say::NoLogin {
                host: "mail.example.com".into()
            }
        );
        let said = say(Check::ImapSignIn, imap::Utf8Only.into());
        assert_eq!(
            said,
            Say::Utf8Only {
                host: "mail.example.com".into()
            }
        );
    }
}

// ---- sending

mod sending {
    use super::*;
    use crate::mail::fake::{FakeSmtp, SmtpEnd};

    #[test]
    fn ehlo_never_names_this_computer() {
        let smtp = FakeSmtp::start(SmtpEnd::Accept);
        smtp::verify_plain(smtp.port, "alice", "pw").unwrap();
        let (first, _) = smtp.finish();
        assert_eq!(first.trim_end(), "EHLO [127.0.0.1]");
        for var in ["COMPUTERNAME", "HOSTNAME"] {
            if let Ok(name) = std::env::var(var) {
                assert!(
                    !first
                        .to_ascii_lowercase()
                        .contains(&name.to_ascii_lowercase()),
                    "{first}"
                );
            }
        }
    }

    #[test]
    fn awkward_passwords_reach_smtp_byte_for_byte() {
        use base64::Engine as _;
        for password in ["p\"w\\ x", "pässwörd ✓", "with space"] {
            let smtp = FakeSmtp::start(SmtpEnd::Accept);
            smtp::verify_plain(smtp.port, "alice", password).unwrap();
            let auth = smtp.finish_with_auth();
            let payload = auth
                .iter()
                .find_map(|l| l.strip_prefix("AUTH PLAIN "))
                .expect("an AUTH PLAIN line");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .unwrap();
            assert_eq!(bytes, format!("\0alice\0{password}").as_bytes());
        }
    }

    /// Send the queued message of a fresh account through `smtp`, a closure
    /// standing for the sending server; returns how often it was called, what
    /// the send ended as and the outbox row's state.
    fn send_with(
        smtp: impl Fn(&Envelope, &[u8]) -> Result<()> + Send + 'static,
    ) -> (usize, Outcome, Option<db::Outbox>) {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[1]);
        let account = db.accounts().unwrap().remove(0);
        let row = queued(&db, account.id, "hello");
        let (mut w, events) = worker_with(&db, None);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        w.smtp = Box::new(move |_, env, raw| {
            count.fetch_add(1, Ordering::SeqCst);
            smtp(env, raw)
        });
        w.run_job(&mut db, Job::Send { row }, &mut || None);
        let outcome = events
            .lock()
            .unwrap()
            .iter()
            .find_map(|e| match e {
                Event::Sent { outcome, .. } => Some(outcome.clone()),
                _ => None,
            })
            .expect("a send ends");
        let state = db.outbox_entry(row).unwrap().map(|e| e.state);
        (calls.load(Ordering::SeqCst), outcome, state)
    }

    #[test]
    fn a_server_refusing_the_sender_is_told_by_name() {
        let server = FakeSmtp::start(SmtpEnd::RejectSender);
        let port = server.port;
        let (calls, outcome, state) = send_with(move |env, raw| smtp::send_plain(port, env, raw));
        assert_eq!(calls, 1);
        assert_eq!(
            outcome,
            Outcome::Failed(Reason::said(errors::sender_refused(
                "imap.example.com",
                "me@example.com"
            )))
        );
        // Not tried again by itself.
        assert_eq!(state, Some(db::Outbox::Failed));
    }

    #[test]
    fn a_changed_certificate_fails_the_send_once() {
        let (calls, outcome, state) =
            send_with(|_, _| Err(pin::PinMismatch("smtp.example.com".into()).into()));
        assert_eq!(calls, 1);
        assert_eq!(
            outcome,
            Outcome::Failed(Reason::said(Say::CertChanged {
                host: "smtp.example.com".into()
            }))
        );
        assert_eq!(state, Some(db::Outbox::Failed));
    }
}

// ---- the workers stop where only the user can help

mod waiting {
    use super::*;
    use crate::mail::errors::Class;
    use std::sync::atomic::AtomicUsize;

    /// A worker whose connections all fail with what `fail` makes; returns it
    /// with its events and the number of connection attempts.
    fn failing(
        db: &Db,
        fail: impl Fn() -> anyhow::Error + Send + 'static,
    ) -> (Worker, Arc<Mutex<Vec<Event>>>, Arc<AtomicUsize>) {
        let (mut w, events) = worker_with(db, None);
        let tries = Arc::new(AtomicUsize::new(0));
        let count = tries.clone();
        w.connect = Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Err(fail())
        });
        (w, events, tries)
    }

    #[test]
    fn a_changed_certificate_makes_one_attempt_and_waits_for_retry() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[1]);
        let id = db.message_ids(&[f.id], "").unwrap()[0];
        let (mut w, events, tries) =
            failing(&db, || pin::PinMismatch("imap.example.com".into()).into());
        // Every kind of job that would connect.
        for _ in 0..3 {
            w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
        }
        let folder = Job::SyncFolder {
            id: f.id,
            full: true,
        };
        w.run_job(&mut db, folder, &mut || None);
        w.run_job(&mut db, Job::FetchBody(id), &mut || None);
        w.run_job(&mut db, Job::Warm, &mut || None);
        assert_eq!(
            tries.load(Ordering::SeqCst),
            1,
            "one attempt, then it waits"
        );
        assert!(w.auth_failed.get() && w.auth_failed.is_secure());
        // One banner, with the sentence for a changed certificate.
        let problems: Vec<Problem> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::Syncing { error: Some(p), .. } => Some(p.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].class, Class::TlsCert);
        assert_eq!(
            problems[0].say(),
            Some(Say::CertChanged {
                host: "imap.example.com".into()
            })
        );
        // Retry (Refresh, or editing the account) lets it try once more.
        w.auth_failed.clear();
        w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
        assert_eq!(tries.load(Ordering::SeqCst), 2);
        assert!(w.auth_failed.get());
    }

    #[test]
    fn an_untrusted_certificate_and_a_rejected_password_wait_too() {
        let cases: [(fn() -> anyhow::Error, bool); 2] = [
            (|| imap::TlsFailed("imap.example.com".into()).into(), true),
            (|| imap::AuthFailed("bad".into()).into(), false),
        ];
        for (fail, secure) in cases {
            let mut db = Db::in_memory().unwrap();
            setup(&mut db, 7, &[1]);
            let (mut w, _, tries) = failing(&db, fail);
            for _ in 0..3 {
                w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
            }
            assert_eq!(tries.load(Ordering::SeqCst), 1);
            assert_eq!(w.auth_failed.is_secure(), secure);
        }
    }

    #[test]
    fn plain_network_trouble_keeps_trying() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[1]);
        let (mut w, _, tries) = failing(&db, || {
            anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::TimedOut))
        });
        for _ in 0..3 {
            w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
        }
        assert_eq!(tries.load(Ordering::SeqCst), 3);
        assert!(!w.auth_failed.get());
    }
}

// ---- folders the server doesn't mark, and the ones the user chooses

mod roles {
    use super::super::fake_imapd::{FakeImapd, MailBox, Profile, sample};
    use super::*;
    use crate::db::NewAccount;

    /// An account on a server whose folders are INBOX, Trash and two the
    /// server does not name by role: "Mail Out" and "Mail Old".
    fn account() -> (FakeImapd, Db, Worker, Arc<Mutex<Vec<Event>>>, i64) {
        let boxes = vec![
            MailBox::new("INBOX", ""),
            MailBox::new("Trash", "\\Trash"),
            MailBox::new("Mail Out", ""),
            MailBox::new("Mail Old", ""),
        ];
        let server = FakeImapd::start(Profile::BARE, boxes);
        let mut db = Db::in_memory().unwrap();
        let host = Server {
            host: "imap.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        let id = db
            .add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: "me@example.com".into(),
                name: "Me".into(),
                imap: host.clone(),
                smtp: host,
                save_sent: true,
            })
            .unwrap();
        let (mut w, events) = worker_with(&db, None);
        let addr = server.addr;
        w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
        w.smtp = Box::new(|_, _, _| Ok(()));
        w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
        (server, db, w, events, id)
    }

    fn sent_outcomes(events: &Arc<Mutex<Vec<Event>>>) -> Vec<Outcome> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::Sent { outcome, .. } => Some(outcome.clone()),
                _ => None,
            })
            .collect()
    }

    fn notices(events: &Arc<Mutex<Vec<Event>>>) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Event::Notice { .. }))
            .count()
    }

    #[test]
    fn without_a_sent_folder_the_message_goes_and_it_is_said_once() {
        let (server, mut db, mut w, events, id) = account();
        assert!(db.folder_by_role(id, Role::Sent).unwrap().is_none());
        for _ in 0..2 {
            let row = queued(&db, id, "hello");
            w.run_job(&mut db, Job::Send { row }, &mut || None);
        }
        assert_eq!(
            sent_outcomes(&events),
            [Outcome::Delivered, Outcome::Delivered]
        );
        assert_eq!(notices(&events), 1, "said once");
        // Nothing was created or appended.
        let st = server.state();
        assert_eq!(st.boxes.len(), 4);
        assert!(st.sent("APPEND").is_empty());
    }

    #[test]
    fn a_folder_chosen_as_sent_gets_the_copy_and_the_choice_is_undone_cleanly() {
        let (server, mut db, mut w, events, id) = account();
        let out = db
            .folders()
            .unwrap()
            .into_iter()
            .find(|f| f.name == "Mail Out")
            .unwrap();
        let old = db
            .folders()
            .unwrap()
            .into_iter()
            .find(|f| f.name == "Mail Old")
            .unwrap();
        db.set_role_override(id, Role::Sent, Some(out.id)).unwrap();
        assert_eq!(
            db.folder_by_role(id, Role::Sent).unwrap().unwrap().id,
            out.id
        );
        assert_eq!(db.folder(out.id).unwrap().unwrap().role, Role::Sent);
        let row = queued(&db, id, "hello");
        w.run_job(&mut db, Job::Send { row }, &mut || None);
        assert_eq!(sent_outcomes(&events), [Outcome::Delivered]);
        assert_eq!(server.state().mailbox("Mail Out").msgs.len(), 1);

        // Choosing another folder replaces the choice; the first one is an
        // ordinary folder again.
        db.set_role_override(id, Role::Sent, Some(old.id)).unwrap();
        assert_eq!(
            db.folder_by_role(id, Role::Sent).unwrap().unwrap().id,
            old.id
        );
        assert_eq!(db.folder(out.id).unwrap().unwrap().role, Role::Other);
        assert_eq!(
            db.chosen_folder(id, Role::Sent).unwrap().unwrap().id,
            old.id
        );
        // A sync keeps the choice (the server still marks neither).
        w.run_job(&mut db, Job::SyncAll { full: true }, &mut || None);
        assert_eq!(
            db.folder_by_role(id, Role::Sent).unwrap().unwrap().id,
            old.id
        );
        // Back to what the server says: no Sent folder.
        db.set_role_override(id, Role::Sent, None).unwrap();
        assert!(db.folder_by_role(id, Role::Sent).unwrap().is_none());
        assert!(db.chosen_folder(id, Role::Sent).unwrap().is_none());
    }

    #[test]
    fn a_chosen_folder_beats_the_one_the_server_marks_and_both_are_known() {
        let (_server, db, _w, _events, id) = account();
        let by_server = db.folder_by_role(id, Role::Trash).unwrap().unwrap();
        assert_eq!(by_server.name, "Trash");
        let old = db
            .folders()
            .unwrap()
            .into_iter()
            .find(|f| f.name == "Mail Old")
            .unwrap();
        db.set_role_override(id, Role::Trash, Some(old.id)).unwrap();
        assert_eq!(
            db.folder_by_role(id, Role::Trash).unwrap().unwrap().id,
            old.id
        );
        // What the server marks stays known, for "Automatic".
        assert_eq!(
            db.detected_folder(id, Role::Trash).unwrap().unwrap().id,
            by_server.id
        );
        // The inbox and ordinary roles can't be chosen, nor the inbox used.
        let inbox = db.folder_by_role(id, Role::Inbox).unwrap().unwrap();
        assert!(db.set_role_override(id, Role::Inbox, Some(old.id)).is_err());
        assert!(
            db.set_role_override(id, Role::Archive, Some(inbox.id))
                .is_err()
        );
        assert!(db.folder_by_role(id, Role::Archive).unwrap().is_none());
        // Another account's folder is not this account's to use.
        assert!(
            db.set_role_override(id + 1, Role::Junk, Some(old.id))
                .is_err()
        );
        let _ = sample(1);
    }
}
