//! Settings' view tests: ports of the Slint app's view-driven settings
//! tests (src/app/composer.rs; docs/GPUI-TEST-PORT.md phase 6). Each names
//! the Slint test it ports. The checks, the password store and the
//! certificate reads are fakes ([`services::Services`]); nothing here
//! touches the network or Windows Credential Manager.

// One test module, so the token lint and the catalog's literal check
// (both skip what a `#[cfg(test)]` is on) leave it out.
#[cfg(test)]
mod port {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;

    use gpui::{AnyWindowHandle, AppContext as _, Entity, Role, TestAppContext};
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use komorebi::db::{DraftRow, ThemeChoice};
    use komorebi::i18n::{Lang, LangSetting};
    use komorebi::mail::errors::{Attention, Class, Problem, Special, Step, StepError};
    use komorebi::mail::pin::Kind;

    use super::super::accounts::{AccountLine, AccountState};
    use super::super::check::StepState;
    use super::super::editor::{AccountEditor, Line};
    use super::super::services::{self, Services};
    use super::super::*;
    use crate::test_support::{self, TempCache, assert_named, in_window, names};

    struct Fixture {
        window: AnyWindowHandle,
        view: Entity<SettingsView>,
        events: Rc<RefCell<Vec<SettingsEvent>>>,
        _cache: TempCache,
        path: std::path::PathBuf,
        account: i64,
        email: String,
    }

