//! The sync parts in the main window (the shell lane): the hub's status
//! line in the list pane with its action, the caution badge, the loader
//! and Refresh, the jobs the views queue reaching the workers, and the
//! Outbox entry's pane. The workers are captured (`hub::Mode::Capture`):
//! nothing connects. Each test names the Slint test it ports
//! (`src/app/composer.rs`; docs/GPUI-TEST-PORT.md).

#[cfg(test)]
mod port {
    use std::sync::atomic::AtomicBool;

    use gpui::{AnyWindowHandle, Entity, Role, TestAppContext};
    use gpui_kit::test::TestWindowExt as _;
    use komorebi::db::{Db, DraftRow, Outbox};
    use komorebi::mail::errors::Problem;
    use komorebi::mail::{Event, Job};
    use komorebi::role::Role as FolderRole;

    use crate::test_support::{self, assert_named, in_window, names};
    use crate::views::mail::MailView;
    use crate::views::mail::hub::{Hub, Mode};
    use crate::views::mail::model::View;
    use crate::views::mail::sync::{Said, Severity};

    struct Opened {
        window: AnyWindowHandle,
        mail: Entity<MailView>,
        hub: Entity<Hub>,
    }

    fn open(cx: &mut TestAppContext, db: Db) -> Opened {
        test_support::init_app(cx);
        let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
        let (mail, hub) = main.read_with(cx, |w, _| {
            (
                w.mail().cloned().expect("the mail view"),
                w.hub().cloned().expect("the hub"),
            )
        });
        assert_eq!(hub.read_with(cx, |h, _| h.mode()), Mode::Capture);
        Opened { window, mail, hub }
    }

