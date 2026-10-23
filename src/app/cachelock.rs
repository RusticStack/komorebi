//! The encrypted mail cache in the app (`komorebi::cachecrypt`, the
//! dialogs in `ui/cachelock.slint`): Settings > Privacy's switch, which
//! records the choice for the next start (`cachecrypt::apply_intent` in
//! `Db::open_ui` converts before any worker opens the cache), Restart now,
//! and the locked-cache dialog shown at start instead of the generic error
//! when the cache's key is missing or not this Windows user's.

use crate::{AppWindow, CacheLock};
use komorebi::cachecrypt::{self, KeyError};
use slint::ComponentHandle;
use std::path::{Path, PathBuf};

/// Show the switch's state: encrypted now, wanted, and whether a restart
/// is still to apply it.
fn show(ui: &AppWindow, db: &Path) {
    let g = ui.global::<CacheLock>();
    let now = cachecrypt::is_encrypted(db);
    let wanted = cachecrypt::intent(db).unwrap_or(now);
    g.set_encrypted(now);
    g.set_wanted(wanted);
    g.set_pending(wanted != now);
}

/// Settings > Privacy: the switch and Restart now.
pub(super) fn wire(ui: &AppWindow, db: &Path) {
    show(ui, db);
    let g = ui.global::<CacheLock>();
    let weak = ui.as_weak();
    let path = db.to_path_buf();
    g.on_set(move |encrypt| {
        cachecrypt::set_intent(&path, encrypt).ok();
        if let Some(ui) = weak.upgrade() {
            show(&ui, &path);
        }
    });
    g.on_restart(|| {
        restart();
        slint::quit_event_loop().ok();
    });
}

/// Start a new copy of the app with the same arguments; it waits for this
/// process to end (`KOMOREBI_RESTART_AFTER`) before it takes the one-app
/// lock and opens the cache.
fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .env("KOMOREBI_RESTART_AFTER", std::process::id().to_string())
            .spawn()
            .ok();
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

/// The locked-cache dialog over the empty window: Quit, or Start a fresh
/// cache (asked first), which sets the cache and its key aside, dated,
/// and starts the app again on a new, empty cache.
pub fn show_locked(ui: &AppWindow, db: PathBuf) {
    let g = ui.global::<CacheLock>();
    g.set_locked(1);
    g.on_quit(|| {
        slint::quit_event_loop().ok();
    });
    g.on_fresh(move || {
        if set_aside(&db).is_ok() {
            restart();
        }
        slint::quit_event_loop().ok();
    });
}

/// Rename the locked cache (with its `-wal`, `-shm`, key and drafts
/// folder) to `cache.locked-<date>.*` beside it. Nothing is deleted.
fn set_aside(db: &Path) -> std::io::Result<()> {
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
