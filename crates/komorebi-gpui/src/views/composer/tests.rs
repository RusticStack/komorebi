//! The composer's view and window tests: ports of the Slint app's
//! view-driven composer tests (src/app/composer.rs, src/app/composer/tests/,
//! docs/GPUI-TEST-PORT.md rows 74 to 130 and 274 to 290). Each names the
//! Slint test it ports. The model's own tests are the library's
//! (`komorebi::compose`).

// One test module, so the token lint and the catalog's literal check
// (both skip what a `#[cfg(test)]` is on) leave it out.
#[cfg(test)]
mod port {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui::{AnyWindowHandle, Role, TestAppContext};
    use gpui_kit::test::TestWindowExt as _;
    use komorebi::compose::{ComposeDefaults, Guard, Kind};
    use komorebi::db::Importance;
    use komorebi::mail::mime;

    use super::super::*;
    use crate::test_support::{
        self, TempCache, assert_named, click_named, in_window, names, press, type_text,
    };

    /// The composer over a seeded throwaway cache, in a headless window, and
    /// what it tells the shell.
    struct Fixture {
        window: AnyWindowHandle,
        view: Entity<ComposerView>,
        events: Rc<RefCell<Vec<ComposerEvent>>>,
        _cache: TempCache,
        account: i64,
        inbox: i64,
        path: std::path::PathBuf,
    }

