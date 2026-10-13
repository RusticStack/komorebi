//! The teaching tips and the What's new bar (UI plan "Onboarding" 6 and 7;
//! docs/WINUI3.md "TeachingTip"): when they start, what the window shows,
//! and the settings `tips_seen` and `whats_new_version`. Which tip comes
//! next is `komorebi::onboard`'s. Both start from timers after the first
//! frame, never during it, and never under `KOMOREBI_SNAPSHOT` unless a
//! snapshot action asks for them.

use super::*;
use komorebi::onboard::{Onboarding, Seen, Shown, Tip};

/// From the list's first rows (or the pane folding) to the first tip.
pub(super) const TIP_DELAY: Duration = Duration::from_millis(1500);
/// From the start to the What's new bar.
pub(super) const WHATS_NEW_AFTER: Duration = Duration::from_secs(2);
/// The version the What's new bar tells about.
pub(super) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The onboarding of a cache with these settings and accounts.
pub(super) fn onboarding(settings: &db::Settings, has_accounts: bool) -> Onboarding {
    let due = komorebi::onboard::whats_new_due(
        settings.whats_new_version.as_deref(),
        VERSION,
        has_accounts,
    );
    Onboarding::new(Seen::parse(&settings.tips_seen), due)
}

impl App {
    /// The list shows rows: the first time it does with an account, the
    /// tips start after their delay.
    pub(super) fn rows_shown(&mut self, rows: usize) {
        if self.tips_armed || rows == 0 || self.accounts.is_empty() {
            return;
        }
        self.tips_armed = true;
        self.arm_tips(TIP_DELAY);
    }

    fn arm_tips(&self, after: Duration) {
        if !self.tips_auto {
            return;
        }
        self.tip_timer.start(TimerMode::SingleShot, after, || {
            with_app(|a| a.tips_due_now());
        });
    }

    /// Something a tip or the bar must not appear over: Settings, the
    /// composer, a dialog, the folder pane opened over the content.
    fn onboarding_blocked(&self) -> bool {
        let ui = self.ui();
        ui.get_show_settings()
            || ui.get_composing()
            || ui.get_src_show()
            || !ui.get_q_title().is_empty()
            || ui.get_l_show()
            || ui.get_c_confirm_discard()
            || ui.get_shortcuts_show()
            || ui.get_pane_open()
    }

    /// The What's new bar is gone once another status took its place.
    fn sync_bar(&mut self) {
        if self.onboarding.bar_up() && self.status_action.get() != StatusAction::WhatsNew {
            self.onboarding.bar_closed();
        }
    }

    /// The tips' timer: start the series due in this layout, or wait
    /// while something is in the way.
    fn tips_due_now(&mut self) {
        self.sync_bar();
        let compact = self.ui().get_compact();
        if self.accounts.is_empty()
            || !self.onboarding.tips_due(compact)
            || self.onboarding.current().is_some()
        {
            return;
        }
        if self.onboarding_blocked() || self.onboarding.bar_up() {
            return self.arm_tips(TIP_DELAY);
        }
        self.start_tips();
    }

    /// Start the series due in this layout now (the timer; snapshots
    /// and tests). Returns the tip shown.
    pub(super) fn start_tips(&mut self) -> Option<Shown> {
        self.sync_bar();
        let compact = self.ui().get_compact();
        let shown = self.onboarding.start(compact);
        self.show_tip(shown);
        shown
    }

    /// Snapshots: the series of this layout at `tip` (nothing is marked
    /// seen). `None`: that tip is not shown in this layout.
    pub(super) fn start_tips_at(&mut self, tip: Tip) -> Option<Shown> {
        let shown = self.onboarding.start_at(self.ui().get_compact(), tip);
        self.show_tip(shown);
        shown
    }

    fn show_tip(&self, shown: Option<Shown>) {
        let ui = self.ui();
        let count = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
        match shown {
            Some(s) => {
                ui.set_tip_n(count(s.n));
                ui.set_tip_of(count(s.of));
                ui.set_tip_kind(s.tip.code());
            }
            None => ui.set_tip_kind(0),
        }
    }

    fn save_tips(&self) {
        let seen = self.onboarding.seen().format();
        if let Err(e) = self.db.set_setting("tips_seen", &seen) {
            self.set_status(&text::detail(&e), true);
        }
    }

    /// Got it (or Esc): the next tip, or, after the last, the What's new
    /// bar if it waited (and the Folders tip if the window folded while
    /// the desktop's series ran).
    pub(super) fn tip_got_it(&mut self) {
        let next = self.onboarding.got_it();
        self.save_tips();
        self.show_tip(next);
        if next.is_none() {
            self.whats_new_due_now();
            if self.onboarding.tips_due(self.ui().get_compact()) {
                self.arm_tips(TIP_DELAY);
            }
        }
    }

    /// Skip tips: none again, in either layout.
    pub(super) fn tip_skip(&mut self) {
        self.onboarding.skip();
        self.save_tips();
        self.show_tip(None);
        self.whats_new_due_now();
    }

    /// The pane folded into the rail or opened out: a Folders tip goes
    /// with the rail; folding with Folders unseen starts it after the
    /// delay (once the list has shown rows).
    pub(super) fn layout_changed(&mut self, compact: bool) {
        let shown = self.onboarding.layout(compact);
        self.show_tip(shown);
        if compact && self.tips_armed && shown.is_none() && self.onboarding.tips_due(true) {
            self.arm_tips(TIP_DELAY);
        }
    }

    /// Settings > About > Show tips again: every tip is due again, and
    /// the series starts shortly, over the mail.
    pub(super) fn show_tips_again(&mut self) {
        self.onboarding.reset();
        self.save_tips();
        self.show_tip(None);
        self.ui().set_show_settings(false);
        self.tips_armed = true;
        self.arm_tips(Duration::from_millis(500));
    }

    /// The What's new bar's timer (and the end of a series): show it if
    /// it is due and nothing is in its way. It shows once a run; its
    /// version is saved once it is closed or acted on.
    pub(super) fn whats_new_due_now(&mut self) {
        if !self.tips_auto {
            return;
        }
        self.show_whats_new_if_due();
    }

    /// The bar, if due now (no tip up, an account); returns whether it
    /// showed.
    pub(super) fn show_whats_new_if_due(&mut self) -> bool {
        self.sync_bar();
        if self.accounts.is_empty() || !self.onboarding.bar_ready() {
            return false;
        }
        self.set_status(&t!(invoke_whats_new_bar, VERSION.into()), false);
        self.status_timer.stop();
        self.set_status_action(StatusAction::WhatsNew);
        self.onboarding.bar_shown();
        true
    }

    /// The bar was closed or its What's new pressed: told, for this
    /// version. (A status that replaced it tells nothing: it comes back
    /// at the next start.)
    pub(super) fn whats_new_seen(&self) {
        if let Err(e) = self.db.set_setting("whats_new_version", VERSION) {
            self.set_status(&text::detail(&e), true);
        }
    }

    /// The first account was added: there is nothing new to tell it, now
    /// or after this version.
    pub(super) fn first_account_added(&mut self) {
        self.onboarding.bar_cancel();
        if let Err(e) = self.db.set_setting("whats_new_version", VERSION) {
            self.set_status(&text::detail(&e), true);
        }
    }
}
