//! Phase 5 ports (docs/GPUI-TEST-PORT.md, `src/app/composer/tests/tabs_tests.rs`):
//! message tabs, draft tabs, Ctrl+W and Ctrl+Shift+T, the "moved or
//! deleted" mark, and message windows. All on a throwaway seeded cache.

use gpui::{AnyWindowHandle, AppContext as _, Entity, Role, TestAppContext};
use komorebi::tabs::TabKind;

use super::MainWindow;
use crate::tabs::{CloseTab, OpenInTab, ReopenTab};
use crate::test_support::{self, assert_named, in_window, names};
use crate::views::mail::MailEvent;
use crate::views::mail::triage::Quick;
use crate::views::tabs::message::MessageEvent;

fn open(
    name: &str,
    cx: &mut TestAppContext,
) -> (
    test_support::TempCache,
    komorebi::db::Db,
    Vec<i64>,
    AnyWindowHandle,
    Entity<MainWindow>,
) {
    let (cache, db, seeded) = test_support::seeded(name, 4);
    let ids = db.message_ids(&[seeded.inbox], "").expect("ids");
    test_support::init_app(cx);
    let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
    (cache, db, ids, window, view)
}

fn kinds(view: &Entity<MainWindow>, cx: &mut TestAppContext) -> Vec<TabKind> {
    view.read_with(cx, |v, _| v.tabs().iter().map(|t| t.kind).collect())
}

fn act(
    window: AnyWindowHandle,
    view: &Entity<MainWindow>,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut MainWindow, &mut gpui::Window, &mut gpui::Context<MainWindow>),
) {
    cx.update_window(window, |_, w, cx| view.update(cx, |v, cx| f(v, w, cx)))
        .expect("update");
    cx.run_until_parked();
}

/// Port of `double_click_opens_a_tab_and_reuses_an_existing_one` (the
/// window's half: the list asks with `MailEvent::OpenTab`): a message opens
/// in a tab after the current one, with its own reader and command bar;
/// asking again shows the same tab; the strip names the tabs.
#[gpui_kit::test]
fn a_message_opens_in_a_tab_once(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-open", cx);
    let ask = MailEvent::OpenTab {
        id: ids[0],
        background: false,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
    act(window, &view, cx, |v, w, cx| v.select_tab(0, w, cx));
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    assert_eq!(kinds(&view, cx).len(), 2, "the same tab");
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
    in_window(cx, window, |window, _| {
        assert_named(window, "Reply", Role::Button);
        let tabs = names(window)
            .into_iter()
            .filter(|(role, _)| *role == Some(Role::Tab))
            .count();
        assert_eq!(tabs, 2);
    });
}

/// Port of `middle_click_opens_in_background_without_marking_read`: a
/// background tab does not take the place of the current one, and its
/// message stays unread until shown.
#[gpui_kit::test]
fn a_background_tab_waits_unread(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("tab-background", cx);
    let unseen = ids[1];
    db.set_seen(unseen, false).expect("unread");
    let ask = MailEvent::OpenTab {
        id: unseen,
        background: true,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 0);
    let seen = db.subjects(&[unseen]).expect("subjects")[0].2;
    assert!(!seen, "not read by opening in the background");
}

/// Port of `close_tab_activates_left_neighbour_and_ctrl_shift_t_reopens`:
/// Ctrl+W closes the current tab and shows its left neighbour; Ctrl+Shift+T
/// brings it back; the Mail tab never closes.
#[gpui_kit::test]
fn ctrl_w_closes_and_ctrl_shift_t_reopens(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-close", cx);
    for id in [ids[0], ids[1]] {
        let ask = MailEvent::OpenTab {
            id,
            background: false,
        };
        act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    }
    assert_eq!(
        kinds(&view, cx),
        [
            TabKind::Mail,
            TabKind::Message(ids[0]),
            TabKind::Message(ids[1])
        ]
    );
    in_window(cx, window, |window, cx| {
        test_support::dispatch(window, &CloseTab, cx)
    });
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
    in_window(cx, window, |window, cx| {
        test_support::dispatch(window, &ReopenTab, cx)
    });
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx).last(), Some(&TabKind::Message(ids[1])));
    act(window, &view, cx, |v, w, cx| v.close_tab(0, w, cx));
    assert_eq!(kinds(&view, cx)[0], TabKind::Mail, "the Mail tab stays");
}

/// The list's Undo, as the status line's Undo or Ctrl+Z would run it.
fn undo(window: AnyWindowHandle, view: &Entity<MainWindow>, cx: &mut TestAppContext) {
    act(window, view, cx, |v, w, cx| {
        if let Some(mail) = v.mail().cloned() {
            mail.update(cx, |m, cx| m.act(w, cx, |m| m.undo()));
        }
    });
}

/// Port of `archive_from_tab_closes_it_and_undo_reopens_at_index`: Archive
/// in a message tab is the list's Archive and the tab closes; Undo brings
/// it back at its place, in front.
#[gpui_kit::test]
fn archive_from_a_tab_closes_it(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-archive", cx);
    for (id, background) in [(ids[0], false), (ids[1], true)] {
        let ask = MailEvent::OpenTab { id, background };
        act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    }
    // The tabs: Mail, ids[1], ids[0] (each opens after the current one).
    assert_eq!(
        kinds(&view, cx),
        [
            TabKind::Mail,
            TabKind::Message(ids[0]),
            TabKind::Message(ids[1])
        ]
    );
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &MessageEvent::Archive(ids[0]), w, cx)
    });
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[1])]);
    let listed = view.read_with(cx, |v, cx| {
        v.mail()
            .map(|m| m.read(cx).mail().rows().ids().contains(&ids[0]))
    });
    assert_eq!(listed, Some(false), "archived from the inbox");
    undo(window, &view, cx);
    assert_eq!(
        kinds(&view, cx),
        [
            TabKind::Mail,
            TabKind::Message(ids[0]),
            TabKind::Message(ids[1])
        ],
        "Undo reopens the tab at its place"
    );
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
}

/// Port of `popout_has_own_reader_and_closes_when_message_archived_from_main`
/// (its second half) and the Slint app's `move_out`: archiving from the
/// list closes the message's tab and window; Undo reopens the tab behind
/// the Mail tab and says the window is not reopened.
#[gpui_kit::test]
fn a_move_from_the_list_closes_its_tab_and_window(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-list-move", cx);
    let ask = MailEvent::OpenTab {
        id: ids[2],
        background: true,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[2], w, cx)
    });
    assert_eq!(view.read_with(cx, |v, _| v.popouts()), [ids[2]]);
    act(window, &view, cx, |v, w, cx| {
        if let Some(mail) = v.mail().cloned() {
            mail.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    m.quick_action(ids[2], crate::views::mail::triage::Quick::Archive)
                })
            });
        }
    });
    assert_eq!(kinds(&view, cx), [TabKind::Mail]);
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
    undo(window, &view, cx);
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[2])]);
    // The Slint app's `reopen_message_tabs`: a move from the list leaves
    // the Mail tab in front.
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 0);
    // The window is not reopened; the status line says so.
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
    let status = view.read_with(cx, |v, cx| {
        v.mail().and_then(|m| m.read(cx).mail().status().cloned())
    });
    assert_eq!(
        status,
        Some(crate::views::mail::model::Status::UndoWindow),
        "Undo says to open the message from the list"
    );
}

