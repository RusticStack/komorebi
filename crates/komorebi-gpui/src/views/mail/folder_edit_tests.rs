//! Folders made, renamed and deleted from the pane: the name's checks and
//! the jobs (plain tests on [`Mail`]), then the window: the account line's
//! New folder button, the menus, the field typed in the tree (Enter, Esc,
//! a name that can't be used said under it), F2, Delete and Ctrl+Shift+N.

use gpui::{AnyWindowHandle, Entity, Role, TestAppContext};
use komorebi::db::Db;
use komorebi::mail::Job;
use komorebi::mail::folders::BadName;
use komorebi::role::Role as FolderRole;
use komorebi::tree::Fold;

use super::MailView;
use super::folder_edit::{FolderMenu, NameProblem, Target, delete_question};
use super::model::Mail;
use super::nav::NavKind;
use crate::test_support::{self, TempCache, assert_named, in_window, press, type_text};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

/// The seed account (Inbox, Sent, Archive) with Work, Work/2024 and Notes.
fn cache(name: &str) -> (TempCache, Db, i64) {
    let (cache, db, seeded) = test_support::seeded(name, 0);
    let a = seeded.account;
    db.upsert_folder(a, "Work", "Work", 0, FolderRole::Other)
        .unwrap();
    db.upsert_folder(a, "Work/2024", "2024", 1, FolderRole::Other)
        .unwrap();
    db.upsert_folder(a, "Notes", "Notes", 0, FolderRole::Other)
        .unwrap();
    (cache, db, a)
}

fn id(m: &Mail, name: &str) -> i64 {
    m.folders()
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no folder {name}"))
        .id
}

#[test]
fn names_are_checked_beside_their_siblings() {
    let (cache, _db, account) = cache("folder-names");
    let mut m = Mail::new(std::rc::Rc::new(cache.open()));
    let top = Target::New {
        account,
        parent: None,
    };
    let under_work = Target::New {
        account,
        parent: Some(id(&m, "Work")),
    };
    assert_eq!(m.name_problem(top, "work"), Some(NameProblem::Taken));
    assert_eq!(m.name_problem(top, "inbox"), Some(NameProblem::Taken));
    assert_eq!(m.name_problem(top, "2024"), None);
    assert_eq!(m.name_problem(under_work, "2024"), Some(NameProblem::Taken));
    assert_eq!(m.name_problem(under_work, "Notes"), None);
    assert_eq!(
        m.name_problem(top, "a/b"),
        Some(NameProblem::Bad(BadName::Delimiter("/".into())))
    );
    assert_eq!(
        m.name_problem(top, "  "),
        Some(NameProblem::Bad(BadName::Empty))
    );
    assert_eq!(
        m.name_problem(top, "a*"),
        Some(NameProblem::Bad(BadName::Character('*')))
    );
    // A rename may keep its own name (nothing is sent), not a sibling's.
    let work = Target::Rename(id(&m, "Work"));
    assert_eq!(m.name_problem(work, "Work"), None);
    assert_eq!(m.name_problem(work, "notes"), Some(NameProblem::Taken));
    assert_eq!(m.commit_folder(work, "Work"), Ok(false));
    assert!(m.take_jobs().is_empty());
    assert_eq!(m.commit_folder(work, "Jobs"), Ok(true));
    assert_eq!(m.commit_folder(top, " Receipts "), Ok(true));
    let jobs = m.take_jobs();
    assert!(matches!(
        &jobs[..],
        [
            (a, Job::RenameFolder { folder_id, name: renamed }),
            (b, Job::CreateFolder { parent: None, name: made }),
        ] if *a == account && *b == account && *folder_id == id(&m, "Work")
            && renamed == "Jobs" && made == "Receipts"
    ));
}

