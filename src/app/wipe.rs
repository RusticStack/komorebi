//! Settings > Privacy > Remove all data. The saved passwords go first,
//! while the app still runs (if one can't be deleted, nothing else is
//! touched); the files go after the window closes, once the cache is no
//! longer open: the cache (with its `-wal` and `-shm`), the drafts and
//! opened-attachment folders, `crash.log` and `crash.seen`, the diagnostics log
//! files (and their `logs` folder once empty) and any damaged caches set aside. Then the data folder itself
//! (`%LOCALAPPDATA%\komorebi`), if nothing else is left in it. Mail on the
//! server is never touched.

use crate::crash;
use crate::db;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the files may stay in use after the window closed (workers
/// logging out still hold the cache open) before the removal gives up.
const IN_USE_GRACE: Duration = Duration::from_secs(10);

/// Delete the saved password of each account in `ids` with `forget`,
/// stopping at the first failure.
pub(super) fn forget_passwords(
    ids: &[i64],
    forget: &dyn Fn(i64) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    ids.iter().try_for_each(|&id| forget(id))
}

/// Everything the app keeps beside the cache at `db_path` (see PRIVACY.md,
/// "Data inventory"), whether it exists or not.
fn data_paths(db_path: &Path) -> Vec<PathBuf> {
    let with = |suffix: &str| PathBuf::from(format!("{}{suffix}", db_path.display()));
    let crash_log = crash::log_path(db_path);
    let mut paths = vec![
        db_path.to_path_buf(),
        with("-wal"),
        with("-shm"),
        with("-journal"),
        db::drafts_dir(db_path),
        super::files::open_dir(db_path),
        crash::seen_path(&crash_log),
        crash_log,
    ];
    // The diagnostics log and its rotated copies, not the rest of `logs`.
    let log = komorebi::diag::log_path(db_path);
    if let (Some(dir), Some(name)) = (log.parent(), log.file_name()) {
        let name = name.to_string_lossy();
        if let Ok(entries) = std::fs::read_dir(dir) {
            paths.extend(
                entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&*name))
                    .map(|e| e.path()),
            );
        }
    }
    // Damaged caches set aside: `cache.damaged-<date>.sqlite3`, its `-wal`
    // and `-shm`, and `cache.damaged-<date>.drafts`.
    let stem = db_path.file_stem().unwrap_or_default().to_string_lossy();
    let aside = format!("{stem}.damaged-");
    if let Ok(entries) = std::fs::read_dir(folder(db_path)) {
        paths.extend(
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&aside))
                .map(|e| e.path()),
        );
    }
    paths
}

/// The folder the cache is in.
fn folder(db_path: &Path) -> &Path {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Remove `path`, a file or a folder; true once it is gone.
fn remove(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).is_ok() || !path.exists(),
        Ok(_) => std::fs::remove_file(path).is_ok(),
    }
}

