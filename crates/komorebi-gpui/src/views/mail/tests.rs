//! The mail view in the main window (docs/GPUI-TESTING.md: window tests on
//! a throwaway cache, keys pressed as on Windows, state asserted on the
//! model, names and roles as UI Automation reads them). Each test names
//! the Slint test it ports (`src/app/composer.rs` unless said otherwise;
//! docs/GPUI-TEST-PORT.md).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, Entity, Role, TestAppContext};
use komorebi::db::{Db, Filter, NewHeader};
use komorebi::mail::Job;
use komorebi::role::Role as FolderRole;
use komorebi::tree::Fold;

use super::MailView;
use super::model::{Mail, View};
use super::nav::NavKind;
use super::triage::{Flag, Quick, UNDO_RESUME, UNDO_WINDOW};
use crate::test_support::{self, TempCache, assert_named, in_window, press};
use crate::views::composer::Draft;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;

/// A throwaway cache with the seed account (Inbox, Sent, Archive), the
/// folders asked for, and six unread messages in the Inbox (the Slint
/// harness's one message and `more_messages`' five): their ids, newest
/// first. Opened messages are marked read at once (the Slint harness's
/// setting).
struct Fixture {
    cache: TempCache,
    account: i64,
    inbox: i64,
    ids: Vec<i64>,
}

fn fixture(name: &str, folders: &[(&str, FolderRole)]) -> (Fixture, Db) {
    let (cache, mut db, seeded) = test_support::seeded(name, 0);
    let headers: Vec<NewHeader> = (1..=6)
        .map(|uid| NewHeader {
            uid,
            msgid: format!("<m{uid}@example.org>"),
            subject: format!("m{uid}"),
            from_name: format!("Sender {uid}"),
            from_addr: format!("s{uid}@example.org"),
            date: 1_700_000_000 + i64::from(uid),
            ..Default::default()
        })
        .collect();
    db.insert_headers(seeded.account, seeded.inbox, &headers)
        .expect("insert the messages");
    for &(name, role) in folders {
        db.upsert_folder(seeded.account, name, name, 0, role)
            .expect("add a folder");
    }
    db.set_setting("mark_read_delay_ms", "0")
        .expect("read at once");
    let ids = db.message_ids(&[seeded.inbox], "").expect("ids");
    // Downloaded bodies: Reply and Forward start a draft only from one.
    for &id in &ids {
        let raw = b"From:s@example.org\r\nTo:seed@komorebi.invalid\r\nSubject:m\r\n\
            Content-Type:text/plain;charset=utf-8\r\n\r\nHello\r\n";
        db.set_body(id, raw, "Hello", false, Default::default())
            .expect("a body");
    }
    (
        Fixture {
            cache,
            account: seeded.account,
            inbox: seeded.inbox,
            ids,
        },
        db,
    )
}

/// The main window on the fixture's cache, and its mail view.
fn open(cx: &mut TestAppContext, db: Db) -> (AnyWindowHandle, Entity<MailView>) {
    test_support::init_app(cx);
    let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
    let mail = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
    (window, mail)
}

fn read<R>(cx: &mut TestAppContext, view: &Entity<MailView>, f: impl FnOnce(&Mail) -> R) -> R {
    view.read_with(cx, |v, _| f(v.mail()))
}

/// Runs `f` on the model as an action would, in the window.
fn act(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    view: &Entity<MailView>,
    f: impl FnOnce(&mut Mail),
) {
    in_window(cx, window, |w, cx| view.update(cx, |v, cx| v.act(w, cx, f)));
}

fn keys(cx: &mut TestAppContext, window: AnyWindowHandle, keys: &[&str]) {
    in_window(cx, window, |w, cx| press(w, keys, cx));
}

/// The list holds the keyboard focus.
fn focus_list(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<MailView>) {
    in_window(cx, window, |w, cx| {
        let focus = view.read(cx).list_focus().clone();
        w.focus(&focus, cx);
    });
}

fn open_id(cx: &mut TestAppContext, view: &Entity<MailView>) -> Option<i64> {
    read(cx, view, Mail::open_id)
}

fn selected(cx: &mut TestAppContext, view: &Entity<MailView>) -> usize {
    read(cx, view, |m| m.selection().len())
}

fn held_ids(cx: &mut TestAppContext, view: &Entity<MailView>) -> Vec<i64> {
    read(cx, view, |m| {
        let mut ids: Vec<i64> = m
            .held()
            .iter()
            .flat_map(|h| h.ids.iter().copied())
            .collect();
        ids.sort_unstable();
        ids
    })
}

fn undo_text(cx: &mut TestAppContext, view: &Entity<MailView>) -> String {
    read(cx, view, |m| {
        m.removal()
            .map(|r| super::text::removal(&r, r.to.and_then(|id| m.folder(id))))
            .unwrap_or_default()
    })
}

fn seen(db: &Db, id: i64) -> bool {
    db.message(id).unwrap().unwrap().seen
}

/// Port of `multi_select_acts_on_every_selected_message`.
#[gpui_kit::test]
fn multi_select_acts_on_every_selected_message(cx: &mut TestAppContext) {
    let (f, db) = fixture("multi", &[("Trash", FolderRole::Trash)]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    assert_eq!(read(cx, &view, |m| m.rows().len()), 6);

    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    // Ctrl+click adds a row without opening it (or marking it read).
    act(cx, window, &view, |m| m.row_toggled(ids[2]));
    assert_eq!(open_id(cx, &view), None);
    assert_eq!(selected(cx, &view), 2);
    assert!(!seen(&check, ids[2]));
    // Shift+click and Shift+Down select from the row Ctrl+clicked.
    act(cx, window, &view, |m| m.row_extended(ids[4]));
    assert_eq!(selected(cx, &view), 3);
    focus_list(cx, window, &view);
    keys(cx, window, &["shift-down"]);
    assert_eq!(selected(cx, &view), 4);
    assert_eq!(read(cx, &view, Mail::current_index), Some(5));
    // Ctrl+A selects the whole view; the commands act on all of it.
    keys(cx, window, &["ctrl-a"]);
    assert_eq!(selected(cx, &view), 6);
    act(cx, window, &view, |m| m.mark(Flag::Seen, false));
    act(cx, window, &view, |m| m.mark(Flag::Flagged, true));
    let briefs = check.briefs(&ids).unwrap();
    assert!(briefs.iter().all(|m| !m.seen && m.flagged));
    // Ctrl+Q: all read.
    keys(cx, window, &["ctrl-q"]);
    assert!(check.briefs(&ids).unwrap().iter().all(|m| m.seen));
    // Esc clears.
    keys(cx, window, &["escape"]);
    assert_eq!(selected(cx, &view), 0);
    assert_eq!(open_id(cx, &view), None);

    // Delete moves the selection to Trash and opens the next message.
    act(cx, window, &view, |m| m.row_clicked(ids[1]));
    act(cx, window, &view, |m| m.row_extended(ids[3]));
    assert_eq!(selected(cx, &view), 3);
    keys(cx, window, &["delete"]);
    assert_eq!(open_id(cx, &view), Some(ids[4]));
    assert_eq!(selected(cx, &view), 1);
    assert_eq!(read(cx, &view, |m| m.rows().len()), 3);
    // Held for Undo: out of the list, still in the cache until the job goes.
    assert!(check.message(ids[2]).unwrap().is_some());
    act(cx, window, &view, Mail::flush_held);
    assert!(check.message(ids[2]).unwrap().is_none());
    // A plain Down moves on from there.
    keys(cx, window, &["down"]);
    assert_eq!(open_id(cx, &view), Some(ids[5]));
}

/// Port of `archive_and_delete_wait_for_their_undo_window`.
#[gpui_kit::test]
fn archive_and_delete_wait_for_their_undo_window(cx: &mut TestAppContext) {
    let (f, db) = fixture("held", &[("Trash", FolderRole::Trash)]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let inbox = f.inbox;
    let (window, view) = open(cx, db);
    let unread = |cx: &mut TestAppContext| {
        read(cx, &view, |m| {
            m.nav()
                .items
                .iter()
                .find(|r| r.kind == NavKind::Folder(inbox))
                .map_or(-1, |r| r.unread)
        })
    };
    let listed = |cx: &mut TestAppContext, id| read(cx, &view, |m| m.rows().index_of(id).is_some());
    assert_eq!(unread(cx), 6);

    // Archive the open message: it leaves the list (and the count), the
    // next one opens, Undo is offered; nothing changed in the cache.
    act(cx, window, &view, |m| m.row_clicked(ids[1]));
    assert_eq!(unread(cx), 5);
    act(cx, window, &view, Mail::archive);
    assert!(!listed(cx, ids[1]));
    assert_eq!(open_id(cx, &view), Some(ids[2]));
    assert_eq!(undo_text(cx, &view), "Moved to Archive");
    assert_eq!(
        read(cx, &view, |m| m.status().cloned()),
        None,
        "the Undo offer has its own slot"
    );
    assert!(check.message(ids[1]).unwrap().is_some());
    assert_eq!(held_ids(cx, &view), [ids[1]]);
    // The job waits: one Move to Archive, held.
    let archive = check
        .folder_by_role(f.account, FolderRole::Archive)
        .unwrap()
        .unwrap()
        .id;
    read(cx, &view, |m| {
        let held = m.held().unwrap();
        assert!(matches!(
            held.jobs[..],
            [(_, Job::Move { folder_id, ref uids, dest })]
                if folder_id == inbox && uids[..] == [5] && dest == archive
        ));
    });
    assert_eq!(unread(cx), 4);
    // A reload (a sync) doesn't bring it back meanwhile.
    act(cx, window, &view, Mail::reload_rows);
    assert!(!listed(cx, ids[1]));

    // Undo: back in the list, open again; no job is left.
    act(cx, window, &view, Mail::undo);
    assert!(listed(cx, ids[1]));
    assert_eq!(open_id(cx, &view), Some(ids[1]));
    assert!(read(cx, &view, |m| m.held().is_none()));
    assert_eq!(undo_text(cx, &view), "");

    // A selection of three, deleted: one label, one held batch.
    act(cx, window, &view, |m| m.row_extended(ids[3]));
    act(cx, window, &view, Mail::delete);
    assert_eq!(undo_text(cx, &view), "3 messages moved to Trash");
    let mut three = vec![ids[1], ids[2], ids[3]];
    three.sort_unstable();
    assert_eq!(held_ids(cx, &view), three);
    // Ctrl+Z in the list undoes too: the three come back selected, none
    // opened.
    focus_list(cx, window, &view);
    keys(cx, window, &["ctrl-z"]);
    assert!(held_ids(cx, &view).is_empty());
    assert_eq!(selected(cx, &view), 3);
    assert_eq!(open_id(cx, &view), None);
    assert!(!seen(&check, ids[3]));

    // A second move sends the first one: only the last can be undone.
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    act(cx, window, &view, Mail::archive);
    act(cx, window, &view, Mail::delete);
    assert!(
        check.message(ids[0]).unwrap().is_none(),
        "the archive went to the server"
    );
    assert_eq!(held_ids(cx, &view), [ids[1]]);
    // The undo window ends: the job goes and the offer with it.
    act(cx, window, &view, Mail::flush_held);
    assert!(check.message(ids[1]).unwrap().is_none());
    assert_eq!(undo_text(cx, &view), "");
    // Undo after that does nothing.
    act(cx, window, &view, Mail::undo);
    assert!(!listed(cx, ids[1]));
}

/// Port of `junk_moves_only_to_a_junk_folder`.
#[gpui_kit::test]
fn junk_moves_only_to_a_junk_folder(cx: &mut TestAppContext) {
    let (f, db) = fixture("junk", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    // No Junk folder: a clear line, and the message stays put.
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    act(cx, window, &view, Mail::report_junk);
    let line = read(cx, &view, |m| m.status().map(super::text::status));
    assert_eq!(
        line.as_deref(),
        Some(
            "This account has no Junk folder, so nothing was moved. You can choose one in Settings: Accounts, Edit."
        )
    );
    assert!(read(cx, &view, |m| m.held().is_none()
        && m.rows().index_of(ids[0]).is_some()));

    // With one: a held move there.
    let junk = check
        .upsert_folder(f.account, "Spam", "Spam", 0, FolderRole::Junk)
        .unwrap();
    act(cx, window, &view, Mail::load_folders);
    act(cx, window, &view, Mail::report_junk);
    assert_eq!(undo_text(cx, &view), "Moved to Spam");
    read(cx, &view, |m| {
        assert!(matches!(
            m.held().unwrap().jobs[..],
            [(_, Job::Move { dest, .. })] if dest == junk
        ));
    });
    act(cx, window, &view, Mail::flush_held);

    // In Junk: a right press on an unread message selects it without
    // opening it; the menu offers Not junk and Mark as read.
    let mut writer = f.cache.open();
    writer
        .insert_headers(
            f.account,
            junk,
            &[NewHeader {
                uid: 9,
                subject: "Prize".into(),
                date: 1_700_000_100,
                ..Default::default()
            }],
        )
        .unwrap();
    act(cx, window, &view, |m| m.set_view(View::Folder(junk)));
    let spam = read(cx, &view, |m| m.rows().id_at(0)).unwrap();
    act(cx, window, &view, |m| {
        m.row_context(spam);
    });
    let menu = view.read_with(cx, |v, _| super::menus::row_entries(v));
    let labels: Vec<String> = menu.iter().flatten().map(|e| e.label.clone()).collect();
    assert!(
        labels.contains(&"Not junk".to_string()) && labels.contains(&"Mark as read".to_string())
    );
    assert_eq!(open_id(cx, &view), None);
    assert_eq!(selected(cx, &view), 1);
    assert!(!seen(&check, spam));
    // Not junk: read now, moved to the Inbox after the undo window.
    act(cx, window, &view, Mail::not_junk);
    assert_eq!(undo_text(cx, &view), "Moved to Inbox");
    assert!(seen(&check, spam));
    let inbox = f.inbox;
    read(cx, &view, |m| {
        assert!(matches!(
            m.held().unwrap().jobs[..],
            [(_, Job::Move { folder_id, dest, .. })] if folder_id == junk && dest == inbox
        ));
    });
}

/// Port of `right_click_selects_the_row_unless_it_is_selected` (without
/// its composer half, phase 4).
#[gpui_kit::test]
fn right_click_selects_the_row_unless_it_is_selected(cx: &mut TestAppContext) {
    let (f, db) = fixture("context", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let menu = |cx: &mut TestAppContext| -> Vec<String> {
        view.read_with(cx, |v, _| super::menus::row_entries(v))
            .into_iter()
            .flatten()
            .map(|e| e.label)
            .collect()
    };
    // On the open message: it stays open and the menu acts on it.
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    act(cx, window, &view, |m| {
        m.row_context(ids[0]);
    });
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    assert!(menu(cx).contains(&"Mark as unread".to_string()));
    // On another row: that row alone, not opened, not read.
    act(cx, window, &view, |m| {
        m.row_context(ids[2]);
    });
    assert_eq!(open_id(cx, &view), None);
    assert_eq!(selected(cx, &view), 1);
    let entries = menu(cx);
    for want in ["Mark as read", "Mark as important", "Report junk"] {
        assert!(entries.contains(&want.to_string()), "{want} in {entries:?}");
    }
    assert!(!seen(&check, ids[2]));
    // On a row of a larger selection: the selection stays.
    act(cx, window, &view, |m| m.row_extended(ids[4]));
    act(cx, window, &view, |m| {
        m.row_context(ids[3]);
    });
    assert_eq!(selected(cx, &view), 3);
    // Reply needs a single message: off with three.
    let reply = view.read_with(cx, |v, _| {
        super::menus::row_entries(v)
            .into_iter()
            .flatten()
            .find(|e| e.label == "Reply")
            .map(|e| e.enabled)
    });
    assert_eq!(reply, Some(false));
    // Reply from the menu opens the one selected row, then replies.
    act(cx, window, &view, |m| {
        m.row_context(ids[5]);
    });
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.respond(
                |message| Draft::Reply {
                    message,
                    all: false,
                },
                w,
                cx,
            )
        })
    });
    assert_eq!(open_id(cx, &view), Some(ids[5]));
    let draft = view.read_with(cx, |v, cx| v.composer().read(cx).draft());
    assert_eq!(
        draft,
        Some(Draft::Reply {
            message: ids[5],
            all: false
        })
    );
}