/// A draft tab is titled by its subject as it is typed (the Slint app's
/// `retitle_draft_tab`).
#[gpui_kit::test]
fn a_draft_tab_follows_its_subject(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-title", cx);
    act(window, &view, cx, |v, w, cx| {
        v.message_action(&MessageEvent::Forward { message: ids[0] }, w, cx)
    });
    let title =
        |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.tabs().active_tab().title.clone());
    let first = title(cx);
    let draft = match kinds(&view, cx)[1] {
        TabKind::Draft(d) => d,
        other => panic!("a draft tab, not {other:?}"),
    };
    act(window, &view, cx, |v, w, cx| {
        let _ = w;
        if let Some(c) = v.draft_composer(draft) {
            c.update(cx, |c, cx| {
                if let Some(d) = c.drafts.active_mut() {
                    d.subject = "Plans for Friday".into();
                }
                cx.notify();
            });
        }
    });
    assert_ne!(first, "Plans for Friday");
    assert_eq!(title(cx), "Plans for Friday");
}

/// A `mailto:` link in a tab's message opens the composer in the Mail tab,
/// filled; Ctrl+N from a message tab shows the Mail tab too (its body
/// would hide the composer).
#[gpui_kit::test]
fn a_tabs_mail_link_and_new_mail_go_to_the_mail_tab(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-mailto", cx);
    let ask = MailEvent::OpenTab {
        id: ids[0],
        background: false,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
    let link = komorebi::mail::render::links::Mailto {
        to: "ana@example.org".into(),
        ..Default::default()
    };
    let compose = MessageEvent::Reader(crate::views::reader::ReaderEvent::Compose(link));
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &compose, w, cx)
    });
    let state = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, cx| {
            let open = v
                .mail()
                .is_some_and(|m| m.read(cx).composer().read(cx).is_open());
            (v.tabs().active(), open)
        })
    };
    assert_eq!(state(cx), (0, true));
    // Back to the message tab, then Ctrl+N.
    act(window, &view, cx, |v, w, cx| v.select_tab(1, w, cx));
    assert_eq!(state(cx).0, 1);
    in_window(cx, window, |window, cx| {
        test_support::press(window, &["ctrl-n"], cx)
    });
    cx.run_until_parked();
    assert_eq!(state(cx), (0, true));
}

/// Port of `server_removal_marks_tab_gone_and_disables_actions` and
/// `removed_message_in_background_tab_shows_warning_on_return`: once the
/// cache no longer has the message, its tab is marked and shows the notice.
#[gpui_kit::test]
fn a_removed_message_marks_its_tab_gone(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("tab-gone", cx);
    for (id, background) in [(ids[0], false), (ids[1], true)] {
        let ask = MailEvent::OpenTab { id, background };
        act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    }
    db.delete_messages(&[ids[0], ids[1]]).expect("removed");
    act(window, &view, cx, |v, w, cx| {
        v.on_mail(&MailEvent::Changed, w, cx)
    });
    let gone: Vec<bool> = view.read_with(cx, |v, _| v.tabs().iter().map(|t| t.gone).collect());
    assert_eq!(gone, [false, true, true]);
    in_window(cx, window, |window, _| {
        assert!(
            names(window)
                .iter()
                .any(|(_, n)| n.contains(&crate::views::tabs::text::gone())),
            "the notice shows"
        );
    });
}

/// Port of `popout_has_own_reader_and_closes_when_message_archived_from_main`
/// and `popout_actions_and_replies_go_to_the_main_window`: a message opens
/// in a window of its own once; its Archive is the main window's and closes
/// it; its Reply opens a draft tab in the main window.
#[gpui_kit::test]
fn a_message_window_acts_on_the_main_window(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("popout", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[0], w, cx)
    });
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[0], w, cx)
    });
    assert_eq!(view.read_with(cx, |v, _| v.popouts()), [ids[0]]);
    act(window, &view, cx, |v, w, cx| {
        v.message_action(
            &MessageEvent::Reply {
                message: ids[0],
                all: false,
            },
            w,
            cx,
        )
    });
    assert!(matches!(
        kinds(&view, cx)[..],
        [TabKind::Mail, TabKind::Draft(_)]
    ));
    view.update(cx, |v, cx| v.close_popouts(cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
}

/// Shift+Enter on the list opens the open message in a tab.
#[gpui_kit::test]
fn shift_enter_opens_the_open_message_in_a_tab(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-key", cx);
    act(window, &view, cx, |v, w, cx| {
        if let Some(mail) = v.mail().cloned() {
            mail.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    m.select_message(ids[2]);
                })
            });
        }
    });
    in_window(cx, window, |window, cx| {
        test_support::dispatch(window, &OpenInTab, cx)
    });
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx).last(), Some(&TabKind::Message(ids[2])));
}

/// The message held in the list's undo window, when `id` is in it.
fn held(view: &Entity<MainWindow>, id: i64, cx: &mut TestAppContext) -> bool {
    view.read_with(cx, |v, cx| {
        v.mail()
            .and_then(|m| m.read(cx).mail().held().map(|h| h.ids.contains(&id)))
            .unwrap_or(false)
    })
}

fn open_tab(window: AnyWindowHandle, view: &Entity<MainWindow>, id: i64, cx: &mut TestAppContext) {
    let ask = MailEvent::OpenTab {
        id,
        background: false,
    };
    act(window, view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
}

/// Report junk from a tab's reader (its info bar, `ReaderEvent::Junk`) is
/// the list's junk report: the message is held for Undo and its tab
/// closes. Without a Junk folder nothing moves and the tab stays (the
/// event was dropped before: stage D shell gap).
#[gpui_kit::test]
fn report_junk_from_a_tab_moves_the_message(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("tab-junk", cx);
    open_tab(window, &view, ids[0], cx);
    let junk = |id| {
        MessageEvent::Reader(crate::views::reader::ReaderEvent::Junk {
            message: id,
            junk: true,
        })
    };
    let ask = junk(ids[0]);
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &ask, w, cx)
    });
    assert!(!held(&view, ids[0], cx), "no Junk folder: stays");
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);
    let account = db.accounts().expect("accounts")[0].id;
    db.upsert_folder(account, "Junk", "Junk", 0, komorebi::role::Role::Junk)
        .expect("a Junk folder");
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &ask, w, cx)
    });
    assert!(held(&view, ids[0], cx), "moved to Junk, held for Undo");
    assert_eq!(kinds(&view, cx), [TabKind::Mail]);
    undo(window, &view, cx);
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);
}

