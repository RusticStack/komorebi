//! The opt-in diagnostics log (Settings > Privacy, key `diagnostics_log`,
//! off by default): `logs\komorebi.log` beside the cache, kept to 1 MB and
//! two older files. A line names only a time, an account id, a command
//! verb, a status or response code and a duration, e.g.
//! `2026-10-09T10:00:00Z account 1 FETCH OK 12 ms`. Never arguments,
//! folder names, addresses, subjects, server text or passwords. Nothing
//! leaves the machine.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A log file this large is moved to `.1` before the next line.
const MAX_LOG: u64 = 1024 * 1024;
/// Older files kept: `komorebi.log.1` and `.2`.
const KEEP: usize = 2;

static ON: AtomicBool = AtomicBool::new(false);
static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `logs\komorebi.log` in the cache's folder (next to `KOMOREBI_DB` when
/// set).
pub fn log_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join("logs")
        .join("komorebi.log")
}

/// Write to `log` from now on (`None`: stop). Nothing is written until the
/// first line, so turning it on costs nothing at start.
pub fn set(log: Option<PathBuf>) {
    let on = log.is_some();
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = log;
    ON.store(on, Ordering::Relaxed);
}

/// Whether the log is on (callers skip building a line when it is off).
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Log one step: `verb` and `status` come from code (command names and
/// response codes), never from mail or server text.
pub fn record(account: i64, verb: &str, status: &str, took: Duration) {
    if !enabled() {
        return;
    }
    let line = format!(
        "{} account {account} {verb} {status} {} ms\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        took.as_millis()
    );
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = log.as_deref() {
        append(path, &line);
    }
}

/// Best effort: a log that can't be written is skipped.
fn append(path: &Path, line: &str) {
    if fs::metadata(path).is_ok_and(|m| m.len() + line.len() as u64 > MAX_LOG) {
        rotate(path);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).ok();
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        f.write_all(line.as_bytes()).ok();
    }
}

/// `komorebi.log` becomes `.1`, `.1` becomes `.2`, and the oldest goes.
fn rotate(path: &Path) {
    let old = |n: usize| PathBuf::from(format!("{}.{n}", path.display()));
    fs::remove_file(old(KEEP)).ok();
    for n in (1..KEEP).rev() {
        fs::rename(old(n), old(n + 1)).ok();
    }
    fs::rename(path, old(1)).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_the_cap_keeping_three_files() {
        let dir = std::env::temp_dir().join(format!("komorebi-diag-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        let log = log_path(&dir.join("cache.sqlite3"));
        assert_eq!(log, dir.join("logs").join("komorebi.log"));
        let line = "x".repeat(64 * 1024 - 1) + "\n";
        for _ in 0..(3 * 16 + 2) {
            append(&log, &line);
        }
        let len = |p: PathBuf| fs::metadata(p).map(|m| m.len()).ok();
        assert!(len(log.clone()).is_some_and(|n| n <= MAX_LOG));
        assert_eq!(len(dir.join("logs").join("komorebi.log.1")), Some(MAX_LOG));
        assert_eq!(len(dir.join("logs").join("komorebi.log.2")), Some(MAX_LOG));
        assert!(len(dir.join("logs").join("komorebi.log.3")).is_none());
        fs::remove_dir_all(&dir).ok();
    }
}