/// Port of `list_keys_work_from_the_start_and_wait_while_composing`
/// (without its search half: `search_keys_and_the_backspace_scope`).
#[gpui_kit::test]
fn list_keys_work_from_the_start_and_wait_while_composing(cx: &mut TestAppContext) {
    let (f, db) = fixture("list-keys", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    // No click first: the list has the keys from the start.
    keys(cx, window, &["down"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    keys(cx, window, &["end"]);
    assert_eq!(open_id(cx, &view), Some(ids[5]));
    keys(cx, window, &["home"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    // Page Down and Page Up move by the rows in view (all six fit here).
    keys(cx, window, &["pagedown"]);
    assert_eq!(open_id(cx, &view), Some(ids[5]));
    keys(cx, window, &["pageup"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    // Esc closes the message; the arrows still work after it.
    keys(cx, window, &["escape"]);
    assert_eq!(open_id(cx, &view), None);
    assert_eq!(selected(cx, &view), 0);
    keys(cx, window, &["down"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    // Two rows selected: Esc clears them.
    act(cx, window, &view, |m| m.row_toggled(ids[2]));
    assert_eq!(selected(cx, &view), 2);
    keys(cx, window, &["escape"]);
    assert_eq!(selected(cx, &view), 0);
    // Enter opens the current row (none: the first).
    keys(cx, window, &["enter"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    // Shift+Down from there, then Enter opens the cursor row.
    keys(cx, window, &["shift-down"]);
    assert_eq!(open_id(cx, &view), None);
    keys(cx, window, &["enter"]);
    assert_eq!(open_id(cx, &view), Some(ids[1]));
    // Ctrl+U marks unread and Ctrl+Q read (neither toggles), Insert
    // marks important, Backspace archives, Ctrl+Z takes it back.
    assert!(seen(&check, ids[1]));
    keys(cx, window, &["ctrl-u", "ctrl-u"]);
    assert!(!seen(&check, ids[1]), "Ctrl+U is not a toggle");
    keys(cx, window, &["ctrl-q", "ctrl-q"]);
    assert!(seen(&check, ids[1]), "Ctrl+Q is not a toggle");
    keys(cx, window, &["insert"]);
    assert!(check.message(ids[1]).unwrap().unwrap().flagged);
    keys(cx, window, &["backspace"]);
    assert_eq!(undo_text(cx, &view), "Moved to Archive");
    assert_eq!(open_id(cx, &view), Some(ids[2]));
    keys(cx, window, &["ctrl-z"]);
    assert_eq!(open_id(cx, &view), Some(ids[1]));

    // While composing (the composer has the focus), none of the list keys
    // act on the message.
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| v.new_mail(w, cx))
    });
    for k in [
        "down", "up", "home", "end", "pagedown", "pageup", "enter", "insert",
    ] {
        keys(cx, window, &[k]);
    }
    keys(cx, window, &["backspace", "ctrl-u", "ctrl-q"]);
    assert!(view.read_with(cx, |v, cx| v.composer().read(cx).is_open()));
    let m = check.message(ids[1]).unwrap().unwrap();
    assert!(m.seen && m.flagged, "untouched");
    assert!(read(cx, &view, |m| m.held().is_none()));
}

/// Port of `search_keys_and_the_backspace_scope` (its Settings and F6
/// halves wait for phase 6 and 13): Ctrl+E, F3 and Ctrl+F put the keys in
/// the search box; typing searches; Esc there clears the search; Backspace
/// there edits the text, never archives.
#[gpui_kit::test]
fn search_keys_and_the_backspace_scope(cx: &mut TestAppContext) {
    let (f, db) = fixture("search-keys", &[]);
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let search = view.read_with(cx, |v, _| v.search_input().clone());
    let text = |cx: &mut TestAppContext| search.read_with(cx, |s, _| s.value().to_string());
    for key in ["ctrl-e", "f3", "ctrl-f"] {
        focus_list(cx, window, &view);
        keys(cx, window, &[key]);
        in_window(cx, window, |w, cx| test_support::type_text(w, "m3", cx));
        cx.run_until_parked();
        assert_eq!(text(cx), "m3", "{key}");
        assert_eq!(read(cx, &view, |m| m.rows().len()), 1);
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.clear_search(w, cx))
        });
        assert_eq!(read(cx, &view, |m| m.rows().len()), 6);
    }
    // Backspace in the search box edits the text: no archive.
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    keys(cx, window, &["ctrl-e"]);
    in_window(cx, window, |w, cx| test_support::type_text(w, "m", cx));
    keys(cx, window, &["backspace"]);
    cx.run_until_parked();
    assert_eq!(text(cx), "");
    assert!(read(cx, &view, |m| m.held().is_none()));
    // Esc in the search box clears the search, then hands the keys back.
    in_window(cx, window, |w, cx| test_support::type_text(w, "m", cx));
    cx.run_until_parked();
    assert!(read(cx, &view, Mail::searching));
    keys(cx, window, &["escape"]);
    assert_eq!(text(cx), "");
    assert!(!read(cx, &view, Mail::searching));
    // Nor Backspace in Drafts or the Outbox, whatever has the focus.
    for v in [View::Drafts, View::Outbox] {
        act(cx, window, &view, |m| m.set_view(v));
        focus_list(cx, window, &view);
        keys(cx, window, &["down", "backspace"]);
        assert!(read(cx, &view, |m| m.held().is_none()), "{v:?}");
    }
}

/// Port of `filters_empty_states_and_mark_all_read`.
#[gpui_kit::test]
fn filters_empty_states_and_mark_all_read(cx: &mut TestAppContext) {
    let (f, db) = fixture("filters", &[("Work", FolderRole::Other)]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    check.set_seen_many(&[ids[0], ids[1]], true).unwrap();
    check.set_flagged(ids[3], true).unwrap();
    let (window, view) = open(cx, db);
    let listed = |cx: &mut TestAppContext| read(cx, &view, |m| m.rows().len());
    let empty = |cx: &mut TestAppContext| {
        read(cx, &view, |m| {
            super::text::empty(m.empty(), komorebi::i18n::Lang::En)
        })
    };
    // Unread: the four unread; a message opened there stays listed (read
    // by now) until the filter or the view changes.
    act(cx, window, &view, |m| m.set_filter(Filter::Unread));
    assert_eq!(listed(cx), 4);
    act(cx, window, &view, |m| m.row_clicked(ids[2]));
    assert_eq!(listed(cx), 4);
    act(cx, window, &view, Mail::reload_rows);
    assert_eq!(listed(cx), 4);
    // Important, then a search with no match.
    act(cx, window, &view, |m| m.set_filter(Filter::Flagged));
    assert_eq!(listed(cx), 1);
    check.set_flagged(ids[3], false).unwrap();
    act(cx, window, &view, |m| m.set_filter(Filter::Flagged));
    assert_eq!(listed(cx), 0);
    assert_eq!(empty(cx), "No important messages");
    act(cx, window, &view, |m| m.search_edited("nothing like it"));
    assert_eq!(empty(cx), "No matches");
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| v.clear_search(w, cx))
    });

    // Mark all as read: every message of the view, at once.
    act(cx, window, &view, |m| m.set_filter(Filter::Unread));
    assert_eq!(listed(cx), 3);
    act(cx, window, &view, Mail::mark_all_read);
    assert_eq!(listed(cx), 0);
    assert_eq!(empty(cx), "No unread messages");
    assert!(check.briefs(&ids).unwrap().iter().all(|m| m.seen));
    assert_eq!(read(cx, &view, Mail::inbox_unread), 0);

    // Another folder: the filter is off again, an empty folder says so.
    let work = read(cx, &view, |m| {
        m.folders().iter().find(|f| f.display == "Work").unwrap().id
    });
    act(cx, window, &view, |m| m.set_view(View::Folder(work)));
    assert_eq!(read(cx, &view, Mail::filter), Filter::All);
    assert_eq!(empty(cx), "Nothing here");
    view.update(cx, |v, _| {
        v.mail
            .account_state
            .insert(f.account, super::model::AccountState::Unreachable);
    });
    assert!(empty(cx).starts_with("Can't reach the server"));
    // Drafts have no filters.
    act(cx, window, &view, |m| m.set_view(View::Drafts));
    act(cx, window, &view, |m| m.set_filter(Filter::Unread));
    assert_eq!(read(cx, &view, Mail::filter), Filter::All);
}

/// Port of `search_all_folders` (the rows' tags).
#[gpui_kit::test]
fn search_all_folders(cx: &mut TestAppContext) {
    let (f, db) = fixture("search-all", &[]);
    let mut writer = f.cache.open();
    let (archive, sent) = {
        let by = |r| writer.folder_by_role(f.account, r).unwrap().unwrap().id;
        (by(FolderRole::Archive), by(FolderRole::Sent))
    };
    let joao = NewHeader {
        uid: 41,
        subject: "Orçamento".into(),
        from_name: "Xavier Gonçalves".into(),
        from_addr: "xavier@example.org".into(),
        date: 1_600_000_000,
        ..Default::default()
    };
    writer
        .insert_headers(f.account, archive, std::slice::from_ref(&joao))
        .unwrap();
    let reply = NewHeader {
        uid: 42,
        subject: "Re: Orçamento".into(),
        from_name: "Me".into(),
        from_addr: "me@example.org".into(),
        to: "Xavier Gonçalves <xavier@example.org>".into(),
        date: 1_600_000_100,
        ..joao
    };
    writer.insert_headers(f.account, sent, &[reply]).unwrap();
    let (window, view) = open(cx, db);
    let rows = |cx: &mut TestAppContext| {
        read(cx, &view, |m| {
            (0..m.rows().len())
                .filter_map(|i| m.rows().row(i))
                .map(|r| (r.sender, r.tag))
                .collect::<Vec<_>>()
        })
    };
    // In the inbox, "xavier" finds nothing; all folders find both, each
    // tagged with its account and folder, the sent one by recipient.
    act(cx, window, &view, |m| m.search_edited("xavier"));
    assert!(rows(cx).is_empty());
    act(cx, window, &view, |m| m.set_search_all(true));
    let tag = |folder: &str| format!("Seed Account · {folder}");
    assert_eq!(
        rows(cx),
        [
            (
                "To: Xavier Gonçalves <xavier@example.org>".to_string(),
                tag("Sent")
            ),
            ("Xavier Gonçalves".to_string(), tag("Archive")),
        ]
    );
    // Another view: the scope goes back to it.
    act(cx, window, &view, |m| m.set_view(View::Folder(archive)));
    assert!(!read(cx, &view, Mail::search_all));
    assert_eq!(rows(cx).len(), 1);
}

fn nav_labels(cx: &mut TestAppContext, view: &Entity<MailView>) -> Vec<(String, usize, bool)> {
    read(cx, view, |m| {
        m.nav()
            .items
            .into_iter()
            .map(|r| (r.label, r.level, r.expanded))
            .collect()
    })
}

fn nav_line(cx: &mut TestAppContext, view: &Entity<MailView>, label: &str) -> usize {
    nav_labels(cx, view)
        .iter()
        .position(|r| r.0 == label)
        .unwrap_or_else(|| panic!("no line {label}"))
}

/// Port of `the_first_default_folds_are_written_after_the_first_frame`:
/// held in memory at start, written once the first frame is up.
#[gpui_kit::test]
fn the_first_default_folds_are_written_after_the_first_frame(cx: &mut TestAppContext) {
    let (f, db) = fixture("folds-init", &[]);
    let check = f.cache.open();
    let projects = check
        .upsert_folder(f.account, "Projects", "Projects", 0, FolderRole::Other)
        .unwrap();
    check
        .upsert_folder(f.account, "Projects/Alpha", "Alpha", 1, FolderRole::Other)
        .unwrap();
    let mail = Mail::new(std::rc::Rc::new(f.cache.open()));
    assert!(mail.folds_unsaved(), "held in memory, not written yet");
    assert_eq!(check.settings().unwrap().nav_folds_init, "");
    drop(mail);
    let (window, view) = open(cx, db);
    in_window(cx, window, |_, _| {});
    cx.run_until_parked();
    assert!(!read(cx, &view, Mail::folds_unsaved));
    let settings = check.settings().unwrap();
    assert_eq!(
        komorebi::tree::parse_ids(&settings.nav_folds_init),
        HashSet::from([f.account])
    );
    assert_eq!(
        komorebi::tree::parse_folds(&settings.nav_folded),
        HashSet::from([Fold::Folder(projects)])
    );
}

/// Port of `folders_fold_into_a_tree_that_is_remembered`.
#[gpui_kit::test]
fn folders_fold_into_a_tree_that_is_remembered(cx: &mut TestAppContext) {
    let (f, db) = fixture("navtree", &[]);
    let mut writer = f.cache.open();
    let add = |db: &Db, name: &str, display: &str, depth| {
        db.upsert_folder(f.account, name, display, depth, FolderRole::Other)
            .unwrap()
    };
    let projects = add(&writer, "Projects", "Projects", 0);
    let alpha = add(&writer, "Projects/Alpha", "Alpha", 1);
    let q1 = add(&writer, "Projects/Alpha/Q1", "Q1", 2);
    add(&writer, "Projects-old", "Projects-old", 0);
    let unread = |uid| NewHeader {
        uid,
        msgid: format!("q{uid}@example.org"),
        date: 1_700_000_000,
        ..Default::default()
    };
    writer
        .insert_headers(f.account, q1, &[unread(1), unread(2)])
        .unwrap();
    // Opened before: no default folds this time.
    writer
        .set_setting("nav_folds_init", &f.account.to_string())
        .unwrap();
    let (window, view) = open(cx, db);
    let tree: Vec<(String, usize)> = nav_labels(cx, &view)
        .into_iter()
        .skip(2)
        .map(|(l, level, _)| (l, level))
        .collect();
    assert_eq!(
        tree,
        [
            ("Inbox".to_string(), 0),
            ("Sent".to_string(), 0),
            ("Archive".to_string(), 0),
            ("Projects".to_string(), 0),
            ("Alpha".to_string(), 1),
            ("Q1".to_string(), 2),
            ("Projects-old".to_string(), 0),
        ]
    );
    // Open Q1, then fold Projects: the view stays, the pill goes to
    // Projects, which shows the unread below it.
    act(cx, window, &view, |m| m.set_view(View::Folder(q1)));
    assert_eq!(
        read(cx, &view, |m| m.nav().pill),
        Some(nav_line(cx, &view, "Q1"))
    );
    let row = nav_line(cx, &view, "Projects");
    act(cx, window, &view, |m| {
        m.nav_toggle(NavKind::Folder(projects))
    });
    assert_eq!(read(cx, &view, Mail::view), View::Folder(q1));
    assert_eq!(read(cx, &view, |m| m.nav().pill), Some(row));
    let lines = read(cx, &view, |m| m.nav().items);
    assert!(!lines.iter().any(|r| r.label == "Alpha" || r.label == "Q1"));
    assert!(!lines[row].expanded);
    assert_eq!(lines[row].unread, 2);
    // Saved, and read back at the next start.
    assert_eq!(
        writer.settings().unwrap().nav_folded,
        format!("f{projects}")
    );

    // The keys: Right opens Projects, Right again goes to Alpha; Left
    // folds the open Alpha, and Left again goes up to Projects.
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.nav_cursor = row;
            let focus = v.nav_focus().clone();
            w.focus(&focus, cx);
        })
    });
    keys(cx, window, &["right"]);
    assert!(nav_labels(cx, &view)[row].2, "Right opens Projects");
    assert_eq!(view.read_with(cx, |v, _| v.nav_cursor), row);
    keys(cx, window, &["right"]);
    let alpha_row = nav_line(cx, &view, "Alpha");
    assert_eq!(view.read_with(cx, |v, _| v.nav_cursor), alpha_row);
    keys(cx, window, &["left"]);
    assert!(!nav_labels(cx, &view)[alpha_row].2, "Left folds Alpha");
    keys(cx, window, &["left"]);
    assert_eq!(view.read_with(cx, |v, _| v.nav_cursor), row, "Left goes up");
    assert!(read(cx, &view, |m| m.is_folded(Fold::Folder(alpha))));
    // Folding the account line from a folder inside it: Left goes to the
    // line, Left again folds it; the view stays.
    let inbox = nav_line(cx, &view, "Inbox");
    view.update(cx, |v, _| v.nav_cursor = inbox);
    keys(cx, window, &["left"]);
    let line = view.read_with(cx, |v, _| v.nav_cursor);
    assert!(matches!(
        read(cx, &view, |m| m.nav().items[line].kind),
        NavKind::Account(_)
    ));
    keys(cx, window, &["left"]);
    assert_eq!(read(cx, &view, |m| m.nav().items.len()), line + 1);
    assert_eq!(read(cx, &view, |m| m.nav().pill), Some(line));
    assert_eq!(read(cx, &view, Mail::view), View::Folder(q1));
    // A folder that is gone loses its fold.
    let keep: Vec<i64> = read(cx, &view, |m| {
        m.folders()
            .iter()
            .map(|f| f.id)
            .filter(|&id| id != alpha)
            .collect()
    });
    writer.retain_folders(f.account, &keep).unwrap();
    act(cx, window, &view, Mail::load_folders);
    assert_eq!(
        writer.settings().unwrap().nav_folded,
        format!("a{}", f.account)
    );
    let _ = &mut writer;
}

