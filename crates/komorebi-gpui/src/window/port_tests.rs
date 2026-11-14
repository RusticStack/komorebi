//! Test-port rows that need the whole main window (docs/GPUI-TEST-PORT.md,
//! the rest of `src/app/composer/tests/tabs_tests.rs` and a few of
//! `src/app/composer.rs`): the pane's draft and the tabs, the window's
//! close, a vanished folder or account, the send offer. All on a throwaway
//! seeded cache; each test names the Slint test it ports.

use std::time::Duration;

use gpui::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use gpui_kit::test::TestWindowExt as _;
use komorebi::compose::Field;
use komorebi::db::{Db, NewHeader};
use komorebi::role::Role as FolderRole;
use komorebi::tabs::TabKind;

use super::MainWindow;
use crate::test_support::{self, TempCache, in_window, press, type_text};
use crate::views::composer::{ComposerView, Draft, Status};
use crate::views::mail::MailEvent;
use crate::views::tabs::message::MessageEvent;

/// The main window over a seeded cache: four inbox messages, Sent and
/// Archive.
struct Rig {
    cache: TempCache,
    db: Db,
    ids: Vec<i64>,
    account: i64,
    inbox: i64,
    window: AnyWindowHandle,
    view: Entity<MainWindow>,
}

impl Rig {
    fn new(name: &str, cx: &mut TestAppContext) -> Rig {
        let (cache, db, seeded) = test_support::seeded(name, 4);
        let ids = db.message_ids(&[seeded.inbox], "").expect("ids");
        db.set_setting("mark_read_delay_ms", "0")
            .expect("read at once");
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
        Rig {
            cache,
            db,
            ids,
            account: seeded.account,
            inbox: seeded.inbox,
            window,
            view,
        }
    }

    /// Runs `f` on the window as one of its own methods would run.
    fn act(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut MainWindow, &mut gpui::Window, &mut gpui::Context<MainWindow>),
    ) {
        cx.update_window(self.window, |_, w, cx| {
            self.view.update(cx, |v, cx| f(v, w, cx))
        })
        .expect("update");
        cx.run_until_parked();
    }

    fn kinds(&self, cx: &mut TestAppContext) -> Vec<TabKind> {
        self.view
            .read_with(cx, |v, _| v.tabs().iter().map(|t| t.kind).collect())
    }

    fn active(&self, cx: &mut TestAppContext) -> usize {
        self.view.read_with(cx, |v, _| v.tabs().active())
    }

    fn title(&self, cx: &mut TestAppContext, tab: usize) -> String {
        self.view.read_with(cx, |v, _| {
            v.tabs()
                .get(tab)
                .map(|t| t.title.clone())
                .unwrap_or_default()
        })
    }

    fn keys(&self, cx: &mut TestAppContext, keys: &[&str]) {
        in_window(cx, self.window, |w, cx| press(w, keys, cx));
        cx.run_until_parked();
    }

    /// The mail view's composer (the Mail tab's reading pane).
    fn pane(&self, cx: &mut TestAppContext) -> Entity<ComposerView> {
        self.view.read_with(cx, |v, cx| {
            v.mail().expect("the mail view").read(cx).composer().clone()
        })
    }

    /// The composer of the draft tab at `tab`.
    fn draft_view(&self, cx: &mut TestAppContext, tab: usize) -> Entity<ComposerView> {
        let TabKind::Draft(id) = self.kinds(cx)[tab] else {
            panic!("tab {tab} is no draft");
        };
        self.view
            .read_with(cx, |v, _| v.draft_composer(id).expect("its composer"))
    }

    /// Ctrl+N: a new message in the Mail tab's pane.
    fn new_mail(&self, cx: &mut TestAppContext) {
        self.keys(cx, &["ctrl-n"]);
    }

    /// `text` typed into `field` of `composer`.
    fn write(
        &self,
        cx: &mut TestAppContext,
        composer: &Entity<ComposerView>,
        field: Field,
        text: &str,
    ) {
        let composer = composer.clone();
        in_window(cx, self.window, |w, cx| {
            composer.update(cx, |c, cx| c.focus_field(field, w, cx));
            type_text(w, text, cx);
        });
        cx.run_until_parked();
    }

    /// Message `id` chosen in the list.
    fn select(&self, cx: &mut TestAppContext, id: i64) {
        self.act(cx, |v, w, cx| {
            let mail = v.mail().cloned().expect("the mail view");
            mail.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    m.select_message(id);
                })
            });
        });
    }

    fn subject(&self, cx: &mut TestAppContext, composer: &Entity<ComposerView>) -> String {
        composer.read_with(cx, |c, _| {
            c.drafts
                .active()
                .map(|d| d.subject.clone())
                .unwrap_or_default()
        })
    }
}

