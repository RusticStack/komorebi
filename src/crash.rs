//! A local crash note. Release builds abort on panic and have no console,
//! so a panic would otherwise end the app without a trace. The hook appends
//! one line (UTC time, version, thread, source location) to `crash.log` next
//! to the cache, and the next launch says so once. The panic message is
//! never written: it can quote mail text. Nothing leaves the machine.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};

/// The log stays this small; a line that would pass it starts a new log.
const MAX_LOG: u64 = 64 * 1024;

/// `crash.log` in the cache's folder (next to `KOMOREBI_DB` when set).
pub fn log_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join("crash.log")
}

/// Records the log length the launch notice last reported.
pub fn seen_path(log: &Path) -> PathBuf {
    log.with_extension("seen")
}

/// Install the panic hook, keeping the default one (stderr in debug builds).
pub fn install(log: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        append(
            &log,
            &line(
                info,
                &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            ),
        );
        previous(info);
    }));
}

/// The log line for a panic. Only metadata: the payload is not read.
fn line(info: &PanicHookInfo, utc: &str) -> String {
    let thread = std::thread::current();
    let at = info.location().map_or_else(
        || "unknown".to_string(),
        |l| format!("{}:{}", l.file(), l.line()),
    );
    format!(
        "{utc} komorebi {} panicked on thread '{}' at {at}\n",
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("unnamed"),
    )
}

/// Best effort: a failure to write the note must not mask the panic.
fn append(log: &Path, line: &str) {
    let full = fs::metadata(log).is_ok_and(|m| m.len() + line.len() as u64 > MAX_LOG);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(!full)
        .truncate(full)
        .open(log);
    if let Ok(mut f) = file {
        f.write_all(line.as_bytes()).ok();
    }
}

/// Whether the log has grown since the last launch notice; marks it seen.
pub fn take_unseen(log: &Path) -> bool {
    let Ok(len) = fs::metadata(log).map(|m| m.len()) else {
        return false;
    };
    let seen = seen_path(log);
    let last = fs::read_to_string(&seen)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    if len == 0 || last == Some(len) {
        return false;
    }
    fs::write(&seen, len.to_string()).ok();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_line_never_carries_the_panic_message() {
        static CAUGHT: Mutex<String> = Mutex::new(String::new());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| {
            *CAUGHT.lock().unwrap() = line(info, "2026-10-08T10:00:00Z");
        }));
        let r = std::thread::Builder::new()
            .name("reader".into())
            .spawn(|| panic!("Subject: secret salary figures for jane@example.com"))
            .unwrap()
            .join();
        std::panic::set_hook(previous);
        assert!(r.is_err());
        let l = CAUGHT.lock().unwrap().clone();
        assert!(l.starts_with("2026-10-08T10:00:00Z komorebi "), "{l}");
        assert!(l.contains("thread 'reader'"), "{l}");
        assert!(l.contains("crash.rs:"), "{l}");
        assert!(l.ends_with('\n') && l.lines().count() == 1, "{l}");
        assert!(!l.contains("secret") && !l.contains("example.com"), "{l}");
    }

    #[test]
    fn log_is_capped_and_noticed_once() {
        let dir = std::env::temp_dir().join(format!("komorebi-crash-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let log = log_path(&dir.join("cache.sqlite3"));
        assert_eq!(log, dir.join("crash.log"));
        fs::remove_file(&log).ok();
        fs::remove_file(seen_path(&log)).ok();

        assert!(!take_unseen(&log), "no log yet");
        let one = "x".repeat(1000) + "\n";
        let mut longest = 0;
        for _ in 0..100 {
            append(&log, &one);
            longest = longest.max(fs::metadata(&log).unwrap().len());
        }
        assert!(longest <= MAX_LOG && longest > MAX_LOG - 1001, "{longest}");
        assert!(fs::metadata(&log).unwrap().len() < longest, "started over");
        assert!(take_unseen(&log));
        assert!(!take_unseen(&log), "noticed once");
        append(&log, "y\n");
        assert!(take_unseen(&log), "a new crash is noticed again");
        fs::remove_dir_all(&dir).ok();
        assert_eq!(
            log_path(Path::new("cache.sqlite3")),
            Path::new("./crash.log")
        );
    }
}
