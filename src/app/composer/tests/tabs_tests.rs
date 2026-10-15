//! Tabs on the real app, headless (the `Harness` of `composer::tests`):
//! the folder pane's toggle, message tabs (opening, switching, closing,
//! reopening, moving, restoring) and composer tabs.

use super::*;
use crate::app::tabs::{Reopen, TabKind};

/// Two more messages in the Inbox ("Second", "Third"), unread; their ids.
fn more_messages(h: &Harness) -> (i64, i64) {
    let (account, folder) = h.app(|a| {
        let f = a.folders.iter().find(|f| f.role == Role::Inbox).unwrap();
        (f.account_id, f.id)
    });
    let mut db = Db::open(&h.path).unwrap();
    let header = |uid: u32, subject: &str| NewHeader {
        uid,
        msgid: format!("m{uid}@example.org"),
        subject: subject.into(),
        from_name: "Cy".into(),
        from_addr: "cy@example.org".into(),
        to: "me@example.org".into(),
        date: 1_700_000_000 + i64::from(uid),
        ..Default::default()
    };
    db.insert_headers(account, folder, &[header(2, "Second"), header(3, "Third")])
        .unwrap();
    let ids = db.message_ids(&[folder], "").unwrap();
    let find = |subject: &str| {
        ids.iter()
            .copied()
            .find(|&id| db.message(id).unwrap().unwrap().subject == subject)
            .unwrap()
    };
    let (second, third) = (find("Second"), find("Third"));
    for (id, text) in [(second, "Two"), (third, "Three")] {
        let raw = format!(
            "From: Cy <cy@example.org>\r\nTo: me@example.org\r\nSubject: x\r\n\r\n{text}\r\n"
        );
        db.set_body(id, raw.as_bytes(), text, false, Default::default())
            .unwrap();
    }
    h.app(|a| a.reload_rows());
    (second, third)
}

fn kinds(h: &Harness) -> Vec<TabKind> {
    h.app(|a| a.tabs.iter().map(|t| t.kind).collect())
}

fn active(h: &Harness) -> usize {
    h.app(|a| a.tabs.active())
}

fn seen(h: &Harness, id: i64) -> bool {
    Db::open(&h.path)
        .unwrap()
        .message(id)
        .unwrap()
        .unwrap()
        .seen
}

#[test]
fn nav_toggle_persists_and_shows_rail_when_wide() {
    let h = Harness::new("navfold");
    assert!(!h.ui().get_compact());
    assert!(!h.ui().get_rail() && !h.ui().get_nav_collapsed());
    h.keys(&[Key::Control, Key::Shift], "E");
    assert!(h.ui().get_nav_collapsed());
    assert!(h.ui().get_rail(), "the rail at 1280 px");
    assert!(!h.ui().get_pane_open());
    let db = Db::open(&h.path).unwrap();
    assert!(db.settings().unwrap().nav_collapsed);
    // The pane's menu says Show folders now, and runs the same toggle.
    let item = h
        .ui()
        .get_rail_items()
        .iter()
        .find(|r| !r.initial.is_empty())
        .expect("the account's rail tile");
    h.app(|a| {
        let (_, extra) = a.nav_menu(&item).unwrap();
        assert_eq!(extra.last().unwrap().title, "Show folders");
    });
    h.ui().invoke_toggle_pane();
    h.frame();
    assert!(!h.ui().get_nav_collapsed() && !h.ui().get_rail());
    assert!(!Db::open(&h.path).unwrap().settings().unwrap().nav_collapsed);
}

#[test]
fn double_click_opens_a_tab_and_reuses_an_existing_one() {
    let h = Harness::new("tabopen");
    let id = h.message;
    assert!(!h.ui().get_tabs_shown(), "one tab: no strip");
    h.ui().invoke_message_selected(id as i32);
    h.ui().invoke_open_tab(id as i32, false);
    h.frame();
    assert_eq!(kinds(&h), [TabKind::Mail, TabKind::Message(id)]);
    assert_eq!(active(&h), 1);
    assert!(h.ui().get_tabs_shown());
    assert_eq!(h.ui().get_tab_kind(), 1);
    assert!(!h.ui().get_show_list() && h.ui().get_show_reader());
    assert_eq!(h.ui().get_r_subject(), "Lunch");
    assert_eq!(h.ui().get_tabs().row_data(1).unwrap().title, "Lunch");
    // Again from the Mail tab: the same tab comes forward.
    h.keys(&[Key::Control], "1");
    assert_eq!(active(&h), 0);
    assert!(h.ui().get_show_list());
    h.ui().invoke_open_tab(id as i32, false);
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 2);
    assert_eq!(active(&h), 1);
    // Shift+Enter in the list does the same for the list's current row.
    h.keys(&[Key::Control], "1");
    h.ui().invoke_open_current_tab();
    h.frame();
    assert_eq!((h.app(|a| a.tabs.len()), active(&h)), (2, 1));
}