/// Port of `the_folder_pane_menu_and_rail_act_on_their_row` (its
/// Account settings step waits for Settings, phase 6).
#[gpui_kit::test]
fn the_folder_pane_menu_and_rail_act_on_their_row(cx: &mut TestAppContext) {
    let (f, db) = fixture("navmenu", &[]);
    let mut writer = f.cache.open();
    let projects = writer
        .upsert_folder(f.account, "Projects", "Projects", 0, FolderRole::Other)
        .unwrap();
    writer
        .upsert_folder(f.account, "Projects/Alpha", "Alpha", 1, FolderRole::Other)
        .unwrap();
    let news = writer
        .upsert_folder(
            f.account,
            "Newsletters",
            "Newsletters",
            0,
            FolderRole::Other,
        )
        .unwrap();
    let unread = |uid| NewHeader {
        uid,
        msgid: format!("n{uid}@example.org"),
        date: 1_700_000_000,
        ..Default::default()
    };
    writer
        .insert_headers(f.account, news, &[unread(1), unread(2)])
        .unwrap();
    writer
        .set_setting("nav_folds_init", &f.account.to_string())
        .unwrap();
    let server = |host: &str| komorebi::providers::Server {
        host: host.into(),
        port: 993,
        security: komorebi::providers::Security::Tls,
    };
    let work = writer
        .add_account(&komorebi::db::NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: "work@example.org".into(),
            name: String::new(),
            imap: server("imap.example.invalid"),
            smtp: server("smtp.example.invalid"),
            save_sent: true,
            smtp_no_auth: false,
        })
        .unwrap();
    let work_inbox = writer
        .upsert_folder(work, "INBOX", "Inbox", 0, FolderRole::Inbox)
        .unwrap();
    let (window, view) = open(cx, db);
    let shown = |cx: &mut TestAppContext, id: i64| {
        read(cx, &view, |m| {
            m.shown_unread().get(&id).copied().unwrap_or(0)
        })
    };
    // The second account starts folded (accounts after the first).
    assert!(read(cx, &view, |m| m.is_folded(Fold::Account(work))));
    // The rail: All inboxes and Drafts, then one tile per account, its
    // pill on the open folder's account.
    let rail = read(cx, &view, Mail::rail);
    assert_eq!(rail.items.len(), 4);
    assert_eq!(rail.items[1].kind, NavKind::Drafts);
    assert_eq!(rail.items[3].kind, NavKind::Account(work));
    assert_eq!(rail.items[3].label, "work@example.org");
    act(cx, window, &view, |m| m.set_view(View::Folder(news)));
    assert_eq!(read(cx, &view, |m| m.rail().pill), Some(2));
    act(cx, window, &view, |m| {
        m.nav_clicked(NavKind::Account(work), true)
    });
    assert_eq!(read(cx, &view, Mail::view), View::Folder(work_inbox));
    assert_eq!(read(cx, &view, |m| m.rail().pill), Some(3));
    // A rail tile's menu goes to its account's Inbox, Sent or Archive
    // (greyed where the account has none).
    let tile = view.read_with(cx, |v, _| {
        super::menus::nav_entries(v, NavKind::Account(work), true)
    });
    // Its first group (the second is Hide folders, as on every line).
    let entries: Vec<(String, bool)> = tile[0]
        .iter()
        .map(|e| (e.label.clone(), e.enabled))
        .collect();
    assert_eq!(
        entries,
        [
            ("Inbox".to_string(), true),
            ("Sent".to_string(), false),
            ("Archive".to_string(), false),
        ]
    );
    act(cx, window, &view, |m| m.set_view(View::AllInboxes));

    let menu = |cx: &mut TestAppContext, kind| -> Vec<(String, bool)> {
        view.read_with(cx, |v, _| super::menus::nav_entries(v, kind, false))
            .into_iter()
            .flatten()
            .map(|e| (e.label, e.enabled))
            .collect()
    };
    // No menu on Drafts.
    assert!(menu(cx, NavKind::Drafts).is_empty());
    // Mark all as read on a folder: exactly that folder, whatever view is
    // open.
    assert_eq!(
        menu(cx, NavKind::Folder(news)),
        [
            ("Mark all as read".to_string(), true),
            ("Collapse all".to_string(), true),
            ("Hide folders".to_string(), true),
        ]
    );
    let inbox = f.inbox;
    assert_eq!(shown(cx, inbox), 6);
    act(cx, window, &view, |m| {
        m.nav_mark_read(NavKind::Folder(news))
    });
    assert_eq!(shown(cx, news), 0);
    assert_eq!(shown(cx, inbox), 6);
    assert_eq!(read(cx, &view, Mail::view), View::AllInboxes);
    assert!(
        !menu(cx, NavKind::Folder(news))[0].1,
        "nothing left to mark read"
    );
    // No Undo for it, as in Windows Mail.
    assert!(read(cx, &view, |m| m.held().is_none()));

    // The account line: Collapse all folders folds its parent folders;
    // then nothing is left to collapse.
    let me = NavKind::Account(f.account);
    assert_eq!(
        menu(cx, me),
        [
            ("Mark all as read".to_string(), true),
            ("Collapse all folders".to_string(), true),
            ("Refresh".to_string(), true),
            ("Account settings".to_string(), true),
            ("Hide folders".to_string(), true),
        ]
    );
    act(cx, window, &view, |m| m.collapse_all(me));
    assert!(read(cx, &view, |m| {
        m.is_folded(Fold::Folder(projects)) && m.is_folded(Fold::Account(work))
    }));
    assert!(!nav_labels(cx, &view).iter().any(|r| r.0 == "Alpha"));
    assert!(!menu(cx, me)[1].1, "nothing left to collapse");
    // Mark all as read on the account: its Inbox.
    act(cx, window, &view, |m| m.nav_mark_read(me));
    assert_eq!(shown(cx, inbox), 0);
    // A folded account line has nothing to collapse.
    assert!(!menu(cx, NavKind::Account(work))[1].1);
    // A server Drafts folder's rows are never shown unread.
    let drafts = writer
        .upsert_folder(f.account, "Drafts", "Drafts", 0, FolderRole::Drafts)
        .unwrap();
    writer
        .insert_headers(
            f.account,
            drafts,
            &[NewHeader {
                uid: 1,
                msgid: "d1@example.org".into(),
                date: 1_700_000_000,
                ..Default::default()
            }],
        )
        .unwrap();
    act(cx, window, &view, Mail::load_folders);
    assert!(!menu(cx, NavKind::Folder(drafts))[0].1);
    act(cx, window, &view, |m| m.set_view(View::Folder(drafts)));
    let list = view.read_with(cx, |v, _| super::menus::list_entries(v));
    assert!(
        !list[0].enabled,
        "no Mark all as read in a server Drafts folder"
    );
}

