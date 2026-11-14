//! Throwaway folders for the library's tests. A folder name is never
//! picked by the test alone: the process id and a counter that only goes
//! up are in it, so two tests (or two merged lanes) that choose the same
//! label get different folders and one's cleanup cannot remove the
//! other's files. The app's own tests have the same in
//! `komorebi_gpui::test_support::unique_dir`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// `<temp>/<prefix>-<pid>-<n>-<label>`, not created.
pub(crate) fn unique(prefix: &str, label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{}-{n}-{label}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_label_is_two_folders() {
        assert_ne!(unique("komorebi-x", "same"), unique("komorebi-x", "same"));
    }
}
