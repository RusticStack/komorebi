//! First-run teaching (UI plan "Onboarding" 6 and 7): the teaching tips and
//! the What's new bar, as plain state the app drives with its timers.
//!
//! - The tips: **Folders** (the narrow rail only), **Search** and **Help**,
//!   one at a time, each remembered on its own (setting `tips_seen`), so a
//!   tip seen in one layout is not shown again in the other. The place in
//!   the series ("1 of 2") counts the tips still due in the layout shown
//!   when the series started.
//! - What's new: an Informational bar when the version saved in
//!   `whats_new_version` is not this one, never at the same time as a tip.

/// A teaching tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tip {
    /// The rail's pane toggle: accounts and folders are behind it.
    Folders,
    /// The title bar's search.
    Search,
    /// The Settings footer: Settings > About and F1.
    Help,
}

impl Tip {
    /// Every tip, in the order a series shows them.
    pub const ALL: [Tip; 3] = [Tip::Folders, Tip::Search, Tip::Help];

    /// Its token in the `tips_seen` setting.
    pub fn token(self) -> &'static str {
        match self {
            Tip::Folders => "folders",
            Tip::Search => "search",
            Tip::Help => "help",
        }
    }

    /// Its number in the UI (`tip-kind`: 0 is none).
    pub fn code(self) -> i32 {
        match self {
            Tip::Folders => 1,
            Tip::Search => 2,
            Tip::Help => 3,
        }
    }

    /// A tip named in a snapshot action (`tips:search`).
    pub fn from_token(token: &str) -> Option<Tip> {
        Tip::ALL.into_iter().find(|t| t.token() == token.trim())
    }
}

/// The tips already seen (setting `tips_seen`: tokens, comma separated).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    folders: bool,
    search: bool,
    help: bool,
}

impl Seen {
    /// Every tip (Skip tips).
    pub const ALL: Seen = Seen {
        folders: true,
        search: true,
        help: true,
    };

    /// The setting's value read back; unknown tokens are ignored.
    pub fn parse(value: &str) -> Seen {
        let mut seen = Seen::default();
        for token in value.split(',') {
            if let Some(tip) = Tip::from_token(token) {
                seen.mark(tip);
            }
        }
        seen
    }

    /// The setting's value: the seen tips' tokens in series order.
    pub fn format(self) -> String {
        Tip::ALL
            .into_iter()
            .filter(|&t| self.has(t))
            .map(Tip::token)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn has(self, tip: Tip) -> bool {
        match tip {
            Tip::Folders => self.folders,
            Tip::Search => self.search,
            Tip::Help => self.help,
        }
    }

    pub fn mark(&mut self, tip: Tip) {
        match tip {
            Tip::Folders => self.folders = true,
            Tip::Search => self.search = true,
            Tip::Help => self.help = true,
        }
    }
}

/// The tips a layout shows (Folders only where the pane folds into the
/// rail), less those already seen.
pub fn due(compact: bool, seen: Seen) -> Vec<Tip> {
    Tip::ALL
        .into_iter()
        .filter(|&t| compact || t != Tip::Folders)
        .filter(|&t| !seen.has(t))
        .collect()
}

/// Whether the What's new bar is due: there is an account (a first run
/// has nothing new to tell) and the version last told is not `current`
/// (none saved: a version from before the bar existed).
pub fn whats_new_due(saved: Option<&str>, current: &str, has_accounts: bool) -> bool {
    has_accounts && saved.map(str::trim) != Some(current)
}

/// A tip on screen with its place in the series (`n` of `of`, from 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    pub tip: Tip,
    pub n: usize,
    pub of: usize,
}

/// The tips and the What's new bar of this run.
#[derive(Debug, Default)]
pub struct Onboarding {
    seen: Seen,
    /// The running series (empty: none) and the index of the tip shown.
    series: Vec<Tip>,
    at: usize,
    /// What's new is due and has not shown yet.
    bar_due: bool,
    /// The What's new bar is up.
    bar_up: bool,
}

impl Onboarding {
    pub fn new(seen: Seen, whats_new_due: bool) -> Self {
        Onboarding {
            seen,
            bar_due: whats_new_due,
            ..Default::default()
        }
    }

    pub fn seen(&self) -> Seen {
        self.seen
    }

    /// The tip on screen, if any.
    pub fn current(&self) -> Option<Shown> {
        self.series.get(self.at).map(|&tip| Shown {
            tip,
            n: self.at + 1,
            of: self.series.len(),
        })
    }

    /// Whether a tip is due in this layout.
    pub fn tips_due(&self, compact: bool) -> bool {
        !due(compact, self.seen).is_empty()
    }

