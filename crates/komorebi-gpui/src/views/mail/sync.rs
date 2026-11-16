//! What the account workers tell the window, as state (docs/GPUI-MIGRATION.md
//! phase 2, the sync parts): which accounts are syncing, each one's last
//! sync, a first download's progress, the problems already said, the
//! accounts that stay out of reach, and the status line with its action.
//! No GPUI here and no sentences: [`Said`] keeps the typed value and
//! `sync_text.rs` words it in the language shown (docs/GPUI-I18N.md). The
//! rules are the Slint app's (`App::on_event`, `sync_error`, `set_status`,
//! `problem_action`), the tests are plain `#[test]`s.
//!
//! `hub.rs` owns the workers and feeds [`Sync::on_event`] from their
//! thread's events.

use std::collections::{HashMap, HashSet};

use komorebi::mail::errors::{Attention, Class, Problem};
use komorebi::mail::{Event, FolderOp, FolderOutcome, Notice, Outcome};

use super::model::AccountState;
use crate::views::composer::Status as ComposerStatus;

/// A line stays this long when it is news, and this long when it is an
/// error (the Slint app's 5 s and 12 s; WCAG 2.2.1 asks for time to read).
pub const INFO_SECS: u64 = 5;
pub const ERROR_SECS: u64 = 12;

/// The button on the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusAction {
    #[default]
    None,
    /// Open this account's settings to update its password.
    EditAccount(i64),
    /// Let this account's workers try again (they waited for the user).
    RetryAccount(i64),
    /// This account's saved certificate changed: open its page and read the
    /// new one.
    ReviewCertificate(i64),
    /// This account's server can't be reached: one attempt now.
    TryNow(i64),
    /// Open the changelog (the What's new bar, after an update).
    WhatsNew,
    /// Take back the send of outbox row `row` while its undo window runs.
    UndoSend(i64),
}

/// The button an account's problem puts on the status line: what the
/// workers wait for decides it (a rejected sign-in: Update password; a
/// changed certificate: Review certificate; another secure-connection
/// failure: Retry), else a server out of reach offers Try now.
pub fn problem_action(id: i64, attention: Option<Attention>, problem: &Problem) -> StatusAction {
    match attention {
        Some(Attention::SignIn) => StatusAction::EditAccount(id),
        Some(Attention::Secure) if problem.changed.is_some() => StatusAction::ReviewCertificate(id),
        Some(Attention::Secure) => StatusAction::RetryAccount(id),
        None if matches!(problem.class, Class::Offline | Class::TooManyConnections) => {
            StatusAction::TryNow(id)
        }
        None => StatusAction::None,
    }
}

/// An account's last sync (this session only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Syncing,
    /// Unix seconds of the last sync that went through.
    Checked(i64),
    /// The server could not be reached.
    Unreachable,
    Failed(Problem),
}

impl State {
    /// What the list's empty state needs of it.
    pub fn for_list(&self) -> AccountState {
        match self {
            State::Syncing => AccountState::Syncing,
            State::Checked(at) => AccountState::Checked(*at),
            State::Unreachable => AccountState::Unreachable,
            State::Failed(_) => AccountState::Failed,
        }
    }
}

/// What the line says, typed: the words come from the language shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// A sentence that is already in its language (the reader's, a cache
    /// error's own words).
    Text(String),
    /// An account's problem, behind its address.
    Problem { email: String, problem: Problem },
    /// A caveat of an action that worked, behind the account's address.
    Notice { email: String, notice: Notice },
    /// A folder's headers downloading.
    Downloading {
        email: String,
        done: usize,
        total: usize,
        folder: String,
    },
    /// How a send ended.
    Outcome(Outcome),
    /// What the composer said.
    Composer(ComposerStatus),
    /// "Sending “Plans” in 10 s" with Undo; at 0 s, while the message is
    /// still being built, "Sending …".
    Sending { secs: u64, subject: String },
    /// "Message discarded."
    Discarded,
    /// A message sent as Komorebi closed could not be prepared again.
    RebuildFailed,
    /// The updater's "What's new" bar.
    WhatsNew { version: String },
    /// A failure of the cache or the system, behind a lead-in sentence.
    Detail(String),
    /// How a folder action ended (or that it waits for the server).
    Folder {
        email: String,
        op: FolderOp,
        name: String,
        outcome: FolderOutcome,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Error,
    /// An account that stays out of reach, until it is back.
    Caution,
}

