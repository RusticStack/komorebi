//! Placing a day's timed events on the week and day time grid: each
//! occurrence clipped to the day and measured in wall-clock minutes from
//! the day's midnight, then overlapping ones put side by side in columns
//! (a greedy colouring of the interval graph: each event takes the first
//! free column of its cluster, and the cluster is as wide as its most
//! columns). An event over midnight shows on each day it touches, each
//! part clipped to that day.
//!
//! Wall minutes, not elapsed ones: on the 23-hour day of a spring-forward
//! change the skipped hour stays empty, and on the 25-hour day an event in
//! the repeated hour is drawn over the first one, as Windows' own calendar
//! does. A grid day is always 24 hours tall.

/// Minutes in a grid day.
pub const DAY_MIN: i64 = 24 * 60;
/// The least height an event takes when columns are decided, so a short
/// event next to another is still readable.
pub const MIN_SPAN: i64 = 20;

/// One event placed on the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Its index in the input.
    pub index: usize,
    /// Minutes from midnight, `0..=DAY_MIN`.
    pub top: i64,
    pub bottom: i64,
    pub col: usize,
    pub cols: usize,
}

/// The part of `[start, end)` (UTC seconds) inside the day `[lo, hi)`, as
/// wall minutes from that day's midnight; `wall_min` gives an instant's
/// wall-clock minutes from its own midnight (the current zone; a fixture
/// in tests). `None` if they don't meet.
pub fn clip(
    start: i64,
    end: i64,
    lo: i64,
    hi: i64,
    wall_min: &dyn Fn(i64) -> i64,
) -> Option<(i64, i64)> {
    let end = end.max(start.saturating_add(1));
    if start >= hi || end <= lo {
        return None;
    }
    let top = if start <= lo {
        0
    } else {
        wall_min(start).clamp(0, DAY_MIN)
    };
    let mut bottom = if end >= hi {
        DAY_MIN
    } else {
        wall_min(end).clamp(0, DAY_MIN)
    };
    // Ending in the hour a fall-back change repeats: its wall end is
    // before its start, so it keeps its length instead.
    if bottom < top {
        let len = end
            .min(hi)
            .saturating_sub(start.max(lo))
            .checked_div(60)
            .unwrap_or(0);
        bottom = top.saturating_add(len).min(DAY_MIN);
    }
    Some((top, bottom))
}

/// Columns for `spans` (top and bottom minutes of one day).
pub fn columns(spans: &[(i64, i64)]) -> Vec<Placed> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    let span = |i: usize| spans.get(i).copied().unwrap_or((0, 0));
    order.sort_by_key(|&i| {
        let (t, b) = span(i);
        (t, std::cmp::Reverse(b))
    });
    let mut out: Vec<Placed> = Vec::with_capacity(spans.len());
    // The current cluster: where its events start in `out`, its latest
    // bottom, and the bottom of each column.
    let mut first = 0usize;
    let mut reach = i64::MIN;
    let mut ends: Vec<i64> = Vec::new();
    for i in order {
        let (top, bottom) = span(i);
        let low = bottom.max(top.saturating_add(MIN_SPAN));
        if top >= reach {
            close(&mut out, first, ends.len());
            first = out.len();
            ends.clear();
        }
        let col = match ends.iter().position(|&e| e <= top) {
            Some(c) => c,
            None => {
                ends.push(i64::MIN);
                ends.len().saturating_sub(1)
            }
        };
        if let Some(e) = ends.get_mut(col) {
            *e = low;
        }
        reach = reach.max(low);
        out.push(Placed {
            index: i,
            top,
            bottom,
            col,
            cols: 1,
        });
    }
    close(&mut out, first, ends.len());
    out
}