/// Port of `mark_all_read_is_one_function_from_the_pane_and_the_list`.
#[gpui_kit::test]
fn mark_all_read_is_one_function_from_the_pane_and_the_list(cx: &mut TestAppContext) {
    let (f, db) = fixture("read-all", &[]);
    let check = f.cache.open();
    let inbox = f.inbox;
    let (window, view) = open(cx, db);
    let unread = |cx: &mut TestAppContext| {
        read(cx, &view, |m| {
            m.shown_unread().get(&inbox).copied().unwrap_or(0)
        })
    };
    assert_eq!(read(cx, &view, Mail::view), View::Folder(inbox));
    assert_eq!(unread(cx), 6);
    let list = view.read_with(cx, |v, _| super::menus::list_entries(v));
    assert_eq!(
        (list[0].label.as_str(), list[0].enabled),
        ("Mark all as read", true)
    );
    act(cx, window, &view, Mail::mark_all_read);
    assert_eq!(unread(cx), 0);
    assert!(read(cx, &view, |m| m.held().is_none()), "no undo record");
    let list = view.read_with(cx, |v, _| super::menus::list_entries(v));
    assert!(!list[0].enabled);
    // The pane's menu on the same folder runs the same function.
    check.set_seen(f.ids[0], false).unwrap();
    act(cx, window, &view, Mail::load_counts);
    assert_eq!(unread(cx), 1);
    act(cx, window, &view, |m| {
        m.nav_mark_read(NavKind::Folder(inbox))
    });
    assert_eq!(unread(cx), 0);
    assert!(read(cx, &view, |m| m.held().is_none()));
}

/// Port of `the_title_counts_unread_inbox_mail` (the window's title, as
/// the taskbar and Alt+Tab show it).
#[gpui_kit::test]
fn the_title_counts_unread_inbox_mail(cx: &mut TestAppContext) {
    let (f, db) = fixture("title", &[]);
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let title = |cx: &mut TestAppContext| read(cx, &view, Mail::inbox_unread);
    assert_eq!(title(cx), 6);
    assert_eq!(crate::text::window_title(6), "Inbox (6) - Komorebi");
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    assert_eq!(title(cx), 5);
    act(cx, window, &view, Mail::toggle_read);
    assert_eq!(title(cx), 6);
}

/// Port of `quick_actions_never_open_or_mark_their_row_read`.
#[gpui_kit::test]
fn quick_actions_never_open_or_mark_their_row_read(cx: &mut TestAppContext) {
    let (f, db) = fixture("quick", &[("Trash", FolderRole::Trash)]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let message = |id| check.message(id).unwrap().unwrap();
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    act(cx, window, &view, |m| m.quick_action(ids[2], Quick::Flag));
    assert!(message(ids[2]).flagged && !message(ids[2]).seen);
    act(cx, window, &view, |m| m.quick_action(ids[2], Quick::Flag));
    assert!(!message(ids[2]).flagged);
    act(cx, window, &view, |m| m.quick_action(ids[2], Quick::Read));
    assert!(message(ids[2]).seen);
    act(cx, window, &view, |m| m.quick_action(ids[2], Quick::Read));
    assert!(!message(ids[2]).seen);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    assert_eq!(selected(cx, &view), 1);
    // Archive another row: held (with Undo), the open message stays.
    act(cx, window, &view, |m| {
        m.quick_action(ids[3], Quick::Archive)
    });
    assert_eq!(held_ids(cx, &view), [ids[3]]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    assert!(!message(ids[3]).seen, "not read by archiving");
    assert_eq!(undo_text(cx, &view), "Moved to Archive");
    act(cx, window, &view, |m| m.quick_action(ids[4], Quick::Delete));
    assert_eq!(held_ids(cx, &view), [ids[4]]);
    assert_eq!(undo_text(cx, &view), "Moved to Trash");
    // A selection of three: archiving one of them from its row leaves the
    // other two selected (nothing opens).
    act(cx, window, &view, |m| m.row_extended(ids[2]));
    assert_eq!((selected(cx, &view), open_id(cx, &view)), (3, None));
    act(cx, window, &view, |m| {
        m.quick_action(ids[1], Quick::Archive)
    });
    assert_eq!(held_ids(cx, &view), [ids[1]]);
    assert_eq!((selected(cx, &view), open_id(cx, &view)), (2, None));
}

/// The commands of a message tab or window (`Mail::act_on_message`): Move,
/// Report junk and Not junk act on that message only, held with Undo as
/// the list's moves are; Move notes its folder as recent; they act while
/// the Mail tab shows another view too.
#[gpui_kit::test]
fn a_message_tabs_move_and_junk_act_on_its_message_only(cx: &mut TestAppContext) {
    let (f, db) = fixture(
        "quick-move",
        &[("Junk", FolderRole::Junk), ("Projects", FolderRole::Other)],
    );
    let mut writer = f.cache.open();
    let by = |r| writer.folder_by_role(f.account, r).unwrap().unwrap().id;
    let junk = by(FolderRole::Junk);
    let projects = writer
        .folders()
        .unwrap()
        .into_iter()
        .find(|x| x.name == "Projects")
        .unwrap()
        .id;
    writer
        .insert_headers(
            f.account,
            junk,
            &[NewHeader {
                uid: 30,
                subject: "spam?".into(),
                date: 1_700_000_100,
                ..Default::default()
            }],
        )
        .unwrap();
    let spam = writer.message_ids(&[junk], "").unwrap()[0];
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));

    act(cx, window, &view, |m| {
        m.act_on_message(ids[2], Quick::Move(projects))
    });
    assert_eq!(held_ids(cx, &view), [ids[2]]);
    assert_eq!(undo_text(cx, &view), "Moved to Projects");
    assert_eq!(open_id(cx, &view), Some(ids[0]), "the open message stays");
    read(cx, &view, |m| {
        assert!(matches!(
            m.held().unwrap().jobs[..],
            [(_, Job::Move { dest, .. })] if dest == projects
        ));
        assert_eq!(m.move_targets("")[0].id, projects, "noted as recent");
    });

    act(cx, window, &view, |m| m.act_on_message(ids[3], Quick::Junk));
    assert_eq!(held_ids(cx, &view), [ids[3]]);
    assert_eq!(undo_text(cx, &view), "Moved to Junk");
    // Not junk on an Inbox message: nothing moves.
    act(cx, window, &view, |m| {
        m.act_on_message(ids[4], Quick::NotJunk)
    });
    assert_eq!(held_ids(cx, &view), [ids[3]]);

    // From Junk, with the Mail tab on Drafts: back to the Inbox, read.
    act(cx, window, &view, |m| m.set_view(View::Drafts));
    act(cx, window, &view, |m| {
        m.act_on_message(spam, Quick::NotJunk)
    });
    assert_eq!(held_ids(cx, &view), [spam]);
    assert!(seen(&f.cache.open(), spam), "marked read");
    read(cx, &view, |m| {
        assert!(matches!(
            m.held().unwrap().jobs[..],
            [(_, Job::Move { folder_id, dest, .. })] if folder_id == junk && dest == f.inbox
        ));
    });
    // A row's quick action stays off outside a message list.
    act(cx, window, &view, Mail::flush_held);
    act(cx, window, &view, |m| m.quick_action(ids[5], Quick::Junk));
    assert!(read(cx, &view, |m| m.held().is_none()));
}

/// A message tab's move with the Mail tab on Drafts leaves the drafts
/// alone: their ids are another table's, so a draft with the moved
/// message's number is neither hidden, nor left, nor selected by Undo.
#[gpui_kit::test]
fn a_message_tabs_move_leaves_the_drafts_list_alone(cx: &mut TestAppContext) {
    let (f, db) = fixture("tab-move-drafts", &[("Projects", FolderRole::Other)]);
    let projects = db
        .folders()
        .unwrap()
        .into_iter()
        .find(|x| x.name == "Projects")
        .unwrap()
        .id;
    let ids = f.ids.clone();
    let target = ids[0];
    // Drafts numbered up to the messages' ids: one shares the target's.
    let mut last = 0;
    while last < *ids.iter().max().unwrap() {
        last = db
            .insert_draft(&komorebi::db::DraftRow {
                account_id: f.account,
                to: "ana@example.org".into(),
                subject: format!("draft {last}"),
                body: "Hi".into(),
                message_id: format!("<d{last}@example.org>"),
                ..Default::default()
            })
            .unwrap();
    }
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| {
        m.load_folders();
        m.set_view(View::Drafts);
    });
    let drafts = read(cx, &view, |m| m.rows().len());
    assert!(read(cx, &view, |m| m.rows().index_of(target).is_some()));
    act(cx, window, &view, |m| m.row_clicked(target));
    let state = |cx: &mut TestAppContext| {
        read(cx, &view, |m| {
            (
                m.rows().len(),
                m.selection().ids(),
                m.draft_open(),
                m.open_id(),
            )
        })
    };
    let before = state(cx);
    assert_eq!(before, (drafts, vec![target], Some(target), None));

    act(cx, window, &view, |m| {
        m.act_on_message(target, Quick::Move(projects))
    });
    assert_eq!(held_ids(cx, &view), [target], "the message is held");
    assert_eq!(
        state(cx),
        before,
        "the drafts list, its selection and the open draft stay"
    );

    act(cx, window, &view, Mail::undo);
    assert!(read(cx, &view, |m| m.held().is_none()));
    assert_eq!(state(cx), before, "Undo selects no draft for the message");
}

/// Port of `deleting_from_trash_or_junk_asks_first`: the question opens
/// as a dialog (no default button: the action is Delete).
#[gpui_kit::test]
fn deleting_from_trash_or_junk_asks_first(cx: &mut TestAppContext) {
    let (f, db) = fixture(
        "delete-ask",
        &[("Trash", FolderRole::Trash), ("Junk", FolderRole::Junk)],
    );
    let mut writer = f.cache.open();
    let by = |r| writer.folder_by_role(f.account, r).unwrap().unwrap().id;
    let (trash, junk) = (by(FolderRole::Trash), by(FolderRole::Junk));
    let header = |uid: u32| NewHeader {
        uid,
        subject: format!("old {uid}"),
        date: 1_700_000_000 + i64::from(uid),
        ..Default::default()
    };
    writer
        .insert_headers(f.account, trash, &[header(20)])
        .unwrap();
    writer
        .insert_headers(f.account, junk, &[header(21)])
        .unwrap();
    let (window, view) = open(cx, db);

    act(cx, window, &view, |m| m.set_view(View::Folder(trash)));
    let id = read(cx, &view, |m| m.rows().id_at(0)).unwrap();
    act(cx, window, &view, |m| m.row_clicked(id));
    focus_list(cx, window, &view);
    keys(cx, window, &["delete"]);
    in_window(cx, window, |w, cx| {
        assert!(w.has_active_dialog(cx), "asked first")
    });
    assert_eq!(
        super::text::question(super::model::Question::DeleteFrom { trash: true }, 1).0,
        "Delete 1 message for good?"
    );
    assert!(read(cx, &view, |m| m.held().is_none()));
    // Cancelled: nothing moves.
    in_dialog(cx, window, "dialog-cancel");
    cx.run_until_parked();
    assert!(read(cx, &view, |m| m.held().is_none()
        && m.rows().index_of(id).is_some()));
    // Asked again and confirmed: a held delete on the server.
    focus_list(cx, window, &view);
    keys(cx, window, &["delete"]);
    in_dialog(cx, window, "dialog-confirm");
    cx.run_until_parked();
    read(cx, &view, |m| {
        assert!(matches!(
            m.held().unwrap().jobs[..],
            [(_, Job::Delete { folder_id, .. })] if folder_id == trash
        ));
    });
    act(cx, window, &view, Mail::flush_held);

    // Junk: asks, then moves to Trash.
    act(cx, window, &view, |m| m.set_view(View::Folder(junk)));
    let id = read(cx, &view, |m| m.rows().id_at(0)).unwrap();
    act(cx, window, &view, |m| m.row_clicked(id));
    act(cx, window, &view, Mail::delete);
    in_window(cx, window, |w, cx| {
        assert!(w.has_active_dialog(cx), "asked first")
    });
    assert_eq!(
        super::text::question(super::model::Question::DeleteFrom { trash: false }, 1).0,
        "Delete 1 message from Junk?"
    );
    in_dialog(cx, window, "dialog-confirm");
    cx.run_until_parked();
    read(cx, &view, |m| {
        assert!(matches!(
            m.held().unwrap().jobs[..],
            [(_, Job::Move { dest, .. })] if dest == trash
        ));
    });
    let _ = &mut writer;
}