/// The status line.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub said: Said,
    pub severity: Severity,
    pub action: StatusAction,
    /// How long it stays (none: until it is dismissed or replaced).
    pub for_secs: Option<u64>,
    /// A download's progress, which clears itself when the sync ends.
    pub progress: bool,
}

/// What the hub asks of the views after an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Reaction {
    /// The folder list changed: reload folders, counts and rows.
    FoldersChanged,
    /// Rows of this folder changed.
    RowsChanged(i64),
    /// A message body arrived.
    BodyReady(i64),
    /// A message body could not be downloaded.
    BodyFailed { id: i64, problem: Problem },
    /// A send ended; its outbox row is settled.
    Sent {
        reply_to: Option<i64>,
        outcome: Outcome,
    },
    /// An account's sync ended without a problem: sign its interactive
    /// worker in now.
    Warm(i64),
    /// Queue a sync of this folder (`full`: skip the STATUS gate).
    SyncFolder { id: i64, full: bool },
    /// A draft's server copy was replaced from another account: remove it.
    DraftCopyLeft(komorebi::db::ServerCopy),
    /// A sync pass ended well on all accounts: "Updated 14:05".
    Updated,
}

/// What the state needs to know of the app to judge an event.
pub struct Env<'a> {
    /// Addresses by account.
    pub email: &'a dyn Fn(i64) -> String,
    /// What an account's workers wait for.
    pub attention: &'a dyn Fn(i64) -> Option<Attention>,
    /// A folder's name, for the download line.
    pub folder_name: &'a dyn Fn(i64) -> String,
    /// Now, in Unix seconds.
    pub now: i64,
}

#[derive(Default)]
pub struct Sync {
    /// Accounts with a sync running.
    syncing: HashSet<i64>,
    /// Each account's last sync.
    states: HashMap<i64, State>,
    /// A folder's download: (done, total) by account.
    progress: HashMap<i64, (usize, usize)>,
    /// Accounts in their first download (a new account).
    first: HashSet<i64>,
    /// The problem already said for each account, until it clears.
    shown: HashMap<i64, (Class, String)>,
    /// Accounts out of reach, said again until they are back.
    standing: Vec<(i64, Problem)>,
    /// Accounts whose interactive worker was signed in.
    warmed: HashSet<i64>,
    /// The last sync pass that went through (Unix seconds).
    updated: Option<i64>,
    line: Option<Line>,
    /// A download line is showing (not a newer line).
    progress_shown: bool,
}

impl Sync {
    pub fn line(&self) -> Option<&Line> {
        self.line.as_ref()
    }

    pub fn syncing(&self) -> bool {
        !self.syncing.is_empty()
    }

    pub fn is_syncing(&self, account: i64) -> bool {
        self.syncing.contains(&account)
    }

    pub fn state(&self, account: i64) -> Option<&State> {
        self.states.get(&account)
    }

    /// The accounts' states as the list reads them.
    pub fn list_states(&self) -> HashMap<i64, AccountState> {
        self.states
            .iter()
            .map(|(id, s)| (*id, s.for_list()))
            .collect()
    }

    /// The caution badge of an account's row and rail tile: its server was
    /// out of reach at the last sync, until one goes through.
    pub fn badge(&self, account: i64) -> bool {
        matches!(self.states.get(&account), Some(State::Unreachable))
    }

    /// The server can't be reached now (no worker, or the last sync could
    /// not reach it): a message not downloaded says so.
    pub fn unreachable(&self, account: i64, has_workers: bool) -> bool {
        !has_workers
            || matches!(
                self.states.get(&account),
                Some(State::Unreachable | State::Failed(_))
            )
    }

    /// Accounts in their first download, with how far they are.
    pub fn first_downloads(&self) -> HashMap<i64, (usize, usize)> {
        self.first
            .iter()
            .map(|id| (*id, self.progress.get(id).copied().unwrap_or((0, 0))))
            .collect()
    }

    pub fn progress(&self, account: i64) -> Option<(usize, usize)> {
        self.progress.get(&account).copied()
    }

    /// Unix seconds of the last sync pass that went through ("Updated").
    pub fn updated(&self) -> Option<i64> {
        self.updated
    }

    /// A new account downloads its mail for the first time.
    pub fn begin_first_download(&mut self, account: i64) {
        self.first.insert(account);
    }