/// Port of `selecting_a_message_moves_a_dirty_pane_draft_to_a_tab`: a new
/// message with writing in it, then another message chosen: the draft is
/// saved and goes to a tab behind the shown one ("Draft kept in a tab"); an
/// empty one goes; the tab shows the draft again.
#[gpui_kit::test]
fn selecting_a_message_moves_a_dirty_pane_draft_to_a_tab(cx: &mut TestAppContext) {
    let r = Rig::new("port-aside", cx);
    r.new_mail(cx);
    let pane = r.pane(cx);
    r.write(cx, &pane, Field::Subject, "Plans");
    r.select(cx, r.ids[1]);
    let kinds = r.kinds(cx);
    assert_eq!(kinds.len(), 2);
    assert!(matches!(kinds[1], TabKind::Draft(_)));
    assert_eq!(r.active(cx), 0, "in the background");
    assert!(!pane.read_with(cx, |c, _| c.composing()));
    assert_eq!(
        pane.read_with(cx, |c, _| c.status().cloned()),
        Some(Status::KeptInTab)
    );
    assert_eq!(r.title(cx, 1), "Plans");
    // Saved already, so a restart would have it.
    assert_eq!(r.db.count_drafts().unwrap(), 1);
    // An empty draft is dropped, as before (no tab).
    r.new_mail(cx);
    r.select(cx, r.ids[2]);
    assert_eq!(r.kinds(cx).len(), 2);
    // Its tab shows it again.
    r.act(cx, |v, w, cx| v.select_tab(1, w, cx));
    let tab = r.draft_view(cx, 1);
    assert!(tab.read_with(cx, |c, _| c.composing()));
    assert_eq!(r.subject(cx, &tab), "Plans");
}

/// Port of `closing_a_draft_tab_saves_it_and_an_empty_one_goes` (its
/// Ctrl+Shift+T half is `a_closed_draft_tab_comes_back_with_ctrl_shift_t`): a
/// draft tab closed with Ctrl+W saves its draft; a reply nobody wrote in
/// goes without a trace.
#[gpui_kit::test]
fn closing_a_draft_tab_saves_it_and_an_empty_one_goes(cx: &mut TestAppContext) {
    let r = Rig::new("port-draft-close", cx);
    let message = r.ids[0];
    r.act(cx, |v, w, cx| {
        v.on_mail(
            &MailEvent::OpenTab {
                id: message,
                background: false,
            },
            w,
            cx,
        )
    });
    // New mail from a message tab: a composer tab to its right.
    r.act(cx, |v, w, cx| v.open_draft_tab(Draft::New, w, cx));
    assert!(matches!(r.kinds(cx)[2], TabKind::Draft(_)));
    assert_eq!(r.active(cx), 2);
    let first = r.title(cx, 2);
    let tab = r.draft_view(cx, 2);
    r.write(cx, &tab, Field::Subject, "Kept");
    assert_eq!(r.title(cx, 2), "Kept", "the title follows the subject");
    assert_eq!(first, "New message");
    r.keys(cx, &["ctrl-w"]);
    assert_eq!(r.kinds(cx).len(), 2);
    assert_eq!(r.active(cx), 1);
    assert_eq!(r.db.count_drafts().unwrap(), 1, "saved as it closed");
    // A reply from the message tab, untouched: closing it drops it.
    r.act(cx, |v, w, cx| v.select_tab(1, w, cx));
    let drafts = r.db.count_drafts().unwrap();
    r.act(cx, |v, w, cx| {
        v.message_action(
            &MessageEvent::Reply {
                message,
                all: false,
            },
            w,
            cx,
        )
    });
    assert!(matches!(r.kinds(cx)[2], TabKind::Draft(_)));
    r.keys(cx, &["ctrl-w"]);
    assert_eq!(r.kinds(cx).len(), 2);
    assert_eq!(r.db.count_drafts().unwrap(), drafts);
}

/// Port of `main_close_closes_popouts_then_flushes`: the window's close
/// closes its message windows, and the pane's draft and the tabs are saved
/// as the app quits.
#[gpui_kit::test]
fn closing_the_main_window_closes_popouts_then_flushes(cx: &mut TestAppContext) {
    let r = Rig::new("port-close", cx);
    let (second, third) = (r.ids[1], r.ids[2]);
    r.act(cx, |v, w, cx| v.open_in_window(second, w, cx));
    assert_eq!(r.view.read_with(cx, |v, _| v.popouts()), [second]);
    r.new_mail(cx);
    let pane = r.pane(cx);
    r.write(cx, &pane, Field::Subject, "Typed just now");
    r.act(cx, |v, w, cx| {
        v.on_mail(
            &MailEvent::OpenTab {
                id: third,
                background: true,
            },
            w,
            cx,
        )
    });
    let closed = in_window(cx, r.window, |w, cx| {
        r.view.update(cx, |v, cx| v.should_close(w, cx))
    });
    assert!(closed, "the window may close");
    cx.run_until_parked();
    assert!(r.view.read_with(cx, |v, _| v.popouts()).is_empty());
    assert_eq!(r.db.count_drafts().unwrap(), 0, "the autosave still waits");
    cx.quit();
    assert_eq!(r.db.count_drafts().unwrap(), 1, "saved as the app quit");
    let tabs = r.db.settings().unwrap().tabs;
    assert!(
        tabs.contains(&format!("m{third}")),
        "the tabs are kept for the next start: {tabs}"
    );
}