/// Clicks `button` in the newest dialog: a closed one stays in the layer
/// while it fades, so the bare id can match two.
fn in_dialog(cx: &mut TestAppContext, window: AnyWindowHandle, button: &str) {
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
        let newest = (0..16)
            .rev()
            .map(|i| gpui::ElementId::Name(format!("dialog-{i}").into()))
            .find(|id| w.try_find(id.clone()).is_some())
            .expect("a dialog is open");
        w.within(newest).click(button.to_owned(), cx);
    });
    cx.run_until_parked();
}

/// Port of `forward_move_and_inbox_keys`.
#[gpui_kit::test]
fn forward_move_and_inbox_keys(cx: &mut TestAppContext) {
    let (f, db) = fixture("shift-keys", &[]);
    let ids = f.ids.clone();
    let inbox = f.inbox;
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    focus_list(cx, window, &view);
    keys(cx, window, &["ctrl-shift-f"]);
    let draft = view.read_with(cx, |v, cx| v.composer().read(cx).draft());
    assert_eq!(
        draft,
        Some(Draft::Forward {
            message: ids[0],
            attached: false
        })
    );
    // Ctrl+Shift+V: the Move flyout opens.
    focus_list(cx, window, &view);
    keys(cx, window, &["ctrl-shift-v"]);
    assert!(
        view.read_with(cx, |v, _| v.flyout.is_some()),
        "the Move flyout is open"
    );
    in_window(cx, window, |w, _| assert_named(w, "Move to", Role::Menu));
    keys(cx, window, &["escape"]);
    assert!(view.read_with(cx, |v, _| v.flyout.is_none()));
    // Ctrl+Shift+I from another view: the Inbox (one account).
    act(cx, window, &view, |m| m.set_view(View::Drafts));
    focus_list(cx, window, &view);
    keys(cx, window, &["ctrl-shift-i"]);
    assert_eq!(read(cx, &view, Mail::view), View::Folder(inbox));
}

/// Port of `the_move_flyout_lists_recent_folders_and_filters`.
#[gpui_kit::test]
fn the_move_flyout_lists_recent_folders_and_filters(cx: &mut TestAppContext) {
    let (f, db) = fixture(
        "move-flyout",
        &[
            ("Drafts", FolderRole::Drafts),
            ("Projects", FolderRole::Other),
            ("Receipts", FolderRole::Other),
        ],
    );
    let writer = f.cache.open();
    writer
        .upsert_folder(
            f.account,
            "Projects/Komorebi",
            "Komorebi",
            1,
            FolderRole::Other,
        )
        .unwrap();
    writer
        .upsert_folder(
            f.account,
            "INBOX/Invoices",
            "Invoices",
            1,
            FolderRole::Other,
        )
        .unwrap();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let targets = |cx: &mut TestAppContext, filter: &str| -> Vec<(String, usize, bool)> {
        read(cx, &view, |m| {
            m.move_targets(filter)
                .into_iter()
                .map(|o| (o.label, o.level, o.recent))
                .collect()
        })
    };
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    let all = targets(cx, "");
    assert!(
        all.iter().all(|t| t.0 != "Drafts" && t.0 != "Inbox"),
        "{all:?}"
    );
    assert!(all.contains(&("Komorebi".into(), 1, false)), "{all:?}");
    // A child of the current folder (left out) is drawn flush.
    assert!(all.contains(&("Invoices".into(), 0, false)), "{all:?}");
    assert!(all.iter().all(|t| !t.2), "nothing used yet");
    // Move one to Receipts: it is the first recent folder after.
    let receipts = read(cx, &view, |m| {
        m.folders()
            .iter()
            .find(|f| f.display == "Receipts")
            .unwrap()
            .id
    });
    act(cx, window, &view, |m| m.move_to(receipts));
    assert_eq!(undo_text(cx, &view), "Moved to Receipts");
    assert_eq!(targets(cx, "")[0], ("Receipts".into(), 0, true));
    assert_eq!(writer.settings().unwrap().move_recent, receipts.to_string());
    // The filter narrows the list (no recent section then).
    assert_eq!(targets(cx, "PRO"), [("Projects".to_string(), 0, false)]);
    assert!(targets(cx, "zz").is_empty());

    // The flyout: typed into, Enter moves to the entry chosen.
    focus_list(cx, window, &view);
    keys(cx, window, &["ctrl-shift-v"]);
    in_window(cx, window, |w, cx| test_support::type_text(w, "proj", cx));
    cx.run_until_parked();
    keys(cx, window, &["enter"]);
    assert_eq!(undo_text(cx, &view), "Moved to Projects");
    assert!(view.read_with(cx, |v, _| v.flyout.is_none()));
}

/// Port of `select_messages_checks_rows_without_opening_them`.
#[gpui_kit::test]
fn select_messages_checks_rows_without_opening_them(cx: &mut TestAppContext) {
    let (f, db) = fixture("select-mode", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let list = view.read_with(cx, |v, _| super::menus::list_entries(v));
    let select = list.iter().find(|e| e.label == "Select messages").unwrap();
    assert_eq!((select.checked, select.enabled), (Some(false), true));
    act(cx, window, &view, |m| m.set_select_mode(true));
    let list = view.read_with(cx, |v, _| super::menus::list_entries(v));
    assert_eq!(list[1].checked, Some(true));
    // A click then checks a row without opening it.
    in_window(cx, window, |w, cx| {
        w.click(gpui::ElementId::Name(format!("row-{}", ids[2]).into()), cx);
        w.click(gpui::ElementId::Name(format!("row-{}", ids[3]).into()), cx);
    });
    cx.run_until_parked();
    assert_eq!(selected(cx, &view), 2);
    assert_eq!(open_id(cx, &view), None, "checked, not opened");
    assert!(!seen(&check, ids[2]));
    // The rows' checks are named checkboxes.
    in_window(cx, window, |w, _| {
        let checks = test_support::names(w)
            .into_iter()
            .filter(|(role, _)| *role == Some(Role::CheckBox))
            .count();
        assert!(checks >= 6, "{checks}");
    });
    // The panel's Select all and Clear selection.
    act(cx, window, &view, Mail::select_all);
    assert_eq!(selected(cx, &view), 6);
    act(cx, window, &view, Mail::clear_selection);
    assert_eq!(selected(cx, &view), 0);
    assert!(read(cx, &view, Mail::select_mode), "Select stays on");
    // Esc with nothing selected turns it off; so does another view.
    focus_list(cx, window, &view);
    keys(cx, window, &["escape"]);
    assert!(!read(cx, &view, Mail::select_mode));
    act(cx, window, &view, |m| m.set_select_mode(true));
    act(cx, window, &view, |m| m.set_view(View::Drafts));
    assert!(!read(cx, &view, Mail::select_mode));
}

/// Port of `the_undo_offer_holds_its_window_and_the_next_move_flushes_it`
/// (the clock with the model's own instants; GPUI's test clock does not
/// move `Instant`).
#[gpui_kit::test]
fn the_undo_offer_holds_its_window_and_the_next_move_flushes_it(cx: &mut TestAppContext) {
    let (f, db) = fixture("undo-slot", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    act(cx, window, &view, Mail::archive);
    assert_eq!(undo_text(cx, &view), "Moved to Archive");
    in_window(cx, window, |w, _| assert_named(w, "Undo", Role::Button));
    let seq = read(cx, &view, |m| m.held().unwrap().seq);
    let t0 = Instant::now();
    // A status line doesn't take its place.
    view.update(cx, |v, _| {
        v.mail.status = Some(super::model::Status::UndoWindow)
    });
    assert_eq!(undo_text(cx, &view), "Moved to Archive");
    // Hovered: the window stops; left: it goes on (3 s at least).
    act(cx, window, &view, |m| m.undo_hold(true, t0));
    act(cx, window, &view, |m| {
        m.undo_due(seq, t0 + UNDO_WINDOW + UNDO_WINDOW)
    });
    assert!(read(cx, &view, |m| m.held().is_some()), "not while hovered");
    let later = t0 + Duration::from_secs(60);
    act(cx, window, &view, |m| m.undo_hold(false, later));
    act(cx, window, &view, |m| {
        m.undo_due(seq, later + Duration::from_secs(1))
    });
    assert!(read(cx, &view, |m| m.held().is_some()));
    // The next archive sends the first and offers its own Undo.
    act(cx, window, &view, Mail::archive);
    assert!(
        check.message(ids[0]).unwrap().is_none(),
        "the first archive went to the server"
    );
    assert_eq!(held_ids(cx, &view), [ids[1]]);
    // Ctrl+Z takes the last one back.
    focus_list(cx, window, &view);
    keys(cx, window, &["ctrl-z"]);
    assert!(held_ids(cx, &view).is_empty());
    assert_eq!(undo_text(cx, &view), "");
    let _ = UNDO_RESUME;
}

/// The window opens on the list: the panes and rows as UI Automation reads
/// them (the list, a row named as the Slint row was, the folder tree and
/// its lines).
#[gpui_kit::test]
fn the_mail_view_is_named_for_screen_readers(cx: &mut TestAppContext) {
    let (f, db) = fixture("names", &[]);
    let (window, view) = open(cx, db);
    let first = read(cx, &view, |m| m.rows().row(0)).unwrap();
    in_window(cx, window, |w, _| {
        assert_named(w, "Messages", Role::List);
        assert_named(w, "Folders", Role::Tree);
        assert_named(w, &super::text::row_name(&first), Role::ListItem);
        assert_named(w, "New mail", Role::Button);
        assert_named(w, "Search mail", Role::TextInput);
        assert_named(w, "Filter", Role::TabList);
    });
    let _ = f;
}

/// A row's quick actions show while it is hovered and act on that row
/// only (the Slint `message-quick`): the open message stays open.
#[gpui_kit::test]
fn quick_actions_show_on_hover_and_act_on_their_row(cx: &mut TestAppContext) {
    let (f, db) = fixture("quick-hover", &[]);
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    let id = |slug: &str, n: i64| gpui::ElementId::Name(format!("{slug}-{n}").into());
    in_window(cx, window, |w, cx| w.hover(id("row", ids[2]), cx));
    assert_eq!(view.read_with(cx, |v, _| v.hovered_row), Some(ids[2]));
    in_window(cx, window, |w, cx| w.click(id("quick-archive", ids[2]), cx));
    cx.run_until_parked();
    assert_eq!(held_ids(cx, &view), [ids[2]]);
    assert_eq!(open_id(cx, &view), Some(ids[0]), "the open message stays");
}

/// Port of `nav_toggle_persists_and_shows_rail_when_wide`
/// (`src/app/composer/tests/tabs_tests.rs`): Ctrl+Shift+E folds the pane
/// to the rail at a wide width, saved; the menus say Show folders; the
/// rail's Folders button shows the pane again.
#[gpui_kit::test]
fn nav_toggle_persists_and_shows_rail_when_wide(cx: &mut TestAppContext) {
    let (f, db) = fixture("navfold", &[]);
    let check = f.cache.open();
    let (window, view) = open(cx, db);
    let rail = |cx: &mut TestAppContext| in_window(cx, window, |w, cx| view.read(cx).is_rail(w));
    assert!(!rail(cx));
    keys(cx, window, &["ctrl-shift-e"]);
    assert!(read(cx, &view, Mail::nav_collapsed));
    assert!(rail(cx), "the rail at 1280 px");
    assert!(!view.read_with(cx, |v, _| v.pane_open));
    assert!(check.settings().unwrap().nav_collapsed);
    // The pane's menu says Show folders now.
    let menu = view.read_with(cx, |v, _| {
        super::menus::nav_entries(v, NavKind::Account(f.account), true)
    });
    assert_eq!(
        menu.last().and_then(|g| g.last()).map(|e| e.label.as_str()),
        Some("Show folders")
    );
    in_window(cx, window, |w, cx| w.click("pane-toggle", cx));
    cx.run_until_parked();
    assert!(!read(cx, &view, Mail::nav_collapsed) && !rail(cx));
    assert!(!check.settings().unwrap().nav_collapsed);
}

/// A narrow window: the rail's Folders button opens the full pane over
/// the content; Esc closes it, and so does opening a folder from it.
#[gpui_kit::test]
fn the_rail_opens_the_full_pane_over_the_content(cx: &mut TestAppContext) {
    let (f, db) = fixture("rail-pane", &[("Projects", FolderRole::Other)]);
    test_support::init_app(cx);
    let (window, main) = cx
        .update(|cx| {
            let bounds = gpui::Bounds::new(
                gpui::point(gpui::px(0.), gpui::px(0.)), // token-exempt: a test window's origin
                gpui::size(gpui::px(900.), gpui::px(700.)), // token-exempt: a test window under the compact width
            );
            let init = test_support::init(Some(db));
            gpui_kit::open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    gpui::AppContext::new(cx, |cx| crate::window::MainWindow::new(init, window, cx))
                },
            )
        })
        .expect("open a narrow window");
    let view = main.read_with(cx, |w, _| w.mail().cloned().unwrap());
    in_window(cx, window, |w, cx| w.click("pane-toggle", cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.pane_open));
    let projects = read(cx, &view, |m| {
        m.folders()
            .iter()
            .find(|f| f.display == "Projects")
            .unwrap()
            .id
    });
    in_window(cx, window, |w, _| {
        let line = w.find(gpui::ElementId::Name(format!("nav-f{projects}").into()));
        assert_eq!(line.role(), Some(Role::TreeItem));
    });
    keys(cx, window, &["escape"]);
    assert!(!view.read_with(cx, |v, _| v.pane_open), "Esc closes it");
    in_window(cx, window, |w, cx| w.click("pane-toggle", cx));
    in_window(cx, window, |w, cx| {
        w.click(gpui::ElementId::Name(format!("nav-f{projects}").into()), cx)
    });
    cx.run_until_parked();
    assert_eq!(read(cx, &view, Mail::view), View::Folder(projects));
    assert!(
        !view.read_with(cx, |v, _| v.pane_open),
        "a folder opened closes it"
    );
    let _ = f;
}