/// The message tab's command bar has the reading pane's commands (stage D
/// shell gap): Move, Mark as read or unread, Mark as important and the
/// More menu are named buttons; Flag and Read toggle the marks (and the
/// bar follows); the Move menu is the reading pane's (translated folder
/// names in tree order, the current folder left out); Move to a folder
/// moves through the list, closes the tab and becomes a recent folder;
/// Forward as attachment opens a draft tab. (The More menu's entries are
/// `more_menu`'s and not opened here.)
#[gpui_kit::test]
fn a_message_tab_has_the_full_command_set(cx: &mut TestAppContext) {
    use crate::views::mail::text::{self as mail_text, Command};
    let (_cache, db, ids, window, view) = open("tab-full-command-set", cx);
    open_tab(window, &view, ids[1], cx);
    let read_button = |seen: bool| {
        mail_text::command(if seen {
            Command::MarkUnread
        } else {
            Command::MarkRead
        })
    };
    let first_seen = db.briefs(&[ids[1]]).expect("briefs")[0].seen;
    in_window(cx, window, |window, _| {
        for name in ["Move", "Mark as important", "More actions"] {
            assert_named(window, name, Role::Button);
        }
        assert_named(window, &read_button(first_seen), Role::Button);
    });
    // The Move entries are the reading pane's (read when the menu opens).
    let targets = |id: i64, cx: &mut TestAppContext| {
        let tab = view
            .read_with(cx, |v, _| v.tab_message_view(id))
            .expect("the tab");
        tab.read_with(cx, |v, _| v.move_options())
    };
    let mail = view.read_with(cx, |v, _| v.mail().cloned()).expect("mail");
    let brief0 = db.briefs(&[ids[1]]).expect("briefs")[0];
    let want = mail.read_with(cx, |m, _| {
        m.mail()
            .message_move_targets(brief0.account_id, brief0.folder_id)
    });
    let got = targets(ids[1], cx);
    assert_eq!(got, want, "the reading pane's Move entries");
    assert!(got.iter().all(|o| o.id != brief0.folder_id && !o.recent));
    let folders = db.folders().expect("folders");
    for o in &got {
        let f = folders.iter().find(|f| f.id == o.id).expect("folder");
        assert_eq!(o.label, mail_text::folder_label(f), "translated names");
    }
    let brief = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        db.briefs(&[ids[1]]).expect("briefs")[0]
    };
    let before = brief(cx);
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[1], &MessageEvent::Flag(ids[1]), w, cx)
    });
    assert_eq!(brief(cx).flagged, !before.flagged);
    let seen = brief(cx).seen;
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[1], &MessageEvent::Read(ids[1]), w, cx)
    });
    assert_eq!(brief(cx).seen, !seen);
    in_window(cx, window, |window, _| {
        assert_named(window, &read_button(!seen), Role::Button)
    });
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[1], &MessageEvent::ForwardAttached(ids[1]), w, cx)
    });
    assert!(matches!(kinds(&view, cx).last(), Some(TabKind::Draft(_))));
    let archive = db
        .folders()
        .expect("folders")
        .into_iter()
        .find(|f| f.role == komorebi::role::Role::Archive)
        .expect("archive")
        .id;
    act(window, &view, cx, |v, w, cx| v.select_tab(1, w, cx));
    let ask = MessageEvent::Move {
        message: ids[1],
        folder: archive,
    };
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[1], &ask, w, cx)
    });
    assert!(held(&view, ids[1], cx));
    assert!(
        !kinds(&view, cx).contains(&TabKind::Message(ids[1])),
        "moved: its tab closed"
    );
    // The folder is a recent one now: first in the next message's menu.
    let brief2 = db.briefs(&[ids[2]]).expect("briefs")[0];
    let next = mail.read_with(cx, |m, _| {
        m.mail()
            .message_move_targets(brief2.account_id, brief2.folder_id)
    });
    assert_eq!(
        next.first().map(|o| (o.id, o.recent)),
        Some((archive, true)),
        "a move from a tab notes the recent folder"
    );
}

// ------------------------------------------- drafts in tabs and windows

use crate::views::composer::{ComposerView, Draft};
use crate::views::tabs::strip::TabCommand;
use komorebi::compose::Field;

/// Writes into a composer's draft, as typing would. A body is written as
/// plain text: the shown rich editor is let go, so the draft's own text is
/// what is saved.
fn write(composer: &Entity<ComposerView>, cx: &mut TestAppContext, fields: &[(Field, &str)]) {
    composer.update(cx, |c, cx| {
        if fields.iter().any(|(f, _)| *f == Field::Body) {
            c.body.update(cx, |b, _| {
                b.take();
            });
        }
        if let Some(d) = c.drafts.active_mut() {
            for (field, text) in fields {
                d.edit(*field, text);
            }
        }
        cx.notify();
    });
    cx.run_until_parked();
}

fn draft_tab(view: &Entity<MainWindow>, cx: &mut TestAppContext) -> Entity<ComposerView> {
    let d = view.read_with(cx, |v, _| match v.tabs().active_kind() {
        TabKind::Draft(d) => d,
        other => panic!("a draft tab, not {other:?}"),
    });
    view.read_with(cx, |v, _| v.draft_composer(d))
        .expect("its composer")
}

fn subject(composer: &Entity<ComposerView>, cx: &mut TestAppContext) -> Option<String> {
    composer.read_with(cx, |c, _| c.drafts.active().map(|d| d.subject.clone()))
}

fn bodies(db: &komorebi::db::Db) -> Vec<String> {
    db.draft_ids("")
        .expect("drafts")
        .into_iter()
        .filter_map(|r| db.draft(r).ok().flatten())
        .map(|d| d.body)
        .collect()
}

/// Ports of `a_composer_window_keeps_its_own_draft_and_close_saves_it` and
/// `drafts_row_and_tab_open_in_a_window_and_quitting_saves_them` (rows 101
/// and 104): a draft tab's Open in new window moves its draft to a window
/// of its own (saved, then opened there); the window's composer keeps its
/// own draft; quitting saves it, and the main window's closing closes it.
#[gpui_kit::test]
fn a_draft_tab_moves_to_a_window_and_quitting_saves_it(cx: &mut TestAppContext) {
    let (_cache, db, _ids, window, view) = open("draft-window", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[(Field::To, "ana@example.org"), (Field::Subject, "Plans")],
    );
    act(window, &view, cx, |v, w, cx| {
        v.tab_command(1, TabCommand::OpenWindow, w, cx)
    });
    assert_eq!(kinds(&view, cx), [TabKind::Mail], "the tab went");
    let n = view.read_with(cx, |v, _| v.draft_window_numbers());
    assert_eq!(n.len(), 1, "a draft window");
    let (_, composer) = view
        .read_with(cx, |v, _| v.draft_window(n[0]))
        .expect("the window");
    assert_eq!(subject(&composer, cx).as_deref(), Some("Plans"));
    write(&composer, cx, &[(Field::Body, "Written in the window")]);
    // Quitting saves it (`on_app_quit`).
    view.update(cx, |v, cx| v.save_drafts(cx));
    assert!(
        bodies(&db)
            .iter()
            .any(|b| b.contains("Written in the window")),
        "saved as the app quits"
    );
    // The main window closes: the draft window goes with it.
    view.update(cx, |v, cx| v.close_popouts(cx));
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.draft_window_numbers())
            .is_empty()
    );
    assert_eq!(db.count_drafts().expect("count"), 1);
}

/// Port of `send_from_a_composer_window_once_and_undo_reopens_it_there`
/// (row 102): Send from a draft window closes it; the status line's Undo
/// opens the draft in a window again, not in a tab.
#[gpui_kit::test]
fn undo_of_a_send_from_a_draft_window_reopens_it_there(cx: &mut TestAppContext) {
    let (_cache, _db, _ids, window, view) = open("draft-window-send", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_window(Draft::New, w, cx)
    });
    let n = view.read_with(cx, |v, _| v.draft_window_numbers())[0];
    let (handle, composer) = view
        .read_with(cx, |v, _| v.draft_window(n))
        .expect("the window");
    write(
        &composer,
        cx,
        &[
            (Field::To, "ana@example.org"),
            (Field::Subject, "From a window"),
            (Field::Body, "from a window"),
        ],
    );
    let c = composer.clone();
    cx.update_window(handle, |_, w, cx| c.update(cx, |c, cx| c.send(true, w, cx)))
        .expect("send");
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.draft_window_numbers())
            .is_empty(),
        "sent: the window closed"
    );
    assert_eq!(kinds(&view, cx), [TabKind::Mail]);
    let hub = view.read_with(cx, |v, _| v.hub().cloned()).expect("hub");
    hub.update(cx, |h, cx| h.press_action(cx));
    cx.run_until_parked();
    let windows = view.read_with(cx, |v, _| v.draft_window_numbers());
    assert_eq!(windows.len(), 1, "Undo opens it in a window again");
    assert_eq!(kinds(&view, cx), [TabKind::Mail], "not in a tab");
    let (_, again) = view
        .read_with(cx, |v, _| v.draft_window(windows[0]))
        .expect("the window");
    assert_eq!(subject(&again, cx).as_deref(), Some("From a window"));
}