/// Port of `language_and_theme_switch_reach_popouts`: the language and the
/// colours are the app's, so a message window follows a switch: its
/// buttons are named in the new language at once.
#[gpui_kit::test]
fn language_and_theme_switch_reach_popouts(cx: &mut TestAppContext) {
    use komorebi::db::ThemeChoice;
    use komorebi::i18n::Lang;
    let r = Rig::new("port-popout-theme", cx);
    let second = r.ids[1];
    r.act(cx, |v, w, cx| v.open_in_window(second, w, cx));
    let popout = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find(|w| *w != r.window)
        .expect("the message window");
    let reply = |cx: &mut TestAppContext| {
        in_window(cx, popout, |w, _| {
            test_support::names(w)
                .into_iter()
                .any(|(_, n)| n == "Reply" || n == "Responder")
        })
    };
    assert!(reply(cx), "the message window has its Reply button");
    let reply_text = |cx: &mut TestAppContext| {
        in_window(cx, popout, |w, _| {
            test_support::names(w)
                .into_iter()
                .map(|(_, n)| n)
                .find(|n| n == "Reply" || n == "Responder")
        })
    };
    assert_eq!(reply_text(cx).as_deref(), Some("Reply"));
    test_support::set_lang(cx, Lang::PtPt);
    assert_eq!(reply_text(cx).as_deref(), Some("Responder"));
    test_support::set_theme(cx, ThemeChoice::Dark);
    let dark = cx.update(|cx| {
        use crate::theme::ActiveTokens as _;
        cx.tokens().color.canvas
    });
    test_support::set_theme(cx, ThemeChoice::Light);
    let light = cx.update(|cx| {
        use crate::theme::ActiveTokens as _;
        cx.tokens().color.canvas
    });
    assert_ne!(
        dark, light,
        "the tokens are the app's, drawn in every window"
    );
    test_support::set_lang(cx, Lang::En);
    assert_eq!(reply_text(cx).as_deref(), Some("Reply"));
}

/// A message in a second folder, "Work" (unread): its id.
fn work_message(r: &Rig) -> i64 {
    let mut db = r.cache.open();
    let work = db
        .upsert_folder(r.account, "Work", "Work", 0, FolderRole::Other)
        .expect("a folder");
    db.insert_headers(
        r.account,
        work,
        &[NewHeader {
            uid: 9,
            msgid: "w9@example.org".into(),
            subject: "Elsewhere".into(),
            from_addr: "cy@example.org".into(),
            to: "me@example.org".into(),
            date: 1_700_000_009,
            ..Default::default()
        }],
    )
    .expect("a message");
    let id = db.message_ids(&[work], "").expect("ids")[0];
    db.retain_folders(r.account, &[r.inbox, work])
        .expect("keep both");
    id
}

fn gone(r: &Rig, cx: &mut TestAppContext) -> Vec<bool> {
    r.view
        .read_with(cx, |v, _| v.tabs().iter().map(|t| t.gone).collect())
}

fn open_tab(r: &Rig, cx: &mut TestAppContext, id: i64, background: bool) {
    r.act(cx, |v, w, cx| {
        v.on_mail(&MailEvent::OpenTab { id, background }, w, cx)
    });
}

fn changed(r: &Rig, cx: &mut TestAppContext) {
    r.act(cx, |v, w, cx| v.on_mail(&MailEvent::Changed, w, cx));
}

/// Port of `a_vanished_folder_marks_the_open_message_tab_gone` and
/// `a_vanished_folder_marks_a_background_message_tab_gone`: once the
/// message's folder is gone from the cache, its tab is marked and the tab
/// stays, readable; shown or not.
#[gpui_kit::test]
fn a_vanished_folder_marks_its_message_tabs_gone(cx: &mut TestAppContext) {
    let r = Rig::new("port-folder-gone", cx);
    let work = work_message(&r);
    // The open tab.
    open_tab(&r, cx, work, false);
    assert_eq!(r.active(cx), 1);
    r.cache
        .open()
        .retain_folders(r.account, &[r.inbox])
        .expect("the folder goes");
    changed(&r, cx);
    assert_eq!(gone(&r, cx), [false, true]);
    assert_eq!(r.kinds(cx).len(), 2, "the tab stays, readable");
    in_window(cx, r.window, |window, _| {
        assert!(
            test_support::names(window)
                .iter()
                .any(|(_, n)| n.contains(&crate::views::tabs::text::gone())),
            "the tab says the message is gone"
        );
    });
    // A background tab: marked while the Mail tab shows, and says so when
    // it is shown.
    let r = Rig::new("port-folder-gone-2", cx);
    let work = work_message(&r);
    open_tab(&r, cx, work, true);
    assert_eq!(r.active(cx), 0);
    r.cache
        .open()
        .retain_folders(r.account, &[r.inbox])
        .expect("the folder goes");
    changed(&r, cx);
    assert_eq!(gone(&r, cx), [false, true]);
    r.act(cx, |v, w, cx| v.select_tab(1, w, cx));
    in_window(cx, r.window, |window, _| {
        assert!(
            test_support::names(window)
                .iter()
                .any(|(_, n)| n.contains(&crate::views::tabs::text::gone()))
        );
    });
}

