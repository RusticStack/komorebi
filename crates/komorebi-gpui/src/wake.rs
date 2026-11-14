//! Waking from sleep (docs/GPUI-TEST-PORT.md row 7, the Slint app's
//! `woke` and `wake_check`): a timer that wakes every [`TICK`] and compares
//! the wall clock with the last time it ran. A gap above [`GAP`] means the
//! computer slept (the timer does not run asleep), so the connections the
//! workers hold are likely dead: the owner hears [`Wake::Woke`] at once (to
//! drop them) and [`Wake::Settled`] [`SETTLE`] later, when the network has
//! had a moment to come back (to sync).
//!
//! The hub starts it with its first sync (`Hub::start`) and answers
//! `Woke` with each worker's `resumed()` and `Settled` with a sync of every
//! account and the Outbox. This module is the detector alone, so it is
//! tested with a clock the test moves; the hub's side is tested in
//! `views::mail::sync_tests`.

use std::time::{Duration, SystemTime};

use gpui::{Context, Task};

/// The check runs this often.
pub const TICK: Duration = Duration::from_secs(60);
/// A wall-clock gap between two checks above this means a sleep.
pub const GAP: Duration = Duration::from_secs(120);
/// After a wake the network gets this long to come back before the sync.
pub const SETTLE: Duration = Duration::from_secs(3);

/// What the owner is told, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// The computer woke: replace the connections.
    Woke,
    /// The network had a moment: sync.
    Settled,
}

/// Whether two checks at wall-clock times `last` and `now` are further apart
/// than the computer could be awake between them (a clock set back is no
/// wake).
pub fn woke(last: SystemTime, now: SystemTime) -> bool {
    now.duration_since(last).is_ok_and(|gap| gap > GAP)
}

/// Watches the wall clock (`clock`, [`SystemTime::now`] in the app) for the
/// owner `T` and calls `told` on each [`Wake`]. The task ends with the owner;
/// keep it in a field.
pub fn watch<T: 'static>(
    clock: impl Fn() -> SystemTime + 'static,
    told: impl Fn(&mut T, Wake, &mut Context<T>) + 'static,
    cx: &mut Context<T>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut last = clock();
        loop {
            cx.background_executor().timer(TICK).await; // timer-exempt: the wake check, a wall-clock gap between two ticks is the sign of a sleep
            let now = clock();
            let slept = woke(last, now);
            last = now;
            if !slept {
                if this.upgrade().is_none() {
                    break;
                }
                continue;
            }
            if this.update(cx, |t, cx| told(t, Wake::Woke, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(SETTLE).await; // timer-exempt: the network gets a moment to come back before the sync
            if this.update(cx, |t, cx| told(t, Wake::Settled, cx)).is_err() {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{AppContext as _, Entity, TestAppContext};

    use super::*;

    /// Port of `a_wall_clock_gap_means_a_wake`.
    #[test]
    fn a_wall_clock_gap_means_a_wake() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert!(!woke(t, t + TICK));
        assert!(!woke(t, t + GAP));
        assert!(woke(t, t + GAP + Duration::from_secs(1)));
        assert!(woke(t, t + Duration::from_secs(8 * 3600)));
        // A clock set back is no wake.
        assert!(!woke(t, t - Duration::from_secs(3600)));
    }

    struct Owner {
        heard: Vec<Wake>,
        _watch: Option<Task<()>>,
    }

    fn start(cx: &mut TestAppContext, clock: Rc<Cell<SystemTime>>) -> Entity<Owner> {
        cx.new(|cx| {
            let task = watch(
                move || clock.get(),
                |o: &mut Owner, wake, _| o.heard.push(wake),
                cx,
            );
            Owner {
                heard: Vec::new(),
                _watch: Some(task),
            }
        })
    }

    fn heard(cx: &mut TestAppContext, o: &Entity<Owner>) -> Vec<Wake> {
        o.read_with(cx, |o, _| o.heard.clone())
    }

    /// A tick that finds the clock where it should be says nothing; one that
    /// finds it hours on tells the owner at once and again after the settle.
    #[gpui_kit::test]
    fn the_owner_hears_a_wake_then_the_settle(cx: &mut TestAppContext) {
        let start_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let clock = Rc::new(Cell::new(start_at));
        let owner = start(cx, clock.clone());
        cx.run_until_parked();
        clock.set(clock.get() + TICK);
        cx.executor().advance_clock(TICK);
        cx.run_until_parked();
        assert!(heard(cx, &owner).is_empty(), "awake all along");
        // Asleep: the timer did not run for hours, the wall clock did.
        clock.set(clock.get() + Duration::from_secs(8 * 3600));
        cx.executor().advance_clock(TICK);
        cx.run_until_parked();
        assert_eq!(heard(cx, &owner), [Wake::Woke]);
        cx.executor().advance_clock(SETTLE);
        cx.run_until_parked();
        assert_eq!(heard(cx, &owner), [Wake::Woke, Wake::Settled]);
        // And it keeps watching.
        clock.set(clock.get() + TICK);
        cx.executor().advance_clock(TICK);
        cx.run_until_parked();
        assert_eq!(heard(cx, &owner).len(), 2);
    }

    /// A clock set back is no wake.
    #[gpui_kit::test]
    fn a_clock_set_back_says_nothing(cx: &mut TestAppContext) {
        let clock = Rc::new(Cell::new(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
        ));
        let owner = start(cx, clock.clone());
        cx.run_until_parked();
        clock.set(clock.get() - Duration::from_secs(3600));
        cx.executor().advance_clock(TICK);
        cx.run_until_parked();
        assert!(heard(cx, &owner).is_empty());
    }
}
