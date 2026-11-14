//! The updater's UI (docs/GPUI-MIGRATION.md phase 12; the Slint app's
//! `src/app/updates.rs`): the state Settings > About shows and the quiet
//! "Version X is available" bar over the mail. The work stays in the
//! library (`komorebi::update`: the check, the verified download, the
//! installer); every network job runs on a short-lived thread started by a
//! press (Check for updates, Download and install) or by the once-a-day
//! automatic check a few seconds after start, never offline, in a snapshot
//! or in a test. Nothing is downloaded or installed without the press.
//!
//! The main window owns the [`Updater`] and draws [`bar`]; Settings > About
//! (the settings lane) reads [`Updater::phase`] and calls
//! [`Updater::check`], [`Updater::download`] and [`Updater::set_auto`].

pub mod card;
pub mod text;
pub mod whats_new;

use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, IntoElement, ParentElement, Styled, div, px,
};
use komorebi::db::Db;
use komorebi::update::{self, Phase, SETTING_AUTO, SETTING_LAST};

use crate::theme::space;
use crate::widgets::button::{Button, Variant};
use crate::widgets::notice::{Kind, Notice};

/// The automatic check waits this long after start.
pub const AUTO_AFTER: Duration = Duration::from_secs(8);

/// What the updater asks of the main window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    /// The installer started: save and really exit (also with close to
    /// tray on, so the installer can replace the app).
    Quit,
    /// Release notes: the page, through the window's link prompt (the
    /// reader's confirm-first flow, as the Slint app).
    OpenLink(String),
}

/// The update state (the Slint app's `Updates`).
#[derive(Default)]
pub struct Updates {
    pub phase: Phase,
    /// Automatic checks (Settings > About), on by default.
    pub auto: bool,
    /// The bar was put away with Later on this day (`update::day_of`).
    later_day: Option<i64>,
    /// The last check was the automatic one: its find shows the bar.
    from_auto: bool,
    /// The installer's bytes received and announced size, while it
    /// downloads.
    pub progress: Option<(u64, Option<u64>)>,
}

impl Updates {
    pub fn new(auto: Option<&str>) -> Self {
        Updates {
            auto: auto != Some("0"),
            ..Default::default()
        }
    }

