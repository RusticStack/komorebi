//! Notification policy: pure logic, no Windows calls, no message content kept.
//!
//! The app feeds [`Notifier::decide`] the messages a sync just stored as new
//! and the user's settings; it returns what (if anything) to show. The result
//! is structured, not English: the UI layer renders it through `Tr`.
//! Message ids are remembered (ids only) so a message never notifies twice.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

/// How much of a message a toast may show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Content {
    /// Sender and subject.
    #[default]
    SenderSubject,
    /// Only "New mail".
    Generic,
}

/// One newly arrived message, as reported by a sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMessage {
    pub account_id: i64,
    pub message_id: i64,
    pub sender: String,
    pub subject: String,
}

/// User settings that gate notifications.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub paused: bool,
    pub content: Content,
    /// Accounts with notifications on. An account not listed stays silent.
    pub enabled_accounts: HashSet<i64>,
}

/// Where a click on the toast should go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub account_id: i64,
    pub message_id: i64,
}

/// What to show. The UI layer turns this into translated text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Toast {
    /// One message, sender and subject allowed.
    Message {
        sender: String,
        subject: String,
        target: Target,
    },
    /// One message, content hidden ("New mail").
    NewMail { target: Target },
    /// Several messages: "N new messages". Click opens the newest.
    Burst { count: usize, target: Target },
}

const SEEN_CAP: usize = 4096;

/// Dedupe memory plus the decision function.
#[derive(Default)]
pub struct Notifier {
    seen: HashSet<(i64, i64)>,
    order: VecDeque<(i64, i64)>,
}

impl Notifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the ids as seen (so they never notify later) and returns the
    /// toast to show, if any. Messages are marked seen even when suppressed:
    /// a paused or focused window must not replay them on resume.
    pub fn decide(
        &mut self,
        events: &[NewMessage],
        settings: &Settings,
        window_focused: bool,
    ) -> Option<Toast> {
        let mut fresh: Vec<&NewMessage> = Vec::new();
        for e in events {
            if self.remember((e.account_id, e.message_id)) {
                fresh.push(e);
            }
        }
        if settings.paused || window_focused {
            return None;
        }
        fresh.retain(|e| settings.enabled_accounts.contains(&e.account_id));
        let newest = *fresh.last()?;
        let target = Target {
            account_id: newest.account_id,
            message_id: newest.message_id,
        };
        Some(match (fresh.len(), settings.content) {
            (1, Content::SenderSubject) => Toast::Message {
                sender: newest.sender.clone(),
                subject: newest.subject.clone(),
                target,
            },
            (1, Content::Generic) => Toast::NewMail { target },
            (n, _) => Toast::Burst { count: n, target },
        })
    }

    /// Forgets everything (an account was removed or the cache reset).
    pub fn clear(&mut self) {
        self.seen.clear();
        self.order.clear();
    }

    fn remember(&mut self, key: (i64, i64)) -> bool {
        if !self.seen.insert(key) {
            return false;
        }
        self.order.push_back(key);
        while self.order.len() > SEEN_CAP {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

/// Argument the Run key adds so a Windows-start launch stays in the tray.
pub const MINIMIZED_ARG: &str = "--minimized";

/// True when the launch arguments ask for a start-minimised (tray only) run.
pub fn launched_minimized<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == MINIMIZED_ARG)
}