/// Remove every data file of the cache at `db_path`, trying again while
/// some are still in use, for at most `grace`. Then the folder itself, if
/// it is the app's own (`komorebi`, not the folder of a `KOMOREBI_DB`
/// elsewhere) and now empty. Returns what is left.
pub(super) fn remove_files_within(db_path: &Path, grace: Duration) -> Vec<PathBuf> {
    let until = Instant::now() + grace;
    let mut left = data_paths(db_path);
    loop {
        left.retain(|p| !remove(p));
        if left.is_empty() || Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // The `logs` folder, once nothing else is in it.
    if let Some(logs) = komorebi::diag::log_path(db_path).parent() {
        std::fs::remove_dir(logs).ok();
    }
    let dir = folder(db_path);
    if left.is_empty() && dir.file_name().is_some_and(|n| n == "komorebi") {
        // Fails, harmlessly, while anything else is in the folder.
        std::fs::remove_dir(dir).ok();
    }
    left
}

/// [`remove_files_within`] the app's grace.
pub(super) fn remove_files(db_path: &Path) -> Vec<PathBuf> {
    remove_files_within(db_path, IN_USE_GRACE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("komorebi-wipe-{}-{name}", std::process::id()))
                .join("komorebi");
            fs::remove_dir_all(&dir).ok();
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(self.0.parent().unwrap()).ok();
        }
    }

    /// A data folder holding one of everything the app writes.
    fn fill(dir: &Path) -> PathBuf {
        let db = dir.join("cache.sqlite3");
        let file = |rel: &str| {
            let p = dir.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"x").unwrap();
        };
        for rel in [
            "cache.sqlite3",
            "cache.sqlite3-wal",
            "cache.sqlite3-shm",
            "cache.drafts/1/report.pdf",
            "cache.open/4f2a/photo.jpg",
            "crash.log",
            "crash.seen",
            "logs/komorebi.log",
            "logs/komorebi.log.1",
            "cache.damaged-20261009-101500.sqlite3",
            "cache.damaged-20261009-101500.sqlite3-wal",
            "cache.damaged-20261009-101500.drafts/2/a.txt",
        ] {
            file(rel);
        }
        db
    }

    #[test]
    fn removes_every_data_file_and_the_emptied_folder() {
        let tmp = TempDir::new("all");
        let db = fill(&tmp.0);
        assert!(remove_files_within(&db, Duration::ZERO).is_empty());
        assert!(!tmp.0.exists(), "the empty data folder goes too");
        // Nothing left to remove is not an error.
        assert!(remove_files_within(&db, Duration::ZERO).is_empty());
    }

    #[test]
    fn leaves_other_files_and_their_folder() {
        let tmp = TempDir::new("other");
        let db = fill(&tmp.0);
        // Another cache side by side (`KOMOREBI_DB`) and an unrelated file.
        fs::write(tmp.0.join("other.sqlite3"), b"x").unwrap();
        fs::write(tmp.0.join("notes.txt"), b"x").unwrap();
        fs::write(tmp.0.join("logs").join("mine.txt"), b"x").unwrap();
        assert!(remove_files_within(&db, Duration::ZERO).is_empty());
        let mut names: Vec<String> = fs::read_dir(&tmp.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["logs", "notes.txt", "other.sqlite3"]);
        assert_eq!(fs::read_dir(tmp.0.join("logs")).unwrap().count(), 1);
    }

    #[test]
    fn keeps_a_folder_that_is_not_the_apps() {
        let tmp = TempDir::new("scratch");
        let dir = tmp.0.parent().unwrap().join("scratch");
        fs::create_dir_all(&dir).unwrap();
        let db = fill(&dir);
        assert!(remove_files_within(&db, Duration::ZERO).is_empty());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn a_file_in_use_is_retried_then_reported() {
        let tmp = TempDir::new("busy");
        let db = fill(&tmp.0);
        // Windows refuses to delete a file opened without FILE_SHARE_DELETE,
        // as SQLite opens the cache.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE
                .open(&db)
                .unwrap();
            let left = remove_files_within(&db, Duration::from_millis(250));
            assert_eq!(left, std::slice::from_ref(&db));
            assert!(tmp.0.exists(), "the folder stays with the file in it");
            assert!(!tmp.0.join("cache.drafts").exists());
            drop(held);
        }
        assert!(remove_files_within(&db, Duration::ZERO).is_empty());
        assert!(!tmp.0.exists());
    }

    #[test]
    fn passwords_stop_at_the_first_failure() {
        let asked = RefCell::new(Vec::new());
        let fake = |id: i64| {
            asked.borrow_mut().push(id);
            if id == 2 {
                anyhow::bail!("Windows Credential Manager is not available")
            }
            Ok(())
        };
        assert!(forget_passwords(&[1, 3], &fake).is_ok());
        assert!(forget_passwords(&[1, 2, 3], &fake).is_err());
        assert_eq!(*asked.borrow(), [1, 3, 1, 2]);
    }
}