#[test]
fn middle_click_opens_in_background_without_marking_read() {
    let h = Harness::new("tabbg");
    let (second, _) = more_messages(&h);
    assert!(!seen(&h, second));
    h.ui().invoke_open_tab(second as i32, true);
    h.frame();
    assert_eq!(kinds(&h), [TabKind::Mail, TabKind::Message(second)]);
    assert_eq!(active(&h), 0, "in the background");
    assert!(!seen(&h, second));
    assert!(h.ui().get_tabs().row_data(1).unwrap().unread);
    // Shown: marked read as any opened message (at once here).
    h.ui().invoke_tab_activate(1);
    h.frame();
    assert!(seen(&h, second));
    assert!(!h.ui().get_tabs().row_data(1).unwrap().unread);
}

#[test]
fn tab_switch_restores_scroll_and_view_flags() {
    let h = Harness::new("tabview");
    let (second, third) = more_messages(&h);
    h.ui().invoke_open_tab(second as i32, false);
    h.frame();
    h.ui().set_r_plain(true);
    h.ui().invoke_open_tab(third as i32, false);
    h.frame();
    assert!(!h.ui().get_r_plain(), "one message's view");
    assert_eq!(h.ui().get_r_subject(), "Third");
    h.ui().invoke_tab_activate(1);
    h.frame();
    assert_eq!(h.ui().get_r_subject(), "Second");
    assert!(h.ui().get_r_plain(), "kept with its tab");
    // The Mail tab gets back what its pane showed (nothing here).
    h.ui().invoke_tab_activate(0);
    h.frame();
    assert!(!h.ui().get_has_message());
}

#[test]
fn ctrl_tab_moves_between_tabs_even_from_a_text_field() {
    let h = Harness::new("tabkeys");
    let (second, third) = more_messages(&h);
    h.ui().invoke_open_tab(second as i32, true);
    h.ui().invoke_open_tab(third as i32, true);
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 3);
    h.keys(&[Key::Control], "\t");
    assert_eq!(active(&h), 1);
    h.keys(&[Key::Control], "\t");
    assert_eq!(active(&h), 2);
    h.keys(&[Key::Control, Key::Shift], "\t");
    assert_eq!(active(&h), 1);
    h.keys(&[Key::Control], &SharedString::from(Key::PageDown));
    assert_eq!(active(&h), 2);
    h.keys(&[Key::Control], "9");
    assert_eq!(active(&h), 2);
    h.keys(&[Key::Control], "1");
    assert_eq!(active(&h), 0);
    // From the composer's To field too (a field would take Tab).
    h.ui().invoke_compose();
    h.frame();
    assert!(h.ui().get_composing());
    h.keys(&[Key::Control], "\t");
    assert_eq!(active(&h), 1);
    assert!(!h.ui().get_composing());
    h.keys(&[Key::Control, Key::Shift], "\t");
    assert_eq!(active(&h), 0);
    assert!(h.ui().get_composing(), "the pane's draft is back");
}