#[test]
fn special_folders_and_accounts_switched_off_offer_no_change() {
    let (cache, db, account) = cache("folder-special");
    let mut m = Mail::new(std::rc::Rc::new(cache.open()));
    let (inbox, sent, work) = (id(&m, "INBOX"), id(&m, "Sent"), id(&m, "Work"));
    let both = FolderMenu {
        new: true,
        change: true,
    };
    let new_only = FolderMenu {
        new: true,
        change: false,
    };
    assert_eq!(m.folder_menu(NavKind::Folder(inbox)), Some(new_only));
    assert_eq!(m.folder_menu(NavKind::Folder(sent)), Some(new_only));
    assert_eq!(m.folder_menu(NavKind::Folder(work)), Some(both));
    assert_eq!(m.folder_menu(NavKind::Account(account)), Some(new_only));
    assert_eq!(m.folder_menu(NavKind::AllInboxes), None);
    assert_eq!(m.folder_menu(NavKind::Drafts), None);
    m.delete_folder(sent);
    assert!(
        m.take_jobs().is_empty(),
        "a special folder is never deleted"
    );
    m.delete_folder(work);
    assert!(matches!(
        &m.take_jobs()[..],
        [(a, Job::DeleteFolder { folder_id })] if *a == account && *folder_id == work
    ));
    // Work holds Work/2024: both go, with what the cache holds of them.
    assert_eq!(m.folder_contents(work), (0, 1));
    db.set_account_enabled(account, false).unwrap();
    m.load_folders();
    assert_eq!(m.folder_menu(NavKind::Account(account)), None);
    assert_eq!(m.folder_menu(NavKind::Folder(work)), None);
    assert_eq!(m.new_folder_account(None), None);
}

#[test]
fn the_delete_question_says_what_goes() {
    let (title, message) = delete_question("Work", 3, 1);
    assert_eq!(title, "Delete \u{201c}Work\u{201d}?");
    assert_eq!(
        message,
        "The folder and the 3 messages in it are deleted on the server. Its 1 subfolder goes with it. Messages in it don't go to Trash and can't be brought back."
    );
    let (_, empty) = delete_question("Old", 0, 0);
    assert_eq!(
        empty,
        "The folder is deleted on the server. Messages in it don't go to Trash and can't be brought back."
    );
}

// ------------------------------------------------------------ the window

fn open(cx: &mut TestAppContext, db: Db) -> (AnyWindowHandle, Entity<MailView>) {
    test_support::init_app(cx);
    let (window, main) = test_support::open_main(cx, test_support::init(Some(db)));
    let mail = main.read_with(cx, |w, _| w.mail().cloned().expect("the mail view"));
    (window, mail)
}

fn menu(cx: &mut TestAppContext, view: &Entity<MailView>, kind: NavKind) -> Vec<String> {
    view.read_with(cx, |v, _| super::menus::nav_entries(v, kind, false))
        .into_iter()
        .flatten()
        .map(|e| e.label)
        .collect()
}

fn editing(cx: &mut TestAppContext, view: &Entity<MailView>) -> Option<Target> {
    view.read_with(cx, |v, _| v.folder_edit.as_ref().map(|e| e.target))
}

fn jobs(cx: &mut TestAppContext, view: &Entity<MailView>) -> Vec<(i64, Job)> {
    let hub = view.read_with(cx, |v, _| v.hub().clone());
    hub.read_with(cx, |h, _| h.captured())
}

/// The folder pane holds the keyboard, on the line named `label`.
fn on_line(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    view: &Entity<MailView>,
    kind: NavKind,
) {
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            let at = v.mail().nav().items.iter().position(|i| i.kind == kind);
            v.nav_cursor = at.expect("the line");
            w.focus(&v.nav_focus, cx);
        });
        w.render_frame(cx);
    });
}