    /// The quiet bar shows: an automatic check found a release and Later
    /// was not pressed today.
    pub fn banner(&self, today: i64) -> bool {
        self.from_auto && matches!(self.phase, Phase::Available(_)) && self.later_day != Some(today)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub struct Updater {
    db: Rc<Db>,
    state: Updates,
}

impl EventEmitter<UpdateEvent> for Updater {}

impl Updater {
    pub fn new(db: Rc<Db>) -> Self {
        let auto = db.settings().ok().and_then(|s| s.update_auto);
        Self {
            state: Updates::new(auto.as_deref()),
            db,
        }
    }

    pub fn state(&self) -> &Updates {
        &self.state
    }

    pub fn phase(&self) -> &Phase {
        &self.state.phase
    }

    /// The bar shows today.
    pub fn banner(&self) -> bool {
        self.state.banner(update::day_of(now()))
    }

    /// The automatic check, [`AUTO_AFTER`] after start: only when due,
    /// never offline or in a snapshot (the window never calls it in a
    /// test).
    pub fn schedule_auto(&mut self, cx: &mut Context<Self>) {
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some()
            || komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some()
        {
            return;
        }
        let last = self.db.settings().ok().and_then(|s| s.update_last_check);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTO_AFTER).await; // timer-exempt: the Slint app's once-a-day check waits a few seconds after start
            let _ = this.update(cx, |u, cx| {
                let auto = if u.state.auto { "1" } else { "0" };
                if update::auto_due(Some(auto), last.as_deref(), now()) {
                    u.check(true, cx);
                }
            });
        })
        .detach();
    }

    /// Automatic checks switched on or off (Settings > About).
    pub fn set_auto(&mut self, on: bool, cx: &mut Context<Self>) {
        self.state.auto = on;
        self.save(SETTING_AUTO, if on { "1" } else { "0" });
        cx.notify();
    }

    /// Later (or the bar's close): hidden until tomorrow.
    pub fn later(&mut self, cx: &mut Context<Self>) {
        self.state.later_day = Some(update::day_of(now()));
        cx.notify();
    }

    /// One HTTPS GET for the latest release, on a short-lived thread.
    pub fn check(&mut self, auto: bool, cx: &mut Context<Self>) {
        if !self.state.phase.start_check() {
            return;
        }
        self.state.from_auto = auto;
        if auto {
            self.save(SETTING_LAST, &now().to_string());
        }
        cx.notify();
        let (tx, rx) = futures::channel::oneshot::channel();
        let started = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                tx.send(update::check()).ok();
            });
        if started.is_err() {
            self.state.phase.checked(Err(update::UpdateError::Io));
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or(Err(update::UpdateError::Io));
            let _ = this.update(cx, |u, cx| {
                u.state.phase.checked(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Download and install: fetch and verify on a short-lived thread, then
    /// start the installer and ask the window to quit.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        let Some(rel) = self.state.phase.start_download() else {
            return;
        };
        self.state.progress = None;
        cx.notify();
        let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded();
        let (done_tx, done_rx) = futures::channel::oneshot::channel();
        let started = std::thread::Builder::new()
            .name("update-download".into())
            .spawn(move || {
                let tell = |done: u64, total: Option<u64>| {
                    progress_tx.unbounded_send((done, total)).ok();
                };
                let result = update::fetch_installer(&rel, &std::env::temp_dir(), &tell);
                done_tx.send(result).ok();
            });
        if started.is_err() {
            self.state.phase.downloaded(Err(update::UpdateError::Io));
            return;
        }
        cx.spawn(async move |this, cx| {
            while let Some(p) = progress_rx.next().await {
                let alive = this.update(cx, |u, cx| {
                    if matches!(u.state.phase, Phase::Downloading(_)) {
                        u.state.progress = Some(p);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
            let result = done_rx.await.unwrap_or(Err(update::UpdateError::Io));
            let _ = this.update(cx, |u, cx| u.downloaded(result, cx));
        })
        .detach();
    }

    fn downloaded(
        &mut self,
        result: Result<std::path::PathBuf, update::UpdateError>,
        cx: &mut Context<Self>,
    ) {
        let path = match result {
            Ok(p) => p,
            Err(e) => {
                self.state.phase.downloaded(Err(e));
                cx.notify();
                return;
            }
        };
        if !self.state.phase.downloaded(Ok(())) {
            return;
        }
        // The installer refuses to run while the app's mutex is held (the
        // Slint app's `downloaded`): let go of it first, and take it back
        // if the installer did not start.
        komorebi::instance::release_app_mutex();
        if let Err(e) = update::run_installer(&path) {
            komorebi::instance::hold_app_mutex();
            self.state.phase = Phase::Failed(e, None);
            cx.notify();
            return;
        }
        cx.emit(UpdateEvent::Quit);
    }

    /// Release notes: the page goes to the window's link prompt.
    pub fn open_notes(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.notes_url() {
            cx.emit(UpdateEvent::OpenLink(url));
        }
    }

    /// The release notes' page (a GitHub release), for the browser.
    pub fn notes_url(&self) -> Option<String> {
        self.state
            .phase
            .release()
            .map(|r| r.notes_url.clone())
            .filter(|url| url.starts_with("https://"))
    }

    fn save(&self, key: &str, value: &str) {
        if let Err(e) = self.db.set_setting(key, value) {
            eprintln!("komorebi-gpui: saving {key}: {e:#}");
        }
    }

    #[cfg(test)]
    pub fn state_mut(&mut self) -> &mut Updates {
        &mut self.state
    }
}

/// The "Version X is available" bar (docs/WINUI3.md "InfoBar"; the Slint
/// app's bar over the list): Download and install, Later, Release notes,
/// and a close that also waits until tomorrow. Nothing while it is not due.
pub fn bar(updater: &Entity<Updater>, cx: &App) -> Option<AnyElement> {
    let u = updater.read(cx);
    if !u.banner() {
        return None;
    }
    let version = u.phase().release()?.version.to_string();
    let (a, b, c, d) = (
        updater.clone(),
        updater.clone(),
        updater.clone(),
        updater.clone(),
    );
    let notice = Notice::new("update-bar", Kind::Info, text::available(&version))
        .action(
            Button::new("update-download")
                .variant(Variant::OnInset)
                .label(text::download())
                .on_click(move |_, _, cx| a.update(cx, |u, cx| u.download(cx))),
        )
        .action(
            Button::new("update-later")
                .variant(Variant::Subtle)
                .label(text::later())
                .on_click(move |_, _, cx| b.update(cx, |u, cx| u.later(cx))),
        )
        .action(
            Button::new("update-notes")
                .variant(Variant::Subtle)
                .label(text::release_notes())
                .on_click(move |_, _, cx| {
                    c.update(cx, |u, cx| u.open_notes(cx));
                }),
        )
        .action(
            Button::new("update-close")
                .variant(Variant::Subtle)
                .label(text::hide_until_tomorrow())
                .on_click(move |_, _, cx| d.update(cx, |u, cx| u.later(cx))),
        );
    Some(
        div()
            .flex_none()
            .px(px(space::PANE_GAP))
            .pb(px(space::SP_2))
            .child(notice)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> update::Release {
        let asset = || update::Asset {
            name: String::new(),
            url: String::new(),
        };
        update::Release {
            version: update::Version(9, 0, 0),
            notes_url: String::new(),
            installer: asset(),
            sha256: asset(),
            sig: asset(),
        }
    }

    /// Port of `auto_defaults_on_and_the_bar_waits_a_day`
    /// (`src/app/updates.rs`).
    #[test]
    fn auto_defaults_on_and_the_bar_waits_a_day() {
        assert!(Updates::new(None).auto);
        assert!(Updates::new(Some("1")).auto);
        assert!(!Updates::new(Some("0")).auto);
        let mut u = Updates::new(None);
        u.phase = Phase::Available(release());
        // A press found it: Settings says so, no bar.
        assert!(!u.banner(10));
        u.from_auto = true;
        assert!(u.banner(10));
        // Later: hidden for the rest of the day, back the next.
        u.later_day = Some(10);
        assert!(!u.banner(10));
        assert!(u.banner(11));
        u.phase = Phase::Downloading(u.phase.release().cloned().unwrap());
        assert!(!u.banner(11));
    }

    /// The bar shows in the main window when an automatic check found a
    /// release, names its buttons, and Later puts it away.
    #[gpui_kit::test]
    fn the_update_bar_shows_and_later_hides_it(cx: &mut gpui::TestAppContext) {
        use crate::test_support::{self, assert_named, in_window};
        let (_cache, db, _) = test_support::seeded("update-bar", 2);
        test_support::init_app(cx);
        let (window, view) = test_support::open_main(cx, test_support::init(Some(db)));
        let updater = view
            .read_with(cx, |v, _| v.updater().cloned())
            .expect("an updater");
        updater.update(cx, |u, cx| {
            let s = u.state_mut();
            s.phase = Phase::Available(release());
            s.from_auto = true;
            cx.notify();
        });
        cx.run_until_parked();
        in_window(cx, window, |window, _| {
            for name in ["Download and install", "Later", "Release notes"] {
                assert_named(window, name, gpui::Role::Button);
            }
        });
        in_window(cx, window, |window, cx| {
            test_support::click_named(window, "Later", cx)
        });
        cx.run_until_parked();
        assert!(!updater.read_with(cx, |u, _| u.banner()));
    }
}