/// Port of `a_removed_account_marks_its_message_tabs_gone` and
/// `a_removed_account_marks_a_background_tab_gone`: with its account
/// removed (Settings, Remove account: the cache loses it and the hub its
/// workers) the open and the background message tabs are marked gone.
#[gpui_kit::test]
fn a_removed_account_marks_its_message_tabs_gone(cx: &mut TestAppContext) {
    let r = Rig::new("port-account-gone", cx);
    open_tab(&r, cx, r.ids[1], false);
    open_tab(&r, cx, r.ids[2], true);
    assert_eq!(gone(&r, cx), [false, false, false]);
    r.cache
        .open()
        .remove_account(r.account)
        .expect("the account goes");
    r.act(cx, |v, w, cx| {
        let hub = v.hub().cloned().expect("the hub");
        hub.update(cx, |h, cx| h.remove_account(r.account, cx));
        v.on_mail(&MailEvent::Changed, w, cx);
    });
    assert_eq!(gone(&r, cx), [false, true, true]);
    assert_eq!(r.kinds(cx).len(), 3, "the tabs stay");
    assert_eq!(r.active(cx), 1, "the one shown is the first one opened");
    in_window(cx, r.window, |window, _| {
        assert!(
            test_support::names(window)
                .iter()
                .any(|(_, n)| n.contains(&crate::views::tabs::text::gone()))
        );
    });
    r.act(cx, |v, w, cx| v.select_tab(1, w, cx));
    in_window(cx, r.window, |window, _| {
        assert!(
            test_support::names(window)
                .iter()
                .any(|(_, n)| n.contains(&crate::views::tabs::text::gone()))
        );
    });
}

/// Port of `delete_in_a_message_tab_never_acts_on_a_draft_row`: with the
/// Mail tab on Drafts and a saved draft listed, Delete in a message tab is
/// that message's, never the draft row's.
#[gpui_kit::test]
fn delete_in_a_message_tab_never_acts_on_a_draft_row(cx: &mut TestAppContext) {
    use crate::views::mail::model::View;
    let r = Rig::new("port-tab-delete", cx);
    // A saved draft, its row listed in the Mail tab's Drafts view.
    r.new_mail(cx);
    let pane = r.pane(cx);
    r.write(cx, &pane, Field::To, "ana@example.org");
    r.write(cx, &pane, Field::Subject, "Draft");
    r.keys(cx, &["escape"]);
    assert_eq!(r.db.count_drafts().unwrap(), 1);
    let message = r.ids[0];
    open_tab(&r, cx, message, false);
    r.act(cx, |v, w, cx| {
        v.select_tab(0, w, cx);
        let mail = v.mail().cloned().expect("the mail view");
        mail.update(cx, |m, cx| m.act(w, cx, |m| m.set_view(View::Drafts)));
        v.select_tab(1, w, cx);
    });
    r.act(cx, |v, w, cx| {
        v.on_message_tab(message, &MessageEvent::Delete(message), w, cx)
    });
    assert_eq!(r.db.count_drafts().unwrap(), 1, "the draft is untouched");
    let composing = r.pane(cx).read_with(cx, |c, _| c.composing());
    assert!(!composing, "nothing was opened to discard");
}

/// Port of `settings_from_a_message_tab_keep_the_pane_and_f6_stays_visible`:
/// Settings opened from a message tab (Ctrl+,) shows over that tab with the
/// folder pane beside it, and Esc steps back to the message.
#[gpui_kit::test]
fn settings_from_a_message_tab_keep_the_pane(cx: &mut TestAppContext) {
    let r = Rig::new("port-tab-settings", cx);
    open_tab(&r, cx, r.ids[1], false);
    assert_eq!(r.active(cx), 1);
    r.keys(cx, &["ctrl-,"]);
    assert_eq!(r.active(cx), 1, "Settings is a page over the tab");
    assert!(r.view.read_with(cx, |v, _| v.settings().is_some()));
    in_window(cx, r.window, |window, _| {
        test_support::assert_named(window, "Folders", gpui::Role::Tree);
    });
    r.keys(cx, &["escape"]);
    assert_eq!(r.active(cx), 1);
    assert!(
        r.view.read_with(cx, |v, _| v.place()
            != Some(crate::views::mail::Place::Settings)),
        "Esc steps back"
    );
}

/// How long the composer's undo window lasts in a test (the default, 5 s).
const UNDO: Duration = Duration::from_secs(5);