#[test]
fn close_tab_activates_left_neighbour_and_ctrl_shift_t_reopens() {
    let h = Harness::new("tabclose");
    let (second, third) = more_messages(&h);
    h.ui().invoke_open_tab(second as i32, false);
    h.ui().invoke_open_tab(third as i32, false);
    h.frame();
    assert_eq!(active(&h), 2);
    h.keys(&[Key::Control], "w");
    assert_eq!(kinds(&h), [TabKind::Mail, TabKind::Message(second)]);
    assert_eq!(active(&h), 1);
    assert_eq!(h.ui().get_r_subject(), "Second");
    // Esc in a message tab never closes it; Ctrl+W on the Mail tab does
    // nothing.
    h.keys(&[], &SharedString::from(Key::Escape));
    assert_eq!(h.app(|a| a.tabs.len()), 2);
    assert!(h.ui().get_has_message());
    h.keys(&[Key::Control, Key::Shift], "T");
    assert_eq!(
        kinds(&h),
        [
            TabKind::Mail,
            TabKind::Message(second),
            TabKind::Message(third)
        ]
    );
    assert_eq!(active(&h), 2);
    h.keys(&[Key::Control], "1");
    h.keys(&[Key::Control], "w");
    assert_eq!(h.app(|a| a.tabs.len()), 3);
    // Moving keeps the Mail tab first.
    h.ui().invoke_tab_activate(1);
    h.frame();
    h.keys(
        &[Key::Control, Key::Shift],
        &SharedString::from(Key::PageUp),
    );
    assert_eq!(kinds(&h)[0], TabKind::Mail);
    h.keys(
        &[Key::Control, Key::Shift],
        &SharedString::from(Key::PageDown),
    );
    assert_eq!(
        kinds(&h),
        [
            TabKind::Mail,
            TabKind::Message(third),
            TabKind::Message(second)
        ]
    );
    assert_eq!(active(&h), 2);
}

#[test]
fn archive_from_tab_closes_it_and_undo_reopens_at_index() {
    let h = Harness::new("tabarchive");
    let (second, third) = more_messages(&h);
    h.app(|a| {
        let account = a.accounts[0].id;
        let db = Db::open(&a.db_path).unwrap();
        db.upsert_folder(account, "Archive", "Archive", 0, Role::Archive)
            .unwrap();
        a.load_accounts().unwrap();
    });
    h.ui().invoke_open_tab(second as i32, false);
    h.ui().invoke_open_tab(third as i32, false);
    h.ui().invoke_tab_activate(1);
    h.frame();
    assert_eq!(h.ui().get_r_subject(), "Second");
    h.ui().invoke_archive();
    h.frame();
    assert_eq!(kinds(&h), [TabKind::Mail, TabKind::Message(third)]);
    assert_eq!(active(&h), 0, "the tab to its left");
    assert!(!h.undo().is_empty());
    h.ui().invoke_undo_clicked();
    h.frame();
    assert_eq!(
        kinds(&h),
        [
            TabKind::Mail,
            TabKind::Message(second),
            TabKind::Message(third)
        ]
    );
    assert_eq!(active(&h), 1);
    assert_eq!(h.ui().get_r_subject(), "Second");
}

#[test]
fn server_removal_marks_tab_gone_and_disables_actions() {
    let h = Harness::new("tabgone");
    let (second, _) = more_messages(&h);
    h.ui().invoke_open_tab(second as i32, false);
    h.frame();
    Db::open(&h.path)
        .unwrap()
        .delete_messages(&[second])
        .unwrap();
    h.app(|a| a.refresh_message_tabs());
    assert!(h.ui().get_r_gone());
    assert!(h.ui().get_tabs().row_data(1).unwrap().gone);
    // Still readable; its actions do nothing.
    assert_eq!(h.ui().get_r_subject(), "Second");
    h.ui().invoke_archive();
    h.ui().invoke_reply();
    h.frame();
    assert!(!h.ui().get_composing());
    assert_eq!(h.app(|a| a.tabs.len()), 2);
}

#[test]
fn removed_message_in_background_tab_shows_warning_on_return() {
    let h = Harness::new("tabgone2");
    let (second, _) = more_messages(&h);
    h.ui().invoke_open_tab(second as i32, false);
    h.frame();
    h.app(|a| a.go_to_mail_tab());
    Db::open(&h.path)
        .unwrap()
        .delete_messages(&[second])
        .unwrap();
    h.app(|a| a.refresh_message_tabs());
    h.app(|a| a.activate_tab(1));
    h.frame();
    assert!(h.ui().get_r_gone());
    assert!(h.ui().get_tabs().row_data(1).unwrap().gone);
    h.ui().invoke_archive();
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 2);
    // Closing it is safe.
    h.app(|a| a.close_tab(1));
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 1);
}