fn close(out: &mut [Placed], first: usize, cols: usize) {
    for p in out.iter_mut().skip(first) {
        p.cols = cols.max(1);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    fn cols(spans: &[(i64, i64)]) -> Vec<(usize, usize)> {
        let mut p = columns(spans);
        p.sort_by_key(|p| p.index);
        p.iter().map(|p| (p.col, p.cols)).collect()
    }

    #[test]
    fn apart_overlapping_and_chains() {
        // Apart: one column each.
        assert_eq!(cols(&[(60, 120), (180, 240)]), [(0, 1), (0, 1)]);
        // Two overlapping: side by side.
        assert_eq!(cols(&[(60, 180), (120, 240)]), [(0, 2), (1, 2)]);
        // A chain A-B, B-C (A and C apart): one cluster of two columns, C
        // reuses A's column.
        assert_eq!(
            cols(&[(0, 120), (60, 180), (150, 240)]),
            [(0, 2), (1, 2), (0, 2)]
        );
        // Three at once, then one alone after them.
        assert_eq!(
            cols(&[(0, 60), (0, 60), (30, 90), (600, 660)]),
            [(0, 3), (1, 3), (2, 3), (0, 1)]
        );
        // Back to back is not an overlap.
        assert_eq!(cols(&[(0, 60), (60, 120)]), [(0, 1), (0, 1)]);
        // Two zero-length events at the same time still sit side by side.
        assert_eq!(cols(&[(600, 600), (600, 600)]), [(0, 2), (1, 2)]);
        assert!(columns(&[]).is_empty());
    }

    /// Lisbon: UTC+0 in winter, UTC+1 in summer; the changes in 2026 are
    /// on 29 March at 01:00 UTC and 25 October at 01:00 UTC.
    fn lisbon_min(t: i64) -> i64 {
        const SPRING: i64 = 1_774_746_000; // 2026-03-29 01:00 UTC
        const AUTUMN: i64 = 1_792_890_000; // 2026-10-25 01:00 UTC
        let off = if (SPRING..AUTUMN).contains(&t) {
            3600
        } else {
            0
        };
        (t + off).rem_euclid(86_400) / 60
    }

    #[test]
    fn midnight_crossing_and_dst_days() {
        let jan5 = 1_767_571_200; // 2026-01-05 00:00 (UTC+0)
        let jan6 = jan5 + 86_400;
        // 22:00 to 02:00: the end of the first day, the start of the next.
        let (s, e) = (jan5 + 22 * 3600, jan6 + 2 * 3600);
        assert_eq!(
            clip(s, e, jan5, jan6, &lisbon_min),
            Some((22 * 60, DAY_MIN))
        );
        assert_eq!(clip(s, e, jan6, jan6 + 86_400, &lisbon_min), Some((0, 120)));
        assert_eq!(
            clip(s, e, jan6 + 86_400, jan6 + 2 * 86_400, &lisbon_min),
            None
        );
        // The 23-hour day (29 March): midnight to midnight is 23 hours;
        // an event at 03:00 local (02:00 UTC) sits at 03:00 on the grid.
        let mar29 = 1_774_742_400; // 00:00 local = 00:00 UTC
        let mar30 = mar29 + 23 * 3600; // 00:00 local = 23:00 UTC
        let at3 = mar29 + 2 * 3600;
        assert_eq!(
            clip(at3, at3 + 3600, mar29, mar30, &lisbon_min),
            Some((180, 240))
        );
        // To the end of that day: the bottom is the grid's end.
        assert_eq!(
            clip(mar30 - 3600, mar30 + 600, mar29, mar30, &lisbon_min),
            Some((23 * 60, DAY_MIN))
        );
        // The 25-hour day (25 October): 01:30 happens twice; both are
        // drawn at 01:30 and so side by side.
        let oct25 = 1_792_882_800; // 00:00 local (UTC+1) = 23:00 UTC on the 24th
        let oct26 = oct25 + 25 * 3600;
        let first = oct25 + 90 * 60; // 01:30 summer time
        let second = first + 3600; // 01:30 winter time
        let a = clip(first, first + 1800, oct25, oct26, &lisbon_min).unwrap();
        let b = clip(second, second + 1800, oct25, oct26, &lisbon_min).unwrap();
        assert_eq!(a, (90, 120));
        assert_eq!(b, (90, 120));
        assert_eq!(cols(&[a, b]), [(0, 2), (1, 2)]);
        // The evening of that day keeps its wall time.
        let ev = oct26 - 2 * 3600; // 22:00 winter time
        assert_eq!(
            clip(ev, ev + 3600, oct25, oct26, &lisbon_min),
            Some((22 * 60, 23 * 60))
        );
    }
}