/// Port of `undo_send_reopens_in_a_new_tab`: a message sent from its draft
/// tab leaves the tab; Undo on the offer opens the draft again in a new tab
/// to the right of the shown one, and the offer counts down meanwhile
/// (`the_undo_offer_counts_down_and_a_delivery_says_sent`, row 107).
#[gpui_kit::test]
fn undo_send_reopens_in_a_new_tab(cx: &mut TestAppContext) {
    let r = Rig::new("port-undo-send", cx);
    let message = r.ids[0];
    open_tab(&r, cx, message, false);
    r.act(cx, |v, w, cx| v.open_draft_tab(Draft::New, w, cx));
    assert_eq!(r.active(cx), 2);
    let tab = r.draft_view(cx, 2);
    r.write(cx, &tab, Field::To, "ana@example.org");
    r.write(cx, &tab, Field::Subject, "Plans");
    r.keys(cx, &["ctrl-enter"]);
    // Sent from its tab: the tab went.
    assert_eq!(r.kinds(cx).len(), 2);
    assert_eq!(r.active(cx), 1);
    let hub = r
        .view
        .read_with(cx, |v, _| v.hub().cloned().expect("the hub"));
    let line = |cx: &mut TestAppContext| {
        hub.read_with(cx, |h, _| {
            h.sync()
                .line()
                .map(|l| crate::views::mail::sync_text::said(&l.said))
        })
    };
    let secs = UNDO.as_secs();
    assert_eq!(line(cx), Some(format!("Sending “Plans” in {secs} s")));
    // The count-down: one second at a time on the test clock.
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        line(cx),
        Some(format!("Sending “Plans” in {} s", secs - 1)),
        "the offer stays and counts down"
    );
    // Undo on the offer.
    r.act(cx, |_, _, cx| hub.update(cx, |h, cx| h.press_action(cx)));
    assert_eq!(r.kinds(cx).len(), 3);
    assert_eq!(r.active(cx), 2);
    let back = r.draft_view(cx, 2);
    assert!(back.read_with(cx, |c, _| c.composing()));
    assert_eq!(r.subject(cx, &back), "Plans");
}

/// The second half of `closing_a_draft_tab_saves_it_and_an_empty_one_goes`:
/// Ctrl+Shift+T brings a closed draft tab back with the draft as it was
/// saved, and the tabs setting names it for the next start.
#[gpui_kit::test]
fn a_closed_draft_tab_comes_back_with_ctrl_shift_t(cx: &mut TestAppContext) {
    let r = Rig::new("port-draft-reopen", cx);
    open_tab(&r, cx, r.ids[0], false);
    r.act(cx, |v, w, cx| v.open_draft_tab(Draft::New, w, cx));
    let tab = r.draft_view(cx, 2);
    r.write(cx, &tab, Field::Subject, "Kept");
    r.keys(cx, &["ctrl-w"]);
    assert_eq!(r.kinds(cx).len(), 2);
    r.keys(cx, &["ctrl-shift-t"]);
    assert!(matches!(r.kinds(cx)[2], TabKind::Draft(_)));
    let back = r.draft_view(cx, 2);
    assert_eq!(r.subject(cx, &back), "Kept");
}

/// Port of `a_second_compose_opens_a_tab_and_switching_round_trips`: New
/// mail while a message with writing in it shows keeps it in a tab and
/// starts a blank one; switching to the tab finds every field as typed, and
/// the blank one is dropped untouched.
#[gpui_kit::test]
fn a_second_compose_opens_a_tab_and_switching_round_trips(cx: &mut TestAppContext) {
    let r = Rig::new("port-second-compose", cx);
    r.new_mail(cx);
    let pane = r.pane(cx);
    r.write(cx, &pane, Field::To, "Ana <ana@example.org>");
    r.write(cx, &pane, Field::Subject, "Plans");
    let draft = |cx: &mut TestAppContext, c: &Entity<ComposerView>| {
        c.read_with(cx, |c, _| {
            c.drafts.active().map(|d| (d.to.clone(), d.subject.clone()))
        })
    };
    assert_eq!(
        draft(cx, &pane),
        Some(("Ana <ana@example.org>".into(), "Plans".into()))
    );
    // New mail while it shows keeps it and starts another, in a tab.
    r.new_mail(cx);
    let kinds = r.kinds(cx);
    assert_eq!(kinds.len(), 2);
    assert!(matches!(kinds[1], TabKind::Draft(_)));
    assert_eq!(r.active(cx), 0);
    assert_eq!(draft(cx, &pane), Some((String::new(), String::new())));
    assert_eq!(r.db.count_drafts().unwrap(), 1);
    // Switching to the tab finds every field as it was.
    r.act(cx, |v, w, cx| v.select_tab(1, w, cx));
    let tab = r.draft_view(cx, 1);
    assert_eq!(
        draft(cx, &tab),
        Some(("Ana <ana@example.org>".into(), "Plans".into()))
    );
    // And back: the blank message is still there, and Esc on it drops it.
    r.act(cx, |v, w, cx| v.select_tab(0, w, cx));
    assert!(pane.read_with(cx, |c, _| c.composing()));
    in_window(cx, r.window, |w, cx| {
        pane.update(cx, |c, cx| c.focus_field(Field::To, w, cx))
    });
    r.keys(cx, &["escape"]);
    assert!(!pane.read_with(cx, |c, _| c.composing()));
    assert_eq!(
        r.db.count_drafts().unwrap(),
        1,
        "the blank one left no trace"
    );
}

