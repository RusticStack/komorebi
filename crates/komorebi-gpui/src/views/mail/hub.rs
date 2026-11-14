//! The account workers and what they say (docs/GPUI-MIGRATION.md phase 2,
//! the sync parts): the [`Hub`] starts one [`Workers`] per enabled account,
//! takes their events from their threads onto the UI thread, keeps the
//! state of `sync.rs`, sends the jobs the views queue, runs the periodic
//! check and the Outbox pass, and owns the status line.
//!
//! Views never touch the workers. They read [`Hub::sync`], send jobs with
//! [`Hub::send`] and hear [`HubEvent`]s.
//!
//! No network in development: with `KOMOREBI_OFFLINE` or `KOMOREBI_SNAPSHOT`
//! set no worker is started ([`Mode::Off`]); in tests the workers are
//! [`Workers::capture`]d, so the jobs the views queue can be read back and
//! nothing connects ([`Mode::Capture`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{Context, EventEmitter, Task};
use komorebi::db::{Account, Folder};
use komorebi::mail::errors::Attention;
use komorebi::mail::{Event, Job, Notify, Workers};

use super::sync::{Env, Line, Reaction, Said, StatusAction, Sync};
use crate::views::Db;
use crate::views::composer::ComposerEvent;
use crate::views::reader::ReaderEvent;

/// A full sync at least this often (the Slint app's `FULL_SYNC_EVERY`).
pub const FULL_SYNC_EVERY: Duration = Duration::from_secs(60 * 60);
/// Whether the workers run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Workers connect to the servers.
    Live,
    /// Workers that run nothing and connect nowhere: every job arrives on
    /// a receiver ([`Hub::captured`]).
    Capture,
    /// No workers: jobs are dropped (development switches).
    Off,
}

impl Mode {
    /// `Off` under `KOMOREBI_OFFLINE` or `KOMOREBI_SNAPSHOT`; `Capture` in
    /// tests; else `Live`.
    pub fn detect() -> Mode {
        if komorebi::dev_var("KOMOREBI_OFFLINE").is_some()
            || komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some()
        {
            Mode::Off
        } else if cfg!(test) {
            Mode::Capture
        } else {
            Mode::Live
        }
    }
}

/// What the hub tells the views.
#[derive(Clone, Debug, PartialEq)]
pub enum HubEvent {
    /// The state changed (syncing, an account's state, progress, the line).
    Changed,
    /// Do this (reload the folders, a body arrived, a send ended).
    React(Reaction),
    /// Undo on the "Sending in 10 s" offer: the composer that holds the
    /// pending send takes it back.
    UndoSend(i64),
    /// The status line's "Update password" and "Review certificate": open
    /// that account's page in Settings.
    EditAccount { id: i64, certificate: bool },
    /// The What's new bar's button (open the changelog).
    WhatsNew,
    /// The periodic check ran (the contacts' CardDAV passes ride on it).
    Tick,
}

/// A send waiting out its undo window (the line counts it down). `until`
/// is on the executor's clock, so tests move it.
struct Sending {
    row: i64,
    subject: String,
    until: Instant,
}

pub struct Hub {
    db: Db,
    mode: Mode,
    accounts: Vec<Account>,
    folders: Vec<Folder>,
    workers: HashMap<i64, Workers>,
    captured: Vec<(i64, Receiver<Job>)>,
    sync: Sync,
    tx: UnboundedSender<Event>,
    /// The status line changed this many times (a timer ends the one it
    /// was started for).
    line_seq: u64,
    sending: Option<Sending>,
    interval: Duration,
    last_full: Option<Instant>,
    started: bool,
    _pump: Task<()>,
    _tick: Option<Task<()>>,
    /// Tells the hub when the computer woke from sleep ([`crate::wake`]).
    _wake: Option<Task<()>>,
}

impl EventEmitter<HubEvent> for Hub {}

impl Hub {
    /// The hub for cache `db`, with the workers of its enabled accounts
    /// ([`Mode::detect`]). Nothing syncs until [`Hub::start`].
    pub fn new(db: Db, cx: &mut Context<Self>) -> Self {
        Self::with_mode(db, Mode::detect(), cx)
    }