/// Port of `undo_send_reopens_in_a_new_tab` (row 305): a send from a
/// draft tab closes the tab; Undo opens the draft in a new tab, in front.
#[gpui_kit::test]
fn undo_send_reopens_in_a_new_tab(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-undo-send", cx);
    open_tab(window, &view, ids[0], cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[
            (Field::To, "ana@example.org"),
            (Field::Subject, "Plans"),
            (Field::Body, "Hello"),
        ],
    );
    let t = tab.clone();
    in_window(cx, window, move |w, cx| {
        t.update(cx, |c, cx| c.send(true, w, cx))
    });
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);
    let hub = view.read_with(cx, |v, _| v.hub().cloned()).expect("hub");
    hub.update(cx, |h, cx| h.press_action(cx));
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx).len(), 3, "a new tab");
    let again = draft_tab(&view, cx);
    assert_eq!(subject(&again, cx).as_deref(), Some("Plans"));
}

/// Port of `tab_switch_flushes_autosave_of_the_leaving_draft` (row 304):
/// leaving a draft tab saves its draft at once.
#[gpui_kit::test]
fn tab_switch_flushes_autosave_of_the_leaving_draft(cx: &mut TestAppContext) {
    let (_cache, db, _ids, window, view) = open("tab-flush", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[
            (Field::To, "ana@example.org"),
            (Field::Body, "Typed just now"),
        ],
    );
    assert_eq!(db.count_drafts().expect("count"), 0, "the autosave waits");
    act(window, &view, cx, |v, w, cx| v.select_tab(0, w, cx));
    assert!(
        bodies(&db).iter().any(|b| b.contains("Typed just now")),
        "saved as the tab was left"
    );
}

/// Port of `closing_a_draft_tab_saves_it_and_an_empty_one_goes` (row 303):
/// Ctrl+W on a draft tab with writing saves it and Ctrl+Shift+T brings it
/// back; an untouched reply's tab closes leaving nothing.
#[gpui_kit::test]
fn closing_a_draft_tab_saves_it_and_an_empty_one_goes(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("tab-draft-close", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[(Field::To, "ana@example.org"), (Field::Subject, "Kept")],
    );
    let i = view.read_with(cx, |v, _| v.tabs().active());
    act(window, &view, cx, |v, w, cx| v.close_tab(i, w, cx));
    assert_eq!(kinds(&view, cx), [TabKind::Mail]);
    assert_eq!(db.count_drafts().expect("count"), 1, "saved");
    act(window, &view, cx, |v, w, cx| v.reopen_tab_now(w, cx));
    let back = draft_tab(&view, cx);
    assert_eq!(subject(&back, cx).as_deref(), Some("Kept"), "back");
    act(window, &view, cx, |v, w, cx| {
        v.message_action(
            &MessageEvent::Reply {
                message: ids[0],
                all: false,
            },
            w,
            cx,
        )
    });
    assert_eq!(kinds(&view, cx).len(), 3);
    let i = view.read_with(cx, |v, _| v.tabs().active());
    act(window, &view, cx, |v, w, cx| v.close_tab(i, w, cx));
    assert_eq!(kinds(&view, cx).len(), 2);
    assert_eq!(
        db.count_drafts().expect("count"),
        1,
        "the untouched reply went"
    );
}

/// The window's half of `selecting_a_message_moves_a_dirty_pane_draft_to_a_tab`
/// (row 302): the pane's set-aside draft (`MailEvent::OpenDraftTab`, in
/// the background) opens in a tab of its own behind the shown one, with
/// the saved draft in its composer.
#[gpui_kit::test]
fn a_set_aside_pane_draft_opens_in_a_background_tab(cx: &mut TestAppContext) {
    let (_cache, db, _ids, window, view) = open("tab-pane-draft", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[(Field::To, "ana@example.org"), (Field::Subject, "Plans")],
    );
    act(window, &view, cx, |v, w, cx| v.close_tab(1, w, cx));
    let row = db.draft_ids("").expect("drafts")[0];
    let ask = MailEvent::OpenDraftTab {
        row,
        background: true,
        sent_by: None,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    let k = kinds(&view, cx);
    assert_eq!(k.len(), 2);
    assert!(matches!(k[1], TabKind::Draft(_)));
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 0, "behind");
    act(window, &view, cx, |v, w, cx| v.select_tab(1, w, cx));
    let back = draft_tab(&view, cx);
    assert_eq!(subject(&back, cx).as_deref(), Some("Plans"));
}

/// Port of `main_close_closes_popouts_then_flushes` (row 308): closing the
/// main window closes the message windows and saves the pane's draft.
#[gpui_kit::test]
fn main_close_closes_popouts_then_flushes(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("main-close", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[1], w, cx)
    });
    let mail = view.read_with(cx, |v, _| v.mail().cloned()).expect("mail");
    let pane = mail.read_with(cx, |m, _| m.composer().clone());
    let m = mail.clone();
    in_window(cx, window, move |w, cx| {
        m.update(cx, |m, cx| m.new_mail(w, cx))
    });
    cx.run_until_parked();
    write(
        &pane,
        cx,
        &[
            (Field::To, "ana@example.org"),
            (Field::Body, "Typed just now"),
        ],
    );
    let v = view.clone();
    let closes = cx
        .update_window(window, move |_, w, cx| {
            v.update(cx, |v, cx| v.should_close(w, cx))
        })
        .expect("close");
    cx.run_until_parked();
    assert!(closes, "no tray in tests: the window closes");
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
    // The pane's draft is saved as the app quits (`on_app_quit`), which
    // the window's close leads to.
    cx.quit();
    assert!(bodies(&db).iter().any(|b| b.contains("Typed just now")));
}

/// Port of `language_and_theme_switch_reach_popouts` (row 309): a message
/// window redraws in the language chosen.
#[gpui_kit::test]
fn a_language_switch_reaches_message_windows(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("popout-lang", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[0], w, cx)
    });
    let popout = view.read_with(cx, |v, _| v.popout_handles())[0];
    in_window(cx, popout, |w, _| assert_named(w, "Reply", Role::Button));
    test_support::set_lang(cx, komorebi::i18n::Lang::PtPt);
    cx.run_until_parked();
    in_window(cx, popout, |w, _| {
        assert_named(w, "Responder", Role::Button)
    });
    test_support::set_lang(cx, komorebi::i18n::Lang::En);
}