/// Port of `tab_switch_restores_scroll_and_view_flags` (the Mail tab's
/// half): the Mail tab gets back what its pane showed. Each message tab
/// owns its `MessageView` and so its reader (scroll and view flags) until
/// the tab closes; nothing is saved and restored.
#[gpui_kit::test]
fn the_mail_tab_gets_back_what_its_pane_showed(cx: &mut TestAppContext) {
    let r = Rig::new("port-tab-view", cx);
    r.select(cx, r.ids[0]);
    let open = |cx: &mut TestAppContext| {
        r.view.read_with(cx, |v, cx| {
            v.mail().and_then(|m| m.read(cx).mail().open_id())
        })
    };
    assert_eq!(open(cx), Some(r.ids[0]));
    open_tab(&r, cx, r.ids[1], false);
    assert_eq!(r.active(cx), 1);
    r.act(cx, |v, w, cx| v.select_tab(0, w, cx));
    assert_eq!(open(cx), Some(r.ids[0]), "the pane still shows its message");
    assert_eq!(r.kinds(cx).len(), 2);
}

/// Port of `tab_switch_flushes_autosave_of_the_leaving_draft`: the autosave
/// waits (a debounce); leaving the draft's tab saves it at once.
#[gpui_kit::test]
fn a_tab_switch_flushes_the_autosave_of_the_leaving_draft(cx: &mut TestAppContext) {
    let r = Rig::new("port-flush", cx);
    open_tab(&r, cx, r.ids[0], false);
    r.act(cx, |v, w, cx| v.open_draft_tab(Draft::New, w, cx));
    let tab = r.draft_view(cx, 2);
    r.write(cx, &tab, Field::To, "ana@example.org");
    r.write(cx, &tab, Field::Subject, "Typed just now");
    assert_eq!(r.db.count_drafts().unwrap(), 0, "the autosave waits");
    r.keys(cx, &["ctrl-1"]);
    assert_eq!(r.active(cx), 0);
    let rows = r.db.draft_ids("").unwrap();
    assert_eq!(rows.len(), 1, "saved as the tab was left");
    assert_eq!(
        r.db.draft(rows[0]).unwrap().unwrap().subject,
        "Typed just now"
    );
}

// ------------------------------------------------------- settings, shell

/// Settings open over the window, and what it tells the shell.
fn settings_of(r: &Rig, cx: &mut TestAppContext) -> Entity<crate::views::settings::SettingsView> {
    r.keys(cx, &["ctrl-,"]);
    r.view
        .read_with(cx, |v, _| v.settings().expect("Settings shows"))
}

/// What the hub's workers were handed since the last read.
fn jobs(r: &Rig, cx: &mut TestAppContext) -> Vec<(i64, komorebi::mail::Job)> {
    r.view
        .read_with(cx, |v, cx| v.hub().expect("the hub").read(cx).captured())
}

fn has_workers(r: &Rig, cx: &mut TestAppContext) -> bool {
    let account = r.account;
    r.view.read_with(cx, |v, cx| {
        v.hub().expect("the hub").read(cx).has_workers(account)
    })
}

/// Port of `switched_off_accounts_spawn_no_workers` (the shell's half; the
/// switch itself is `views::settings::tests::port::...`): the account
/// switched off loses its workers at once, and switched on again it gets
/// new ones and syncs in full.
#[gpui_kit::test]
fn switched_off_accounts_spawn_no_workers(cx: &mut TestAppContext) {
    use crate::views::settings::SettingsEvent;
    let r = Rig::new("port-switch-off", cx);
    let settings = settings_of(&r, cx);
    let id = r.account;
    assert!(has_workers(&r, cx), "an enabled account has workers");
    r.db.set_account_enabled(id, false).expect("switch off");
    settings.update(cx, |_, cx| {
        cx.emit(SettingsEvent::AccountEnabled { id, on: false })
    });
    cx.run_until_parked();
    assert!(!has_workers(&r, cx), "switched off: no workers");
    let _ = jobs(&r, cx);
    r.db.set_account_enabled(id, true).expect("switch on");
    settings.update(cx, |_, cx| {
        cx.emit(SettingsEvent::AccountEnabled { id, on: true })
    });
    cx.run_until_parked();
    assert!(has_workers(&r, cx), "switched on: new workers");
    assert!(
        jobs(&r, cx)
            .iter()
            .any(|(a, j)| *a == id && matches!(j, komorebi::mail::Job::SyncAll { full: true })),
        "and a full sync"
    );
}

