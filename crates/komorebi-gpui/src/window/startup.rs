//! What start-up hands the main window (the Slint app's `src/main.rs` and
//! `src/app/cachelock.rs`): a `mailto:` from the command line, Start with
//! Windows (`--minimized`), the notices of the last run (a crash, a damaged
//! cache set aside) and the locked-cache dialog, shown over the empty window
//! instead of the generic start-up error when the cache's key is missing or
//! not this Windows user's.

use std::path::{Path, PathBuf};

use gpui::{Context, Window};
use komorebi::cachecrypt::KeyError;
use komorebi::mail::render::links::Mailto;
use komorebi_i18n::tr;

use super::MainWindow;
use crate::views::mail::sync::{Said, StatusAction};
use crate::widgets::dialog::{self, Confirm};

/// What `main` found before the window opened.
#[derive(Default)]
pub struct Start {
    /// `komorebi-gpui.exe "mailto:..."`: a prefilled composer (never sent);
    /// the window shows even with `--minimized`.
    pub mailto: Option<Mailto>,
    /// Started by Windows at sign-in (`--minimized`): the window stays
    /// hidden and the tray icon is the way in (it shows when no icon can be
    /// made).
    pub minimized: bool,
    /// The cache at this path could not be unlocked: the locked-cache
    /// dialog over the empty window.
    pub locked: Option<PathBuf>,
    /// The crash log grew since the last launch: said once.
    pub crashed: bool,
    /// A damaged cache was set aside under this file name.
    pub damaged: Option<String>,
}

impl Start {
    /// Whether the window stays hidden at start.
    pub fn hidden(&self) -> bool {
        self.minimized && self.mailto.is_none() && self.locked.is_none()
    }
}

/// Whether a start-up error is a cache this Windows user cannot unlock.
pub fn is_locked(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        matches!(
            c.downcast_ref::<KeyError>(),
            Some(KeyError::Missing | KeyError::Locked)
        )
    })
}

/// Rename the locked cache (with its `-wal`, `-shm`, key and drafts folder)
/// to `cache.locked-<date>.*` beside it. Nothing is deleted.
pub fn set_aside(db: &Path) -> std::io::Result<()> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let stem = db.file_stem().unwrap_or_default().to_string_lossy();
    let ext = db.extension().unwrap_or_default().to_string_lossy();
    let aside = db.with_file_name(format!("{stem}.locked-{stamp}.{ext}"));
    for suffix in ["-wal", "-shm", ".key", ".encrypt"] {
        let from = PathBuf::from(format!("{}{suffix}", db.display()));
        if from.exists() {
            std::fs::rename(&from, format!("{}{suffix}", aside.display()))?;
        }
    }
    let drafts = komorebi::db::drafts_dir(db);
    if drafts.exists() {
        std::fs::rename(&drafts, komorebi::db::drafts_dir(&aside)).ok();
    }
    std::fs::rename(db, &aside)
}

/// The words of the start-up dialogs and notices (`ui/cachelock.slint`,
/// `ui/tr.slint`).
pub mod text {
    use super::*;

    pub fn locked_title() -> String {
        tr!("The mail cache is locked")
    }

    pub fn locked_body() -> String {
        tr!(
            "Komorebi cannot unlock its encrypted mail cache on this computer: its key is missing or belongs to another Windows user."
        )
    }

    pub fn fresh() -> String {
        tr!("Start a fresh cache")
    }

    pub fn quit() -> String {
        tr!("dialog button" => "Quit")
    }

    pub fn fresh_title() -> String {
        tr!("Start a fresh cache?")
    }

    pub fn fresh_body() -> String {
        tr!(
            "The locked cache is set aside, not deleted. Komorebi starts empty: add your accounts again and their mail is downloaded again. Drafts and the Outbox in the locked cache are not recovered."
        )
    }

    pub fn cancel() -> String {
        crate::views::mail::text::cancel()
    }

    pub fn crash_notice() -> String {
        tr!("Komorebi closed unexpectedly last time; see crash.log")
    }

    pub fn cache_damaged(name: &str) -> String {
        tr!(
            "The mail cache was damaged, so Komorebi started a new one; the old file is kept as {0}. Add your accounts again in Settings.",
            name
        )
    }
}

impl MainWindow {
    /// The start-up notices, on the status line once the hub is there. As
    /// the Slint app's `app::notice`, each stays until it is closed or
    /// another line replaces it (a `--minimized` start shows it when the
    /// window is opened); the damaged cache comes after the crash, as there.
    pub(super) fn say_start_notices(&mut self, start: &Start, cx: &mut Context<Self>) {
        let Some(hub) = self.hub.clone() else {
            return;
        };
        let mut notices = Vec::new();
        if start.crashed {
            notices.push(text::crash_notice());
        }
        if let Some(name) = &start.damaged {
            notices.push(text::cache_damaged(name));
        }
        for notice in notices {
            hub.update(cx, |h, cx| {
                h.say_standing(Said::Text(notice), StatusAction::None, cx)
            });
        }
    }