/// Port of `delete_in_a_message_tab_never_acts_on_a_draft_row` (row 310):
/// with the list on Drafts, a message tab's Delete acts on its message,
/// never on the draft listed there.
#[gpui_kit::test]
fn delete_in_a_message_tab_never_acts_on_a_draft_row(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("tab-delete", cx);
    let account = db.accounts().expect("accounts")[0].id;
    db.upsert_folder(account, "Trash", "Trash", 0, komorebi::role::Role::Trash)
        .expect("a Trash folder");
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(&tab, cx, &[(Field::To, "ana@example.org")]);
    act(window, &view, cx, |v, w, cx| v.close_tab(1, w, cx));
    assert_eq!(db.count_drafts().expect("count"), 1);
    open_tab(window, &view, ids[0], cx);
    act(window, &view, cx, |v, w, cx| {
        if let Some(mail) = v.mail().cloned() {
            mail.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    m.set_view(crate::views::mail::model::View::Drafts)
                })
            });
        }
    });
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &MessageEvent::Delete(ids[0]), w, cx)
    });
    assert_eq!(db.count_drafts().expect("count"), 1, "the draft stays");
    assert!(held(&view, ids[0], cx), "the tab's message went to Trash");
}

/// Ports of `a_vanished_folder_marks_the_open_message_tab_gone`,
/// `a_vanished_folder_marks_a_background_message_tab_gone`,
/// `a_removed_account_marks_its_message_tabs_gone` and
/// `a_removed_account_marks_a_background_tab_gone` (rows 311 to 314): the
/// cache change marks the tabs gone; the shown one says so and stays.
#[gpui_kit::test]
fn a_vanished_folder_and_a_removed_account_mark_tabs_gone(cx: &mut TestAppContext) {
    let (_cache, mut db, ids, window, view) = open("tab-vanished", cx);
    let account = db.accounts().expect("accounts")[0].id;
    let work = db
        .upsert_folder(account, "Work", "Work", 0, komorebi::role::Role::Other)
        .expect("work");
    db.insert_headers(
        account,
        work,
        &[komorebi::db::NewHeader {
            uid: 9,
            msgid: "w9@example.org".into(),
            subject: "Elsewhere".into(),
            from_addr: "cy@example.org".into(),
            to: "me@example.org".into(),
            date: 1_700_000_009,
            ..Default::default()
        }],
    )
    .expect("headers");
    let elsewhere = db.message_ids(&[work], "").expect("ids")[0];
    open_tab(window, &view, elsewhere, cx);
    let ask = MailEvent::OpenTab {
        id: ids[0],
        background: true,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    // The Work folder vanishes on the server.
    let kept: Vec<i64> = db
        .folders()
        .expect("folders")
        .into_iter()
        .filter(|f| f.id != work)
        .map(|f| f.id)
        .collect();
    db.retain_folders(account, &kept).expect("retain");
    act(window, &view, cx, |v, w, cx| {
        v.on_mail(&MailEvent::Changed, w, cx)
    });
    let gone: Vec<bool> = view.read_with(cx, |v, _| v.tabs().iter().map(|t| t.gone).collect());
    assert_eq!(gone.len(), 3, "the tabs stay");
    let shown = view.read_with(cx, |v, _| v.tabs().active());
    assert!(gone[shown], "the shown tab is marked");
    assert_eq!(
        gone.iter().filter(|g| **g).count(),
        1,
        "the inbox one stays"
    );
    in_window(cx, window, |window, _| {
        assert!(
            names(window)
                .iter()
                .any(|(_, n)| n.contains(&crate::views::tabs::text::gone())),
            "the shown tab says the message is gone"
        );
    });
    // The account is removed: its background tab is marked too.
    db.remove_account(account).expect("removed");
    act(window, &view, cx, |v, w, cx| {
        v.on_mail(&MailEvent::Changed, w, cx)
    });
    let gone: Vec<bool> = view.read_with(cx, |v, _| v.tabs().iter().map(|t| t.gone).collect());
    assert_eq!(gone, [false, true, true]);
}

/// Port of `settings_from_a_message_tab_keep_the_pane_and_f6_stays_visible`
/// (row 315): Settings opened from a message tab show beside the folder
/// pane (the mail view draws them, whatever tab is current), and closing
/// them shows the message tab again.
#[gpui_kit::test]
fn settings_from_a_message_tab_keep_the_pane(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-settings", cx);
    open_tab(window, &view, ids[0], cx);
    let pane_shown = |cx: &mut TestAppContext| {
        in_window(cx, window, |window, _| {
            names(window).iter().any(|(_, n)| n == "Inbox")
        })
    };
    assert!(!pane_shown(cx), "a message tab: no folder pane");
    act(window, &view, cx, |v, w, cx| {
        v.show_place(crate::views::mail::Place::Settings, w, cx)
    });
    assert!(pane_shown(cx), "the folder pane beside Settings");
    act(window, &view, cx, |v, w, cx| v.close_place(w, cx));
    assert!(!pane_shown(cx), "the message tab again");
    assert_eq!(view.read_with(cx, |v, _| v.tabs().active()), 1);
}

/// Port of `tab_switch_restores_scroll_and_view_flags` (row 295): each
/// message tab keeps its own view (its reader, with its scroll place, find
/// bar and image choice) while another tab shows, and the Mail tab keeps
/// its list; switching back shows the same ones, not new ones.
#[gpui_kit::test]
fn tab_switch_keeps_each_tabs_own_view(cx: &mut TestAppContext) {
    let (_cache, _db, ids, window, view) = open("tab-keep", cx);
    open_tab(window, &view, ids[0], cx);
    open_tab(window, &view, ids[1], cx);
    let reader =
        |cx: &mut TestAppContext, id: i64| view.read_with(cx, |v, cx| v.message_reader(id, cx));
    let first = reader(cx, ids[0]).expect("its reader");
    let mail = view.read_with(cx, |v, _| v.mail().map(|m| m.entity_id()));
    for i in [0, 1, 2, 1] {
        act(window, &view, cx, |v, w, cx| v.select_tab(i, w, cx));
    }
    assert_eq!(reader(cx, ids[0]), Some(first), "the same reader");
    assert_eq!(
        view.read_with(cx, |v, _| v.mail().map(|m| m.entity_id())),
        mail
    );
}

/// The snapshot tool's `compose` action (`KOMOREBI_SNAPSHOT_ACTION=compose`,
/// refused before): New mail, the composer in the Mail tab's reading pane.
#[gpui_kit::test]
fn the_compose_snapshot_action_opens_the_composer(cx: &mut TestAppContext) {
    let (_cache, _db, _ids, window, view) = open("snapshot-compose", cx);
    act(window, &view, cx, |v, w, cx| {
        v.snapshot_action("compose", w, cx)
    });
    let composing = view.read_with(cx, |v, cx| {
        v.mail
            .as_ref()
            .is_some_and(|m| m.read(cx).composer().read(cx).composing())
    });
    assert!(composing, "the composer is open");
}

/// The gap lane "mail" (Stage D shell's Not done): a message tab's Move,
/// Report junk, Mark as important, Mark as read and Forward as attachment
/// reach the mail view and the draft tabs. A move closes the tab and Undo
/// brings it back; one that cannot happen (no Junk folder) leaves it open.
#[gpui_kit::test]
fn a_message_tabs_list_commands_reach_the_mail_view(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("tab-list-commands", cx);
    let account = db.accounts().expect("accounts")[0].id;
    let sent = db
        .folder_by_role(account, komorebi::role::Role::Sent)
        .expect("folders")
        .expect("Sent")
        .id;
    let ask = MailEvent::OpenTab {
        id: ids[0],
        background: false,
    };
    act(window, &view, cx, |v, w, cx| v.on_mail(&ask, w, cx));
    let state = || {
        let b = db.briefs(&[ids[0]]).expect("brief")[0];
        (b.seen, b.flagged)
    };
    let (seen, flagged) = state();
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &MessageEvent::Flag(ids[0]), w, cx)
    });
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &MessageEvent::Read(ids[0]), w, cx)
    });
    assert_eq!(state(), (!seen, !flagged), "turned over");
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);

    // No Junk folder: nothing moves and the tab stays.
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &MessageEvent::Junk(ids[0]), w, cx)
    });
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);

    let to_sent = MessageEvent::Move {
        message: ids[0],
        folder: sent,
    };
    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &to_sent, w, cx)
    });
    assert_eq!(kinds(&view, cx), [TabKind::Mail]);
    let held = view.read_with(cx, |v, cx| {
        v.mail()
            .and_then(|m| m.read(cx).mail().held().map(|h| h.ids.contains(&ids[0])))
    });
    assert_eq!(held, Some(true), "held with Undo");
    undo(window, &view, cx);
    assert_eq!(kinds(&view, cx), [TabKind::Mail, TabKind::Message(ids[0])]);

    act(window, &view, cx, |v, w, cx| {
        v.on_message_tab(ids[0], &MessageEvent::ForwardAttached(ids[0]), w, cx)
    });
    assert!(matches!(
        kinds(&view, cx)[..],
        [TabKind::Mail, TabKind::Message(_), TabKind::Draft(_)]
    ));
}