/// Port of `download_again_keeps_the_account_drafts_and_settings` (the
/// shell's half; the question is `views::settings::tests::port::...`): the
/// account's messages go and are fetched again by new workers; its
/// drafts and the settings stay, and the status line says so.
#[gpui_kit::test]
fn download_again_keeps_the_account_drafts_and_settings(cx: &mut TestAppContext) {
    use crate::views::mail::sync::Said;
    use crate::views::settings::SettingsEvent;
    let r = Rig::new("port-redownload", cx);
    // A draft of the account, kept by the reset.
    r.new_mail(cx);
    let pane = r.pane(cx);
    r.write(cx, &pane, Field::Subject, "Kept draft");
    pane.update(cx, |c, cx| c.save_all(cx));
    cx.run_until_parked();
    assert_eq!(r.db.count_drafts().unwrap(), 1);
    r.db.set_setting("undo_send_secs", "7").expect("a setting");
    let settings = settings_of(&r, cx);
    let id = r.account;
    let _ = jobs(&r, cx);
    settings.update(cx, |_, cx| cx.emit(SettingsEvent::Redownload(id)));
    cx.run_until_parked();
    assert!(
        r.db.message_ids(&[r.inbox], "").unwrap().is_empty(),
        "the account's messages are forgotten"
    );
    assert_eq!(r.db.count_drafts().unwrap(), 1, "the draft stays");
    assert_eq!(
        r.db.settings().unwrap().undo_send_secs,
        7,
        "the settings stay"
    );
    assert!(has_workers(&r, cx), "new workers");
    assert!(
        jobs(&r, cx)
            .iter()
            .any(|(a, j)| *a == id && matches!(j, komorebi::mail::Job::SyncAll { full: true })),
        "fetching it all again"
    );
    let said = r.view.read_with(cx, |v, cx| {
        v.hub()
            .expect("the hub")
            .read(cx)
            .sync()
            .line()
            .map(|l| l.said.clone())
    });
    let email =
        r.db.accounts()
            .unwrap()
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| a.email)
            .expect("the account");
    assert_eq!(
        said,
        Some(Said::Text(crate::views::settings::text::redownloading(
            &email
        ))),
        "the line names the account and says it downloads again"
    );
}

/// Port of `the_outbox_offers_edit_retry_and_discard` (its Edit half; Retry
/// and Discard are `sync_tests::port::the_outbox_pane_retries_and_discards`):
/// Edit takes a failed entry out of the Outbox, back to Drafts, and opens it
/// in the pane's composer.
#[gpui_kit::test]
fn the_outbox_edit_returns_the_entry_to_drafts_and_opens_it(cx: &mut TestAppContext) {
    use komorebi::db::{DraftRow, Outbox};
    use std::sync::atomic::AtomicBool;
    let r = Rig::new("port-outbox-edit", cx);
    let row =
        r.db.insert_draft(&DraftRow {
            account_id: r.account,
            to: "ana@example.com".into(),
            subject: "Plans".into(),
            body: "See you on Friday".into(),
            ..Default::default()
        })
        .expect("a draft");
    assert!(r.db.outbox_send(row).unwrap());
    assert!(
        r.db.store_raw(row, b"raw", &AtomicBool::new(false))
            .unwrap()
    );
    assert!(r.db.outbox_queue(row).unwrap());
    assert!(r.db.claim_outbox(row).unwrap().is_some());
    r.db.outbox_result(row, Outbox::Failed, None, "451 4.3.0 try later")
        .unwrap();
    r.act(cx, |v, w, cx| {
        let mail = v.mail().cloned().expect("the mail view");
        mail.update(cx, |m, cx| {
            m.act(w, cx, |m| {
                m.set_view(crate::views::mail::model::View::Outbox);
                m.row_clicked(row);
            })
        });
    });
    in_window(cx, r.window, |w, cx| w.click("outbox-edit", cx));
    cx.run_until_parked();
    assert!(
        r.db.outbox_entry(row).unwrap().is_none(),
        "out of the Outbox"
    );
    let pane = r.pane(cx);
    assert!(pane.read_with(cx, |c, _| c.composing()), "open to edit");
    assert_eq!(r.subject(cx, &pane), "Plans");
}

/// Port of `escape_dismisses_the_status_line_last`: with nothing else for
/// Esc to close (no search, no message), it dismisses the sync line; a search
/// or an open message goes first.
#[gpui_kit::test]
fn escape_dismisses_the_status_line_last(cx: &mut TestAppContext) {
    use crate::views::mail::sync::Said;
    let r = Rig::new("port-esc-line", cx);
    let hub = r
        .view
        .read_with(cx, |v, _| v.hub().cloned().expect("the hub"));
    let line = |cx: &mut TestAppContext| {
        hub.read_with(cx, |h, _| {
            h.sync()
                .line()
                .is_some_and(|l| l.said != Said::Text(String::new()))
        })
    };
    hub.update(cx, |h, cx| {
        h.say(Said::Text("Something happened".into()), false, cx)
    });
    r.select(cx, r.ids[0]);
    assert!(line(cx));
    r.keys(cx, &["escape"]);
    assert!(r.view.read_with(cx, |v, cx| {
        v.mail().unwrap().read(cx).mail().open_id().is_none()
    }));
    assert!(line(cx), "the message closed first");
    r.keys(cx, &["escape"]);
    assert!(!line(cx), "then the line");
}