    /// An account's workers are replaced or gone.
    pub fn forget(&mut self, account: i64) {
        self.syncing.remove(&account);
        self.warmed.remove(&account);
        self.states.remove(&account);
        self.progress.remove(&account);
        self.first.remove(&account);
        self.shown.remove(&account);
        self.standing.retain(|(a, _)| *a != account);
    }

    /// Refresh: a problem shows again if it is still there.
    pub fn refreshing(&mut self) {
        self.shown.clear();
    }

    /// Retry on a problem: it shows again if it returns.
    pub fn retrying(&mut self, account: i64) {
        self.shown.remove(&account);
    }

    /// "Try now": the line goes, it comes back if the server is still out
    /// of reach.
    pub fn trying_now(&mut self, account: i64) {
        self.standing.retain(|(a, _)| *a != account);
        self.shown.remove(&account);
        self.say(Said::Text(String::new()), false);
    }

    /// Show `said` (news for 5 s, an error for 12 s). An empty text clears
    /// the line, or shows the latest account out of reach, until that
    /// changes.
    pub fn say(&mut self, said: Said, error: bool) {
        self.progress_shown = false;
        let empty = matches!(&said, Said::Text(t) if t.is_empty());
        if empty && let Some((id, problem)) = self.standing.last().cloned() {
            self.line = Some(Line {
                said: Said::Problem {
                    // The address is filled in by `with_standing_email`.
                    email: String::new(),
                    problem,
                },
                severity: Severity::Caution,
                action: StatusAction::TryNow(id),
                for_secs: None,
                progress: false,
            });
            return;
        }
        self.line = if empty {
            None
        } else {
            Some(Line {
                said,
                severity: if error {
                    Severity::Error
                } else {
                    Severity::Info
                },
                action: StatusAction::None,
                for_secs: Some(if error { ERROR_SECS } else { INFO_SECS }),
                progress: false,
            })
        };
    }

    /// Put an action button on the line just set.
    pub fn set_action(&mut self, action: StatusAction) {
        if let Some(line) = &mut self.line {
            line.action = action;
        }
    }

    /// A send that failed for good stays said until it is dismissed.
    pub fn keep_line(&mut self) {
        if let Some(line) = &mut self.line {
            line.for_secs = None;
        }
    }

    /// The line was closed (its button, or Esc): an account out of reach is
    /// not said again until it syncs or Refresh.
    pub fn dismiss(&mut self) {
        if self
            .line
            .as_ref()
            .is_some_and(|l| l.severity == Severity::Caution)
        {
            self.standing.clear();
        }
        self.say(Said::Text(String::new()), false);
    }

    /// The line's account out of reach has an address now (the state keeps
    /// no addresses).
    pub fn with_standing_email(&mut self, email: &dyn Fn(i64) -> String) {
        if let Some(Line {
            said: Said::Problem { email: shown, .. },
            action: StatusAction::TryNow(id),
            severity: Severity::Caution,
            ..
        }) = &mut self.line
        {
            *shown = email(*id);
        }
    }

    /// An event from the workers. Returns what the views must do.
    pub fn on_event(&mut self, event: Event, env: &Env) -> Vec<Reaction> {
        let mut out = Vec::new();
        match event {
            Event::Syncing {
                account_id,
                active,
                error,
            } => self.on_syncing(account_id, active, error, env, &mut out),
            Event::Progress {
                account_id,
                folder_id,
                done,
                total,
            } => self.on_progress(account_id, folder_id, done, total, env),
            Event::FolderChanged { folder_id: None } => out.push(Reaction::FoldersChanged),
            Event::FolderChanged { folder_id: Some(f) } => out.push(Reaction::RowsChanged(f)),
            Event::BodyReady { message_id } => out.push(Reaction::BodyReady(message_id)),
            Event::BodyFailed { message_id, error } => out.push(Reaction::BodyFailed {
                id: message_id,
                problem: error,
            }),
            Event::Sent {
                reply_to, outcome, ..
            } => {
                let error = !matches!(outcome, Outcome::Delivered);
                // A send that failed for good stays said until dismissed.
                let stay = matches!(outcome, Outcome::Failed(_) | Outcome::Unconfirmed(_));
                self.say(Said::Outcome(outcome.clone()), error);
                if stay {
                    self.keep_line();
                }
                out.push(Reaction::Sent { reply_to, outcome });
            }
            Event::ActionFailed { account_id, error } => {
                self.on_problem(account_id, &error, env);
            }
            Event::NeedsSync { folder_id } => out.push(Reaction::SyncFolder {
                id: folder_id,
                full: true,
            }),
            Event::RemoteChanged { folder_id } => out.push(Reaction::SyncFolder {
                id: folder_id,
                full: false,
            }),
            Event::Notice {
                account_id,
                message,
            } => self.say(
                Said::Notice {
                    email: (env.email)(account_id),
                    notice: message,
                },
                false,
            ),
            Event::DraftCopyLeft(copy) => out.push(Reaction::DraftCopyLeft(copy)),
            Event::FolderDone {
                account_id,
                op,
                name,
                outcome,
            } => {
                let error = !matches!(outcome, FolderOutcome::Done | FolderOutcome::Waiting);
                self.say(
                    Said::Folder {
                        email: (env.email)(account_id),
                        op,
                        name,
                        outcome,
                    },
                    error,
                );
            }
        }
        out
    }

