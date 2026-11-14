//! The What's new bar (the Slint app's `src/app/tips.rs`): after an update,
//! once a run, the status line says "Komorebi was updated to X." with a
//! What's new button. Its version is saved (setting `whats_new_version`)
//! once it is closed or acted on; a status that took its place tells
//! nothing, so it comes back at the next start. Never without an account
//! (the first account saves the version silently: the onboarding lane).
//!
//! The bar never shows with a teaching tip: the window asks for it through
//! `MainWindow::whats_new_due_now` (`window/tips.rs`), after its two
//! seconds and again after the last tip.

use std::time::Duration;

use gpui::{App, Entity};
use komorebi::db::Db;

use crate::views::mail::hub::Hub;
use crate::views::mail::sync::{Said, StatusAction};

/// From the start to the bar.
pub const AFTER: Duration = Duration::from_secs(2);

/// The version the bar tells about (the app's).
pub fn version() -> String {
    komorebi::update::current().to_string()
}

/// The bar is due: an account, and this version not told yet.
pub fn due(db: &Db) -> bool {
    let Ok(settings) = db.settings() else {
        return false;
    };
    let has_accounts = db.accounts().is_ok_and(|a| !a.is_empty());
    komorebi::onboard::whats_new_due(
        settings.whats_new_version.as_deref(),
        &version(),
        has_accounts,
    )
}

/// Show the bar if it is due and the status line is free; returns whether
/// it showed.
pub fn show_if_due(db: &Db, hub: &Entity<Hub>, cx: &mut App) -> bool {
    if !due(db) || hub.read(cx).sync().line().is_some() {
        return false;
    }
    hub.update(cx, |hub, cx| {
        hub.say_standing(
            Said::WhatsNew { version: version() },
            StatusAction::WhatsNew,
            cx,
        )
    });
    true
}

/// The bar was closed or its What's new pressed: told, for this version.
pub fn seen(db: &Db) {
    if let Err(e) = db.set_setting("whats_new_version", &version()) {
        eprintln!("komorebi-gpui: saving whats_new_version: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::views::mail::hub::Mode;
    use gpui::{AppContext as _, TestAppContext};
    use std::rc::Rc;

    fn hub(db: komorebi::db::Db, cx: &mut TestAppContext) -> (Rc<Db>, Entity<Hub>) {
        let db = Rc::new(db);
        let d = db.clone();
        let hub = cx.new(|cx| Hub::with_mode(d, Mode::Off, cx));
        (db, hub)
    }

    /// Port of `whats_new_shows_once_and_never_with_a_tip` (the bar's half;
    /// the tips are the onboarding lane's): due after an update, shown on
    /// the status line with its action, saved only once acted on, then
    /// never again.
    #[gpui_kit::test]
    fn whats_new_shows_once_and_is_saved_when_acted_on(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("whats-new", 2);
        test_support::init_app(cx);
        let (db, hub) = hub(db, cx);
        assert!(due(&db));
        assert!(cx.update(|cx| show_if_due(&db, &hub, cx)));
        let action = hub.read_with(cx, |h, _| h.sync().line().map(|l| l.action));
        assert_eq!(action, Some(StatusAction::WhatsNew));
        assert_eq!(
            db.settings().unwrap().whats_new_version,
            None,
            "not told yet"
        );
        // The bar stays (no 5 s timeout).
        cx.executor().advance_clock(Duration::from_secs(30));
        cx.run_until_parked();
        assert!(hub.read_with(cx, |h, _| h.sync().line().is_some()));
        // Its action: told.
        let told = std::rc::Rc::new(std::cell::Cell::new(false));
        let t = told.clone();
        let db2 = db.clone();
        let _sub = cx.update(|cx| {
            cx.subscribe(&hub, move |_, e: &crate::views::mail::hub::HubEvent, _| {
                if matches!(e, crate::views::mail::hub::HubEvent::WhatsNew) {
                    seen(&db2);
                    t.set(true);
                }
            })
        });
        hub.update(cx, |h, cx| h.press_action(cx));
        assert!(told.get());
        assert_eq!(
            db.settings().unwrap().whats_new_version.as_deref(),
            Some(version().as_str())
        );
        assert!(!cx.update(|cx| show_if_due(&db, &hub, cx)), "never again");
    }

    /// Port of `whats_new_replaced_by_another_status_comes_back_next_start`:
    /// a status that took its place tells nothing.
    #[gpui_kit::test]
    fn whats_new_replaced_by_another_status_comes_back(cx: &mut TestAppContext) {
        let (_cache, db, _) = test_support::seeded("whats-new-replaced", 2);
        test_support::init_app(cx);
        let (db, hub) = hub(db, cx);
        assert!(cx.update(|cx| show_if_due(&db, &hub, cx)));
        hub.update(cx, |h, cx| {
            h.say(Said::Text("Can't reach the server.".into()), true, cx)
        });
        assert_eq!(db.settings().unwrap().whats_new_version, None);
        assert!(due(&db), "the next start shows it");
    }

    /// Without an account the bar is never due (port of the half of
    /// `the_first_account_saves_the_version_silently` that is the shell's).
    #[test]
    fn no_account_no_bar() {
        let cache = test_support::TempCache::new("whats-new-none");
        let db = cache.open();
        assert!(!due(&db));
    }
}