/// A message window's Move is the main window's (held, with Undo), and the
/// window closes with it.
#[gpui_kit::test]
fn a_message_windows_move_closes_it(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("popout-move", cx);
    let account = db.accounts().expect("accounts")[0].id;
    let archive = db
        .folder_by_role(account, komorebi::role::Role::Archive)
        .expect("folders")
        .expect("Archive")
        .id;
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[1], w, cx)
    });
    assert_eq!(view.read_with(cx, |v, _| v.popouts()), [ids[1]]);
    let ask = MessageEvent::Move {
        message: ids[1],
        folder: archive,
    };
    act(window, &view, cx, |v, w, cx| v.message_action(&ask, w, cx));
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
}

/// A message window closes for its own command only when the move is held:
/// Report junk with no Junk folder and a Trash delete waiting for its
/// question leave it open; a Move closes it. Asked from the window itself
/// (`on_popout`), not through the list.
#[gpui_kit::test]
fn a_message_window_closes_only_for_a_held_move(cx: &mut TestAppContext) {
    let (_cache, mut db, ids, window, view) = open("popout-held", cx);
    let account = db.accounts().expect("accounts")[0].id;
    let archive = db
        .folder_by_role(account, komorebi::role::Role::Archive)
        .expect("folders")
        .expect("Archive")
        .id;
    let trash = db
        .upsert_folder(account, "Trash", "Trash", 0, komorebi::role::Role::Trash)
        .expect("a Trash folder");
    db.insert_headers(
        account,
        trash,
        &[komorebi::db::NewHeader {
            uid: 90,
            subject: "binned".into(),
            date: 1_700_000_900,
            ..Default::default()
        }],
    )
    .expect("a message in Trash");
    let binned = db.message_ids(&[trash], "").expect("ids")[0];
    act(window, &view, cx, |v, w, cx| {
        if let Some(mail) = v.mail().cloned() {
            mail.update(cx, |m, cx| m.act(w, cx, |m| m.load_folders()));
        }
    });
    let ask_window = |id: i64, event: MessageEvent, cx: &mut TestAppContext| {
        let popout = view
            .read_with(cx, |v, _| v.popout_view(id))
            .expect("the window");
        popout.update(cx, |v, cx| v.ask(event, cx));
        cx.run_until_parked();
    };
    let popouts = |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.popouts());

    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[1], w, cx)
    });
    ask_window(ids[1], MessageEvent::Junk(ids[1]), cx);
    assert_eq!(popouts(cx), [ids[1]], "no Junk folder: nothing moved");

    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(binned, w, cx)
    });
    ask_window(binned, MessageEvent::Delete(binned), cx);
    assert_eq!(
        popouts(cx),
        [ids[1], binned],
        "the delete waits for its question"
    );

    ask_window(
        ids[1],
        MessageEvent::Move {
            message: ids[1],
            folder: archive,
        },
        cx,
    );
    assert_eq!(popouts(cx), [binned], "held: the window goes");
}

/// A message window's Mark as read follows the list: marked read or
/// important there, the window's bar says so without being asked.
#[gpui_kit::test]
fn a_message_windows_bar_follows_the_lists_marks(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("popout-flags", cx);
    db.set_seen(ids[2], false).expect("unread");
    db.set_flagged(ids[2], false).expect("not important");
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[2], w, cx)
    });
    let bar = |cx: &mut TestAppContext| {
        let popout = view
            .read_with(cx, |v, _| v.popout_view(ids[2]))
            .expect("the window");
        popout.read_with(cx, |v, _| v.bar())
    };
    assert!(!bar(cx).seen && !bar(cx).flagged);
    act(window, &view, cx, |v, w, cx| {
        if let Some(mail) = v.mail().cloned() {
            mail.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    m.quick_action(ids[2], Quick::Read);
                    m.quick_action(ids[2], Quick::Flag);
                })
            });
        }
    });
    assert!(bar(cx).seen && bar(cx).flagged, "the window follows");
}

/// The composer's More menu has Open in new window (Gaps integration): the
/// event it emits moves a draft tab's draft, and the reading pane's, to a
/// composer window of its own. A composer window's own menu does not offer
/// it.
#[gpui_kit::test]
fn the_composers_more_opens_it_in_a_window(cx: &mut TestAppContext) {
    use crate::views::composer::ComposerEvent;
    let (_cache, _db, _ids, window, view) = open("composer-more-window", cx);
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[(Field::To, "ana@example.org"), (Field::Subject, "Plans")],
    );
    tab.update(cx, |_, cx| cx.emit(ComposerEvent::OpenInWindow));
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx), [TabKind::Mail], "the tab went");
    let n = view.read_with(cx, |v, _| v.draft_window_numbers());
    assert_eq!(n.len(), 1, "a composer window");
    let (_, composer) = view
        .read_with(cx, |v, _| v.draft_window(n[0]))
        .expect("the window");
    assert!(
        composer.read_with(cx, |c, _| c.is_windowed()),
        "no Open in new window in a window"
    );
    assert_eq!(subject(&composer, cx).as_deref(), Some("Plans"));

    // The reading pane's draft.
    let mail = view.read_with(cx, |v, _| v.mail().cloned()).expect("mail");
    let m = mail.clone();
    in_window(cx, window, move |w, cx| {
        m.update(cx, |m, cx| m.new_mail(w, cx))
    });
    cx.run_until_parked();
    let pane = mail.read_with(cx, |m, _| m.composer().clone());
    write(
        &pane,
        cx,
        &[(Field::To, "bia@example.org"), (Field::Subject, "Lunch")],
    );
    pane.update(cx, |_, cx| cx.emit(ComposerEvent::OpenInWindow));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.draft_window_numbers()).len(),
        2,
        "the pane's draft has a window too"
    );
    assert!(!pane.read_with(cx, |c, _| c.is_open()), "the pane is free");
}

// ------------------------------------------- closing windows of their own