/// The pane opens scrolled to the open view's line (a folder far down a
/// long tree).
#[gpui_kit::test]
fn the_pane_shows_the_open_views_line(cx: &mut TestAppContext) {
    let (f, db) = fixture("pane-scroll", &[]);
    let writer = f.cache.open();
    let mut last = 0;
    for i in 0..40 {
        let name = format!("Folder {i:02}");
        last = writer
            .upsert_folder(f.account, &name, &name, 0, FolderRole::Other)
            .unwrap();
    }
    writer
        .set_setting("last_view", &format!("f{last}"))
        .unwrap();
    let (window, view) = open(cx, db);
    let state = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, _| {
            (
                v.nav_cursor,
                v.nav_scroll.offset().y,
                v.nav_scroll.bounds().size.height,
                v.nav_scroll.bounds_for_item(v.nav_cursor),
            )
        })
    };
    in_window(cx, window, |_, _| {});
    cx.run_until_parked();
    in_window(cx, window, |_, _| {});
    let (cursor, _, height, line) = state(cx);
    let line = line.expect("the line is laid out");
    assert!(
        line.size.height > gpui::Pixels::ZERO && height > gpui::Pixels::ZERO,
        "{cursor}"
    );
    let offset = view.read_with(cx, |v, _| v.nav_scroll.offset().y);
    assert!(
        offset < gpui::Pixels::ZERO,
        "scrolled to the last folder: {offset:?}"
    );
}

// ------------------------------------------------------------- stage D

/// A window of `width` x 700 on the fixture's cache (narrow layouts).
fn open_sized(cx: &mut TestAppContext, db: Db, width: f32) -> (AnyWindowHandle, Entity<MailView>) {
    test_support::init_app(cx);
    let (window, main) = cx
        .update(|cx| {
            let bounds = gpui::Bounds::new(
                gpui::point(gpui::px(0.), gpui::px(0.)), // token-exempt: a test window's origin
                gpui::size(gpui::px(width), gpui::px(700.)), // token-exempt: a test window of the width asked for
            );
            let init = test_support::init(Some(db));
            gpui_kit::open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    gpui::AppContext::new(cx, |cx| crate::window::MainWindow::new(init, window, cx))
                },
            )
        })
        .expect("open a window");
    let view = main.read_with(cx, |w, _| w.mail().cloned().unwrap());
    (window, view)
}

/// Slint parity: the list's moves work from the reading pane (a click in
/// the message leaves the keys there), as the Slint window's bindings do
/// while reading; F6 and Shift+F6 go round the panes (the Slint
/// `cycle-pane`: search, folders, list, reader).
#[gpui_kit::test]
fn the_reading_pane_moves_through_the_list_and_f6_goes_round(cx: &mut TestAppContext) {
    let (f, db) = fixture("reading-keys", &[]);
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    keys(cx, window, &["down"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    // The keys in the reader (as a click there leaves them).
    // (The reader's page is laid out by its worker, which does not run
    // here: the reading pane's own stop stands in for it.)
    let focus = view.read_with(cx, |v, _| v.reading_focus.clone());
    in_window(cx, window, |w, cx| w.focus(&focus, cx));
    let stack = in_window(cx, window, |w, _| w.context_stack());
    assert!(
        stack.iter().any(|c| c.contains(super::commands::MESSAGES)),
        "the reading pane has the message keys"
    );
    keys(cx, window, &["down"]);
    assert_eq!(open_id(cx, &view), Some(ids[1]));
    // Home, End and the paging keys are the page's (they scroll the
    // message in the Slint app): never another message from here.
    for key in ["end", "home", "pagedown", "pageup"] {
        keys(cx, window, &[key]);
        assert_eq!(open_id(cx, &view), Some(ids[1]), "{key}");
    }
    keys(cx, window, &["shift-down"]);
    assert_eq!(selected(cx, &view), 2);

    // F6 from the list: the reader (a message is open), then the search,
    // the folders and the list again; Shift+F6 goes back.
    focus_list(cx, window, &view);
    keys(cx, window, &["escape"]);
    keys(cx, window, &["down"]);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    let stop = |cx: &mut TestAppContext| {
        in_window(cx, window, |w, cx| {
            view.read_with(cx, |v, cx| v.pane_of_focus(w, cx))
        })
    };
    let cycle = |cx: &mut TestAppContext| {
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.cycle_pane(false, w, cx))
        })
    };
    keys(cx, window, &["f6"]);
    assert_eq!(stop(cx), super::Stop::Reading);
    // (The page is not laid out here, so it has no keys of its own: the
    // next steps call what F6 sends. From the search box the window
    // routes F6, an integrator request.)
    cycle(cx);
    assert_eq!(stop(cx), super::Stop::Search);
    cycle(cx);
    assert_eq!(stop(cx), super::Stop::Folders);
    keys(cx, window, &["f6"]);
    assert_eq!(stop(cx), super::Stop::List);
    keys(cx, window, &["shift-f6"]);
    assert_eq!(stop(cx), super::Stop::Folders);
}

/// The stops F6 skips: a pane that does not show.
#[test]
fn f6_skips_the_panes_that_do_not_show() {
    use super::{Stop, next_stop};
    let all = [true; 4];
    assert_eq!(next_stop(Stop::List, &all, false), Some(Stop::Reading));
    assert_eq!(next_stop(Stop::Reading, &all, false), Some(Stop::Search));
    assert_eq!(next_stop(Stop::Search, &all, true), Some(Stop::Reading));
    // No message open: the list goes on to the search.
    let no_reader = [true, true, true, false];
    assert_eq!(next_stop(Stop::List, &no_reader, false), Some(Stop::Search));
    // One pane while reading: the list is hidden.
    let one_pane = [true, true, false, true];
    assert_eq!(
        next_stop(Stop::Folders, &one_pane, false),
        Some(Stop::Reading)
    );
    assert_eq!(
        next_stop(Stop::Folders, &[false, true, false, false], false),
        None
    );
}

/// Slint parity: the selection panel has Archive, Delete, Move, Mark as
/// read and as unread, Mark as important and Remove importance, Select all
/// and Clear selection, and says why Move is off.
#[gpui_kit::test]
fn the_selection_panel_has_the_slint_commands(cx: &mut TestAppContext) {
    let (f, db) = fixture("selection-panel", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.row_toggled(ids[0]));
    act(cx, window, &view, |m| m.row_toggled(ids[1]));
    in_window(cx, window, |w, _| {
        // (Archive and Delete share their names with folder lines.)
        for name in [
            "Move",
            "Mark as read",
            "Mark as unread",
            "Mark as important",
            "Remove importance",
            "Select all",
            "Clear selection",
        ] {
            assert_named(w, name, Role::Button);
        }
    });
    in_window(cx, window, |w, cx| w.click("sel-flag", cx));
    cx.run_until_parked();
    assert!(check.message(ids[0]).unwrap().unwrap().flagged);
    assert!(check.message(ids[1]).unwrap().unwrap().flagged);
    in_window(cx, window, |w, cx| w.click("sel-unflag", cx));
    cx.run_until_parked();
    assert!(!check.message(ids[0]).unwrap().unwrap().flagged);
    in_window(cx, window, |w, cx| w.click("sel-read", cx));
    cx.run_until_parked();
    assert!(seen(&check, ids[0]) && seen(&check, ids[1]));
    in_window(cx, window, |w, cx| w.click("sel-unread", cx));
    cx.run_until_parked();
    assert!(!seen(&check, ids[0]) && !seen(&check, ids[1]));
    // The caption shows only while Move is off (this account has an
    // Archive to move to).
    let why = in_window(cx, window, |w, _| {
        test_support::names(w)
            .iter()
            .any(|(_, n)| n == "Select messages of one account to move them")
    });
    assert!(!why);
    // Move opens the Move flyout.
    in_window(cx, window, |w, cx| w.click("sel-move", cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.flyout.is_some()));
}

/// Slint parity: below 720 px the reader's bar leads with Back to list
/// (Esc), and the list's Undo offer shows at the message's foot while the
/// list is hidden.
#[gpui_kit::test]
fn one_pane_has_back_to_list_and_keeps_the_undo_offer(cx: &mut TestAppContext) {
    let (f, db) = fixture("one-pane", &[]);
    let ids = f.ids.clone();
    let (window, view) = open_sized(cx, db, 680.);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    in_window(cx, window, |w, _| {
        assert_named(w, "Back to list (Esc)", Role::Button);
    });
    // Archive the open one: the next opens, the offer shows under it.
    act(cx, window, &view, Mail::archive);
    assert_eq!(open_id(cx, &view), Some(ids[1]));
    in_window(cx, window, |w, _| assert_named(w, "Undo", Role::Button));
    in_window(cx, window, |w, cx| w.click("cmd-back", cx));
    cx.run_until_parked();
    assert_eq!(open_id(cx, &view), None);
}

/// Port of the rest of `narrow_windows_show_one_pane_at_a_time`: at desktop
/// width the list and the reading pane show together; below 720 px one at a
/// time: the list while nothing is open, the message over it once one is,
/// and Esc brings the list back. (Here the arrows open the message they
/// reach, so a selection alone is only a multiple one, which shows the
/// selection panel in the reading pane.)
#[gpui_kit::test]
fn the_panes_show_together_on_a_desktop_and_one_at_a_time_when_narrow(cx: &mut TestAppContext) {
    // [search, folders, list, reading pane]
    let stops =
        |cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<MailView>| -> [bool; 4] {
            in_window(cx, window, |w, cx| view.read(cx).pane_stops(w, cx))
        };
    // Desktop: both panes with a message open.
    let (f, db) = fixture("panes-wide", &[]);
    let (window, view) = open_sized(cx, db, 1100.);
    act(cx, window, &view, |m| m.row_clicked(f.ids[0]));
    assert_eq!(open_id(cx, &view), Some(f.ids[0]));
    let s = stops(cx, window, &view);
    assert!(s[2] && s[3], "the list and the reader together: {s:?}");

    // Narrow: the list alone, then the message alone, then Esc.
    let (_f, db) = fixture("panes-narrow", &[]);
    let (window, view) = open_sized(cx, db, 680.);
    let s = stops(cx, window, &view);
    assert!(s[2] && !s[3], "the list alone: {s:?}");
    // Enter opens the first message (none selected): it takes the pane.
    focus_list(cx, window, &view);
    keys(cx, window, &["enter"]);
    assert!(open_id(cx, &view).is_some(), "Enter opens the message");
    let s = stops(cx, window, &view);
    assert!(!s[2] && s[3], "the message over the list: {s:?}");
    // Esc steps back to the list.
    // (The list is not on screen to hold the focus, so the list's Esc is
    // run on the model, as its action does.)
    act(cx, window, &view, |m| {
        m.escape();
    });
    assert_eq!(open_id(cx, &view), None);
    let s = stops(cx, window, &view);
    assert!(s[2] && !s[3], "Esc brings the list back: {s:?}");
}

/// Slint parity: an empty list under a filter offers Show all, which takes
/// the filter away.
#[gpui_kit::test]
fn an_empty_filter_offers_show_all(cx: &mut TestAppContext) {
    let (f, db) = fixture("show-all", &[]);
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.set_filter(Filter::Flagged));
    assert!(read(cx, &view, |m| m.rows().is_empty()));
    in_window(cx, window, |w, _| {
        assert_named(w, "No important messages", Role::Status);
        assert_named(w, "Show all", Role::Button);
    });
    in_window(cx, window, |w, cx| w.click("list-show-all", cx));
    cx.run_until_parked();
    assert_eq!(read(cx, &view, Mail::filter), Filter::All);
    assert_eq!(read(cx, &view, |m| m.rows().len()), f.ids.len());
}

