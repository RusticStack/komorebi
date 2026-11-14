//! Settings > Rules in a headless window: the port of the Slint view test
//! `rules_are_saved_toggled_reordered_and_deleted_in_settings`
//! (src/app/composer/tests/rules_tests.rs, docs/GPUI-TEST-PORT.md row
//! 291). The page's own rules are the library's tests
//! (`komorebi::rules_page`, with row 227).

// One test module, so the token lint and the catalog's literal check
// (both skip what a `#[cfg(test)]` is on) leave it out.
#[cfg(test)]
mod port {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, AppContext as _, Entity, Role, TestAppContext};
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use komorebi::db::Db;
    use komorebi::rules::Field;
    use komorebi::rules_page::Problem;

    use super::super::{RulesEvent, RulesView};
    use crate::test_support::{
        self, TempCache, assert_named, click_named, in_window, press, type_text,
    };

    struct Fixture {
        window: AnyWindowHandle,
        view: Entity<RulesView>,
        events: Rc<RefCell<Vec<RulesEvent>>>,
        cache: TempCache,
        account: i64,
    }

    fn open(cx: &mut TestAppContext, name: &str) -> Fixture {
        test_support::init_app(cx);
        let (cache, db, seeded) = test_support::seeded(name, 5);
        let (window, view) = test_support::open(cx, move |window, cx| {
            cx.new(|cx| {
                let mut v = RulesView::new(Rc::new(db), window, cx);
                v.reload(cx);
                v
            })
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, e: &RulesEvent, _| {
                sink.borrow_mut().push(e.clone());
            })
            .detach();
        });
        Fixture {
            window,
            view,
            events,
            cache,
            account: seeded.account,
        }
    }

    impl Fixture {
        fn click(&self, cx: &mut TestAppContext, id: impl Into<gpui::ElementId>) {
            let id = id.into();
            in_window(cx, self.window, move |window, cx| window.click(id, cx));
            cx.run_until_parked();
        }

        fn click_named(&self, cx: &mut TestAppContext, name: &str) {
            in_window(cx, self.window, |window, cx| click_named(window, name, cx));
            cx.run_until_parked();
        }

        fn typed(&self, cx: &mut TestAppContext, text: &str) {
            in_window(cx, self.window, |window, cx| type_text(window, text, cx));
            cx.run_until_parked();
        }

        fn keys(&self, cx: &mut TestAppContext, keys: &[&str]) {
            in_window(cx, self.window, |window, cx| press(window, keys, cx));
            cx.run_until_parked();
        }

        /// Focus condition `i`'s value field (as a click into it would).
        fn focus_value(&self, cx: &mut TestAppContext, i: usize) {
            let view = self.view.clone();
            in_window(cx, self.window, move |window, cx| {
                let input = view.read(cx).rule.as_ref().expect("open").values[i].clone();
                input.update(cx, |s, cx| s.focus(window, cx));
            });
            cx.run_until_parked();
        }

        fn problem(&self, cx: &mut TestAppContext) -> Option<Problem> {
            self.view
                .read_with(cx, |v, _| v.rule.as_ref().and_then(|r| r.problem))
        }

        fn editing(&self, cx: &mut TestAppContext) -> bool {
            self.view.read_with(cx, |v, _| v.is_editing())
        }

        fn names(&self) -> Vec<String> {
            Db::open(&self.cache.path)
                .expect("open the test cache")
                .rules_for(self.account)
                .expect("rules")
                .into_iter()
                .map(|r| r.name)
                .collect()
        }
    }

    /// Ported from `rules_are_saved_toggled_reordered_and_deleted_in_settings`
    /// (src/app/composer/tests/rules_tests.rs): saving writes the rule, the
    /// toggle and the order persist, Delete asks first. Driven by clicks and
    /// typing where the Slint test called the globals.
    #[gpui_kit::test]
    fn rules_are_saved_toggled_reordered_and_deleted_in_settings(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-rules-settings");
        f.view
            .read_with(cx, |v, _| assert!(v.page.cards.is_empty()));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Rules", Role::Heading);
        });

        // A new rule needs a value and an action.
        f.click(cx, "rules-add");
        assert!(f.editing(cx));
        f.keys(cx, &["ctrl-a"]);
        f.typed(cx, "GitHub");
        f.click(cx, "rule-save");
        assert_eq!(f.problem(cx), Some(Problem::Value));
        in_window(cx, f.window, |window, _| {
            assert_eq!(
                window.find_by_label("Each condition needs a value.").role(),
                Some(Role::Alert)
            );
        });
        f.focus_value(cx, 0);
        f.typed(cx, "@github.com");
        f.click_named(cx, "Condition 1: From");
        f.click_named(cx, "From");
        f.click_named(cx, "Condition 1 comparison: contains");
        f.click_named(cx, "ends with");
        f.click(cx, "rule-save");
        assert_eq!(f.problem(cx), Some(Problem::Action));
        f.click_named(cx, "Mark as read");
        f.click(cx, "rule-save");
        assert!(!f.editing(cx));
        f.view.read_with(cx, |v, _| {
            assert_eq!(v.page.cards.len(), 1);
            assert_eq!(
                super::super::text::summary(&v.page.cards[0].parts),
                "From ends with @github.com \u{b7} Mark as read"
            );
        });
        in_window(cx, f.window, |window, _| {
            assert_eq!(
                window.find_by_label("Edit GitHub").role(),
                Some(Role::Button)
            );
            assert_eq!(
                window.find_by_label("More actions for GitHub").role(),
                Some(Role::Button)
            );
        });

        // A second one with two conditions, the second removed again.
        f.click(cx, "rules-add");
        f.keys(cx, &["ctrl-a"]);
        f.typed(cx, "Invoices");
        f.click_named(cx, "Condition 1: From");
        f.click_named(cx, "Subject");
        f.focus_value(cx, 0);
        f.typed(cx, "invoice");
        f.click_named(cx, "Add condition");
        f.view.read_with(cx, |v, _| {
            assert_eq!(v.rule.as_ref().expect("open").form.conds.len(), 2);
        });
        // Each row's controls carry the row's number.
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Condition 2 value", Role::TextInput);
            assert_named(window, "Remove condition 2", Role::Button);
            assert_named(window, "Remove condition 1", Role::Button);
        });
        f.view
            .update(cx, |v, cx| v.set_field(1, Field::HasAttachment, cx));
        f.click(cx, ("rule-cond-remove", 1usize));
        f.view.read_with(cx, |v, _| {
            let r = v.rule.as_ref().expect("open");
            assert_eq!((r.form.conds.len(), r.values.len()), (1, 1));
        });
        f.click_named(cx, "Mark as important");
        f.click(cx, "rule-save");
        assert!(!f.editing(cx));

        assert_eq!(f.names(), ["GitHub", "Invoices"]);
        let db = Db::open(&f.cache.path).expect("cache");
        let saved = &db.rules_for(f.account).expect("rules")[1];
        assert_eq!(saved.conditions.len(), 1);
        assert_eq!(saved.conditions[0].value, "invoice");
        assert!(saved.flag && !saved.mark_read);

        // Off, then down: both persist.
        f.click(cx, ("rule-toggle", 0usize));
        assert!(!db.rules_for(f.account).expect("rules")[0].enabled);
        f.view
            .read_with(cx, |v, _| assert!(!v.page.cards[0].enabled));
        f.click(cx, ("rule-more", 0usize));
        f.click_named(cx, "Move down");
        assert_eq!(f.names(), ["Invoices", "GitHub"]);

        // A card dragged by its grip onto another takes its place, and back.
        in_window(cx, f.window, |window, cx| {
            window.drag_to(("rule-grip", 1usize), ("rule", 0usize), cx)
        });
        cx.run_until_parked();
        assert_eq!(f.names(), ["GitHub", "Invoices"]);
        in_window(cx, f.window, |window, cx| {
            window.drag_to(("rule-grip", 0usize), ("rule", 1usize), cx)
        });
        cx.run_until_parked();
        assert_eq!(f.names(), ["Invoices", "GitHub"]);
        // A drag that starts on Edit or the switch moves nothing.
        for from in ["rule-open", "rule-toggle"] {
            in_window(cx, f.window, |window, cx| {
                window.drag_to((from, 1usize), ("rule", 0usize), cx)
            });
            cx.run_until_parked();
            assert_eq!(f.names(), ["Invoices", "GitHub"], "{from}");
        }
        // A drop names the rules by id: a rule gone since moves nothing.
        f.view
            .update(cx, |v, cx| v.drop_rule(-1, v.page.cards[0].id, cx));
        assert_eq!(f.names(), ["Invoices", "GitHub"]);

        // Delete asks first.
        f.click(cx, ("rule-more", 1usize));
        f.click_named(cx, "Delete GitHub");
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert_eq!(window.find_by_label("Delete").role(), Some(Role::Button));
        });
        assert_eq!(f.names().len(), 2);
        f.click(cx, "dialog-confirm");
        assert_eq!(f.names(), ["Invoices"]);
        f.view
            .read_with(cx, |v, _| assert_eq!(v.page.cards.len(), 1));
        assert!(f.events.borrow().is_empty());
    }

    /// While a card is dragged by its grip, a line in the gap shows where
    /// it lands: above the card it moves up onto, below the one it moves
    /// down onto, none over its own card; the line goes with the drop. The
    /// grip is Fluent's re-order glyph, named for Narrator by its rule.
    #[gpui_kit::test]
    fn a_drag_shows_where_the_rule_lands(cx: &mut TestAppContext) {
        use gpui::{
            InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
            Pixels, Point,
        };

        let f = open(cx, "gpui-rules-drop-line");
        {
            let db = Db::open(&f.cache.path).expect("cache");
            for name in ["One", "Two", "Three"] {
                let rule = komorebi::rules::Rule {
                    name: name.into(),
                    enabled: true,
                    mark_read: true,
                    conditions: vec![komorebi::rules::Condition {
                        field: Field::From,
                        op: komorebi::rules::Op::Contains,
                        value: "a@b.c".into(),
                    }],
                    ..Default::default()
                };
                db.save_rule(f.account, &rule).expect("save");
            }
        }
        f.view.update(cx, |v, cx| v.reload(cx));
        cx.run_until_parked();
        assert_eq!(f.names(), ["One", "Two", "Three"]);
        in_window(cx, f.window, |window, _| {
            assert_eq!(
                window.find_by_label("Drag to reorder Two").role(),
                Some(Role::Image)
            );
        });

        let center =
            |window: &gpui::Window, id: (&'static str, usize)| window.find(id).bounds().center();
        let move_to = |window: &mut gpui::Window, at: Point<Pixels>, cx: &mut gpui::App| {
            window.dispatch_event(
                MouseMoveEvent {
                    position: at,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        };
        // Press on Three's grip and move up onto One: the line is above One.
        in_window(cx, f.window, |window, cx| {
            window.render_frame(cx);
            let from = center(window, ("rule-grip", 2));
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: from,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let to = center(window, ("rule", 0));
            for step in 1..=8 {
                let k = step as f32 / 8.;
                move_to(window, from + (to - from) * k, cx);
            }
            let card = window.find(("rule", 0usize)).bounds();
            let line = window.find(("rule-drop-line", 0usize)).bounds();
            assert!(line.bottom() <= card.top(), "{line:?} above {card:?}");
            assert!(window.try_find(("rule-drop-line", 1usize)).is_none());

            // Over its own card: no line.
            move_to(window, center(window, ("rule", 2)), cx);
            assert!(window.try_find(("rule-drop-line", 0usize)).is_none());
            assert!(window.try_find(("rule-drop-line", 2usize)).is_none());

            // Back onto Two (one up): the line is above Two, then the drop.
            let to = center(window, ("rule", 1));
            move_to(window, to, cx);
            let card = window.find(("rule", 1usize)).bounds();
            let line = window.find(("rule-drop-line", 1usize)).bounds();
            assert!(line.bottom() <= card.top(), "{line:?} above {card:?}");
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: to,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find(("rule-drop-line", 1usize)).is_none());
        });
        cx.run_until_parked();
        assert_eq!(f.names(), ["One", "Three", "Two"]);
        f.view.read_with(cx, |v, _| assert_eq!(v.drop_at, None));

        // Dragging One down onto Three: the line is below Three.
        in_window(cx, f.window, |window, cx| {
            window.render_frame(cx);
            let from = center(window, ("rule-grip", 0));
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: from,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let to = center(window, ("rule", 1));
            for step in 1..=8 {
                let k = step as f32 / 8.;
                move_to(window, from + (to - from) * k, cx);
            }
            let card = window.find(("rule", 1usize)).bounds();
            let line = window.find(("rule-drop-line", 1usize)).bounds();
            assert!(line.top() >= card.bottom(), "{line:?} below {card:?}");
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: to,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        });
        cx.run_until_parked();
        assert_eq!(f.names(), ["Three", "One", "Two"]);

        // A drag released off every card (no move out first, as when the
        // pointer leaves fast): no drop, and the line goes once GPUI ends
        // the drag (it redraws the window on that mouse up).
        let view = f.view.clone();
        in_window(cx, f.window, |window, cx| {
            window.render_frame(cx);
            let from = center(window, ("rule-grip", 2));
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: from,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let to = center(window, ("rule", 0));
            for step in 1..=8 {
                let k = step as f32 / 8.;
                move_to(window, from + (to - from) * k, cx);
            }
            assert!(window.try_find(("rule-drop-line", 0usize)).is_some());
            let off = window.find("rules-count").bounds().center();
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: off,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            assert!(!cx.has_active_drag());
            window.render_frame(cx);
            for i in 0..3usize {
                assert!(
                    window.try_find(("rule-drop-line", i)).is_none(),
                    "line at {i}"
                );
            }
            assert_eq!(view.read(cx).drop_at, None);
        });
        cx.run_until_parked();
        assert_eq!(f.names(), ["Three", "One", "Two"]);
    }

    /// Esc and Back leave a rule's page without saving; Copy as Sieve puts
    /// the script on the clipboard and says so.
    #[gpui_kit::test]
    fn cancel_leaves_the_page_and_sieve_goes_to_the_clipboard(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-rules-sieve");
        f.click(cx, "rules-add");
        f.keys(cx, &["escape"]);
        assert!(!f.editing(cx));
        assert!(f.names().is_empty());

        f.view.update(cx, |v, cx| {
            komorebi::rules_page::sample(&v.db);
            v.reload(cx);
        });
        // Samples are written only offline; seed one rule otherwise.
        if f.names().is_empty() {
            f.click(cx, "rules-add");
            f.focus_value(cx, 0);
            f.typed(cx, "a@b.c");
            f.click_named(cx, "Mark as read");
            f.click(cx, "rule-save");
        }
        f.click(cx, ("rule-open", 0usize));
        assert!(f.editing(cx));
        f.click(cx, "rule-back");
        assert!(!f.editing(cx));

        f.click(cx, "rules-sieve");
        let copied = cx.read_from_clipboard().and_then(|c| c.text());
        assert!(copied.is_some_and(|s| s.contains("require")));
        assert_eq!(*f.events.borrow(), [RulesEvent::SieveCopied]);
        in_window(cx, f.window, |window, _| {
            assert!(window
                .try_find_by_label(
                    "Copied as a Sieve script. Paste it into your provider\u{2019}s mail filters."
                )
                .is_some());
        });
    }
}