/// The window's size as the next window will see it (logical px).
fn size_of(handle: AnyWindowHandle, cx: &mut TestAppContext) -> (f32, f32) {
    cx.update_window(handle, |_, w, _| crate::views::windows::popout::size_of(w))
        .expect("the window")
}

fn resize(handle: AnyWindowHandle, to: (f32, f32), cx: &mut TestAppContext) {
    cx.simulate_window_resize(handle, gpui::size(gpui::px(to.0), gpui::px(to.1)));
}

/// The title bar's Close button, Alt+F4 or the taskbar's Close: the call
/// the platform makes on the window's `on_window_should_close` handler.
/// The test platform sends no OS close by itself, so this is the same
/// handler (`gpui::VisualTestContext::simulate_close`). True when the
/// platform would close the window now.
fn os_close(handle: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    let closed = gpui::VisualTestContext::from_window(handle, cx).simulate_close();
    cx.run_until_parked();
    closed
}

fn draft_window_ready(
    window: AnyWindowHandle,
    view: &Entity<MainWindow>,
    cx: &mut TestAppContext,
) -> (u64, AnyWindowHandle, Entity<ComposerView>) {
    let before = view.read_with(cx, |v, _| v.draft_window_numbers());
    act(window, view, cx, |v, w, cx| {
        v.open_draft_window(Draft::New, w, cx)
    });
    let n = *view
        .read_with(cx, |v, _| v.draft_window_numbers())
        .iter()
        .find(|n| !before.contains(n))
        .expect("a new draft window");
    let (handle, composer) = view
        .read_with(cx, |v, _| v.draft_window(n))
        .expect("the window");
    (n, handle, composer)
}

/// The window's own close path (the handler the platform calls for the
/// title bar's Close and Alt+F4): a draft worth keeping is parked
/// (`park_composer`: saved here and on the server), the window goes, and
/// the size it had is the next window's. A draft that could not be saved
/// keeps its window and its text, and the size stays what it was; the next
/// close, once it saves, goes through.
#[gpui_kit::test]
fn the_os_close_of_a_composer_window_parks_the_draft_and_keeps_its_size(cx: &mut TestAppContext) {
    let (_cache, db, _ids, window, view) = open("draft-window-os-close", cx);
    let (_, first, composer) = draft_window_ready(window, &view, cx);
    write(
        &composer,
        cx,
        &[
            (Field::To, "ana@example.org"),
            (Field::Subject, "Closed by the OS"),
            (Field::Body, "Parked on close"),
        ],
    );
    assert_eq!(view.read_with(cx, |v, _| v.popout_size()), None, "none yet");
    resize(first, (1010., 650.), cx);
    // The handler leaves the closing to the composer (it says Closed once
    // the draft is parked), so the platform is told not to close it now.
    assert!(!os_close(first, cx), "the composer closes the window");
    assert!(
        view.read_with(cx, |v, _| v.draft_window_numbers())
            .is_empty(),
        "the window went"
    );
    assert!(
        bodies(&db).iter().any(|b| b.contains("Parked on close")),
        "the draft was saved: {:?}",
        bodies(&db)
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.popout_size()),
        Some((1010., 650.)),
        "the size it had"
    );

    // The next window opens at that size.
    let (_, second, again) = draft_window_ready(window, &view, cx);
    assert_eq!(size_of(second, cx), (1010., 650.));

    // A save that fails keeps the window, the text and the old size.
    write(
        &again,
        cx,
        &[
            (Field::To, "bia@example.org"),
            (Field::Body, "Not saved yet"),
        ],
    );
    again.update(cx, |c, _| {
        c.drafts.active_mut().expect("a draft").account_id = i64::MAX;
    });
    resize(second, (900., 600.), cx);
    assert!(!os_close(second, cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.draft_window_numbers()).len(),
        1,
        "the window stays"
    );
    assert!(again.read_with(cx, |c, _| c.composing()), "with its draft");
    assert!(matches!(
        again.read_with(cx, |c, _| c.status().cloned()),
        Some(crate::views::composer::Status::CouldNotSave(_))
    ));
    assert_eq!(
        view.read_with(cx, |v, _| v.popout_size()),
        Some((1010., 650.)),
        "a window that stays does not change the size"
    );
    let account = db.accounts().expect("accounts")[0].id;
    again.update(cx, |c, _| {
        c.drafts.active_mut().expect("a draft").account_id = account;
    });
    assert!(!os_close(second, cx));
    assert!(
        view.read_with(cx, |v, _| v.draft_window_numbers())
            .is_empty()
    );
    assert!(bodies(&db).iter().any(|b| b.contains("Not saved yet")));
    assert_eq!(
        view.read_with(cx, |v, _| v.popout_size()),
        Some((900., 600.))
    );
}

/// A message window's own close button keeps its size and forgets the
/// window, so it opens again; the app closing it (a held Move) keeps the
/// size the same way, and so does Esc or Close in a composer window.
#[gpui_kit::test]
fn closes_by_the_os_and_by_the_app_keep_the_size(cx: &mut TestAppContext) {
    let (_cache, db, ids, window, view) = open("popout-close-size", cx);
    let size = |cx: &mut TestAppContext| view.read_with(cx, |v, _| v.popout_size());

    // The message window's close button: the platform closes it (true).
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[0], w, cx)
    });
    let handle = view.read_with(cx, |v, _| v.popout_handles())[0];
    resize(handle, (760., 560.), cx);
    assert!(os_close(handle, cx), "a message window closes at once");
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
    assert_eq!(size(cx), Some((760., 560.)));
    act(window, &view, cx, |v, w, cx| {
        v.open_in_window(ids[0], w, cx)
    });
    let reopened = view.read_with(cx, |v, _| v.popout_handles())[0];
    assert_eq!(size_of(reopened, cx), (760., 560.), "opens at that size");

    // The app closes it: a Move held for Undo.
    let account = db.accounts().expect("accounts")[0].id;
    let archive = db
        .folder_by_role(account, komorebi::role::Role::Archive)
        .expect("folders")
        .expect("Archive")
        .id;
    resize(reopened, (720., 540.), cx);
    let ask = MessageEvent::Move {
        message: ids[0],
        folder: archive,
    };
    act(window, &view, cx, |v, w, cx| v.message_action(&ask, w, cx));
    assert!(view.read_with(cx, |v, _| v.popouts()).is_empty());
    assert_eq!(size(cx), Some((720., 540.)), "kept by the app's close");

    // Esc (Close) in a composer window.
    let (_, win, composer) = draft_window_ready(window, &view, cx);
    write(&composer, cx, &[(Field::To, "ana@example.org")]);
    resize(win, (680., 520.), cx);
    cx.update_window(win, |_, w, cx| composer.update(cx, |c, cx| c.close(w, cx)))
        .expect("close");
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.draft_window_numbers())
            .is_empty()
    );
    assert_eq!(size(cx), Some((680., 520.)), "kept by Close");
}

// ------------------------------------------------- Undo of a send