/// Slint parity: an account line's Refresh syncs that account only (the
/// Slint `retry_account`), not every account.
#[gpui_kit::test]
fn an_account_lines_refresh_syncs_that_account(cx: &mut TestAppContext) {
    let (f, db) = fixture("account-refresh", &[]);
    // A second account, so "that account only" is a real choice.
    let server = |host: &str| komorebi::providers::Server {
        host: host.into(),
        port: 993,
        security: komorebi::providers::Security::Tls,
    };
    let work = db
        .add_account(&komorebi::db::NewAccount {
            imap_user: String::new(),
            smtp_user: String::new(),
            email: "work@example.org".into(),
            name: String::new(),
            imap: server("imap.example.invalid"),
            smtp: server("smtp.example.invalid"),
            save_sent: true,
            smtp_no_auth: false,
        })
        .unwrap();
    db.upsert_folder(work, "INBOX", "Inbox", 0, FolderRole::Inbox)
        .unwrap();
    let (window, view) = open(cx, db);
    let hub = view.read_with(cx, |v, _| v.hub().clone());
    let _ = hub.read_with(cx, |h, _| h.captured());
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.run(
                super::menus::Run::Nav(NavKind::Account(f.account), super::menus::NavRun::Refresh),
                w,
                cx,
            )
        })
    });
    let jobs = hub.read_with(cx, |h, _| h.captured());
    assert!(
        jobs.iter()
            .any(|(a, j)| *a == f.account && matches!(j, Job::SyncAll { full: true })),
        "a full sync of the account"
    );
    assert!(
        jobs.iter().all(|(a, _)| *a == f.account),
        "never the other account"
    );
    // The window's Refresh (F5) syncs both, so the check above can fail.
    let _ = hub.read_with(cx, |h, _| h.captured());
    keys(cx, window, &["f5"]);
    let jobs = hub.read_with(cx, |h, _| h.captured());
    assert!(
        jobs.iter().any(|(a, _)| *a == work),
        "F5 reaches the other account"
    );
}

/// Port of the Slint app's source commands (`src/app/original.rs`; rows
/// 140 and 141; row 170's offline "isn't downloaded yet" and row 173's
/// focus back after Show original are not checked here): Copy Message-ID puts `<id>` on
/// the clipboard and says so, Show original opens its dialog, Save as .eml
/// writes the exact bytes where the user chose, and a body not downloaded
/// yet is fetched first, then the command runs.
#[gpui_kit::test]
fn the_source_commands_copy_show_and_save(cx: &mut TestAppContext) {
    let (f, db) = fixture("source", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let said = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, cx| {
            v.hub()
                .read(cx)
                .sync()
                .line()
                .map(|l| super::sync_text::said(&l.said))
                .unwrap_or_default()
        })
    };
    // From the row menu on a selected row: it opens first.
    act(cx, window, &view, |m| {
        m.row_context(ids[0]);
    });
    let entries = view.read_with(cx, |v, _| super::menus::row_entries(v));
    for want in ["Save as .eml", "Show original", "Copy Message-ID"] {
        let e = entries.iter().flatten().find(|e| e.label == want).unwrap();
        assert!(e.enabled, "{want}");
    }
    let run = |cx: &mut TestAppContext, cmd| {
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.run(super::menus::Run::Cmd(cmd), w, cx))
        });
        cx.run_until_parked();
    };
    use super::text::Command;
    run(cx, Command::CopyId);
    assert_eq!(open_id(cx, &view), Some(ids[0]));
    let msgid = check.message(ids[0]).unwrap().unwrap().msgid;
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some(msgid.clone())
    );
    assert_eq!(said(cx), "Message-ID copied.");

    run(cx, Command::ShowOriginal);
    assert!(in_window(cx, window, |w, cx| w.has_active_dialog(cx)));
    in_window(cx, window, |w, _| {
        assert_named(w, "Header fields", Role::MultilineTextInput);
        assert_named(w, "Message source", Role::MultilineTextInput);
    });
    in_window(cx, window, |w, cx| w.close_dialog(cx));

    // Save as .eml: the bytes as cached, where the user chose.
    let out = f.cache.dir().join("saved.eml");
    run(cx, Command::SaveEml);
    let chosen = out.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let raw = check.message(ids[0]).unwrap().unwrap().raw.unwrap();
    assert_eq!(std::fs::read(&out).expect("the saved file"), raw);
    assert!(said(cx).starts_with("Saved "), "{}", said(cx));
    std::fs::remove_file(&out).ok();

    // A body not downloaded yet: fetched first (the worker's job), then
    // the command runs once it arrives.
    let hub = view.read_with(cx, |v, _| v.hub().clone());
    let _ = hub.read_with(cx, |h, _| h.captured());
    let mut db = f.cache.open();
    let header = NewHeader {
        uid: 7,
        msgid: "<m7@example.org>".into(),
        subject: "m7".into(),
        from_addr: "s7@example.org".into(),
        date: 1_700_000_100,
        ..Default::default()
    };
    db.insert_headers(f.account, f.inbox, &[header]).unwrap();
    act(cx, window, &view, Mail::reload_rows);
    let raw_less = db.message_ids(&[f.inbox], "").unwrap()[0];
    assert!(!ids.contains(&raw_less));
    act(cx, window, &view, |m| m.row_clicked(raw_less));
    run(cx, Command::ShowOriginal);
    assert!(!in_window(cx, window, |w, cx| w.has_active_dialog(cx)));
    let jobs = hub.read_with(cx, |h, _| h.captured());
    assert!(
        jobs.iter()
            .any(|(_, j)| matches!(j, Job::FetchBody(id) if *id == raw_less))
    );
    // The reader says so after a second.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1100)); // token-exempt: test clock
    cx.run_until_parked();
    assert_eq!(said(cx), "Fetching message\u{a0}…");
    db.set_body(raw_less, &raw, "Hello", false, Default::default())
        .unwrap();
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.react(&super::sync::Reaction::BodyReady(raw_less), w, cx)
        })
    });
    cx.run_until_parked();
    assert!(in_window(cx, window, |w, cx| w.has_active_dialog(cx)));
}

/// The mail view's events, as the window hears them.
fn events(
    cx: &mut TestAppContext,
    view: &Entity<MailView>,
) -> (
    std::rc::Rc<std::cell::RefCell<Vec<super::MailEvent>>>,
    gpui::Subscription,
) {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let into = seen.clone();
    let sub = cx.update(|cx| {
        cx.subscribe(view, move |_, e: &super::MailEvent, _| {
            into.borrow_mut().push(e.clone());
        })
    });
    (seen, sub)
}

/// A press and release of `button` with `clicks` on the centre of row `id`.
fn press_row(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: i64,
    button: gpui::MouseButton,
    clicks: usize,
) {
    use gpui::InputEvent as _;
    in_window(cx, window, |w, cx| {
        let at = w
            .find(gpui::ElementId::Name(format!("row-{id}").into()))
            .bounds()
            .center();
        let modifiers = gpui::Modifiers::default();
        w.dispatch_event(
            gpui::MouseDownEvent {
                button,
                position: at,
                modifiers,
                click_count: clicks,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        w.dispatch_event(
            gpui::MouseUpEvent {
                button,
                position: at,
                modifiers,
                click_count: clicks,
            }
            .to_platform_input(),
            cx,
        );
    });
    cx.run_until_parked();
}

/// Port of `double_click_opens_a_tab_and_reuses_an_existing_one` and
/// `middle_click_opens_in_background_without_marking_read`
/// (src/app/composer/tests/tabs_tests.rs; the list halves, the window
/// halves are `window::tab_tests`): a double click on a row asks for its
/// tab, a middle click for a background tab without opening or marking it
/// read, and neither on a Drafts row.
#[gpui_kit::test]
fn double_and_middle_clicks_ask_for_tabs(cx: &mut TestAppContext) {
    let (f, db) = fixture("row-clicks", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let (seen, _sub) = events(cx, &view);
    let tabs = |seen: &std::rc::Rc<std::cell::RefCell<Vec<super::MailEvent>>>| {
        seen.borrow()
            .iter()
            .filter_map(|e| match e {
                super::MailEvent::OpenTab { id, background } => Some((*id, *background)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    press_row(cx, window, ids[1], gpui::MouseButton::Middle, 1);
    assert_eq!(tabs(&seen), vec![(ids[1], true)]);
    assert_eq!(open_id(cx, &view), None, "not opened");
    assert!(!seen_now(&check, ids[1]), "not read");
    press_row(cx, window, ids[0], gpui::MouseButton::Left, 2);
    assert_eq!(tabs(&seen).last(), Some(&(ids[0], false)));
    // (The window opened that tab in front: back to the mail.)
    keys(cx, window, &["ctrl-1"]);

    // A Drafts row is no message: no tab.
    check
        .insert_draft(&komorebi::db::DraftRow {
            account_id: f.account,
            to: "ana@example.org".into(),
            subject: "draft".into(),
            body: "Hi".into(),
            ..Default::default()
        })
        .unwrap();
    act(cx, window, &view, |m| {
        m.load_folders();
        m.set_view(View::Drafts);
    });
    let draft = read(cx, &view, |m| m.rows().id_at(0)).unwrap();
    let before = tabs(&seen).len();
    press_row(cx, window, draft, gpui::MouseButton::Middle, 1);
    assert_eq!(tabs(&seen).len(), before);
    // Nor a double click on it (the Slint list's `selected-folder >= 0`).
    press_row(cx, window, draft, gpui::MouseButton::Left, 2);
    assert_eq!(tabs(&seen).len(), before, "a Drafts row: no tab");

    // An Outbox row is none either, by double or middle click.
    let queued = check
        .insert_draft(&komorebi::db::DraftRow {
            account_id: f.account,
            to: "ana@example.org".into(),
            subject: "queued".into(),
            body: "Hi".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(check.outbox_send(queued).unwrap());
    assert!(
        check
            .store_raw(queued, b"RAW", &std::sync::atomic::AtomicBool::new(false))
            .unwrap()
    );
    assert!(check.outbox_queue(queued).unwrap());
    if view.read_with(cx, |v, cx| v.composer().read(cx).is_open()) {
        // (The double click above opened the draft: close it.)
        keys(cx, window, &["escape"]);
    }
    act(cx, window, &view, |m| {
        m.load_folders();
        m.set_view(View::Outbox);
    });
    let row = read(cx, &view, |m| m.rows().id_at(0)).expect("the Outbox row");
    press_row(cx, window, row, gpui::MouseButton::Left, 2);
    press_row(cx, window, row, gpui::MouseButton::Middle, 1);
    assert_eq!(tabs(&seen).len(), before, "an Outbox row: no tab");
}

fn seen_now(db: &Db, id: i64) -> bool {
    seen(db, id)
}

/// Port of `a_right_press_on_a_row_selects_it_without_opening_it`: the
/// right press itself (the menu's builder half is
/// `right_click_selects_the_row_unless_it_is_selected`).
#[gpui_kit::test]
fn a_right_press_on_a_row_selects_it_without_opening_it(cx: &mut TestAppContext) {
    let (f, db) = fixture("right-press", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    in_window(cx, window, |w, cx| {
        let at = w
            .find(gpui::ElementId::Name(format!("row-{}", ids[2]).into()))
            .bounds()
            .center();
        gpui_kit::test::TestEventExt::pointer_down(w, at, gpui::MouseButton::Right, cx);
        gpui_kit::test::TestEventExt::pointer_up(w, at, gpui::MouseButton::Right, cx);
    });
    cx.run_until_parked();
    assert_eq!(open_id(cx, &view), None);
    assert_eq!(read(cx, &view, |m| m.selection().ids()), vec![ids[2]]);
    assert!(!seen(&check, ids[2]));
}

/// Port of `drafts_answer_enter_delete_and_their_menu`: in Drafts, Enter
/// opens the current draft in the composer, the row menu offers Open and
/// Discard, Delete asks first as Discard does (Cancel keeps it), and New
/// mail never brings a saved draft back.
#[gpui_kit::test]
fn drafts_answer_enter_delete_and_their_menu(cx: &mut TestAppContext) {
    let (f, db) = fixture("drafts-keys", &[]);
    let check = f.cache.open();
    for subject in ["older", "newer"] {
        db.insert_draft(&komorebi::db::DraftRow {
            account_id: f.account,
            to: "ana@example.org".into(),
            subject: subject.into(),
            body: "Hi".into(),
            message_id: format!("<{subject}@example.org>"),
            ..Default::default()
        })
        .unwrap();
    }
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| {
        m.load_folders();
        m.set_view(View::Drafts);
    });
    assert_eq!(read(cx, &view, |m| m.rows().len()), 2);
    let subject =
        |cx: &mut TestAppContext| view.read_with(cx, |v, cx| v.composer().read(cx).draft());
    focus_list(cx, window, &view);
    keys(cx, window, &["down", "down", "enter"]);
    let older = read(cx, &view, |m| m.rows().id_at(1)).unwrap();
    assert_eq!(subject(cx), Some(Draft::Saved(older)));
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.composer().update(cx, |c, cx| c.open(None, w, cx))
        })
    });
    // The row menu: Open and Discard.
    let first = read(cx, &view, |m| m.rows().id_at(0)).unwrap();
    act(cx, window, &view, |m| {
        m.row_context(first);
    });
    let labels: Vec<String> = view
        .read_with(cx, |v, _| super::menus::row_entries(v))
        .into_iter()
        .flatten()
        .map(|e| e.label)
        .collect();
    assert_eq!(labels, vec!["Open".to_string(), "Discard".to_string()]);
    // Delete asks first (the draft has words); Cancel keeps it.
    focus_list(cx, window, &view);
    keys(cx, window, &["delete"]);
    assert_eq!(subject(cx), Some(Draft::Saved(first)));
    assert!(in_window(cx, window, |w, cx| w.has_active_dialog(cx)));
    in_window(cx, window, |w, cx| w.click("dialog-cancel", cx));
    cx.run_until_parked();
    assert_eq!(check.count_drafts().unwrap(), 2);
    // New mail is a new message.
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| v.new_mail(w, cx))
    });
    assert_eq!(subject(cx), Some(Draft::New));
}

/// Port of `draft_rows_show_the_text_without_its_marks`: a draft's row
/// previews its text without the markup's marks.
#[gpui_kit::test]
fn draft_rows_show_the_text_without_its_marks(cx: &mut TestAppContext) {
    let (f, db) = fixture("draft-marks", &[]);
    db.insert_draft(&komorebi::db::DraftRow {
        account_id: f.account,
        to: "ana@example.org".into(),
        body: "The **report** is _ready_, 2 * 3".into(),
        ..Default::default()
    })
    .unwrap();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| {
        m.load_folders();
        m.set_view(View::Drafts);
    });
    let preview = read(cx, &view, |m| m.rows().row(0).map(|r| r.preview));
    assert_eq!(preview.as_deref(), Some("The report is ready, 2 * 3"));
}

/// Port of `a_message_opened_in_unread_stays_until_the_view_changes`: a
/// message opened in the Unread filter stays listed (read by now) through
/// a reload, until the filter changes.
#[gpui_kit::test]
fn a_message_opened_in_unread_stays_until_the_view_changes(cx: &mut TestAppContext) {
    let (f, db) = fixture("kept-unread", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.set_filter(Filter::Unread));
    let n = read(cx, &view, |m| m.rows().len());
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    cx.run_until_parked();
    assert!(seen(&check, ids[0]));
    act(cx, window, &view, Mail::reload_rows);
    assert!(read(cx, &view, |m| m.rows().index_of(ids[0]).is_some()));
    assert_eq!(read(cx, &view, |m| m.rows().len()), n);
    act(cx, window, &view, |m| m.set_filter(Filter::All));
    act(cx, window, &view, |m| m.set_filter(Filter::Unread));
    assert!(read(cx, &view, |m| m.rows().index_of(ids[0]).is_none()));
    assert_eq!(read(cx, &view, |m| m.rows().len()), n - 1);
}

/// Port of `message_actions_wait_while_composing`: with the composer open,
/// the message commands (Reply, Forward, read, important, Archive, Delete,
/// the arrows) leave the open message and the draft alone.
#[gpui_kit::test]
fn message_actions_wait_while_composing(cx: &mut TestAppContext) {
    let (f, db) = fixture("guard", &[]);
    let check = f.cache.open();
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| v.new_mail(w, cx))
    });
    for k in [
        "ctrl-r",
        "ctrl-shift-r",
        "ctrl-shift-f",
        "ctrl-u",
        "insert",
        "ctrl-shift-g",
        "delete",
        "down",
    ] {
        keys(cx, window, &[k]);
    }
    let draft = view.read_with(cx, |v, cx| v.composer().read(cx).draft());
    assert_eq!(draft, Some(Draft::New), "still the new message");
    let m = check.message(ids[0]).unwrap().expect("not deleted");
    assert!(m.seen && !m.flagged);
    assert!(read(cx, &view, |m| m.held().is_none()));
    // Closed, the same keys work again.
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.composer().update(cx, |c, cx| c.open(None, w, cx))
        })
    });
    focus_list(cx, window, &view);
    act(cx, window, &view, |m| m.row_clicked(ids[0]));
    keys(cx, window, &["ctrl-r"]);
    let draft = view.read_with(cx, |v, cx| v.composer().read(cx).draft());
    assert_eq!(
        draft,
        Some(Draft::Reply {
            message: ids[0],
            all: false
        })
    );
}

