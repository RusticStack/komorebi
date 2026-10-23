//! Event reminders: which are due now and when the next one is, from the
//! stored occurrences with a reminder (pure; the app arms one timer to the
//! next one and shows a notification only while it runs and only when the
//! user turned reminders on).

use std::collections::HashSet;

/// A reminder due this long ago still fires (a start just after it, or a
/// timer a little late); an older one stays a marker only.
pub const GRACE: i64 = 5 * 60;
/// Reminders further ahead than this are left to a later pass (the timer
/// is armed at most this far, which also covers sleep and clock changes).
pub const HORIZON: i64 = 60 * 60;
/// The longest reminder read (minutes before the start).
pub const MAX_BEFORE_MIN: i64 = 7 * 24 * 60;

/// An occurrence with a reminder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    pub event_id: i64,
    pub start: i64,
    /// Minutes before the start (0: at the start).
    pub before_min: i64,
}

impl Alarm {
    /// When the reminder is due (UTC seconds).
    pub fn at(&self) -> i64 {
        let before = self.before_min.clamp(0, MAX_BEFORE_MIN);
        self.start.saturating_sub(before.saturating_mul(60))
    }
}

/// The reminders to show at `now` (due within [`GRACE`], not yet shown),
/// and when to look again: the next reminder's time, at most
/// [`HORIZON`] from now.
pub fn next_due<'a>(
    now: i64,
    alarms: &'a [Alarm],
    done: &HashSet<(i64, i64)>,
) -> (Vec<&'a Alarm>, i64) {
    let mut fire = Vec::new();
    let mut next = now.saturating_add(HORIZON);
    for a in alarms {
        if done.contains(&(a.event_id, a.start)) {
            continue;
        }
        let at = a.at();
        if at <= now {
            if now.saturating_sub(at) <= GRACE {
                fire.push(a);
            }
        } else if at < next {
            next = at;
        }
    }
    (fire, next)
}

/// The agenda's marker for an occurrence at `now`: under way, or its
/// reminder is due (minutes left to the start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    Now,
    InMinutes(i64),
}

pub fn marker(now: i64, start: i64, end: i64, before_min: Option<i64>) -> Marker {
    if start <= now && now < end {
        return Marker::Now;
    }
    match before_min {
        Some(b) if now < start => {
            let a = Alarm {
                event_id: 0,
                start,
                before_min: b,
            };
            if a.at() <= now {
                // Rounded up: "in 1 min" until the start.
                let secs = start.saturating_sub(now);
                Marker::InMinutes(secs.saturating_add(59) / 60)
            } else {
                Marker::None
            }
        }
        _ => Marker::None,
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

    const NOW: i64 = 1_790_000_000;

    fn a(id: i64, start: i64, before: i64) -> Alarm {
        Alarm {
            event_id: id,
            start,
            before_min: before,
        }
    }

    #[test]
    fn due_now_and_next() {
        let alarms = [
            a(1, NOW + 15 * 60, 15),  // due now
            a(2, NOW + 600, 30),      // due 20 min ago: marker only
            a(3, NOW + 3 * 3600, 60), // due in 2 h: past the horizon
            a(4, NOW + 40 * 60, 15),  // due in 25 min
            a(5, NOW - 60, 0),        // started 1 min ago, at start
        ];
        let (fire, next) = next_due(NOW, &alarms, &HashSet::new());
        let ids: Vec<i64> = fire.iter().map(|a| a.event_id).collect();
        assert_eq!(ids, [1, 5]);
        assert_eq!(next, NOW + 25 * 60);
        // Shown ones never repeat (a restart reads them from alarms_done).
        let done: HashSet<(i64, i64)> = [(1, NOW + 15 * 60), (5, NOW - 60)].into();
        assert!(next_due(NOW, &alarms, &done).0.is_empty());
        // Nothing ahead: look again at the horizon.
        assert_eq!(next_due(NOW, &[], &done).1, NOW + HORIZON);
    }

    #[test]
    fn a_restart_fires_only_the_last_five_minutes() {
        let alarms = [a(1, NOW + 60, 7), a(2, NOW + 60, 4)];
        let (fire, _) = next_due(NOW, &alarms, &HashSet::new());
        assert_eq!(fire.len(), 1);
        assert_eq!(fire[0].event_id, 2);
    }

    #[test]
    fn hostile_values_are_clamped() {
        let alarms = [a(1, i64::MIN, i64::MAX), a(2, i64::MAX, -5)];
        let (fire, next) = next_due(NOW, &alarms, &HashSet::new());
        assert!(fire.is_empty());
        assert_eq!(next, NOW + HORIZON);
    }

    #[test]
    fn markers() {
        assert_eq!(marker(NOW, NOW - 60, NOW + 60, None), Marker::Now);
        assert_eq!(
            marker(NOW, NOW + 600, NOW + 900, Some(15)),
            Marker::InMinutes(10)
        );
        assert_eq!(
            marker(NOW, NOW + 601, NOW + 900, Some(15)),
            Marker::InMinutes(11)
        );
        assert_eq!(marker(NOW, NOW + 3600, NOW + 7200, Some(15)), Marker::None);
        assert_eq!(marker(NOW, NOW + 600, NOW + 900, None), Marker::None);
        assert_eq!(marker(NOW, NOW - 900, NOW - 60, Some(15)), Marker::None);
    }
}