/// Port of `f6_cycles_the_panes_and_the_folders_take_arrows` (its second
/// half; F6 is `window::tests::f6_cycles_the_panes`): with the focus in the
/// folder pane, Down and Up move along the folders and Enter opens the
/// one reached.
#[gpui_kit::test]
fn the_folders_take_the_arrows(cx: &mut TestAppContext) {
    use crate::views::mail::model::View;
    let r = Rig::new("port-folder-arrows", cx);
    r.act(cx, |v, w, cx| {
        let mail = v.mail().cloned().expect("the mail view");
        mail.update(cx, |m, cx| w.focus(m.nav_focus(), cx));
    });
    let view = |cx: &mut TestAppContext| {
        r.view
            .read_with(cx, |v, cx| v.mail().unwrap().read(cx).mail().view())
    };
    let first = view(cx);
    // Down moves the line, Enter opens it: the view changes when the line
    // is another folder than the one shown.
    r.keys(cx, &["down", "down", "enter"]);
    let opened = view(cx);
    assert_ne!(opened, first, "Down and Enter opened another folder");
    assert!(matches!(opened, View::Folder(_)), "{opened:?}");
    // Up and Enter go back.
    r.keys(cx, &["up", "up", "enter"]);
    assert_ne!(view(cx), opened);
}

// ------------------------------------------------- Settings > Signatures

/// The Signatures page over a throwaway cache.
fn signatures(
    name: &str,
    cx: &mut TestAppContext,
) -> (
    TempCache,
    AnyWindowHandle,
    Entity<crate::views::settings::signatures::SignaturesPage>,
) {
    use crate::views::settings::signatures::SignaturesPage;
    let cache = TempCache::new(name);
    let db = cache.open();
    test_support::init_app(cx);
    let (window, page) = test_support::open(cx, move |window, cx| {
        cx.new(|cx| SignaturesPage::new(std::rc::Rc::new(db), window, cx))
    });
    (cache, window, page)
}

/// Port of `signatures_are_made_renamed_and_deleted_in_settings`: a new
/// signature needs a name that no other has; a saved one is renamed in its
/// page; Delete takes it out of the list.
#[gpui_kit::test]
fn signatures_are_made_renamed_and_deleted_in_settings(cx: &mut TestAppContext) {
    use crate::views::settings::signatures::SigError;
    let (_cache, window, page) = signatures("port-signatures", cx);
    let names = |cx: &mut TestAppContext| -> Vec<String> {
        page.read_with(cx, |p, _| p.list().iter().map(|s| s.name.clone()).collect())
    };
    let set_name = |cx: &mut TestAppContext, text: &'static str| {
        cx.update_window(window, |_, w, cx| {
            let input = page
                .read(cx)
                .name_input()
                .expect("the page is open")
                .clone();
            input.update(cx, |s, cx| s.set_value(text, w, cx));
        })
        .expect("update");
    };
    // New, without a name: refused.
    cx.update_window(window, |_, w, cx| {
        page.update(cx, |p, cx| p.open(None, w, cx))
    })
    .expect("open");
    page.update(cx, |p, cx| p.save(cx));
    assert_eq!(
        page.read_with(cx, |p, _| p.error().cloned()),
        Some(SigError::EmptyName)
    );
    // New, named: saved, and the page goes back to the list.
    set_name(cx, "Work");
    page.update(cx, |p, cx| p.save(cx));
    assert_eq!(names(cx), ["Work"]);
    assert!(!page.read_with(cx, |p, _| p.editing()));
    // Another with the same name: refused.
    cx.update_window(window, |_, w, cx| {
        page.update(cx, |p, cx| p.open(None, w, cx))
    })
    .expect("open");
    set_name(cx, "Work");
    page.update(cx, |p, cx| p.save(cx));
    assert_eq!(
        page.read_with(cx, |p, _| p.error().cloned()),
        Some(SigError::NameTaken)
    );
    // Renamed in its own page.
    let id = page.read_with(cx, |p, _| p.list()[0].id);
    cx.update_window(window, |_, w, cx| {
        page.update(cx, |p, cx| p.open(Some(id), w, cx))
    })
    .expect("open");
    set_name(cx, "Office");
    page.update(cx, |p, cx| p.save(cx));
    assert_eq!(names(cx), ["Office"]);
    // Deleted.
    page.update(cx, |p, cx| p.delete_now(id, cx));
    assert!(names(cx).is_empty());
}

/// Port of `a_logo_over_200_kb_is_refused`: the file is checked before it is
/// read (more than 200 KB is too large), and the page says so; a small
/// picture that is neither PNG nor JPEG is refused by the library's check.
#[gpui_kit::test]
fn a_logo_over_200_kb_is_refused(cx: &mut TestAppContext) {
    use crate::views::settings::signatures::{SigError, read_logo};
    use komorebi::mail::rich::{LogoError, MAX_LOGO_BYTES};
    let (cache, window, page) = signatures("port-logo", cx);
    let big = cache.dir().join("big.png");
    std::fs::write(&big, vec![0u8; MAX_LOGO_BYTES + 1]).expect("write the file");
    let refused = read_logo(&big);
    assert!(matches!(refused, Err(LogoError::TooLarge)));
    let not_a_picture = cache.dir().join("small.png");
    std::fs::write(&not_a_picture, b"not a picture").expect("write the file");
    assert!(read_logo(&not_a_picture).is_err());
    cx.update_window(window, |_, w, cx| {
        page.update(cx, |p, cx| p.open(None, w, cx))
    })
    .expect("open");
    page.update(cx, |p, cx| p.set_logo(refused, cx));
    assert_eq!(
        page.read_with(cx, |p, _| p.error().cloned()),
        Some(SigError::LogoRefused)
    );
}