    fn on_syncing(
        &mut self,
        id: i64,
        active: bool,
        error: Option<Problem>,
        env: &Env,
        out: &mut Vec<Reaction>,
    ) {
        if active {
            self.syncing.insert(id);
        } else {
            self.syncing.remove(&id);
            self.end_progress(id);
            self.first.remove(&id);
            if self.syncing.is_empty() && error.is_none() {
                self.updated = Some(env.now);
                out.push(Reaction::Updated);
            }
        }
        let state = match &error {
            _ if active => State::Syncing,
            None => State::Checked(env.now),
            Some(p) if p.class == Class::Offline => State::Unreachable,
            Some(p) => State::Failed(p.clone()),
        };
        self.states.insert(id, state);
        out.push(Reaction::FoldersChanged);
        if let Some(p) = error {
            self.on_problem(id, &p, env);
        } else if !active {
            // It went through: a problem shows again if it returns, and
            // the line saying it can't be reached goes.
            self.shown.remove(&id);
            let before = self.standing.len();
            self.standing.retain(|(a, _)| *a != id);
            if self.standing.len() < before
                && self
                    .line
                    .as_ref()
                    .is_some_and(|l| l.severity == Severity::Caution)
            {
                self.say(Said::Text(String::new()), false);
            }
            if self.warmed.insert(id) {
                out.push(Reaction::Warm(id));
            }
        }
    }

    /// Show an account's problem, once until it clears (a sync that goes
    /// through, or Refresh); a rejected sign-in offers to update the
    /// password.
    fn on_problem(&mut self, id: i64, problem: &Problem, env: &Env) {
        let (class, text) = problem.key();
        let key = (class, text.to_string());
        if self.shown.get(&id) == Some(&key) {
            return;
        }
        self.shown.insert(id, key);
        self.standing.retain(|(a, _)| *a != id);
        // A server out of reach stays said until it is back (FLUENT2 "Wait
        // UX": attached to what it concerns); it waits for an offer (Undo)
        // to end rather than take it away.
        if matches!(problem.class, Class::Offline | Class::TooManyConnections) {
            self.standing.push((id, problem.clone()));
            if !self
                .line
                .as_ref()
                .is_some_and(|l| matches!(l.action, StatusAction::UndoSend(_)))
            {
                self.say(Said::Text(String::new()), false);
                self.with_standing_email(env.email);
            }
            return;
        }
        self.say(
            Said::Problem {
                email: (env.email)(id),
                problem: problem.clone(),
            },
            true,
        );
        let action = problem_action(id, (env.attention)(id), problem);
        if action != StatusAction::None {
            self.set_action(action);
        }
    }

    /// A folder's headers are downloading: the status line says how far,
    /// while it shows nothing else.
    fn on_progress(&mut self, id: i64, folder: i64, done: usize, total: usize, env: &Env) {
        self.progress.insert(id, (done, total));
        if self.line.is_none() || self.progress_shown {
            self.line = Some(Line {
                said: Said::Downloading {
                    email: (env.email)(id),
                    done,
                    total,
                    folder: (env.folder_name)(folder),
                },
                severity: Severity::Info,
                action: StatusAction::None,
                for_secs: None,
                progress: true,
            });
            self.progress_shown = true;
        }
    }

