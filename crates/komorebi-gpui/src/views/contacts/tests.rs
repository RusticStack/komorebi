//! The Contacts view in a headless window. GPUI-TEST-PORT.md lists no
//! Slint test for phase 9 (the Slint app has none for this view); these
//! check what the view does the way the Slint controller does it
//! (`src/app/contacts.rs`, `contacts_sync.rs`). The page's own rules are
//! the library's tests (`komorebi::contacts_page`, `komorebi::contacts_sync`).
//! No test reaches the network: Find and Find address books are never
//! pressed, and sync passes need `plan`, which needs a found book.

// One test module, so the token lint and the catalog's literal check
// (both skip what a `#[cfg(test)]` is on) leave it out.
#[cfg(test)]
mod port {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, AppContext as _, Entity, Role, TestAppContext};
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use komorebi::contacts_page::Problem;
    use komorebi::db::Db;

    use super::super::list::{self, Item};
    use super::super::{ContactsEvent, ContactsView, Side};
    use crate::test_support::{
        self, TempCache, assert_named, click_named, in_window, press, type_text,
    };

    struct Fixture {
        window: AnyWindowHandle,
        view: Entity<ContactsView>,
        events: Rc<RefCell<Vec<ContactsEvent>>>,
        cache: TempCache,
        account: i64,
    }

    fn open(cx: &mut TestAppContext, name: &str) -> Fixture {
        test_support::init_app(cx);
        let (cache, db, seeded) = test_support::seeded(name, 3);
        komorebi::contacts_page::sample(&db);
        let (window, view) = test_support::open(cx, move |window, cx| {
            cx.new(|cx| {
                let mut v = ContactsView::new(Rc::new(db), window, cx);
                v.opened(window, cx);
                v
            })
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, e: &ContactsEvent, _| {
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

        fn focus(
            &self,
            cx: &mut TestAppContext,
            pick: impl FnOnce(&ContactsView) -> Entity<gpui_kit::component::input::InputState>,
        ) {
            let view = self.view.clone();
            in_window(cx, self.window, move |window, cx| {
                let input = pick(view.read(cx));
                input.update(cx, |s, cx| s.focus(window, cx));
            });
            cx.run_until_parked();
        }

        fn shown_name(&self, cx: &mut TestAppContext) -> Option<String> {
            self.view
                .read_with(cx, |v, _| v.page.shown().map(|d| d.name.clone()))
        }

        fn total(&self, cx: &mut TestAppContext) -> usize {
            self.view.read_with(cx, |v, _| v.page.total())
        }

        fn db(&self) -> Db {
            Db::open(&self.cache.path).expect("open the test cache")
        }

        /// Six more letters of six contacts each, so the list scrolls.
        fn many(&self, cx: &mut TestAppContext) {
            {
                let db = self.db();
                let local = db.local_book().expect("the local book");
                for l in ["Gomes", "Henriques", "Lopes", "Nunes", "Pinto", "Ramos"] {
                    for n in 0..6 {
                        let mut c = komorebi::contacts::Contact {
                            book_id: local,
                            name: format!("Pessoa{n} {l}"),
                            family: l.into(),
                            ..Default::default()
                        };
                        db.save_contact(&mut c).expect("save");
                    }
                }
            }
            self.view.update(cx, |v, cx| v.reload(false, cx));
            cx.run_until_parked();
        }
    }

    /// The list shows every contact with its count; a click shows one,
    /// the keys move it, a search narrows the list and Esc clears it; New
    /// message asks the shell for the composer from the book's account,
    /// Copy puts the address on the clipboard (the Slint controller's
    /// `select_contact`, `contacts_search`, `write_to_contact`,
    /// `copy_contact_text`).
    #[gpui_kit::test]
    fn the_list_searches_shows_and_writes_to_a_contact(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-contacts-list");
        assert_eq!(f.total(cx), 6);
        in_window(cx, f.window, |window, _| {
            assert_eq!(
                window
                    .find_by_label("Bruno Esteves, bruno@example.com")
                    .role(),
                Some(Role::ListItem)
            );
        });
        assert_eq!(f.shown_name(cx), None);

        f.click(cx, ("contact", 1usize));
        assert_eq!(f.shown_name(cx).as_deref(), Some("Bruno Esteves"));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Bruno Esteves", Role::Heading);
            let row = window.find_by_label("Bruno Esteves, bruno@example.com");
            assert_eq!(row.selected(), Some(true));
            // The page's avatar is the shared large spot (72 px).
            let avatar = window.find("contact-avatar").bounds();
            let lg = gpui::px(crate::theme::layout::AVATAR_LG);
            assert_eq!((avatar.size.width, avatar.size.height), (lg, lg));
            let heading = window.find("contact-name").bounds();
            assert!(
                avatar.bottom() <= heading.top(),
                "{avatar:?} over {heading:?}"
            );
        });
        // The list sorts by family name: Esteves, Ferreira, Mendes.
        f.keys(cx, &["down"]);
        assert_eq!(f.shown_name(cx).as_deref(), Some("Diogo Ferreira"));
        f.keys(cx, &["home"]);
        assert_eq!(f.shown_name(cx).as_deref(), Some("Ana Concei\u{e7}\u{e3}o"));

        f.focus(cx, |v| v.search.clone());
        f.typed(cx, "Eva");
        assert_eq!(f.total(cx), 1);
        f.keys(cx, &["escape"]);
        assert_eq!(f.total(cx), 6);

        f.click(cx, ("contact", 1usize));
        f.click(cx, "contact-write");
        f.click(cx, ("contact-copy-email", 0usize));
        let copied = cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(copied.as_deref(), Some("bruno@example.com"));
        assert_eq!(
            *f.events.borrow(),
            [
                ContactsEvent::Compose {
                    account: f.account,
                    to: "Bruno Esteves <bruno@example.com>".into()
                },
                ContactsEvent::Copied
            ]
        );
        in_window(cx, f.window, |window, _| {
            assert_eq!(window.find_by_label("Copied").role(), Some(Role::Status));
        });
    }

    /// The rows sit under their letter's header (the mockup's `.letter`),
    /// the family name's letter as the list sorts; Home brings the first
    /// header into view with its row.
    #[gpui_kit::test]
    fn rows_sit_under_their_letter(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-contacts-letters");
        in_window(cx, f.window, |window, _| {
            for letter in ["C", "E", "F", "M"] {
                assert_named(window, letter, Role::Heading);
            }
        });
        f.view.update(cx, |v, _| {
            let entries = v.entries();
            let items = entries.items();
            assert_eq!(items.len(), 12);
            assert_eq!(items[0], Item::Letter('C'));
            assert_eq!(items[1], Item::Row(0));
            assert_eq!(items[2], Item::Letter('E'));
            assert_eq!(entries.item_of(0), 0);
            assert_eq!(entries.item_of(1), 2);
        });
    }

    /// Contacts with no family name (a shown name only, as FN-only vCards
    /// and contacts typed in by hand) list under their name's letter among
    /// the family names, each letter's header once and in order (review:
    /// 'B' came twice and the letters out of order).
    #[gpui_kit::test]
    fn mixed_contacts_keep_one_header_per_letter(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-contacts-mixed");
        {
            let db = f.db();
            let local = db.local_book().expect("the local book");
            for (name, family) in [
                ("Bruno", ""),
                ("Ze To", ""),
                ("Rui Almeida", "Almeida"),
                ("Ana Borges", "Borges"),
            ] {
                let mut c = komorebi::contacts::Contact {
                    book_id: local,
                    name: name.into(),
                    family: family.into(),
                    ..Default::default()
                };
                db.save_contact(&mut c).expect("save");
            }
        }
        f.view.update(cx, |v, cx| v.reload(false, cx));
        cx.run_until_parked();
        let letters = f.view.update(cx, |v, _| {
            v.entries()
                .items()
                .iter()
                .filter_map(|i| match i {
                    Item::Letter(l) => Some(*l),
                    Item::Row(_) => None,
                })
                .collect::<Vec<_>>()
        });
        let mut sorted = letters.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(letters, sorted, "one header per letter, in order");
        for l in ['A', 'B', 'Z'] {
            assert!(letters.contains(&l), "{l} in {letters:?}");
        }
        let names = f.view.read_with(cx, |v, _| {
            v.page
                .rows
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
        });
        let at = |n: &str| names.iter().position(|x| x == n).expect(n);
        assert!(at("Rui Almeida") < at("Ana Borges"));
        assert!(at("Ana Borges") < at("Bruno"));
        assert!(at("Bruno") < at("Ze To"));
    }

    /// Once the list scrolls, the letter of the rows at the top stays over
    /// the list (a sticky header, not a second heading); the wheel over it
    /// still scrolls the list, and back at the top it goes.
    #[gpui_kit::test]
    fn the_letter_sticks_over_the_scrolled_list(cx: &mut TestAppContext) {
        use gpui::{ScrollDelta, point, px};

        let f = open(cx, "gpui-contacts-sticky");
        f.many(cx);
        in_window(cx, f.window, |window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("contacts-sticky-letter").is_none());
            window.scroll(
                "contacts-rows",
                ScrollDelta::Pixels(point(px(0.), px(-600.))),
                cx,
            );
            window.render_frame(cx);
        });
        let (scrolled, sticky) = f.view.update(cx, |v, cx| {
            use crate::theme::ActiveTokens as _;
            let t = cx.tokens().clone();
            let scrolled = -f32::from(v.list_scroll.offset().y);
            let (row_h, letter_h) = (list::row_h(&t), list::letter_h(&t));
            (scrolled, v.entries().sticky(row_h, letter_h, scrolled))
        });
        assert!(scrolled > 0., "the list scrolled");
        let sticky = sticky.expect("a sticky letter once scrolled");
        assert!(('E'..='R').contains(&sticky.letter), "{sticky:?}");
        in_window(cx, f.window, |window, cx| {
            let shown = window.find("contacts-sticky-letter");
            let list = window.find("contacts-rows").bounds();
            assert!(shown.bounds().top() <= list.top(), "{:?}", shown.bounds());
            assert!(shown.bounds().bottom() > list.top());
            // Only the list's own headers are headings.
            assert_ne!(shown.role(), Some(Role::Heading));
            // The wheel over the sticky header scrolls the list on.
            window.scroll(
                "contacts-sticky-letter",
                ScrollDelta::Pixels(point(px(0.), px(-100.))),
                cx,
            );
            window.render_frame(cx);
        });
        let further = f
            .view
            .read_with(cx, |v, _| -f32::from(v.list_scroll.offset().y));
        assert!(further > scrolled, "{further} after {scrolled}");
        in_window(cx, f.window, |window, cx| {
            window.scroll(
                "contacts-rows",
                ScrollDelta::Pixels(point(px(0.), px(100_000.))),
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("contacts-sticky-letter").is_none());
        });
    }

    /// The frame a key or a search draws already has the sticky header for
    /// where the list went (review: the virtual list moved only when it
    /// laid out, after the render that drew the header, so Home left 'R'
    /// over 'A'), and the current row is never under it (WCAG 2.4.11).
    #[gpui_kit::test]
    fn keys_and_search_keep_the_sticky_letter_true(cx: &mut TestAppContext) {
        use gpui::{ScrollDelta, point, px};

        let f = open(cx, "gpui-contacts-sticky-keys");
        f.many(cx);
        let view = f.view.clone();
        in_window(cx, f.window, |window, cx| {
            window.scroll(
                "contacts-rows",
                ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("contacts-sticky-letter").is_some());
            view.update(cx, |v, cx| v.list_focus.focus(window, cx));
            // Home from the bottom (what its key calls), then one frame:
            // that frame, not a later one, is clear (the test window draws
            // again after a key, the app's window does not).
            view.update(cx, |v, cx| v.select(Some(0), cx));
            window.render_frame(cx);
            assert!(
                window.try_find("contacts-sticky-letter").is_none(),
                "the sticky letter left over the top of the list"
            );
            // Up from the bottom through every row: the current row stays
            // whole below the sticky header.
            press(window, &["end"], cx);
            let total = view.read(cx).page.rows.len();
            for _ in 1..total {
                press(window, &["up"], cx);
                let at = view.read(cx).page.current().expect("a row is current");
                let row = window.find(("contact", at)).bounds();
                let list = window.find("contacts-rows").bounds();
                assert!(row.top() >= list.top(), "row {at} {row:?} above {list:?}");
                assert!(
                    row.bottom() <= list.bottom(),
                    "row {at} {row:?} below {list:?}"
                );
                if let Some(sticky) = window.try_find("contacts-sticky-letter") {
                    assert!(
                        row.top() >= sticky.bounds().bottom(),
                        "row {at} {row:?} under the sticky letter {:?}",
                        sticky.bounds()
                    );
                }
            }
            // A search that leaves too few rows to scroll: the frame it
            // draws has no sticky letter.
            window.scroll(
                "contacts-rows",
                ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
                cx,
            );
            window.render_frame(cx);
            assert!(window.try_find("contacts-sticky-letter").is_some());
            // As the search field's change does (that runs once the
            // window's update ends, after this check).
            view.update(cx, |v, cx| {
                v.page.query = "Pessoa0 Ramos".into();
                v.reload(false, cx);
            });
            window.render_frame(cx);
            assert_eq!(view.read(cx).page.rows.len(), 1);
            assert!(
                window.try_find("contacts-sticky-letter").is_none(),
                "the sticky letter left over a list that no longer scrolls"
            );
        });
    }

    /// At 150 and 225 percent text each row's name and address lie inside
    /// the row as it is laid out (stage D: the 56 px row clipped the
    /// address).
    #[gpui_kit::test]
    fn rows_hold_both_lines_at_large_text(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-contacts-large-text");
        for scale in [1.0, 1.5, 2.25] {
            cx.update(|cx| {
                crate::theme::install(
                    crate::theme::ThemeInput::new(
                        komorebi::db::ThemeChoice::Light,
                        komorebi::theme::SystemTheme {
                            text_scale: scale,
                            animate: false,
                            ..Default::default()
                        },
                    ),
                    cx,
                );
            });
            cx.run_until_parked();
            in_window(cx, f.window, |window, _| {
                let row = window.find(("contact", 0usize)).bounds();
                for line in ["contact-name", "contact-email"] {
                    let b = window.find((line, 0usize)).bounds();
                    assert!(
                        b.top() >= row.top() && b.bottom() <= row.bottom(),
                        "{line} {b:?} outside the row {row:?} at {scale}"
                    );
                    assert!(b.size.height > gpui::px(0.), "{line} at {scale}");
                }
            });
        }
    }

    /// New contact opens the editor; it says what is wrong and saves to
    /// the local book; Edit and Cancel ask before throwing changes away;
    /// Delete asks first (the Slint controller's `save_contact`,
    /// `cancel_contact`, `delete_contact`).
    #[gpui_kit::test]
    fn contacts_are_added_edited_and_deleted(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-contacts-edit");
        f.click(cx, "contacts-new");
        f.view
            .read_with(cx, |v, _| assert!(matches!(v.side, Side::Edit(_))));
        // "New contact" names both the list's button and the editor's
        // heading, so the heading is found by its id.
        in_window(cx, f.window, |window, _| {
            assert_eq!(
                window.find("contact-edit-title").role(),
                Some(Role::Heading)
            );
        });
        f.click(cx, "contact-save");
        f.view.read_with(cx, |v, _| {
            let Side::Edit(e) = &v.side else {
                panic!("the editor stays open");
            };
            assert_eq!(e.problem, Some(Problem::Empty));
        });

        // The name field has the focus.
        f.typed(cx, "Marta Silva");
        f.focus(cx, |v| match &v.side {
            Side::Edit(e) => e.emails[0].clone(),
            _ => panic!("editing"),
        });
        f.typed(cx, "Marta@Example.PT");
        f.click(cx, "contact-add-phone");
        f.typed(cx, "+351 912 000 000");
        // Each line's field and remove button carry the line's number.
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Email 1", Role::TextInput);
            assert_named(window, "Remove email 1", Role::Button);
            assert_named(window, "Phone 1", Role::TextInput);
            assert_named(window, "Remove phone 1", Role::Button);
        });
        f.click(cx, "contact-save");
        f.view
            .read_with(cx, |v, _| assert!(matches!(v.side, Side::Detail)));
        assert_eq!(f.total(cx), 7);
        assert_eq!(f.shown_name(cx).as_deref(), Some("Marta Silva"));
        let id = f.view.read_with(cx, |v, _| {
            let d = v.page.shown().expect("shown");
            assert_eq!(d.contact.emails[0].addr, "marta@example.pt");
            assert_eq!(d.contact.phones.len(), 1);
            d.contact.id
        });

        // A change, then Cancel: Discard asks first.
        f.click(cx, "contact-edit");
        f.typed(cx, " Costa");
        f.click(cx, "contact-cancel");
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx))
        });
        f.click(cx, "dialog-confirm");
        f.view
            .read_with(cx, |v, _| assert!(matches!(v.side, Side::Detail)));
        let stored = f.db().contact(id).expect("read").expect("stored");
        assert_eq!(stored.name, "Marta Silva");

        // No change: Cancel closes at once.
        f.click(cx, "contact-edit");
        f.click(cx, "contact-cancel");
        in_window(cx, f.window, |window, cx| {
            assert!(!window.has_active_dialog(cx))
        });

        // Delete asks first.
        f.click(cx, "contact-delete");
        in_window(cx, f.window, |window, cx| {
            assert!(window.has_active_dialog(cx))
        });
        assert_eq!(f.total(cx), 7);
        f.click(cx, "dialog-confirm");
        assert_eq!(f.total(cx), 6);
        assert!(f.db().contact(id).expect("read").is_none());
        assert_eq!(f.shown_name(cx), None);
    }

    /// The sync pane opens from the Sync button (one press, named "Sync"
    /// with the tooltip "Address book sync", as the Slint app's); choosing a mode saves the
    /// account's settings; Off with nothing synced asks nothing; Find
    /// address books waits for a server address (the Slint controller's
    /// `open_sync_pane`, `s_mode_chosen`).
    #[gpui_kit::test]
    fn the_sync_pane_saves_the_mode_and_waits_for_an_address(cx: &mut TestAppContext) {
        let f = open(cx, "gpui-contacts-sync");
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Sync", Role::Button);
        });
        f.click(cx, "contacts-sync");
        f.view
            .read_with(cx, |v, _| assert!(matches!(v.side, Side::Sync(_))));
        in_window(cx, f.window, |window, _| {
            assert_named(window, "Address book sync", Role::Heading);
        });

        f.click_named(cx, "Sync address books: Off");
        f.click_named(cx, "Read only");
        let mode = |f: &Fixture| f.db().dav_account(f.account).ok().flatten().map(|d| d.mode);
        assert_eq!(mode(&f), Some(1));
        in_window(cx, f.window, |window, _| {
            assert!(
                window
                    .try_find_by_label("Sync address books: Read only")
                    .is_some()
            );
        });
        f.click(cx, "sync-find-books");
        // Disabled without a server address: nothing was asked of anyone.
        f.view.read_with(cx, |v, _| assert!(v.busy.is_empty()));

        f.click_named(cx, "Sync address books: Read only");
        f.click_named(cx, "Off");
        in_window(cx, f.window, |window, cx| {
            assert!(!window.has_active_dialog(cx))
        });
        assert_eq!(mode(&f), Some(0));

        f.keys(cx, &["escape"]);
        f.view
            .read_with(cx, |v, _| assert!(matches!(v.side, Side::Detail)));
    }
}

