//! The Drafts list's Ctrl+O (Gaps integration, "Still open"): a draft from
//! the list opens in a composer window of its own, by the draft-window path
//! the composers' Ctrl+O and More use. On a throwaway seeded cache.

use gpui::{AnyWindowHandle, Entity, TestAppContext};
use komorebi::db::DraftRow;

use super::MainWindow;
use crate::test_support::{self, in_window, press};
use crate::views::composer::ComposerView;
use crate::views::mail::MailView;
use crate::views::mail::model::View;

struct Rig {
    _cache: test_support::TempCache,
    db: komorebi::db::Db,
    window: AnyWindowHandle,
    view: Entity<MainWindow>,
    mail: Entity<MailView>,
}

fn open(name: &str, cx: &mut TestAppContext) -> Rig {
    let (cache, db, _) = test_support::seeded(name, 2);
    test_support::init_app(cx);
    let (window, view) = test_support::open_main(cx, test_support::init(Some(cache.open())));
    let mail = view.read_with(cx, |v, _| v.mail().cloned().expect("the mail view"));
    Rig {
        _cache: cache,
        db,
        window,
        view,
        mail,
    }
}

impl Rig {
    fn draft(&self, subject: &str) -> i64 {
        let account = self.db.accounts().expect("accounts")[0].id;
        self.db
            .insert_draft(&DraftRow {
                account_id: account,
                to: "ana@example.org".into(),
                subject: subject.into(),
                body: "See you on Friday".into(),
                ..Default::default()
            })
            .expect("a draft")
    }

    fn act(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut crate::views::mail::model::Mail)) {
        let mail = self.mail.clone();
        in_window(cx, self.window, move |w, cx| {
            mail.update(cx, |m, cx| {
                m.act(w, cx, |m| {
                    f(m);
                })
            })
        });
        cx.run_until_parked();
    }

    /// Ctrl+O with the list holding the keys.
    fn ctrl_o(&self, cx: &mut TestAppContext) {
        let focus = self.mail.read_with(cx, |m, _| m.list_focus().clone());
        in_window(cx, self.window, |w, cx| {
            w.focus(&focus, cx);
            press(w, &["ctrl-o"], cx);
        });
        cx.run_until_parked();
    }

    fn windows(&self, cx: &mut TestAppContext) -> Vec<Entity<ComposerView>> {
        self.view.read_with(cx, |v, _| {
            v.draft_window_numbers()
                .into_iter()
                .filter_map(|n| v.draft_window(n).map(|(_, c)| c))
                .collect()
        })
    }

    fn pane(&self, cx: &mut TestAppContext) -> Entity<ComposerView> {
        self.mail.read_with(cx, |m, _| m.composer().clone())
    }
}

fn subject(c: &Entity<ComposerView>, cx: &mut TestAppContext) -> Option<String> {
    c.read_with(cx, |c, _| c.drafts.active().map(|d| d.subject.clone()))
}

/// Ctrl+O on a Drafts row: the draft the pane shows moves to a window
/// (the pane is free), a row only selected opens in a window too, and a
/// draft already in a window comes forward instead of opening twice.
#[gpui_kit::test]
fn ctrl_o_on_a_drafts_row_opens_it_in_a_composer_window(cx: &mut TestAppContext) {
    let r = open("drafts-ctrl-o", cx);
    let plans = r.draft("Plans");
    let lunch = r.draft("Lunch");
    r.act(cx, |m| m.set_view(View::Drafts));
    // A click opens the row in the pane's composer.
    r.act(cx, |m| m.row_clicked(plans));
    assert!(r.pane(cx).read_with(cx, |c, _| c.is_open()), "in the pane");
    r.ctrl_o(cx);
    let windows = r.windows(cx);
    assert_eq!(windows.len(), 1, "a composer window");
    assert_eq!(subject(&windows[0], cx).as_deref(), Some("Plans"));
    assert!(
        windows[0].read_with(cx, |c, _| c.is_windowed()),
        "a window's composer"
    );
    assert!(
        !r.pane(cx).read_with(cx, |c, _| c.is_open()),
        "the pane is free"
    );

    // A row only selected (its menu, which never opens it).
    r.act(cx, |m| {
        m.row_context(lunch);
    });
    assert!(!r.pane(cx).read_with(cx, |c, _| c.is_open()));
    r.ctrl_o(cx);
    let windows = r.windows(cx);
    assert_eq!(windows.len(), 2, "its own window");
    assert_eq!(subject(&windows[1], cx).as_deref(), Some("Lunch"));

    // Again on the same row: the window comes forward, no third one.
    r.ctrl_o(cx);
    assert_eq!(r.windows(cx).len(), 2, "not opened twice");
    assert_eq!(r.db.count_drafts().expect("count"), 2, "no copy made");
}