#[gpui_kit::test]
fn the_menus_offer_new_rename_and_delete_but_never_on_special_folders(cx: &mut TestAppContext) {
    let (_cache, db, account) = cache("folder-menus");
    let (_window, view) = open(cx, db);
    let (inbox, work) = view.read_with(cx, |v, _| (id(v.mail(), "INBOX"), id(v.mail(), "Work")));
    let account_menu = menu(cx, &view, NavKind::Account(account));
    assert!(account_menu.contains(&"New folder".to_string()));
    let work_menu = menu(cx, &view, NavKind::Folder(work));
    for label in ["New subfolder", "Rename", "Delete folder"] {
        assert!(work_menu.contains(&label.to_string()), "{label} on Work");
    }
    let inbox_menu = menu(cx, &view, NavKind::Folder(inbox));
    assert!(inbox_menu.contains(&"New subfolder".to_string()));
    assert!(!inbox_menu.contains(&"Rename".to_string()));
    assert!(!inbox_menu.contains(&"Delete folder".to_string()));
    // The room under the lines: New folder.
    let room: Vec<String> = view
        .read_with(cx, |v, _| super::menus::pane_entries(v))
        .into_iter()
        .flatten()
        .map(|e| e.label)
        .collect();
    assert_eq!(room, ["New folder"]);
}

#[gpui_kit::test]
fn the_account_line_has_a_named_new_folder_button(cx: &mut TestAppContext) {
    let (_cache, db, account) = cache("folder-plus");
    let (window, view) = open(cx, db);
    let email = view.read_with(cx, |v, _| {
        Mail::account_label(&v.mail().accounts()[0]).to_string()
    });
    let name = format!("New folder in {email}");
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find_by_label(&name).role(), Some(Role::Button));
    });
    in_window(cx, window, |w, cx| {
        test_support::click_named(w, &name, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        editing(cx, &view),
        Some(Target::New {
            account,
            parent: None
        })
    );
    in_window(cx, window, |w, _| {
        assert_named(w, "New folder name", Role::TextInput);
    });
}

#[gpui_kit::test]
fn a_new_folder_is_typed_in_place_and_enter_makes_it(cx: &mut TestAppContext) {
    let (_cache, db, account) = cache("folder-new");
    let (window, view) = open(cx, db);
    let _ = jobs(cx, &view);
    on_line(cx, window, &view, NavKind::Account(account));
    in_window(cx, window, |w, cx| press(w, &["ctrl-shift-n"], cx));
    cx.run_until_parked();
    assert_eq!(
        editing(cx, &view),
        Some(Target::New {
            account,
            parent: None
        })
    );
    // A name already beside it is said under the field, and Enter keeps
    // the field open.
    in_window(cx, window, |w, cx| type_text(w, "notes", cx));
    cx.run_until_parked();
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
        assert_named(w, &NameProblem::Taken.text(), Role::Alert);
        press(w, &["enter"], cx);
    });
    cx.run_until_parked();
    assert!(editing(cx, &view).is_some());
    assert!(jobs(cx, &view).is_empty());
    // Space, Home and End are the text's while the field is open.
    in_window(cx, window, |w, cx| {
        press(w, &["ctrl-a", "backspace"], cx);
        type_text(w, "Tax", cx);
        press(w, &["space"], cx);
        type_text(w, "2025", cx);
        press(w, &["enter"], cx);
    });
    cx.run_until_parked();
    assert_eq!(editing(cx, &view), None);
    let sent = jobs(cx, &view);
    assert!(
        matches!(
            &sent[..],
            [(a, Job::CreateFolder { parent: None, name })] if *a == account && name == "Tax 2025"
        ),
        "one CreateFolder"
    );
}