    pub fn with_mode(db: Db, mode: Mode, cx: &mut Context<Self>) -> Self {
        let (tx, mut rx) = unbounded::<Event>();
        let pump = cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |hub, cx| hub.on_event(event, cx)).is_err() {
                    break;
                }
            }
        });
        let accounts = db.accounts().unwrap_or_default();
        let folders = db.folders().unwrap_or_default();
        let interval = db
            .settings()
            .map(|s| Duration::from_secs(s.check_interval_secs))
            .unwrap_or(Duration::from_secs(300));
        let mut hub = Hub {
            db,
            mode,
            accounts,
            folders,
            workers: HashMap::new(),
            captured: Vec::new(),
            sync: Sync::default(),
            tx,
            line_seq: 0,
            sending: None,
            interval,
            last_full: None,
            started: false,
            _pump: pump,
            _tick: None,
            _wake: None,
        };
        for account in hub.accounts.clone().into_iter().filter(|a| a.enabled) {
            hub.spawn_workers(account);
        }
        hub
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn sync(&self) -> &Sync {
        &self.sync
    }

    /// Whether the account has workers (it is enabled and they run).
    pub fn has_workers(&self, account: i64) -> bool {
        self.workers.contains_key(&account)
    }

    /// What the account's workers wait for the user to fix.
    pub fn attention(&self, account: i64) -> Option<Attention> {
        self.workers.get(&account).and_then(Workers::attention)
    }

    /// The server of this account can't be reached now.
    pub fn unreachable(&self, account: i64) -> bool {
        self.sync.unreachable(account, self.has_workers(account))
    }

    /// The jobs captured so far, by account ([`Mode::Capture`], tests).
    pub fn captured(&self) -> Vec<(i64, Job)> {
        let mut jobs = Vec::new();
        for (account, rx) in &self.captured {
            jobs.extend(rx.try_iter().map(|job| (*account, job)));
        }
        jobs
    }

    // ------------------------------------------------------- the workers

    fn notify(&self) -> Notify {
        let tx = self.tx.clone();
        Arc::new(move |event| {
            tx.unbounded_send(event).ok();
        })
    }

    fn db_path(&self) -> Option<PathBuf> {
        self.db.file()
    }

    fn spawn_workers(&mut self, account: Account) {
        let id = account.id;
        let Some(path) = self.db_path() else {
            return;
        };
        match self.mode {
            Mode::Off => {}
            Mode::Live => {
                let workers = Workers::spawn(account, path, self.notify());
                self.workers.insert(id, workers);
            }
            Mode::Capture => {
                let (workers, rx) = Workers::capture(account, path, self.notify());
                self.workers.insert(id, workers);
                self.captured.push((id, rx));
            }
        }
    }

    /// A new account (Settings, onboarding): its workers start, it
    /// downloads its mail for the first time.
    pub fn add_account(&mut self, account: Account, cx: &mut Context<Self>) {
        let id = account.id;
        self.accounts = self.db.accounts().unwrap_or_default();
        self.sync.begin_first_download(id);
        self.spawn_workers(account);
        self.send(id, Job::SyncAll { full: true });
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// An account's servers or settings changed: its workers are replaced
    /// and it syncs in full. The old ones finish their job and log out.
    pub fn respawn(&mut self, id: i64, cx: &mut Context<Self>) {
        self.workers.remove(&id);
        self.captured.retain(|(a, _)| *a != id);
        self.sync.forget(id);
        self.accounts = self.db.accounts().unwrap_or_default();
        if let Some(account) = self
            .accounts
            .iter()
            .find(|a| a.id == id && a.enabled)
            .cloned()
        {
            self.spawn_workers(account);
            self.send(id, Job::SyncAll { full: true });
        }
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// An account was removed.
    pub fn remove_account(&mut self, id: i64, cx: &mut Context<Self>) {
        self.workers.remove(&id);
        self.captured.retain(|(a, _)| *a != id);
        self.sync.forget(id);
        self.accounts = self.db.accounts().unwrap_or_default();
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// Queue `job` on the account's workers (dropped when it has none).
    pub fn send(&self, account: i64, job: Job) {
        if let Some(w) = self.workers.get(&account) {
            w.send(job);
        }
    }

    /// Hand outbox row `row` to its account's send worker. Handing one over
    /// twice is harmless: only one send can take it.
    pub fn dispatch(&self, row: i64, account: i64) {
        self.send(account, Job::Send { row });
    }

    /// Hand every queued outbox row that is due to its send worker.
    pub fn drain_outbox(&mut self, cx: &mut Context<Self>) {
        match self.db.outbox_due(chrono::Utc::now().timestamp()) {
            Ok(rows) => {
                for (row, account) in rows {
                    self.dispatch(row, account);
                }
            }
            Err(e) => self.say(Said::Detail(format!("{e:#}")), true, cx),
        }
    }

    /// Sync every account; `full` ignores folder STATUS (start, Refresh).
    pub fn sync_all(&mut self, full: bool) {
        if full {
            self.last_full = Some(Instant::now());
        }
        for w in self.workers.values() {
            w.send(Job::SyncAll { full });
        }
    }

    /// The Refresh button: also retries a rejected sign-in, and shows any
    /// problem again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.sync.refreshing();
        for w in self.workers.values() {
            w.credentials_changed();
        }
        self.sync_all(true);
        self.drain_outbox(cx);
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// Start the network work once the window is on screen: a full sync of
    /// every account, the Outbox, and the periodic check.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        self.sync_all(true);
        self.drain_outbox(cx);
        if self.mode == Mode::Off {
            return;
        }
        // Asleep, the connections died: the workers drop them at once, and a
        // moment later (the network is back) every account syncs and the
        // Outbox is handed over.
        self._wake = Some(crate::wake::watch(
            std::time::SystemTime::now,
            |hub: &mut Hub, wake, cx| hub.woke(wake, cx),
            cx,
        ));
        self._tick = Some(cx.spawn(async move |this, cx| {
            loop {
                // The interval is read each round: Settings can change it.
                let Ok(interval) = this.update(cx, |hub, _| hub.interval) else {
                    break;
                };
                let wait = interval.max(Duration::from_secs(30));
                cx.background_executor().timer(wait).await; // timer-exempt: the new-mail check interval the user chose (setting check_interval_secs)
                let alive = this.update(cx, |hub, cx| {
                    let full = hub.last_full.is_none_or(|t| t.elapsed() >= FULL_SYNC_EVERY);
                    hub.sync_all(full);
                    hub.drain_outbox(cx);
                    cx.emit(HubEvent::Tick);
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// Whether the watch for a wake from sleep runs (it starts with the
    /// first sync).
    #[cfg(test)]
    pub(super) fn watches_for_a_wake(&self) -> bool {
        self._wake.is_some()
    }

    /// The computer woke from sleep ([`crate::wake::watch`]).
    pub(super) fn woke(&mut self, wake: crate::wake::Wake, cx: &mut Context<Self>) {
        match wake {
            crate::wake::Wake::Woke => {
                for w in self.workers.values() {
                    w.resumed();
                }
            }
            crate::wake::Wake::Settled => {
                let full = self
                    .last_full
                    .is_none_or(|t| t.elapsed() >= FULL_SYNC_EVERY);
                self.sync_all(full);
                self.drain_outbox(cx);
            }
        }
    }

    /// The new-mail check interval changed (Settings).
    pub fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    // ------------------------------------------------------------ events

    /// An event from the workers' threads, on the UI thread.
    pub fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let before = self.sync.line().cloned();
        let reactions = {
            let Self {
                sync,
                accounts,
                folders,
                workers,
                ..
            } = self;
            let email = |id: i64| {
                accounts
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| a.email.clone())
                    .unwrap_or_default()
            };
            let attention = |id: i64| workers.get(&id).and_then(Workers::attention);
            let folder_name = |id: i64| {
                folders
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| f.name.clone())
                    .unwrap_or_default()
            };
            let env = Env {
                email: &email,
                attention: &attention,
                folder_name: &folder_name,
                now: chrono::Utc::now().timestamp(),
            };
            sync.on_event(event, &env)
        };
        for reaction in reactions {
            match &reaction {
                Reaction::Warm(id) => self.send(*id, Job::Warm),
                Reaction::SyncFolder { id, full } => {
                    if let Some(account) = self
                        .folders
                        .iter()
                        .find(|f| f.id == *id)
                        .map(|f| f.account_id)
                    {
                        self.send(
                            account,
                            Job::SyncFolder {
                                id: *id,
                                full: *full,
                            },
                        );
                    }
                }
                Reaction::DraftCopyLeft(copy) => self.remove_server_copy(Some(*copy)),
                Reaction::FoldersChanged => {
                    self.folders = self.db.folders().unwrap_or_default();
                }
                _ => {}
            }
            cx.emit(HubEvent::React(reaction));
        }
        self.line_changed(before, cx);
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// A line that changed starts its own timer.
    fn line_changed(&mut self, before: Option<Line>, cx: &mut Context<Self>) {
        let now = self.sync.line().cloned();
        if now == before {
            return;
        }
        self.line_seq += 1;
        let seq = self.line_seq;
        if let Some(secs) = now.as_ref().and_then(|l| l.for_secs) {
            let shown_for = Duration::from_secs(secs);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(shown_for).await; // timer-exempt: the line goes after 5 s (news) or 12 s (errors), WCAG 2.2.1
                let _ = this.update(cx, |hub, cx| {
                    if hub.line_seq == seq {
                        hub.clear_line(cx);
                    }
                });
            })
            .detach();
        }
    }

    // ------------------------------------------------------- the status line

    /// Show `said` on the status line (news for 5 s, an error for 12 s).
    pub fn say(&mut self, said: Said, error: bool, cx: &mut Context<Self>) {
        let before = self.sync.line().cloned();
        self.sync.say(said, error);
        self.standing_email();
        self.line_changed(before, cx);
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// Show `said` with a button.
    pub fn say_with(
        &mut self,
        said: Said,
        error: bool,
        action: StatusAction,
        cx: &mut Context<Self>,
    ) {
        self.say(said, error, cx);
        self.sync.set_action(action);
        cx.notify();
    }

    /// Show `said` with a button until it is closed, acted on or replaced
    /// (the What's new bar).
    pub fn say_standing(&mut self, said: Said, action: StatusAction, cx: &mut Context<Self>) {
        let before = self.sync.line().cloned();
        self.sync.say(said, false);
        self.sync.set_action(action);
        self.sync.keep_line();
        self.line_changed(before, cx);
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// The line goes, or the account out of reach shows again.
    pub fn clear_line(&mut self, cx: &mut Context<Self>) {
        self.say(Said::Text(String::new()), false, cx);
    }

    /// The line was closed (its button, or Esc).
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        let before = self.sync.line().cloned();
        let whats_new = self
            .sync
            .line()
            .is_some_and(|l| l.action == StatusAction::WhatsNew);
        self.sync.dismiss();
        self.standing_email();
        self.line_changed(before, cx);
        if whats_new {
            cx.emit(HubEvent::WhatsNew);
        }
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// The line's button was pressed.
    pub fn press_action(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.sync.line().map(|l| l.action) else {
            return;
        };
        match action {
            StatusAction::None => {}
            StatusAction::EditAccount(id) => {
                self.clear_line(cx);
                cx.emit(HubEvent::EditAccount {
                    id,
                    certificate: false,
                });
            }
            StatusAction::ReviewCertificate(id) => {
                self.clear_line(cx);
                cx.emit(HubEvent::EditAccount {
                    id,
                    certificate: true,
                });
            }
            StatusAction::RetryAccount(id) => {
                self.clear_line(cx);
                self.retry_account(id, cx);
            }
            StatusAction::TryNow(id) => {
                self.sync.trying_now(id);
                self.line_seq += 1;
                self.send(id, Job::SyncAll { full: true });
                cx.emit(HubEvent::Changed);
                cx.notify();
            }
            StatusAction::WhatsNew => {
                self.clear_line(cx);
                cx.emit(HubEvent::WhatsNew);
            }
            StatusAction::UndoSend(row) => cx.emit(HubEvent::UndoSend(row)),
        }
    }

    /// Retry on the status line: the account's workers make one new attempt.
    pub fn retry_account(&mut self, id: i64, cx: &mut Context<Self>) {
        self.sync.retrying(id);
        if let Some(w) = self.workers.get(&id) {
            w.credentials_changed();
            w.send(Job::SyncAll { full: true });
        }
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    // -------------------------------------------- what the views' events need

    /// Remove a draft's copy on the server.
    pub fn remove_server_copy(&self, copy: Option<komorebi::db::ServerCopy>) {
        if let Some(copy) = copy {
            self.send(copy.account_id, Job::DeleteDraft(copy));
        }
    }

    /// The part of a composer's event that belongs to the shell: the status
    /// line, the server copies, the send worker and the Undo offer. True if
    /// it was one of these.
    pub fn on_composer(&mut self, event: &ComposerEvent, cx: &mut Context<Self>) -> bool {
        match event {
            ComposerEvent::Status(status) => {
                self.say(Said::Composer(status.clone()), status.is_error(), cx);
            }
            ComposerEvent::SaveToServer { row, account } => {
                self.send(*account, Job::SaveDraft { row: *row });
            }
            ComposerEvent::RemoveServerCopy(copy) => self.remove_server_copy(Some(*copy)),
            ComposerEvent::Sending { row, secs, subject } => {
                self.offer_undo_send(*row, *secs, subject.clone(), cx)
            }
            ComposerEvent::Queued { row, account } => {
                self.end_undo_offer(*row, cx);
                self.dispatch(*row, *account);
            }
            ComposerEvent::Unsent { row } => self.end_undo_offer(*row, cx),
            _ => return false,
        }
        true
    }

    /// The part of a reader's event that belongs to the shell. True if it
    /// was one of these.
    pub fn on_reader(&mut self, event: &ReaderEvent, cx: &mut Context<Self>) -> bool {
        match event {
            ReaderEvent::FetchBody(id) | ReaderEvent::RetryBody(id) => {
                if let Ok(Some(m)) = self.db.message(*id) {
                    self.send(m.account_id, Job::FetchBody(*id));
                }
                cx.notify();
            }
            ReaderEvent::Status { text, error } => {
                self.say(Said::Text(text.clone()), *error, cx);
            }
            ReaderEvent::ReplyQueued { row, account } => self.dispatch(*row, *account),
            _ => return false,
        }
        true
    }

    // ------------------------------------------------- the Undo send offer

    fn offer_undo_send(&mut self, row: i64, secs: u64, subject: String, cx: &mut Context<Self>) {
        self.sending = Some(Sending {
            row,
            subject,
            until: cx.background_executor().now() + Duration::from_secs(secs),
        });
        self.show_sending(row, secs, cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await; // timer-exempt: the Sending in N s count-down of the undo window
                let go = this.update(cx, |hub, cx| hub.tick_sending(row, cx));
                if !matches!(go, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn show_sending(&mut self, row: i64, secs: u64, cx: &mut Context<Self>) {
        let before = self.sync.line().cloned();
        let subject = self
            .sending
            .as_ref()
            .filter(|s| s.row == row)
            .map(|s| s.subject.clone())
            .unwrap_or_default();
        self.sync.say(Said::Sending { secs, subject }, false);
        self.sync.set_action(StatusAction::UndoSend(row));
        // The offer stays until it ends.
        self.sync.keep_line();
        self.line_changed(before, cx);
        cx.emit(HubEvent::Changed);
        cx.notify();
    }

    /// One second of the count-down; false once the offer is over.
    fn tick_sending(&mut self, row: i64, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.sending.as_ref().filter(|s| s.row == row) else {
            return false;
        };
        let now = cx.background_executor().now();
        let left = secs_left(s.until.saturating_duration_since(now));
        // At 0 s the line says "Sending …" until the build is done (the
        // composer's Queued ends the offer).
        self.show_sending(row, left, cx);
        left > 0
    }

    fn end_undo_offer(&mut self, row: i64, cx: &mut Context<Self>) {
        if self.sending.as_ref().is_some_and(|s| s.row == row) {
            self.sending = None;
            if self
                .sync
                .line()
                .is_some_and(|l| l.action == StatusAction::UndoSend(row))
            {
                self.clear_line(cx);
            }
        }
    }

    /// The send offer showing, if any (its outbox row).
    pub fn sending(&self) -> Option<i64> {
        self.sending.as_ref().map(|s| s.row)
    }
}

impl Hub {
    /// The standing "can't reach" line names its account.
    fn standing_email(&mut self) {
        let Self { sync, accounts, .. } = self;
        sync.with_standing_email(&|id| {
            accounts
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.email.clone())
                .unwrap_or_default()
        });
    }
}

/// The whole seconds left of the undo window, rounded up as the Slint
/// `count_down`'s `div_ceil`: a timer that fires a little late leaves
/// 8.99 s, which says 9, and the last second says 1 until the window is
/// really over.
fn secs_left(left: Duration) -> u64 {
    u64::try_from(left.as_millis().div_ceil(1000)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_down_rounds_up() {
        assert_eq!(secs_left(Duration::from_millis(8_990)), 9);
        assert_eq!(secs_left(Duration::from_millis(9_000)), 9);
        assert_eq!(secs_left(Duration::from_millis(10)), 1);
        assert_eq!(secs_left(Duration::ZERO), 0);
    }
}
