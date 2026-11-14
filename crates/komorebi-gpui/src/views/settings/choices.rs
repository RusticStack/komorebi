//! The General choices Settings offers and how a saved value shows among
//! them (the Slint app's `src/app/settings.rs`: `nearest`, `read_index`,
//! `UNDO_CHOICES`, `READ_CHOICES`).

use std::time::Duration;

use komorebi::db::{CHECK_INTERVALS, MarkRead, ThemeChoice};

/// The undo-send delays Settings offers, in seconds.
pub const UNDO_CHOICES: [u64; 5] = [0, 5, 10, 20, 30];

/// The mark-as-read delays Settings offers, as `mark_read_delay_ms` values:
/// at once, after 1 or 3 seconds, never.
pub const READ_CHOICES: [i64; 4] = [0, 1000, 3000, -1];

/// The themes, in the order the Theme choice lists them.
pub const THEMES: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

/// The check interval choices, in seconds.
pub const INTERVALS: [u64; 4] = CHECK_INTERVALS;

/// The index of the offered value nearest to `value` (a setting saved by
/// hand may lie between them).
pub fn nearest(choices: &[u64], value: u64) -> usize {
    (0..choices.len())
        .min_by_key(|&i| choices[i].abs_diff(value))
        .unwrap_or(0)
}

/// The mark-as-read choice that shows for `mode`.
pub fn read_index(mode: MarkRead) -> usize {
    match mode {
        MarkRead::Now => 0,
        MarkRead::After(d) if d <= Duration::from_secs(2) => 1,
        MarkRead::After(_) => 2,
        MarkRead::Never => 3,
    }
}

/// The mode a `mark_read_delay_ms` value means.
pub fn read_mode(ms: i64) -> MarkRead {
    match ms {
        0 => MarkRead::Now,
        ms if ms < 0 => MarkRead::Never,
        ms => MarkRead::After(Duration::from_millis(ms as u64)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ports `choices_show_the_nearest_offered_value` (src/app/settings.rs).
    #[test]
    fn choices_show_the_nearest_offered_value() {
        assert_eq!(nearest(&UNDO_CHOICES, 5), 1);
        assert_eq!(nearest(&UNDO_CHOICES, 7), 1);
        assert_eq!(nearest(&UNDO_CHOICES, 60), 4);
        assert_eq!(nearest(&INTERVALS, 300), 2);
        assert_eq!(read_index(MarkRead::Now), 0);
        assert_eq!(read_index(MarkRead::After(Duration::from_secs(1))), 1);
        assert_eq!(read_index(MarkRead::After(Duration::from_secs(60))), 2);
        assert_eq!(read_index(MarkRead::Never), 3);
        // Every offered delay shows as itself.
        for (i, ms) in READ_CHOICES.iter().enumerate() {
            assert_eq!(read_index(read_mode(*ms)), i);
        }
    }
}