    /// Start a series of the tips due in this layout, unless one runs or
    /// the What's new bar is up. Returns the tip to show (`None`: nothing
    /// starts).
    pub fn start(&mut self, compact: bool) -> Option<Shown> {
        if !self.series.is_empty() || self.bar_up {
            return None;
        }
        self.series = due(compact, self.seen);
        self.at = 0;
        self.current()
    }

    /// Start a series and move on to `tip` without marking the ones before
    /// it (snapshots). `None` when `tip` is not due in this layout.
    pub fn start_at(&mut self, compact: bool, tip: Tip) -> Option<Shown> {
        self.series.clear();
        self.bar_up = false;
        self.start(compact)?;
        self.at = self.series.iter().position(|&t| t == tip)?;
        self.current()
    }

    /// Got it (or Esc): the tip shown is seen; the next one, if any.
    pub fn got_it(&mut self) -> Option<Shown> {
        let tip = *self.series.get(self.at)?;
        self.seen.mark(tip);
        self.at += 1;
        if self.at >= self.series.len() {
            self.end();
        }
        self.current()
    }

    /// Skip tips: every tip is seen, in either layout.
    pub fn skip(&mut self) {
        self.seen = Seen::ALL;
        self.end();
    }

    /// The window changed layout. A Folders tip can't stay once the rail
    /// is gone: the series ends there, Folders still unseen. Returns the
    /// tip shown now.
    pub fn layout(&mut self, compact: bool) -> Option<Shown> {
        if !compact && self.current().is_some_and(|s| s.tip == Tip::Folders) {
            self.end();
        }
        self.current()
    }

    /// Show tips again (Settings > About): every tip is due again.
    pub fn reset(&mut self) {
        self.seen = Seen::default();
        self.end();
    }

    fn end(&mut self) {
        self.series.clear();
        self.at = 0;
    }

    /// Whether the What's new bar may show now: due, not shown yet, and no
    /// tip on screen.
    pub fn bar_ready(&self) -> bool {
        self.bar_due && !self.bar_up && self.series.is_empty()
    }

    /// The bar is up (once a run; the caller saves its version once it is
    /// closed or acted on).
    pub fn bar_shown(&mut self) {
        self.bar_due = false;
        self.bar_up = true;
    }

    /// The bar went (closed, its action, or another status took its place).
    pub fn bar_closed(&mut self) {
        self.bar_up = false;
    }

    pub fn bar_up(&self) -> bool {
        self.bar_up
    }

