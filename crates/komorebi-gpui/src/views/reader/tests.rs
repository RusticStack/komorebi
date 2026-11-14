//! The reader view's tests: the Slint reader's view tests and the App
//! tests of the reader (GPUI-TEST-PORT.md rows 12, 83, 132, 138, 139,
//! 207, 210, 212, 216, 220, 224), in a headless Kit window with a fake
//! render worker (the test plays the worker: it reads the jobs and sends
//! the replies) and a shell that opens nothing. The logic itself is
//! tested in the library (`komorebi::reader::session`, `worker`).

// One inner module, so the token and catalog lints see test code (they
// skip what follows a `#[cfg(test)]`).
#[cfg(test)]
mod view {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures::channel::mpsc;
    use gpui::{
        AnyWindowHandle, AppContext as _, Entity, Role, ScrollDelta, TestAppContext, point, px,
    };
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::{ClickOptions, TestEventExt as _, TestWindowExt as _};
    use komorebi::db::{Db, MarkRead, ThemeChoice};
    use komorebi::mail::render::Rect;
    use komorebi::reader::{Backend, Job, Key, Page, PageLink, Reply, SelectOp, Shell, page_width};

    use crate::test_support::{self, TempCache, assert_named, in_window, press};
    use crate::views::reader::actions::{MenuCmd, SourceUse};
    use crate::views::reader::worker::Incoming;
    use crate::views::reader::*;