#[gpui_kit::test]
fn f2_renames_in_place_esc_leaves_it_and_delete_asks(cx: &mut TestAppContext) {
    let (_cache, db, _account) = cache("folder-rename");
    let (window, view) = open(cx, db);
    let _ = jobs(cx, &view);
    let (work, sent) = view.read_with(cx, |v, _| (id(v.mail(), "Work"), id(v.mail(), "Sent")));
    on_line(cx, window, &view, NavKind::Folder(work));
    in_window(cx, window, |w, cx| press(w, &["f2"], cx));
    cx.run_until_parked();
    assert_eq!(editing(cx, &view), Some(Target::Rename(work)));
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
        assert_named(w, "Folder name", Role::TextInput);
        type_text(w, "Jobs", cx);
        press(w, &["escape"], cx);
    });
    cx.run_until_parked();
    assert_eq!(editing(cx, &view), None, "Esc leaves the name as it was");
    assert!(jobs(cx, &view).is_empty());

    in_window(cx, window, |w, cx| press(w, &["f2"], cx));
    cx.run_until_parked();
    in_window(cx, window, |w, cx| {
        type_text(w, "Jobs", cx);
        press(w, &["enter"], cx);
    });
    cx.run_until_parked();
    assert!(matches!(
        &jobs(cx, &view)[..],
        [(_, Job::RenameFolder { folder_id, name })] if *folder_id == work && name == "Jobs"
    ));

    // Delete asks first and says what goes; Cancel keeps it.
    on_line(cx, window, &view, NavKind::Folder(work));
    in_window(cx, window, |w, cx| press(w, &["delete"], cx));
    cx.run_until_parked();
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
        assert_named(w, "Delete \u{201c}Work\u{201d}?", Role::AlertDialog);
        test_support::click_named(w, "Cancel", cx);
    });
    cx.run_until_parked();
    assert!(jobs(cx, &view).is_empty());
    on_line(cx, window, &view, NavKind::Folder(work));
    in_window(cx, window, |w, cx| press(w, &["delete"], cx));
    cx.run_until_parked();
    in_window(cx, window, |w, cx| {
        test_support::click_named(w, "Delete folder", cx);
    });
    cx.run_until_parked();
    assert!(matches!(
        &jobs(cx, &view)[..],
        [(_, Job::DeleteFolder { folder_id })] if *folder_id == work
    ));

    // On a special folder F2 and Delete do nothing.
    on_line(cx, window, &view, NavKind::Folder(sent));
    in_window(cx, window, |w, cx| press(w, &["f2", "delete"], cx));
    cx.run_until_parked();
    assert_eq!(editing(cx, &view), None);
    assert!(jobs(cx, &view).is_empty());
}

#[gpui_kit::test]
fn a_new_subfolder_opens_its_folded_parent(cx: &mut TestAppContext) {
    let (_cache, db, _account) = cache("folder-sub");
    let (window, view) = open(cx, db);
    let work = view.read_with(cx, |v, _| id(v.mail(), "Work"));
    in_window(cx, window, |w, cx| {
        view.update(cx, |v, cx| {
            v.act(w, cx, |m| {
                if !m.is_folded(Fold::Folder(work)) {
                    m.nav_toggle(NavKind::Folder(work));
                }
            });
            v.run(
                super::menus::Run::Nav(NavKind::Folder(work), super::menus::NavRun::NewFolder),
                w,
                cx,
            );
        });
    });
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.mail().is_folded(Fold::Folder(work))));
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
        assert_named(w, "New subfolder name", Role::TextInput);
    });
}

/// The account line shows the chosen image; without one (and no provider
/// for a `.invalid` address) it has no icon slot at all.
#[gpui_kit::test]
fn the_account_line_shows_its_icon_or_nothing(cx: &mut TestAppContext) {
    use komorebi::favicon;
    let (_cache, db, account) = cache("folder-icon");
    let px: Vec<u8> = (0..16 * 16).flat_map(|_| [30u8, 110, 216, 255]).collect();
    favicon::choose(&db, account, &favicon::encode_png(&px, 16, 16), 1).unwrap();
    let (window, view) = open(cx, db);
    assert!(view.read_with(cx, |v, _| v.account_look(account).is_some()));
    in_window(cx, window, |w, cx| {
        w.render_frame(cx);
    });
    view.update(cx, |v, _| {
        favicon::remove(v.mail.db(), account).unwrap();
        v.reload_icons();
        assert!(v.account_look(account).is_none());
    });
}