    fn open(cx: &mut TestAppContext, name: &str, rich: bool) -> Fixture {
        test_support::init_app(cx);
        let (cache, db, seeded) = test_support::seeded(name, 20);
        let path = cache.path.clone();
        let defaults = ComposeDefaults {
            rich,
            ..ComposeDefaults::default()
        };
        let (window, view) = test_support::open(cx, move |window, cx| {
            cx.new(|cx| {
                let mut v = ComposerView::new(Rc::new(db), window, cx);
                v.set_defaults(defaults);
                v.set_undo_delay(Duration::ZERO);
                v
            })
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, e: &ComposerEvent, _| {
                sink.borrow_mut().push(e.clone());
            })
            .detach();
        });
        Fixture {
            window,
            view,
            events,
            _cache: cache,
            account: seeded.account,
            inbox: seeded.inbox,
            path,
        }
    }

    impl Fixture {
        fn act<R>(
            &self,
            cx: &mut TestAppContext,
            f: impl FnOnce(&mut ComposerView, &mut Window, &mut Context<ComposerView>) -> R,
        ) -> R {
            let view = self.view.clone();
            let r = in_window(cx, self.window, move |window, cx| {
                view.update(cx, |v, cx| f(v, window, cx))
            });
            cx.run_until_parked();
            r
        }

        fn keys(&self, cx: &mut TestAppContext, keys: &[&str]) {
            in_window(cx, self.window, |window, cx| press(window, keys, cx));
            cx.run_until_parked();
        }

        fn typed(&self, cx: &mut TestAppContext, text: &str) {
            in_window(cx, self.window, |window, cx| type_text(window, text, cx));
            cx.run_until_parked();
        }

        fn db(&self) -> Db {
            Db::open(&self.path).expect("open the test cache")
        }

        fn saw(&self, f: impl Fn(&ComposerEvent) -> bool) -> bool {
            self.events.borrow().iter().any(f)
        }

        fn status(&self, cx: &mut TestAppContext) -> Option<Status> {
            self.view.read_with(cx, |v, _| v.status().cloned())
        }

        /// The newest seeded inbox message with its parsed body.
        fn message(&self) -> (db::Message, mime::Body) {
            let db = self.db();
            let id = db.message_ids(&[self.inbox], "").unwrap()[0];
            let m = db.message(id).unwrap().unwrap();
            let body = mime::parse_body(m.raw.as_deref().expect("seeded with a body"));
            (m, body)
        }

        fn draft_text(&self, cx: &mut TestAppContext) -> String {
            self.view.read_with(cx, |v, cx| {
                if let Some(ed) = v.body.read(cx).editor() {
                    return ed.text().to_string();
                }
                v.drafts
                    .active()
                    .map(|d| d.body.clone())
                    .unwrap_or_default()
            })
        }
    }

    /// Port of `the_composer_view_follows_its_draft`: a new message shows its
    /// fields by name and role, To has the focus, and the title is the kind.
    #[gpui_kit::test]
    fn the_composer_view_follows_its_draft(cx: &mut TestAppContext) {
        let f = open(cx, "follows", true);
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        assert!(f.view.read_with(cx, |v, _| v.composing()));
        in_window(cx, f.window, |window, _| {
            for name in ["To", "Subject"] {
                assert_named(window, name, Role::TextInput);
            }
            assert_named(window, "Message body", Role::MultilineTextInput);
            assert_named(window, "Send", Role::Button);
            assert_named(window, "Attach", Role::Button);
            assert_named(window, "Discard", Role::Button);
        });
        f.typed(cx, "ana@example.org");
        let to = f
            .view
            .read_with(cx, |v, _| v.drafts.active().unwrap().to.clone());
        assert_eq!(to, "ana@example.org", "To had the focus");
        let kind = f.view.read_with(cx, |v, _| v.drafts.active().unwrap().kind);
        assert_eq!(text::title(kind), "New message");
    }

    /// Port of `send_is_always_on_and_the_field_says_why`: Send with no
    /// recipient says so under To (an alert) and puts the focus there.
    #[gpui_kit::test]
    fn send_is_always_on_and_the_field_says_why(cx: &mut TestAppContext) {
        let f = open(cx, "send-why", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        in_window(cx, f.window, |window, cx| click_named(window, "Send", cx));
        cx.run_until_parked();
        // Said by the field, not the status line (as in the Slint app).
        assert_eq!(f.status(cx), None);
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Add a recipient to send.", Role::Alert);
        });
        // Typing an address clears it.
        f.typed(cx, "a");
        in_window(cx, f.window, |window, _| {
            assert!(
                !names(window)
                    .iter()
                    .any(|(_, n)| n == "Add a recipient to send.")
            );
        });
    }

    /// Port of `composer_keys_and_inline_validation`: an address is checked
    /// on leaving its field, the error follows the typing once shown, and
    /// Ctrl+Shift+C shows Cc and Bcc.
    #[gpui_kit::test]
    fn composer_keys_and_inline_validation(cx: &mut TestAppContext) {
        let f = open(cx, "validation", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "j@");
        f.act(cx, |v, w, cx| v.focus_field(Field::Subject, w, cx));
        // The blur reaches the view with the next frame.
        in_window(cx, f.window, |_, _| {});
        cx.run_until_parked();

        in_window(cx, f.window, |window, _| {
            assert_named(window, "“j@” is not a valid address", Role::Alert);
        });
        f.act(cx, |v, w, cx| v.focus_field(Field::To, w, cx));
        f.typed(cx, "x.pt");
        in_window(cx, f.window, |window, _| {
            assert!(!names(window).iter().any(|(_, n)| n.contains("not a valid")));
        });
        assert!(
            !f.view
                .read_with(cx, |v, _| v.drafts.active().unwrap().show_cc)
        );
        f.keys(cx, &["ctrl-shift-c"]);
        assert!(
            f.view
                .read_with(cx, |v, _| v.drafts.active().unwrap().show_cc)
        );
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Cc", Role::TextInput);
            assert_named(window, "Bcc", Role::TextInput);
        });
    }

    /// The recipients as chips: a separator typed makes one, its button
    /// removes it, Backspace in an empty field removes the last one, and the
    /// draft's text keeps the form it always had.
    #[gpui_kit::test]
    fn recipients_become_chips_and_go_again(cx: &mut TestAppContext) {
        let f = open(cx, "chips", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "Ana <ana@x.pt>, bo@x.pt;");
        let to = |cx: &mut TestAppContext| {
            f.view
                .read_with(cx, |v, _| v.drafts.active().unwrap().to.clone())
        };
        assert_eq!(to(cx), "Ana <ana@x.pt>, bo@x.pt;");
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Remove Ana", Role::Button);
            assert_named(window, "Remove bo@x.pt", Role::Button);
        });
        // The field holds only what is typed after the chips.
        let tail = f
            .view
            .read_with(cx, |v, cx| v.to.input.read(cx).value().to_string());
        assert_eq!(tail, "");
        in_window(cx, f.window, |window, cx| {
            click_named(window, "Remove Ana", cx)
        });
        cx.run_until_parked();
        assert_eq!(to(cx), "bo@x.pt, ");
        f.act(cx, |v, w, cx| v.focus_field(Field::To, w, cx));
        f.keys(cx, &["backspace"]);
        assert_eq!(to(cx), "");
        // Enter makes a chip of the address typed.
        f.typed(cx, "cy@x.pt");
        f.keys(cx, &["enter"]);
        assert_eq!(to(cx), "cy@x.pt, ");
    }

    /// Port of `recipient_suggestions_from_the_address_index`: senders of the
    /// cached mail are suggested after two characters, Down and Enter take
    /// one, and an address the draft has is not suggested again.
    #[gpui_kit::test]
    fn recipient_suggestions_from_the_address_index(cx: &mut TestAppContext) {
        let f = open(cx, "suggest", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "a");
        assert!(f.view.read_with(cx, |v, _| v.drafts.suggest.is_none()));
        f.typed(cx, "d");
        let shown = f.view.read_with(cx, |v, _| {
            v.drafts
                .suggest
                .as_ref()
                .map(|s| s.items.iter().map(|r| r.addr.clone()).collect::<Vec<_>>())
        });
        let shown = shown.expect("suggestions for \"ad\"");
        assert!(
            shown.contains(&"ada.lovelace@example.invalid".to_string()),
            "{shown:?}"
        );
        in_window(cx, f.window, |window, _| {
            assert_named(
                window,
                "Ada Lovelace, ada.lovelace@example.invalid",
                Role::ListBoxOption,
            );
            // Phase 13: a named list box of options.
            assert_named(window, "Suggested recipients", Role::ListBox);
            // The field it belongs to says its suggestions are open.
            assert_eq!(
                gpui_kit::test::TestQueryExt::find_by_label(&*window, &text::words::to())
                    .expanded(),
                Some(true)
            );
        });
        let index = shown
            .iter()
            .position(|a| a == "ada.lovelace@example.invalid")
            .unwrap();
        for _ in 0..index {
            f.keys(cx, &["down"]);
        }
        f.keys(cx, &["enter"]);
        let to = f
            .view
            .read_with(cx, |v, _| v.drafts.active().unwrap().to.clone());
        assert_eq!(to, "Ada Lovelace <ada.lovelace@example.invalid>, ");
        assert!(f.view.read_with(cx, |v, _| v.drafts.suggest.is_none()));
        // Taken already: not suggested again.
        f.typed(cx, "ad");

        let again = f.view.read_with(cx, |v, _| {
            v.drafts.suggest.as_ref().map_or(Vec::new(), |s| {
                s.items.iter().map(|r| r.addr.clone()).collect::<Vec<_>>()
            })
        });
        assert!(!again.contains(&"ada.lovelace@example.invalid".to_string()));
        // Esc closes the suggestions, not the composer.
        f.keys(cx, &["backspace", "backspace"]);
        f.typed(cx, "gr");
        assert!(f.view.read_with(cx, |v, _| v.drafts.suggest.is_some()));
        f.keys(cx, &["escape"]);
        assert!(f.view.read_with(cx, |v, _| v.drafts.suggest.is_none()));
        assert!(f.view.read_with(cx, |v, _| v.composing()));
    }

    /// Port of `reply_focuses_the_top_of_the_body`: a reply has its
    /// recipient, its subject and the folded quote; the body has the focus.
    #[gpui_kit::test]
    fn reply_focuses_the_top_of_the_body(cx: &mut TestAppContext) {
        let f = open(cx, "reply", true);
        let (m, body) = f.message();
        f.act(cx, |v, w, cx| v.reply(&m, Some(&body), false, w, cx));
        let (kind, to, subject) = f.view.read_with(cx, |v, _| {
            let d = v.drafts.active().unwrap();
            (d.kind, d.to.clone(), d.subject.clone())
        });
        assert_eq!(kind, Kind::Reply);
        assert_eq!(to, m.from_addr);
        assert_eq!(subject, format!("Re: {}", m.subject));
        // The quote is folded: the body starts empty, then typing goes in.
        f.typed(cx, "Thanks!");
        assert!(f.draft_text(cx).starts_with("Thanks!"));
        // Replying again brings the same draft back.
        f.act(cx, |v, w, cx| v.reply(&m, Some(&body), false, w, cx));
        assert!(f.draft_text(cx).starts_with("Thanks!"));
        assert_eq!(f.view.read_with(cx, |v, _| v.drafts.len()), 1);
    }

    /// Port of `closing_saves_and_the_drafts_view_reopens_it`: Esc saves at
    /// once ("Draft saved") and the saved draft opens again as it was.
    #[gpui_kit::test]
    fn closing_saves_and_the_drafts_view_reopens_it(cx: &mut TestAppContext) {
        let f = open(cx, "close-saves", false);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        f.act(cx, |v, w, cx| v.focus_field(Field::Subject, w, cx));
        f.typed(cx, "Plans");
        f.keys(cx, &["escape"]);
        assert_eq!(f.status(cx), Some(Status::DraftSaved));
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert!(f.saw(|e| *e == ComposerEvent::Closed));
        let db = f.db();
        let rows = db.draft_ids("").unwrap();
        assert_eq!(rows.len(), 1);
        let saved = db.draft(rows[0]).unwrap().unwrap();
        assert_eq!(
            (saved.to.as_str(), saved.subject.as_str()),
            ("ana@example.org", "Plans")
        );
        assert!(!saved.message_id.is_empty());
        // Copied to the server's Drafts at once.
        assert!(f.saw(|e| matches!(e, ComposerEvent::SaveToServer { row, .. } if *row == rows[0])));
        let row = rows[0];
        f.act(cx, |v, w, cx| v.open_saved(row, w, cx));
        let subject = f
            .view
            .read_with(cx, |v, cx| v.subject.read(cx).value().to_string());
        assert_eq!(subject, "Plans");
    }

    /// Port of `an_untouched_draft_is_discarded_without_asking` and
    /// `importance_alone_does_not_keep_a_draft`: a new message with nothing
    /// written goes on Esc and on Discard, saved nowhere, even with High
    /// importance.
    #[gpui_kit::test]
    fn an_untouched_draft_goes_without_asking(cx: &mut TestAppContext) {
        let f = open(cx, "untouched", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.act(cx, |v, _, cx| v.set_importance(Importance::High, cx));
        f.keys(cx, &["escape"]);
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert_eq!(f.db().count_drafts().unwrap(), 0);
        assert!(!f.saw(|e| matches!(e, ComposerEvent::SaveToServer { .. })));
        assert_eq!(f.status(cx), None);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        in_window(cx, f.window, |window, cx| {
            click_named(window, "Discard", cx)
        });
        cx.run_until_parked();
        assert!(!f.view.read_with(cx, |v, _| v.composing()), "no question");
        assert_eq!(f.db().count_drafts().unwrap(), 0);
    }

    /// Port of `discarding_removes_the_draft_here_and_its_server_copy`: a
    /// draft with writing asks first; confirming removes its saved row.
    #[gpui_kit::test]
    fn discarding_asks_and_removes_the_saved_draft(cx: &mut TestAppContext) {
        let f = open(cx, "discard", false);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        let id = f.view.read_with(cx, |v, _| v.drafts.active_id().unwrap());
        f.act(cx, |v, _, cx| v.save_draft(id, cx));
        assert_eq!(f.db().count_drafts().unwrap(), 1);
        in_window(cx, f.window, |window, cx| {
            click_named(window, "Discard", cx)
        });
        cx.run_until_parked();
        in_window(cx, f.window, |window, _| {
            // The dialog's own pills (its title is Kit's, unnamed).
            assert_named(window, "Cancel", Role::Button);
        });
        assert!(f.view.read_with(cx, |v, _| v.composing()), "asked first");
        in_window(cx, f.window, |window, cx| {
            window.click("dialog-confirm", cx);
        });
        cx.run_until_parked();
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert_eq!(f.db().count_drafts().unwrap(), 0);
        assert!(f.saw(|e| matches!(e, ComposerEvent::DraftsChanged { count: true, .. })));
    }

    /// Port of `edits_within_two_seconds_end_in_one_save` at the view, on
    /// GPUI's test clock: typing saves two seconds after the last key, once,
    /// and copies to the server.
    #[gpui_kit::test]
    fn edits_are_saved_two_seconds_after_the_last(cx: &mut TestAppContext) {
        let f = open(cx, "autosave", false);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        cx.executor().advance_clock(Duration::from_millis(1_500));
        cx.run_until_parked();
        f.typed(cx, ".pt");
        cx.executor().advance_clock(Duration::from_millis(1_500));
        cx.run_until_parked();
        assert_eq!(f.db().count_drafts().unwrap(), 0, "not yet: 1.5 s since");
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        let db = f.db();
        let rows = db.draft_ids("").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(db.draft(rows[0]).unwrap().unwrap().to, "ana@example.org.pt");
        assert_eq!(
            f.events
                .borrow()
                .iter()
                .filter(|e| matches!(e, ComposerEvent::SaveToServer { .. }))
                .count(),
            1
        );
    }

    /// Ports of `a_send_is_queued_once_built_and_past_its_undo_window`,
    /// `undo_before_the_build_ends_returns_the_exact_draft` and
    /// `the_undo_offer_counts_down_and_a_delivery_says_sent` (the offer is the
    /// shell's): Send closes the composer, the message is built while the undo
    /// window runs and queued after it; Undo brings the draft back.
    #[gpui_kit::test]
    fn a_send_waits_out_its_undo_window_and_can_be_undone(cx: &mut TestAppContext) {
        let f = open(cx, "send", false);
        f.act(cx, |v, _, _| v.set_undo_delay(Duration::from_secs(10)));
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        f.act(cx, |v, w, cx| v.focus_field(Field::Subject, w, cx));
        f.typed(cx, "Plans");
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "See you at noon.");
        f.keys(cx, &["ctrl-enter"]);
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        let row = f
            .events
            .borrow()
            .iter()
            .find_map(|e| match e {
                ComposerEvent::Sending { row, subject, secs } => {
                    assert_eq!((subject.as_str(), *secs), ("Plans", 10));
                    Some(*row)
                }
                _ => None,
            })
            .expect("the undo offer");
        assert!(!f.saw(|e| matches!(e, ComposerEvent::Queued { .. })));
        // Undo within the window: back in the composer, as it was.
        f.act(cx, |v, w, cx| v.undo_send(row, w, cx));
        assert!(f.view.read_with(cx, |v, _| v.composing()));
        assert!(f.draft_text(cx).starts_with("See you at noon."));
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        assert!(
            !f.saw(|e| matches!(e, ComposerEvent::Queued { .. })),
            "undone"
        );
        // Sent again and left alone: queued once the window is over.
        f.keys(cx, &["ctrl-enter"]);
        assert!(f.view.read_with(cx, |v, _| v.pending_sends().len()) == 1);
        cx.executor().advance_clock(Duration::from_secs(10));
        cx.run_until_parked();
        let account = f.account;
        assert!(f.saw(|e| matches!(e, ComposerEvent::Queued { account: a, .. } if *a == account)));
        assert!(f.view.read_with(cx, |v, _| v.pending_sends().is_empty()));
        assert_eq!(f.db().count_drafts().unwrap(), 0);
    }

    /// A second composer, as a draft tab's, with a message ready to send
    /// and what it tells the shell.
    fn tab_composer(
        f: &Fixture,
        cx: &mut TestAppContext,
    ) -> (Entity<ComposerView>, Rc<RefCell<Vec<ComposerEvent>>>) {
        let db = Rc::new(f.db());
        let tab = in_window(cx, f.window, move |window, cx| {
            cx.new(|cx| {
                let mut v = ComposerView::new(db, window, cx);
                v.set_defaults(ComposeDefaults {
                    rich: false,
                    ..ComposeDefaults::default()
                });
                v.set_undo_delay(Duration::from_secs(10));
                v
            })
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&tab, move |_, e: &ComposerEvent, _| {
                sink.borrow_mut().push(e.clone());
            })
            .detach();
        });
        let t = tab.clone();
        in_window(cx, f.window, move |window, cx| {
            t.update(cx, |v, cx| {
                v.compose_new(None, window, cx);
                let d = v.drafts.active_mut().unwrap();
                d.edit(Field::To, "ana@example.org");
                d.edit(Field::Subject, "From a tab");
                d.edit(Field::Body, "Sent from a draft tab.");
            })
        });
        cx.run_until_parked();
        (tab, events)
    }

    /// The Slint app's one list of sends (`send_draft`, `count_down`,
    /// `promote`; tabs_tests.rs `undo_send_reopens_in_a_new_tab`, its model
    /// half): a draft tab's composer goes with its tab as Send closes it,
    /// yet its send still waits out the undo window and is queued, told
    /// through the home composer; the undo offer and the Drafts count come
    /// before Closed, while the tab still listens; and the home composer
    /// undoes a tab's send.
    #[gpui_kit::test]
    fn a_send_from_a_draft_tab_goes_out_after_the_tab_goes(cx: &mut TestAppContext) {
        let f = open(cx, "tab-send", false);
        let (tab, events) = tab_composer(&f, cx);
        let t = tab.clone();
        in_window(cx, f.window, move |window, cx| {
            t.update(cx, |v, cx| v.send(true, window, cx))
        });
        cx.run_until_parked();
        let seen = events.borrow().clone();
        let at = |p: &dyn Fn(&ComposerEvent) -> bool| seen.iter().position(p);
        let closed = at(&|e| *e == ComposerEvent::Closed).expect("closed");
        let sending = at(&|e| matches!(e, ComposerEvent::Sending { .. })).expect("the offer");
        let drafts = at(&|e| matches!(e, ComposerEvent::DraftsChanged { count: true, .. }))
            .expect("the Drafts count");
        assert!(sending < closed && drafts < closed, "{seen:?}");
        let row = match &seen[sending] {
            ComposerEvent::Sending { row, .. } => *row,
            _ => unreachable!(),
        };
        // The tab goes, and its composer with it (tab_host on Closed).
        let origin = tab.entity_id();
        assert_eq!(f.view.read_with(cx, |v, _| v.sent_by(row)), Some(origin));
        assert_eq!(f.view.read_with(cx, |v, _| v.pending_sends()), vec![row]);
        drop(tab);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        let account = f.account;
        assert!(
            f.saw(|e| matches!(e, ComposerEvent::Queued { row: r, account: a } if *r == row && *a == account)),
            "queued through the home composer"
        );
        assert_eq!(
            f.db().outbox_entry(row).unwrap().map(|e| e.state),
            Some(db::Outbox::Queued)
        );
        assert!(f.view.read_with(cx, |v, _| v.pending_sends().is_empty()));

        // Another tab's send, undone from the home composer (the status
        // line's Undo goes to the mail view's composer).
        let (tab, events) = tab_composer(&f, cx);
        let t = tab.clone();
        in_window(cx, f.window, move |window, cx| {
            t.update(cx, |v, cx| v.send(true, window, cx))
        });
        cx.run_until_parked();
        drop(tab);
        let row = events
            .borrow()
            .iter()
            .find_map(|e| match e {
                ComposerEvent::Sending { row, .. } => Some(*row),
                _ => None,
            })
            .expect("the offer");
        f.act(cx, |v, w, cx| v.undo_send(row, w, cx));
        assert!(f.view.read_with(cx, |v, _| v.composing()), "back, here");
        assert!(f.draft_text(cx).starts_with("Sent from a draft tab."));
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        assert!(
            !f.saw(|e| matches!(e, ComposerEvent::Queued { row: r, .. } if *r == row)),
            "undone"
        );
        assert_eq!(f.db().count_drafts().unwrap(), 1);
    }

    /// The Slint app's file-hover overlay (ui/app.slint `file-hover`):
    /// files dragged from Explorer over the composer show "Drop to attach"
    /// (a status), and it goes when the drag leaves the window.
    #[gpui_kit::test]
    fn files_dragged_over_the_composer_say_drop_to_attach(cx: &mut TestAppContext) {
        use gpui::{ExternalPaths, FileDropEvent, InputEvent as _, point, px};
        let f = open(cx, "drop-overlay", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        let shown = |cx: &mut TestAppContext| {
            in_window(cx, f.window, |window, _| {
                names(window).iter().any(|(_, n)| n == "Drop to attach")
            })
        };
        assert!(!shown(cx));
        in_window(cx, f.window, |window, cx| {
            let paths = ExternalPaths([f.path.clone()].into_iter().collect());
            window.dispatch_event(
                FileDropEvent::Entered {
                    position: point(px(400.), px(400.)),
                    paths,
                }
                .to_platform_input(),
                cx,
            );
        });
        cx.run_until_parked();
        assert!(shown(cx), "while files are dragged over it");
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Drop to attach", Role::Status);
        });
        in_window(cx, f.window, |window, cx| {
            window.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
        });
        cx.run_until_parked();
        assert!(!shown(cx), "gone with the drag");
    }

    /// The Slint app's `edit_server_draft` and `show_draft_reply`
    /// (src/app/composer.rs): a message in the server's Drafts opens as a
    /// saved draft of this PC linked to it, once (editing it again finds the
    /// same draft); a message elsewhere does not. A reply in progress is
    /// reported for the reader's "You started a reply" bar.
    #[gpui_kit::test]
    fn a_server_draft_is_edited_as_a_saved_draft(cx: &mut TestAppContext) {
        let f = open(cx, "server-draft", false);
        let (m, body) = f.message();
        f.act(cx, |v, w, cx| v.edit_server_draft(m.id, w, cx));
        assert!(!f.view.read_with(cx, |v, _| v.composing()), "not in Drafts");
        // A draft saved elsewhere, in the server's Drafts.
        let mut db = f.db();
        let drafts = db
            .upsert_folder(
                f.account,
                "Drafts",
                "Drafts",
                0,
                komorebi::role::Role::Drafts,
            )
            .unwrap();
        db.insert_headers(
            f.account,
            drafts,
            &[db::NewHeader {
                uid: 1,
                msgid: "<webmail-1@example.org>".into(),
                subject: m.subject.clone(),
                to: "ana@example.org".into(),
                date: 1_700_000_000,
                ..Default::default()
            }],
        )
        .unwrap();
        let id = db.message_ids(&[drafts], "").unwrap()[0];
        let raw = b"From: seed@komorebi.invalid\r\nTo: ana@example.org\r\n\
            Subject: x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nFrom webmail\r\n";
        db.set_body(id, raw, "From webmail", false, Default::default())
            .unwrap();
        let m = db.message(id).unwrap().unwrap();
        f.act(cx, |v, w, cx| v.edit_server_draft(m.id, w, cx));
        assert!(f.view.read_with(cx, |v, _| v.composing()));
        let subject = f
            .view
            .read_with(cx, |v, cx| v.subject.read(cx).value().to_string());
        assert_eq!(subject, m.subject);
        assert_eq!(f.db().count_drafts().unwrap(), 1);
        assert!(f.saw(|e| matches!(e, ComposerEvent::DraftsChanged { count: true, .. })));
        f.act(cx, |v, w, cx| v.edit_server_draft(m.id, w, cx));
        assert_eq!(f.db().count_drafts().unwrap(), 1, "the same draft");

        // A reply in progress, then saved: the reader's bar has it.
        let other = f.db().message_ids(&[f.inbox], "").unwrap()[1];
        assert!(!f.view.read_with(cx, |v, _| v.reply_started(other)));
        let msg = f.db().message(other).unwrap().unwrap();
        f.act(cx, |v, w, cx| v.reply(&msg, Some(&body), false, w, cx));
        assert!(f.view.read_with(cx, |v, _| v.reply_started(other)));
    }

    /// The composer half of `selecting_a_message_moves_a_dirty_pane_draft_to_a_tab`
    /// (the Slint app's `pane_draft_aside`): the pane's draft with writing is
    /// saved, here and on the server, and handed over by its row for a
    /// background tab; an empty one goes.
    #[gpui_kit::test]
    fn a_dirty_pane_draft_is_set_aside_for_a_tab(cx: &mut TestAppContext) {
        let f = open(cx, "aside", false);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        let row = f.act(cx, |v, _, cx| v.set_aside(cx));
        let row = row.expect("kept for a tab");
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert_eq!(f.view.read_with(cx, |v, _| v.drafts.len()), 0);
        assert_eq!(f.db().draft(row).unwrap().unwrap().to, "ana@example.org");
        assert!(f.saw(|e| matches!(e, ComposerEvent::SaveToServer { row: r, .. } if *r == row)));
        assert_eq!(f.status(cx), Some(Status::KeptInTab));
        assert_eq!(text::draft_kept_in_tab(), "Draft kept in a tab");
        // Nothing written: nothing kept, nothing saved.
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        assert_eq!(f.act(cx, |v, _, cx| v.set_aside(cx)), None);
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert_eq!(f.db().count_drafts().unwrap(), 1);
    }

    /// A pane draft whose save fails is neither set aside nor closed: it
    /// stays shown with the error, so its text is never out of sight with
    /// no saved copy (stage D integration's "Not done": `set_aside` hid it).
    #[gpui_kit::test]
    fn a_draft_that_could_not_be_saved_stays_shown(cx: &mut TestAppContext) {
        let f = open(cx, "aside-fails", false);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        // An account that is not in the cache: the drafts row cannot be
        // written (its foreign key).
        f.act(cx, |v, _, _| {
            v.drafts.active_mut().expect("a draft").account_id = i64::MAX;
        });
        assert_eq!(f.act(cx, |v, _, cx| v.set_aside(cx)), None);
        assert!(f.view.read_with(cx, |v, _| v.composing()), "still shown");
        assert!(matches!(f.status(cx), Some(Status::CouldNotSave(_))));
        assert!(!f.saw(|e| matches!(e, ComposerEvent::Closed)));
        // Esc (Close) keeps it too.
        f.act(cx, |v, w, cx| v.close(w, cx));
        assert!(f.view.read_with(cx, |v, _| v.composing()), "Esc keeps it");
        assert!(!f.saw(|e| matches!(e, ComposerEvent::Closed)));
        assert_eq!(f.view.read_with(cx, |v, _| v.drafts.len()), 1);
        assert_eq!(f.db().count_drafts().unwrap(), 0);
        // The account back: set aside as any other.
        let account = f.account;
        f.act(cx, |v, _, _| {
            v.drafts.active_mut().expect("a draft").account_id = account;
        });
        let row = f
            .act(cx, |v, _, cx| v.set_aside(cx))
            .expect("kept for a tab");
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert_eq!(f.db().draft(row).unwrap().unwrap().to, "ana@example.org");
    }

    /// Port of `reply_reopens_the_saved_draft_and_the_row_is_tagged` (row
    /// 95), its tag half (the reopening is `reply_focuses_the_top_of_the_body`):
    /// in the main window, a reply saved from the mail view's composer tags
    /// the message's row "Draft" (`ComposerEvent::DraftsChanged` reloads the
    /// list), and the message reports the reply started for the reader's bar.
    #[gpui_kit::test]
    fn a_saved_reply_tags_its_row_in_the_list(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let (cache, db, seeded) = test_support::seeded("row-tag", 5);
        let id = db.message_ids(&[seeded.inbox], "").unwrap()[0];
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let mail = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        let composer = mail.read_with(cx, |m, _| m.composer().clone());
        let tagged = |cx: &mut TestAppContext| {
            mail.read_with(cx, |m, _| {
                let rows = m.mail().rows();
                (0..rows.len())
                    .filter_map(|i| rows.row(i))
                    .find(|r| r.id == id)
                    .map(|r| r.draft)
            })
        };
        assert_eq!(tagged(cx), Some(false));
        let c = composer.clone();
        in_window(cx, window, move |w, cx| {
            c.update(cx, |c, cx| {
                c.open(
                    Some(Draft::Reply {
                        message: id,
                        all: false,
                    }),
                    w,
                    cx,
                )
            })
        });
        cx.run_until_parked();
        in_window(cx, window, |w, cx| type_text(w, "Ok", cx));
        cx.run_until_parked();
        let c = composer.clone();
        in_window(cx, window, move |w, cx| {
            c.update(cx, |c, cx| c.open(None, w, cx))
        });
        cx.run_until_parked();
        assert_eq!(cache.open().count_drafts().unwrap(), 1);
        assert_eq!(tagged(cx), Some(true), "the row says Draft");
        assert!(composer.read_with(cx, |c, _| c.reply_started(id)));
    }

    /// The composer-field half of `ctrl_tab_moves_between_tabs_even_from_a_text_field`
    /// (row 296; its tab half is `window::tests::ctrl_tab_moves_between_tabs`):
    /// Ctrl+Tab and Ctrl+Shift+Tab move between tabs from the pane
    /// composer's To field (a field takes Tab), and the pane's draft is
    /// there again on the way back.
    #[gpui_kit::test]
    fn ctrl_tab_moves_between_tabs_from_the_composer(cx: &mut TestAppContext) {
        use komorebi::tabs::{Tab, TabKind};
        test_support::init_app(cx);
        let (_cache, db, seeded) = test_support::seeded("ctrl-tab", 3);
        let id = db.message_ids(&[seeded.inbox], "").unwrap()[1];
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        main.update(cx, |v, _| {
            v.tabs_mut()
                .insert(1, Tab::new(TabKind::Message(id), String::new()));
        });
        let mail = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        let composer = mail.read_with(cx, |m, _| m.composer().clone());
        let c = composer.clone();
        in_window(cx, window, move |w, cx| {
            c.update(cx, |c, cx| c.open(Some(Draft::New), w, cx))
        });
        cx.run_until_parked();
        let to_focused = composer.read_with(cx, |c, _| c.to.input.clone());
        in_window(cx, window, move |w, cx| {
            assert!(
                to_focused.read(cx).focus_handle(cx).is_focused(w),
                "To has the focus"
            );
        });
        let active = |cx: &mut TestAppContext| main.read_with(cx, |v, _| v.tabs().active());
        in_window(cx, window, |w, cx| press(w, &["ctrl-tab"], cx));
        cx.run_until_parked();
        assert_eq!(active(cx), 1);
        in_window(cx, window, |w, cx| press(w, &["ctrl-shift-tab"], cx));
        cx.run_until_parked();
        assert_eq!(active(cx), 0);
        assert!(
            composer.read_with(cx, |c, _| c.composing()),
            "the pane's draft is back"
        );
    }

    /// Port of `send_guards_ask_once_and_send_anyway_goes_ahead`: no subject
    /// asks first; "Send anyway" sends.
    #[gpui_kit::test]
    fn send_guards_ask_once_and_send_anyway_goes_ahead(cx: &mut TestAppContext) {
        let f = open(cx, "guards", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        f.act(cx, |v, w, cx| v.send(false, w, cx));
        assert_eq!(
            f.view.read_with(cx, |v, _| v.guard()),
            Some(Guard::NoSubject)
        );
        in_window(cx, f.window, |window, _| {
            assert_named(window, "This message has no subject.", Role::Status);
            assert_named(window, "Send anyway", Role::Button);
        });
        assert!(f.view.read_with(cx, |v, _| v.composing()));
        in_window(cx, f.window, |window, cx| {
            click_named(window, "Send anyway", cx)
        });
        cx.run_until_parked();
        assert!(!f.view.read_with(cx, |v, _| v.composing()));
        assert!(f.saw(|e| matches!(e, ComposerEvent::Queued { .. })));
    }

    /// Port of `added_files_update_the_counter_and_are_saved`: files read for
    /// the draft show as pills with the counter, and are copied once.
    #[gpui_kit::test]
    fn added_files_update_the_counter_and_are_saved(cx: &mut TestAppContext) {
        let f = open(cx, "attach", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        let file = |name: &str, len: usize| {
            komorebi::compose::Attached::from(komorebi::mail::smtp::OutAttachment {
                filename: name.into(),
                data: vec![1; len],
            })
        };
        let id = f.view.read_with(cx, |v, _| v.drafts.active_id().unwrap());
        f.act(cx, |v, _, cx| {
            v.add_attachments(id, vec![file("notes.txt", 3_000)], Vec::new(), cx)
        });
        in_window(cx, f.window, |window, _| {
            assert_named(window, "notes.txt, 3 KB", Role::Group);
            assert_named(window, "Attachments 4 KB of 25 MB", Role::Status);
            assert_named(window, "Remove notes.txt", Role::Button);
        });
        let row = f
            .view
            .read_with(cx, |v, _| v.drafts.active().unwrap().row.expect("saved"));
        assert_eq!(f.db().draft_files(row).unwrap().len(), 1);
        in_window(cx, f.window, |window, cx| {
            click_named(window, "Remove notes.txt", cx)
        });
        cx.run_until_parked();
        assert!(f.db().draft_files(row).unwrap().is_empty());
    }

    /// Ports of `formatted_mail_goes_out_as_text_and_html_with_the_logo` and
    /// `a_message_with_no_formatting_goes_out_as_plain_text_only`: what the
    /// editor holds is what is sent.
    #[gpui_kit::test]
    fn formatted_mail_goes_out_as_text_and_html(cx: &mut TestAppContext) {
        let f = open(cx, "formatted", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        f.act(cx, |v, w, cx| v.focus_field(Field::Subject, w, cx));
        f.typed(cx, "Hello");
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "Plain words");
        let raw = |f: &Fixture, cx: &mut TestAppContext| {
            let id = f.view.read_with(cx, |v, _| v.drafts.active_id().unwrap());
            let db = f.db();
            let account = db.accounts().unwrap().remove(0);
            f.view.update(cx, |v, cx| {
                v.lend(cx, |d| {
                    let copy = komorebi::compose::Draft {
                        to: d.to.clone(),
                        subject: d.subject.clone(),
                        rich: d
                            .rich
                            .as_ref()
                            .map(|e| komorebi::mail::rich::edit::Editor::new(e.doc().clone())),
                        body: d.body.clone(),
                        ..komorebi::compose::Draft::new(d.account_id)
                    };
                    let _ = id;
                    komorebi::mail::smtp::build_raw(komorebi::compose::outgoing(
                        &db, &account, copy,
                    ))
                    .unwrap()
                })
                .unwrap()
            })
        };
        let plain = String::from_utf8(raw(&f, cx)).unwrap();
        assert!(!plain.contains("text/html"), "no formatting: text only");
        f.keys(cx, &["ctrl-a", "ctrl-b"]);
        let html = String::from_utf8(raw(&f, cx)).unwrap();
        assert!(html.contains("text/html") && html.contains("text/plain"));
        assert!(html.contains("<strong>Plain words</strong>"), "{html}");
    }

    /// Ports of `switching_to_plain_text_can_be_undone` and
    /// `a_message_switched_to_plain_text_goes_out_as_plain_text_only`: the
    /// More menu's switch keeps the text; switching back before typing gives
    /// the formatting back.
    #[gpui_kit::test]
    fn switching_to_plain_text_and_back(cx: &mut TestAppContext) {
        let f = open(cx, "plain-switch", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "Hi there");
        f.keys(cx, &["ctrl-a", "ctrl-b"]);
        f.act(cx, |v, w, cx| v.set_rich(false, w, cx));
        assert!(!f.view.read_with(cx, |v, cx| v.body.read(cx).is_formatted()));
        let plain = f
            .view
            .read_with(cx, |v, cx| v.plain.read(cx).value().to_string());
        assert_eq!(plain.trim_end(), "Hi there");
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Message body", Role::MultilineTextInput);
        });
        // Port of `a_message_switched_to_plain_text_goes_out_as_plain_text_only`.
        let sent = f.view.update(cx, |v, cx| {
            let db = v.db().file().map(|p| Db::open(&p).unwrap()).unwrap();
            let account = db.accounts().unwrap().remove(0);
            v.lend(cx, |d| {
                let copy = komorebi::compose::Draft {
                    to: "ana@example.org".into(),
                    body: d.body.clone(),
                    ..komorebi::compose::Draft::new(d.account_id)
                };
                komorebi::mail::smtp::build_raw(komorebi::compose::outgoing(&db, &account, copy))
                    .unwrap()
            })
            .unwrap()
        });
        assert!(!String::from_utf8(sent).unwrap().contains("text/html"));
        f.act(cx, |v, w, cx| v.set_rich(true, w, cx));
        let bold = f.view.read_with(cx, |v, cx| {
            v.body
                .read(cx)
                .editor()
                .is_some_and(|e| e.format_at(0, 8).b)
        });
        assert!(bold, "the formatting came back");
    }

    /// Ports of `new_mail_and_replies_start_with_the_signature` and
    /// `the_signature_menu_and_another_from_account_swap_the_signature`.
    #[gpui_kit::test]
    fn the_signature_goes_in_and_the_menu_swaps_it(cx: &mut TestAppContext) {
        let f = open(cx, "signature", true);
        let db = f.db();
        let doc =
            komorebi::compose::rich::doc_from_text("Seed\nACME", ComposeDefaults::default().doc());
        let sig = db.add_signature("Work", &doc, None).unwrap();
        db.set_account_signatures(f.account, Some(sig), Some(sig))
            .unwrap();
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        assert!(f.draft_text(cx).contains("ACME"), "{}", f.draft_text(cx));
        let current = f.view.read_with(cx, |v, cx| v.current_signature(cx));
        assert_eq!(current, Some(sig));
        f.act(cx, |v, w, cx| v.pick_signature(None, w, cx));
        assert!(!f.draft_text(cx).contains("ACME"));
        f.act(cx, |v, w, cx| v.pick_signature(Some(sig), w, cx));
        assert!(f.draft_text(cx).contains("ACME"));
        // A reply starts with it too, above the quote.
        f.keys(cx, &["escape"]);
        let (m, body) = f.message();
        f.act(cx, |v, w, cx| v.reply(&m, Some(&body), false, w, cx));
        assert!(f.draft_text(cx).contains("ACME"));
    }

    /// Port of `a_formatted_draft_is_saved_and_opens_again_the_same`.
    #[gpui_kit::test]
    fn a_formatted_draft_is_saved_and_opens_again_the_same(cx: &mut TestAppContext) {
        let f = open(cx, "formatted-saved", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.typed(cx, "ana@example.org");
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "Bold words");
        f.keys(cx, &["ctrl-a", "ctrl-b"]);
        f.keys(cx, &["escape"]);
        let row = f.db().draft_ids("").unwrap()[0];
        let saved = f.db().draft(row).unwrap().unwrap();
        assert!(saved.body_rich.is_some(), "saved formatted");
        f.act(cx, |v, w, cx| v.open_saved(row, w, cx));
        let bold = f.view.read_with(cx, |v, cx| {
            v.body
                .read(cx)
                .editor()
                .is_some_and(|e| e.format_at(0, 4).b)
        });
        assert!(bold);
    }

    /// Port of `a_formatted_reply_does_not_ask_about_an_attachment_the_original_mentions`:
    /// the seeded mail says "attached"; only the user's own words count.
    #[gpui_kit::test]
    fn a_reply_does_not_ask_about_the_originals_attachment(cx: &mut TestAppContext) {
        let f = open(cx, "reply-guard", true);
        let (m, body) = f.message();
        assert!(body.text.contains("attached") || body.text.contains("thanks"));
        f.act(cx, |v, w, cx| v.reply(&m, Some(&body), false, w, cx));
        f.typed(cx, "Sounds good");
        f.act(cx, |v, w, cx| v.send(false, w, cx));
        assert_eq!(f.view.read_with(cx, |v, _| v.guard()), None);
        assert!(!f.view.read_with(cx, |v, _| v.composing()), "sent");
    }

    /// The composer's names in European Portuguese (phase 13 asks every
    /// control for a name in the language shown).
    #[gpui_kit::test]
    fn the_composer_is_named_in_portuguese(cx: &mut TestAppContext) {
        let f = open(cx, "pt", true);
        test_support::set_lang(cx, komorebi::i18n::Lang::PtPt);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        in_window(cx, f.window, |window, _| {
            let all = names(window);
            assert!(all.iter().any(|(_, n)| n == "Enviar"), "{all:?}");
            assert!(!all.iter().any(|(_, n)| n == "Send"));
        });
    }

    /// Port of `switching_to_plain_text_can_be_undone`: Ctrl+Z right after
    /// the switch brings the formatted message back.
    #[gpui_kit::test]
    fn ctrl_z_after_the_switch_to_plain_text_undoes_it(cx: &mut TestAppContext) {
        let f = open(cx, "plain-undo", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "Hi there");
        f.keys(cx, &["ctrl-a", "ctrl-i"]);
        f.act(cx, |v, w, cx| v.set_rich(false, w, cx));
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.keys(cx, &["ctrl-z"]);
        let italic = f.view.read_with(cx, |v, cx| {
            v.body
                .read(cx)
                .editor()
                .is_some_and(|e| e.format_at(0, 8).i)
        });
        assert!(italic, "formatted again, as it was");
    }

    /// Ports of `a_formatted_reply_keeps_the_original_as_its_quote` and
    /// `an_html_reply_quotes_the_original_in_its_html_part`: a reply to an
    /// HTML message quotes the original's HTML, folded, and sends it in its
    /// HTML part; a plain-text reply keeps the text quote only.
    #[gpui_kit::test]
    fn a_reply_to_html_mail_quotes_its_html(cx: &mut TestAppContext) {
        let f = open(cx, "html-reply", true);
        let mut db = f.db();
        let id = db.message_ids(&[f.inbox], "").unwrap()[0];
        let raw = b"From: Bob <bob@example.invalid>\r\nTo: seed@komorebi.invalid\r\n\
            Subject: Lunch\r\nMessage-ID: <html-1@example.invalid>\r\n\
            Content-Type: text/html; charset=utf-8\r\n\r\n\
            <p>Noon at <b>the usual place</b>?</p>\r\n";
        db.set_body(
            id,
            raw,
            "Noon at the usual place?",
            false,
            Default::default(),
        )
        .unwrap();
        let m = db.message(id).unwrap().unwrap();
        let body = mime::parse_body(m.raw.as_deref().unwrap());
        f.act(cx, |v, w, cx| v.reply(&m, Some(&body), false, w, cx));
        let quoted = f.view.read_with(cx, |v, cx| {
            v.body.read(cx).editor().is_some_and(|e| {
                e.has_quote()
                    && e.quote_folded()
                    && e.doc().blocks.iter().any(|b| match b {
                        komorebi::mail::rich::Block::Quote(q) => q.html().contains("usual place"),
                        _ => false,
                    })
            })
        });
        assert!(quoted, "the original's HTML is the folded quote");
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Show quoted text", Role::Button);
        });
        f.typed(cx, "Yes!");
        let account = db.accounts().unwrap().remove(0);
        let raw = f.view.update(cx, |v, cx| {
            v.lend(cx, |d| {
                let copy = komorebi::compose::Draft {
                    to: d.to.clone(),
                    rich: d
                        .rich
                        .as_ref()
                        .map(|e| komorebi::mail::rich::edit::Editor::new(e.doc().clone())),
                    ..komorebi::compose::Draft::new(d.account_id)
                };
                komorebi::mail::smtp::build_raw(komorebi::compose::outgoing(&db, &account, copy))
                    .unwrap()
            })
            .unwrap()
        });
        let raw = String::from_utf8(raw).unwrap();
        assert!(raw.contains("text/html"), "{raw}");
        assert!(raw.contains("usual place"), "{raw}");
    }

    /// The More menu's font, size and colour reach the selection (the
    /// Slint toolbar's boxes).
    #[gpui_kit::test]
    fn the_more_menu_sets_font_size_and_colour(cx: &mut TestAppContext) {
        let f = open(cx, "more-menu", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "Colours");
        f.keys(cx, &["ctrl-a"]);
        for path in [
            ["More options", "Font colour", "Blue"],
            ["More options", "Font", "Georgia"],
            ["More options", "Font size", "18"],
        ] {
            for name in path {
                in_window(cx, f.window, |window, cx| click_named(window, name, cx));
                cx.run_until_parked();
            }
        }
        let fmt = f
            .view
            .read_with(cx, |v, cx| v.body.read(cx).format().unwrap());
        assert_eq!(fmt.color, Some(komorebi::mail::rich::ColorId::Blue));
        assert_eq!(fmt.font, komorebi::mail::rich::FontId::Georgia);
        assert_eq!(fmt.size.get(), 18);
    }

    /// Port of `the_formatting_row_is_one_tab_stop` (row 155): Bold to Link
    /// are one Tab stop, the one used last; Right moves along the row and
    /// Space presses the button reached.
    #[gpui_kit::test]
    fn the_formatting_row_is_one_tab_stop(cx: &mut TestAppContext) {
        let f = open(cx, "fmt-row", true);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        f.act(cx, |v, w, cx| v.focus_body(w, cx));
        f.typed(cx, "Z");
        let handles = f.view.read_with(cx, |v, _| v.fmt_focus.clone());
        // Round the window with Tab: of the six buttons only one is reached.
        let reached = |cx: &mut TestAppContext| {
            let handles = handles.clone();
            in_window(cx, f.window, move |window, cx| {
                let mut seen = Vec::new();
                for _ in 0..40 {
                    window.focus_next(cx);
                    if let Some(i) = handles.iter().position(|h| h.is_focused(window)) {
                        seen.push(i);
                    }
                }
                seen
            })
        };
        let seen = reached(cx);
        assert!(!seen.is_empty(), "the row is reached");
        assert!(seen.iter().all(|&i| i == 0), "only Bold: {seen:?}");
        // Right from Bold: Italic, now the row's stop.
        let bold = handles[0].clone();
        in_window(cx, f.window, move |window, cx| window.focus(&bold, cx));
        f.keys(cx, &["right"]);
        assert!(in_window(cx, f.window, {
            let italic = handles[1].clone();
            move |window, _| italic.is_focused(window)
        }));
        assert_eq!(f.view.read_with(cx, |v, _| v.fmt_at), 1);
        let seen = reached(cx);
        assert!(seen.iter().all(|&i| i == 1), "only Italic: {seen:?}");
        // Left wraps from Bold to Link; End and Home go to the ends.
        assert_eq!(toolbar::format_step("left", 0), Some(5));
        assert_eq!(toolbar::format_step("right", 5), Some(0));
        assert_eq!(toolbar::format_step("end", 2), Some(5));
        assert_eq!(toolbar::format_step("home", 4), Some(0));
        assert_eq!(toolbar::format_step("up", 1), None);
        // Space on Italic: on for what is typed next, the body has the keys.
        let italic = handles[1].clone();
        in_window(cx, f.window, move |window, cx| window.focus(&italic, cx));
        f.keys(cx, &["space"]);
        let (on, body) = f.view.read_with(cx, |v, cx| {
            (
                v.body.read(cx).format().unwrap_or_default().i,
                v.body.read(cx).focus_handle(cx),
            )
        });
        assert!(on, "Italic is on");
        assert!(in_window(cx, f.window, move |window, _| body.is_focused(window)));
    }

    /// The recipients wrap onto more lines as the field fills (TODO.md
    /// "Reader and composer"): chips past the row's width go under the
    /// first ones, and the field grows with them.
    #[gpui_kit::test]
    fn recipient_chips_wrap_onto_more_lines(cx: &mut TestAppContext) {
        let f = open(cx, "chip-wrap", false);
        f.act(cx, |v, w, cx| v.compose_new(None, w, cx));
        let names: Vec<String> = (1..=24)
            .map(|i| format!("recipient-number-{i}@example.org"))
            .collect();
        f.typed(cx, &format!("{}, ", names.join(", ")));
        let tops = in_window(cx, f.window, |window, _| {
            use gpui_kit::test::TestQueryExt as _;
            let mut tops: Vec<i32> = names
                .iter()
                .map(|n| {
                    let b = window.find_by_label(&format!("Remove {n}")).bounds();
                    f32::from(b.origin.y).round() as i32
                })
                .collect();
            tops.sort_unstable();
            tops.dedup();
            tops
        });
        assert!(tops.len() > 1, "the chips wrap: rows at {tops:?}");
    }

    /// The main window, its mail view's composer, and the cache.
    fn main_window(
        cx: &mut TestAppContext,
        name: &str,
    ) -> (
        TempCache,
        AnyWindowHandle,
        Entity<crate::window::MainWindow>,
        Entity<ComposerView>,
    ) {
        test_support::init_app(cx);
        let (cache, db, _) = test_support::seeded(name, 3);
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let mail = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
        let pane = mail.read_with(cx, |m, _| m.composer().clone());
        (cache, window, main, pane)
    }

    /// The text a composer's draft has, from the editor while it shows.
    fn text_of(view: &Entity<ComposerView>, cx: &mut TestAppContext) -> String {
        view.read_with(cx, |v, cx| {
            if let Some(ed) = v.body.read(cx).editor() {
                return ed.text().to_string();
            }
            v.drafts
                .active()
                .map(|d| d.body.clone())
                .unwrap_or_default()
        })
    }

    /// The pane's composer with a new message to ana@example.org, moved to a
    /// composer window with Ctrl+O; the window's saved row and composer.
    fn pop_out_pane_draft(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        main: &Entity<crate::window::MainWindow>,
        pane: &Entity<ComposerView>,
    ) -> (i64, AnyWindowHandle, Entity<ComposerView>) {
        let p = pane.clone();
        in_window(cx, window, move |w, cx| {
            p.update(cx, |c, cx| c.open(Some(Draft::New), w, cx))
        });
        cx.run_until_parked();
        in_window(cx, window, |w, cx| type_text(w, "ana@example.org", cx));
        cx.run_until_parked();
        in_window(cx, window, |w, cx| press(w, &["ctrl-o"], cx));
        cx.run_until_parked();
        assert!(!pane.read_with(cx, |c, _| c.composing()), "left the pane");
        let rows = main.read_with(cx, |v, cx| v.draft_windows(cx));
        assert_eq!(rows.len(), 1, "one composer window");
        let row = rows[0].expect("saved as it left");
        let (win, popped) = main
            .read_with(cx, |v, _| v.draft_window_composer(0))
            .expect("the window's composer");
        (row, win, popped)
    }

    /// Port of `a_composer_window_keeps_its_own_draft_and_close_saves_it`
    /// (row 101): Ctrl+O moves the pane's draft to a window of its own;
    /// another draft starts in the pane, typing in the window never reaches
    /// it, and Esc in the window saves the draft and closes the window.
    #[gpui_kit::test]
    fn a_composer_window_keeps_its_own_draft_and_close_saves_it(cx: &mut TestAppContext) {
        let (cache, window, main, pane) = main_window(cx, "cwin-close");
        let (row, win, popped) = pop_out_pane_draft(cx, window, &main, &pane);
        assert!(popped.read_with(cx, |c, _| c.composing()));
        assert_eq!(
            popped.read_with(cx, |c, _| c.drafts.active().map(|d| d.to.clone())),
            Some("ana@example.org".to_string())
        );
        // Another draft in the main window.
        let p = pane.clone();
        in_window(cx, window, move |w, cx| {
            p.update(cx, |c, cx| c.open(Some(Draft::New), w, cx))
        });
        cx.run_until_parked();
        assert!(pane.read_with(cx, |c, _| c.composing()));
        // Typing in the window (the body has its keys: To is filled).
        in_window(cx, win, |w, cx| type_text(w, "From the window", cx));
        cx.run_until_parked();
        assert!(text_of(&popped, cx).contains("From the window"));
        assert!(!text_of(&pane, cx).contains("From the window"));
        // Esc in the window: saved, and the window goes.
        in_window(cx, win, |w, cx| press(w, &["escape"], cx));
        cx.run_until_parked();
        assert!(main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty());
        let saved = cache.open().draft(row).unwrap().expect("the saved draft");
        assert!(saved.body.contains("From the window"), "{}", saved.body);
        assert!(pane.read_with(cx, |c, _| c.composing()), "the pane's stays");
    }

    /// Port of `send_from_a_composer_window_once_and_undo_reopens_it_there`
    /// (row 102): Send from a composer window sends once and the window
    /// goes; the status line's Undo brings the draft back in a window, not
    /// in a tab or the pane.
    #[gpui_kit::test]
    fn send_from_a_composer_window_once_and_undo_reopens_it_there(cx: &mut TestAppContext) {
        let (cache, window, main, pane) = main_window(cx, "cwin-send");
        let (row, win, popped) = pop_out_pane_draft(cx, window, &main, &pane);
        popped.update(cx, |c, _| c.set_undo_delay(Duration::from_secs(10)));
        let p = popped.clone();
        in_window(cx, win, move |w, cx| {
            p.update(cx, |c, cx| c.send(true, w, cx))
        });
        cx.run_until_parked();
        assert!(
            main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty(),
            "the window went"
        );
        assert_eq!(cache.open().outbox_ids("").unwrap(), vec![row]);
        // Send again from the composer of the closed window: nothing.
        let p = popped.clone();
        in_window(cx, window, move |w, cx| {
            p.update(cx, |c, cx| c.send(true, w, cx))
        });
        cx.run_until_parked();
        assert_eq!(cache.open().outbox_ids("").unwrap().len(), 1);
        drop(popped);
        // Undo on the status line.
        let hub = main.read_with(cx, |v, _| v.hub().cloned().expect("the hub"));
        hub.update(cx, |h, cx| h.press_action(cx));
        cx.run_until_parked();
        assert_eq!(
            main.read_with(cx, |v, cx| v.draft_windows(cx)),
            vec![Some(row)]
        );
        assert!(!pane.read_with(cx, |c, _| c.composing()), "not in the pane");
        assert_eq!(main.read_with(cx, |v, _| v.tabs().len()), 1, "not in a tab");
        assert!(cache.open().outbox_ids("").unwrap().is_empty(), "undone");
    }

    /// Port of `a_popped_out_draft_leaves_a_placeholder_and_comes_forward_once`
    /// (row 103): a draft in a window comes forward when it is opened again
    /// (from the pane, the Drafts list or a tab) and never opens twice;
    /// files dropped on the window go to its draft; the title bar's Close
    /// saves and closes it. The pane needs no placeholder: a popped draft
    /// leaves it closed, as Esc does.
    #[gpui_kit::test]
    fn a_popped_out_draft_comes_forward_once(cx: &mut TestAppContext) {
        let (cache, window, main, pane) = main_window(cx, "cwin-popped");
        let (row, win, popped) = pop_out_pane_draft(cx, window, &main, &pane);
        let p = pane.clone();
        in_window(cx, window, move |w, cx| {
            p.update(cx, |c, cx| c.open(Some(Draft::Saved(row)), w, cx))
        });
        cx.run_until_parked();
        assert!(
            !pane.read_with(cx, |c, _| c.composing()),
            "not opened twice"
        );
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| v.open_draft_tab(Draft::Saved(row), w, cx))
        });
        cx.run_until_parked();
        assert_eq!(main.read_with(cx, |v, _| v.tabs().len()), 1, "no tab");
        assert_eq!(main.read_with(cx, |v, cx| v.draft_windows(cx)).len(), 1);
        // Files dropped on the window: its draft's.
        let file = cache.dir().join("notes.txt");
        std::fs::write(&file, b"notes").unwrap();
        popped.update(cx, |c, cx| c.attach_paths(vec![file], cx));
        cx.run_until_parked();
        assert_eq!(
            popped.read_with(cx, |c, _| c.drafts.active().map(|d| d.attachments.len())),
            Some(1)
        );
        assert!(!pane.read_with(cx, |c, _| c.composing()));
        // The title bar's Close (Alt+F4): saved, and the window goes.
        let mut vcx = gpui::VisualTestContext::from_window(win, cx);
        vcx.simulate_close();
        cx.run_until_parked();
        assert!(main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty());
        assert_eq!(cache.open().draft_files(row).unwrap().len(), 1);
    }

    /// Port of `drafts_row_and_tab_open_in_a_window_and_quitting_saves_them`
    /// (row 104), its tab half: Ctrl+O on a draft tab moves it to a window
    /// (the tab goes), and the main window closing saves what the window's
    /// draft has.
    #[gpui_kit::test]
    fn a_draft_tab_opens_in_a_window_and_closing_saves_it(cx: &mut TestAppContext) {
        let (cache, window, main, _pane) = main_window(cx, "cwin-tab");
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| v.open_draft_tab(Draft::New, w, cx))
        });
        cx.run_until_parked();
        let tab = main.read_with(cx, |v, _| {
            let komorebi::tabs::TabKind::Draft(d) = v.tabs().active_kind() else {
                panic!("a draft tab");
            };
            v.draft_composer(d).expect("its composer")
        });
        let t = tab.clone();
        in_window(cx, window, move |w, cx| {
            t.update(cx, |c, cx| c.focus_field(Field::To, w, cx))
        });
        in_window(cx, window, |w, cx| type_text(w, "ana@example.org", cx));
        cx.run_until_parked();
        in_window(cx, window, |w, cx| press(w, &["ctrl-o"], cx));
        cx.run_until_parked();
        assert_eq!(main.read_with(cx, |v, _| v.tabs().len()), 1, "the tab went");
        let rows = main.read_with(cx, |v, cx| v.draft_windows(cx));
        let row = rows[0].expect("saved");
        let (win, _popped) = main
            .read_with(cx, |v, _| v.draft_window_composer(0))
            .unwrap();
        in_window(cx, win, |w, cx| type_text(w, "Last words", cx));
        cx.run_until_parked();
        // The main window closes: every composer window's draft is saved.
        main.update(cx, |v, cx| v.close_popouts(cx));
        cx.run_until_parked();
        assert!(main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty());
        let saved = cache.open().draft(row).unwrap().expect("saved");
        assert!(saved.body.contains("Last words"), "{}", saved.body);
    }

    /// Port of `tab_switch_flushes_autosave_of_the_leaving_draft` (row 304):
    /// leaving a draft tab saves it at once, not when its autosave delay
    /// ends; leaving the Mail tab saves the pane's draft.
    #[gpui_kit::test]
    fn tab_switch_flushes_autosave_of_the_leaving_draft(cx: &mut TestAppContext) {
        let (cache, window, main, pane) = main_window(cx, "tab-flush");
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| v.open_draft_tab(Draft::New, w, cx))
        });
        cx.run_until_parked();
        let tab = main.read_with(cx, |v, _| {
            let komorebi::tabs::TabKind::Draft(d) = v.tabs().active_kind() else {
                panic!("a draft tab");
            };
            v.draft_composer(d).expect("its composer")
        });
        let t = tab.clone();
        in_window(cx, window, move |w, cx| {
            t.update(cx, |c, cx| c.focus_field(Field::To, w, cx))
        });
        in_window(cx, window, |w, cx| type_text(w, "ana@example.org", cx));
        cx.run_until_parked();
        assert_eq!(
            cache.open().count_drafts().unwrap(),
            0,
            "the autosave waits"
        );
        in_window(cx, window, |w, cx| press(w, &["ctrl-1"], cx));
        cx.run_until_parked();
        assert_eq!(main.read_with(cx, |v, _| v.tabs().active()), 0);
        let db = cache.open();
        let rows = db.draft_ids("").unwrap();
        assert_eq!(rows.len(), 1, "saved as the tab was left");
        assert_eq!(db.draft(rows[0]).unwrap().unwrap().to, "ana@example.org");
        // The pane's draft, as the Mail tab is left for the draft tab.
        let p = pane.clone();
        in_window(cx, window, move |w, cx| {
            p.update(cx, |c, cx| c.open(Some(Draft::New), w, cx))
        });
        cx.run_until_parked();
        in_window(cx, window, |w, cx| type_text(w, "bo@example.org", cx));
        cx.run_until_parked();
        assert_eq!(cache.open().count_drafts().unwrap(), 1);
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| v.select_tab(1, w, cx))
        });
        cx.run_until_parked();
        assert_eq!(cache.open().count_drafts().unwrap(), 2, "the pane's saved");
    }

    /// A draft tab with ana@example.org typed in To; its composer.
    fn typed_draft_tab(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        main: &Entity<crate::window::MainWindow>,
    ) -> Entity<ComposerView> {
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| v.open_draft_tab(Draft::New, w, cx))
        });
        cx.run_until_parked();
        let tab = main.read_with(cx, |v, _| {
            let komorebi::tabs::TabKind::Draft(d) = v.tabs().active_kind() else {
                panic!("a draft tab");
            };
            v.draft_composer(d).expect("its composer")
        });
        let t = tab.clone();
        in_window(cx, window, move |w, cx| {
            t.update(cx, |c, cx| c.focus_field(Field::To, w, cx))
        });
        in_window(cx, window, |w, cx| type_text(w, "ana@example.org", cx));
        cx.run_until_parked();
        tab
    }

    /// Close tabs to the right with a draft tab whose save fails at the
    /// right end: that tab stays with its error, the others go, and the
    /// command ends (it once tried the last tab again for ever).
    #[gpui_kit::test]
    fn close_right_passes_a_draft_tab_that_could_not_be_saved(cx: &mut TestAppContext) {
        let (cache, window, main, _pane) = main_window(cx, "close-right-fails");
        let _fine = typed_draft_tab(cx, window, &main);
        let stuck = typed_draft_tab(cx, window, &main);
        // An account that is not in the cache: its drafts row cannot be
        // written (the foreign key).
        stuck.update(cx, |c, _| {
            c.drafts.active_mut().expect("a draft").account_id = i64::MAX;
        });
        assert_eq!(main.read_with(cx, |v, _| v.tabs().len()), 3);
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| {
                v.tab_command(0, crate::views::tabs::strip::TabCommand::CloseRight, w, cx)
            })
        });
        cx.run_until_parked();
        assert_eq!(main.read_with(cx, |v, _| v.tabs().len()), 2, "one stays");
        assert!(stuck.read_with(cx, |c, _| c.composing()), "with its draft");
        assert!(matches!(
            stuck.read_with(cx, |c, _| c.status.clone()),
            Some(Status::CouldNotSave(_))
        ));
        assert_eq!(cache.open().count_drafts().unwrap(), 1, "the other saved");
    }

    /// The main window closing (or Quit) with a composer window whose save
    /// fails: that window stays, with its draft, and the main window is
    /// told not to close.
    #[gpui_kit::test]
    fn closing_keeps_a_composer_window_that_could_not_be_saved(cx: &mut TestAppContext) {
        let (cache, window, main, pane) = main_window(cx, "cwin-keep");
        let (row, win, popped) = pop_out_pane_draft(cx, window, &main, &pane);
        popped.update(cx, |c, _| {
            c.drafts.active_mut().expect("a draft").account_id = i64::MAX;
        });
        in_window(cx, win, |w, cx| type_text(w, "Kept words", cx));
        cx.run_until_parked();
        assert!(!main.update(cx, |v, cx| v.close_popouts(cx)), "not closed");
        cx.run_until_parked();
        assert_eq!(
            main.read_with(cx, |v, cx| v.draft_windows(cx)),
            vec![Some(row)]
        );
        assert!(text_of(&popped, cx).contains("Kept words"));
        // Saved again: it goes.
        let account = cache.open().accounts().unwrap()[0].id;
        popped.update(cx, |c, _| {
            c.drafts.active_mut().expect("a draft").account_id = account;
        });
        assert!(main.update(cx, |v, cx| v.close_popouts(cx)));
        cx.run_until_parked();
        assert!(main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty());
        let saved = cache.open().draft(row).unwrap().expect("saved");
        assert!(saved.body.contains("Kept words"), "{}", saved.body);
    }

    /// A composer window's send that went out leaves nothing that sends a
    /// later open of the same row to a window: only an Undo of that very
    /// send does.
    #[gpui_kit::test]
    fn a_window_send_that_went_out_sends_no_later_draft_to_a_window(cx: &mut TestAppContext) {
        let (_cache, window, main, pane) = main_window(cx, "cwin-sent");
        let (row, win, popped) = pop_out_pane_draft(cx, window, &main, &pane);
        popped.update(cx, |c, _| c.set_undo_delay(Duration::from_secs(10)));
        let p = popped.clone();
        in_window(cx, win, move |w, cx| {
            p.update(cx, |c, cx| c.send(true, w, cx))
        });
        cx.run_until_parked();
        drop(popped);
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        assert!(
            pane.read_with(cx, |c, _| c.pending_sends().is_empty()),
            "out"
        );
        // The same row opened again, from a tab or by another composer's
        // Undo: a tab, not a window.
        let m = main.clone();
        let other = pane.entity_id();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| {
                v.open_undone_or_aside(row, false, Some(other), w, cx);
            })
        });
        cx.run_until_parked();
        assert!(main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty());
        let m = main.clone();
        in_window(cx, window, move |w, cx| {
            m.update(cx, |v, cx| v.open_draft_tab(Draft::Saved(row), w, cx))
        });
        cx.run_until_parked();
        assert!(main.read_with(cx, |v, cx| v.draft_windows(cx)).is_empty());
    }
}