    /// An account's sync ended: its download progress goes.
    fn end_progress(&mut self, id: i64) {
        if self.progress.remove(&id).is_some() && self.progress.is_empty() && self.progress_shown {
            self.say(Said::Text(String::new()), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(now: i64) -> Env<'static> {
        Env {
            email: &|id| format!("a{id}@example.com"),
            attention: &|_| None,
            folder_name: &|_| "Inbox".into(),
            now,
        }
    }

    fn offline() -> Problem {
        Problem::of(&anyhow::Error::new(std::io::Error::from(
            std::io::ErrorKind::TimedOut,
        )))
    }

    fn syncing(id: i64, active: bool, error: Option<Problem>) -> Event {
        Event::Syncing {
            account_id: id,
            active,
            error,
        }
    }

    /// Each account problem puts its own fix on the status line (the Slint
    /// app's `a_problem_maps_to_its_status_action`).
    #[test]
    fn a_problem_maps_to_its_status_action() {
        let changed =
            Problem::of(&komorebi::mail::pin::PinMismatch("imap.example.com".into()).into());
        assert!(changed.changed.is_some());
        let untrusted = Problem {
            changed: None,
            ..changed.clone()
        };
        let other = Problem::of(&anyhow::anyhow!("opening folder X"));
        assert_eq!(
            problem_action(7, Some(Attention::Secure), &changed),
            StatusAction::ReviewCertificate(7)
        );
        assert_eq!(
            problem_action(7, Some(Attention::Secure), &untrusted),
            StatusAction::RetryAccount(7)
        );
        assert_eq!(
            problem_action(7, Some(Attention::SignIn), &other),
            StatusAction::EditAccount(7)
        );
        assert_eq!(problem_action(7, None, &offline()), StatusAction::TryNow(7));
        assert_eq!(problem_action(7, None, &other), StatusAction::None);
    }

    /// A sync marks the account syncing, then checked; the pass that ends
    /// the last sync says "Updated", and the first one signs the
    /// interactive worker in.
    #[test]
    fn a_sync_runs_then_the_account_is_checked_and_warmed_once() {
        let mut s = Sync::default();
        let out = s.on_event(syncing(1, true, None), &env(100));
        assert!(s.syncing() && s.is_syncing(1));
        assert_eq!(s.state(1), Some(&State::Syncing));
        assert_eq!(out, vec![Reaction::FoldersChanged]);
        let out = s.on_event(syncing(1, false, None), &env(130));
        assert!(!s.syncing());
        assert_eq!(s.state(1), Some(&State::Checked(130)));
        assert_eq!(s.updated(), Some(130));
        assert!(out.contains(&Reaction::Updated));
        assert!(out.contains(&Reaction::Warm(1)));
        s.on_event(syncing(1, true, None), &env(200));
        let out = s.on_event(syncing(1, false, None), &env(230));
        assert!(!out.contains(&Reaction::Warm(1)), "warmed once");
    }

    /// A server out of reach: a caution line with Try now stays, the badge
    /// shows, and a sync that goes through takes both away.
    #[test]
    fn an_unreachable_server_stays_said_with_a_badge_until_it_is_back() {
        let mut s = Sync::default();
        s.on_event(syncing(1, true, None), &env(1));
        s.on_event(syncing(1, false, Some(offline())), &env(2));
        assert_eq!(s.state(1), Some(&State::Unreachable));
        assert!(s.badge(1));
        assert!(s.unreachable(1, true));
        let line = s.line().expect("a standing line").clone();
        assert_eq!(line.severity, Severity::Caution);
        assert_eq!(line.action, StatusAction::TryNow(1));
        assert_eq!(line.for_secs, None);
        // The same problem again is not said again.
        s.dismiss();
        assert!(s.line().is_none(), "dismissing clears it");
        s.on_event(syncing(1, true, None), &env(3));
        s.on_event(syncing(1, false, Some(offline())), &env(4));
        assert!(s.line().is_none(), "dismissed, and not shown twice");
        // Refresh shows it again.
        s.refreshing();
        s.on_event(syncing(1, false, Some(offline())), &env(5));
        assert_eq!(s.line().map(|l| l.severity), Some(Severity::Caution));
        // Back: the line and the badge go.
        s.on_event(syncing(1, false, None), &env(6));
        assert!(s.line().is_none());
        assert!(!s.badge(1));
    }

    /// Another problem is an error line for 12 s with its action; the same
    /// problem is said once.
    #[test]
    fn a_failed_sync_is_said_once_with_its_action() {
        let mut s = Sync::default();
        let e = Env {
            attention: &|_| Some(Attention::SignIn),
            ..env(1)
        };
        let rejected = Problem::of(&anyhow::anyhow!("no"));
        s.on_event(syncing(2, false, Some(rejected.clone())), &e);
        let line = s.line().expect("a line").clone();
        assert_eq!(line.severity, Severity::Error);
        assert_eq!(line.for_secs, Some(ERROR_SECS));
        assert_eq!(line.action, StatusAction::EditAccount(2));
        assert!(matches!(line.said, Said::Problem { ref email, .. } if email == "a2@example.com"));
        s.say(Said::Text("later".into()), false);
        s.on_event(syncing(2, false, Some(rejected)), &e);
        assert_eq!(
            s.line().map(|l| l.said.clone()),
            Some(Said::Text("later".into())),
            "not said twice"
        );
    }

    /// The download line shows only while nothing else is said, follows the
    /// count, and goes with the sync.
    #[test]
    fn a_first_download_shows_its_progress_and_ends_with_the_sync() {
        let mut s = Sync::default();
        s.begin_first_download(3);
        s.on_event(syncing(3, true, None), &env(1));
        let p = |done, total| Event::Progress {
            account_id: 3,
            folder_id: 9,
            done,
            total,
        };
        s.on_event(p(100, 5000), &env(2));
        s.on_event(p(300, 5000), &env(3));
        assert_eq!(s.progress(3), Some((300, 5000)));
        assert_eq!(s.first_downloads().get(&3), Some(&(300, 5000)));
        assert!(matches!(
            s.line().map(|l| &l.said),
            Some(Said::Downloading {
                done: 300,
                total: 5000,
                ..
            })
        ));
        s.on_event(syncing(3, false, None), &env(4));
        assert!(s.line().is_none());
        assert!(s.first_downloads().is_empty());
        // Something else said first stays.
        s.say(Said::Discarded, false);
        s.on_event(p(1, 2), &env(5));
        assert_eq!(s.line().map(|l| l.said.clone()), Some(Said::Discarded));
    }

    /// A failed send stays said; a delivered one goes after 5 s.
    #[test]
    fn a_send_that_failed_for_good_stays_on_the_line() {
        let mut s = Sync::default();
        let sent = |outcome| Event::Sent {
            row: 1,
            reply_to: Some(5),
            outcome,
        };
        let out = s.on_event(sent(Outcome::Delivered), &env(1));
        assert_eq!(s.line().and_then(|l| l.for_secs), Some(INFO_SECS));
        assert_eq!(
            out,
            vec![Reaction::Sent {
                reply_to: Some(5),
                outcome: Outcome::Delivered
            }]
        );
        s.on_event(sent(Outcome::Failed("refused".into())), &env(2));
        let line = s.line().expect("line");
        assert_eq!(line.severity, Severity::Error);
        assert_eq!(line.for_secs, None);
    }

    /// IDLE and our own moves ask for syncs of a folder: the first through
    /// the STATUS gate, the second in full.
    #[test]
    fn remote_changes_ask_for_folder_syncs() {
        let mut s = Sync::default();
        assert_eq!(
            s.on_event(Event::RemoteChanged { folder_id: 4 }, &env(1)),
            vec![Reaction::SyncFolder { id: 4, full: false }]
        );
        assert_eq!(
            s.on_event(Event::NeedsSync { folder_id: 4 }, &env(1)),
            vec![Reaction::SyncFolder { id: 4, full: true }]
        );
    }

    /// An action that failed says why once; the standing line is not
    /// replaced by an Undo offer's end.
    #[test]
    fn a_notice_is_news_behind_the_address() {
        let mut s = Sync::default();
        s.on_event(
            Event::Notice {
                account_id: 1,
                message: Notice::MoveGone,
            },
            &env(1),
        );
        let line = s.line().expect("line");
        assert_eq!(line.severity, Severity::Info);
        assert!(matches!(&line.said, Said::Notice { email, .. } if email == "a1@example.com"));
    }

    /// A removed account's state is forgotten.
    #[test]
    fn forgetting_an_account_clears_what_was_known_of_it() {
        let mut s = Sync::default();
        s.begin_first_download(1);
        s.on_event(syncing(1, true, None), &env(1));
        s.on_event(syncing(1, false, Some(offline())), &env(2));
        s.forget(1);
        assert!(s.state(1).is_none() && !s.badge(1) && !s.syncing());
        s.on_event(syncing(1, false, Some(offline())), &env(3));
        assert!(
            s.line().is_some(),
            "said again after the account was replaced"
        );
    }
}