    fn offline(account: i64) -> Event {
        let e = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::TimedOut));
        Event::Syncing {
            account_id: account,
            active: false,
            error: Some(Problem::of(&e)),
        }
    }

    fn synced(account: i64) -> Event {
        Event::Syncing {
            account_id: account,
            active: false,
            error: None,
        }
    }

    fn tell(cx: &mut TestAppContext, o: &Opened, event: Event) {
        o.hub.update(cx, |h, cx| h.on_event(event, cx));
        cx.run_until_parked();
    }

    fn severity(cx: &mut TestAppContext, o: &Opened) -> Option<Severity> {
        o.hub
            .read_with(cx, |h, _| h.sync().line().map(|l| l.severity))
    }

    /// The account's line in the folder pane, as UI Automation names it.
    fn account_line(cx: &mut TestAppContext, o: &Opened) -> String {
        in_window(cx, o.window, |w, _| {
            names(w)
                .into_iter()
                .find(|(role, name)| {
                    *role == Some(Role::TreeItem) && name.contains("seed@komorebi.invalid")
                })
                .map(|(_, name)| name)
                .expect("the account's line")
        })
    }

    /// Port of `the_offline_badge_stays_until_a_sync_goes_through`: a
    /// server out of reach puts Try now on the status line and a caution
    /// badge on the account's line that stays (also when the line is
    /// closed) until a sync goes through. The rail tile reads the same
    /// state (`MailView::badge`); Settings' account card is phase 6.
    #[gpui_kit::test]
    fn the_offline_badge_stays_until_a_sync_goes_through(cx: &mut TestAppContext) {
        let (_cache, db, seeded) = test_support::seeded("offline-badge", 3);
        let o = open(cx, db);
        let account = seeded.account;
        assert!(!account_line(cx, &o).contains("Can't reach"));

        tell(cx, &o, offline(account));
        in_window(cx, o.window, |w, _| {
            assert_named(w, "Try now", Role::Button)
        });
        let line = account_line(cx, &o);
        assert!(line.contains("Can't reach "), "{line}");
        assert!(o.mail.read_with(cx, |v, _| v.badge(account).is_some()));

        // Closing the line keeps the badge: the server is still out of reach.
        in_window(cx, o.window, |w, cx| w.click("sync-status-close", cx));
        cx.run_until_parked();
        assert_eq!(severity(cx, &o), None);
        assert!(account_line(cx, &o).contains("Can't reach "));

        // Try now: one attempt; the badge waits for its answer.
        o.hub.update(cx, |h, cx| h.refresh(cx));
        tell(cx, &o, offline(account));
        in_window(cx, o.window, |w, cx| w.click("sync-status-action", cx));
        cx.run_until_parked();
        let jobs = o.hub.read_with(cx, |h, _| h.captured());
        assert!(
            jobs.iter()
                .any(|(a, j)| *a == account && matches!(j, Job::SyncAll { full: true })),
            "Try now asks for a sync"
        );
        assert!(account_line(cx, &o).contains("Can't reach "));

        // A sync that goes through clears it everywhere.
        tell(cx, &o, synced(account));
        assert!(!account_line(cx, &o).contains("Can't reach"));
        assert!(o.mail.read_with(cx, |v, _| v.badge(account).is_none()));
    }

    /// Port of `problems_show_once_and_a_vanished_folder_leads_to_the_inbox`:
    /// a server out of reach is a Caution that stays (another line takes its
    /// place for a while and it comes back), it is said once until a sync
    /// goes through, and a folder dropped from the server's list leaves its
    /// view for the account's Inbox.
    #[gpui_kit::test]
    fn problems_show_once_and_a_vanished_folder_leads_to_the_inbox(cx: &mut TestAppContext) {
        let (cache, db, seeded) = test_support::seeded("resilience", 3);
        let work = db
            .upsert_folder(seeded.account, "Work", "Work", 0, FolderRole::Other)
            .expect("a folder");
        let o = open(cx, db);
        let account = seeded.account;
        let said = |cx: &mut TestAppContext| {
            o.hub.read_with(cx, |h, _| {
                h.sync()
                    .line()
                    .map(|l| crate::views::mail::sync_text::said(&l.said))
                    .unwrap_or_default()
            })
        };
        tell(cx, &o, offline(account));
        assert!(said(cx).contains("Can't reach the server"), "{}", said(cx));
        assert_eq!(severity(cx, &o), Some(Severity::Caution));
        // Another line replaces it for a while; when that one goes, it is
        // back.
        o.hub.update(cx, |h, cx| {
            h.say(Said::Text("Draft saved".into()), false, cx)
        });
        assert_eq!(severity(cx, &o), Some(Severity::Info));
        cx.executor().advance_clock(std::time::Duration::from_secs(
            crate::views::mail::sync::INFO_SECS,
        ));
        cx.run_until_parked();
        assert_eq!(severity(cx, &o), Some(Severity::Caution));
        // A sync that goes through takes it away.
        tell(cx, &o, synced(account));
        assert_eq!(severity(cx, &o), None);
        tell(cx, &o, offline(account));
        assert_eq!(severity(cx, &o), Some(Severity::Caution));
        o.hub.update(cx, |h, cx| h.dismiss(cx));
        // Closed: quiet, also when the line it waited under goes. After a
        // sync that went through, it shows again.
        tell(cx, &o, offline(account));
        assert_eq!(severity(cx, &o), None);
        o.hub.update(cx, |h, cx| h.clear_line(cx));
        assert_eq!(severity(cx, &o), None);
        tell(cx, &o, synced(account));
        tell(cx, &o, offline(account));
        assert_eq!(severity(cx, &o), Some(Severity::Caution));

        // The open folder leaves the server's list: its Inbox shows.
        in_window(cx, o.window, |w, cx| {
            o.mail
                .update(cx, |v, cx| v.act(w, cx, |m| m.set_view(View::Folder(work))))
        });
        cache
            .open()
            .retain_folders(account, &[seeded.inbox, seeded.sent, seeded.archive])
            .expect("drop the folder");
        tell(cx, &o, Event::FolderChanged { folder_id: None });
        assert_eq!(
            o.mail.read_with(cx, |v, _| v.mail().view()),
            View::Folder(seeded.inbox)
        );
    }

    /// The jobs the views queue reach the account's workers (the Slint
    /// app's sync worker), and F5 and the Refresh button sync every
    /// account in full; the loader takes Refresh's place while it runs.
    #[gpui_kit::test]
    fn refresh_syncs_and_the_loader_shows_while_it_runs(cx: &mut TestAppContext) {
        let (_cache, db, seeded) = test_support::seeded("refresh", 3);
        let o = open(cx, db);
        o.hub.read_with(cx, |h, _| drop(h.captured()));
        in_window(cx, o.window, |w, _| {
            assert_named(w, "Refresh", Role::Button);
        });
        in_window(cx, o.window, |w, cx| w.click("list-refresh", cx));
        cx.run_until_parked();
        let jobs = o.hub.read_with(cx, |h, _| h.captured());
        assert!(
            jobs.iter()
                .any(|(a, j)| *a == seeded.account && matches!(j, Job::SyncAll { full: true })),
            "the job reached the workers"
        );
        tell(
            cx,
            &o,
            Event::Syncing {
                account_id: seeded.account,
                active: true,
                error: None,
            },
        );
        let syncing = crate::views::mail::sync_text::syncing();
        in_window(cx, o.window, |w, _| {
            let names = names(w);
            assert!(
                !names.iter().any(|(_, n)| n.starts_with("Refresh")),
                "the loader takes Refresh's place"
            );
            assert!(names.iter().any(|(_, n)| *n == syncing), "{names:?}");
        });
        tell(cx, &o, synced(seeded.account));
        assert!(!o.hub.read_with(cx, |h, _| h.sync().syncing()));
        // After a sync pass: "Refresh (F5). Updated 14:05".
        in_window(cx, o.window, |w, _| {
            assert!(
                names(w).iter().any(
                    |(r, n)| *r == Some(Role::Button) && n.starts_with("Refresh (F5). Updated")
                ),
                "{:?}",
                names(w)
            );
        });
    }

    /// A send that failed for good in the Outbox: its pane says why, Retry
    /// hands it to the send worker, Discard asks and removes it (the Slint
    /// app's `src/app/outbox.rs`).
    #[gpui_kit::test]
    fn the_outbox_pane_retries_and_discards(cx: &mut TestAppContext) {
        let (cache, db, seeded) = test_support::seeded("outbox-pane", 1);
        let row = db
            .insert_draft(&DraftRow {
                account_id: seeded.account,
                to: "ana@example.com".into(),
                subject: "Plans".into(),
                body: "See you on Friday".into(),
                ..Default::default()
            })
            .expect("a draft");
        assert!(db.outbox_send(row).unwrap());
        assert!(db.store_raw(row, b"raw", &AtomicBool::new(false)).unwrap());
        assert!(db.outbox_queue(row).unwrap());
        assert!(db.claim_outbox(row).unwrap().is_some());
        db.outbox_result(row, Outbox::Failed, None, "451 4.3.0 try later")
            .unwrap();
        let o = open(cx, db);
        o.hub.read_with(cx, |h, _| drop(h.captured()));
        in_window(cx, o.window, |w, cx| {
            o.mail.update(cx, |v, cx| {
                v.act(w, cx, |m| {
                    m.set_view(View::Outbox);
                    m.row_clicked(row);
                })
            })
        });
        in_window(cx, o.window, |w, _| {
            assert_named(w, "Plans", Role::Heading);
            assert_named(w, "Retry", Role::Button);
            assert_named(w, "Edit", Role::Button);
            assert_named(w, "Discard", Role::Button);
        });
        in_window(cx, o.window, |w, cx| w.click("outbox-retry", cx));
        cx.run_until_parked();
        let jobs = o.hub.read_with(cx, |h, _| h.captured());
        assert!(
            jobs.iter().any(
                |(a, j)| *a == seeded.account && matches!(j, Job::Send { row: r } if *r == row)
            ),
            "the job reached the workers"
        );
        assert_eq!(
            cache.open().outbox_entry(row).unwrap().map(|e| e.state),
            Some(Outbox::Queued)
        );
        in_window(cx, o.window, |w, cx| w.click("outbox-discard", cx));
        cx.run_until_parked();
        in_window(cx, o.window, |w, cx| w.click("dialog-confirm", cx));
        cx.run_until_parked();
        assert!(cache.open().outbox_entry(row).unwrap().is_none());
        assert_eq!(
            o.hub
                .read_with(cx, |h, _| h.sync().line().map(|l| l.said.clone())),
            Some(Said::Discarded)
        );
    }

    /// Port of `a_wall_clock_gap_means_a_wake` (the hub's half; the detector
    /// is `wake::tests`): the hub starts the watch with its first sync, and
    /// after a wake the workers drop their connections at once and, once
    /// the network has had a moment, every account syncs.
    #[gpui_kit::test]
    fn a_wake_syncs_every_account_once_the_network_is_back(cx: &mut TestAppContext) {
        use crate::wake::Wake;
        let (_cache, db, seeded) = test_support::seeded("wake", 3);
        let o = open(cx, db);
        assert!(
            o.hub.read_with(cx, |h, _| h.watches_for_a_wake()),
            "the watch starts with the first sync"
        );
        o.hub.read_with(cx, |h, _| drop(h.captured()));
        o.hub.update(cx, |h, cx| h.woke(Wake::Woke, cx));
        o.hub.update(cx, |h, cx| h.woke(Wake::Settled, cx));
        let jobs = o.hub.read_with(cx, |h, _| h.captured());
        assert!(
            jobs.iter()
                .any(|(a, j)| *a == seeded.account && matches!(j, Job::SyncAll { .. })),
            "the account syncs after the wake"
        );
    }
}
