//! Settings > About > Updates and the "Version X is available" bar: the UI
//! side of `komorebi::update`. Every network job runs on a short-lived
//! thread started by a press (Check for updates, Download and install) or
//! by the once-a-day automatic check a few seconds after start; answers come
//! back through `on_ui`. Nothing is downloaded or installed without the
//! press. The sentences are in `ui/app.slint`, chosen by the numbers set
//! here (`Phase::code`, `update::error_code`).

use super::{App, background, format, on_ui, with_app};
use komorebi::update::{self, Phase, SETTING_AUTO, SETTING_LAST};
use slint::Timer;
use std::time::Duration;

/// The automatic check waits this long after start.
pub(super) const AUTO_AFTER: Duration = Duration::from_secs(8);

/// The update state the controller keeps.
#[derive(Default)]
pub(super) struct Updates {
    pub(super) phase: Phase,
    /// Automatic checks (Settings > About), on by default.
    pub(super) auto: bool,
    /// The bar was put away with Later on this day (`update::day_of`).
    later_day: Option<i64>,
    /// The last check was the automatic one: its find shows the bar.
    from_auto: bool,
    /// The installer's bytes received and announced size, while it downloads.
    progress: Option<(u64, Option<u64>)>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Updates {
    pub(super) fn new(auto: Option<&str>) -> Self {
        Updates {
            auto: auto != Some("0"),
            ..Default::default()
        }
    }

    /// The quiet bar shows: an automatic check found a release and Later
    /// was not pressed today.
    fn banner(&self, today: i64) -> bool {
        self.from_auto && matches!(self.phase, Phase::Available(_)) && self.later_day != Some(today)
    }
}

/// The automatic check, `AUTO_AFTER` after start: only when due, never
/// offline or in a snapshot run.
pub(super) fn schedule_auto(last: Option<String>) {
    if komorebi::dev_var("KOMOREBI_OFFLINE").is_some()
        || komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some()
    {
        return;
    }
    Timer::single_shot(AUTO_AFTER, move || {
        with_app(|a| {
            let auto = if a.updates.auto { "1" } else { "0" };
            if update::auto_due(Some(auto), last.as_deref(), now()) {
                a.check_updates(true);
            }
        });
    });
}

impl App {
    /// Show the update state in Settings > About and the bar.
    pub(super) fn show_updates(&self) {
        let ui = self.ui();
        let u = &self.updates;
        ui.set_up_state(u.phase.code());
        ui.set_up_version(
            u.phase
                .release()
                .map(|r| r.version.to_string())
                .unwrap_or_default()
                .into(),
        );
        ui.set_up_error(match &u.phase {
            Phase::Failed(e, _) => update::error_code(e),
            _ => -1,
        });
        let (done, total) = match (&u.phase, u.progress) {
            (Phase::Downloading(_), Some(p)) => p,
            _ => (0, None),
        };
        ui.set_up_progress(update::fraction(done, total).unwrap_or(-1.0));
        let (done_text, total_text) = match total.filter(|&t| t > 0) {
            Some(t) => (
                format::size(self.lang, done.min(t) as usize),
                format::size(self.lang, t as usize),
            ),
            None => Default::default(),
        };
        ui.set_up_done(done_text.into());
        ui.set_up_total(total_text.into());
        ui.set_up_retry(u.phase.release().is_some());
        ui.set_up_auto(u.auto);
        ui.set_up_banner(u.banner(update::day_of(now())));
    }

    /// 0 Check for updates, 1 Download and install, 2 Later, 3 Release
    /// notes.
    pub(super) fn update_action(&mut self, which: i32) {
        match which {
            0 => self.check_updates(false),
            1 => self.download_update(),
            2 => {
                self.updates.later_day = Some(update::day_of(now()));
                self.show_updates();
            }
            3 => {
                if let Some(url) = self.updates.phase.release().map(|r| r.notes_url.clone()) {
                    self.reader.follow(url);
                }
            }
            _ => {}
        }
    }

    /// Automatic checks switched on or off.
    pub(super) fn set_update_auto(&mut self, on: bool) {
        self.updates.auto = on;
        self.save_setting(SETTING_AUTO, if on { "1" } else { "0" });
        self.show_updates();
    }