#[cfg(test)]
mod rows {
    use komorebi::db::ThemeChoice;
    use komorebi::db::contacts::ContactRow;
    use komorebi::theme::SystemTheme;

    use super::super::list::{Entries, Item, items, letter_h, row_h};
    use crate::theme::{ThemeInput, tokens};

    fn row(id: i64, letter: char) -> ContactRow {
        ContactRow {
            id,
            name: format!("n{id}"),
            email: String::new(),
            letter,
        }
    }

    /// A header before the first row of each letter; scrolling to a
    /// letter's first row shows its header.
    #[test]
    fn a_header_starts_each_letter() {
        let rows = [row(1, 'A'), row(2, 'A'), row(3, 'B'), row(4, '#')];
        assert_eq!(
            items(&rows),
            [
                Item::Letter('A'),
                Item::Row(0),
                Item::Row(1),
                Item::Letter('B'),
                Item::Row(2),
                Item::Letter('#'),
                Item::Row(3),
            ]
        );
        let entries = Entries::new(&rows);
        assert_eq!(
            (0..4).map(|i| entries.item_of(i)).collect::<Vec<_>>(),
            [0, 2, 3, 5]
        );
        assert!(items(&[]).is_empty());
    }

    /// The sticky header: none at the top; the letter of the entry at the
    /// top edge once scrolled (a row's or a header's own); pushed up by the
    /// next header as it comes within a header's height, and not past it.
    #[test]
    fn the_sticky_letter_follows_the_top_row() {
        // Tops with a 20 px header and 50 px rows: A 0, r0 20, r1 70,
        // B 120, r2 140, # 190, r3 210.
        let rows = [row(1, 'A'), row(2, 'A'), row(3, 'B'), row(4, '#')];
        let mut e = Entries::new(&rows);
        let at = |e: &mut Entries, y: f32| e.sticky(50., 20., y);
        assert_eq!(at(&mut e, 0.), None);
        let a = at(&mut e, 10.).expect("A");
        assert_eq!((a.letter, a.shift), ('A', 0.));
        assert_eq!(at(&mut e, 75.).map(|s| s.letter), Some('A'));
        // B's header is 10 px below the top: A is pushed up by 10.
        let pushed = at(&mut e, 110.).expect("A pushed");
        assert_eq!((pushed.letter, pushed.shift), ('A', -10.));
        // B's header at the top edge: B, in place.
        let b = at(&mut e, 120.).expect("B");
        assert_eq!((b.letter, b.shift), ('B', 0.));
        let last = at(&mut e, 500.).expect("#");
        assert_eq!((last.letter, last.shift), ('#', 0.));
        assert_eq!(Entries::new(&[]).sticky(50., 20., 30.), None);
    }

    /// The row and header heights are the mockup's at 100 percent and
    /// grow with the text size (that the lines fit is
    /// `rows_hold_both_lines_at_large_text`, on the laid-out rows).
    #[test]
    fn rows_grow_with_the_text_size() {
        let at = |scale| {
            tokens(&ThemeInput::new(
                ThemeChoice::Light,
                SystemTheme {
                    text_scale: scale,
                    ..SystemTheme::default()
                },
            ))
        };
        let normal = at(1.0);
        assert_eq!(row_h(&normal), crate::theme::layout::CONTACT_ROW_H);
        for scale in [1.5, 2.25] {
            let t = at(scale);
            assert!(row_h(&t) > row_h(&normal), "{scale}");
            assert!(letter_h(&t) > letter_h(&normal), "{scale}");
        }
    }
}
