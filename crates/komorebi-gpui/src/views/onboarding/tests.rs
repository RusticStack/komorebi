//! The setup flow's view tests: ports of the Slint app's view-driven
//! onboarding tests (src/app/composer.rs; docs/GPUI-TEST-PORT.md phase 7).
//! The check, the lookup and the password store are fakes
//! ([`services::Services`]); nothing here touches the network.

// One test module, so the token lint and the catalog's literal check
// (both skip what a `#[cfg(test)]` is on) leave it out.
#[cfg(test)]
mod port {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui::{AnyWindowHandle, AppContext as _, Entity, Role, TestAppContext};
    use gpui_kit::component::input::InputState;
    use komorebi::mail::discover::Outcome;
    use komorebi::mail::errors::{Step as CheckStep, StepError};

    use super::super::*;
    use crate::test_support::{self, TempCache, assert_named, in_window};
    use crate::views::settings::services::{self, Services};

    struct Fixture {
        window: AnyWindowHandle,
        view: Entity<OnboardingView>,
        events: Rc<RefCell<Vec<OnboardingEvent>>>,
        cache: TempCache,
    }

    fn open(cx: &mut TestAppContext, name: &str, svc: Services, first_run: bool) -> Fixture {
        test_support::init_app(cx);
        cx.update(|cx| cx.set_global(svc));
        let cache = TempCache::new(name);
        let db = cache.open();
        let (window, view) = test_support::open(cx, move |window, cx| {
            cx.new(|cx| {
                if first_run {
                    OnboardingView::new(Rc::new(db), window, cx)
                } else {
                    OnboardingView::adding(Rc::new(db), window, cx)
                }
            })
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, e: &OnboardingEvent, _| {
                sink.borrow_mut().push(e.clone());
            })
            .detach();
        });
        cx.run_until_parked();
        Fixture {
            window,
            view,
            events,
            cache,
        }
    }

    impl Fixture {
        fn set(
            &self,
            cx: &mut TestAppContext,
            pick: fn(&OnboardingView) -> &Entity<InputState>,
            text: &str,
        ) {
            let state = cx.read(|cx| pick(self.view.read(cx)).clone());
            let text = text.to_string();
            in_window(cx, self.window, |window, cx| {
                state.update(cx, |s, cx| s.set_value(text, window, cx));
            });
        }

        fn act(
            &self,
            cx: &mut TestAppContext,
            f: impl FnOnce(&mut OnboardingView, &mut Window, &mut Context<OnboardingView>),
        ) {
            let view = self.view.clone();
            in_window(cx, self.window, |window, cx| {
                view.update(cx, |v, cx| f(v, window, cx))
            });
        }

        fn db(&self) -> komorebi::db::Db {
            self.cache.open()
        }
    }

    fn pass_all(
        _: &komorebi::providers::Server,
        _: &komorebi::providers::Server,
        _: komorebi::mail::Pins,
        _: (&str, &str),
        _: &str,
        _: bool,
        report: &mut dyn FnMut(CheckStep),
    ) -> Result<(), StepError> {
        CheckStep::ALL.into_iter().for_each(report);
        Ok(())
    }

    static LOOKUPS: AtomicUsize = AtomicUsize::new(0);

    fn counted(_: &str) -> Outcome {
        LOOKUPS.fetch_add(1, Ordering::SeqCst);
        Outcome::default()
    }

    fn fakes() -> Services {
        Services {
            verify: pass_all,
            lookup: counted,
            ..services::refusing()
        }
    }

    /// Ports `cancel_drops_a_late_check_answer_and_keeps_the_form`.
    #[gpui::test]
    fn cancel_drops_a_late_check_answer_and_keeps_the_form(cx: &mut TestAppContext) {
        let f = open(cx, "ob-cancel", fakes(), false);
        f.set(cx, |v| v.email_input(), "ana.silva@purelymail.com");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.read(|cx| assert!(f.view.read(cx).busy()));
        // Esc while it runs: the check is dropped, the form stays.
        f.act(cx, |v, w, cx| assert!(v.back(w, cx)));
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert!(!v.busy());
            assert_eq!(v.step(), Step::SignIn);
            assert_eq!(v.password_input().read(cx).value().to_string(), "secret");
            assert!(v.added().is_none());
        });
        assert!(f.db().accounts().unwrap().is_empty());
        assert!(f.events.borrow().is_empty());
    }

    /// Ports `an_added_account_goes_on_to_make_it_yours_and_its_first_download`.
    #[gpui::test]
    fn an_added_account_goes_on_to_make_it_yours_and_its_first_download(cx: &mut TestAppContext) {
        let f = open(cx, "ob-added", fakes(), true);
        cx.read(|cx| assert_eq!(f.view.read(cx).step(), Step::Welcome));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Add your email account", Role::Button);
        });
        f.act(cx, |v, w, cx| v.start(w, cx));
        f.set(cx, |v| v.email_input(), "ana.silva@purelymail.com");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.read(|cx| {
            assert_eq!(
                f.view.read(cx).chip(),
                &flow::Chip::Preset(preset("purelymail"))
            )
        });
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.run_until_parked();
        let id = cx.read(|cx| f.view.read(cx).added()).expect("added");
        assert_eq!(f.events.borrow()[0], OnboardingEvent::Added(id));
        in_window(cx, f.window, |_, _| {});
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert_eq!(v.step(), Step::Yours);
            assert_eq!(v.name_input().read(cx).value().to_string(), "Ana Silva");
        });
        let a = f.db().accounts().unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].email, "ana.silva@purelymail.com");
        // The first download, as the shell reports it.
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_syncing(id, true, cx);
                v.set_progress(id, 30, 100, cx);
                v.set_progress(id, 20, 100, cx);
            })
        });
        cx.read(|cx| {
            assert_eq!(
                f.view.read(cx).download(),
                Download::Counting {
                    done: 30,
                    total: 100
                }
            )
        });
        f.set(cx, |v| v.signature_input(), "Ana\nExample");
        f.act(cx, |v, w, cx| v.personalise(true, w, cx));
        cx.read(|cx| assert_eq!(f.view.read(cx).step(), Step::Done));
        let a = f.db().accounts().unwrap();
        assert_eq!(a[0].name, "Ana Silva");
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Open your inbox", Role::Button);
        });
        f.act(cx, |v, _, cx| v.open_inbox(cx));
        assert_eq!(
            f.events.borrow().last(),
            Some(&OnboardingEvent::Finished(Some(id)))
        );
    }

    /// Ports `skip_writes_nothing_and_continue_never_looks_up_a_preset`.
    #[gpui::test]
    fn skip_writes_nothing_and_continue_never_looks_up_a_preset(cx: &mut TestAppContext) {
        let f = open(cx, "ob-skip", fakes(), false);
        let before = LOOKUPS.load(Ordering::SeqCst);
        f.set(cx, |v| v.email_input(), "ana@postale.io");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        assert_eq!(
            LOOKUPS.load(Ordering::SeqCst),
            before,
            "a preset is never looked up"
        );
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(f.view.read(cx).step(), Step::Yours));
        f.act(cx, |v, w, cx| v.personalise(false, w, cx));
        let a = f.db().accounts().unwrap();
        assert_eq!(a[0].name, "");
        assert_eq!(a[0].signature, "");
        // Another domain is looked up, once, because of the press.
        f.act(cx, |v, w, cx| v.add_another(w, cx));
        f.set(cx, |v| v.email_input(), "rui@example.org");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        assert_eq!(LOOKUPS.load(Ordering::SeqCst), before + 1);
        cx.read(|cx| assert_eq!(f.view.read(cx).chip(), &flow::Chip::NotFound));
    }

    /// The email step refuses what isn't an address and an account that is
    /// already here.
    #[gpui::test]
    fn the_email_step_refuses_a_bad_or_known_address(cx: &mut TestAppContext) {
        let f = open(cx, "ob-email", fakes(), false);
        f.set(cx, |v| v.email_input(), "ana@");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.read(|cx| {
            assert_eq!(f.view.read(cx).email_problem(), Some(EmailProblem::Invalid));
            assert_eq!(f.view.read(cx).step(), Step::Email);
        });
        // Offline: the provider list, no lookup.
        cx.update(|cx| {
            cx.set_global(Services {
                offline: true,
                ..fakes()
            })
        });
        f.set(cx, |v| v.email_input(), "rui@example.org");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert!(v.choosing() && v.offline() && !v.manual());
            assert_eq!(v.step(), Step::SignIn);
        });
        // Sign in with nothing typed marks what is missing.
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.read(|cx| {
            let m = f.view.read(cx).missing();
            assert!(m.password && m.provider);
        });
        f.act(cx, |v, w, cx| v.go_manual(w, cx));
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.read(|cx| {
            let m = f.view.read(cx).missing();
            assert!(m.password && m.imap && m.smtp && !m.provider);
        });
    }

    fn found_settings(_: &str) -> Outcome {
        use komorebi::mail::discover::{Discovered, Found, Source};
        use komorebi::providers::{Security, Server};
        let found = |host: &str, port, security| Found {
            server: Server {
                host: host.into(),
                port,
                security,
            },
            user: None,
        };
        Outcome {
            found: Discovered {
                imap: Some(found("imap.example.org", 993, Security::Tls)),
                smtp: Some(found("smtp.example.org", 465, Security::Tls)),
            },
            sources: vec![Source::Autoconfig],
            refused: Vec::new(),
            preset: None,
        }
    }

    /// Manual setup words its fields as the Slint flow, has Find settings
    /// (the lookup again, for the address typed), and adds a relay that
    /// sends without a sign-in.
    #[gpui::test]
    fn manual_setup_finds_settings_and_adds_a_relay(cx: &mut TestAppContext) {
        let svc = Services {
            verify: pass_all,
            lookup: found_settings,
            offline: false,
            ..services::refusing()
        };
        let f = open(cx, "ob-manual-find", svc, false);
        f.set(cx, |v| v.email_input(), "rui@example.org");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        f.act(cx, |v, w, cx| v.go_manual(w, cx));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Incoming (IMAP) server", Role::TextInput);
            assert_named(window, "Outgoing (SMTP) server", Role::TextInput);
            assert_named(window, "Username", Role::TextInput);
            assert_named(window, "Find settings", Role::Button);
        });
        // The first lookup filled the servers; cleared, Find fills them again.
        let servers = cx.read(|cx| f.view.read(cx).server_form().clone());
        in_window(cx, f.window, |window, cx| {
            servers.update(cx, |s, cx| s.set(&Fields::default(), window, cx));
        });
        f.act(cx, |v, w, cx| v.find_again(w, cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let typed = servers.read(cx).fields(cx);
            assert_eq!(typed.imap_host, "imap.example.org");
            assert_eq!(typed.smtp_host, "smtp.example.org");
        });
        in_window(cx, f.window, |window, cx| {
            crate::test_support::click_named(
                window,
                "This server does not need a sign-in for sending",
                cx,
            );
        });
        cx.run_until_parked();
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(f.view.read(cx).step(), Step::Yours));
        let a = f.db().accounts().unwrap();
        assert!(a[0].smtp_no_auth, "the relay sends without a sign-in");
    }

    /// A manual server's confirmed certificate shows its line with Forget
    /// certificate (the Slint flow's `CertLine`).
    #[gpui::test]
    fn a_confirmed_certificate_shows_with_forget(cx: &mut TestAppContext) {
        let svc = Services {
            offline: true,
            ..fakes()
        };
        let f = open(cx, "ob-cert-line", svc, false);
        f.set(cx, |v| v.email_input(), "rui@example.org");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        f.act(cx, |v, w, cx| v.go_manual(w, cx));
        let server = komorebi::providers::Server {
            host: "mail.example.org".into(),
            port: 993,
            security: komorebi::providers::Security::Tls,
        };
        let servers = cx.read(|cx| f.view.read(cx).server_form().clone());
        in_window(cx, f.window, |window, cx| {
            servers.update(cx, |s, cx| {
                s.set(
                    &Fields {
                        imap_host: "mail.example.org".into(),
                        imap_port: "993".into(),
                        smtp_host: "mail.example.org".into(),
                        smtp_port: "465".into(),
                        ..Fields::default()
                    },
                    window,
                    cx,
                )
            });
        });
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.pins.set(Kind::Imap, b"a certificate".to_vec(), server);
                cx.notify();
            })
        });
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Forget certificate", Role::Button);
        });
        f.act(cx, |v, _, cx| v.forget_certificate(Kind::Imap, cx));
        cx.read(|cx| assert!(!f.view.read(cx).pins().is_saved(Kind::Imap)));
        in_window(cx, f.window, |window, _| {
            assert!(
                !crate::test_support::names(window)
                    .iter()
                    .any(|(_, n)| n == "Forget certificate")
            );
        });
    }

    fn preset(id: &str) -> usize {
        komorebi::providers::index_of(id).expect("a preset")
    }

    /// Open the flow at Sign in for an address no preset knows and the
    /// lookup can't find: the provider list shows, its search focused.
    fn at_the_list(cx: &mut TestAppContext, name: &str) -> Fixture {
        let svc = Services {
            verify: pass_all,
            lookup: |_| Outcome::default(),
            ..services::refusing()
        };
        let f = open(cx, name, svc, false);
        f.set(cx, |v| v.email_input(), "rui@example.org");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        f.act(cx, |v, w, cx| v.focus_first(w, cx));
        cx.run_until_parked();
        f
    }

    fn shown(f: &Fixture, cx: &mut TestAppContext) -> Vec<usize> {
        cx.read(|cx| f.view.read(cx).picker().read(cx).matches().to_vec())
    }

    /// When nothing was found, the providers are a named list box whose
    /// options say how each signs in, with Add provider after it; the
    /// welcome lists its three values; the step indicator is a named list
    /// that says which steps are done.
    #[gpui::test]
    fn the_flow_names_its_steps_values_and_providers(cx: &mut TestAppContext) {
        // Not `fakes()`: its lookup counter belongs to the skip test.
        let svc = Services {
            verify: pass_all,
            lookup: |_| Outcome::default(),
            ..services::refusing()
        };
        let f = open(cx, "ob-names", svc, true);
        in_window(cx, f.window, |window, _| {
            assert_named(
                window,
                "Private by design. No ads, no tracking, no AI reading your mail.",
                Role::ListItem,
            );
            assert_named(
                window,
                "Works with your provider. Gmail, iCloud, Fastmail, Postale.io and more, or your own server.",
                Role::ListItem,
            );
        });
        f.act(cx, |v, w, cx| v.start(w, cx));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Steps to add an account", Role::List);
            assert_named(window, "Your email, current step", Role::ListItem);
        });
        f.set(cx, |v| v.email_input(), "rui@example.org");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert_eq!(v.chip(), &flow::Chip::NotFound);
            assert!(v.choosing());
        });
        f.act(cx, |v, w, cx| v.focus_first(w, cx));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Your email, done", Role::ListItem);
            assert_named(window, "Sign in, current step", Role::ListItem);
            assert_named(window, "Search for your provider", Role::TextInput);
            assert_named(window, "Providers", Role::ListBox);
            assert_named(window, "Gmail, App password needed", Role::ListBoxOption);
            assert_named(window, "Free, Your account password", Role::ListBoxOption);
            assert_named(
                window,
                "Add provider: any other IMAP and SMTP server",
                Role::Button,
            );
        });
        // A provider further down, found by the search.
        in_window(cx, f.window, |window, cx| {
            test_support::type_text(window, "outlook", cx)
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, _| {
            assert_named(
                window,
                "Outlook.com, Not supported yet: needs Microsoft sign-in",
                Role::ListBoxOption,
            );
        });
        // Sign in with no provider asks for one.
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.read(|cx| assert!(f.view.read(cx).missing().provider));
    }

    /// The keyboard: the focus stays in the search box; Down, End, Home
    /// and Up move the highlighted provider; Enter picks it, which closes
    /// the list and names it on the chip; Change opens the list again and
    /// Esc closes it.
    #[gpui::test]
    fn the_keyboard_moves_through_the_providers_and_picks_one(cx: &mut TestAppContext) {
        let f = at_the_list(cx, "ob-provider-keys");
        let all = shown(&f, cx);
        assert_eq!(all.len(), komorebi::providers::PRESETS.len());
        let active = |f: &Fixture, cx: &mut TestAppContext| {
            cx.read(|cx| f.view.read(cx).picker().read(cx).active())
        };
        assert_eq!(active(&f, cx), Some(all[0]));
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["down", "down"], cx);
        });
        cx.run_until_parked();
        assert_eq!(active(&f, cx), Some(all[2]));
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["end"], cx)
        });
        cx.run_until_parked();
        assert_eq!(active(&f, cx), all.last().copied());
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["home", "down", "up"], cx);
        });
        cx.run_until_parked();
        assert_eq!(active(&f, cx), Some(all[0]));
        // The highlighted option is the list box's selected one.
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["down"], cx);
        });
        cx.run_until_parked();
        assert_eq!(active(&f, cx), Some(all[1]));
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["enter"], cx)
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert_eq!(v.provider(), Some(all[1]));
            assert_eq!(v.chip(), &flow::Chip::Preset(all[1]));
            assert!(!v.choosing() && !v.manual());
        });
        in_window(cx, f.window, |window, cx| {
            assert_named(window, "Change provider", Role::Button);
            test_support::click_named(window, "Change provider", cx);
        });
        cx.run_until_parked();
        cx.read(|cx| assert!(f.view.read(cx).choosing()));
        // Esc in the empty search box closes the list over the provider.
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["escape"], cx)
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert!(!v.choosing());
            assert_eq!(v.step(), Step::SignIn);
            assert_eq!(v.provider(), Some(all[1]));
        });
    }

    /// Typing filters by name or domain (accents and case aside); a search
    /// with no match says so and points at Add provider; Esc clears it.
    #[gpui::test]
    fn the_search_filters_the_providers(cx: &mut TestAppContext) {
        let f = at_the_list(cx, "ob-provider-search");
        in_window(cx, f.window, |window, cx| {
            test_support::type_text(window, "POSTÁLE", cx)
        });
        cx.run_until_parked();
        assert_eq!(shown(&f, cx), vec![preset("postale")]);
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["enter"], cx)
        });
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(f.view.read(cx).provider(), Some(preset("postale"))));
        // A domain finds its provider.
        in_window(cx, f.window, |window, cx| {
            test_support::click_named(window, "Change provider", cx);
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            test_support::type_text(window, "hotmail.com", cx);
        });
        cx.run_until_parked();
        assert_eq!(shown(&f, cx), vec![preset("outlook")]);
        // Esc clears the search, then a search with no match.
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["escape"], cx)
        });
        cx.run_until_parked();
        assert_eq!(shown(&f, cx).len(), komorebi::providers::PRESETS.len());
        in_window(cx, f.window, |window, cx| {
            test_support::type_text(window, "nothing like it", cx);
        });
        cx.run_until_parked();
        assert!(shown(&f, cx).is_empty());
        in_window(cx, f.window, |window, _| {
            assert!(
                test_support::names(window)
                    .iter()
                    .any(|(_, n)| n.contains("No provider matches"))
            );
        });
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["escape"], cx)
        });
        cx.run_until_parked();
        assert_eq!(shown(&f, cx).len(), komorebi::providers::PRESETS.len());
    }

    /// Add provider opens manual setup; Back returns to the list.
    #[gpui::test]
    fn add_provider_opens_manual_setup(cx: &mut TestAppContext) {
        let f = at_the_list(cx, "ob-add-provider");
        in_window(cx, f.window, |window, cx| {
            test_support::click_named(window, "Add provider: any other IMAP and SMTP server", cx);
        });
        cx.run_until_parked();
        cx.read(|cx| assert!(f.view.read(cx).manual()));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Incoming (IMAP) server", Role::TextInput);
            assert_named(window, "Find settings", Role::Button);
        });
        f.act(cx, |v, w, cx| assert!(v.back(w, cx)));
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert!(!v.manual() && v.choosing());
            assert_eq!(v.step(), Step::SignIn);
        });
    }

    /// Picking a provider fills in its servers: the account added uses the
    /// preset's, the address as the username.
    #[gpui::test]
    fn a_picked_provider_signs_in_with_its_servers(cx: &mut TestAppContext) {
        let f = at_the_list(cx, "ob-provider-pick");
        in_window(cx, f.window, |window, cx| {
            test_support::click_named(window, "Fastmail, App password needed", cx);
        });
        cx.run_until_parked();
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(f.view.read(cx).step(), Step::Yours));
        let a = f.db().accounts().unwrap();
        assert_eq!(a[0].imap.host, "imap.fastmail.com");
        assert_eq!(a[0].smtp.host, "smtp.fastmail.com");
    }

    /// Outlook.com can't be picked (no Microsoft sign-in yet); an address
    /// at it says so and keeps the list open.
    #[gpui::test]
    fn outlook_is_listed_but_cannot_be_picked(cx: &mut TestAppContext) {
        let f = at_the_list(cx, "ob-outlook");
        in_window(cx, f.window, |window, cx| {
            test_support::type_text(window, "outlook", cx)
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            test_support::click_named(
                window,
                "Outlook.com, Not supported yet: needs Microsoft sign-in",
                cx,
            );
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert_eq!(v.provider(), None);
            assert!(v.choosing());
        });
        // By its domain: the chip says why, nothing is looked up.
        f.act(cx, |v, w, cx| {
            v.back(w, cx);
        });
        f.set(cx, |v| v.email_input(), "rui@hotmail.com");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert_eq!(v.chip(), &flow::Chip::Unsupported(preset("outlook")));
            assert!(v.choosing() && v.provider().is_none());
        });
        in_window(cx, f.window, |window, _| {
            assert!(
                test_support::names(window)
                    .iter()
                    .any(|(_, n)| n.starts_with("Outlook.com isn't supported yet"))
            );
        });
    }

    /// Proton Mail fills manual setup with Bridge's local servers and says
    /// Bridge must run.
    #[gpui::test]
    fn proton_fills_in_bridge(cx: &mut TestAppContext) {
        let f = at_the_list(cx, "ob-bridge");
        in_window(cx, f.window, |window, cx| {
            test_support::type_text(window, "proton", cx)
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            test_support::click_named(window, "Proton Mail, Needs Proton Mail Bridge", cx);
        });
        cx.run_until_parked();
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert!(v.manual() && v.bridge());
            let typed = v.server_form().read(cx).fields(cx);
            assert_eq!(typed.imap_host, "127.0.0.1");
            assert_eq!(typed.imap_port, "1143");
            assert!(typed.imap_starttls);
            assert_eq!(typed.smtp_port, "1025");
        });
        in_window(cx, f.window, |window, _| {
            assert!(
                test_support::names(window)
                    .iter()
                    .any(|(_, n)| n.contains("Proton Mail Bridge must be running"))
            );
        });
        // Back: the list again, Bridge's note gone.
        f.act(cx, |v, w, cx| assert!(v.back(w, cx)));
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert!(!v.manual() && !v.bridge() && v.choosing());
        });
    }

    fn hosted_by_postale(_: &str) -> Outcome {
        use komorebi::mail::discover::Source;
        let i = komorebi::providers::index_of("postale").unwrap();
        Outcome {
            found: discover::preset_servers(i),
            sources: vec![Source::Mx],
            refused: Vec::new(),
            preset: Some(i),
        }
    }

    /// A custom domain whose MX records name a built-in provider: the chip
    /// says who hosts it, and Sign in uses the preset.
    #[gpui::test]
    fn a_custom_domain_hosted_by_a_preset_says_so(cx: &mut TestAppContext) {
        let svc = Services {
            verify: pass_all,
            lookup: hosted_by_postale,
            ..services::refusing()
        };
        let f = open(cx, "ob-hosted", svc, false);
        f.set(cx, |v| v.email_input(), "ana@example.pt");
        f.act(cx, |v, w, cx| v.continue_email(w, cx));
        cx.run_until_parked();
        let postale = preset("postale");
        cx.read(|cx| {
            let v = f.view.read(cx);
            assert_eq!(v.chip(), &flow::Chip::Hosted(postale));
            assert_eq!(v.provider(), Some(postale));
            assert!(!v.choosing());
        });
        in_window(cx, f.window, |window, _| {
            assert!(
                test_support::names(window)
                    .iter()
                    .any(|(_, n)| n.starts_with("Your mail is hosted by Postale.io"))
            );
        });
        f.set(cx, |v| v.password_input(), "secret");
        f.act(cx, |v, w, cx| v.submit(w, cx));
        cx.run_until_parked();
        let a = f.db().accounts().unwrap();
        assert_eq!(a[0].email, "ana@example.pt");
        assert_eq!(a[0].imap.host, "mail.postale.io");
    }

    /// Ports the onboarding half of `the_first_account_saves_the_version_silently`
    /// (the bar half is `views::updates::whats_new::tests::no_account_no_bar`):
    /// on a first run, the account the flow adds saves the What's new
    /// version, so the bar never shows for a new user.
    #[gpui::test]
    fn the_first_account_saves_the_version_silently(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        cx.update(|cx| cx.set_global(fakes()));
        let cache = TempCache::new("ob-first-version");
        let db = cache.open();
        assert_eq!(db.settings().unwrap().whats_new_version, None);
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        cx.run_until_parked();
        let flow = cx
            .read(|cx| main.read(cx).onboarding())
            .expect("a first run opens the flow");
        in_window(cx, window, |window, cx| {
            flow.update(cx, |v, cx| v.start(window, cx));
        });
        let email = cx.read(|cx| flow.read(cx).email_input().clone());
        in_window(cx, window, |window, cx| {
            email.update(cx, |s, cx| s.set_value("ana@purelymail.com", window, cx));
        });
        in_window(cx, window, |window, cx| {
            flow.update(cx, |v, cx| v.continue_email(window, cx));
        });
        let password = cx.read(|cx| flow.read(cx).password_input().clone());
        in_window(cx, window, |window, cx| {
            password.update(cx, |s, cx| s.set_value("secret", window, cx));
        });
        in_window(cx, window, |window, cx| {
            flow.update(cx, |v, cx| v.submit(window, cx));
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| flow.read(cx).added()).is_some());
        let db = cache.open();
        assert_eq!(
            db.settings().unwrap().whats_new_version,
            Some(crate::views::updates::whats_new::version())
        );
        assert!(!crate::views::updates::whats_new::due(&db));
        cx.read(|cx| assert_eq!(flow.read(cx).step(), Step::Yours));
    }
}