    /// One HTTPS GET for the latest release, on a short-lived thread.
    pub(super) fn check_updates(&mut self, auto: bool) {
        if !self.updates.phase.start_check() {
            return;
        }
        self.updates.from_auto = auto;
        if auto {
            self.save_setting(SETTING_LAST, &now().to_string());
        }
        self.show_updates();
        let job = move || {
            let result = update::check();
            on_ui(Box::new(move |app| {
                app.updates.phase.checked(result);
                app.show_updates();
            }));
        };
        if background("update-check", Box::new(job)).is_err() {
            self.updates.phase.checked(Err(update::UpdateError::Io));
            self.show_updates();
        }
    }

    /// Download and install: fetch and verify on a short-lived thread, then
    /// save, start the installer and quit.
    fn download_update(&mut self) {
        let Some(rel) = self.updates.phase.start_download() else {
            return;
        };
        self.updates.progress = None;
        self.show_updates();
        let job = move || {
            let tell = |done: u64, total: Option<u64>| {
                on_ui(Box::new(move |app| app.download_progress(done, total)));
            };
            let result = update::fetch_installer(&rel, &std::env::temp_dir(), &tell);
            on_ui(Box::new(move |app| app.downloaded(result)));
        };
        if background("update-download", Box::new(job)).is_err() {
            self.updates.phase.downloaded(Err(update::UpdateError::Io));
            self.show_updates();
        }
    }

    /// Bytes of the installer arrived (ignored when no download runs, so a
    /// late report never changes another state).
    fn download_progress(&mut self, done: u64, total: Option<u64>) {
        if matches!(self.updates.phase, Phase::Downloading(_)) {
            self.updates.progress = Some((done, total));
            self.show_updates();
        }
    }

    /// The download answered: on a verified installer, save drafts and tabs,
    /// start it and quit (also with close to tray on: the app really
    /// exits, so the installer can replace it).
    fn downloaded(&mut self, result: Result<std::path::PathBuf, update::UpdateError>) {
        let path = match result {
            Ok(p) => p,
            Err(e) => {
                self.updates.phase.downloaded(Err(e));
                self.show_updates();
                return;
            }
        };
        if !self.updates.phase.downloaded(Ok(())) {
            return;
        }
        self.show_updates();
        self.notes.quitting = true;
        self.before_close();
        crate::release_app_mutex();
        if let Err(e) = update::run_installer(&path) {
            self.notes.quitting = false;
            self.updates.phase = Phase::Failed(e, None);
            self.show_updates();
            return;
        }
        #[cfg(windows)]
        {
            self.notes.shell = None;
        }
        slint::quit_event_loop().ok();
    }

    /// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=update:<state>`): Settings >
    /// About in a made-up state (`checking`, `uptodate`, `available`,
    /// `downloading`, `failed`, `failed-verify`, `banner`); nothing is
    /// looked up or downloaded.
    pub(super) fn sample_update(&mut self, state: &str) {
        let v = update::Version(update::current().0, update::current().1 + 1, 0);
        let rel = update::Release {
            version: v,
            notes_url: String::new(),
            installer: update::Asset {
                name: String::new(),
                url: String::new(),
            },
            sha256: update::Asset {
                name: String::new(),
                url: String::new(),
            },
            sig: update::Asset {
                name: String::new(),
                url: String::new(),
            },
        };
        self.updates.from_auto = state == "banner";
        self.updates.phase = match state {
            "checking" => Phase::Checking,
            "uptodate" => Phase::UpToDate,
            "available" | "banner" => Phase::Available(rel),
            "downloading" => {
                self.updates.progress = Some((3_355_443, Some(9_122_611)));
                Phase::Downloading(rel)
            }
            "failed" => Phase::Failed(update::UpdateError::Network, None),
            "failed-verify" => Phase::Failed(update::UpdateError::BadSignature, Some(rel)),
            _ => Phase::Idle,
        };
        self.show_updates();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_defaults_on_and_the_bar_waits_a_day() {
        assert!(Updates::new(None).auto);
        assert!(Updates::new(Some("1")).auto);
        assert!(!Updates::new(Some("0")).auto);
        let rel = update::Release {
            version: update::Version(9, 0, 0),
            notes_url: String::new(),
            installer: update::Asset {
                name: String::new(),
                url: String::new(),
            },
            sha256: update::Asset {
                name: String::new(),
                url: String::new(),
            },
            sig: update::Asset {
                name: String::new(),
                url: String::new(),
            },
        };
        let mut u = Updates::new(None);
        u.phase = Phase::Available(rel);
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
}