#[test]
fn tabs_setting_restores_skipping_missing_ids() {
    let h = Harness::new("tabrestore");
    let (second, third) = more_messages(&h);
    h.ui().invoke_open_tab(second as i32, false);
    h.ui().invoke_open_tab(third as i32, false);
    h.frame();
    h.app(|a| a.save_tabs());
    let saved = Db::open(&h.path).unwrap().settings().unwrap().tabs;
    assert_eq!(saved, format!("m{second},m{third};a=2"));
    assert!(!saved.contains("Second") && !saved.contains('@'));
    // A fresh session over the same cache, one message gone since.
    Db::open(&h.path)
        .unwrap()
        .delete_messages(&[second])
        .unwrap();
    h.app(|a| {
        a.tabs = crate::app::tabs::Tabs::default();
        a.restore_tabs(&format!("m{second},m{third},m999999;a=2"));
    });
    assert_eq!(kinds(&h), [TabKind::Mail, TabKind::Message(third)]);
    assert_eq!(active(&h), 1);
    assert_eq!(h.ui().get_r_subject(), "Third");
    // "Reopen tabs at start" off: the setting is cleared.
    h.app(|a| {
        a.reopen_tabs = false;
        a.save_tabs();
    });
    assert_eq!(Db::open(&h.path).unwrap().settings().unwrap().tabs, "");
    let _ = Reopen::Message(1);
}

#[test]
fn selecting_a_message_moves_a_dirty_pane_draft_to_a_tab() {
    let h = Harness::new("tabpane");
    h.ui().invoke_compose();
    h.frame();
    type_into(h.ui(), ComposeField::Subject, "Plans");
    type_into(h.ui(), ComposeField::Body, "Hello");
    h.ui().invoke_message_selected(h.message as i32);
    h.frame();
    assert!(!h.ui().get_composing());
    assert_eq!(h.status(), "Draft kept in a tab");
    let k = kinds(&h);
    assert_eq!(k.len(), 2);
    assert!(matches!(k[1], TabKind::Draft(_)));
    assert_eq!(active(&h), 0);
    assert_eq!(h.ui().get_tabs().row_data(1).unwrap().title, "Plans");
    // Saved already, so a restart would have it.
    assert_eq!(Db::open(&h.path).unwrap().count_drafts().unwrap(), 1);
    // An empty draft is dropped, as before (no tab).
    h.ui().invoke_compose();
    h.frame();
    h.ui().invoke_message_selected(h.message as i32);
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 2);
    // Its tab shows it again.
    h.ui().invoke_tab_activate(1);
    h.frame();
    assert!(h.ui().get_composing());
    assert_eq!(h.ui().get_c_subject(), "Plans");
    assert_eq!(h.ui().get_c_body(), "Hello");
    assert_eq!(h.ui().get_tab_kind(), 2);
}

#[test]
fn closing_a_draft_tab_saves_it_and_an_empty_one_goes() {
    let h = Harness::new("tabdraftclose");
    h.ui().invoke_open_tab(h.message as i32, false);
    h.frame();
    // New mail from a message tab: a composer tab to its right.
    h.ui().invoke_compose();
    h.frame();
    assert!(h.ui().get_composing());
    assert!(matches!(kinds(&h)[2], TabKind::Draft(_)));
    assert_eq!(active(&h), 2);
    assert_eq!(h.ui().get_tabs().row_data(2).unwrap().title, "New message");
    type_into(h.ui(), ComposeField::Subject, "Kept");
    assert_eq!(h.ui().get_tabs().row_data(2).unwrap().title, "Kept");
    // Esc closes the composer tab, saving the draft.
    h.ui().invoke_c_close();
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 2);
    assert_eq!(active(&h), 1);
    assert_eq!(Db::open(&h.path).unwrap().count_drafts().unwrap(), 1);
    // Ctrl+Shift+T brings it back.
    h.keys(&[Key::Control, Key::Shift], "T");
    assert!(h.ui().get_composing());
    assert_eq!(h.ui().get_c_subject(), "Kept");
    // A reply from a message tab: a composer tab too; untouched, closing
    // it drops it.
    h.ui().invoke_tab_activate(1);
    h.frame();
    h.ui().invoke_reply();
    h.frame();
    assert_eq!(h.ui().get_c_title(), "Reply");
    let n = h.app(|a| a.drafts.len());
    h.keys(&[Key::Control], "w");
    assert_eq!(h.app(|a| a.drafts.len()), n - 1);
    assert!(!h.ui().get_composing());
}