/// Undo of a send from a draft tab while the reading pane holds a draft of
/// its own that cannot be saved: the undone message opens in a tab, and the
/// pane's draft stays shown with its text. The undone message once opened
/// in the pane, parked the pane's draft (the save failed, "Could not save
/// the draft") and then left the pane closed, so that text was out of
/// sight with no tab and no saved copy.
#[gpui_kit::test]
fn undo_of_a_send_leaves_a_pane_draft_that_could_not_be_saved_shown(cx: &mut TestAppContext) {
    let (_cache, db, _ids, window, view) = open("tab-undo-send-unsaved", cx);
    let pane = view.read_with(cx, |v, cx| {
        v.mail().expect("the mail view").read(cx).composer().clone()
    });
    // A message sent from a draft tab, waiting in its undo window.
    act(window, &view, cx, |v, w, cx| {
        v.open_draft_tab(Draft::New, w, cx)
    });
    let tab = draft_tab(&view, cx);
    write(
        &tab,
        cx,
        &[
            (Field::To, "ana@example.org"),
            (Field::Subject, "Plans"),
            (Field::Body, "Hello"),
        ],
    );
    let t = tab.clone();
    in_window(cx, window, move |w, cx| {
        t.update(cx, |c, cx| c.send(true, w, cx))
    });
    cx.run_until_parked();
    assert_eq!(kinds(&view, cx), [TabKind::Mail]);
    // The reading pane has a draft of its own, written since, which the
    // cache would refuse (an account that is not in it).
    let p = pane.clone();
    in_window(cx, window, move |w, cx| {
        p.update(cx, |c, cx| c.open(Some(Draft::New), w, cx))
    });
    write(
        &pane,
        cx,
        &[
            (Field::To, "bia@example.org"),
            (Field::Body, "Only in the pane"),
        ],
    );
    pane.update(cx, |c, _| {
        c.drafts.active_mut().expect("a draft").account_id = i64::MAX;
    });
    let drafts_before = db.count_drafts().expect("count");

    let hub = view.read_with(cx, |v, _| v.hub().cloned()).expect("hub");
    hub.update(cx, |h, cx| h.press_action(cx));
    cx.run_until_parked();

    // The undone message is back, in a tab.
    assert_eq!(kinds(&view, cx).len(), 2, "a tab for the undone message");
    let again = draft_tab(&view, cx);
    assert_eq!(subject(&again, cx).as_deref(), Some("Plans"));
    assert_eq!(db.count_drafts().expect("count"), drafts_before + 1);
    // And the pane still shows its draft, text and all.
    assert!(pane.read_with(cx, |c, _| c.composing()), "still shown");
    assert_eq!(
        pane.read_with(cx, |c, _| c.drafts.active().map(|d| d.to.clone())),
        Some("bia@example.org".to_string())
    );
}

// ------------------------------------------- scroll and view flags per tab

const BODY: &str = "From: Marta Silva <marta.silva@example.pt>\r\nTo: Ines <ines@example.com>\r\n\
    Subject: A long one\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
    <p>Hi Ines,</p><p>Here they are.</p>";

/// Port of `tab_switch_restores_scroll_and_view_flags` (row 295): each
/// message tab keeps its page's scroll place and its view flags (the plain
/// text view, the remote images choice) while other tabs show; switching
/// back shows them as they were, and another tab's are its own. A tab owns
/// its reader, so nothing is saved and restored; this proves the readers
/// are not reset, redrawn at the top or asked to lay out again by a
/// switch. (The reader has no find bar in 1.0, so there is none to keep.)
#[gpui_kit::test]
fn tab_switch_restores_scroll_and_view_flags(cx: &mut TestAppContext) {
    use crate::views::reader::testing;
    use gpui_kit::test::TestWindowExt as _;
    let (_cache, mut db, ids, window, view) = open("tab-view-flags", cx);
    for id in [ids[0], ids[1]] {
        db.set_body(id, BODY.as_bytes(), "", false, Default::default())
            .expect("a cached body");
    }
    // Two tabs, each with a fake render worker the test plays.
    let first = testing::inject();
    open_tab(window, &view, ids[0], cx);
    let second = testing::inject();
    open_tab(window, &view, ids[1], cx);
    let reader = |cx: &mut TestAppContext, id: i64| {
        view.read_with(cx, |v, cx| {
            v.tab_message_view(id)
                .expect("its view")
                .read(cx)
                .reader()
                .clone()
        })
    };
    let (a, b) = (reader(cx, ids[0]), reader(cx, ids[1]));
    let show = |cx: &mut TestAppContext, tab: usize| {
        act(window, &view, cx, |v, w, cx| v.select_tab(tab, w, cx));
        in_window(cx, window, |_, _| {});
        cx.run_until_parked();
    };
    let scroll = |cx: &mut TestAppContext, by: f32| {
        in_window(cx, window, move |w, cx| {
            w.scroll(
                "reader-scroll",
                gpui::ScrollDelta::Pixels(gpui::point(gpui::Pixels::ZERO, gpui::px(-by))),
                cx,
            )
        });
        cx.run_until_parked();
        in_window(cx, window, |_, _| {});
    };
    // Answers the layout a tab asked for when it first showed.
    let answer = |cx: &mut TestAppContext, w: &testing::Worker, remote_images: u32| {
        let layouts = w.layouts();
        let (generation, key) = *layouts.last().expect("a layout job");
        w.page(generation, key, 6000., remote_images);
        cx.run_until_parked();
        in_window(cx, window, |_, _| {});
        cx.run_until_parked();
    };

    // Tab 2 (the message `ids[1]`) is shown: a page with blocked images.
    answer(cx, &second, 2);
    scroll(cx, 1200.);
    let b_top = b.read_with(cx, |r, _| r.scrolled());
    assert!(b_top > 1000., "scrolled: {b_top}");

    // Tab 1: scrolled further, in the plain text view, images allowed.
    show(cx, 1);
    answer(cx, &first, 2);
    in_window(cx, window, |w, cx| w.click("reader-load-images", cx));
    cx.run_until_parked();
    let (generation, key) = *first.layouts().last().expect("laid out with consent");
    assert!(key.remote);
    first.page(generation, key, 6000., 0);
    cx.run_until_parked();
    in_window(cx, window, |_, _| {});
    scroll(cx, 2400.);
    let a_top = a.read_with(cx, |r, _| r.scrolled());
    assert!(a_top > 2000., "scrolled: {a_top}");
    act(window, &view, cx, |_, w, cx| {
        a.update(cx, |r, cx| r.set_plain(true, w, cx))
    });
    let flags = |cx: &mut TestAppContext, r: &Entity<crate::views::reader::ReaderView>| {
        r.read_with(cx, |r, _| {
            (r.scrolled(), r.session().plain(), r.session().remote())
        })
    };
    let a_before = flags(cx, &a);
    assert!(
        a_before.1 && a_before.2,
        "plain and with images: {a_before:?}"
    );

    // Away to the Mail tab and the other message, and back.
    for tab in [0, 2, 0, 1] {
        show(cx, tab);
    }
    let a_after = flags(cx, &a);
    assert_eq!(a_after.1, a_before.1, "the plain text view is kept");
    assert_eq!(a_after.2, a_before.2, "the images choice is kept");
    assert!(
        (a_after.0 - a_before.0).abs() < 1.0,
        "the scroll place is kept: {a_before:?} then {a_after:?}"
    );
    assert!(
        first.layouts().is_empty(),
        "switching asked for no new layout"
    );

    // The other tab's are its own.
    show(cx, 2);
    let b_after = flags(cx, &b);
    assert!(!b_after.1 && !b_after.2, "defaults: {b_after:?}");
    assert!(
        (b_after.0 - b_top).abs() < 1.0,
        "its own scroll place: {b_top} then {}",
        b_after.0
    );
    assert!(second.layouts().is_empty());
}