/// Port of `sends_cut_short_by_the_last_exit_go_out_at_start`: after the
/// first frame, a send left in its undo window by the last exit goes on:
/// one already built is queued (and handed to the send worker), one not
/// built is built again with no undo window, and one that cannot be built
/// (its attachment copy is gone) goes back to Drafts and the line says so.
#[gpui_kit::test]
fn sends_cut_short_by_the_last_exit_go_out_at_start(cx: &mut TestAppContext) {
    let (f, db) = fixture("resume", &[]);
    let draft = |subject: &str| {
        db.insert_draft(&komorebi::db::DraftRow {
            account_id: f.account,
            to: "ana@example.org".into(),
            subject: subject.into(),
            message_id: format!("<{subject}@example.org>"),
            ..Default::default()
        })
        .unwrap()
    };
    let built = draft("built");
    assert!(db.outbox_send(built).unwrap());
    assert!(
        db.store_raw(built, b"RAW", &std::sync::atomic::AtomicBool::new(false))
            .unwrap()
    );
    let unbuilt = draft("unbuilt");
    assert!(db.outbox_send(unbuilt).unwrap());
    let lost = draft("lost");
    let file = db
        .add_draft_file(lost, "a.txt", "text/plain", b"a")
        .unwrap();
    assert!(db.outbox_send(lost).unwrap());
    std::fs::remove_file(
        komorebi::db::drafts_dir(&f.cache.dir().join("cache.sqlite3"))
            .join(lost.to_string())
            .join(file.id.to_string()),
    )
    .unwrap();
    let check = f.cache.open();
    let (_window, view) = open(cx, db);
    cx.run_until_parked();
    let state = |row| check.outbox_entry(row).unwrap().map(|e| e.state);
    assert_eq!(state(built), Some(komorebi::db::Outbox::Queued));
    assert_eq!(state(lost), None);
    assert_eq!(check.draft_ids("").unwrap(), [lost]);
    let pending = view.read_with(cx, |v, cx| v.composer().read(cx).pending_sends());
    assert!(
        pending.contains(&unbuilt) || state(unbuilt) == Some(komorebi::db::Outbox::Queued),
        "the unbuilt one is built again"
    );
    let line = view.read_with(cx, |v, cx| {
        v.hub().read(cx).sync().line().map(|l| l.said.clone())
    });
    assert_eq!(line, Some(super::sync::Said::RebuildFailed));
    let hub = view.read_with(cx, |v, _| v.hub().clone());
    let jobs = hub.read_with(cx, |h, _| h.captured());
    assert!(
        jobs.iter()
            .any(|(_, j)| matches!(j, Job::Send { row } if *row == built)),
        "the built one went to the send worker"
    );
}

/// Slint parity: a message row dropped on a folder line moves there (the
/// selection when the row is in it); a folder it cannot move to is
/// ignored. The drag itself is GPUI's (`on_drag` and `on_drop`).
#[gpui_kit::test]
fn a_row_dropped_on_a_folder_moves_there(cx: &mut TestAppContext) {
    let (f, db) = fixture("drop-move", &[("Projects", FolderRole::Other)]);
    let ids = f.ids.clone();
    let (window, view) = open(cx, db);
    let projects = read(cx, &view, |m| {
        m.folders()
            .iter()
            .find(|f| f.display == "Projects")
            .unwrap()
            .id
    });
    let drop = |cx: &mut TestAppContext, row, folder| {
        in_window(cx, window, |w, cx| {
            view.update(cx, |v, cx| v.drop_on_folder(row, folder, w, cx))
        })
    };
    // Its own folder: nothing, and the selection stays as it was.
    act(cx, window, &view, |m| m.row_toggled(ids[3]));
    drop(cx, ids[0], f.inbox);
    assert!(read(cx, &view, |m| m.held().is_none()));
    assert_eq!(read(cx, &view, |m| m.selection().ids()), [ids[3]]);
    act(cx, window, &view, Mail::clear_selection);
    // Two selected, one dragged: both move.
    act(cx, window, &view, |m| m.row_toggled(ids[1]));
    act(cx, window, &view, |m| m.row_toggled(ids[2]));
    drop(cx, ids[2], projects);
    assert_eq!(held_ids(cx, &view), {
        let mut v = vec![ids[1], ids[2]];
        v.sort_unstable();
        v
    });
    // A row outside the selection moves alone, never opened.
    drop(cx, ids[4], projects);
    assert!(held_ids(cx, &view).contains(&ids[4]));
    assert_ne!(open_id(cx, &view), Some(ids[4]), "never opened");
}

/// The composer is a cached view in the reading pane (`reading::render`,
/// docs/PERF.md section 12): GPUI replays its last frame unless it is
/// notified, so every entity its render reads has to notify it. A key typed
/// in the subject field, an entity the composer only observes, draws it
/// again; a change elsewhere in the window leaves it replayed.
#[gpui_kit::test]
fn the_cached_composer_follows_the_entities_it_reads(cx: &mut TestAppContext) {
    use crate::views::composer;
    use komorebi::compose::Field;

    let (_f, db) = fixture("cached-composer", &[]);
    let (window, view) = open(cx, db);
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| v.new_mail(w, cx));
    });
    cx.run_until_parked();
    let first = composer::renders();
    assert!(first >= 1, "the composer drew");

    // A notify of the mail view (a hover, a status) draws the window around
    // the composer, not the composer.
    for _ in 0..3 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(composer::renders(), first, "the composer is replayed");

    // The subject field is read by the composer's render.
    let composer = view.read_with(cx, |v, _| v.composer().clone());
    in_window(cx, window, |w, cx| {
        composer.update(cx, |c, cx| c.focus_field(Field::Subject, w, cx));
    });
    cx.run_until_parked();
    let before = composer::renders();
    in_window(cx, window, |w, cx| test_support::type_text(w, "Hi", cx));
    cx.run_until_parked();
    assert!(
        composer::renders() > before,
        "typing in the subject draws the cached composer"
    );
}

// ---- the cached panes (`panes.rs`, docs/PERF.md section 12) ------------------

/// The names of the elements in the last frame the window drew (caches
/// honoured: no `render_frame`, which refreshes and ignores every cache).
fn drawn_names(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<String> {
    gpui::AppContext::update_window(cx, window, |_, w, _| {
        test_support::names(w)
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    })
    .expect("the test window is open")
}

/// A pane's own notify (the list's wheel, a row's hover) draws that pane
/// alone: the folder pane and the rest of the window are replayed.
#[gpui_kit::test]
fn a_notify_of_the_list_leaves_the_folder_pane_replayed(cx: &mut TestAppContext) {
    let (_f, db) = fixture("pane-list-notify", &[]);
    let (_window, view) = open(cx, db);
    cx.run_until_parked();
    let (folders, list) = super::panes::renders();
    assert!(folders >= 1 && list >= 1, "both panes drew");
    for _ in 0..3 {
        view.update(cx, |v, cx| v.notify_list(cx));
        cx.run_until_parked();
    }
    let (folders_after, list_after) = super::panes::renders();
    assert_eq!(folders_after, folders, "the folder pane is replayed");
    assert_eq!(list_after, list + 3, "the list draws each time");
}

/// A change of the mail view's state reaches both cached panes: the folder
/// pane (its unread counts, its pill) and the list (the selection) follow
/// what they read.
#[gpui_kit::test]
fn the_cached_panes_follow_the_mail_view(cx: &mut TestAppContext) {
    let (_f, db) = fixture("pane-follow", &[]);
    let (window, view) = open(cx, db);
    cx.run_until_parked();
    let (folders, list) = super::panes::renders();
    act(cx, window, &view, |m| m.select_offset(1));
    cx.run_until_parked();
    let (folders_after, list_after) = super::panes::renders();
    assert!(folders_after > folders, "the folder pane draws again");
    assert!(list_after > list, "the list draws again");
}

/// The language is a global the panes read: switching it shows in both
/// (the app refreshes every window, which bypasses the caches).
#[gpui_kit::test]
fn the_cached_panes_follow_the_language(cx: &mut TestAppContext) {
    let (_f, db) = fixture("pane-lang", &[]);
    let (window, _view) = open(cx, db);
    cx.run_until_parked();
    assert!(drawn_names(cx, window).contains(&"New mail".to_owned()));
    // As the Settings choice does: sets the language, refreshes the windows.
    cx.update(|cx| crate::i18n::set_language(komorebi::i18n::Lang::PtPt, cx));
    cx.run_until_parked();
    let names = drawn_names(cx, window);
    assert!(names.contains(&"Novo correio".to_owned()), "{names:?}");
    assert!(!names.contains(&"New mail".to_owned()));
}