/// The command line stored in the Run key for `exe`.
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\" {}", exe.display(), MINIMIZED_ARG)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(a: i64, m: i64) -> NewMessage {
        NewMessage {
            account_id: a,
            message_id: m,
            sender: "Ana".into(),
            subject: "Hi".into(),
        }
    }
    fn on(accounts: &[i64]) -> Settings {
        Settings {
            enabled_accounts: accounts.iter().copied().collect(),
            ..Settings::default()
        }
    }
    fn tgt(a: i64, m: i64) -> Target {
        Target {
            account_id: a,
            message_id: m,
        }
    }

    #[test]
    fn single_message_shows_sender_and_subject() {
        let mut n = Notifier::new();
        let t = n.decide(&[msg(1, 10)], &on(&[1]), false);
        assert!(matches!(t, Some(Toast::Message { .. })));
    }

    #[test]
    fn generic_level_hides_content() {
        let mut n = Notifier::new();
        let mut s = on(&[1]);
        s.content = Content::Generic;
        assert_eq!(
            n.decide(&[msg(1, 10)], &s, false),
            Some(Toast::NewMail { target: tgt(1, 10) })
        );
    }

    #[test]
    fn burst_coalesces_and_targets_newest() {
        let mut n = Notifier::new();
        let t = n.decide(&[msg(1, 1), msg(1, 2), msg(1, 3)], &on(&[1]), false);
        assert_eq!(
            t,
            Some(Toast::Burst {
                count: 3,
                target: tgt(1, 3)
            })
        );
    }

    #[test]
    fn dedupes_by_message_id() {
        let mut n = Notifier::new();
        assert!(n.decide(&[msg(1, 1)], &on(&[1]), false).is_some());
        assert!(n.decide(&[msg(1, 1)], &on(&[1]), false).is_none());
        // Same message id on another account is a different message.
        assert!(n.decide(&[msg(2, 1)], &on(&[2]), false).is_some());
    }

    #[test]
    fn paused_focused_and_disabled_stay_silent_without_replay() {
        let mut n = Notifier::new();
        let mut s = on(&[1]);
        s.paused = true;
        assert!(n.decide(&[msg(1, 1)], &s, false).is_none());
        s.paused = false;
        assert!(n.decide(&[msg(1, 1)], &s, false).is_none());
        assert!(n.decide(&[msg(1, 2)], &s, true).is_none());
        assert!(n.decide(&[msg(1, 3)], &on(&[]), false).is_none());
        assert!(
            n.decide(&[msg(1, 4), msg(2, 5)], &on(&[2]), false)
                .is_some()
        );
    }

    #[test]
    fn disabled_accounts_do_not_count_in_a_burst() {
        let mut n = Notifier::new();
        let t = n.decide(&[msg(1, 1), msg(2, 2)], &on(&[2]), false);
        assert!(matches!(t, Some(Toast::Message { .. })));
    }

    #[test]
    fn seen_memory_is_bounded() {
        let mut n = Notifier::new();
        for i in 0..(SEEN_CAP as i64 + 10) {
            n.decide(&[msg(1, i)], &on(&[]), false);
        }
        assert!(n.seen.len() <= SEEN_CAP);
    }

    #[test]
    fn focused_window_is_silent_and_does_not_replay() {
        let mut n = Notifier::new();
        assert!(n.decide(&[msg(1, 1)], &on(&[1]), true).is_none());
        assert!(n.decide(&[msg(1, 1)], &on(&[1]), false).is_none());
        assert!(n.decide(&[msg(1, 2)], &on(&[1]), false).is_some());
    }

    #[test]
    fn generic_burst_and_empty_events() {
        let mut n = Notifier::new();
        let mut s = on(&[1]);
        s.content = Content::Generic;
        assert!(n.decide(&[], &s, false).is_none());
        assert_eq!(
            n.decide(&[msg(1, 1), msg(1, 2)], &s, false),
            Some(Toast::Burst {
                count: 2,
                target: tgt(1, 2)
            })
        );
    }

    #[test]
    fn sender_subject_toast_carries_text_and_target() {
        let mut n = Notifier::new();
        assert_eq!(
            n.decide(&[msg(3, 7)], &on(&[3]), false),
            Some(Toast::Message {
                sender: "Ana".into(),
                subject: "Hi".into(),
                target: tgt(3, 7)
            })
        );
    }

    #[test]
    fn duplicates_inside_one_batch_count_once() {
        let mut n = Notifier::new();
        let t = n.decide(&[msg(1, 1), msg(1, 1)], &on(&[1]), false);
        assert!(matches!(t, Some(Toast::Message { .. })));
    }

    #[test]
    fn oldest_remembered_id_is_evicted_at_the_cap() {
        let mut n = Notifier::new();
        for i in 0..=(SEEN_CAP as i64) {
            n.decide(&[msg(1, i)], &on(&[1]), false);
        }
        assert_eq!(n.seen.len(), SEEN_CAP);
        // Id 0 was forgotten; the newest is still remembered.
        assert!(n.decide(&[msg(1, 0)], &on(&[1]), false).is_some());
        assert!(
            n.decide(&[msg(1, SEEN_CAP as i64)], &on(&[1]), false)
                .is_none()
        );
    }

    #[test]
    fn clear_forgets_everything() {
        let mut n = Notifier::new();
        n.decide(&[msg(1, 1)], &on(&[1]), false);
        n.clear();
        assert!(n.decide(&[msg(1, 1)], &on(&[1]), false).is_some());
    }

    #[test]
    fn minimized_flag_edge_cases() {
        assert!(!launched_minimized(Vec::<String>::new()));
        assert!(!launched_minimized(["--minimized-not"]));
    }

    #[test]
    fn minimized_flag_and_run_command() {
        assert!(launched_minimized(["komorebi.exe", "--minimized"]));
        assert!(!launched_minimized(["komorebi.exe"]));
        assert_eq!(
            run_command(Path::new("C:\\K\\komorebi.exe")),
            "\"C:\\K\\komorebi.exe\" --minimized"
        );
    }
}