    /// The locked-cache dialog: Quit (also Escape), or Start a fresh cache,
    /// asked again first.
    pub(super) fn ask_locked(&mut self, db: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        dialog::confirm_or(
            Confirm {
                title: text::locked_title().into(),
                message: text::locked_body().into(),
                confirm: text::fresh().into(),
                cancel: text::quit().into(),
                destructive: true,
            },
            move |window, cx| {
                let db = db.clone();
                let _ = this.update(cx, |_, cx| {
                    cx.defer_in(window, move |this, window, cx| {
                        this.ask_fresh(db, window, cx)
                    })
                });
            },
            |_, cx| cx.quit(),
            window,
            cx,
        );
    }

    /// "Start a fresh cache?": the cache is set aside and a new copy of the
    /// app starts on an empty one; Cancel goes back to the first question.
    fn ask_fresh(&mut self, db: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        let back = db.clone();
        dialog::confirm_or(
            Confirm {
                title: text::fresh_title().into(),
                message: text::fresh_body().into(),
                confirm: text::fresh().into(),
                cancel: text::cancel().into(),
                destructive: true,
            },
            move |_, cx| {
                if set_aside(&db).is_ok() {
                    super::places::restart();
                }
                cx.quit();
            },
            move |window, cx| {
                let back = back.clone();
                let _ = this.update(cx, |_, cx| {
                    cx.defer_in(window, move |this, window, cx| {
                        this.ask_locked(back, window, cx)
                    })
                });
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, in_window, names};
    use gpui::{Role, TestAppContext};
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    #[test]
    fn a_locked_key_is_told_apart_from_other_errors() {
        let locked = anyhow::Error::new(KeyError::Locked).context("opening the cache");
        assert!(is_locked(&locked));
        assert!(is_locked(&anyhow::Error::new(KeyError::Missing)));
        assert!(!is_locked(&anyhow::Error::new(KeyError::Failed(
            "x".into()
        ))));
        assert!(!is_locked(&anyhow::anyhow!("disk full")));
    }

    #[test]
    fn a_minimized_start_shows_the_window_for_a_link_or_a_dialog() {
        let mut start = Start {
            minimized: true,
            ..Start::default()
        };
        assert!(start.hidden());
        start.mailto = Some(Mailto::default());
        assert!(!start.hidden(), "a mail link shows the window");
        start.mailto = None;
        start.locked = Some(PathBuf::from("cache.sqlite3"));
        assert!(!start.hidden(), "the locked dialog shows the window");
    }

    #[test]
    fn setting_aside_renames_the_cache_and_its_files() {
        let cache = test_support::TempCache::new("set-aside");
        let db = cache.dir().join("cache.sqlite3");
        std::fs::write(&db, b"x").unwrap();
        std::fs::write(format!("{}.key", db.display()), b"k").unwrap();
        set_aside(&db).unwrap();
        assert!(!db.exists());
        let left: Vec<String> = std::fs::read_dir(cache.dir())
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("cache.locked-"))
            .collect();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(left.iter().any(|n| n.ends_with(".sqlite3.key")), "{left:?}");
    }

    #[test]
    fn the_start_up_words_are_the_slint_apps() {
        with_lang(Lang::En, || {
            assert_eq!(text::locked_title(), "The mail cache is locked");
            assert_eq!(text::quit(), "Quit");
            assert_eq!(
                text::crash_notice(),
                "Komorebi closed unexpectedly last time; see crash.log"
            );
        });
        with_lang(Lang::PtPt, || {
            assert_ne!(text::locked_title(), "The mail cache is locked");
            assert_ne!(text::fresh_title(), "Start a fresh cache?");
        });
    }

    /// The locked cache asks over the empty window: Start a fresh cache or
    /// Quit; the fresh cache is asked again, and Cancel goes back.
    #[gpui_kit::test]
    fn a_locked_cache_asks_before_anything_else(cx: &mut TestAppContext) {
        test_support::init_app(cx);
        let mut init = test_support::plain_init();
        init.start.locked = Some(PathBuf::from("C:/nowhere/cache.sqlite3"));
        let (window, _) = test_support::open_main(cx, init);
        cx.run_until_parked();
        let shown = |cx: &mut TestAppContext| {
            in_window(cx, window, |window, _| {
                names(window)
                    .into_iter()
                    .filter(|(role, _)| *role == Some(Role::Button))
                    .map(|(_, n)| n)
                    .collect::<Vec<_>>()
            })
        };
        let buttons = shown(cx);
        assert!(
            buttons.contains(&"Start a fresh cache".to_string()),
            "{buttons:?}"
        );
        assert!(buttons.contains(&"Quit".to_string()), "{buttons:?}");
        in_window(cx, window, |window, cx| {
            test_support::click_named(window, "Start a fresh cache", cx)
        });
        cx.run_until_parked();
        let buttons = shown(cx);
        assert!(buttons.contains(&"Cancel".to_string()), "{buttons:?}");
        in_window(cx, window, |window, cx| {
            test_support::click_named(window, "Cancel", cx)
        });
        cx.run_until_parked();
        assert!(shown(cx).contains(&"Quit".to_string()));
    }
}
