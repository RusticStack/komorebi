//! Event reminders as notifications, off unless Settings > General >
//! Notifications for event reminders is on: one in-app timer, armed to the
//! next reminder (at most an hour ahead), shows each due reminder once
//! through the tray icon's native notification while Komorebi runs. No
//! scheduled notification, task, wake timer or service: with Komorebi
//! closed nothing fires. The agenda's "In 10 minutes" and "Now" markers
//! need none of this.

use super::*;
use komorebi::cal::remind::{self, GRACE, HORIZON, MAX_BEFORE_MIN};

thread_local! {
    /// The one reminder timer (only while reminders are on).
    static TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

impl App {
    /// Settings: reminders on or off.
    pub(super) fn set_reminders(&mut self, on: bool) {
        self.save_setting("cal_reminders", if on { "1" } else { "0" });
        self.ui().set_g_cal_remind(self.db.reminders_on());
        self.arm_reminders();
    }

    /// Show the reminders due now and arm the timer to the next one; with
    /// reminders off (or in a snapshot) the timer is dropped.
    pub(super) fn arm_reminders(&mut self) {
        if !self.db.reminders_on() || komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some() {
            TIMER.with(|t| t.borrow_mut().take());
            return;
        }
        let now = chrono::Utc::now().timestamp();
        let rows = self
            .db
            .reminders(
                now.saturating_sub(GRACE),
                now.saturating_add(MAX_BEFORE_MIN * 60 + HORIZON),
            )
            .unwrap_or_default();
        let done = self
            .db
            .alarms_done(now.saturating_sub(2 * 86_400))
            .unwrap_or_default();
        let alarms: Vec<remind::Alarm> = rows.iter().map(|r| r.alarm.clone()).collect();
        let (fire, next) = remind::next_due(now, &alarms, &done);
        for a in fire {
            self.db.mark_alarm_done(a.event_id, a.start).ok();
            if let Some(r) = rows.iter().find(|r| &r.alarm == a) {
                self.show_reminder(r);
            }
        }
        let wait = Duration::from_secs(next.saturating_sub(now).clamp(1, HORIZON) as u64);
        let timer = Timer::default();
        timer.start(TimerMode::SingleShot, wait, || {
            // The agenda's markers move on too; this arms the next one.
            with_app(|a| a.calendar_reload());
        });
        TIMER.with(|t| *t.borrow_mut() = Some(timer));
    }

    /// One reminder's notification: the title, start time and place (never
    /// the description). The notification pause is honoured.
    fn show_reminder(&mut self, r: &komorebi::cal::store::Reminder) {
        if self.notes.paused {
            return;
        }
        #[cfg(windows)]
        if let Some(shell) = &self.notes.shell {
            let title: String = if r.summary.trim().is_empty() {
                t!(invoke_invite_no_title)
            } else {
                r.summary.clone()
            };
            let time = format::time(r.alarm.start);
            let body = if r.location.trim().is_empty() {
                t!(invoke_cal_remind_at, time.into())
            } else {
                t!(
                    invoke_cal_remind_at_place,
                    time.into(),
                    r.location.as_str().into()
                )
            };
            // A negative id marks an event: a click opens the Calendar.
            let target = komorebi::notify::Target {
                account_id: 0,
                message_id: r.alarm.event_id.saturating_neg().min(-1),
            };
            shell.show_toast(&title, &body, Some(target));
        }
        #[cfg(not(windows))]
        let _ = r;
    }
}