#[test]
fn tab_switch_flushes_autosave_of_the_leaving_draft() {
    let h = Harness::new("tabflush");
    h.ui().invoke_open_tab(h.message as i32, false);
    h.ui().invoke_compose();
    h.frame();
    type_into(h.ui(), ComposeField::To, "ana@example.org");
    type_into(h.ui(), ComposeField::Body, "Typed just now");
    assert_eq!(
        Db::open(&h.path).unwrap().count_drafts().unwrap(),
        0,
        "the autosave waits"
    );
    h.keys(&[Key::Control], "1");
    let db = Db::open(&h.path).unwrap();
    let rows = db.draft_ids("").unwrap();
    assert_eq!(rows.len(), 1, "saved as the tab was left");
    assert!(
        db.draft(rows[0])
            .unwrap()
            .unwrap()
            .body
            .contains("Typed just now")
    );
}

#[test]
fn undo_send_reopens_in_a_new_tab() {
    let h = Harness::new("tabundo");
    let _jobs = hold_builds(&h);
    h.ui().invoke_open_tab(h.message as i32, false);
    h.frame();
    write_message(&h);
    assert_eq!(active(&h), 2);
    h.ui().invoke_send();
    h.frame();
    // Sent from its tab: the tab went.
    assert_eq!(h.app(|a| a.tabs.len()), 2);
    assert_eq!(active(&h), 1);
    assert!(!h.ui().get_composing());
    h.ui().invoke_undo_clicked();
    h.frame();
    assert_eq!(h.app(|a| a.tabs.len()), 3);
    assert_eq!(active(&h), 2);
    assert!(h.ui().get_composing());
    assert_eq!(h.ui().get_c_subject(), "Plans");
}

/// The harness's account gets an Archive folder.
fn with_archive(h: &Harness) {
    h.app(|a| {
        let account = a.accounts[0].id;
        let db = Db::open(&a.db_path).unwrap();
        db.upsert_folder(account, "Archive", "Archive", 0, Role::Archive)
            .unwrap();
        a.load_accounts().unwrap();
    });
}

fn popout(h: &Harness, i: usize) -> AppWindow {
    h.app(|a| a.popouts.window(i)).expect("a message window")
}

#[test]
fn popout_has_own_reader_and_closes_when_message_archived_from_main() {
    let h = Harness::new("popout");
    with_archive(&h);
    let (second, third) = more_messages(&h);
    // Ctrl+O on the list's message: a window of its own.
    h.ui().invoke_message_selected(second as i32);
    h.frame();
    h.keys(&[Key::Control], "o");
    assert_eq!(h.app(|a| a.popouts.messages()), [second]);
    let w = popout(&h, 0);
    assert!(w.get_detail_only() && w.get_has_message());
    assert_eq!(w.get_r_subject(), "Second");
    assert_eq!(w.get_window_title(), "Second - Komorebi");
    assert!(!w.get_show_list() && w.get_show_reader());
    assert_eq!(w.get_r_body().trim(), "Two");
    // The main window still shows its own message.
    assert_eq!(h.ui().get_r_subject(), "Second");
    h.ui().invoke_message_selected(third as i32);
    h.frame();
    assert_eq!(h.ui().get_r_subject(), "Third");
    assert_eq!(w.get_r_subject(), "Second", "its own reader");
    // A render result for a window that closed is dropped.
    h.app(|a| {
        a.popout_render(
            999,
            reader::Reply::Failed {
                generation: 1,
                error: crate::mail::render::RenderError::Malformed,
            },
        )
    });
    // Archived from the main window: its window closes; Undo says it
    // does not come back.
    h.ui().invoke_message_selected(second as i32);
    h.frame();
    h.ui().invoke_archive();
    h.frame();
    assert_eq!(h.app(|a| a.popouts.len()), 0);
    h.ui().invoke_undo_clicked();
    h.frame();
    assert_eq!(
        h.status(),
        "Undo brought the message back; open it from the list."
    );
}