    /// What's new is no longer due (the first account was just added).
    pub fn bar_cancel(&mut self) {
        self.bar_due = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(tip: Tip, n: usize, of: usize) -> Option<Shown> {
        Some(Shown { tip, n, of })
    }

    #[test]
    fn tips_seen_reads_and_writes_its_tokens() {
        assert_eq!(Seen::parse(""), Seen::default());
        let s = Seen::parse(" help, folders ,bogus,,");
        assert!(s.has(Tip::Help) && s.has(Tip::Folders) && !s.has(Tip::Search));
        assert_eq!(s.format(), "folders,help");
        assert_eq!(Seen::ALL.format(), "folders,search,help");
        assert_eq!(Seen::parse(&Seen::ALL.format()), Seen::ALL);
        assert_eq!(Seen::default().format(), "");
    }

    #[test]
    fn the_desktop_series_is_search_then_help() {
        assert_eq!(due(false, Seen::default()), [Tip::Search, Tip::Help]);
        let mut o = Onboarding::new(Seen::default(), false);
        assert_eq!(o.start(false), shown(Tip::Search, 1, 2));
        // A second trigger while it runs starts nothing.
        assert_eq!(o.start(false), None);
        assert_eq!(o.got_it(), shown(Tip::Help, 2, 2));
        assert_eq!(o.got_it(), None);
        assert_eq!(o.seen().format(), "search,help");
        // Nothing is due on the desktop any more.
        assert_eq!(o.start(false), None);
    }

    #[test]
    fn the_compact_series_starts_with_folders() {
        assert_eq!(
            due(true, Seen::default()),
            [Tip::Folders, Tip::Search, Tip::Help]
        );
        let mut o = Onboarding::new(Seen::default(), false);
        assert_eq!(o.start(true), shown(Tip::Folders, 1, 3));
        assert_eq!(o.got_it(), shown(Tip::Search, 2, 3));
        assert_eq!(o.got_it(), shown(Tip::Help, 3, 3));
        assert_eq!(o.got_it(), None);
        assert_eq!(o.seen(), Seen::ALL);
    }

    #[test]
    fn a_tip_seen_in_one_layout_is_not_repeated_in_the_other() {
        // The desktop series done, the window folds: Folders alone, 1 of 1.
        let mut o = Onboarding::new(Seen::parse("search,help"), false);
        assert_eq!(o.start(true), shown(Tip::Folders, 1, 1));
        assert_eq!(o.got_it(), None);
        assert_eq!(o.start(true), None);
        assert_eq!(o.start(false), None);
        // The compact series done: nothing on the desktop either.
        let mut o = Onboarding::new(Seen::ALL, false);
        assert_eq!(o.start(false), None);
        // Folders seen in the rail, the rest not: the desktop shows 1 of 2.
        let mut o = Onboarding::new(Seen::parse("folders"), false);
        assert_eq!(o.start(false), shown(Tip::Search, 1, 2));
        let mut o = Onboarding::new(Seen::parse("folders"), false);
        assert_eq!(o.start(true), shown(Tip::Search, 1, 2));
    }

    #[test]
    fn skip_marks_every_tip_in_either_layout() {
        let mut o = Onboarding::new(Seen::default(), false);
        o.start(false);
        o.skip();
        assert_eq!(o.current(), None);
        assert_eq!(o.seen(), Seen::ALL);
        assert!(!o.tips_due(true) && !o.tips_due(false));
        // Show tips again: all due again.
        o.reset();
        assert_eq!(o.start(true), shown(Tip::Folders, 1, 3));
    }

    #[test]
    fn a_folders_tip_ends_when_the_rail_goes() {
        let mut o = Onboarding::new(Seen::default(), false);
        o.start(true);
        assert_eq!(o.layout(true), shown(Tip::Folders, 1, 3));
        assert_eq!(o.layout(false), None);
        assert!(!o.seen().has(Tip::Folders));
        // The desktop series starts afresh.
        assert_eq!(o.start(false), shown(Tip::Search, 1, 2));
        // Search and Help stay through a fold.
        assert_eq!(o.layout(true), shown(Tip::Search, 1, 2));
    }

    #[test]
    fn snapshots_start_at_a_named_tip() {
        let mut o = Onboarding::new(Seen::ALL, false);
        assert_eq!(o.start_at(false, Tip::Help), None, "all seen");
        let mut o = Onboarding::new(Seen::default(), false);
        assert_eq!(o.start_at(false, Tip::Folders), None, "no rail");
        assert_eq!(o.start_at(false, Tip::Search), shown(Tip::Search, 1, 2));
        assert_eq!(o.start_at(true, Tip::Search), shown(Tip::Search, 2, 3));
        assert_eq!(o.start_at(true, Tip::Folders), shown(Tip::Folders, 1, 3));
        assert_eq!(o.seen(), Seen::default(), "nothing marked");
        assert_eq!(Tip::from_token("help"), Some(Tip::Help));
        assert_eq!(Tip::from_token("nope"), None);
    }

    #[test]
    fn whats_new_shows_once_per_version_with_an_account() {
        // Missing key with an account: an update from before the bar.
        assert!(whats_new_due(None, "0.3.0", true));
        assert!(whats_new_due(Some("0.2.0"), "0.3.0", true));
        assert!(!whats_new_due(Some("0.3.0"), "0.3.0", true));
        assert!(!whats_new_due(Some(" 0.3.0 "), "0.3.0", true));
        // The welcome has nothing new to tell.
        assert!(!whats_new_due(None, "0.3.0", false));
        let mut o = Onboarding::new(Seen::ALL, true);
        assert!(o.bar_ready());
        o.bar_shown();
        assert!(o.bar_up() && !o.bar_ready());
        o.bar_closed();
        assert!(!o.bar_ready(), "once");
        // The first account added: not due any more.
        let mut o = Onboarding::new(Seen::ALL, true);
        o.bar_cancel();
        assert!(!o.bar_ready());
    }

    #[test]
    fn tips_and_the_bar_never_show_together() {
        // A tip up: the bar waits until the series ends.
        let mut o = Onboarding::new(Seen::default(), true);
        assert!(o.start(false).is_some());
        assert!(!o.bar_ready());
        o.got_it();
        assert!(!o.bar_ready());
        o.got_it();
        assert!(o.bar_ready());
        // The bar up: no series starts until it goes.
        let mut o = Onboarding::new(Seen::default(), true);
        o.bar_shown();
        assert_eq!(o.start(false), None);
        o.bar_closed();
        assert_eq!(o.start(false), shown(Tip::Search, 1, 2));
        assert!(!o.bar_ready());
    }
}