    fn open(cx: &mut TestAppContext, name: &str, svc: Services) -> Fixture {
        test_support::init_app(cx);
        cx.update(|cx| {
            bind_keys(cx);
            cx.set_global(svc);
        });
        let (cache, db, _) = test_support::seeded(name, 3);
        let path = cache.path.clone();
        let account = db.accounts().unwrap()[0].clone();
        let (window, view) = test_support::open(cx, move |window, cx| {
            cx.new(|cx| SettingsView::new(Rc::new(db), window, cx))
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, e: &SettingsEvent, _| {
                sink.borrow_mut().push(e.clone());
            })
            .detach();
        });
        cx.run_until_parked();
        Fixture {
            window,
            view,
            events,
            _cache: cache,
            path,
            account: account.id,
            email: account.email,
        }
    }

    impl Fixture {
        fn db(&self) -> komorebi::db::Db {
            komorebi::db::Db::open(&self.path).unwrap()
        }

        /// The account's page, opened.
        fn edit(&self, cx: &mut TestAppContext) -> Entity<AccountEditor> {
            let id = self.account;
            let view = self.view.clone();
            in_window(cx, self.window, |window, cx| {
                view.update(cx, |v, cx| v.edit_account(id, window, cx));
            });
            cx.run_until_parked();
            cx.read(|cx| {
                self.view
                    .read(cx)
                    .editor()
                    .cloned()
                    .expect("the page shows")
            })
        }

        fn events(&self) -> Vec<SettingsEvent> {
            self.events.borrow().clone()
        }
    }

    fn pass_all(
        _: &komorebi::providers::Server,
        _: &komorebi::providers::Server,
        _: komorebi::mail::Pins,
        _: (&str, &str),
        _: &str,
        _: bool,
        report: &mut dyn FnMut(Step),
    ) -> Result<(), StepError> {
        Step::ALL.into_iter().for_each(report);
        Ok(())
    }

    fn reject(
        imap: &komorebi::providers::Server,
        _: &komorebi::providers::Server,
        _: komorebi::mail::Pins,
        logins: (&str, &str),
        _: &str,
        _: bool,
        report: &mut dyn FnMut(Step),
    ) -> Result<(), StepError> {
        report(Step::Resolve);
        report(Step::Connect);
        report(Step::Secure);
        Err(StepError {
            step: Step::ImapSignIn,
            class: Class::Auth,
            special: Special::None,
            host: imap.host.clone(),
            port: imap.port,
            login: logins.0.to_string(),
            source: anyhow::anyhow!("NO [AUTHENTICATIONFAILED]"),
        })
    }

    fn untrusted(
        imap: &komorebi::providers::Server,
        _: &komorebi::providers::Server,
        _: komorebi::mail::Pins,
        _: (&str, &str),
        _: &str,
        _: bool,
        report: &mut dyn FnMut(Step),
    ) -> Result<(), StepError> {
        report(Step::Resolve);
        report(Step::Connect);
        Err(StepError {
            step: Step::Secure,
            class: Class::TlsCert,
            special: Special::None,
            host: imap.host.clone(),
            port: imap.port,
            login: String::new(),
            source: anyhow::anyhow!("invalid peer certificate: UnknownIssuer"),
        })
    }

    fn with(verify: check::Verifier) -> Services {
        Services {
            verify,
            read_password: |_| Ok("saved".into()),
            ..services::refusing()
        }
    }

    fn type_into(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        state: &Entity<gpui_kit::component::input::InputState>,
        text: &str,
    ) {
        let state = state.clone();
        let text = text.to_string();
        in_window(cx, window, |window, cx| {
            state.update(cx, |s, cx| s.set_value(text, window, cx));
        });
    }

    /// Ports `removing_an_account_with_unsent_mail_asks_first`.
    #[gpui::test]
    fn removing_an_account_with_unsent_mail_asks_first(cx: &mut TestAppContext) {
        let f = open(cx, "settings-remove-unsent", services::refusing());
        f.db()
            .insert_draft(&DraftRow {
                account_id: f.account,
                to: "ana@example.org".into(),
                subject: "Plans".into(),
                ..Default::default()
            })
            .unwrap();
        let editor = f.edit(cx);
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.ask_remove(window, cx));
        });
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert_named(window, "Remove", Role::Button);
            window.click("dialog-cancel", cx);
        });
        // The question's sentence names the unsent mail.
        assert_eq!(account::unsent(&f.db(), Some(f.account)), 1);
        assert_eq!(
            text::q_remove_unsent(1),
            "Its 1 unsent draft or Outbox message will be lost."
        );
        assert_eq!(
            text::q_remove_title(&f.email),
            format!("Remove {}?", f.email)
        );
        // Cancel keeps everything.
        assert_eq!(f.db().accounts().unwrap().len(), 1);
        assert_eq!(f.db().count_unsent(f.account).unwrap(), 1);
    }

    /// Ports `removing_any_account_asks_first`, and the answer: the
    /// password is forgotten first, then the account goes.
    #[gpui::test]
    fn removing_any_account_asks_first(cx: &mut TestAppContext) {
        static FORGOT: Mutex<Vec<i64>> = Mutex::new(Vec::new());
        let svc = Services {
            forget_password: |id| {
                FORGOT.lock().unwrap().push(id);
                Ok(())
            },
            ..services::refusing()
        };
        let f = open(cx, "settings-remove-plain", svc);
        let editor = f.edit(cx);
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.ask_remove(window, cx));
        });
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            window.click("dialog-cancel", cx);
        });
        cx.run_until_parked();
        assert_eq!(f.db().accounts().unwrap().len(), 1);
        assert!(FORGOT.lock().unwrap().is_empty());
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.ask_remove(window, cx));
        });
        in_window(cx, f.window, |window, cx| {
            window.click("dialog-confirm", cx)
        });
        cx.run_until_parked();
        assert_eq!(*FORGOT.lock().unwrap(), vec![f.account]);
        assert!(f.db().accounts().unwrap().is_empty());
        assert!(
            f.events()
                .contains(&SettingsEvent::AccountRemoved(f.account))
        );
        cx.read(|cx| assert!(!f.view.read(cx).editing()));
    }

    /// Ports `a_rejected_password_sets_the_username_path_by_provider`: a
    /// rejected sign-in marks the password box and offers Set username.
    #[gpui::test]
    fn a_rejected_password_sets_the_username_path_by_provider(cx: &mut TestAppContext) {
        let f = open(cx, "settings-rejected", with(reject));
        let editor = f.edit(cx);
        let password = cx.read(|cx| editor.read(cx).password_input().clone());
        type_into(cx, f.window, &password, "wrong");
        cx.update(|cx| editor.update(cx, |e, cx| e.test(cx)));
        cx.run_until_parked();
        cx.read(|cx| {
            let e = editor.read(cx);
            assert!(e.password_rejected());
            assert!(matches!(e.line(), Some(Line::Check(_))));
            let steps = e.steps().unwrap();
            assert_eq!(steps[3], StepState::Failed);
            assert_eq!(steps[6], StepState::Skipped);
        });
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Set username", Role::Button);
        });
    }

    /// Ports `test_connection_shows_its_steps_and_saves_nothing`.
    #[gpui::test]
    fn test_connection_shows_its_steps_and_saves_nothing(cx: &mut TestAppContext) {
        let f = open(cx, "settings-test-connection", with(pass_all));
        let before = format!("{:?}", f.db().accounts().unwrap());
        let editor = f.edit(cx);
        let name = cx.read(|cx| editor.read(cx).name_input().clone());
        type_into(cx, f.window, &name, "Changed, not saved");
        cx.update(|cx| editor.update(cx, |e, cx| e.test(cx)));
        cx.run_until_parked();
        cx.read(|cx| {
            let e = editor.read(cx);
            assert_eq!(e.line(), Some(&Line::Passed));
            assert_eq!(e.steps().unwrap(), vec![StepState::Passed; 7]);
        });
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Find the incoming server, passed", Role::ListItem);
        });
        assert_eq!(format!("{:?}", f.db().accounts().unwrap()), before);
        assert!(f.events().is_empty(), "{:?}", f.events());
    }

    /// Ports `an_untrusted_certificate_offers_the_review_and_a_changed_one_does_not`
    /// (the view half; the rule is `pins::review_of`, row 51).
    #[gpui::test]
    fn an_untrusted_certificate_offers_the_review(cx: &mut TestAppContext) {
        let f = open(cx, "settings-untrusted", with(untrusted));
        let editor = f.edit(cx);
        cx.update(|cx| editor.update(cx, |e, cx| e.test(cx)));
        cx.run_until_parked();
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Review certificate", Role::Button);
        });
    }

    /// Ports `a_certificate_is_pinned_only_after_the_user_confirms_it`: the
    /// certificate read is only asked about; Trust pins it in the form, and
    /// only Save writes it.
    #[gpui::test]
    fn a_certificate_is_pinned_only_after_the_user_confirms_it(cx: &mut TestAppContext) {
        let svc = Services {
            probe: |_, _| Ok((b"not a real certificate".to_vec(), None)),
            ..with(pass_all)
        };
        let f = open(cx, "settings-pin-confirm", svc);
        let editor = f.edit(cx);
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.review_certificate(Kind::Imap, window, cx));
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx), "the question shows");
            assert!(editor.read(cx).review().is_some());
            assert!(!editor.read(cx).pins().is_saved(Kind::Imap));
            window.close_dialog(cx);
        });
        assert!(f.db().accounts().unwrap()[0].imap_pin.is_none());
        cx.update(|cx| editor.update(cx, |e, cx| e.trust(cx)));
        cx.read(|cx| {
            assert!(matches!(editor.read(cx).line(), Some(Line::Trusted(_))));
        });
        // Trusted in the form only.
        assert!(f.db().accounts().unwrap()[0].imap_pin.is_none());
        cx.update(|cx| editor.update(cx, |e, cx| e.save(cx)));
        cx.run_until_parked();
        assert!(f.db().accounts().unwrap()[0].imap_pin.is_some());
    }

    /// Ports `editing_an_account_keeps_its_mail_and_never_shows_the_password`.
    #[gpui::test]
    fn editing_an_account_keeps_its_mail_and_never_shows_the_password(cx: &mut TestAppContext) {
        let f = open(cx, "settings-edit", services::refusing());
        let folders = f.db().folders().unwrap().len();
        let editor = f.edit(cx);
        let (name, password) = cx.read(|cx| {
            let e = editor.read(cx);
            (e.name_input().clone(), e.password_input().clone())
        });
        cx.read(|cx| assert_eq!(password.read(cx).value().to_string(), ""));
        type_into(cx, f.window, &name, "Me Myself");
        // Nothing about the servers changed: saved at once, no check.
        cx.update(|cx| editor.update(cx, |e, cx| e.save(cx)));
        cx.run_until_parked();
        let a = f.db().accounts().unwrap()[0].clone();
        assert_eq!(a.name, "Me Myself");
        assert_eq!(a.email, f.email);
        assert_eq!(f.db().folders().unwrap().len(), folders);
        assert!(
            f.events()
                .iter()
                .any(|e| matches!(e, SettingsEvent::AccountSaved { id, .. } if *id == f.account))
        );
        cx.read(|cx| assert!(!f.view.read(cx).editing(), "Save closes the page"));
    }

    /// Ports `a_changed_certificate_offers_review_certificate`.
    #[gpui::test]
    fn a_changed_certificate_offers_review_certificate(cx: &mut TestAppContext) {
        let f = open(cx, "settings-changed-cert", services::refusing());
        let id = f.account;
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_account_state(
                    id,
                    AccountLine {
                        state: Some(AccountState::Failed(Problem {
                            class: Class::TlsCert,
                            detail: String::new(),
                            changed: Some("imap.example.invalid".into()),
                        })),
                        attention: None,
                        download: None,
                    },
                    cx,
                )
            })
        });
        in_window(cx, f.window, |window, _| {
            assert_named(
                window,
                &format!("Review the certificate of {}", f.email),
                Role::Button,
            );
        });
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_account_state(
                    id,
                    AccountLine {
                        attention: Some(Attention::SignIn),
                        ..AccountLine::default()
                    },
                    cx,
                )
            })
        });
        in_window(cx, f.window, |window, _| {
            assert_named(
                window,
                &format!("Update password for {}", f.email),
                Role::Button,
            );
        });
    }

    /// Ports `switched_off_accounts_spawn_no_workers` (the Settings half: the
    /// switch is saved and the shell told; the workers are the shell's).
    #[gpui::test]
    fn switched_off_accounts_spawn_no_workers(cx: &mut TestAppContext) {
        let f = open(cx, "settings-switch-off", services::refusing());
        let editor = f.edit(cx);
        in_window(cx, f.window, |window, cx| {
            let name = format!("Sync {}", f.email);
            assert_named(window, &name, Role::Switch);
            crate::test_support::click_named(window, &name, cx);
        });
        cx.run_until_parked();
        assert!(!f.db().accounts().unwrap()[0].enabled);
        assert!(f.events().contains(&SettingsEvent::AccountEnabled {
            id: f.account,
            on: false
        }));
        let _ = editor;
    }

    /// Ports `download_again_keeps_the_account_drafts_and_settings` (the
    /// Settings half: asked first; the shell resets and syncs).
    #[gpui::test]
    fn download_again_keeps_the_account_drafts_and_settings(cx: &mut TestAppContext) {
        let f = open(cx, "settings-redownload", services::refusing());
        let editor = f.edit(cx);
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.ask_redownload(window, cx));
        });
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            window.click("dialog-cancel", cx);
        });
        cx.run_until_parked();
        assert!(
            !f.events()
                .iter()
                .any(|e| matches!(e, SettingsEvent::Redownload(_)))
        );
        // The answer goes to the shell; the account stays as it is.
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.ask_redownload(window, cx));
        });
        in_window(cx, f.window, |window, cx| {
            window.click("dialog-confirm", cx)
        });
        cx.run_until_parked();
        assert!(f.events().contains(&SettingsEvent::Redownload(f.account)));
        assert_eq!(f.db().accounts().unwrap().len(), 1);
    }

    /// Ports `the_language_switches_live_and_is_saved`.
    #[gpui::test]
    fn the_language_switches_live_and_is_saved(cx: &mut TestAppContext) {
        let f = open(cx, "settings-language", services::refusing());
        in_window(cx, f.window, |window, cx| {
            f.view
                .update(cx, |v, cx| v.show(Section::General, window, cx));
        });
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_language(LangSetting::PortuguesePortugal, cx)
            })
        });
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(crate::i18n::lang(cx), Lang::PtPt));
        assert_eq!(
            f.db().settings().unwrap().language,
            LangSetting::PortuguesePortugal
        );
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Contas", Role::Tab);
        });
        test_support::set_lang(cx, Lang::En);
    }

    /// Ports `general_choices_apply_and_are_saved`.
    #[gpui::test]
    fn general_choices_apply_and_are_saved(cx: &mut TestAppContext) {
        let f = open(cx, "settings-general", services::refusing());
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_setting(Setting::CheckInterval(300), cx);
                v.set_setting(Setting::UndoSend(20), cx);
                v.set_setting(Setting::MarkRead(-1), cx);
                v.set_setting(Setting::NotifyDetail(false), cx);
                v.set_theme(ThemeChoice::Dark, cx);
            })
        });
        let s = f.db().settings().unwrap();
        assert_eq!(s.check_interval_secs, 300);
        assert_eq!(s.undo_send_secs, 20);
        assert!(!s.notify_detail);
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(
            f.events()
                .contains(&SettingsEvent::Changed(Setting::CheckInterval(300)))
        );
        assert!(
            f.events()
                .contains(&SettingsEvent::Changed(Setting::UndoSend(20)))
        );
        in_window(cx, f.window, |window, cx| {
            f.view
                .update(cx, |v, cx| v.show(Section::General, window, cx));
        });
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Dark", Role::RadioButton);
        });
        test_support::set_theme(cx, ThemeChoice::Light);
    }

    /// Ports `settings_open_with_ctrl_comma_and_escape_steps_back` (the
    /// Settings half; Ctrl+, is the shell's): Esc closes the account page
    /// first, then asks the shell to close Settings.
    #[gpui::test]
    fn settings_open_with_ctrl_comma_and_escape_steps_back(cx: &mut TestAppContext) {
        let f = open(cx, "settings-escape", services::refusing());
        let _ = f.edit(cx);
        in_window(cx, f.window, |window, cx| {
            let focus = f.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            test_support::dispatch(window, &Close, cx);
        });
        cx.read(|cx| assert!(!f.view.read(cx).editing()));
        assert!(!f.events().contains(&SettingsEvent::Close));
        in_window(cx, f.window, |window, cx| {
            test_support::dispatch(window, &Close, cx);
        });
        assert!(f.events().contains(&SettingsEvent::Close));
    }

    /// Ports `settings_show_an_accounts_download`.
    #[gpui::test]
    fn settings_show_an_accounts_download(cx: &mut TestAppContext) {
        let f = open(cx, "settings-download", services::refusing());
        let id = f.account;
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_account_state(
                    id,
                    AccountLine {
                        download: Some((10, 200)),
                        ..AccountLine::default()
                    },
                    cx,
                )
            })
        });
        cx.read(|cx| {
            assert_eq!(
                f.view.read(cx).account_lines()[&id].download,
                Some((10, 200))
            )
        });
        in_window(cx, f.window, |window, _| {
            assert_named(window, &format!("Edit {}", f.email), Role::Button);
        });
    }

    /// Ports `remove_all_data_forgets_every_password_then_closes_the_cache`:
    /// every password goes first; then the shell closes the cache and
    /// removes the files (`wipe::remove_files`, rows 255 to 258).
    #[gpui::test]
    fn remove_all_data_forgets_every_password_then_closes_the_cache(cx: &mut TestAppContext) {
        static FORGOT: Mutex<Vec<i64>> = Mutex::new(Vec::new());
        let svc = Services {
            forget_password: |id| {
                FORGOT.lock().unwrap().push(id);
                Ok(())
            },
            ..services::refusing()
        };
        let f = open(cx, "settings-wipe", svc);
        cx.update(|cx| f.view.update(cx, |v, cx| v.wipe_now(cx)));
        assert_eq!(*FORGOT.lock().unwrap(), vec![f.account]);
        assert_eq!(f.events(), vec![SettingsEvent::WipeAndQuit]);
        // A password that can't be deleted stops it, with the reason.
        let refuse = Services {
            forget_password: |_| anyhow::bail!("locked"),
            ..services::refusing()
        };
        cx.update(|cx| cx.set_global(refuse));
        f.events.borrow_mut().clear();
        cx.update(|cx| f.view.update(cx, |v, cx| v.wipe_now(cx)));
        assert!(f.events().is_empty());
        cx.read(|cx| {
            assert!(matches!(
                f.view.read(cx).message,
                Some(Message::NotReset(_))
            ))
        });
    }

    /// The sections show, each a tab named by its section, in English and
    /// Portuguese.
    #[gpui::test]
    fn every_section_renders(cx: &mut TestAppContext) {
        let f = open(cx, "settings-sections", services::refusing());
        for section in [
            Section::Accounts,
            Section::Signatures,
            Section::General,
            Section::Privacy,
            Section::About,
        ] {
            in_window(cx, f.window, |window, cx| {
                f.view.update(cx, |v, cx| v.show(section, window, cx));
            });
            in_window(cx, f.window, |window, _| {
                assert_named(window, &section.name(), Role::Tab);
            });
        }
        in_window(cx, f.window, |window, _| {
            let all = names(window);
            assert!(
                all.contains(&(Some(Role::Button), "Copy the data folder's path".into())),
                "{all:?}"
            );
        });
    }

    /// One element named `name` with `role`, on screen or scrolled out of
    /// view (the account page is longer than the test window).
    fn has(window: &gpui::Window, name: &str, role: Role) -> bool {
        names(window)
            .iter()
            .filter(|(r, n)| n == name && *r == Some(role))
            .count()
            == 1
    }

    fn found_settings(_: &str) -> komorebi::mail::discover::Outcome {
        use komorebi::mail::discover::{Discovered, Found, Outcome, Source};
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
                smtp: Some(found("smtp.example.org", 587, Security::StartTls)),
            },
            sources: vec![Source::Autoconfig],
            refused: Vec::new(),
            preset: None,
        }
    }

    /// The server fields with both hosts empty (what a lookup may fill).
    fn clear_hosts(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        editor: &Entity<AccountEditor>,
    ) {
        let servers = cx.read(|cx| editor.read(cx).server_form().clone());
        in_window(cx, window, |window, cx| {
            servers.update(cx, |s, cx| {
                let mut f = s.fields(cx);
                f.imap_host.clear();
                f.smtp_host.clear();
                s.set(&f, window, cx);
            });
        });
    }

    /// Server settings start folded on an account with no saved
    /// certificate; Find settings fills the empty servers and unfolds them;
    /// a form edited while the lookup ran is left alone and says so (the
    /// Slint page's `FindSettingsRow` and its `found-seq` guard).
    #[gpui::test]
    fn find_settings_fills_the_servers_unless_the_form_changed(cx: &mut TestAppContext) {
        let svc = Services {
            lookup: found_settings,
            ..services::refusing()
        };
        let f = open(cx, "settings-find", svc);
        let editor = f.edit(cx);
        cx.read(|cx| {
            let e = editor.read(cx);
            assert!(!e.servers_open(), "folded: nothing pinned, SMTP signs in");
            assert!(e.can_find(), "not a preset's servers");
        });
        in_window(cx, f.window, |window, _| {
            assert!(has(window, "Server settings", Role::Button));
            assert!(!has(window, "Find settings", Role::Button), "folded");
        });
        cx.update(|cx| editor.update(cx, |e, cx| e.set_servers_open(true, cx)));
        in_window(cx, f.window, |window, _| {
            assert!(has(window, "Find settings", Role::Button));
        });
        clear_hosts(cx, f.window, &editor);
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| e.find_settings(window, cx));
        });
        cx.run_until_parked();
        let servers = cx.read(|cx| editor.read(cx).server_form().clone());
        cx.read(|cx| {
            let typed = servers.read(cx).fields(cx);
            assert_eq!(typed.imap_host, "imap.example.org");
            assert_eq!(
                (typed.smtp_host.as_str(), typed.smtp_starttls),
                ("smtp.example.org", true)
            );
            let e = editor.read(cx);
            assert!(!e.finding());
            assert!(e.servers_open());
            assert!(!e.find_note().is_empty());
        });
        // Typed into while it looked: nothing is filled, the note says why.
        clear_hosts(cx, f.window, &editor);
        let from = cx.read(|cx| servers.read(cx).fields(cx));
        let host = cx.read(|cx| servers.read(cx).imap_host.clone());
        type_into(cx, f.window, &host, "mail.mine.example");
        in_window(cx, f.window, |window, cx| {
            editor.update(cx, |e, cx| {
                e.finish_find(from, found_settings(""), window, cx)
            });
        });
        cx.read(|cx| {
            assert_eq!(servers.read(cx).fields(cx).imap_host, "mail.mine.example");
            assert_eq!(
                editor.read(cx).find_note(),
                "Not filled in: the form changed while Komorebi looked."
            );
        });
    }

    /// "This server does not need a sign-in for sending": the check box
    /// hides Same sign-in, shows its note, and is saved (after the check, as
    /// how the server is reached changed); an account that has it opens with
    /// Server settings unfolded.
    #[gpui::test]
    fn a_relay_that_needs_no_sign_in_can_be_set_and_saved(cx: &mut TestAppContext) {
        let f = open(cx, "settings-no-auth", with(pass_all));
        let editor = f.edit(cx);
        cx.update(|cx| editor.update(cx, |e, cx| e.set_servers_open(true, cx)));
        let label = "This server does not need a sign-in for sending";
        in_window(cx, f.window, |window, _| {
            assert!(has(window, label, Role::CheckBox));
            assert!(has(window, "Same sign-in as incoming", Role::CheckBox));
        });
        // What the check box does (it is below the test window's fold).
        let servers = cx.read(|cx| editor.read(cx).server_form().clone());
        cx.update(|cx| {
            servers.update(cx, |s, cx| {
                s.set_no_auth(true, cx);
                cx.emit(super::super::servers::ServerEvent::Edited);
            })
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, _| {
            let all = names(window);
            assert!(
                !all.iter().any(|(_, n)| n == "Same sign-in as incoming"),
                "{all:?}"
            );
        });
        cx.update(|cx| editor.update(cx, |e, cx| e.save(cx)));
        cx.run_until_parked();
        assert!(f.db().accounts().unwrap()[0].smtp_no_auth);
        // Opened again: Server settings show, the box is ticked.
        let editor = f.edit(cx);
        cx.read(|cx| {
            let e = editor.read(cx);
            assert!(e.servers_open());
            assert!(e.server_form().read(cx).no_auth());
        });
    }

    /// Move up and Move down (the account page's "..." menu, with two
    /// accounts or more) set the accounts' order and tell the shell.
    #[gpui::test]
    fn move_up_and_down_set_the_account_order(cx: &mut TestAppContext) {
        let f = open(cx, "settings-move", services::refusing());
        let other = f.db().accounts().unwrap()[0].clone();
        let second = f
            .db()
            .add_account(&komorebi::db::NewAccount {
                email: "two@example.org".into(),
                name: "Two".into(),
                imap: other.imap.clone(),
                smtp: other.smtp.clone(),
                save_sent: true,
                imap_user: String::new(),
                smtp_user: String::new(),
                smtp_no_auth: false,
            })
            .unwrap();
        let editor = f.edit(cx);
        cx.read(|cx| assert_eq!(editor.read(cx).place(), (0, 2)));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "More options for this account", Role::Button);
        });
        // The first can't go up.
        cx.update(|cx| editor.update(cx, |e, cx| e.move_account(true, cx)));
        assert!(!f.events().contains(&SettingsEvent::AccountsChanged));
        cx.update(|cx| editor.update(cx, |e, cx| e.move_account(false, cx)));
        let order: Vec<i64> = f.db().accounts().unwrap().iter().map(|a| a.id).collect();
        assert_eq!(order, vec![second, f.account]);
        cx.read(|cx| assert_eq!(editor.read(cx).place(), (1, 2)));
        assert!(f.events().contains(&SettingsEvent::AccountsChanged));
    }

    /// Enter in Name saves, as the Save button (the Slint page's
    /// `submit(false)`).
    #[gpui::test]
    fn enter_in_a_field_saves_the_account(cx: &mut TestAppContext) {
        let f = open(cx, "settings-enter", services::refusing());
        let editor = f.edit(cx);
        let name = cx.read(|cx| editor.read(cx).name_input().clone());
        type_into(cx, f.window, &name, "Entered");
        in_window(cx, f.window, |window, cx| {
            name.update(cx, |s, cx| s.focus(window, cx));
        });
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["enter"], cx);
        });
        cx.run_until_parked();
        assert_eq!(f.db().accounts().unwrap()[0].name, "Entered");
        assert!(
            f.events()
                .iter()
                .any(|e| matches!(e, SettingsEvent::AccountSaved { .. }))
        );
    }

    /// Ports `saved_pins_follow_the_servers_they_were_confirmed_for` (the
    /// view half; the rule is `pins::tests::a_pin_goes_when_host_port_or_security_change`):
    /// a saved certificate stays while the server is the same, goes from
    /// the form at the first edit of its host, and that Save writes.
    #[gpui::test]
    fn saved_pins_follow_the_servers_they_were_confirmed_for(cx: &mut TestAppContext) {
        let f = open(cx, "settings-pins-follow", with(pass_all));
        let mut a = f.db().accounts().unwrap()[0].clone();
        a.imap_pin = Some(b"a saved certificate".to_vec());
        f.db().update_account(&a).unwrap();
        let editor = f.edit(cx);
        cx.read(|cx| {
            let e = editor.read(cx);
            assert!(e.pins().is_saved(Kind::Imap));
            assert!(
                e.servers_open(),
                "a saved certificate unfolds Server settings"
            );
        });
        // A name edit keeps it.
        let name = cx.read(|cx| editor.read(cx).name_input().clone());
        type_into(cx, f.window, &name, "Other");
        cx.read(|cx| assert!(editor.read(cx).pins().is_saved(Kind::Imap)));
        // The incoming host typed into: gone from the form at once (a typed
        // change is the form's `Edited`).
        let servers = cx.read(|cx| editor.read(cx).server_form().clone());
        let host = cx.read(|cx| servers.read(cx).imap_host.clone());
        let edited = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                servers.update(cx, |_, cx| {
                    cx.emit(super::super::servers::ServerEvent::Edited)
                })
            });
            cx.run_until_parked();
        };
        type_into(cx, f.window, &host, "other.example.org");
        edited(cx);
        cx.read(|cx| assert!(!editor.read(cx).pins().is_saved(Kind::Imap)));
        // Typing the old host back does not bring it back.
        type_into(cx, f.window, &host, &a.imap.host);
        edited(cx);
        cx.read(|cx| assert!(!editor.read(cx).pins().is_saved(Kind::Imap)));
        cx.update(|cx| editor.update(cx, |e, cx| e.save(cx)));
        cx.run_until_parked();
        assert_eq!(f.db().accounts().unwrap()[0].imap_pin, None);
    }

    /// Ports `a_pin_round_trips_with_a_confirmed_edit_and_goes_with_the_account`
    /// (the half `a_certificate_is_pinned_only_after_the_user_confirms_it`
    /// leaves): a pin saved with the account goes when the account is
    /// removed.
    #[gpui::test]
    fn a_pin_goes_with_the_account(cx: &mut TestAppContext) {
        let f = open(cx, "settings-pin-goes", services::refusing());
        let mut a = f.db().accounts().unwrap()[0].clone();
        a.imap_pin = Some(b"imap certificate".to_vec());
        a.smtp_pin = Some(b"smtp certificate".to_vec());
        f.db().update_account(&a).unwrap();
        let saved = f.db().accounts().unwrap()[0].clone();
        assert_eq!(
            (saved.imap_pin.as_deref(), saved.smtp_pin.as_deref()),
            (
                Some(&b"imap certificate"[..]),
                Some(&b"smtp certificate"[..])
            )
        );
        let editor = f.edit(cx);
        cx.update(|cx| editor.update(cx, |e, cx| e.remove_now(cx)));
        assert!(
            f.events()
                .contains(&SettingsEvent::AccountRemoved(f.account))
        );
        assert!(f.db().accounts().unwrap().is_empty());
    }

    /// Ports the rest of `a_rejected_password_sets_the_username_path_by_provider`:
    /// the rejection's sentence by provider (`text::rejected`): another
    /// server's ends with the username advice; a preset on a custom domain
    /// gives the app password advice, then a pointer back to the list.
    #[test]
    fn a_rejected_password_is_worded_by_provider() {
        komorebi_i18n::with_lang(Lang::En, || {
            let host = "mail.example.net";
            let login = "ana@example.net";
            let other = text::rejected(None, login, host, login);
            assert!(other.ends_with("set the username."), "{other}");
            let preset = text::rejected(
                komorebi::providers::index_of("purelymail"),
                login,
                host,
                login,
            );
            assert_eq!(
                preset,
                "mail.example.net rejected the password for ana@example.net. If sign-in uses two steps, \
                 create an app password. Is your mail really at Purelymail? Go back and choose another \
                 provider."
            );
        });
    }

    /// General > Compose > Default font: the font, size and colour are
    /// saved as the Slint app saves them, and the composer is told.
    #[gpui::test]
    fn the_default_font_is_saved_and_told(cx: &mut TestAppContext) {
        use komorebi::mail::rich::{ColorId, FontId, SizePt};
        let f = open(cx, "settings-font", services::refusing());
        in_window(cx, f.window, |window, cx| {
            f.view
                .update(cx, |v, cx| v.show(Section::General, window, cx));
        });
        in_window(cx, f.window, |window, _| {
            assert!(has(window, "Default font", Role::Group));
            assert!(has(window, "Font size: 11", Role::Button));
        });
        let size = SizePt::new(14).unwrap();
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_compose_font(FontId::Georgia, size, Some(ColorId::Blue), cx)
            })
        });
        let s = f.db().settings().unwrap();
        assert_eq!(
            (s.compose_font, s.compose_size, s.compose_color),
            (FontId::Georgia, size, Some(ColorId::Blue))
        );
        assert!(
            f.events()
                .contains(&SettingsEvent::Changed(Setting::ComposeRich(
                    s.compose_rich
                )))
        );
        // Automatic again.
        cx.update(|cx| {
            f.view.update(cx, |v, cx| {
                v.set_compose_font(FontId::Georgia, size, None, cx)
            })
        });
        assert_eq!(f.db().settings().unwrap().compose_color, None);
    }

    /// The account card: a first download shows its bar; a failed sync is
    /// said in full with the warning glyph, not cut short in the quiet
    /// colour.
    #[gpui::test]
    fn an_account_card_shows_its_download_and_its_warning(cx: &mut TestAppContext) {
        let f = open(cx, "settings-card-lines", services::refusing());
        let id = f.account;
        let set = |cx: &mut TestAppContext, line: AccountLine| {
            cx.update(|cx| f.view.update(cx, |v, cx| v.set_account_state(id, line, cx)));
        };
        set(
            cx,
            AccountLine {
                download: Some((10, 200)),
                ..AccountLine::default()
            },
        );
        let bar = format!("Downloading the mail of {}", f.email);
        in_window(cx, f.window, |window, _| {
            assert_named(window, &bar, Role::ProgressIndicator);
        });
        let failed = AccountLine {
            state: Some(AccountState::Unreachable),
            ..AccountLine::default()
        };
        assert!(accounts::is_warning(&failed));
        assert!(!accounts::is_warning(&AccountLine::default()));
        set(cx, failed);
        in_window(cx, f.window, |window, _| {
            assert!(!names(window).iter().any(|(_, n)| *n == bar));
            // The warning line, with its glyph, says the whole line.
            let warnings: Vec<String> = names(window)
                .into_iter()
                .filter(|(r, n)| *r == Some(Role::Label) && n.starts_with("Warning: "))
                .map(|(_, n)| n)
                .collect();
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].contains(&f.email), "{warnings:?}");
        });
        // A quiet line is no warning.
        set(cx, AccountLine::default());
        in_window(cx, f.window, |window, _| {
            assert!(
                !names(window)
                    .iter()
                    .any(|(_, n)| n.starts_with("Warning: "))
            );
        });
    }

    /// The segmented control is one Tab stop: the arrows move the choice
    /// (Fluent and UI Automation radio groups).
    #[gpui::test]
    fn the_theme_choice_moves_with_the_arrows(cx: &mut TestAppContext) {
        let f = open(cx, "settings-seg-keys", services::refusing());
        in_window(cx, f.window, |window, cx| {
            f.view
                .update(cx, |v, cx| v.show(Section::General, window, cx));
        });
        let before = f.db().settings().unwrap().theme;
        in_window(cx, f.window, |window, cx| {
            crate::test_support::click_named(window, "Light", cx);
        });
        cx.run_until_parked();
        assert_eq!(f.db().settings().unwrap().theme, ThemeChoice::Light);
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["right"], cx);
        });
        cx.run_until_parked();
        assert_eq!(f.db().settings().unwrap().theme, ThemeChoice::Dark);
        in_window(cx, f.window, |window, cx| {
            test_support::press(window, &["left", "left"], cx);
        });
        cx.run_until_parked();
        assert_eq!(f.db().settings().unwrap().theme, ThemeChoice::System);
        let _ = before;
        test_support::set_theme(cx, ThemeChoice::Light);
    }

    /// A signature's page: the Slint app's reduced bar (Strikethrough, the
    /// colour, the indents, Link, Clear formatting); Link's row refuses
    /// what is not a web or e-mail address and Esc closes the row first.
    #[gpui::test]
    fn a_signature_takes_a_link(cx: &mut TestAppContext) {
        let f = open(cx, "settings-sig-link", services::refusing());
        in_window(cx, f.window, |window, cx| {
            f.view
                .update(cx, |v, cx| v.show(Section::Signatures, window, cx));
        });
        let sigs = cx.read(|cx| f.view.read(cx).signatures().clone());
        in_window(cx, f.window, |window, cx| {
            sigs.update(cx, |s, cx| s.open(None, window, cx));
        });
        in_window(cx, f.window, |window, _| {
            for name in [
                "Strikethrough (Ctrl+Shift+X)",
                "Font colour",
                "Increase indent",
                "Decrease indent",
                "Link (Ctrl+K)",
                "Clear formatting",
            ] {
                assert_named(window, name, Role::Button);
            }
        });
        // Ctrl+K from the focused text reaches Settings' Link binding.
        let body = cx.read(|cx| sigs.read(cx).body().cloned().unwrap());
        in_window(cx, f.window, |window, cx| {
            let focus = body.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            test_support::press(window, &["ctrl-k"], cx);
        });
        cx.run_until_parked();
        cx.read(|cx| assert!(sigs.read(cx).link_open(), "Ctrl+K opens the link row"));
        let link = cx.read(|cx| sigs.read(cx).link_input().cloned().unwrap());
        type_into(cx, f.window, &link, "not a link");
        in_window(cx, f.window, |window, cx| {
            sigs.update(cx, |s, cx| s.insert_link(window, cx));
        });
        cx.read(|cx| {
            assert!(sigs.read(cx).link_open());
            assert!(sigs.read(cx).link_error());
        });
        // Esc: the row closes, the page stays.
        in_window(cx, f.window, |window, cx| {
            let focus = f.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            test_support::dispatch(window, &Close, cx);
        });
        cx.read(|cx| {
            assert!(!sigs.read(cx).link_open());
            assert!(sigs.read(cx).editing());
        });
        // A web address goes on the text.
        cx.update(|cx| {
            body.update(cx, |b, cx| {
                let ed = komorebi::mail::rich::edit::Editor::new(
                    komorebi::compose::rich::doc_from_text(
                        "Ana",
                        komorebi::compose::ComposeDefaults::default().doc(),
                    ),
                );
                b.set_doc(Some(ed), 0, cx);
                b.set_selection(0, 3, cx);
            })
        });
        in_window(cx, f.window, |window, cx| {
            sigs.update(cx, |s, cx| s.open_link(window, cx));
        });
        type_into(cx, f.window, &link, "https://example.org");
        in_window(cx, f.window, |window, cx| {
            sigs.update(cx, |s, cx| s.insert_link(window, cx));
        });
        cx.read(|cx| assert!(!sigs.read(cx).link_open()));
        cx.update(|cx| body.update(cx, |b, cx| b.set_selection(1, 2, cx)));
        cx.read(|cx| {
            let fmt = body.read(cx).format().unwrap();
            assert!(fmt.link.is_some(), "{fmt:?}");
        });
    }

    /// F1 and Ctrl+/ open the Keyboard shortcuts dialog (the sheet lists
    /// them).
    #[gpui::test]
    fn f1_opens_the_keyboard_shortcuts(cx: &mut TestAppContext) {
        let f = open(cx, "settings-f1", services::refusing());
        in_window(cx, f.window, |window, cx| {
            let focus = f.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            test_support::press(window, &["f1"], cx);
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx), "F1");
            window.close_dialog(cx);
        });
        in_window(cx, f.window, |window, cx| {
            let focus = f.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            test_support::press(window, &["ctrl-/"], cx);
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx), "Ctrl+/");
            // F1 over an open dialog stacks no second one.
            test_support::press(window, &["f1"], cx);
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, cx| {
            window.close_dialog(cx);
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx), "one dialog at a time");
        });
    }
}