    /// Records jobs; the test plays the worker.
    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<Vec<Job>>>);

    impl Backend for Fake {
        fn send(&mut self, job: Job) {
            self.0.borrow_mut().push(job);
        }
    }

    /// Records the URLs it is asked to open.
    #[derive(Clone, Default)]
    struct NoShell(Rc<RefCell<Vec<String>>>);

    impl Shell for NoShell {
        fn open_url(&self, url: &str) {
            self.0.borrow_mut().push(url.into());
        }
    }

    /// A reader in a headless window over a seeded throwaway cache.
    struct Rig {
        window: AnyWindowHandle,
        view: Entity<ReaderView>,
        jobs: Fake,
        opened: NoShell,
        tx: mpsc::UnboundedSender<Incoming>,
        events: Rc<RefCell<Vec<ReaderEvent>>>,
        ids: Vec<i64>,
        cache: TempCache,
    }

    impl Rig {
        fn new(cx: &mut TestAppContext, name: &str) -> Rig {
            test_support::init_app(cx);
            let (cache, db, seeded) = test_support::seeded(name, 3);
            let ids = db.message_ids(&[seeded.inbox], "").unwrap();
            // Bodies are set per test; the reader reads the read delay.
            db.set_setting("mark_read_delay_ms", "0").unwrap();
            drop(db);
            let (jobs, opened) = (Fake::default(), NoShell::default());
            let (tx, rx) = mpsc::unbounded();
            let view_db = cache.open();
            let (backend, shell) = (jobs.clone(), opened.clone());
            let (window, view) = test_support::open(cx, move |window, cx| {
                cx.new(|cx| {
                    ReaderView::with_backend(
                        Rc::new(view_db),
                        Box::new(backend),
                        Box::new(shell),
                        rx,
                        window,
                        cx,
                    )
                })
            });
            let events: Rc<RefCell<Vec<ReaderEvent>>> = Rc::default();
            let sink = events.clone();
            cx.update(|cx| {
                cx.subscribe(&view, move |_, e: &ReaderEvent, _| {
                    sink.borrow_mut().push(e.clone())
                })
                .detach()
            });
            Rig {
                window,
                view,
                jobs,
                opened,
                tx,
                events,
                ids,
                cache,
            }
        }

        fn db(&self) -> Db {
            self.cache.open()
        }

        /// Message `id` gets `raw` as its cached body.
        fn body(&self, id: i64, raw: &str) {
            self.db()
                .set_body(id, raw.as_bytes(), "", false, Default::default())
                .unwrap();
        }

        fn open(&self, cx: &mut TestAppContext, id: i64) {
            let view = self.view.clone();
            in_window(cx, self.window, move |window, cx| {
                view.update(cx, |r, cx| assert!(r.open(id, window, cx)))
            });
            self.frame(cx);
        }

        /// A frame, then what it deferred (the view measures itself).
        fn frame(&self, cx: &mut TestAppContext) {
            in_window(cx, self.window, |_, _| {});
            cx.run_until_parked();
            in_window(cx, self.window, |_, _| {});
            cx.run_until_parked();
        }

        fn reply(&self, cx: &mut TestAppContext, reply: Reply) {
            self.tx.unbounded_send(Incoming::Reply(reply)).unwrap();
            cx.run_until_parked();
            self.frame(cx);
        }

        fn take(&self) -> Vec<Job> {
            std::mem::take(&mut *self.jobs.0.borrow_mut())
        }

        fn events(&self) -> Vec<ReaderEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }

        fn layouts(&self) -> Vec<(u64, Key)> {
            self.take()
                .into_iter()
                .filter_map(|j| match j {
                    Job::Layout {
                        generation, key, ..
                    } => Some((generation, key)),
                    _ => None,
                })
                .collect()
        }
    }

    fn tile_jobs(jobs: Vec<Job>) -> Vec<(u64, Vec<u32>)> {
        jobs.into_iter()
            .filter_map(|j| match j {
                Job::Tiles {
                    generation, tiles, ..
                } => Some((generation, tiles)),
                _ => None,
            })
            .collect()
    }

    fn selects(jobs: Vec<Job>) -> Vec<SelectOp> {
        jobs.into_iter()
            .filter_map(|j| match j {
                Job::Select { op, .. } => Some(op),
                _ => None,
            })
            .collect()
    }

    const HTML: &str = "From: Marta Silva <marta.silva@example.pt>\r\nTo: Ines <ines@example.com>\r\n\
    Subject: Photos from Geres\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
    <p>Hi Ines,</p><p>Here they are.</p>";

    fn page(key: Key, height: f32) -> Page {
        Page {
            width: key.width as f32,
            height,
            bg: [255; 4],
            truncated: false,
            partial: false,
            remote_images: 0,
            pending_images: false,
            failed_images: 0,
            dimmed: key.dim,
        }
    }

    /// Opens an HTML message and answers its layout with a page `height` tall
    /// and `links`: the layout's generation and key.
    fn open_page(
        rig: &Rig,
        cx: &mut TestAppContext,
        height: f32,
        links: Vec<PageLink>,
    ) -> (u64, Key) {
        let id = rig.ids[0];
        rig.body(id, HTML);
        rig.open(cx, id);
        let layouts = rig.layouts();
        let [(generation, key)] = layouts[..] else {
            panic!("expected one layout job, got {layouts:?}");
        };
        rig.reply(
            cx,
            Reply::Page {
                generation,
                page: page(key, height),
                links,
            },
        );
        (generation, key)
    }

    /// Rows 207: the page's tiles follow the view; a resize lays it out once
    /// the resizing pauses (the test clock), at the new column's width.
    #[gpui_kit::test]
    fn tile_requests_follow_scroll_and_resize(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-tiles");
        let (generation, key) = open_page(&rig, cx, 5000.0, vec![]);
        // The 1280 px window less the column's 40 px sides: the widest page.
        assert_eq!(key.width, page_width(1200.0).unwrap());
        assert_eq!(key.width, 960);
        let frame = rig.view.read_with(cx, |r, _| r.session().frame());
        assert!(frame.top < 0.0, "the header is above the page: {frame:?}");
        let requests = tile_jobs(rig.take());
        let [(g, ref tiles)] = requests[..] else {
            panic!("expected one tile job, got {requests:?}");
        };
        assert_eq!(g, generation);
        assert_eq!(
            tiles,
            &komorebi::reader::wanted_tiles(frame.top, frame.height, 5000.0)
        );

        // Scroll down: the tiles under the new view are asked for.
        in_window(cx, rig.window, |window, cx| {
            window.scroll(
                "reader-scroll",
                ScrollDelta::Pixels(point(px(0.), px(-3000.))),
                cx,
            )
        });
        rig.frame(cx);
        let top = rig.view.read_with(cx, |r, _| r.session().frame().top);
        assert!(top > 2000.0, "scrolled: {top}");
        let requests = tile_jobs(rig.take());
        assert!(
            requests
                .iter()
                .any(|(_, t)| t.contains(&((top / komorebi::reader::TILE_H) as u32))),
            "{requests:?}"
        );

        // Narrow the window: one relayout once the resize pauses.
        cx.simulate_window_resize(rig.window, gpui::size(px(1000.), px(800.)));
        rig.frame(cx);
        assert!(rig.layouts().is_empty(), "debounced");
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        let layouts = rig.layouts();
        let [(g2, k2)] = layouts[..] else {
            panic!("expected a relayout, got {layouts:?}");
        };
        assert!(g2 > generation);
        // The column, and the body margin the page reaches into each side.
        assert_eq!(
            k2.width,
            page_width(920.0 + 2. * crate::theme::reader::BODY_MARGIN).unwrap()
        );
    }

    /// Row 210: a press, a drag and a release on the page reach the worker as
    /// page coordinates; Ctrl+A, Ctrl+C and Escape act on the selection;
    /// copied text lands on the clipboard.
    #[gpui_kit::test]
    fn page_gestures_and_keys_reach_the_worker_and_clipboard(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-gestures");
        let (generation, _) = open_page(&rig, cx, 2000.0, vec![]);
        rig.take();
        let origin = in_window(cx, rig.window, |window, _| {
            window.find("reader-page").bounds().origin
        });
        let from = origin + point(px(50.), px(60.));
        let to = origin + point(px(150.), px(80.));
        in_window(cx, rig.window, |window, cx| window.drag(from, to, cx));
        let ops = selects(rig.take());
        let Some(SelectOp::Press { x, y, clicks: 1 }) = ops.first().copied() else {
            panic!("expected a press first, got {ops:?}");
        };
        assert_eq!((x, y), (50.0, 60.0));
        assert_eq!(ops.last(), Some(&SelectOp::Extend { x: 150.0, y: 80.0 }));

        // A highlight from the worker shows; a stale one is ignored.
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 18.0,
        };
        rig.reply(
            cx,
            Reply::Selection {
                generation,
                rects: vec![rect],
            },
        );
        rig.reply(
            cx,
            Reply::Selection {
                generation: generation - 1,
                rects: vec![],
            },
        );
        assert_eq!(
            rig.view.read_with(cx, |r, _| r.session().selection().len()),
            1
        );

        // The press focused the page: its keys act on the selection.
        in_window(cx, rig.window, |window, cx| {
            press(window, &["ctrl-a", "ctrl-c", "escape"], cx)
        });
        assert_eq!(
            selects(rig.take()),
            [SelectOp::All, SelectOp::Copy, SelectOp::Clear]
        );
        rig.reply(
            cx,
            Reply::Copied {
                generation,
                text: "Olá Ana,\nObrigada".into(),
            },
        );
        let copied = cx.read_from_clipboard().and_then(|c| c.text());
        assert_eq!(copied.as_deref(), Some("Olá Ana,\nObrigada"));

        // A double click in place takes a word.
        in_window(cx, rig.window, |window, cx| {
            window.click_with_options(
                "reader-page",
                ClickOptions::new()
                    .with_offset(point(px(30.), px(30.)))
                    .with_count(2),
                cx,
            )
        });
        let clicks: Vec<u8> = selects(rig.take())
            .into_iter()
            .filter_map(|op| match op {
                SelectOp::Press { clicks, .. } => Some(clicks),
                _ => None,
            })
            .collect();
        assert_eq!(clicks, [1, 2]);
    }

    fn link(href: &str, text: &str, y: f32, target: Option<f32>) -> PageLink {
        PageLink {
            href: href.into(),
            text: text.into(),
            rects: vec![Rect {
                x: 20.0,
                y,
                w: 120.0,
                h: 20.0,
            }],
            target,
        }
    }

    /// Rows 212 and 216: links are named for screen readers, show their target
    /// when hovered, open the confirm prompt (Open goes to the shell), and a
    /// `mailto:` link composes.
    #[gpui_kit::test]
    fn links_hover_open_compose_and_take_keyboard_focus(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-links");
        open_page(
            &rig,
            cx,
            3000.0,
            vec![
                link(
                    "https://evil.example.invalid/x",
                    "https://bank.example.com",
                    20.0,
                    None,
                ),
                link("mailto:ana@example.com?subject=Ol%C3%A1", "Ana", 60.0, None),
                link("#end", "End", 100.0, Some(1500.0)),
            ],
        );
        in_window(cx, rig.window, |window, _| {
            assert_named(window, "https://bank.example.com", Role::Link);
            assert_named(window, "Ana", Role::Link);
            assert_named(window, "End", Role::Link);
        });
        let origin = in_window(cx, rig.window, |window, _| {
            window.find("reader-page").bounds().origin
        });
        let on = |y: f32| origin + point(px(70.), px(y + 10.));

        // Hover: the real target in the strip.
        in_window(cx, rig.window, |window, cx| {
            window.pointer_move(on(20.0), None, cx);
        });
        rig.frame(cx);
        let target = rig.view.read_with(cx, |r, _| r.session().target());
        assert_eq!(target, "https://evil.example.invalid/x");

        // A click on the web link asks first, with the host apart.
        in_window(cx, rig.window, |window, cx| {
            window.click_at("reader-page", point(px(70.), px(30.)), cx)
        });
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert_named(window, "evil.example.invalid", Role::Label);
            assert_named(
                window,
                "The link text says \u{201c}https://bank.example.com\u{201d}, but it opens evil.example.invalid.",
                Role::Alert,
            );
            window.click("link-open", cx);
        });
        rig.frame(cx);
        assert_eq!(*rig.opened.0.borrow(), ["https://evil.example.invalid/x"]);
        in_window(cx, rig.window, |window, cx| {
            assert!(!window.has_active_dialog(cx));
        });

        // mailto: composes (past the double-click time).
        cx.executor().advance_clock(Duration::from_secs(1));
        in_window(cx, rig.window, |window, cx| {
            window.click_at("reader-page", point(px(70.), px(70.)), cx)
        });
        rig.frame(cx);
        let compose = rig.events().into_iter().find_map(|e| match e {
            ReaderEvent::Compose(m) => Some(m),
            _ => None,
        });
        let m = compose.expect("a mailto");
        assert_eq!(
            (m.to.as_str(), m.subject.as_str()),
            ("ana@example.com", "Olá")
        );

        // Tab walks the links from the page; Enter follows the anchor.
        in_window(cx, rig.window, |window, cx| {
            press(window, &["tab", "tab", "tab"], cx)
        });
        let focused = rig.view.read_with(cx, |r, _| r.session().focused_link());
        assert_eq!(focused, Some(2));
        in_window(cx, rig.window, |window, cx| press(window, &["enter"], cx));
        rig.frame(cx);
        let top = rig.view.read_with(cx, |r, _| r.session().frame().top);
        assert!((top - 1492.0).abs() < 2.0, "the anchor at the top: {top}");
    }

    /// Row 220: the image bar loads once, laid out again with consent.
    #[gpui_kit::test]
    fn remote_banner_loads_once_and_lays_out_again(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-remote");
        let id = rig.ids[0];
        rig.body(id, HTML);
        rig.open(cx, id);
        let [(generation, key)] = rig.layouts()[..] else {
            panic!("expected one layout job");
        };
        assert!(!key.remote);
        rig.reply(
            cx,
            Reply::Page {
                generation,
                page: Page {
                    remote_images: 3,
                    ..page(key, 600.0)
                },
                links: vec![],
            },
        );
        in_window(cx, rig.window, |window, cx| {
            assert_named(window, "Load images", Role::Button);
            assert_named(window, "Always load from this sender", Role::Button);
            window.click("reader-load-images", cx);
        });
        rig.frame(cx);
        let [(_, key)] = rig.layouts()[..] else {
            panic!("expected a relayout");
        };
        assert!(key.remote);
        // Always: the sender is remembered, with Undo.
        let id2 = rig.ids[1];
        rig.body(id2, HTML);
        rig.open(cx, id2);
        let [(generation, key)] = rig.layouts()[..] else {
            panic!("expected one layout job");
        };
        assert!(!key.remote, "consent was for the other message");
        rig.reply(
            cx,
            Reply::Page {
                generation,
                page: Page {
                    remote_images: 1,
                    ..page(key, 600.0)
                },
                links: vec![],
            },
        );
        in_window(cx, rig.window, |window, cx| {
            window.click("reader-always-load", cx)
        });
        rig.frame(cx);
        // Keyed on the row's From address.
        let sender = rig.db().message(id2).unwrap().unwrap().from_addr;
        assert!(rig.db().remote_allowed(&sender).unwrap());
        in_window(cx, rig.window, |window, cx| {
            assert_named(
                window,
                &format!("Images from {sender} will always load."),
                Role::Status,
            );
            window.click("reader-always-undo", cx);
        });
        rig.frame(cx);
        assert!(!rig.db().remote_allowed(&sender).unwrap());
    }

    /// Row 224: in the dark theme the page is dimmed; Show original colours
    /// lays it out again without; images that could not be loaded are counted.
    #[gpui_kit::test]
    fn failed_images_and_original_colours_reach_the_reader(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-dark");
        test_support::set_theme(cx, ThemeChoice::Dark);
        let id = rig.ids[0];
        rig.body(id, HTML);
        let sender = rig.db().message(id).unwrap().unwrap().from_addr;
        rig.db().allow_remote(&sender).unwrap();
        rig.open(cx, id);
        let [(generation, key)] = rig.layouts()[..] else {
            panic!("expected one layout job");
        };
        assert!(key.remote && key.dark && key.dim);
        rig.reply(
            cx,
            Reply::Page {
                generation,
                page: Page {
                    remote_images: 2,
                    failed_images: 2,
                    ..page(key, 600.0)
                },
                links: vec![],
            },
        );
        in_window(cx, rig.window, |window, _| {
            assert_named(window, "2 images couldn't be loaded.", Role::Status);
        });
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.show_original(true, window, cx))
        });
        let [(_, key)] = rig.layouts()[..] else {
            panic!("expected a relayout");
        };
        assert!(key.dark && !key.dim);
    }

    /// Rows 12 and 132: an unread message is marked read once it has been
    /// open for the delay, and not when the reader moved on first.
    #[gpui_kit::test]
    fn opened_messages_are_marked_read_after_the_delay(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-read");
        rig.view.update(cx, |r, _| {
            r.set_mark_read(MarkRead::After(Duration::from_secs(1)))
        });
        for id in &rig.ids {
            rig.body(*id, HTML);
        }
        let unread = rig.ids.clone();
        rig.db().set_seen_many(&unread, false).unwrap();
        rig.open(cx, unread[0]);
        rig.open(cx, unread[1]);
        rig.events();
        cx.executor().advance_clock(Duration::from_millis(1100));
        cx.run_until_parked();
        let marked: Vec<i64> = rig
            .events()
            .into_iter()
            .filter_map(|e| match e {
                ReaderEvent::MarkRead(id) => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(marked, [unread[1]], "only the one still open");
        // At once.
        rig.view.update(cx, |r, _| r.set_mark_read(MarkRead::Now));
        rig.open(cx, unread[0]);
        assert!(rig.events().contains(&ReaderEvent::MarkRead(unread[0])));
    }

    /// Row 83: the importance its sender set is said under the header.
    #[gpui_kit::test]
    fn the_reader_shows_the_senders_importance(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-importance");
        let id = rig.ids[0];
        rig.body(
            id,
            "From: Bob <bob@example.org>\r\nSubject: Now\r\nImportance: high\r\n\r\nPlease.\r\n",
        );
        rig.open(cx, id);
        in_window(cx, rig.window, |window, _| {
            assert_named(window, "Sent with high importance", Role::Label);
        });
        // The row keeps it.
        assert_eq!(
            rig.db().message(id).unwrap().unwrap().importance,
            komorebi::db::Importance::High
        );
    }

    /// Row 138: the sender notice, the encrypted state and Unsubscribe.
    #[gpui_kit::test]
    fn notice_encryption_and_unsubscribe_in_the_reader(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-notice");
        let id = rig.ids[0];
        rig.body(
            id,
            "Authentication-Results: mx.example.invalid; spf=pass; dmarc=fail\r\n\
         From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Lunch\r\n\
         List-Unsubscribe: <https://list.example.org/u/1>,\r\n \
         <mailto:leave@list.example.org?subject=leave%20me>\r\n\r\nNoon?\r\n",
        );
        // The account's server is the seed's; DMARC failed there.
        rig.view.update(cx, |r, cx| {
            let mut accounts = r.accounts.clone();
            accounts[0].imap.host = "mx.example.invalid".into();
            r.set_accounts(accounts, cx);
        });
        rig.open(cx, id);
        in_window(cx, rig.window, |window, cx| {
            assert_named(
                window,
                "This message may not be from who it says Reason: it failed your server's sender check (DMARC).",
                Role::Status,
            );
            assert_named(window, "Report junk", Role::Button);
            window.click("reader-unsubscribe", cx);
        });
        rig.frame(cx);
        let compose = rig.events().into_iter().find_map(|e| match e {
            ReaderEvent::Compose(m) => Some(m),
            _ => None,
        });
        let m = compose.expect("the mailto unsubscribe");
        assert_eq!(
            (m.to.as_str(), m.subject.as_str()),
            ("leave@list.example.org", "leave me")
        );

        // Encrypted: the notice, no Unsubscribe left over.
        let id2 = rig.ids[1];
        rig.body(
            id2,
            "From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Secret\r\n\
         MIME-Version: 1.0\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nU3ludGhldGlj\r\n",
        );
        rig.open(cx, id2);
        in_window(cx, rig.window, |window, _| {
            let notice = window.find("reader-notice");
            assert!(
                notice
                    .label()
                    .is_some_and(|l| l.starts_with("This message is encrypted (S/MIME).")),
                "{:?}",
                notice.label()
            );
            assert!(window.try_find("reader-unsubscribe").is_none());
            assert_named(window, "smime.p7m", Role::Button);
        });
    }

    /// Row 139: the invite card offers the answers; an answer asks the owner
    /// first, which then records it (and queues the reply).
    #[gpui_kit::test]
    fn invite_card_answers_and_replies(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-invite");
        let id = rig.ids[0];
        let email = komorebi::seed::EMAIL;
        rig.body(
            id,
            &format!(
                "From: Ana <ana@example.org>\r\nTo: {email}\r\nSubject: Plan\r\n\
             Content-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\n\
             METHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:plan@example.org\r\nSEQUENCE:1\r\n\
             DTSTART:20991008T130000Z\r\nSUMMARY:Planning\r\n\
             DESCRIPTION:Agenda\r\nORGANIZER;CN=Ana:mailto:ana@example.org\r\n\
             ATTENDEE;RSVP=TRUE:mailto:{email}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            ),
        );
        rig.open(cx, id);
        in_window(cx, rig.window, |window, cx| {
            assert_named(window, "Planning", Role::Group);
            assert_named(window, "Accept", Role::Button);
            assert_named(window, "Details", Role::Button);
            window.click("invite-answer-1", cx);
        });
        rig.frame(cx);
        // The question first: who the reply goes to, exactly; Send reply
        // queues it for the organizer only.
        in_window(cx, rig.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            let text = window
                .find("rsvp-text")
                .label()
                .map(str::to_string)
                .unwrap_or_default();
            assert!(
                text.starts_with(
                    "The reply goes only to the organizer, Ana <ana@example.org>, from "
                ),
                "{text}"
            );
            assert_named(window, "Don't send a reply", Role::Button);
            window.click("rsvp-send", cx);
        });
        rig.frame(cx);
        let events = rig.events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ReaderEvent::ReplyQueued { .. })),
            "{events:?}"
        );
        assert_eq!(rig.db().outbox_due(i64::MAX).unwrap().len(), 1);
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert_named(window, "Change", Role::Button);
            assert!(window.try_find("invite-answer-1").is_none());
            // Change, then Decline without a reply: kept here only.
            window.click("invite-tool-5", cx);
        });
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            window.click("invite-answer-3", cx)
        });
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| window.click("rsvp-keep", cx));
        rig.frame(cx);
        assert!(rig.events().contains(&ReaderEvent::Status {
            text: "Answer kept on this computer; no reply was sent.".into(),
            error: false
        }));
        assert_eq!(rig.db().outbox_due(i64::MAX).unwrap().len(), 1);
        // Komorebi 1.0 keeps nothing in a calendar.
        let db = rig.db();
        let account = db.accounts().unwrap()[0].id;
        assert_eq!(
            db.rsvp(account, "plan@example.org", "")
                .unwrap()
                .map(|r| r.1),
            Some(Answer::Declined as i64)
        );
    }

    /// The page's text for screen readers: the lines from the worker, under
    /// the page node.
    #[gpui_kit::test]
    fn the_page_keeps_its_lines_for_screen_readers(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-lines");
        let (generation, _) = open_page(&rig, cx, 600.0, vec![]);
        rig.reply(
            cx,
            Reply::Text {
                generation,
                lines: vec![komorebi::mail::render::TextLine {
                    x: 0.0,
                    y: 0.0,
                    w: 100.0,
                    h: 20.0,
                    text: "Hi Ines,".into(),
                    ..Default::default()
                }],
            },
        );
        let lines = rig.view.read_with(cx, |r, _| r.lines.clone());
        assert_eq!(lines.len(), 1);
        in_window(cx, rig.window, |window, _| {
            assert_named(window, "Message", Role::Document);
        });
    }

    /// Closing the reader releases its tiles from the GPU and cancels the
    /// worker's work.
    #[gpui_kit::test]
    fn closing_cancels_and_releases(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-close");
        let (generation, _) = open_page(&rig, cx, 600.0, vec![]);
        let pixels =
            komorebi::reader::Pixels::new(2, 2, std::sync::Arc::from(vec![255u8; 16])).unwrap();
        let image = worker::image(&pixels).unwrap();
        rig.tx
            .unbounded_send(Incoming::Tile {
                generation,
                index: 0,
                image,
            })
            .unwrap();
        cx.run_until_parked();
        let shown = rig
            .view
            .read_with(cx, |r, _| r.session().tiles()[0].image.is_some());
        assert!(shown);
        in_window(cx, rig.window, |window, cx| {
            rig.view.update(cx, |r, cx| r.close(window, cx))
        });
        assert!(matches!(rig.take().last(), Some(Job::Cancel { .. })));
        assert_eq!(rig.view.read_with(cx, |r, _| r.message()), None);
    }

    /// The statuses the reader sent its owner.
    fn statuses(events: Vec<ReaderEvent>) -> Vec<(String, bool)> {
        events
            .into_iter()
            .filter_map(|e| match e {
                ReaderEvent::Status { text, error } => Some((text, error)),
                _ => None,
            })
            .collect()
    }

    const ATTACHED: &str = "From: Bob <bob@example.org>\r\nTo: me@example.org\r\n\
        Subject: Files\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\n\r\nTwo files.\r\n\
        --b\r\nContent-Type: text/plain; name=\"notes.txt\"\r\n\
        Content-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nhello notes\r\n\
        --b\r\nContent-Type: application/octet-stream; name=\"run.exe\"\r\n\
        Content-Disposition: attachment; filename=\"run.exe\"\r\n\r\nMZ\r\n--b--\r\n";

    /// The Slint app's `open_attachment` and `save_attachment`
    /// (src/app.rs): a pill opens a copy in its default app (types that run
    /// code are refused), and its save button saves it where the dialog
    /// says, in every place the reader shows (it needs no owner).
    #[gpui_kit::test]
    fn attachments_open_in_their_app_and_save_through_the_dialog(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-attachments");
        let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
        let sink = opened.clone();
        rig.view.update(cx, |r, _| {
            r.set_opener(Arc::new(move |p: &std::path::Path| {
                sink.lock().unwrap().push(p.to_path_buf());
                true
            }))
        });
        let id = rig.ids[0];
        rig.body(id, ATTACHED);
        rig.open(cx, id);
        in_window(cx, rig.window, |window, cx| {
            assert_named(window, "notes.txt", Role::Button);
            window.click("attachment-0", cx);
        });
        cx.run_until_parked();
        let paths = opened.lock().unwrap().clone();
        let [path] = &paths[..] else {
            panic!("one file opened, got {paths:?}");
        };
        assert_eq!(path.file_name().unwrap(), "notes.txt");
        assert_eq!(std::fs::read(path).unwrap(), b"hello notes");
        assert!(statuses(rig.events()).is_empty());

        // A program is refused, with the reason.
        in_window(cx, rig.window, |window, cx| {
            window.click("attachment-1", cx)
        });
        cx.run_until_parked();
        assert_eq!(opened.lock().unwrap().len(), 1);
        let said = statuses(rig.events());
        assert!(
            said.iter()
                .any(|(t, e)| *e && t.starts_with("run.exe can run programs")),
            "{said:?}"
        );

        // Save: the dialog, then the bytes where it says.
        in_window(cx, rig.window, |window, cx| {
            window.click("attachment-save-0", cx)
        });
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        let target = rig.cache.dir().join("saved-notes.txt");
        let pick = target.clone();
        cx.simulate_new_path_selection(move |_| Some(pick));
        cx.run_until_parked();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello notes");
        let said = statuses(rig.events());
        assert_eq!(
            said,
            [(format!("Saved {}", target.display()), false)],
            "{said:?}"
        );
    }

    /// The sender notice's Why? opens "Why am I seeing this?", which Close
    /// closes.
    #[gpui_kit::test]
    fn why_explains_the_sender_notice(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-why");
        let id = rig.ids[0];
        rig.body(
            id,
            "Authentication-Results: mx.example.invalid; spf=pass; dmarc=fail\r\n\
         From: Bob <bob@example.org>\r\nTo: me@example.org\r\nSubject: Lunch\r\n\r\nNoon?\r\n",
        );
        rig.view.update(cx, |r, cx| {
            let mut accounts = r.accounts.clone();
            accounts[0].imap.host = "mx.example.invalid".into();
            r.set_accounts(accounts, cx);
        });
        rig.open(cx, id);
        in_window(cx, rig.window, |window, cx| window.click("reader-why", cx));
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            let what = window
                .find("why-what")
                .label()
                .map(str::to_string)
                .unwrap_or_default();
            assert!(what.starts_with("Komorebi looks at the checks"), "{what}");
            window.click("why-close", cx);
        });
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(!window.has_active_dialog(cx))
        });
    }

    /// Row 140, the Slint app's `source_actions_use_the_cached_bytes`: Save
    /// as .eml writes exactly the cached bytes (Ctrl+S too), Show original
    /// shows the Message-ID, header fields and source (Escape closes it),
    /// and Copy Message-ID copies it. (Forward as attachment is the
    /// composer's, `Draft::Forward { attached: true }`.)
    #[gpui_kit::test]
    fn source_actions_use_the_cached_bytes(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-source");
        let id = rig.ids[0];
        // Nothing open: nothing to act on.
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::ShowOriginal, window, cx))
        });
        in_window(cx, rig.window, |window, cx| {
            assert!(!window.has_active_dialog(cx))
        });
        let raw = "From: Bob <bob@example.org>\r\nSubject: Lunch\r\n\
            Message-ID: <m1@example.org>\r\n\r\nNoon?\r\n";
        rig.body(id, raw);
        rig.open(cx, id);
        let msgid = rig.db().message(id).unwrap().unwrap().msgid;
        let (name, bytes) = rig.view.read_with(cx, |r, _| r.eml_export()).unwrap();
        assert_eq!(bytes, raw.as_bytes());
        assert!(name.ends_with(").eml"), "{name}");

        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::ShowOriginal, window, cx))
        });
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            let line = window
                .find("source-msgid")
                .label()
                .map(str::to_string)
                .unwrap_or_default();
            assert_eq!(
                line,
                format!("Message-ID: {}", rig.view.read(cx).message_id())
            );
            assert_named(window, "Copy headers", Role::Button);
            window.click("source-copy-all", cx);
        });
        let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
        assert!(
            copied.contains("Subject: Lunch") && copied.ends_with("Noon?\n"),
            "{copied:?}"
        );
        in_window(cx, rig.window, |window, cx| press(window, &["escape"], cx));
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(!window.has_active_dialog(cx))
        });

        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::CopyId, window, cx))
        });
        let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
        assert_eq!(
            copied.trim_matches(['<', '>']),
            msgid.trim_matches(['<', '>'])
        );
        assert!(statuses(rig.events()).contains(&("Message-ID copied.".into(), false)));

        // Ctrl+S from the reader (its text view has the keyboard): Save as
        // .eml through the dialog.
        let field = rig
            .view
            .read_with(cx, |r, _| r.text_view().cloned())
            .unwrap();
        let focus = field.read_with(cx, |s, cx| s.focus_handle(cx));
        in_window(cx, rig.window, |window, cx| {
            window.focus(&focus, cx);
            press(window, &["ctrl-s"], cx);
        });
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        let target = rig.cache.dir().join("lunch.eml");
        let pick = target.clone();
        cx.simulate_new_path_selection(move |_| Some(pick));
        cx.run_until_parked();
        assert_eq!(std::fs::read(&target).unwrap(), raw.as_bytes());
    }

    /// Row 141, the Slint app's `source_actions_wait_for_the_body`: a body
    /// that is not downloaded yet is asked for; a failed download says so,
    /// and one that arrives runs the command (another message's changes
    /// nothing).
    #[gpui_kit::test]
    fn source_actions_wait_for_the_body(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-source-wait");
        let mut db = rig.db();
        let first = db.message(rig.ids[0]).unwrap().unwrap();
        let header = komorebi::db::NewHeader {
            uid: 9_000,
            msgid: "<report@example.org>".into(),
            subject: "Report".into(),
            date: 1_700_000_100,
            ..Default::default()
        };
        db.insert_headers(first.account_id, first.folder_id, &[header])
            .unwrap();
        let id = db
            .message_ids(&[first.folder_id], "")
            .unwrap()
            .into_iter()
            .find(|&m| db.message(m).unwrap().unwrap().raw.is_none())
            .expect("the message without a body");
        let view = rig.view.clone();
        in_window(cx, rig.window, |window, cx| {
            view.update(cx, |r, cx| assert!(r.open(id, window, cx)))
        });
        rig.events();
        // Copy Message-ID needs only the cached header: at once, and no
        // download.
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::CopyId, window, cx))
        });
        let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
        assert_eq!(copied, "<report@example.org>");
        let events = rig.events();
        assert!(!events.contains(&ReaderEvent::FetchBody(id)), "{events:?}");
        assert_eq!(rig.view.read_with(cx, |r, _| r.source_waiting()), None);
        // The row menu's, from the header alone, for a message not shown;
        // one without a Message-ID says so.
        let other = rig.ids[1];
        let row = rig.db().message_id(other).unwrap().unwrap();
        assert!(!row.is_empty());
        rig.view.update(cx, |r, cx| r.copy_message_id_of(other, cx));
        let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
        assert_eq!(copied, format!("<{}>", row.trim_matches(['<', '>'])));
        let header = komorebi::db::NewHeader {
            uid: 9_002,
            ..Default::default()
        };
        db.insert_headers(first.account_id, first.folder_id, &[header])
            .unwrap();
        let bare = db
            .message_ids(&[first.folder_id], "")
            .unwrap()
            .into_iter()
            .find(|&m| db.message_id(m).unwrap().as_deref() == Some(""))
            .expect("the message without a Message-ID");
        rig.events();
        rig.view.update(cx, |r, cx| r.copy_message_id_of(bare, cx));
        assert_eq!(
            statuses(rig.events()),
            [("This message has no Message-ID.".into(), false)]
        );

        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::ShowOriginal, window, cx))
        });
        assert!(rig.events().contains(&ReaderEvent::FetchBody(id)));
        assert_eq!(
            rig.view.read_with(cx, |r, _| r.source_waiting()),
            Some((id, SourceUse::ShowOriginal))
        );
        // A second on, the status line says it is fetching.
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(statuses(rig.events()).contains(&("Fetching message\u{a0}…".into(), false)));

        rig.view
            .update(cx, |r, cx| r.body_failed(id, "timed out", cx));
        assert_eq!(rig.view.read_with(cx, |r, _| r.source_waiting()), None);
        assert_eq!(
            statuses(rig.events()),
            [("Couldn't download this message: timed out".into(), true)]
        );

        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::ShowOriginal, window, cx))
        });
        let other = rig.ids[1];
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.body_arrived(other, window, cx))
        });
        assert!(rig.view.read_with(cx, |r, _| r.source_waiting()).is_some());
        db.set_body(
            id,
            b"Subject: Report\r\n\r\nQ3\r\n",
            "Q3",
            false,
            Default::default(),
        )
        .unwrap();
        in_window(cx, rig.window, |window, cx| {
            rig.view.update(cx, |r, cx| r.body_arrived(id, window, cx))
        });
        assert_eq!(rig.view.read_with(cx, |r, _| r.source_waiting()), None);
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(window.has_active_dialog(cx), "Show original ran");
            assert_named(window, "Message-ID: <report@example.org>", Role::Label);
        });
    }

    /// Row 173, the Slint app's
    /// `show_original_gives_the_focus_back_to_its_opener`: Show original,
    /// then Escape; the page has the keyboard again.
    #[gpui_kit::test]
    fn show_original_gives_the_focus_back_to_its_opener(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-source-focus");
        open_page(&rig, cx, 2000.0, vec![]);
        let focus = rig.view.read_with(cx, |r, cx| r.focus_handle(cx));
        in_window(cx, rig.window, |window, cx| {
            window.focus(&focus, cx);
            rig.view
                .update(cx, |r, cx| r.source(SourceUse::ShowOriginal, window, cx));
        });
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(window.has_active_dialog(cx));
            assert!(!focus.is_focused(window));
            press(window, &["escape"], cx);
        });
        cx.executor().advance_clock(Duration::from_secs(1));
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert!(focus.is_focused(window), "the page has the focus back");
        });
    }

    /// The page keys (the Slint reader's page-keys scope): PageDown and
    /// Space scroll 90 percent of the view, PageUp and Shift+Space back,
    /// Down and Up a few lines, End to the end and Home to the top.
    #[gpui_kit::test]
    fn the_page_scrolls_from_the_keyboard(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-keys");
        open_page(&rig, cx, 6000.0, vec![]);
        let focus = rig.view.read_with(cx, |r, cx| r.focus_handle(cx));
        let y = |cx: &mut TestAppContext| {
            rig.view
                .read_with(cx, |r, _| -f32::from(r.scroll.offset().y))
        };
        let view = rig
            .view
            .read_with(cx, |r, _| f32::from(r.scroll.bounds().size.height));
        assert!(view > 0.0);
        in_window(cx, rig.window, |window, cx| {
            window.focus(&focus, cx);
            press(window, &["pagedown"], cx);
        });
        assert!((y(cx) - 0.9 * view).abs() < 1.0, "{} of {view}", y(cx));
        in_window(cx, rig.window, |window, cx| press(window, &["space"], cx));
        assert!((y(cx) - 1.8 * view).abs() < 1.0);
        in_window(cx, rig.window, |window, cx| {
            press(window, &["shift-space", "down"], cx)
        });
        assert!((y(cx) - 0.9 * view - crate::theme::reader::SCROLL_LINE).abs() < 1.0);
        in_window(cx, rig.window, |window, cx| press(window, &["end"], cx));
        let max = rig
            .view
            .read_with(cx, |r, _| f32::from(r.scroll.max_offset().y));
        assert!(max > 5000.0 && (y(cx) - max).abs() < 1.0, "{max}");
        in_window(cx, rig.window, |window, cx| press(window, &["home"], cx));
        assert_eq!(y(cx), 0.0);
        in_window(cx, rig.window, |window, cx| press(window, &["up"], cx));
        assert_eq!(y(cx), 0.0, "never above the top");
    }

    /// A source command waits for the message it was asked on: opening
    /// another or closing drops it, so no "Fetching message" over the
    /// other and no dialog when the first body comes later.
    #[gpui_kit::test]
    fn a_source_wait_ends_with_its_message(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-source-wait-ends");
        let mut db = rig.db();
        let first = db.message(rig.ids[0]).unwrap().unwrap();
        let header = komorebi::db::NewHeader {
            uid: 9_001,
            msgid: "<late@example.org>".into(),
            subject: "Late".into(),
            date: 1_700_000_200,
            ..Default::default()
        };
        db.insert_headers(first.account_id, first.folder_id, &[header])
            .unwrap();
        let id = db
            .message_ids(&[first.folder_id], "")
            .unwrap()
            .into_iter()
            .find(|&m| db.message(m).unwrap().unwrap().raw.is_none())
            .expect("the message without a body");
        let other = rig.ids[1];
        for close in [false, true] {
            rig.open(cx, id);
            in_window(cx, rig.window, |window, cx| {
                rig.view
                    .update(cx, |r, cx| r.source(SourceUse::ShowOriginal, window, cx))
            });
            assert!(rig.view.read_with(cx, |r, _| r.source_waiting()).is_some());
            in_window(cx, rig.window, |window, cx| {
                rig.view.update(cx, |r, cx| {
                    if close {
                        r.close(window, cx)
                    } else {
                        assert!(r.open(other, window, cx))
                    }
                })
            });
            assert_eq!(rig.view.read_with(cx, |r, _| r.source_waiting()), None);
            rig.events();
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            assert!(statuses(rig.events()).is_empty(), "close: {close}");
        }
    }

    /// The text view's field has the keyboard, and Up/Down, Page and
    /// Home/End still scroll the reader (not the field's hidden caret).
    #[gpui_kit::test]
    fn the_text_view_scrolls_from_the_keyboard(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-text-keys");
        let id = rig.ids[0];
        let lines: String = (0..400)
            .map(|n| {
                format!(
                    "Line {n}
"
                )
            })
            .collect();
        rig.body(
            id,
            &format!(
                "From: Bob <bob@example.org>
Subject: Long

{lines}"
            ),
        );
        rig.open(cx, id);
        rig.frame(cx);
        let field = rig
            .view
            .read_with(cx, |r, _| r.text_view().cloned())
            .expect("the text view's field");
        let focus = field.read_with(cx, |s, cx| s.focus_handle(cx));
        let y = |cx: &mut TestAppContext| {
            rig.view
                .read_with(cx, |r, _| -f32::from(r.scroll.offset().y))
        };
        in_window(cx, rig.window, |window, cx| {
            window.focus(&focus, cx);
            press(window, &["down"], cx);
        });
        assert!(
            (y(cx) - crate::theme::reader::SCROLL_LINE).abs() < 1.0,
            "{}",
            y(cx)
        );
        in_window(cx, rig.window, |window, cx| press(window, &["end"], cx));
        let max = rig
            .view
            .read_with(cx, |r, _| f32::from(r.scroll.max_offset().y));
        assert!(max > 0.0 && (y(cx) - max).abs() < 1.0, "{max}");
        in_window(cx, rig.window, |window, cx| press(window, &["pageup"], cx));
        assert!(y(cx) < max);
        in_window(cx, rig.window, |window, cx| press(window, &["home"], cx));
        assert_eq!(y(cx), 0.0);
    }

    /// The text view is a read-only field: selectable and copied with the
    /// keyboard, as the Slint app's.
    #[gpui_kit::test]
    fn the_text_view_can_be_selected_and_copied(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-text");
        let id = rig.ids[0];
        rig.body(
            id,
            "From: Bob <bob@example.org>\r\nSubject: Lunch\r\n\r\nNoon at the usual place?\r\n",
        );
        rig.open(cx, id);
        let field = rig
            .view
            .read_with(cx, |r, _| r.text_view().cloned())
            .expect("the text view's field");
        let value = field.read_with(cx, |s, _| s.value().to_string());
        assert!(value.starts_with("Noon at the usual place?"), "{value:?}");
        let focus = field.read_with(cx, |s, cx| s.focus_handle(cx));
        in_window(cx, rig.window, |window, cx| {
            window.focus(&focus, cx);
            press(window, &["ctrl-a", "ctrl-c"], cx);
        });
        let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
        assert!(copied.starts_with("Noon at the usual place?"), "{copied:?}");
        // Read-only: typing changes nothing.
        in_window(cx, rig.window, |window, cx| {
            test_support::type_text(window, "x", cx)
        });
        let after = field.read_with(cx, |s, _| s.value().to_string());
        assert_eq!(after, value);
    }

    /// The Slint app's "You started a reply to this message" bar
    /// (`show_draft_reply`): shown while a reply to the open message waits
    /// in Drafts, once the owner opens saved replies; Resume asks the owner
    /// for it.
    #[gpui_kit::test]
    fn a_reply_in_drafts_offers_resume(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-draft-reply");
        let id = rig.ids[0];
        rig.open(cx, id);
        in_window(cx, rig.window, |window, _| {
            assert!(window.try_find("reader-draft-reply").is_none())
        });
        let db = rig.db();
        let account = db.accounts().unwrap()[0].id;
        let draft = db
            .insert_draft(&komorebi::db::DraftRow {
                account_id: account,
                reply_to_msg: Some(id),
                subject: "Re: x".into(),
                ..Default::default()
            })
            .unwrap();
        rig.view.update(cx, |_, cx| cx.notify());
        rig.frame(cx);
        in_window(cx, rig.window, |window, _| {
            assert!(
                window.try_find("reader-draft-reply").is_none(),
                "no Resume that nothing handles"
            )
        });
        rig.view.update(cx, |r, cx| r.handle_resume(cx));
        rig.frame(cx);
        in_window(cx, rig.window, |window, cx| {
            assert_named(window, "You started a reply to this message.", Role::Status);
            window.click("reader-resume", cx);
        });
        assert!(
            rig.events()
                .contains(&ReaderEvent::ResumeReply { message: id, draft })
        );
    }

    /// The More menu's view toggles: View as plain text on and off, and
    /// Stop loading images from a sender whose images always load.
    #[gpui_kit::test]
    fn the_view_toggles_of_the_more_menu(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-view-menu");
        open_page(&rig, cx, 2000.0, vec![]);
        let menu = rig.view.read_with(cx, |r, _| r.view_menu());
        assert_eq!(menu.plain, (true, false));
        assert!(!menu.stop_images);
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.menu(MenuCmd::Plain, window, cx))
        });
        let menu = rig.view.read_with(cx, |r, _| r.view_menu());
        assert_eq!(menu.plain, (true, true));
        assert!(rig.view.read_with(cx, |r, _| r.session().text_shown()));
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.menu(MenuCmd::Plain, window, cx))
        });
        assert!(rig.view.read_with(cx, |r, _| r.session().page_shown()));

        let from = rig.db().message(rig.ids[0]).unwrap().unwrap().from_addr;
        rig.db().allow_remote(&from).unwrap();
        assert!(rig.view.read_with(cx, |r, _| r.view_menu().stop_images));
        in_window(cx, rig.window, |window, cx| {
            rig.view
                .update(cx, |r, cx| r.menu(MenuCmd::StopImages, window, cx))
        });
        assert!(!rig.db().remote_allowed(&from).unwrap());
        assert!(!rig.view.read_with(cx, |r, _| r.view_menu().stop_images));
    }

    /// Enter and Escape on the page are its own only while they have
    /// something to do (a focused link; a selection or a focused link);
    /// otherwise they go on to the owner (Back to the list, closing a
    /// message window), which sees the action pass the page.
    #[gpui_kit::test]
    fn enter_and_escape_pass_the_page_when_it_has_nothing_to_do(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-keys-pass");
        let (generation, _) = open_page(&rig, cx, 2000.0, vec![link("#x", "X", 40.0, None)]);
        let passed: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let (esc, enter) = (passed.clone(), passed.clone());
        cx.update(|cx| {
            cx.on_action(move |_: &ClearSelection, _| esc.borrow_mut().push("escape"));
            cx.on_action(move |_: &FollowLink, _| enter.borrow_mut().push("enter"));
        });
        let focus = rig.view.read_with(cx, |r, cx| r.focus_handle(cx));
        in_window(cx, rig.window, |window, cx| {
            window.focus(&focus, cx);
            press(window, &["enter", "escape"], cx)
        });
        assert_eq!(*passed.borrow(), ["enter", "escape"]);
        assert!(selects(rig.take()).is_empty(), "nothing to clear");

        // A selection: Escape clears it and stops there.
        rig.reply(
            cx,
            Reply::Selection {
                generation,
                rects: vec![Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                }],
            },
        );
        in_window(cx, rig.window, |window, cx| press(window, &["escape"], cx));
        assert_eq!(selects(rig.take()), [SelectOp::Clear]);
        // A focused link: Enter follows it and stops there.
        in_window(cx, rig.window, |window, cx| {
            press(window, &["tab", "enter"], cx)
        });
        assert_eq!(
            rig.view.read_with(cx, |r, _| r.session().focused_link()),
            Some(0)
        );
        assert_eq!(*passed.borrow(), ["enter", "escape"]);
    }

    /// The page's selection reaches screen readers as (line, character)
    /// over its lines, and goes with the page.
    #[gpui_kit::test]
    fn the_text_selection_reaches_the_session(cx: &mut TestAppContext) {
        let rig = Rig::new(cx, "reader-text-selection");
        let (generation, _) = open_page(&rig, cx, 600.0, vec![]);
        rig.reply(
            cx,
            Reply::TextSelection {
                generation,
                range: Some(((0, 1), (2, 4))),
            },
        );
        assert_eq!(
            rig.view.read_with(cx, |r, _| r.session().text_selection()),
            Some(((0, 1), (2, 4)))
        );
        // A stale one is ignored; closing drops it.
        rig.reply(
            cx,
            Reply::TextSelection {
                generation: generation - 1,
                range: None,
            },
        );
        assert!(
            rig.view
                .read_with(cx, |r, _| r.session().text_selection())
                .is_some()
        );
        in_window(cx, rig.window, |window, cx| {
            rig.view.update(cx, |r, cx| r.close(window, cx))
        });
        assert_eq!(
            rig.view.read_with(cx, |r, _| r.session().text_selection()),
            None
        );
    }
}