#[test]
fn popout_actions_and_replies_go_to_the_main_window() {
    let h = Harness::new("popoutact");
    with_archive(&h);
    let (second, third) = more_messages(&h);
    h.app(|a| a.open_window(second));
    h.app(|a| a.open_window(third));
    assert_eq!(h.app(|a| a.popouts.len()), 2);
    let w = popout(&h, 0);
    // The same message again: its window comes forward, no new one.
    h.app(|a| a.open_window(second));
    assert_eq!(h.app(|a| a.popouts.len()), 2);
    // Flags reach every view of the message.
    h.ui().invoke_open_tab(second as i32, false);
    h.frame();
    assert!(!w.get_r_flagged());
    h.ui().invoke_toggle_flag();
    h.frame();
    assert!(w.get_r_flagged() && h.ui().get_r_flagged());
    w.invoke_mark_flagged(false);
    h.frame();
    assert!(!h.ui().get_r_flagged());
    // Reply in the window: a composer tab in the main window.
    w.invoke_reply();
    h.frame();
    assert!(h.ui().get_composing());
    assert_eq!(h.ui().get_c_subject(), "Re: Second");
    assert_eq!(h.app(|a| a.tabs.active_kind().code()), 2);
    // Archive in the window: it closes, and the message's tab with it.
    w.invoke_archive();
    h.frame();
    assert_eq!(h.app(|a| a.popouts.messages()), [third]);
    assert!(h.app(|a| a.tabs.find_message(second)).is_none());
    // Esc closes a window.
    let w2 = popout(&h, 0);
    w2.invoke_close_window();
    h.frame();
    assert_eq!(h.app(|a| a.popouts.len()), 0);
}

#[test]
fn main_close_closes_popouts_then_flushes() {
    let h = Harness::new("popoutclose");
    let (second, _) = more_messages(&h);
    h.app(|a| a.open_window(second));
    h.ui().invoke_compose();
    h.frame();
    type_into(h.ui(), ComposeField::To, "ana@example.org");
    type_into(h.ui(), ComposeField::Body, "Typed just now");
    h.ui().invoke_open_tab(second as i32, true);
    h.frame();
    h.ui().window().dispatch_event(WindowEvent::CloseRequested);
    assert_eq!(h.app(|a| a.popouts.len()), 0);
    let db = Db::open(&h.path).unwrap();
    assert_eq!(db.draft_ids("").unwrap().len(), 1);
    assert_eq!(db.settings().unwrap().tabs, format!("m{second};a=0"));
}

#[test]
fn language_and_theme_switch_reach_popouts() {
    let h = Harness::new("popouttheme");
    let (second, _) = more_messages(&h);
    h.app(|a| a.open_window(second));
    let w = popout(&h, 0);
    let dark = |ui: &AppWindow| {
        ui.global::<crate::Palette>().get_color_scheme() == slint::language::ColorScheme::Dark
    };
    h.app(|a| a.general_changed(0, 2));
    assert!(dark(h.ui()) && dark(&w));
    h.app(|a| a.general_changed(0, 1));
    assert!(!dark(h.ui()) && !dark(&w));
    let date = w.get_r_date();
    h.app(|a| a.general_changed(6, 2));
    assert_ne!(w.get_r_date(), date, "the date in Portuguese");
    assert_eq!(w.get_window_title(), "Second - Komorebi");
    h.app(|a| a.general_changed(6, 1));
}

#[test]
fn delete_in_a_message_tab_never_acts_on_a_draft_row() {
    let h = Harness::new("tabdelete");
    // A saved draft, its row listed in the Mail tab's Drafts view.
    h.ui().invoke_compose();
    h.frame();
    type_into(h.ui(), ComposeField::To, "ana@example.org");
    h.ui().invoke_c_close();
    h.frame();
    let id = h.message;
    h.ui().invoke_open_tab(id as i32, false);
    h.frame();
    h.app(|a| {
        a.activate_tab(0);
        a.set_view(View::Drafts);
        a.activate_tab(1);
    });
    assert_eq!(h.ui().get_tab_kind(), 1);
    // Delete acts on the tab's message (here it has no Trash: nothing
    // moves), never on a draft.
    h.ui().invoke_delete();
    h.frame();
    assert!(!h.ui().get_composing());
    assert!(!h.ui().get_c_confirm_discard());
    assert_eq!(Db::open(&h.path).unwrap().count_drafts().unwrap(), 1);
}
