//! Mail workers.
//!
//! Each account gets two worker threads, each with its own IMAP connection
//! and SQLite connection:
//! - **sync**: folder list, header sync, flag sync and body prefetch. These
//!   jobs can run for a long time. Prefetch runs one batch at a time, only
//!   while no other job is queued.
//! - **interactive**: opening messages, flag changes and moves. These run
//!   while a long sync is in progress, so the UI never waits on a sync.
//!
//! Each send runs on its own short-lived thread (with its own connections,
//! for the Sent copy), so a large upload never holds up opens and flags. A
//! send takes one queued outbox row (`db::Outbox`) and settles it: delivered
//! rows go, and only a send that surely did not reach the server is queued
//! again, with backoff ([`settle`]).
//!
//! Drafts are copied to the server's Drafts folder by the sync worker
//! ([`Job::SaveDraft`], and after each sync pass for drafts changed while
//! offline): it appends the new copy first, then removes the one it
//! replaces like a delete does (UID EXPUNGE only with UIDPLUS).
//!
//! A third thread, **idle** ([`idle`]), keeps a read-only connection in
//! IMAP IDLE on the inbox and only reports changes
//! ([`Event::RemoteChanged`]); the sync worker fetches them.
//!
//! Workers report back through [`Event`]s, which the UI applies on its own
//! thread. Errors go out as [`Problem`]s ([`errors`]). An action that
//! failed only because the server couldn't be reached waits on the
//! interactive worker and runs once more on the next connection that works
//! ([`Worker::retry_parked`]); after a wake from sleep ([`Workers::resumed`])
//! every connection is replaced.

pub mod compose_html;
pub mod discover;
pub mod errors;
#[cfg(test)]
mod fake;
#[cfg(test)]
mod fake_imapd;
#[cfg(test)]
pub(crate) mod fake_tls;
pub mod html;
pub mod ical;
mod idle;
pub mod imap;
pub mod mime;
pub mod pin;
#[cfg(test)]
mod profiles;
pub mod render;
pub mod rich;
pub mod smtp;
pub mod source;
#[cfg(test)]
mod tolerance;

use crate::db::{self, Account, Db, Folder, Fp};
use crate::diag;
use crate::providers::Server;
use crate::role::Role;
use crate::secrets;
use anyhow::{Context, Result, anyhow};
pub use errors::{Attention, Class, Problem, Say};
use errors::{is_connection_error, is_parse_error};
use idle::{IdleHandle, Idler, spawn_idle};
use imap::{FolderStatus, Imap, Statuses};
use lettre::address::Envelope;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// At most this many headers are fetched per round trip during a first
/// sync (the header batch ramp's ceiling).
pub const HEADER_BATCH: usize = 500;
/// A folder's first header batch, and the ramp's floor: small, so the
/// newest rows show up after one short round trip even on a slow link.
pub const FIRST_HEADER_BATCH: usize = 50;
/// The header batch ramp aims for round trips about this long.
const HEADER_BATCH_TARGET: Duration = Duration::from_millis(1500);
/// Bodies larger than this are fetched only when opened.
pub const PREFETCH_MAX_SIZE: i64 = 512 * 1024;
/// At most this many bodies are prefetched per round trip.
const PREFETCH_BATCH: usize = 25;
/// Every this many sync passes (about hourly at the 1-minute timer), every
/// folder is synced in full whatever its STATUS says.
const FULL_PASS_EVERY: u32 = 60;
/// A folder sync downloading at least this many headers reports its
/// progress ([`Event::Progress`]): a first sync, or a large catch-up.
#[cfg(not(test))]
const PROGRESS_FROM: usize = 1000;
#[cfg(test)]
const PROGRESS_FROM: usize = 2;
/// While an action waits for the server ([`Parked`]), the interactive
/// worker tries to connect this often.
const PARK_PROBE: Duration = Duration::from_secs(30);
/// A waiting action that still can't reach the server after this long is
/// given up: the next sync shows the server's state again.
const PARK_MAX: Duration = Duration::from_secs(5 * 60);

/// Which inbox bodies are downloaded ahead of opening, so they open
/// instantly. Sizes are the server's `RFC822.SIZE`, summed before fetching.
/// Opening a message always fetches its body, whatever these say.
#[derive(Debug, Clone, Copy)]
pub struct PrefetchPolicy {
    /// Only the newest this many messages are prefetched.
    pub window: usize,
    /// Bodies larger than this are fetched only when opened.
    pub max_msg: i64,
    /// Bytes fetched per sync pass at most.
    pub pass_budget: i64,
    /// Bytes per round trip at most (and at most 25 messages).
    pub batch_bytes: i64,
}

/// The policy in use. Tunables, to be set from the probe's size histogram.
pub const PREFETCH: PrefetchPolicy = PrefetchPolicy {
    window: 300,
    max_msg: PREFETCH_MAX_SIZE,
    pass_budget: 8 << 20,
    batch_bytes: 2 << 20,
};

#[derive(Debug, Clone)]
pub enum Event {
    Syncing {
        account_id: i64,
        active: bool,
        error: Option<Problem>,
    },
    /// A folder's headers are downloading: `done` of `total` are cached
    /// (sent per batch, from [`PROGRESS_FROM`] new headers).
    Progress {
        account_id: i64,
        folder_id: i64,
        done: usize,
        total: usize,
    },
    /// Folder rows changed (`None`: the folder list itself changed).
    FolderChanged {
        folder_id: Option<i64>,
    },
    BodyReady {
        message_id: i64,
    },
    BodyFailed {
        message_id: i64,
        error: Problem,
    },
    /// A `Job::Send` of outbox row `row` ended (its row is settled).
    Sent {
        row: i64,
        /// The message it replies to (now answered, if delivered).
        reply_to: Option<i64>,
        outcome: Outcome,
    },
    /// A server action failed; the next sync restores the true state.
    ActionFailed {
        account_id: i64,
        error: Problem,
    },
    /// A folder got new mail from our own action (move/append), or its
    /// cache is out of date, and should be synced.
    NeedsSync {
        folder_id: i64,
    },
    /// An action worked, with a caveat the user should see.
    Notice {
        account_id: i64,
        message: Notice,
    },
    /// The server reported a change in this folder (IMAP IDLE); a
    /// STATUS-gated sync picks it up.
    RemoteChanged {
        folder_id: i64,
    },
    /// A draft's new server copy replaced one saved from another account
    /// (its From changed): that account's worker should remove it.
    DraftCopyLeft(db::ServerCopy),
}

/// How a send ended. Only [`Outcome::Retry`] is ever sent again without
/// the user; an unconfirmed send never is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    /// The server took the message, but saving the Sent copy failed.
    DeliveredNotSaved(String),
    /// Not sent, and safe to try again later: the server could not be
    /// reached or refused for now (4xx). The row is queued with backoff.
    Retry(Reason),
    /// Not sent: refused, or out of retries.
    Failed(Reason),
    /// The connection broke after the session was up: it may or may not
    /// have been sent.
    Unconfirmed(Reason),
}

/// Why a send did not go through. `text` is what the outbox row keeps (the
/// failure's own words, in English); `say` is the sentence the status line
/// renders in the display language, when the failure has one of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub text: String,
    pub say: Option<Say>,
}

impl Reason {
    /// A failure with a sentence of its own (kept in the row as its
    /// English record).
    pub fn said(say: Say) -> Self {
        Reason {
            text: say.record().unwrap_or_default(),
            say: Some(say),
        }
    }
}

impl From<String> for Reason {
    fn from(text: String) -> Self {
        Reason { text, say: None }
    }
}

impl From<&str> for Reason {
    fn from(text: &str) -> Self {
        Reason::from(text.to_string())
    }
}

impl std::ops::Deref for Reason {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl Outcome {
    /// The message is out, so the draft can go.
    pub fn delivered(&self) -> bool {
        matches!(self, Self::Delivered | Self::DeliveredNotSaved(_))
    }

    /// What went wrong (empty for a clean delivery).
    pub fn reason(&self) -> &str {
        match self {
            Self::Delivered => "",
            Self::DeliveredNotSaved(e) => e,
            Self::Retry(e) | Self::Failed(e) | Self::Unconfirmed(e) => &e.text,
        }
    }
}

/// A caveat of an action that worked, for the status line (the app words
/// it in the display language).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A delete or COPY-move left the original flagged on the server.
    LeftFlagged,
    /// A move whose messages were already gone from the server.
    MoveGone,
    /// Said once per session when drafts can't be copied to the server.
    NoDraftsFolder,
    /// Said once per account and session when sent mail has no folder to
    /// be copied to (nothing is created for it).
    NoSentFolder,
    /// Said once per session when a draft saved to the server can't be
    /// found there (its next save would leave it behind).
    NoDraftUid,
    /// Saving a draft to the server failed for a reason of the server's
    /// (its own words): drafts stay on this PC.
    DraftsNotSaved(String),
}

/// A send that was running when the app stopped (`Db::recover_outbox`): the
/// outbox record, in English as the one above.
pub const INTERRUPTED: &str = "Komorebi closed while sending, so it may or may not have been sent; check Sent before you retry.";

/// Drafts whose attachments are larger than this stay on this PC: every
/// server copy uploads them again.
const MAX_DRAFT_COPY_ATTACHMENTS: i64 = 10 * 1024 * 1024;

/// A send is tried at most this many times by itself.
pub const MAX_SEND_ATTEMPTS: i64 = 6;
/// The wait before the first automatic retry; it doubles, up to [`RETRY_MAX`].
const RETRY_FIRST: i64 = 30;
const RETRY_MAX: i64 = 15 * 60;

pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;

/// Messages with a queued or running interactive action, keyed by
/// (folder id, UID), with a count per message. The sync worker leaves these
/// rows alone, so a sync running at the same time cannot bring back a
/// message the user just moved, or undo a flag change.
#[derive(Clone, Default)]
struct Pending(Arc<Mutex<HashMap<(i64, u32), usize>>>);

impl Pending {
    fn add(&self, key: (i64, u32)) {
        *self.0.lock().expect("pending lock").entry(key).or_default() += 1;
    }
    fn done(&self, key: (i64, u32)) {
        let mut map = self.0.lock().expect("pending lock");
        if let Some(n) = map.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                map.remove(&key);
            }
        }
    }
    fn uids(&self, folder_id: i64) -> HashSet<u32> {
        self.0
            .lock()
            .expect("pending lock")
            .keys()
            .filter(|(f, _)| *f == folder_id)
            .map(|(_, uid)| *uid)
            .collect()
    }
}

pub enum Job {
    /// Sync every folder. Unless `full`, folders whose STATUS matches their
    /// last complete sync are skipped.
    SyncAll {
        full: bool,
    },
    /// Sync one folder. Unless `full`, its STATUS is read first, and it is
    /// skipped or takes the delta as in a pass.
    SyncFolder {
        id: i64,
        full: bool,
    },
    FetchBody(i64),
    /// Set or clear a flag on messages of one folder (one STORE per UID
    /// set, however many are selected).
    SetFlags {
        folder_id: i64,
        uids: Vec<u32>,
        flag: &'static str,
        on: bool,
    },
    /// Mark every message of a folder read, up to UID `through` (the
    /// highest cached): one STORE. `uids` are those that were unread in the
    /// cache, kept [`Pending`] so a sync meanwhile can't show them unread.
    MarkAllRead {
        folder_id: i64,
        through: u32,
        uids: Vec<u32>,
    },
    Move {
        folder_id: i64,
        uids: Vec<u32>,
        dest: i64,
    },
    Delete {
        folder_id: i64,
        uids: Vec<u32>,
    },
    /// Send queued outbox row `row`, if it is still queued. Runs on its
    /// own thread, so a long upload never holds up the interactive worker.
    /// Reported as [`Event::Sent`].
    Send {
        row: i64,
    },
    /// Copy saved draft `row` to the Drafts folder if it changed since its
    /// last copy there, then remove that copy (sync worker).
    SaveDraft {
        row: i64,
    },
    /// Remove a draft's copy from the server: the draft was sent,
    /// discarded or emptied (sync worker).
    DeleteDraft(db::ServerCopy),
    /// Sign in ahead of the first interactive action.
    Warm,
}

pub struct Workers {
    account: Account,
    db_path: PathBuf,
    notify: Notify,
    sync: Sender<Job>,
    interactive: Sender<Job>,
    pending: Pending,
    auth_failed: errors::Latch,
    /// Raised by [`Workers::resumed`]: every worker replaces its connection
    /// before its next command.
    resume: Arc<AtomicU64>,
    /// Stops the IDLE thread when the workers are dropped.
    idle: IdleHandle,
    /// The missing Sent folder was said once.
    sent_noted: Arc<std::sync::atomic::AtomicBool>,
    /// Every job goes to the channel and nothing runs (`Workers::capture`).
    capture: bool,
}

impl Workers {
    /// Workers that run nothing and connect nowhere: every job sent,
    /// sends included, arrives on the returned receiver as it was queued.
    /// For the app's own tests, which check what the UI queues (the
    /// workers' tests run those jobs against the fake servers).
    #[doc(hidden)]
    pub fn capture(account: Account, db_path: PathBuf, notify: Notify) -> (Self, Receiver<Job>) {
        let (tx, rx) = channel();
        // No ready signal ever comes: the IDLE thread ends at once.
        let (_, ready_rx) = channel();
        let auth_failed = errors::Latch::default();
        let idle = spawn_idle(
            Idler::new(
                account.clone(),
                Box::new(|_| Err(anyhow!("captured workers connect nowhere"))),
                notify.clone(),
                auth_failed.clone(),
            ),
            ready_rx,
        );
        let workers = Self {
            account,
            db_path,
            notify,
            sync: tx.clone(),
            interactive: tx,
            pending: Pending::default(),
            auth_failed,
            resume: Arc::new(AtomicU64::new(0)),
            idle,
            sent_noted: Arc::default(),
            capture: true,
        };
        (workers, rx)
    }

    pub fn spawn(account: Account, db_path: PathBuf, notify: Notify) -> Self {
        let (sync, sync_rx) = channel();
        let (interactive, inter_rx) = channel();
        let pending = Pending::default();
        let auth_failed = errors::Latch::default();
        let resume = Arc::new(AtomicU64::new(0));
        let (ready, ready_rx) = channel();
        let idle = spawn_idle(
            Idler::new(
                account.clone(),
                Box::new(connect_account),
                notify.clone(),
                auth_failed.clone(),
            ),
            ready_rx,
        );
        let mut ready = Some(ready);
        let workers = Self {
            account,
            db_path,
            notify,
            sync,
            interactive,
            pending,
            auth_failed,
            resume,
            idle,
            sent_noted: Arc::default(),
            capture: false,
        };
        for (rx, name) in [(sync_rx, "sync"), (inter_rx, "interactive")] {
            let mut w = workers.worker();
            w.unselect_when_idle = name == "sync";
            if name == "sync" {
                w.backfill = Backfill::AfterFirstPass;
            }
            w.ready = ready.take();
            std::thread::Builder::new()
                .name(format!("{name}-{}", workers.account.id))
                .spawn(move || w.run(rx))
                .expect("spawning mail worker");
        }
        workers
    }

    /// A worker for this account with no connection yet.
    fn worker(&self) -> Worker {
        Worker {
            account: self.account.clone(),
            db_path: self.db_path.clone(),
            notify: self.notify.clone(),
            imap: None,
            connect: Box::new(connect_account),
            smtp: Box::new(smtp_account),
            auth_failed: self.auth_failed.clone(),
            latch_reported: false,
            pending: self.pending.clone(),
            prefetch: None,
            backfill: Backfill::Done,
            passes_since_full: None,
            ready: None,
            unselect_when_idle: false,
            drafts_noted: false,
            sent_noted: self.sent_noted.clone(),
            resume: self.resume.clone(),
            resume_seen: self.resume.load(Ordering::Relaxed),
            parked: Vec::new(),
        }
    }

    /// The computer woke from sleep (or the clock jumped): the pooled
    /// connections are likely dead. Each worker drops its connection
    /// before its next command (without the NOOP probe), and the IDLE
    /// connection is cut and reopened at once, its backoff reset.
    pub fn resumed(&self) {
        self.resume.fetch_add(1, Ordering::Relaxed);
        self.idle.kick();
    }

    /// Let the workers sign in again after a rejected sign-in (new
    /// password, or the user asked to retry).
    pub fn credentials_changed(&self) {
        self.auth_failed.clear();
    }

    /// An action on messages is queued or running (its UIDs are pending).
    pub fn busy(&self) -> bool {
        !self.pending.0.lock().expect("pending lock").is_empty()
    }

    /// The server rejected the sign-in (or no password is saved): no
    /// LOGIN is tried until [`Workers::credentials_changed`].
    pub fn sign_in_rejected(&self) -> bool {
        self.auth_failed.get()
    }

    /// What the workers wait for the user to fix, if anything: the sign-in
    /// or the secure connection. One attempt was made; none follows until
    /// [`Workers::credentials_changed`].
    pub fn attention(&self) -> Option<errors::Attention> {
        self.auth_failed.attention()
    }

    pub fn send(&self, job: Job) {
        if let Job::Send { row } = job
            && !self.capture
        {
            return self.spawn_send(row);
        }
        for key in job.message_keys() {
            self.pending.add(key);
        }
        // Draft copies go one at a time through the sync worker, so a save
        // never overtakes the one before it.
        let tx = match job {
            Job::SyncAll { .. }
            | Job::SyncFolder { .. }
            | Job::SaveDraft { .. }
            | Job::DeleteDraft(_) => &self.sync,
            _ => &self.interactive,
        };
        // A closed channel means the account was removed; nothing to do.
        tx.send(job).ok();
    }

    /// Run a send on a short-lived worker with its own database and IMAP
    /// connections (for the Sent copy), then log out. If the cache can't be
    /// opened, the row stays queued for the next pass.
    fn spawn_send(&self, row: i64) {
        let mut w = self.worker();
        std::thread::Builder::new()
            .name(format!("send-{}", self.account.id))
            .spawn(move || {
                match Db::open_worker(&w.db_path) {
                    Ok(mut db) => w.run_job(&mut db, Job::Send { row }, &mut || None),
                    Err(e) => w.fail(e),
                }
                if let Some(imap) = w.imap.take() {
                    imap.logout();
                }
            })
            .expect("spawning send worker");
    }
}

impl Job {
    /// The folder a message action works in (0 for other jobs).
    fn folder_id(&self) -> i64 {
        match self {
            Job::SetFlags { folder_id, .. }
            | Job::MarkAllRead { folder_id, .. }
            | Job::Move { folder_id, .. }
            | Job::Delete { folder_id, .. } => *folder_id,
            _ => 0,
        }
    }

    /// The messages this job acts on, as (folder id, UID): every one is
    /// [`Pending`] until the job is done.
    fn message_keys(&self) -> Vec<(i64, u32)> {
        match self {
            Job::SetFlags {
                folder_id, uids, ..
            }
            | Job::MarkAllRead {
                folder_id, uids, ..
            }
            | Job::Move {
                folder_id, uids, ..
            }
            | Job::Delete { folder_id, uids } => uids.iter().map(|&u| (*folder_id, u)).collect(),
            Job::DeleteDraft(db::ServerCopy { folder_id, uid, .. }) => vec![(*folder_id, *uid)],
            _ => Vec::new(),
        }
    }
}

struct Worker {
    account: Account,
    db_path: PathBuf,
    notify: Notify,
    imap: Option<Imap>,
    /// Opens a signed-in connection (tests point it at a fake server).
    connect: Connect,
    /// Hands a message to the account's SMTP server (tests: a fake one).
    smtp: SmtpSend,
    /// Set when the server rejected the sign-in (or no password is saved),
    /// shared by the account's workers: no LOGIN is tried until cleared.
    auth_failed: errors::Latch,
    /// The sync worker already reported the current rejection.
    latch_reported: bool,
    pending: Pending,
    /// Inbox bodies still to prefetch after the last sync pass.
    prefetch: Option<Prefetch>,
    /// Where the address index backfill is (sync worker only).
    backfill: Backfill,
    /// Sync passes since the last full one (`None`: none yet).
    passes_since_full: Option<u32>,
    /// Hands the inbox (id, server name) to the IDLE thread after its first
    /// successful sync (sync worker only).
    ready: Option<Sender<(i64, String)>>,
    /// Leave the selected folder before waiting for the next job, so the
    /// next steady-state pass is a single LIST-STATUS (sync worker only; the
    /// interactive worker keeps its folder open for fast message opens).
    unselect_when_idle: bool,
    /// Draft copies to the server stopped for this session, after one
    /// notice (no Drafts folder, the server refused, or no UID came back).
    drafts_noted: bool,
    /// The account's missing Sent folder was said (shared by its workers
    /// and send threads).
    sent_noted: Arc<std::sync::atomic::AtomicBool>,
    /// [`Workers::resumed`]'s count, and the value this worker last acted
    /// on.
    resume: Arc<AtomicU64>,
    resume_seen: u64,
    /// Actions that failed only because the server couldn't be reached
    /// (interactive worker).
    parked: Vec<Parked>,
}

/// A message action waiting for the server: it runs once more on the next
/// connection that works, and only if that fails too is the cache brought
/// back to the server's state. It keeps its messages [`Pending`] meanwhile,
/// so a sync can't undo what the user did. In memory only: after a restart
/// the sync shows the server's state.
struct Parked {
    job: Job,
    since: Instant,
}

/// The address index backfill (`Db::backfill_addresses`) of mail cached
/// before the index existed: it starts after the sync worker's first pass,
/// never on the launch path, and runs a chunk at a time while idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backfill {
    AfterFirstPass,
    Running,
    Done,
}

#[derive(Clone, Copy)]
struct Prefetch {
    folder_id: i64,
    budget_left: i64,
}

/// Returns the next `SyncFolder` (id, full) queued while a sync pass runs.
type NextFolder<'a> = &'a mut dyn FnMut() -> Option<(i64, bool)>;

type Connect = Box<dyn Fn(&Account) -> Result<Imap> + Send>;

fn connect_account(account: &Account) -> Result<Imap> {
    let started = Instant::now();
    let imap = secrets::get_password(account.id).and_then(|password| {
        Imap::connect(
            &account.imap,
            account.imap_pin.as_deref(),
            account.imap_login(),
            &password,
        )
    });
    let status = match &imap {
        Ok(_) => "OK",
        Err(e) => errors::classify(e).name(),
    };
    diag::record(account.id, "CONNECT", status, started.elapsed());
    imap.map(|mut imap| {
        imap.set_account(account.id);
        imap
    })
}

type SmtpSend = Box<dyn Fn(&Account, &Envelope, &[u8]) -> Result<()> + Send>;

fn smtp_account(account: &Account, envelope: &Envelope, raw: &[u8]) -> Result<()> {
    let password = secrets::get_password(account.id)?;
    smtp::send_raw(
        &account.smtp,
        account.smtp_pin.as_deref(),
        account.smtp_login(),
        &password,
        envelope,
        raw,
    )
}

impl Worker {
    fn run(&mut self, rx: Receiver<Job>) {
        let mut db = match Db::open_worker(&self.db_path) {
            Ok(db) => db,
            Err(e) => return self.fail(e),
        };
        self.serve(&mut db, &rx);
    }

    /// Run jobs until the channel closes, then log out. Queued jobs go
    /// first; the backfill and prefetch run a step at a time only while
    /// nothing is queued.
    fn serve(&mut self, db: &mut Db, rx: &Receiver<Job>) {
        let mut backlog = VecDeque::new();
        loop {
            let mut job = match backlog.pop_front() {
                Some(job) => job,
                None if self.prefetch.is_some() || self.backfill == Backfill::Running => {
                    match rx.try_recv() {
                        Ok(job) => job,
                        Err(TryRecvError::Empty) => {
                            self.idle_step(db);
                            continue;
                        }
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
                None => {
                    if self.unselect_when_idle
                        && let Some(imap) = self.imap.as_mut()
                        && imap.leave_folder().is_err()
                    {
                        // The stream may be out of step; reconnect next time.
                        self.imap = None;
                    }
                    if self.parked.is_empty() {
                        match rx.recv() {
                            Ok(job) => job,
                            Err(_) => break,
                        }
                    } else {
                        // Actions wait for the server: try to reach it now
                        // and then.
                        match rx.recv_timeout(PARK_PROBE) {
                            Ok(job) => job,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                self.retry_parked(db);
                                continue;
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                }
            };
            // Account removed: stop, but still log out below.
            if matches!(job, Job::SyncAll { .. } | Job::SyncFolder { .. })
                && !coalesce(&mut job, rx, &mut backlog)
            {
                break;
            }
            let keys = job.message_keys();
            self.run_job(db, job, &mut || next_sync_folder(rx, &mut backlog));
            for key in keys {
                self.pending.done(key);
            }
            // That job got through to the server: the waiting actions too.
            if self.imap.is_some() {
                self.retry_parked(db);
            }
        }
        // Actions still waiting are lost with the session: the cache may
        // differ from the server, so the next sync must look.
        for p in std::mem::take(&mut self.parked) {
            db.clear_fingerprint(p.job.folder_id()).ok();
        }
        if let Some(imap) = self.imap.take() {
            imap.logout();
        }
    }

    fn run_job(&mut self, db: &mut Db, job: Job, next_folder: NextFolder) {
        let id = self.account.id;
        match job {
            Job::SyncAll { full } => {
                // No network while the sign-in is rejected; say so once.
                if self.auth_failed.get() {
                    if !std::mem::replace(&mut self.latch_reported, true) {
                        (self.notify)(Event::Syncing {
                            account_id: id,
                            active: false,
                            error: Some(Problem::of(&self.auth_failed.error())),
                        });
                    }
                    return;
                }
                let full = full_pass(&mut self.passes_since_full, full);
                (self.notify)(Event::Syncing {
                    account_id: id,
                    active: true,
                    error: None,
                });
                let error = self.sync_all(db, next_folder, full).err();
                db.checkpoint();
                if self.backfill == Backfill::AfterFirstPass {
                    self.backfill = Backfill::Running;
                }
                self.latch_reported = self.auth_failed.get();
                let synced = error.is_none();
                (self.notify)(Event::Syncing {
                    account_id: id,
                    active: false,
                    error: error.as_ref().map(Problem::of),
                });
                // Connected again: copy the drafts changed meanwhile.
                if synced {
                    match db.drafts_to_upload(id) {
                        Ok(rows) => rows
                            .into_iter()
                            .for_each(|(row, _)| self.save_draft(db, row)),
                        Err(e) => self.fail(e),
                    }
                }
            }
            Job::SyncFolder { id, full } => self.sync_one(db, id, full),
            Job::FetchBody(message_id) => {
                if let Err(e) = self.fetch_body(db, message_id) {
                    (self.notify)(Event::BodyFailed {
                        message_id,
                        error: Problem::of(&e),
                    });
                }
            }
            job @ (Job::SetFlags { .. }
            | Job::MarkAllRead { .. }
            | Job::Move { .. }
            | Job::Delete { .. }) => {
                // Earlier actions on this folder still waiting go first,
                // in order (a flag set then cleared must end cleared).
                if !self.parked.is_empty() {
                    self.retry_parked(db);
                }
                if self
                    .parked
                    .iter()
                    .any(|p| p.job.folder_id() == job.folder_id())
                {
                    self.park(job);
                } else {
                    self.act(db, job, false);
                }
            }
            Job::Send { row } => self.send_row(db, row),
            Job::SaveDraft { row } => self.save_draft(db, row),
            Job::DeleteDraft(copy) => self.remove_draft_copy(db, copy),
            // A failure shows up on the next action that needs the server.
            Job::Warm => {
                self.with_imap(|_, _| Ok(())).ok();
            }
        }
    }

    fn fail(&self, e: anyhow::Error) {
        (self.notify)(Event::ActionFailed {
            account_id: self.account.id,
            error: Problem::of(&e),
        });
    }

    /// Run a message action (flags, mark all read, move, delete). If it
    /// fails only because the server can't be reached, it waits
    /// ([`Parked`]) for one more try on the next connection that works
    /// (`retry`: this is that try). Messages already gone from the server
    /// count as done. Any other failure is reported and the cache brought
    /// back to the server's state ([`Worker::undo`]).
    fn act(&mut self, db: &mut Db, job: Job, retry: bool) {
        let result = self.try_act(db, &job);
        let e = match result {
            Ok(expunged) => {
                if let Some(expunged) = expunged {
                    self.notice_left_flagged(expunged);
                }
                if let Job::Move { dest, .. } = job {
                    (self.notify)(Event::NeedsSync { folder_id: dest });
                }
                return;
            }
            Err(e) => e,
        };
        let folder_id = job.folder_id();
        if errors::message_gone(&e) {
            // Nothing left to do on the server; a sync drops the rows.
            if matches!(job, Job::Move { .. }) {
                (self.notify)(Event::Notice {
                    account_id: self.account.id,
                    message: Notice::MoveGone,
                });
            }
            db.clear_fingerprint(folder_id).ok();
            (self.notify)(Event::NeedsSync { folder_id });
            return;
        }
        // A retried MOVE is harmless (moved UIDs are gone), and STORE and
        // UID EXPUNGE repeat safely. A COPY move (no MOVE) cut off after
        // the COPY may leave a second copy in the destination.
        if !retry
            && matches!(
                errors::classify(&e),
                Class::Offline | Class::TooManyConnections
            )
        {
            self.park(job);
            return;
        }
        let flags = matches!(job, Job::SetFlags { .. } | Job::MarkAllRead { .. });
        if flags && uidvalidity_changed(&e) {
            (self.notify)(Event::NeedsSync { folder_id });
        }
        self.fail(e);
        self.undo(db, &job);
    }

    /// Hold an action for [`Worker::retry_parked`]; its messages stay busy
    /// so a sync can't undo what the user did on screen.
    fn park(&mut self, job: Job) {
        for key in job.message_keys() {
            self.pending.add(key);
        }
        self.parked.push(Parked {
            job,
            since: Instant::now(),
        });
    }

    /// The server calls of [`Worker::act`]: what a move or delete left
    /// behind (none for flags).
    fn try_act(&mut self, db: &Db, job: &Job) -> Result<Option<imap::Expunged>> {
        let (folder, v) = folder_ref(db, job.folder_id())?;
        let at = (folder.as_str(), v);
        match job {
            Job::SetFlags { uids, flag, on, .. } => self
                .with_imap(|imap, _| imap.set_flags(at, uids, flag, *on))
                .map(|()| None),
            Job::MarkAllRead { through, .. } => self
                .with_imap(|imap, _| imap.mark_read_through(at, *through))
                .map(|()| None),
            Job::Move { uids, dest, .. } => {
                // The messages are still here; only where they go is gone.
                let (dest_name, _) = folder_ref(db, *dest)
                    .map_err(|_| anyhow!("the destination folder no longer exists"))?;
                self.with_imap(|imap, _| imap.move_messages(at, uids, &dest_name))
                    .map(Some)
            }
            Job::Delete { uids, .. } => self
                .with_imap(|imap, _| imap.delete_messages(at, uids))
                .map(Some),
            _ => Ok(None),
        }
    }

    /// After a failed action: the cache may now differ from the server
    /// while the folder's STATUS stays the same, so its fingerprint goes;
    /// a move or delete also syncs the folder to bring the messages back
    /// if the server kept them.
    fn undo(&self, db: &Db, job: &Job) {
        let folder_id = job.folder_id();
        db.clear_fingerprint(folder_id).ok();
        if matches!(job, Job::Move { .. } | Job::Delete { .. }) {
            (self.notify)(Event::NeedsSync { folder_id });
        }
    }

    /// Give the waiting actions their one more try once the server can be
    /// reached. While it still can't, they wait, up to [`PARK_MAX`]; then
    /// they are reported and undone.
    fn retry_parked(&mut self, db: &mut Db) {
        if self.parked.is_empty() {
            return;
        }
        match self.with_imap(|_, _| Ok(())) {
            Ok(()) => {
                for p in std::mem::take(&mut self.parked) {
                    let keys = p.job.message_keys();
                    self.act(db, p.job, true);
                    for key in keys {
                        self.pending.done(key);
                    }
                }
            }
            Err(e) => {
                let (expired, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.parked)
                    .into_iter()
                    .partition(|p| p.since.elapsed() >= PARK_MAX);
                self.parked = waiting;
                if expired.is_empty() {
                    return;
                }
                self.fail(e);
                for p in expired {
                    self.undo(db, &p.job);
                    for key in p.job.message_keys() {
                        self.pending.done(key);
                    }
                }
            }
        }
    }

    fn notice_left_flagged(&self, expunged: imap::Expunged) {
        if expunged == imap::Expunged::LeftFlagged {
            (self.notify)(Event::Notice {
                account_id: self.account.id,
                message: Notice::LeftFlagged,
            });
        }
    }

    /// Run `f` on a live connection, reconnecting once if the old
    /// connection went stale (servers drop idle sessions). A response the
    /// parser can't read leaves the stream out of step, so the connection is
    /// dropped without a retry (the command may already have run). After a
    /// rejected sign-in, fails without touching the network.
    fn with_imap<T>(&mut self, mut f: impl FnMut(&mut Imap, &Notify) -> Result<T>) -> Result<T> {
        if self.auth_failed.get() {
            return Err(self.auth_failed.error());
        }
        // Woken from sleep: the connection is surely dead; don't wait on
        // a probe (or a LOGOUT) to find out.
        let resume = self.resume.load(Ordering::Relaxed);
        if resume != self.resume_seen {
            self.resume_seen = resume;
            self.imap = None;
        }
        // A connection unused for a while may have died silently (sleep,
        // network change). Only the NOOP probe is lost; nothing is replayed.
        if self
            .imap
            .as_mut()
            .is_some_and(|imap| imap.check_if_stale().is_err())
        {
            self.imap = None;
        }
        if let Some(imap) = self.imap.as_mut() {
            let result = f(imap, &self.notify);
            imap.touch();
            match result {
                Ok(v) => return Ok(v),
                Err(e) if !is_connection_error(&e) => return Err(e),
                Err(e) if is_parse_error(&e) => {
                    self.imap = None;
                    return Err(e);
                }
                Err(_) => self.imap = None,
            }
        }
        let imap = match (self.connect)(&self.account) {
            Ok(imap) => self.imap.insert(imap),
            // A rejected sign-in, or a certificate or STARTTLS failure: one
            // attempt, then the workers wait for the user (no retry loop).
            Err(e) => {
                self.auth_failed.set_from(&e);
                return Err(e);
            }
        };
        let result = f(imap, &self.notify);
        imap.touch();
        if result.as_ref().is_err_and(is_connection_error) {
            self.imap = None;
        }
        result
    }

    /// Sync one folder. A `full` sync takes the full path and records no
    /// fingerprint. Otherwise the folder's STATUS is read before the SELECT
    /// and the pass's skip gate and delta apply (without CONDSTORE there is
    /// no STATUS, so the full path runs).
    fn sync_one(&mut self, db: &mut Db, folder_id: i64, full: bool) {
        let result = db.folder(folder_id).and_then(|f| {
            let f = f.ok_or(errors::FolderGone)?;
            let pending = self.pending.clone();
            let mut ramp = FIRST_HEADER_BATCH;
            self.with_imap(|imap, notify| {
                let st = if full { None } else { imap.status(&f.name)? };
                if st.is_some() && skippable(db, &f, st.as_ref(), &pending, false)? {
                    return Ok(());
                }
                sync_folder(imap, db, &f, notify, &pending, st.as_ref(), full, &mut ramp).map(drop)
            })
        });
        // A folder the server dropped needs no sync (and no complaint).
        if let Err(e) = result
            && !e.chain().any(|c| c.is::<errors::FolderGone>())
        {
            self.fail(e);
        }
    }

    fn sync_all(&mut self, db: &mut Db, next_folder: NextFolder, full: bool) -> Result<()> {
        let id = self.account.id;
        let (mut folders, statuses) = self.with_imap(|imap, _| refresh_folders(imap, db, id))?;
        // The inbox first; the sort is stable, so the rest keep their order.
        folders.sort_by_key(|f| f.role != Role::Inbox);
        (self.notify)(Event::FolderChanged { folder_id: None });

        let mut errors = Vec::new();
        let pending = self.pending.clone();
        for f in &folders {
            let st = statuses.get(&f.name);
            let result = skippable(db, f, st, &pending, full).and_then(|skip| {
                if skip {
                    return Ok(());
                }
                let mut ramp = FIRST_HEADER_BATCH;
                self.with_imap(|imap, notify| {
                    sync_folder(imap, db, f, notify, &pending, st, full, &mut ramp)
                })
                .map(drop)
            });
            if result.is_ok()
                && f.role == Role::Inbox
                && let Some(ready) = self.ready.take()
            {
                ready.send((f.id, f.name.clone())).ok();
            }
            if let Err(e) = result {
                // The connection failed and couldn't be replaced, or the
                // sign-in was rejected: every later folder would fail the
                // same way. The 1-minute timer retries. A parse error only
                // dropped the connection, so the pass goes on.
                let ends = self.auth_failed.get() || (self.imap.is_none() && !is_parse_error(&e));
                errors.push((f.display.clone(), e));
                if ends {
                    return Err(pass_error(errors));
                }
            }
            // Between folders only: a folder the user opened goes next.
            while let Some((folder_id, full)) = next_folder() {
                self.sync_one(db, folder_id, full);
            }
        }
        if let Some(imap) = self.imap.as_mut() {
            imap.drain_unsolicited();
        }
        // Prefetch after the pass, so other folders never wait for it.
        self.prefetch = folders
            .iter()
            .find(|f| f.role == Role::Inbox)
            .map(|f| Prefetch {
                folder_id: f.id,
                budget_left: PREFETCH.pass_budget,
            });
        if errors.is_empty() {
            Ok(())
        } else {
            Err(pass_error(errors))
        }
    }

    /// One step of idle work: a backfill chunk (local and short, so it goes
    /// before the network), else a prefetch batch.
    fn idle_step(&mut self, db: &mut Db) {
        if self.backfill != Backfill::Running {
            return self.prefetch_step(db);
        }
        match db.backfill_addresses(self.account.id, db::BACKFILL_CHUNK) {
            Ok(true) => {}
            // Done, or the cache was busy: the next start carries on.
            Ok(false) | Err(_) => self.backfill = Backfill::Done,
        }
    }

    /// Fetch one batch of prefetch bodies. An error or nothing left to
    /// fetch ends the prefetch; the sync pass is already done.
    fn prefetch_step(&mut self, db: &mut Db) {
        let Some(Prefetch {
            folder_id,
            mut budget_left,
        }) = self.prefetch
        else {
            return;
        };
        let result = self
            .with_imap(|imap, _| prefetch_batch(imap, db, folder_id, &PREFETCH, &mut budget_left));
        self.prefetch = match result {
            Ok(stored) if stored > 0 => {
                (self.notify)(Event::FolderChanged {
                    folder_id: Some(folder_id),
                });
                Some(Prefetch {
                    folder_id,
                    budget_left,
                })
            }
            _ => None,
        };
    }

    fn fetch_body(&mut self, db: &mut Db, message_id: i64) -> Result<()> {
        let m = db
            .message(message_id)?
            .ok_or_else(|| anyhow!("message is gone"))?;
        if m.raw.is_none() {
            let (folder, v) = folder_ref(db, m.folder_id)?;
            let mut bodies = self
                .with_imap(|imap, _| imap.fetch_bodies((&folder, v), &[m.uid]))
                .inspect_err(|e| {
                    if uidvalidity_changed(e) {
                        (self.notify)(Event::NeedsSync {
                            folder_id: m.folder_id,
                        });
                    }
                })?;
            let (_, raw) = bodies
                .pop()
                .ok_or_else(|| anyhow!("the server no longer has this message"))?;
            store_body(db, message_id, &raw)?;
        }
        (self.notify)(Event::BodyReady { message_id });
        Ok(())
    }

    /// Copy draft `row` to the Drafts folder if it changed since its last
    /// copy, then remove that copy. The new copy is found by its Message-ID
    /// (the newest UID with it) and recorded before the old one goes, so a failure leaves two copies, never none. Errors are
    /// quiet: the draft is safe here, and the next sync pass tries again.
    fn save_draft(&mut self, db: &Db, row: i64) {
        // Debug builds: `KOMOREBI_NO_SERVER_DRAFTS` keeps drafts on this PC,
        // for live tests that must not add or remove server messages.
        #[cfg(debug_assertions)]
        if crate::dev_var("KOMOREBI_NO_SERVER_DRAFTS").is_some() {
            return;
        }
        if self.drafts_noted {
            return;
        }
        if let Err(e) = self.try_save_draft(db, row)
            && !is_connection_error(&e)
            && !self.auth_failed.get()
        {
            self.note_drafts(Notice::DraftsNotSaved(format!("{e:#}")));
        }
    }

    fn try_save_draft(&mut self, db: &Db, row: i64) -> Result<()> {
        let id = self.account.id;
        let changed = db.drafts_to_upload(id)?;
        let Some(&(_, rev)) = changed.iter().find(|d| d.0 == row) else {
            return Ok(());
        };
        let Some(draft) = db.draft(row)? else {
            return Ok(());
        };
        let Some(folder) = db.folder_by_role(id, Role::Drafts)? else {
            // Without an inbox the folder list isn't known yet.
            if db.folder_by_role(id, Role::Inbox)?.is_some() {
                self.note_drafts(Notice::NoDraftsFolder);
            }
            return Ok(());
        };
        let files = db.draft_files(row)?;
        if files.iter().map(|f| f.size).sum::<i64>() > MAX_DRAFT_COPY_ATTACHMENTS {
            return Ok(());
        }
        let mut attachments = Vec::with_capacity(files.len());
        for f in files {
            let data = db.read_draft_file(row, &f, MAX_DRAFT_COPY_ATTACHMENTS as u64)?;
            attachments.push(smtp::OutAttachment {
                filename: f.name,
                data,
            });
        }
        // A formatted draft is saved as it is stored (tagged, so it reads
        // back exactly), with its signature logo as the related part.
        let (body, body_html, inline) = match draft.body_rich.as_deref() {
            Some(stored) => {
                let inline = match rich::import::strict(stored) {
                    Some(doc) => db.inline_images(&doc)?,
                    None => Vec::new(),
                };
                (draft.body.clone(), Some(stored.to_string()), inline)
            }
            // A plain-text draft is plain text only, as it is sent.
            None => (draft.body.clone(), None, Vec::new()),
        };
        let out = smtp::Outgoing {
            from_name: self.account.name.clone(),
            from_email: self.account.email.clone(),
            to: draft.to,
            cc: draft.cc,
            bcc: draft.bcc,
            subject: draft.subject,
            body,
            body_html,
            inline,
            attachments,
            in_reply_to: draft.in_reply_to,
            references: draft.refs,
            message_id: draft.message_id.clone(),
            calendar: None,
            importance: draft.importance,
        };
        let raw = smtp::build_draft(out, row)?;
        // Signed in (and a stale connection replaced) first, so the APPEND
        // itself is never retried: cut off after the server stored it, a
        // retry would leave a copy that no later save removes.
        self.with_imap(|_, _| Ok(()))?;
        let imap = self.imap.as_mut().context("not connected")?;
        let appended = imap.append_draft(&folder.name, &raw);
        imap.touch();
        if appended.as_ref().is_err_and(is_connection_error) {
            self.imap = None;
        }
        appended?;
        let found =
            self.with_imap(|imap, _| imap.find_message_id(&folder.name, &draft.message_id))?;
        let copy = found.map(|(uidvalidity, uid)| db::ServerCopy {
            account_id: id,
            folder_id: folder.id,
            uidvalidity,
            uid,
        });
        let mut found_new = copy.is_some();
        match db.record_server_copy(row, rev, copy.as_ref())? {
            // The search found no new copy, only the old one (or nothing):
            // delete nothing.
            db::Recorded::Replaced(Some(old)) if copy.is_none_or(|c| c == old) => {
                found_new = false;
            }
            db::Recorded::Replaced(old) => {
                if let Some(old) = old {
                    self.remove_draft_copy(db, old);
                }
            }
            // Sent or discarded meanwhile: this copy is not wanted either.
            db::Recorded::NotADraft => {
                if let Some(copy) = copy {
                    self.remove_draft_copy(db, copy);
                }
            }
        }
        if !found_new {
            // Every later save would leave another copy behind.
            self.note_drafts(Notice::NoDraftUid);
        }
        Ok(())
    }

    /// Say why drafts stay local, once per session, and stop copying them.
    fn note_drafts(&mut self, message: Notice) {
        if !std::mem::replace(&mut self.drafts_noted, true) {
            (self.notify)(Event::Notice {
                account_id: self.account.id,
                message,
            });
        }
    }

    /// Remove a draft's server copy like a delete: flagged \Deleted, and
    /// expunged by UID only with UIDPLUS. A copy of another account goes to
    /// that account's worker; one whose folder's UIDVALIDITY changed is
    /// left alone, since its UID may now name another message. Best effort:
    /// a copy left behind is only a stale draft.
    fn remove_draft_copy(&mut self, db: &Db, copy: db::ServerCopy) {
        if copy.account_id != self.account.id {
            return (self.notify)(Event::DraftCopyLeft(copy));
        }
        let Ok(Some(folder)) = db.folder(copy.folder_id) else {
            return;
        };
        self.with_imap(|imap, _| {
            imap.delete_messages((&folder.name, copy.uidvalidity), &[copy.uid])
        })
        .ok();
    }

    /// Take queued outbox row `row`, send it and settle the row. A row no
    /// longer queued (undone, discarded, or another send has it) is left
    /// alone, so a message is never sent twice from one Send.
    fn send_row(&mut self, db: &mut Db, row: i64) {
        let out = match db.claim_outbox(row) {
            Ok(Some(out)) => out,
            Ok(None) => return,
            Err(e) => return self.fail(e),
        };
        let reply_to = out.reply_to_msg;
        let mut outcome = self.deliver(db, row, &out);
        if !outcome.delivered() {
            let now = chrono::Utc::now().timestamp();
            let (state, next_try, settled) = settle(outcome, out.attempts, now);
            if let Err(e) = db.outbox_result(row, state, next_try, settled.reason()) {
                self.fail(e);
            }
            outcome = settled;
        }
        (self.notify)(Event::Sent {
            row,
            reply_to,
            outcome,
        });
    }

    /// Send the built bytes of `out`, then (once the server took them)
    /// delete outbox row `row`, save the same bytes to Sent and mark the
    /// original answered.
    fn deliver(&mut self, db: &mut Db, row: i64, out: &db::Outbound) -> Outcome {
        let envelope = match smtp::envelope(&self.account.email, &out.to, &out.cc, &out.bcc) {
            Ok(env) => env,
            Err(e) => return Outcome::Failed(format!("{e:#}").into()),
        };
        let started = Instant::now();
        let sent = (self.smtp)(&self.account, &envelope, &out.raw);
        let status = match &sent {
            Ok(()) => "OK",
            Err(e) => errors::classify(e).name(),
        };
        diag::record(self.account.id, "SMTP SEND", status, started.elapsed());
        if let Err(e) = sent {
            // Its own sentence: the server won't send as this address.
            if smtp::sender_refused(&e) {
                return Outcome::Failed(Reason::said(errors::sender_refused(
                    &self.account.smtp.host,
                    &self.account.email,
                )));
            }
            return send_failure(&e);
        }
        // The message is out: drop its row first, so nothing can send it
        // again (if this fails, the row stays Sending and the next start
        // marks it unconfirmed). Nothing below may report it as not sent,
        // and a failed Sent copy still marks the original as answered.
        let draft_copy = match db.delete_draft(row) {
            Ok(copy) => copy,
            Err(e) => {
                self.fail(e);
                None
            }
        };
        let saved = if !self.account.save_sent {
            Ok(())
        } else if let Some(folder) = db
            .folder_by_role(self.account.id, Role::Sent)
            .ok()
            .flatten()
        {
            smtp::sent_copy(&out.raw, &out.bcc)
                .and_then(|raw| self.with_imap(|imap, _| imap.append(&folder.name, &raw)))
                .context("saving a copy to Sent failed")
                .map(|()| {
                    (self.notify)(Event::NeedsSync {
                        folder_id: folder.id,
                    })
                })
        } else if db
            .folder_by_role(self.account.id, Role::Inbox)
            .ok()
            .flatten()
            .is_some()
        {
            // The folder list is known and has no Sent folder: nothing is
            // created for it. The message is out; say so once.
            if !self.sent_noted.swap(true, Ordering::Relaxed) {
                (self.notify)(Event::Notice {
                    account_id: self.account.id,
                    message: Notice::NoSentFolder,
                });
            }
            Ok(())
        } else {
            Err(anyhow!("there is no Sent folder, so no copy was saved"))
        };
        if let Some(original) = out.reply_to_msg
            && let Ok(Some(m)) = db.message(original)
        {
            db.set_answered(original).ok();
            if let Ok((folder, v)) = folder_ref(db, m.folder_id) {
                self.with_imap(|imap, _| {
                    imap.set_flags((&folder, v), &[m.uid], "\\Answered", true)
                })
                .ok();
            }
        }
        if let Some(copy) = draft_copy {
            self.remove_draft_copy(db, copy);
        }
        match saved {
            Ok(()) => Outcome::Delivered,
            Err(e) => Outcome::DeliveredNotSaved(format!("{e:#}")),
        }
    }
}

/// One error for a pass from its failed folders (name, error): the last
/// keeps its type, so it is classed as before; the others' text goes first.
fn pass_error(mut failed: Vec<(String, anyhow::Error)>) -> anyhow::Error {
    let Some((name, last)) = failed.pop() else {
        return anyhow!("sync failed");
    };
    let mut label: Vec<String> = failed.iter().map(|(n, e)| format!("{n}: {e:#}")).collect();
    label.push(name);
    last.context(label.join("; "))
}

/// Whether `f` can be left alone this pass: [`can_skip`] on its stored
/// fingerprint and the cache.
fn skippable(
    db: &Db,
    f: &Folder,
    st: Option<&FolderStatus>,
    pending: &Pending,
    full: bool,
) -> Result<bool> {
    let fp = db.fingerprint(f.id)?;
    let local = db.count_messages(f.id)?;
    let idle = pending.uids(f.id).is_empty();
    Ok(can_skip(fp, f.uidvalidity, st, local, idle, full))
}

/// True only if the server's STATUS equals the fingerprint of the folder's
/// last complete sync and the cache holds exactly that many messages, with
/// no local action in flight. Equality only: a stored value above the
/// server's means a reset, so the full path runs. A folder holding messages
/// flagged \Deleted never skips (they aren't cached, so the counts differ);
/// neither does a NOMODSEQ folder (`hms == 0`) or one without a STATUS line.
/// MESSAGES and UIDNEXT must match too, because a CONDSTORE-only server
/// need not raise HIGHESTMODSEQ on expunge.
fn can_skip(
    fp: Option<Fp>,
    stored_uv: u32,
    st: Option<&FolderStatus>,
    local: u32,
    pending_empty: bool,
    force: bool,
) -> bool {
    let (Some(fp), Some(st)) = (fp, st) else {
        return false;
    };
    !force
        && pending_empty
        && st.hms > 0
        && st.uidvalidity != 0
        && st.uidvalidity == stored_uv
        && st.uidnext == fp.uidnext
        && st.messages == fp.messages
        && st.hms == fp.hms
        && local == st.messages
}

/// What a folder sync did (for tools and measurements).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FolderSync {
    /// The changed-folder delta ran instead of the full path.
    pub delta: bool,
    /// Headers stored.
    pub new: usize,
    /// Messages a CHANGEDSINCE fetch reported (delta only).
    pub flag_changes: usize,
}

/// Bring the folder's cache in line with the server. `st` is the folder's
/// STATUS read before the SELECT (none for a single-folder sync). With it,
/// and unless `full`, a folder with a fingerprint takes the delta when it
/// can (see [`sync_delta`]). Only if every step succeeds and no action was
/// in flight is `st` recorded as the folder's fingerprint, so a change since
/// the STATUS shows up as a mismatch on the next pass, never as a skip.
/// `ramp` is the first header batch size, kept across a reconnect so the
/// retry starts smaller (see [`sync_full`]).
#[allow(clippy::too_many_arguments)]
fn sync_folder(
    imap: &mut Imap,
    db: &mut Db,
    f: &Folder,
    notify: &Notify,
    pending: &Pending,
    st: Option<&FolderStatus>,
    full: bool,
    ramp: &mut usize,
) -> Result<FolderSync> {
    let sel = imap.select(&f.name)?;
    if db.set_uidvalidity(f.id, sel.uidvalidity)? {
        folder_changed(notify, f);
    }
    let mut stats = FolderSync::default();
    let delta = match st {
        Some(st) if !full => sync_delta(imap, db, f, pending, &sel, st, &mut stats)?,
        _ => None,
    };
    let hid_pending = match delta {
        Some(d) => {
            stats.delta = true;
            if d.changed {
                folder_changed(notify, f);
            }
            d.hid_pending
        }
        None => sync_full(imap, db, f, notify, pending, &sel, ramp, &mut stats)?,
    };
    if let Some(st) = st
        && !hid_pending
        && pending.uids(f.id).is_empty()
        && st.hms > 0
        && sel.uidvalidity == st.uidvalidity
    {
        let fp = Fp {
            uidnext: st.uidnext,
            messages: st.messages,
            hms: st.hms,
        };
        db.finish_folder(f.id, fp)?;
    }
    Ok(stats)
}

fn folder_changed(notify: &Notify, f: &Folder) {
    notify(Event::FolderChanged {
        folder_id: Some(f.id),
    });
}

/// The full path: every UID and its flags in one fetch, then removals, new
/// headers and flag updates. Returns whether an action in flight kept a row
/// from matching the server.
///
/// New headers come newest first in batches that start at `*ramp` and
/// follow [`next_header_batch`], each stored and shown before the next is
/// asked for. If a batch fails on the connection, `*ramp` becomes half its
/// size, so the reconnect's retry (or the next pass) starts smaller; it
/// fetches whatever is still missing, so an interrupted ramp leaves no gap.
#[allow(clippy::too_many_arguments)]
fn sync_full(
    imap: &mut Imap,
    db: &mut Db,
    f: &Folder,
    notify: &Notify,
    pending: &Pending,
    sel: &imap::Selected,
    ramp: &mut usize,
    stats: &mut FolderSync,
) -> Result<bool> {
    // Read the cache first: a row inserted after this point (e.g. by the
    // other worker) is then never mistaken for one the server dropped.
    let known = db.known_uids(f.id)?;
    let mut remote = if sel.exists == 0 {
        HashMap::new()
    } else {
        imap.uid_flags()?
    };
    // Messages flagged \Deleted are hidden, and come back if unflagged.
    remote.retain(|_, bits| *bits & db::DELETED == 0);
    let gone: Vec<u32> = known
        .iter()
        .filter(|u| !remote.contains_key(u))
        .copied()
        .collect();
    if !gone.is_empty() {
        db.delete_uids(f.id, &gone)?;
        folder_changed(notify, f);
    }
    let mut new: Vec<u32> = remote
        .keys()
        .filter(|u| !known.contains(u))
        .copied()
        .collect();
    new.sort_unstable_by(|a, b| b.cmp(a)); // newest first
    // An action in flight may have kept a row from matching the server,
    // so the sync doesn't count as complete.
    let mut hid_pending = false;
    // A long download (a first sync) says how far it is, out of the
    // folder's messages as the UID list just read counts them.
    let total = remote.len();
    let mut cached = known.len() - gone.len();
    let (mut rest, mut size) = (&new[..], *ramp);
    while !rest.is_empty() {
        let (chunk, tail) = rest.split_at(size.min(rest.len()));
        rest = tail;
        let t = Instant::now();
        let mut headers = imap.fetch_headers(chunk).inspect_err(|e| {
            if is_connection_error(e) {
                *ramp = (size / 2).max(FIRST_HEADER_BATCH);
            }
        })?;
        size = next_header_batch(size, t.elapsed());
        // A message the user moved or deleted locally still exists on the
        // server until the interactive worker gets to it; don't re-add it.
        let busy = pending.uids(f.id);
        hid_pending |= !busy.is_empty();
        headers.retain(|h| !busy.contains(&h.uid));
        db.insert_headers(f.account_id, f.id, &headers)?;
        stats.new += headers.len();
        cached += chunk.len();
        if new.len() >= PROGRESS_FROM {
            notify(Event::Progress {
                account_id: f.account_id,
                folder_id: f.id,
                done: cached.min(total),
                total,
            });
        }
        folder_changed(notify, f);
    }
    if !known.is_empty() && !remote.is_empty() {
        let busy = pending.uids(f.id);
        hid_pending |= !busy.is_empty();
        if db.apply_flags(f.id, &remote, &busy)? > 0 {
            folder_changed(notify, f);
        }
    }
    Ok(hid_pending)
}

/// The header batch after one of `size` that took `elapsed`: sized for a
/// round trip of about [`HEADER_BATCH_TARGET`], at most double the last,
/// and within [`FIRST_HEADER_BATCH`]..=[`HEADER_BATCH`].
fn next_header_batch(size: usize, elapsed: Duration) -> usize {
    let ms = elapsed.as_millis().max(1);
    let aim = size as u128 * HEADER_BATCH_TARGET.as_millis() / ms;
    let aim = usize::try_from(aim).unwrap_or(usize::MAX);
    aim.min(size * 2).clamp(FIRST_HEADER_BATCH, HEADER_BATCH)
}

/// What the delta did, for [`sync_folder`].
#[derive(Default)]
struct Delta {
    /// An action in flight kept a row from matching the server.
    hid_pending: bool,
    /// The cache changed.
    changed: bool,
}

/// Whether the changed-folder delta may run: the server's state must carry
/// on from the fingerprint `fp`. UIDNEXT or HIGHESTMODSEQ below the
/// fingerprint's means a reset, and a tail longer than one header batch is
/// left to the full path, as is an empty folder (`n:*` needs a message).
/// The stored UIDVALIDITY already equals the SELECT's (a change drops the
/// fingerprint).
fn can_delta(fp: Fp, st: &FolderStatus, sel: &imap::Selected, pending_empty: bool) -> bool {
    pending_empty
        && sel.exists > 0
        && st.hms > 0
        && st.uidvalidity != 0
        && sel.uidvalidity == st.uidvalidity
        && sel.uid_next >= fp.uidnext
        && st.hms >= fp.hms
        && sel.uid_next - fp.uidnext <= HEADER_BATCH as u32
}

/// The changed-folder delta against the fingerprint of the folder's last
/// complete sync: headers from its UIDNEXT up, flags changed since its
/// HIGHESTMODSEQ (CHANGEDSINCE), and, only if the cached count differs from
/// EXISTS, removals found with UID SEARCH ALL. CHANGEDSINCE never reports
/// expunges, so that count check and the hourly forced full pass are the
/// mandatory backstops for them; a ghost row that keeps the counts equal
/// lasts until the forced pass. Deletions only mirror the server's UID list
/// from the same SELECT snapshot. `None` means the full path must run
/// instead, in the same pass.
fn sync_delta(
    imap: &mut Imap,
    db: &mut Db,
    f: &Folder,
    pending: &Pending,
    sel: &imap::Selected,
    st: &FolderStatus,
    stats: &mut FolderSync,
) -> Result<Option<Delta>> {
    let Some(fp) = db.fingerprint(f.id)? else {
        return Ok(None);
    };
    if !can_delta(fp, st, sel, pending.uids(f.id).is_empty()) {
        return Ok(None);
    }
    let mut out = Delta::default();
    if sel.uid_next != fp.uidnext {
        let mut headers = imap.fetch_headers_from(fp.uidnext)?;
        let busy = pending.uids(f.id);
        out.hid_pending |= !busy.is_empty();
        headers.retain(|h| !busy.contains(&h.uid));
        db.insert_headers(f.account_id, f.id, &headers)?;
        stats.new = headers.len();
        out.changed |= !headers.is_empty();
    }
    if st.hms != fp.hms {
        // Read the cache first, as on the full path; after the tail insert, so
        // a tail row flagged \Deleted since is hidden too.
        let known = db.known_uids(f.id)?;
        let mut delta = imap.uid_flags_changed(fp.hms)?;
        stats.flag_changes = delta.len();
        let busy = pending.uids(f.id);
        out.hid_pending |= delta.keys().any(|u| busy.contains(u));
        // Newly flagged \Deleted: hidden, as on the full path.
        let hidden: Vec<u32> = delta
            .iter()
            .filter(|&(u, bits)| bits & db::DELETED != 0 && known.contains(u) && !busy.contains(u))
            .map(|(u, _)| *u)
            .collect();
        delta.retain(|_, bits| *bits & db::DELETED == 0);
        // Unflagged \Deleted (so not cached): the full path adds it back.
        if delta
            .keys()
            .any(|u| *u < fp.uidnext && !known.contains(u) && !busy.contains(u))
        {
            return Ok(None);
        }
        if !hidden.is_empty() {
            db.delete_uids(f.id, &hidden)?;
            out.changed = true;
        }
        if db.apply_flag_delta(f.id, &delta, &busy)? > 0 {
            out.changed = true;
        }
    }
    if db.count_messages(f.id)? != sel.exists {
        let known = db.known_uids(f.id)?;
        let server = imap.uids()?;
        // The same snapshot as the SELECT only if the counts agree. This
        // also keeps an empty answer from emptying a non-empty folder.
        if server.len() != sel.exists as usize {
            return Ok(None);
        }
        let busy = pending.uids(f.id);
        out.hid_pending |= known
            .symmetric_difference(&server)
            .any(|u| busy.contains(u));
        let gone: Vec<u32> = known
            .difference(&server)
            .filter(|u| !busy.contains(u))
            .copied()
            .collect();
        let unknown: Vec<u32> = server
            .difference(&known)
            .filter(|u| !busy.contains(u))
            .copied()
            .collect();
        // On the server but not cached: fine if flagged \Deleted (hidden);
        // anything else only the full path adds.
        if unknown.len() > HEADER_BATCH
            || imap
                .uid_flags_of(&unknown)?
                .values()
                .any(|bits| bits & db::DELETED == 0)
        {
            return Ok(None);
        }
        if !gone.is_empty() {
            db.delete_uids(f.id, &gone)?;
            out.changed = true;
        }
    }
    Ok(Some(out))
}

/// Refresh an account's folder list from the server and return its folders
/// with their STATUS by server name: one LIST-STATUS round trip if the
/// server can, else LIST plus a STATUS per folder (only with CONDSTORE).
/// A folder without an entry takes the full sync path.
pub fn refresh_folders(
    imap: &mut Imap,
    db: &Db,
    account_id: i64,
) -> Result<(Vec<Folder>, Statuses)> {
    let (remote, statuses) = match imap.list_status()? {
        Some(both) => both,
        None => {
            let remote = imap.list_folders()?;
            let mut statuses = HashMap::new();
            for r in &remote {
                if let Some(st) = imap.status(&r.name)? {
                    statuses.insert(r.name.clone(), st);
                }
            }
            (remote, statuses)
        }
    };
    let mut keep = Vec::new();
    for r in remote {
        keep.push(db.upsert_folder(account_id, &r.name, &r.display, r.depth, r.role)?);
    }
    db.retain_folders(account_id, &keep)?;
    let folders = db
        .folders()?
        .into_iter()
        .filter(|f| f.account_id == account_id)
        .collect();
    Ok((folders, statuses))
}

/// Sync one folder outside the workers (tools and measurements), always in
/// full and without recording a fingerprint.
pub fn sync_folder_once(imap: &mut Imap, db: &mut Db, folder: &Folder) -> Result<FolderSync> {
    let notify: Notify = Arc::new(|_| {});
    sync_folder_notify(imap, db, folder, &notify)
}

/// [`sync_folder_once`], reporting to `notify` as the sync worker does (a
/// [`Event::FolderChanged`] after each header batch is stored).
pub fn sync_folder_notify(
    imap: &mut Imap,
    db: &mut Db,
    folder: &Folder,
    notify: &Notify,
) -> Result<FolderSync> {
    let mut ramp = FIRST_HEADER_BATCH;
    let pending = Pending::default();
    sync_folder(imap, db, folder, notify, &pending, None, true, &mut ramp)
}

/// Sync one folder outside the workers with its STATUS `st`, as a
/// steady-state pass does: the delta where it applies, and the fingerprint
/// recorded on success.
pub fn sync_folder_with_status(
    imap: &mut Imap,
    db: &mut Db,
    folder: &Folder,
    st: &FolderStatus,
) -> Result<FolderSync> {
    let notify: Notify = Arc::new(|_| {});
    let pending = Pending::default();
    let mut ramp = FIRST_HEADER_BATCH;
    sync_folder(
        imap,
        db,
        folder,
        &notify,
        &pending,
        Some(st),
        false,
        &mut ramp,
    )
}

/// What a sync pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PassStats {
    pub folders: usize,
    pub skipped: usize,
}

/// One sync pass over every folder outside the workers (tools and
/// measurements), with the sync worker's folder order, skip gate and
/// fingerprint rules. Unlike the worker it stops at the first error.
pub fn sync_pass_once(
    imap: &mut Imap,
    db: &mut Db,
    account_id: i64,
    full: bool,
) -> Result<PassStats> {
    let notify: Notify = Arc::new(|_| {});
    let pending = Pending::default();
    let (mut folders, statuses) = refresh_folders(imap, db, account_id)?;
    folders.sort_by_key(|f| f.role != Role::Inbox);
    let mut stats = PassStats {
        folders: folders.len(),
        skipped: 0,
    };
    for f in &folders {
        let st = statuses.get(&f.name);
        if skippable(db, f, st, &pending, full)? {
            stats.skipped += 1;
        } else {
            let mut ramp = FIRST_HEADER_BATCH;
            sync_folder(imap, db, f, &notify, &pending, st, full, &mut ramp)?;
        }
    }
    imap.drain_unsolicited();
    imap.leave_folder()?;
    Ok(stats)
}

/// Prefetch one sync pass's worth of `folder`'s bodies under `policy`.
/// Returns how many were stored.
pub fn prefetch_bodies(
    imap: &mut Imap,
    db: &mut Db,
    folder: &Folder,
    policy: &PrefetchPolicy,
) -> Result<usize> {
    let mut budget = policy.pass_budget;
    let mut stored = 0;
    loop {
        match prefetch_batch(imap, db, folder.id, policy, &mut budget)? {
            0 => return Ok(stored),
            n => stored += n,
        }
    }
}

/// Download and store the next batch of missing bodies in the folder's
/// window, taking what it requests off `budget`. Returns how many were
/// stored; 0 means the prefetch is done.
fn prefetch_batch(
    imap: &mut Imap,
    db: &mut Db,
    folder_id: i64,
    policy: &PrefetchPolicy,
    budget: &mut i64,
) -> Result<usize> {
    // Asked again every batch, so a sync in between is taken into account.
    let missing = db.missing_bodies(folder_id, policy.window, policy.max_msg)?;
    let batch = next_batch(&missing, policy, *budget);
    if batch.is_empty() {
        return Ok(0);
    }
    // The cached UIDVALIDITY now, not when the folder was loaded.
    let (name, uidvalidity) = folder_ref(db, folder_id)?;
    let uids: Vec<u32> = batch.iter().map(|m| m.1).collect();
    // Parse first, then write the whole batch in one short transaction.
    let parsed: Vec<_> = imap
        .fetch_bodies((&name, uidvalidity), &uids)?
        .into_iter()
        .filter_map(|(uid, raw)| {
            let (id, _, _) = batch.iter().find(|m| m.1 == uid)?;
            let body = mime::parse_body(&raw);
            let attach = !body.attachments.is_empty();
            Some((*id, raw, body.preview(), attach, body.importance))
        })
        .collect();
    let items: Vec<_> = parsed
        .iter()
        .map(|(id, raw, preview, attach, importance)| {
            (*id, raw.as_slice(), preview.as_str(), *attach, *importance)
        })
        .collect();
    db.set_bodies(&items)?;
    let stored = items.len();
    *budget -= batch.iter().map(|m| m.2).sum::<i64>();
    Ok(stored)
}

/// The leading `missing` rows (id, UID, size) to fetch in one round trip:
/// at most 25 and `batch_bytes`, stopping before the first row that would
/// overrun `budget`.
fn next_batch<'a>(
    missing: &'a [(i64, u32, i64)],
    policy: &PrefetchPolicy,
    budget: i64,
) -> &'a [(i64, u32, i64)] {
    let (mut n, mut bytes) = (0, 0);
    for &(_, _, size) in missing.iter().take(PREFETCH_BATCH) {
        if bytes + size > budget || (n > 0 && bytes + size > policy.batch_bytes) {
            break;
        }
        n += 1;
        bytes += size;
    }
    &missing[..n]
}

/// Whether a sync pass ignores STATUS: when asked (start, Refresh), on a
/// worker's first pass, and at least every [`FULL_PASS_EVERY`] passes. This
/// bounds how long a server bug can hide a change. `since` counts the
/// passes since the last full one.
fn full_pass(since: &mut Option<u32>, asked: bool) -> bool {
    let full = asked || since.is_none_or(|n| n + 1 >= FULL_PASS_EVERY);
    *since = Some(match *since {
        Some(n) if !full => n + 1,
        _ => 0,
    });
    full
}

/// Move every queued job into `backlog`, then drop the sync requests `job`
/// covers: a `SyncAll` covers every sync (and becomes full if one of them
/// was), a `SyncFolder` its own folder's (and becomes full likewise).
/// False if the channel is closed (the account was removed).
fn coalesce(job: &mut Job, rx: &Receiver<Job>, backlog: &mut VecDeque<Job>) -> bool {
    loop {
        match rx.try_recv() {
            Ok(queued) => backlog.push_back(queued),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
    match job {
        Job::SyncAll { full } => backlog.retain(|j| match *j {
            Job::SyncAll { full: f } => {
                *full |= f;
                false
            }
            Job::SyncFolder { .. } => false,
            _ => true,
        }),
        Job::SyncFolder { id, full } => backlog.retain(|j| match *j {
            Job::SyncFolder { id: other, full: f } if other == *id => {
                *full |= f;
                false
            }
            _ => true,
        }),
        _ => {}
    }
    true
}

/// The next queued `SyncFolder` (id, full), if any; other jobs queued
/// before it move to `backlog`.
fn next_sync_folder(rx: &Receiver<Job>, backlog: &mut VecDeque<Job>) -> Option<(i64, bool)> {
    loop {
        match rx.try_recv() {
            Ok(Job::SyncFolder { id, full }) => return Some((id, full)),
            Ok(other) => backlog.push_back(other),
            Err(_) => return None,
        }
    }
}

pub fn store_body(db: &mut Db, id: i64, raw: &[u8]) -> Result<()> {
    let body = mime::parse_body(raw);
    let attach = !body.attachments.is_empty();
    db.set_body(id, raw, &body.preview(), attach, body.importance)
}

/// A folder's server name and cached UIDVALIDITY.
fn folder_ref(db: &Db, folder_id: i64) -> Result<(String, u32)> {
    let f = db.folder(folder_id)?.ok_or(errors::FolderGone)?;
    Ok((f.name, f.uidvalidity))
}

/// A failed SMTP send: a connection lost mid-session is
/// [`Outcome::Unconfirmed`]; unreachable or refused for now is
/// [`Outcome::Retry`]; any other refusal is [`Outcome::Failed`].
fn send_failure(e: &anyhow::Error) -> Outcome {
    // A changed or untrusted certificate: trying again can't help, and the
    // outbox must not loop on it. The row fails with the sentence; the user
    // fixes the account and sends it again.
    let problem = Problem::of(e);
    if problem.class == Class::TlsCert {
        let say = problem.say().unwrap_or(Say::Class(Class::TlsCert));
        return Outcome::Failed(Reason::said(say));
    }
    let detail = format!("{e:#}");
    // A sentence of its own: a rejected sign-in that is waited out, or a
    // message the server found too large.
    let say = if e.chain().any(|c| c.is::<errors::SignInLatched>()) {
        Some(Say::SignInLatched)
    } else if smtp::too_large(e) {
        Some(Say::TooLargeSend {
            detail: detail.clone(),
        })
    } else {
        None
    };
    let reason = Reason {
        text: say.as_ref().and_then(Say::record).unwrap_or(detail),
        say,
    };
    if smtp::may_have_sent(e) {
        Outcome::Unconfirmed(Reason::said(Say::Unconfirmed {
            detail: reason.text,
        }))
    } else if smtp::retryable(e) {
        Outcome::Retry(reason)
    } else {
        Outcome::Failed(reason)
    }
}

/// The wait before the next automatic try after `attempts` failed ones:
/// 30 s, doubling, at most 15 minutes. `None`: out of tries.
pub fn retry_delay(attempts: i64) -> Option<i64> {
    if !(1..MAX_SEND_ATTEMPTS).contains(&attempts) {
        return None;
    }
    Some((RETRY_FIRST << (attempts - 1)).min(RETRY_MAX))
}

/// Where a send that did not deliver leaves its outbox row, given the
/// tries so far (this one included) and the time: (state, next try, the
/// outcome to report). A retry past the last try is reported as failed.
fn settle(outcome: Outcome, attempts: i64, now: i64) -> (db::Outbox, Option<i64>, Outcome) {
    match outcome {
        Outcome::Retry(e) => match retry_delay(attempts) {
            Some(wait) => (db::Outbox::Queued, Some(now + wait), Outcome::Retry(e)),
            None => (db::Outbox::Failed, None, Outcome::Failed(e)),
        },
        Outcome::Unconfirmed(e) => (db::Outbox::Unconfirmed, None, Outcome::Unconfirmed(e)),
        other => (db::Outbox::Failed, None, other),
    }
}

fn uidvalidity_changed(e: &anyhow::Error) -> bool {
    e.chain()
        .any(|c| c.downcast_ref::<imap::UidValidityChanged>().is_some())
}

/// The saved certificates of an account's two servers (IMAP, SMTP), as
/// the checks take them.
pub type Pins<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

/// Check an account's servers and password before saving them, step by
/// step ([`errors::Step`]): the name, the connection, TLS, the IMAP sign-in
/// and folder list, then the SMTP connection and sign-in (nothing is sent),
/// each with its own login name (`Account::imap_login`,
/// `Account::smtp_login`). `report` hears each step as it passes; the error
/// names the one that failed, with a sentence for the user
/// ([`errors::StepError::text`]). Nothing is saved here. The diagnostics
/// log gets the step and class of a failure, never the server's words.
pub fn verify_account(
    imap: &Server,
    smtp: &Server,
    pins: Pins,
    logins: (&str, &str),
    password: &str,
    report: &mut dyn FnMut(errors::Step),
) -> Result<(), errors::StepError> {
    let began = std::time::Instant::now();
    let result = verify_steps(imap, smtp, pins, logins, password, report);
    if let Err(e) = &result {
        diag::record(
            0,
            "VERIFY",
            &format!("{} {}", e.step.name(), e.class.name()),
            began.elapsed(),
        );
    }
    result
}

fn verify_steps(
    imap: &Server,
    smtp: &Server,
    pins: Pins,
    logins: (&str, &str),
    password: &str,
    report: &mut dyn FnMut(errors::Step),
) -> Result<(), errors::StepError> {
    run_steps(
        imap,
        pins.0.is_some(),
        logins.0,
        |report| Imap::connect_steps(imap, pins.0, logins.0, password, Some(report)),
        |report| smtp::verify_smtp(smtp, pins.1, logins.1, password, report),
        report,
    )
}

/// The sequence of [`verify_account`] around its two connections: IMAP
/// (`connect` reports its own steps), the folder list, then SMTP.
fn run_steps(
    imap: &Server,
    pinned: bool,
    imap_login: &str,
    connect: impl FnOnce(&mut dyn FnMut(errors::Step)) -> Result<Imap, imap::StepFailure>,
    smtp_check: impl FnOnce(&mut dyn FnMut(errors::Step)) -> Result<(), errors::StepError>,
    report: &mut dyn FnMut(errors::Step),
) -> Result<(), errors::StepError> {
    use errors::Step;
    let failed = |step, e| imap_step_error(step, e, imap, pinned, imap_login);
    let mut session = connect(report).map_err(|(s, e)| failed(s, e))?;
    session
        .list_folders()
        .map_err(|e| failed(Step::ListFolders, e))?;
    report(Step::ListFolders);
    session.logout();
    smtp_check(report)
}

/// A failed IMAP step as the form reports it.
fn imap_step_error(
    step: errors::Step,
    e: anyhow::Error,
    server: &Server,
    pinned: bool,
    login: &str,
) -> errors::StepError {
    use errors::{Class, Special, Step};
    let (class, special) = match step {
        Step::Resolve | Step::Connect => (Class::Offline, Special::None),
        Step::Secure if e.chain().any(|c| c.is::<imap::StartTlsRefused>()) => {
            (Class::TlsCert, Special::NoStartTls)
        }
        Step::Secure if e.downcast_ref::<imap::PlainAnswer>().is_some() => {
            (Class::TlsCert, Special::PlainAnswer)
        }
        Step::Secure | Step::ImapSignIn if e.chain().any(|c| c.is::<imap::GreetingRefused>()) => {
            (Class::Other, Special::BadGreeting)
        }
        Step::ImapSignIn if e.chain().any(|c| c.is::<imap::NoSignIn>()) => {
            (Class::Other, Special::NoLogin)
        }
        Step::ImapSignIn if e.chain().any(|c| c.is::<imap::Utf8Only>()) => {
            (Class::Other, Special::Utf8Only)
        }
        Step::Secure if pinned && errors::classify(&e) == Class::TlsCert => {
            (Class::TlsCert, Special::CertChanged)
        }
        _ => (errors::classify(&e), Special::None),
    };
    errors::StepError {
        step,
        class,
        special,
        host: server.host.clone(),
        port: server.port,
        login: login.to_string(),
        source: e,
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;
    use crate::db::{NewAccount, NewHeader};
    use crate::providers::Security;
    use errors::UNCONFIRMED;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    /// An account with a synced INBOX (UIDVALIDITY `uidvalidity`) holding
    /// `uids`; returns the folder.
    pub(super) fn setup(db: &mut Db, uidvalidity: u32, uids: &[u32]) -> Folder {
        let server = Server {
            host: "imap.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        let account_id = db
            .add_account(&NewAccount {
                imap_user: String::new(),
                smtp_user: String::new(),
                email: "me@example.com".into(),
                name: "Me".into(),
                imap: server.clone(),
                smtp: server,
                save_sent: false,
            })
            .unwrap();
        let id = db
            .upsert_folder(account_id, "INBOX", "Inbox", 0, Role::Inbox)
            .unwrap();
        db.set_uidvalidity(id, uidvalidity).unwrap();
        let headers: Vec<_> = uids
            .iter()
            .map(|&uid| NewHeader {
                uid,
                ..Default::default()
            })
            .collect();
        db.insert_headers(account_id, id, &headers).unwrap();
        db.folder(id).unwrap().unwrap()
    }

    /// A worker on `imap` whose events are collected.
    fn worker(db: &Db, imap: Imap) -> (Worker, Arc<Mutex<Vec<Event>>>) {
        worker_with(db, Some(imap))
    }

    pub(super) fn worker_with(db: &Db, imap: Option<Imap>) -> (Worker, Arc<Mutex<Vec<Event>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let w = Worker {
            account: db.accounts().unwrap().remove(0),
            db_path: PathBuf::new(),
            notify: Arc::new(move |e| sink.lock().unwrap().push(e)),
            imap,
            connect: Box::new(|_| Err(anyhow!("no server in this test"))),
            smtp: Box::new(|_, _, _| Err(anyhow!("no SMTP server in this test"))),
            auth_failed: errors::Latch::default(),
            latch_reported: false,
            pending: Pending::default(),
            prefetch: None,
            backfill: Backfill::Done,
            passes_since_full: None,
            ready: None,
            unselect_when_idle: false,
            drafts_noted: false,
            sent_noted: Arc::default(),
            resume: Arc::default(),
            resume_seen: 0,
            parked: Vec::new(),
        };
        (w, events)
    }

    #[test]
    fn unparseable_response_drops_the_connection() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Send("* VANISHED (EARLIER) 1\r\n{tag} OK FETCH completed\r\n"),
            Step::Close,
        ]);
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[1]);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        // No retry (the command may have run); the next job reconnects.
        w.run_job(
            &mut db,
            Job::SyncFolder {
                id: f.id,
                full: true,
            },
            &mut || None,
        );
        assert!(w.imap.is_none());
        let events = events.lock().unwrap();
        // A parse error, not a failed reconnect.
        assert!(
            matches!(&events[..], [Event::ActionFailed { error, .. }] if error.detail.contains("parse"))
        );
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([1]));
        server.finish();
    }

    /// Save as .eml, Show original and Forward as attachment on a message
    /// whose body isn't cached: the interactive worker fetches it, and the
    /// bytes kept are exactly those the server sent.
    #[test]
    fn body_fetched_on_demand_is_kept_byte_for_byte() {
        let raw = "Subject: =?UTF-8?Q?Ol=C3=A1?=\r\n\tagain\r\nX-8bit: café\r\n\r\nbody ✓\r\n";
        let reply = leak(format!(
            "* 1 FETCH (UID 5 BODY[] {{{}}}\r\n{raw})\r\n{{tag}} OK done\r\n",
            raw.len()
        ));
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 5 (UID BODY.PEEK[])"),
            Step::Send(reply),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        let id = db.message_ids(&[f.id], "").unwrap()[0];
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        w.run_job(&mut db, Job::FetchBody(id), &mut || None);
        assert!(matches!(
            events.lock().unwrap()[..],
            [Event::BodyReady { message_id }] if message_id == id
        ));
        let stored = db.message(id).unwrap().unwrap().raw.unwrap();
        assert_eq!(stored, raw.as_bytes());
        let original = source::original(&stored);
        assert_eq!(original.headers, "Subject: Olá again\nX-8bit: café");
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn changed_uidvalidity_refuses_fetch_body() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            // The server says 7; the cache has 1.
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
        ]);
        script.extend(logout_steps("a4 LOGOUT"));
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 1, &[5]);
        let id = db.message_ids(&[f.id], "").unwrap()[0];
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        w.run_job(&mut db, Job::FetchBody(id), &mut || None);
        assert!(db.message(id).unwrap().unwrap().raw.is_none());
        {
            let events = events.lock().unwrap();
            assert!(matches!(
                events[..],
                [Event::NeedsSync { folder_id }, Event::BodyFailed { message_id, .. }]
                    if folder_id == f.id && message_id == id
            ));
        }
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn deleted_messages_are_hidden_until_unflagged() {
        let reply = header_fetch(2, 2, "", "Subject: back\r\n\r\n") + "{tag} OK done\r\n";
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(concat!(
                "* 1 FETCH (UID 1 FLAGS ())\r\n",
                "* 2 FETCH (UID 2 FLAGS (\\Deleted))\r\n",
                "{tag} OK done\r\n",
            )),
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a6 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(concat!(
                "* 1 FETCH (UID 1 FLAGS ())\r\n",
                "* 2 FETCH (UID 2 FLAGS ())\r\n",
                "{tag} OK done\r\n",
            )),
            Step::Expect("a7 UID FETCH 2 (UID FLAGS INTERNALDATE"),
            Step::Send(leak(reply)),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[1, 2]);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        sync_folder_once(&mut imap, &mut db, &f).unwrap();
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([1]));
        sync_folder_once(&mut imap, &mut db, &f).unwrap();
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([1, 2]));
        imap.logout();
        server.finish();
    }

    #[test]
    fn row_inserted_during_flags_fetch_survives() {
        // Two connections to one in-memory database.
        const URI: &str = "file:komorebi-sync-race?mode=memory&cache=shared";
        let mut db = Db::open(Path::new(URI)).unwrap();
        let f = setup(&mut db, 7, &[1]);
        let (account_id, folder_id) = (f.account_id, f.id);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            // Another writer adds UID 9 while the server answers without it.
            Step::Then(Box::new(move || {
                let mut other = Db::open(Path::new(URI)).unwrap();
                let h = NewHeader {
                    uid: 9,
                    ..Default::default()
                };
                other.insert_headers(account_id, folder_id, &[h]).unwrap();
            })),
            Step::Send("* 1 FETCH (UID 1 FLAGS ())\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        sync_folder_once(&mut imap, &mut db, &f).unwrap();
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([1, 9]));
        imap.logout();
        server.finish();
    }

    /// Adds messages `uids` of `size` bytes each, dated by UID.
    fn add_sized(db: &mut Db, f: &Folder, uids: impl Iterator<Item = u32>, size: i64) {
        let headers: Vec<_> = uids
            .map(|uid| NewHeader {
                uid,
                date: uid.into(),
                size,
                ..Default::default()
            })
            .collect();
        db.insert_headers(f.account_id, f.id, &headers).unwrap();
    }

    #[test]
    fn a_failed_body_batch_stores_nothing_and_is_retried() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        add_sized(&mut db, &f, 1..=2, 100);
        let missing = db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE).unwrap();
        assert_eq!(missing.len(), 2);
        let raw: &[u8] = b"Subject: x

hello
";
        // The second row names a message that does not exist: the batch fails.
        let bad = [
            (missing[0].0, raw, "hello", false, Default::default()),
            (i64::MAX, raw, "hello", false, Default::default()),
        ];
        assert!(db.set_bodies(&bad).is_err());
        let again = db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE).unwrap();
        assert_eq!(again.len(), 2, "nothing was stored; the next pass retries");
        let good = [
            (missing[0].0, raw, "hello", false, Default::default()),
            (missing[1].0, raw, "hello", true, Default::default()),
        ];
        db.set_bodies(&good).unwrap();
        assert!(
            db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE)
                .unwrap()
                .is_empty()
        );
    }

    /// `KOMOREBI_PERF_DIR=<scratch> cargo test --release --locked -- --ignored
    /// --nocapture body_write_harness`: 300 bodies on a file cache (WAL),
    /// one transaction per body (old) against one per batch of 25 (new),
    /// interleaved, fresh cache each run.
    #[test]
    #[ignore]
    fn body_write_harness() {
        use std::time::Instant;
        let dir = std::path::PathBuf::from(std::env::var("KOMOREBI_PERF_DIR").unwrap());
        let mut text = String::new();
        for i in 0..60 {
            text.push_str(&format!(
                "Line {i} of a plausible message body, with words.
"
            ));
        }
        let raw = format!(
            "From: a@example.com
To: b@example.com
Subject: s
MIME-Version: 1.0
             Content-Type: text/plain; charset=utf-8

{text}"
        )
        .into_bytes();
        let prepare = |name: &str| {
            let path = dir.join(name);
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
            }
            let mut db = Db::open(&path).unwrap();
            let f = setup(&mut db, 7, &[]);
            add_sized(&mut db, &f, 1..=300, 3000);
            let missing = db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE).unwrap();
            assert_eq!(missing.len(), 300);
            (db, f, missing)
        };
        let (mut old, mut new) = (Vec::new(), Vec::new());
        for run in 0..9 {
            for variant in [run % 2, 1 - run % 2] {
                let (mut db, f, missing) = prepare(&format!("w{variant}.db"));
                let t = Instant::now();
                if variant == 0 {
                    for m in &missing {
                        let b = mime::parse_body(&raw);
                        db.set_body(
                            m.0,
                            &raw,
                            &b.preview(),
                            !b.attachments.is_empty(),
                            b.importance,
                        )
                        .unwrap();
                    }
                    old.push(t.elapsed().as_secs_f64() * 1e3);
                } else {
                    for chunk in missing.chunks(PREFETCH_BATCH) {
                        let parsed: Vec<_> = chunk
                            .iter()
                            .map(|m| {
                                let b = mime::parse_body(&raw);
                                (m.0, b.preview(), !b.attachments.is_empty(), b.importance)
                            })
                            .collect();
                        let items: Vec<_> = parsed
                            .iter()
                            .map(|(id, p, a, i)| (*id, raw.as_slice(), p.as_str(), *a, *i))
                            .collect();
                        db.set_bodies(&items).unwrap();
                    }
                    new.push(t.elapsed().as_secs_f64() * 1e3);
                }
                assert!(
                    db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE)
                        .unwrap()
                        .is_empty()
                );
            }
        }
        for (n, v) in [("per-body", &mut old), ("per-batch", &mut new)] {
            v.sort_by(|a, b| a.total_cmp(b));
            println!(
                "{n}: median {:.2} ms min {:.2} max {:.2} runs {}",
                v[v.len() / 2],
                v[0],
                v[v.len() - 1],
                v.len()
            );
        }
    }

    #[test]
    fn batches_split_at_count_bytes_and_budget() {
        let rows = |n: usize, size| vec![(0, 0, size); n];
        let p = PREFETCH;
        assert_eq!(next_batch(&rows(30, 100), &p, p.pass_budget).len(), 25);
        assert_eq!(next_batch(&rows(5, 1 << 20), &p, p.pass_budget).len(), 2);
        // One row over `batch_bytes` still goes, alone.
        let big = PrefetchPolicy {
            batch_bytes: 50,
            ..p
        };
        assert_eq!(next_batch(&rows(3, 100), &big, p.pass_budget).len(), 1);
        assert_eq!(next_batch(&rows(3, 100), &p, 250).len(), 2);
        assert!(next_batch(&rows(3, 100), &p, 99).is_empty());
    }

    #[test]
    fn prefetch_stops_at_the_byte_budget() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 2:3 (UID BODY.PEEK[])"),
            Step::Send(body_fetch([3, 2])),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        add_sized(&mut db, &f, 1..=3, 100);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let policy = PrefetchPolicy {
            pass_budget: 250,
            ..PREFETCH
        };
        assert_eq!(prefetch_bodies(&mut imap, &mut db, &f, &policy).unwrap(), 2);
        // The oldest didn't fit and stays missing for the next pass.
        let missing = db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE).unwrap();
        assert_eq!(missing.iter().map(|m| m.1).collect::<Vec<_>>(), [1]);
        imap.logout();
        server.finish();
    }

    #[test]
    fn sync_folder_preempts_prefetch_after_one_batch() {
        let (tx, rx) = channel();
        let early = tx.clone();
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        add_sized(&mut db, &f, 1..=26, 100);
        let folder_id = f.id;
        let flags: String = (1..=26)
            .map(|uid| format!("* {uid} FETCH (UID {uid} FLAGS ())\r\n"))
            .collect();
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 2:26 (UID BODY.PEEK[])"),
            // The user opens the inbox while the first batch is in flight.
            Step::Then(Box::new(move || {
                early
                    .send(Job::SyncFolder {
                        id: folder_id,
                        full: true,
                    })
                    .unwrap();
            })),
            Step::Send(body_fetch((2..=26).rev())),
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a6 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(leak(flags + "{tag} OK done\r\n")),
            Step::Expect("a7 UID FETCH 1 (UID BODY.PEEK[])"),
            // The account is removed: the worker stops and logs out.
            Step::Then(Box::new(move || drop(tx))),
            Step::Send(body_fetch([1])),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, _) = worker(&db, imap);
        w.prefetch = Some(Prefetch {
            folder_id,
            budget_left: PREFETCH.pass_budget,
        });

        w.serve(&mut db, &rx);
        assert!(w.imap.is_none());
        assert!(
            db.missing_bodies(f.id, 300, PREFETCH_MAX_SIZE)
                .unwrap()
                .is_empty()
        );
        server.finish();
    }

    #[test]
    fn duplicate_folder_syncs_run_once() {
        let (tx, rx) = channel();
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[1]);
        tx.send(Job::SyncFolder {
            id: f.id,
            full: true,
        })
        .unwrap();
        tx.send(Job::SyncFolder {
            id: f.id,
            full: true,
        })
        .unwrap();
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Then(Box::new(move || drop(tx))),
            Step::Send("* 1 FETCH (UID 1 FLAGS ())\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        w.serve(&mut db, &rx);
        assert!(events.lock().unwrap().is_empty());
        server.finish();
    }

    #[test]
    fn sync_all_absorbs_queued_syncs() {
        let (tx, rx) = channel();
        for job in [
            Job::SyncFolder { id: 1, full: true },
            Job::FetchBody(5),
            Job::SyncAll { full: true },
            Job::SyncAll { full: false },
        ] {
            tx.send(job).unwrap();
        }
        let mut backlog = VecDeque::new();
        // A queued full pass makes the one that runs full.
        let mut job = Job::SyncAll { full: false };
        assert!(coalesce(&mut job, &rx, &mut backlog));
        assert!(matches!(job, Job::SyncAll { full: true }));
        assert!(matches!(backlog.make_contiguous(), [Job::FetchBody(5)]));
        drop(tx);
        assert!(!coalesce(&mut job, &rx, &mut backlog));
    }

    #[test]
    fn full_pass_first_on_request_and_every_nth() {
        let mut since = None;
        assert!(full_pass(&mut since, false), "first pass");
        for _ in 1..FULL_PASS_EVERY {
            assert!(!full_pass(&mut since, false));
        }
        assert!(full_pass(&mut since, false), "FULL_PASS_EVERY-th pass");
        assert!(!full_pass(&mut since, false));
        // Refresh is full and restarts the count.
        assert!(full_pass(&mut since, true));
        for _ in 1..FULL_PASS_EVERY {
            assert!(!full_pass(&mut since, false));
        }
        assert!(full_pass(&mut since, false));
    }

    #[test]
    fn can_skip_needs_every_value_to_match() {
        let fp = Fp {
            uidnext: 12,
            messages: 3,
            hms: 100,
        };
        let st = FolderStatus {
            messages: 3,
            uidnext: 12,
            uidvalidity: 7,
            hms: 100,
        };
        let skip = |fp, st: FolderStatus, local| can_skip(fp, 7, Some(&st), local, true, false);
        assert!(skip(Some(fp), st, 3));
        // No STATUS line, or no complete sync yet.
        assert!(!can_skip(Some(fp), 7, None, 3, true, false));
        assert!(!skip(None, st, 3));
        // NOMODSEQ.
        let nomodseq = FolderStatus { hms: 0, ..st };
        assert!(!skip(Some(Fp { hms: 0, ..fp }), nomodseq, 3));
        // Each value differing.
        assert!(!skip(
            Some(fp),
            FolderStatus {
                uidvalidity: 8,
                ..st
            },
            3
        ));
        assert!(!skip(Some(fp), FolderStatus { uidnext: 13, ..st }, 3));
        assert!(!skip(Some(fp), FolderStatus { messages: 4, ..st }, 4));
        assert!(!skip(Some(fp), FolderStatus { hms: 101, ..st }, 3));
        // A stored modseq above the server's means a reset.
        assert!(!skip(Some(Fp { hms: 200, ..fp }), st, 3));
        // The cache lost a row (an optimistic delete) or holds an extra one.
        assert!(!skip(Some(fp), st, 2));
        assert!(!skip(Some(fp), st, 4));
        // An action in flight, or a forced pass.
        assert!(!can_skip(Some(fp), 7, Some(&st), 3, false, false));
        assert!(!can_skip(Some(fp), 7, Some(&st), 3, true, true));
    }

    const FP: Fp = Fp {
        uidnext: 12,
        messages: 3,
        hms: 100,
    };

    /// Adds folder `name` (UIDVALIDITY 7) holding `uids`, with fingerprint `fp`.
    fn add_folder(
        db: &mut Db,
        account_id: i64,
        name: &str,
        uids: &[u32],
        fp: Option<Fp>,
    ) -> Folder {
        let id = db
            .upsert_folder(account_id, name, name, 0, Role::Other)
            .unwrap();
        db.set_uidvalidity(id, 7).unwrap();
        let headers: Vec<_> = uids
            .iter()
            .map(|&uid| NewHeader {
                uid,
                ..Default::default()
            })
            .collect();
        db.insert_headers(account_id, id, &headers).unwrap();
        if let Some(fp) = fp {
            db.finish_folder(id, fp).unwrap();
        }
        db.folder(id).unwrap().unwrap()
    }

    /// One LIST line per folder, each followed by a STATUS line from
    /// `status` (none where it returns `None`), then the tagged OK.
    fn list_reply(names: &[&str], status: impl Fn(&str) -> Option<&'static str>) -> &'static str {
        let mut reply = String::new();
        for name in names {
            reply += &format!("* LIST (\\HasNoChildren) \"/\" \"{name}\"\r\n");
            if let Some(items) = status(name) {
                reply += &format!("* STATUS \"{name}\" ({items})\r\n");
            }
        }
        leak(reply + "{tag} OK LIST completed\r\n")
    }

    const UNCHANGED: &str = "MESSAGES 3 UIDNEXT 12 UIDVALIDITY 7 HIGHESTMODSEQ 100";

    #[test]
    fn unchanged_folders_are_skipped_in_one_command() {
        let names = ["INBOX", "Sent", "Drafts", "Trash", "Archive"];
        let mut db = Db::in_memory().unwrap();
        let account_id = setup(&mut db, 7, &[]).account_id;
        for name in names {
            add_folder(&mut db, account_id, name, &[1, 2, 3], Some(FP));
        }
        let mut script = login_steps(STATUS_CAPS);
        script.extend([
            Step::Expect(leak(format!("a3 {LIST_STATUS}"))),
            Step::Send(list_reply(&names, |_| Some(UNCHANGED))),
        ]);
        script.extend(logout_steps("a4 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let before = imap.commands();
        let stats = sync_pass_once(&mut imap, &mut db, account_id, false).unwrap();
        assert_eq!(
            stats,
            PassStats {
                folders: 5,
                skipped: 5
            }
        );
        assert_eq!(imap.commands() - before, 1);
        imap.logout();
        assert_eq!(server.finish().len(), 4);
    }

    #[test]
    fn refused_list_status_falls_back_to_list_and_status() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11]);
        let status = concat!(
            "* STATUS \"INBOX\" (MESSAGES 2 UIDNEXT 12 UIDVALIDITY 7 HIGHESTMODSEQ 100)\r\n",
            "{tag} OK STATUS completed\r\n",
        );
        let list = list_reply(&["INBOX"], |_| None);
        let status_cmd = "STATUS \"INBOX\" (MESSAGES UIDNEXT UIDVALIDITY HIGHESTMODSEQ)";
        let mut script = login_steps(STATUS_CAPS);
        script.extend([
            Step::Expect(leak(format!("a3 {LIST_STATUS}"))),
            Step::Send("{tag} BAD unknown LIST option\r\n"),
            // The stream is still in step: plain LIST, a STATUS, full path.
            Step::Expect(r#"a4 LIST "" *"#),
            Step::Send(list),
            Step::Expect(leak(format!("a5 {status_cmd}"))),
            Step::Send(status),
            Step::Expect(r#"a6 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a7 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(concat!(
                "* 1 FETCH (UID 10 FLAGS ())\r\n",
                "* 2 FETCH (UID 11 FLAGS ())\r\n",
                "{tag} OK done\r\n",
            )),
            // Next pass: no second LIST-STATUS on this connection, and the
            // fingerprint just recorded lets the inbox be skipped.
            Step::Expect(r#"a8 LIST "" *"#),
            Step::Send(list),
            Step::Expect(leak(format!("a9 {status_cmd}"))),
            Step::Send(status),
        ]);
        script.extend(logout_steps("a10 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let stats = sync_pass_once(&mut imap, &mut db, f.account_id, false).unwrap();
        assert_eq!(stats.skipped, 0);
        let fp = Fp {
            uidnext: 12,
            messages: 2,
            hms: 100,
        };
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp));
        let stats = sync_pass_once(&mut imap, &mut db, f.account_id, false).unwrap();
        assert_eq!(stats.skipped, 1);
        imap.logout();
        server.finish();
    }

    #[test]
    fn missing_status_or_nomodseq_takes_the_full_path() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[10, 11]);
        let old = Fp {
            uidnext: 12,
            messages: 2,
            hms: 50,
        };
        db.finish_folder(inbox.id, old).unwrap();
        let sent = add_folder(&mut db, inbox.account_id, "Sent", &[4], None);
        let mut script = login_steps(STATUS_CAPS);
        script.extend([
            Step::Expect(leak(format!("a3 {LIST_STATUS}"))),
            // No STATUS line for the inbox; the Sent folder is NOMODSEQ.
            Step::Send(list_reply(&["INBOX", "Sent"], |name| {
                (name == "Sent").then_some("MESSAGES 1 UIDNEXT 5 UIDVALIDITY 7 HIGHESTMODSEQ 0")
            })),
            Step::Expect(r#"a4 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a5 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(concat!(
                "* 1 FETCH (UID 10 FLAGS ())\r\n",
                "* 2 FETCH (UID 11 FLAGS ())\r\n",
                "{tag} OK done\r\n",
            )),
            Step::Expect(r#"a6 SELECT "Sent""#),
            Step::Send(SELECT_OK),
            Step::Expect("a7 UID FETCH 1:* (UID FLAGS)"),
            Step::Send("* 1 FETCH (UID 4 FLAGS ())\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let stats = sync_pass_once(&mut imap, &mut db, inbox.account_id, false).unwrap();
        assert_eq!(stats.skipped, 0);
        // Nothing to record without a usable STATUS.
        assert_eq!(db.fingerprint(inbox.id).unwrap(), Some(old));
        assert_eq!(db.fingerprint(sent.id).unwrap(), None);
        imap.logout();
        server.finish();
    }

    #[test]
    fn error_after_inserting_headers_keeps_the_old_fingerprint() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let old = Fp {
            uidnext: 1,
            messages: 0,
            hms: 50,
        };
        db.finish_folder(f.id, old).unwrap();
        // 501 new messages: the first header batch works, the second fails.
        let n = HEADER_BATCH as u32 + 1;
        let first = n - FIRST_HEADER_BATCH as u32 + 1;
        let flags: String = (1..=n)
            .map(|uid| format!("* {uid} FETCH (UID {uid} FLAGS ())\r\n"))
            .collect();
        let headers: String = (first..=n)
            .rev()
            .map(|uid| header_fetch(uid, uid, "", "Subject: x\r\n\r\n"))
            .collect();
        let status = format!(
            "MESSAGES {n} UIDNEXT {} UIDVALIDITY 7 HIGHESTMODSEQ 60",
            n + 1
        );
        let status: &'static str = leak(status);
        let mut script = login_steps(STATUS_CAPS);
        script.extend([
            Step::Expect(leak(format!("a3 {LIST_STATUS}"))),
            Step::Send(list_reply(&["INBOX"], |_| Some(status))),
            Step::Expect(r#"a4 SELECT "INBOX""#),
            // A tail longer than one header batch takes the full path.
            Step::Send(select_reply(n, n + 1)),
            Step::Expect("a5 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(leak(flags + "{tag} OK done\r\n")),
            Step::Expect(leak(format!("a6 UID FETCH {first}:{n} (UID FLAGS"))),
            Step::Send(leak(headers + "{tag} OK done\r\n")),
            // The ramp sizes the second batch from the clock.
            Step::Expect("a7 UID FETCH "),
            Step::Send("{tag} NO server busy\r\n"),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        assert!(sync_pass_once(&mut imap, &mut db, f.account_id, false).is_err());
        assert_eq!(db.count_messages(f.id).unwrap(), FIRST_HEADER_BATCH as u32);
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(old));
        imap.logout();
        server.finish();
    }

    #[test]
    fn header_batches_ramp_up_when_warm_and_stay_small_when_cold() {
        // A round trip plus a cost per message.
        let ramp = |rtt: u64, per_msg_us: u64| {
            let mut sizes = vec![FIRST_HEADER_BATCH];
            for _ in 0..5 {
                let size = *sizes.last().unwrap();
                let took =
                    Duration::from_millis(rtt) + Duration::from_micros(per_msg_us) * size as u32;
                sizes.push(next_header_batch(size, took));
            }
            sizes
        };
        // Warm: growth is capped at double, so 500 comes on the fourth step.
        assert_eq!(ramp(150, 500), [50, 100, 200, 400, 500, 500]);
        // Cold (87 ms per message): never above the floor.
        assert_eq!(ramp(150, 87_000), [50; 6]);
        // A slow batch shrinks the next one, down to the floor.
        assert_eq!(next_header_batch(500, Duration::from_secs(3)), 250);
        assert_eq!(next_header_batch(100, Duration::from_secs(60)), 50);
        assert_eq!(next_header_batch(50, Duration::ZERO), 100);
    }

    #[test]
    fn interrupted_header_ramp_leaves_no_gap() {
        let headers = |uids: std::ops::RangeInclusive<u32>| {
            let reply: String = uids
                .rev()
                .map(|uid| header_fetch(uid, uid, "", "Subject: x\r\n\r\n"))
                .collect();
            leak(reply + "{tag} OK done\r\n")
        };
        let flags: String = (1..=200)
            .map(|uid| format!("* {uid} FETCH (UID {uid} FLAGS ())\r\n"))
            .collect();
        let flags = leak(flags + "{tag} OK done\r\n");
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(200, 201)),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(flags),
            Step::Expect("a5 UID FETCH 151:200 (UID FLAGS"),
            Step::Send(headers(151..=200)),
            Step::Expect("a6 UID FETCH 51:150 (UID FLAGS"),
            Step::Send(headers(51..=150)),
            // The connection drops during the third batch.
            Step::Expect("a7 UID FETCH 1:50 (UID FLAGS"),
            Step::Close,
        ]);
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let events = Arc::new(Mutex::new(0));
        let sink = events.clone();
        let notify: Notify = Arc::new(move |e| {
            if matches!(e, Event::FolderChanged { .. }) {
                *sink.lock().unwrap() += 1;
            }
        });
        let pending = Pending::default();
        let mut ramp = FIRST_HEADER_BATCH;
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let r = sync_folder(
            &mut imap, &mut db, &f, &notify, &pending, None, true, &mut ramp,
        );
        assert!(r.is_err_and(|e| is_connection_error(&e)));
        server.finish();
        // Each stored batch was shown, and the retry starts at half the
        // failed batch's size.
        assert_eq!(*events.lock().unwrap(), 2);
        assert_eq!(db.count_messages(f.id).unwrap(), 150);
        assert_eq!(ramp, 100);

        // The next try fetches only what is missing.
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(200, 201)),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(flags),
            Step::Expect("a5 UID FETCH 1:50 (UID FLAGS"),
            Step::Send(headers(1..=50)),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let got = sync_folder(
            &mut imap, &mut db, &f, &notify, &pending, None, true, &mut ramp,
        );
        assert_eq!(got.unwrap().new, 50);
        assert_eq!(db.known_uids(f.id).unwrap(), (1..=200).collect());
        imap.logout();
        server.finish();
    }

    #[test]
    fn pending_action_keeps_the_fingerprint_unrecorded() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11]);
        let st = FolderStatus {
            messages: 2,
            uidnext: 12,
            uidvalidity: 7,
            hms: 100,
        };
        let flags = concat!(
            "* 1 FETCH (UID 10 FLAGS ())\r\n",
            "* 2 FETCH (UID 11 FLAGS ())\r\n",
            "{tag} OK done\r\n",
        );
        let mut script = login_steps(PLAIN_CAPS);
        for (select, fetch) in [("a3", "a4"), ("a5", "a6")] {
            script.extend([
                Step::Expect(leak(format!(r#"{select} SELECT "INBOX""#))),
                Step::Send(SELECT_OK),
                Step::Expect(leak(format!("{fetch} UID FETCH 1:* (UID FLAGS)"))),
                Step::Send(flags),
            ]);
        }
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let notify: Notify = Arc::new(|_| {});
        let mut ramp = FIRST_HEADER_BATCH;

        let pending = Pending::default();
        pending.add((f.id, 10));
        sync_folder(
            &mut imap,
            &mut db,
            &f,
            &notify,
            &pending,
            Some(&st),
            false,
            &mut ramp,
        )
        .unwrap();
        assert_eq!(db.fingerprint(f.id).unwrap(), None);
        pending.done((f.id, 10));
        sync_folder(
            &mut imap,
            &mut db,
            &f,
            &notify,
            &pending,
            Some(&st),
            false,
            &mut ramp,
        )
        .unwrap();
        let fp = Fp {
            uidnext: 12,
            messages: 2,
            hms: 100,
        };
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp));
        imap.logout();
        server.finish();
    }

    #[test]
    fn failed_move_clears_the_fingerprint_and_resync_restores_the_row() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        db.finish_folder(f.id, FP).unwrap();
        let archive = add_folder(&mut db, f.account_id, "Archive", &[], None);
        // The UI removes the row before the move runs.
        let id = db.message_ids(&[f.id], "").unwrap()[0];
        db.delete_message(id).unwrap();
        let header = header_fetch(1, 5, "", "Subject: back\r\n\r\n") + "{tag} OK done\r\n";
        let mut script = login_steps("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n");
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID MOVE 5"),
            Step::Send("{tag} NO [TRYCREATE] no such folder\r\n"),
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a6 UID FETCH 1:* (UID FLAGS)"),
            Step::Send("* 1 FETCH (UID 5 FLAGS ())\r\n{tag} OK done\r\n"),
            Step::Expect("a7 UID FETCH 5 (UID FLAGS INTERNALDATE"),
            Step::Send(leak(header)),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        let job = Job::Move {
            folder_id: f.id,
            uids: vec![5],
            dest: archive.id,
        };
        w.run_job(&mut db, job, &mut || None);
        assert_eq!(db.fingerprint(f.id).unwrap(), None);
        assert!(matches!(
            events.lock().unwrap()[..],
            [Event::ActionFailed { .. }, Event::NeedsSync { folder_id }] if folder_id == f.id
        ));
        w.run_job(
            &mut db,
            Job::SyncFolder {
                id: f.id,
                full: true,
            },
            &mut || None,
        );
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([5]));
        w.imap.take().unwrap().logout();
        server.finish();
    }

    fn fp(uidnext: u32, messages: u32, hms: u64) -> Fp {
        Fp {
            uidnext,
            messages,
            hms,
        }
    }

    /// The STATUS a pass read before the SELECT (UIDVALIDITY 7).
    fn status(messages: u32, uidnext: u32, hms: u64) -> FolderStatus {
        FolderStatus {
            messages,
            uidnext,
            uidvalidity: 7,
            hms,
        }
    }

    /// UID → seen of the folder's cached messages.
    fn seen(db: &Db, f: &Folder) -> HashMap<u32, bool> {
        db.message_ids(&[f.id], "")
            .unwrap()
            .into_iter()
            .map(|id| {
                let m = db.message(id).unwrap().unwrap();
                (m.uid, m.seen)
            })
            .collect()
    }

    const DELTA: FolderSync = FolderSync {
        delta: true,
        new: 0,
        flag_changes: 0,
    };

    #[test]
    fn can_delta_needs_the_server_to_carry_on_from_the_fingerprint() {
        let (fp, st) = (fp(12, 3, 100), status(3, 12, 100));
        let sel = |uid_next| imap::Selected {
            uidvalidity: 7,
            exists: 3,
            uid_next,
        };
        let batch = HEADER_BATCH as u32;
        assert!(can_delta(fp, &st, &sel(12), true));
        assert!(can_delta(fp, &st, &sel(12 + batch), true));
        // A tail longer than one batch, or UIDNEXT going back.
        assert!(!can_delta(fp, &st, &sel(13 + batch), true));
        assert!(!can_delta(fp, &st, &sel(11), true));
        // An empty folder.
        let empty = imap::Selected {
            exists: 0,
            ..sel(12)
        };
        assert!(!can_delta(fp, &st, &empty, true));
        // HIGHESTMODSEQ going back (a reset), or NOMODSEQ.
        assert!(!can_delta(fp, &status(3, 12, 99), &sel(12), true));
        assert!(!can_delta(fp, &status(3, 12, 0), &sel(12), true));
        let other = FolderStatus {
            uidvalidity: 8,
            ..st
        };
        assert!(!can_delta(fp, &other, &sel(12), true));
        // An action in flight.
        assert!(!can_delta(fp, &st, &sel(12), false));
    }

    #[test]
    fn delta_fetches_only_the_new_tail() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11]);
        db.finish_folder(f.id, fp(12, 2, 100)).unwrap();
        let header = leak(header_fetch(3, 12, "", "Subject: new\r\n\r\n") + "{tag} OK done\r\n");
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(3, 13)),
            Step::Expect("a4 UID FETCH 12:* (UID FLAGS INTERNALDATE"),
            Step::Send(header),
            // UID 13 came and went: `13:*` still returns the highest, 12.
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(select_reply(3, 14)),
            Step::Expect("a6 UID FETCH 13:* (UID FLAGS INTERNALDATE"),
            Step::Send(header),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let before = imap.commands();
        let got = sync_folder_with_status(&mut imap, &mut db, &f, &status(3, 13, 100)).unwrap();
        assert_eq!(got, FolderSync { new: 1, ..DELTA });
        assert_eq!(imap.commands() - before, 2);
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([10, 11, 12]));
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(13, 3, 100)));
        let got = sync_folder_with_status(&mut imap, &mut db, &f, &status(3, 14, 100)).unwrap();
        assert_eq!(got, DELTA);
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(14, 3, 100)));
        imap.logout();
        server.finish();
    }

    #[test]
    fn flag_delta_applies_only_the_listed_uids() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11, 12]);
        db.finish_folder(f.id, fp(13, 3, 100)).unwrap();
        // Read here: the server doesn't list it, so the delta leaves it be.
        let id = db.message_ids(&[f.id], "").unwrap()[2];
        db.set_seen(id, true).unwrap();
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(3, 13)),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 100)"),
            Step::Send("* 2 FETCH (UID 11 FLAGS (\\Seen) MODSEQ (150))\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let before = imap.commands();
        let got = sync_folder_with_status(&mut imap, &mut db, &f, &status(3, 13, 150)).unwrap();
        assert_eq!(
            got,
            FolderSync {
                flag_changes: 1,
                ..DELTA
            }
        );
        assert_eq!(imap.commands() - before, 2);
        assert_eq!(
            seen(&db, &f),
            HashMap::from([(10, true), (11, true), (12, false)])
        );
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(13, 3, 150)));
        imap.logout();
        server.finish();
    }

    #[test]
    fn delta_hides_newly_deleted_messages() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11, 12]);
        db.finish_folder(f.id, fp(13, 3, 100)).unwrap();
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(3, 13)),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 100)"),
            Step::Send("* 3 FETCH (UID 12 FLAGS (\\Deleted) MODSEQ (150))\r\n{tag} OK done\r\n"),
            // The hidden message makes the counts differ: check it is the
            // only one the cache lacks, and still flagged.
            Step::Expect("a5 UID SEARCH ALL"),
            Step::Send("* SEARCH 10 11 12\r\n{tag} OK done\r\n"),
            Step::Expect("a6 UID FETCH 12 (UID FLAGS)"),
            Step::Send("* 3 FETCH (UID 12 FLAGS (\\Deleted))\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let got = sync_folder_with_status(&mut imap, &mut db, &f, &status(3, 13, 150)).unwrap();
        assert!(got.delta);
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([10, 11]));
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(13, 3, 150)));
        imap.logout();
        server.finish();
    }

    #[test]
    fn busy_uid_in_the_flag_delta_keeps_the_old_fingerprint() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11]);
        db.finish_folder(f.id, fp(12, 2, 100)).unwrap();
        let pending = Pending::default();
        let (busy, folder_id) = (pending.clone(), f.id);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(2, 12)),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 100)"),
            // The user marks UID 11 unread while the fetch is in flight.
            Step::Then(Box::new(move || busy.add((folder_id, 11)))),
            Step::Send("* 2 FETCH (UID 11 FLAGS (\\Seen) MODSEQ (150))\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let notify: Notify = Arc::new(|_| {});
        let mut ramp = FIRST_HEADER_BATCH;

        let st = status(2, 12, 150);
        let got = sync_folder(
            &mut imap,
            &mut db,
            &f,
            &notify,
            &pending,
            Some(&st),
            false,
            &mut ramp,
        );
        assert!(got.unwrap().delta);
        assert_eq!(seen(&db, &f), HashMap::from([(10, false), (11, false)]));
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(12, 2, 100)));
        imap.logout();
        server.finish();
    }

    #[test]
    fn count_mismatch_deletes_exactly_what_search_lacks() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11, 12]);
        db.finish_folder(f.id, fp(13, 3, 100)).unwrap();
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            // An expunge that left HIGHESTMODSEQ alone (CONDSTORE only).
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(select_reply(2, 13)),
            Step::Expect("a4 UID SEARCH ALL"),
            Step::Send("* SEARCH 10 12\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        let before = imap.commands();
        let got = sync_folder_with_status(&mut imap, &mut db, &f, &status(2, 13, 100)).unwrap();
        assert_eq!(got, DELTA);
        assert_eq!(imap.commands() - before, 2);
        assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([10, 12]));
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(13, 2, 100)));
        imap.logout();
        server.finish();
    }

    #[test]
    fn search_that_disagrees_with_exists_deletes_nothing() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11, 12]);
        db.finish_folder(f.id, fp(13, 3, 100)).unwrap();
        let mut script = login_steps(PLAIN_CAPS);
        // Three UIDs for two messages, then none for two: the delta deletes
        // nothing and falls back to the full path, which fails here.
        for (tag, search) in [(3, "* SEARCH 10 11 12\r\n"), (6, "* SEARCH\r\n")] {
            script.extend([
                Step::Expect(leak(format!(r#"a{tag} SELECT "INBOX""#))),
                Step::Send(select_reply(2, 13)),
                Step::Expect(leak(format!("a{} UID SEARCH ALL", tag + 1))),
                Step::Send(leak(format!("{search}{{tag}} OK done\r\n"))),
                Step::Expect(leak(format!("a{} UID FETCH 1:* (UID FLAGS)", tag + 2))),
                Step::Send("{tag} NO server busy\r\n"),
            ]);
        }
        script.extend(logout_steps("a9 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();

        for _ in 0..2 {
            assert!(sync_folder_with_status(&mut imap, &mut db, &f, &status(2, 13, 100)).is_err());
            assert_eq!(db.known_uids(f.id).unwrap(), HashSet::from([10, 11, 12]));
            assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(13, 3, 100)));
        }
        imap.logout();
        server.finish();
    }

    #[test]
    fn uidnext_regression_or_a_forced_pass_takes_the_full_path() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[10, 11, 12]);
        db.finish_folder(f.id, fp(13, 3, 100)).unwrap();
        let flags = concat!(
            "* 1 FETCH (UID 10 FLAGS ())\r\n",
            "* 2 FETCH (UID 11 FLAGS ())\r\n",
            "* 3 FETCH (UID 12 FLAGS ())\r\n",
            "{tag} OK done\r\n",
        );
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            // UIDNEXT 12 is below the fingerprint's 13.
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(flags),
            // A tail the delta could take, but the pass is forced.
            Step::Expect(r#"a5 SELECT "INBOX""#),
            Step::Send(select_reply(3, 13)),
            Step::Expect("a6 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(flags),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let notify: Notify = Arc::new(|_| {});
        let mut ramp = FIRST_HEADER_BATCH;
        let pending = Pending::default();

        let got = sync_folder_with_status(&mut imap, &mut db, &f, &status(3, 12, 100)).unwrap();
        assert!(!got.delta);
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(fp(12, 3, 100)));
        let st = status(3, 13, 100);
        let got = sync_folder(
            &mut imap,
            &mut db,
            &f,
            &notify,
            &pending,
            Some(&st),
            true,
            &mut ramp,
        );
        assert!(!got.unwrap().delta);
        imap.logout();
        server.finish();
    }

    /// Make `w` connect to whatever address `addr` holds; returns the
    /// number of connection attempts so far.
    fn connect_to(w: &mut Worker, addr: &Arc<Mutex<std::net::SocketAddr>>) -> Arc<Mutex<usize>> {
        let attempts = Arc::new(Mutex::new(0));
        let (n, addr) = (attempts.clone(), addr.clone());
        w.connect = Box::new(move |_| {
            *n.lock().unwrap() += 1;
            Imap::connect_plain(*addr.lock().unwrap(), "user", "pass")
        });
        attempts
    }

    fn login_refused(reply: &'static str) -> FakeServer {
        FakeServer::start(vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect("a1 LOGIN"),
            Step::Send(reply),
        ])
    }

    fn sync_errors(events: &Mutex<Vec<Event>>) -> Vec<Problem> {
        events
            .lock()
            .unwrap()
            .drain(..)
            .filter_map(|e| match e {
                Event::Syncing { error, .. } => error,
                _ => None,
            })
            .collect()
    }

    #[test]
    fn address_backfill_runs_after_the_first_pass() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let header = |uid| NewHeader {
            uid,
            from_name: "Ana".into(),
            from_addr: "ana@example.org".into(),
            ..Default::default()
        };
        let headers: Vec<_> = (1..=db::BACKFILL_CHUNK as u32 + 1).map(header).collect();
        db.insert_headers(f.account_id, f.id, &headers).unwrap();
        db.rebuild_addresses(f.account_id).unwrap();
        let (mut w, _events) = worker_with(&db, None);
        w.backfill = Backfill::AfterFirstPass;
        // Nothing before the first pass (it fails here: no server).
        w.idle_step(&mut db);
        assert!(
            db.suggest_addresses(f.account_id, "ana", 6)
                .unwrap()
                .is_empty()
        );
        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        assert_eq!(w.backfill, Backfill::Running);
        // One chunk per idle step.
        w.idle_step(&mut db);
        assert_eq!(w.backfill, Backfill::Running);
        w.idle_step(&mut db);
        assert_eq!(w.backfill, Backfill::Done);
        let found = db.suggest_addresses(f.account_id, "ana", 6).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].addr, "ana@example.org");
    }

    #[test]
    fn rejected_sign_in_latches_until_cleared() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let server = login_refused("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n");
        let addr = Arc::new(Mutex::new(server.addr));
        let (mut w, events) = worker_with(&db, None);
        let attempts = connect_to(&mut w, &addr);

        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        server.finish();
        assert!(w.auth_failed.get());
        let errors = sync_errors(&events);
        assert!(
            matches!(&errors[..], [e] if e.class == Class::Auth && e.detail.contains("Invalid credentials")),
            "{errors:?}"
        );

        // Latched: no connection for any job. A user action still says
        // why; timer passes stay quiet, as this one already reported it.
        w.run_job(
            &mut db,
            Job::SyncFolder {
                id: f.id,
                full: true,
            },
            &mut || None,
        );
        w.run_job(&mut db, Job::Warm, &mut || None);
        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        assert!(matches!(
            &events.lock().unwrap()[..],
            [Event::ActionFailed { error, .. }]
                if error.class == Class::Auth && error.detail.starts_with("Sign-in rejected.")
        ));
        assert!(sync_errors(&events).is_empty());
        // Latched by the other worker: the sync worker reports it once.
        w.latch_reported = false;
        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        let errors = sync_errors(&events);
        assert!(
            matches!(&errors[..], [e] if e.class == Class::Auth && e.say() == Some(Say::Class(Class::Auth))),
            "{errors:?}"
        );
        assert_eq!(*attempts.lock().unwrap(), 1);

        // Cleared (new password or Refresh): the next job signs in again.
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(logout_steps("a3 LOGOUT"));
        let server = FakeServer::start(script);
        *addr.lock().unwrap() = server.addr;
        w.auth_failed.clear();
        w.run_job(&mut db, Job::Warm, &mut || None);
        assert_eq!(*attempts.lock().unwrap(), 2);
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn a_new_password_clears_the_latch() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let refused = FakeServer::start(vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect(r#"a1 LOGIN "user" "old""#),
            Step::Send("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"),
        ]);
        // The saved password, as the edit form replaces it.
        let password = Arc::new(Mutex::new("old".to_string()));
        let addr = Arc::new(Mutex::new(refused.addr));
        let (mut w, events) = worker_with(&db, None);
        let (pw, at) = (password.clone(), addr.clone());
        w.connect = Box::new(move |_| {
            Imap::connect_plain(*at.lock().unwrap(), "user", &pw.lock().unwrap())
        });

        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        refused.finish();
        assert!(w.auth_failed.get());
        let errors = sync_errors(&events);
        assert!(
            matches!(&errors[..], [e] if e.class == Class::Auth && e.say() == Some(Say::Class(Class::Auth))),
            "{errors:?}"
        );

        // Saved from Settings: the new password, then the latch cleared
        // (`Workers::credentials_changed`). The next job signs in with it.
        let mut script = vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect(r#"a1 LOGIN "user" "new""#),
            Step::Send("{tag} OK LOGIN completed\r\n"),
            Step::Expect("a2 CAPABILITY"),
            Step::Send(PLAIN_CAPS),
        ];
        script.extend(logout_steps("a3 LOGOUT"));
        let accepted = FakeServer::start(script);
        *addr.lock().unwrap() = accepted.addr;
        *password.lock().unwrap() = "new".into();
        w.auth_failed.clear();
        w.run_job(&mut db, Job::Warm, &mut || None);
        assert!(!w.auth_failed.get());
        assert!(events.lock().unwrap().is_empty());
        w.imap.take().expect("signed in").logout();
        accepted.finish();
    }

    #[test]
    fn unavailable_sign_in_does_not_latch() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let server = login_refused("{tag} NO [UNAVAILABLE] Try again later\r\n");
        let addr = Arc::new(Mutex::new(server.addr));
        let (mut w, events) = worker_with(&db, None);
        let attempts = connect_to(&mut w, &addr);

        w.run_job(
            &mut db,
            Job::SyncFolder {
                id: f.id,
                full: true,
            },
            &mut || None,
        );
        server.finish();
        assert!(!w.auth_failed.get());
        assert!(matches!(
            &events.lock().unwrap()[..],
            [Event::ActionFailed { error, .. }]
                if error.class == Class::TooManyConnections && error.detail.contains("UNAVAILABLE")
        ));
        // The next job tries again (the server is gone by now).
        w.run_job(
            &mut db,
            Job::SyncFolder {
                id: f.id,
                full: true,
            },
            &mut || None,
        );
        assert_eq!(*attempts.lock().unwrap(), 2);
    }

    #[test]
    fn connection_failure_ends_the_pass() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        db.upsert_folder(f.account_id, "Archive", "Archive", 0, Role::Archive)
            .unwrap();
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 LIST "" *"#),
            Step::Send(list_reply(&["INBOX", "Archive"], |_| None)),
            Step::Expect(r#"a4 SELECT "INBOX""#),
            Step::Close,
        ]);
        let server = FakeServer::start(script);
        let addr = Arc::new(Mutex::new(server.addr));
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);
        let attempts = connect_to(&mut w, &addr);

        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        // One reconnect (refused), then no SELECT of Archive.
        assert_eq!(*attempts.lock().unwrap(), 1);
        assert!(server.finish().iter().all(|l| !l.contains("Archive")));
        let errors = sync_errors(&events);
        assert!(
            matches!(&errors[..], [e] if e.class == Class::Offline && e.detail.to_lowercase().starts_with("inbox: ") && !e.detail.contains("Archive")),
            "{errors:?}"
        );
        assert!(w.prefetch.is_none());
    }

    #[test]
    fn stale_connection_is_probed_and_replaced() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([Step::Expect("a3 NOOP"), Step::Close]);
        let old = FakeServer::start(script);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(logout_steps("a3 LOGOUT"));
        let new = FakeServer::start(script);
        let mut imap = Imap::connect_plain(old.addr, "user", "pass").unwrap();
        imap.set_last_used(std::time::SystemTime::UNIX_EPOCH);
        let (mut w, _) = worker(&db, imap);
        let attempts = connect_to(&mut w, &Arc::new(Mutex::new(new.addr)));

        w.run_job(&mut db, Job::Warm, &mut || None);
        old.finish();
        assert_eq!(*attempts.lock().unwrap(), 1);
        w.imap.take().unwrap().logout();
        new.finish();
    }

    #[test]
    fn idle_wake_reads_status_and_skips_an_unchanged_inbox() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[1, 2, 3]);
        db.finish_folder(f.id, FP).unwrap();
        let mut script = login_steps(STATUS_CAPS);
        script.extend([
            Step::Expect(r#"a3 STATUS "INBOX" (MESSAGES UIDNEXT UIDVALIDITY HIGHESTMODSEQ)"#),
            Step::Send(leak(format!(
                "* STATUS \"INBOX\" ({UNCHANGED})\r\n{{tag}} OK done\r\n"
            ))),
        ]);
        script.extend(logout_steps("a4 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        let job = Job::SyncFolder {
            id: f.id,
            full: false,
        };
        w.run_job(&mut db, job, &mut || None);
        assert!(events.lock().unwrap().is_empty());
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn folder_syncs_coalesce_to_full_if_any_was() {
        let (tx, rx) = channel();
        tx.send(Job::SyncFolder { id: 1, full: true }).unwrap();
        tx.send(Job::SyncFolder { id: 2, full: false }).unwrap();
        let mut backlog = VecDeque::new();
        let mut job = Job::SyncFolder { id: 1, full: false };
        assert!(coalesce(&mut job, &rx, &mut backlog));
        assert!(matches!(job, Job::SyncFolder { id: 1, full: true }));
        assert!(matches!(
            backlog.make_contiguous(),
            [Job::SyncFolder { id: 2, full: false }]
        ));
    }

    /// A message to you@example.com, built and queued in the outbox as Send
    /// leaves it when its undo window ends; returns its row.
    pub(super) fn queued(db: &Db, account_id: i64, body: &str) -> i64 {
        let raw = smtp::build_raw(smtp::Outgoing {
            from_email: "me@example.com".into(),
            to: "you@example.com".into(),
            subject: "Hi".into(),
            body: body.into(),
            message_id: "<m1@example.com>".into(),
            ..Default::default()
        })
        .unwrap();
        let row = db
            .insert_draft(&db::DraftRow {
                account_id,
                to: "you@example.com".into(),
                subject: "Hi".into(),
                body: body.into(),
                message_id: "<m1@example.com>".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(db.outbox_send(row).unwrap());
        assert!(db.store_raw(row, &raw, &AtomicBool::new(false)).unwrap());
        assert!(db.outbox_queue(row).unwrap());
        row
    }

    /// Run `Job::Send { row }` with SMTP on local `port`; returns the events
    /// and how many times SMTP was reached.
    fn run_send(db: &mut Db, mut w: Worker, row: i64, port: u16) -> (Vec<Event>, usize) {
        let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = tries.clone();
        w.smtp = Box::new(move |_, env, raw| {
            count.fetch_add(1, Ordering::SeqCst);
            smtp::send_plain(port, env, raw)
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        w.notify = Arc::new(move |e| sink.lock().unwrap().push(e));
        w.run_job(db, Job::Send { row }, &mut || None);
        if let Some(imap) = w.imap.take() {
            imap.logout();
        }
        let events = events.lock().unwrap().clone();
        (events, tries.load(Ordering::SeqCst))
    }

    /// Like [`run_send`], through a fake SMTP server ending as `end`; also
    /// returns the DATA it got.
    fn send_through(end: SmtpEnd, db: &mut Db, w: Worker, row: i64) -> (Vec<Event>, String) {
        let server = FakeSmtp::start(end);
        let (events, tries) = run_send(db, w, row, server.port);
        assert_eq!(tries, 1);
        let (_, data) = server.finish();
        (events, data)
    }

    /// A port nothing listens on: connecting fails before any SMTP.
    fn closed_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn outbox_state(db: &Db, row: i64) -> Option<(db::Outbox, Option<i64>)> {
        db.outbox_entry(row).unwrap().map(|e| (e.state, e.next_try))
    }

    #[test]
    fn retry_waits_double_and_stop() {
        let waits: Vec<_> = (1..=7).map(retry_delay).collect();
        assert_eq!(
            waits,
            [
                Some(30),
                Some(60),
                Some(120),
                Some(240),
                Some(480),
                None,
                None
            ]
        );
        assert_eq!(retry_delay(0), None);
        let busy = || Outcome::Retry("busy".into());
        assert_eq!(
            settle(busy(), 1, 1_000),
            (db::Outbox::Queued, Some(1_030), busy())
        );
        assert_eq!(
            settle(busy(), MAX_SEND_ATTEMPTS, 1_000),
            (db::Outbox::Failed, None, Outcome::Failed("busy".into()))
        );
        let (state, at, _) = settle(Outcome::Unconfirmed("cut".into()), 1, 1_000);
        assert_eq!((state, at), (db::Outbox::Unconfirmed, None));
        let (state, at, _) = settle(Outcome::Failed("550".into()), 1, 1_000);
        assert_eq!((state, at), (db::Outbox::Failed, None));
    }

    #[test]
    fn unreachable_server_is_retried_with_backoff_then_fails() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = queued(&db, inbox.account_id, "Hello");
        let before = chrono::Utc::now().timestamp();
        let (w, _) = worker_with(&db, None);
        let (events, tries) = run_send(&mut db, w, row, closed_port());
        let after = chrono::Utc::now().timestamp();
        assert_eq!(tries, 1);
        assert!(
            matches!(&events[..], [Event::Sent { row: r, outcome: Outcome::Retry(_), .. }] if *r == row),
            "{events:?}"
        );
        let (state, at) = outbox_state(&db, row).unwrap();
        assert_eq!(state, db::Outbox::Queued);
        let at = at.unwrap();
        assert!((before + 30..=after + 30).contains(&at), "{at}");
        assert!(db.outbox_due(at - 1).unwrap().is_empty());
        // Each try waits longer (temporary refusals from here on, which
        // come back faster than a refused connect); the sixth failure stops
        // the retries.
        for attempt in 2..=MAX_SEND_ATTEMPTS {
            assert_eq!(db.outbox_due(i64::MAX).unwrap(), [(row, inbox.account_id)]);
            let (w, _) = worker_with(&db, None);
            let (events, _) = send_through(SmtpEnd::TempRejectRcpt, &mut db, w, row);
            let last = attempt == MAX_SEND_ATTEMPTS;
            let [Event::Sent { outcome, .. }] = &events[..] else {
                panic!("{events:?}");
            };
            assert_eq!(matches!(outcome, Outcome::Failed(_)), last, "{outcome:?}");
        }
        let e = db.outbox_entry(row).unwrap().unwrap();
        assert_eq!((e.state, e.next_try), (db::Outbox::Failed, None));
        assert!(!e.last_error.is_empty());
        assert!(db.outbox_due(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn temporary_refusal_is_retried_with_backoff() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = queued(&db, inbox.account_id, "Hello");
        let (w, _) = worker_with(&db, None);
        let (events, data) = send_through(SmtpEnd::TempRejectRcpt, &mut db, w, row);
        assert!(data.is_empty());
        assert!(
            matches!(&events[..], [Event::Sent { outcome: Outcome::Retry(e), .. }] if e.contains("450")),
            "{events:?}"
        );
        let (state, at) = outbox_state(&db, row).unwrap();
        assert_eq!(state, db::Outbox::Queued);
        assert!(at.is_some());
    }

    #[test]
    fn refused_recipient_fails_and_is_not_retried() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = queued(&db, inbox.account_id, "Hello");
        let (w, _) = worker_with(&db, None);
        let (events, data) = send_through(SmtpEnd::RejectRcpt, &mut db, w, row);
        assert!(data.is_empty());
        assert!(
            matches!(&events[..], [Event::Sent { outcome: Outcome::Failed(e), .. }] if e.contains("550")),
            "{events:?}"
        );
        let e = db.outbox_entry(row).unwrap().unwrap();
        assert_eq!(e.state, db::Outbox::Failed);
        assert!(e.last_error.contains("550"), "{}", e.last_error);
        assert!(db.outbox_due(i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn drop_after_data_is_unconfirmed_and_never_sent_again() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = queued(&db, inbox.account_id, "Hello");
        let (w, _) = worker_with(&db, None);
        let (events, data) = send_through(SmtpEnd::DropAfterData, &mut db, w, row);
        assert!(data.contains("Subject: Hi"));
        let [Event::Sent { outcome, .. }] = &events[..] else {
            panic!("{events:?}");
        };
        assert!(
            matches!(outcome, Outcome::Unconfirmed(e) if e.contains(UNCONFIRMED)),
            "{outcome:?}"
        );
        assert!(!outcome.delivered());
        assert_eq!(
            outbox_state(&db, row),
            Some((db::Outbox::Unconfirmed, None))
        );
        // No pass and no stray Send job ever reaches SMTP again.
        assert!(db.outbox_due(i64::MAX).unwrap().is_empty());
        let (w, _) = worker_with(&db, None);
        let (events, tries) = run_send(&mut db, w, row, closed_port());
        assert_eq!((events.len(), tries), (0, 0), "no second attempt");
        assert_eq!(
            outbox_state(&db, row),
            Some((db::Outbox::Unconfirmed, None))
        );
    }

    #[test]
    fn a_send_cut_off_by_a_restart_is_not_sent_again() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = queued(&db, inbox.account_id, "Hello");
        // A send took it, then the app stopped.
        db.claim_outbox(row).unwrap().unwrap();
        assert_eq!(db.recover_outbox(INTERRUPTED).unwrap(), 1);
        assert_eq!(
            outbox_state(&db, row),
            Some((db::Outbox::Unconfirmed, None))
        );
        let (w, _) = worker_with(&db, None);
        let (events, tries) = run_send(&mut db, w, row, closed_port());
        assert_eq!((events.len(), tries), (0, 0));
        assert_eq!(
            db.outbox_entry(row).unwrap().unwrap().last_error,
            INTERRUPTED
        );
    }

    #[test]
    fn sent_copy_is_the_bytes_sent_over_smtp() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 APPEND "Sent""#),
            Step::Send("+ go ahead\r\n"),
            Step::Literal,
            Step::Send("{tag} OK APPEND completed\r\n"),
        ]);
        script.extend(logout_steps("a4 LOGOUT"));
        let imap = FakeServer::start(script);
        let addr = imap.addr;
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let sent = db
            .upsert_folder(inbox.account_id, "Sent", "Sent", 0, Role::Sent)
            .unwrap();
        let row = queued(&db, inbox.account_id, "First line\nSecond line with Zoë\n");
        let (mut w, _) = worker_with(&db, None);
        w.account.save_sent = true;
        w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
        let (events, data) = send_through(SmtpEnd::Accept, &mut db, w, row);
        let lines = imap.finish();
        let copy = lines
            .iter()
            .find(|l| l.contains("Subject: Hi"))
            .expect("APPEND literal");
        // DATA ends with the CRLF that opens the <CRLF>.<CRLF> terminator.
        assert_eq!(
            Some(copy.as_str()),
            data.strip_suffix("\r\n"),
            "the Sent copy is the SMTP DATA, byte for byte"
        );
        assert!(
            matches!(&events[..], [Event::NeedsSync { folder_id }, Event::Sent { outcome: Outcome::Delivered, .. }] if *folder_id == sent),
            "{events:?}"
        );
        // Delivered: the row is gone.
        assert!(db.draft(row).unwrap().is_none());
        assert_eq!(db.count_outbox().unwrap(), 0);
    }

    #[test]
    fn invite_reply_goes_only_to_the_organizer() {
        let mut db = Db::in_memory().unwrap();
        let account_id = setup(&mut db, 7, &[]).account_id;
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:plan-7@x.org\r\n\
                   SEQUENCE:3\r\nDTSTART:20261008T130000Z\r\nSUMMARY:Planning\r\n\
                   ORGANIZER;CN=Jane:mailto:jane@x.org\r\nATTENDEE;RSVP=TRUE:mailto:me@example.com\r\n\
                   ATTENDEE;RSVP=TRUE:mailto:bob@x.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let ev = ical::parse(ics.as_bytes()).unwrap();
        let me = ("Me", "me@example.com");
        let now = chrono::Utc::now();
        let out = ical::reply_message(&ev, "Me", "me@example.com", me, ical::Answer::Accepted, now)
            .unwrap();
        let draft = db::DraftRow {
            account_id,
            to: out.to.clone(),
            subject: out.subject.clone(),
            body: out.body.clone(),
            message_id: out.message_id.clone(),
            ..Default::default()
        };
        let row = db
            .queue_reply(&draft, &smtp::build_raw(out).unwrap())
            .unwrap();
        let (w, _) = worker_with(&db, None);
        let server = FakeSmtp::start(SmtpEnd::Accept);
        let (events, tries) = run_send(&mut db, w, row, server.port);
        let (_, data, rcpt) = server.finish_with_rcpt();
        assert_eq!(tries, 1);
        assert!(
            matches!(
                &events[..],
                [Event::Sent {
                    outcome: Outcome::Delivered,
                    ..
                }]
            ),
            "{events:?}"
        );
        // One recipient: the organizer, never the other attendees.
        assert_eq!(rcpt.len(), 1, "{rcpt:?}");
        assert!(rcpt[0].contains("<jane@x.org>"), "{rcpt:?}");
        let msg = mail_parser::MessageParser::default()
            .parse(data.as_bytes())
            .unwrap();
        assert_eq!(msg.subject(), Some("Accepted: Planning"));
        assert!(msg.cc().is_none());
        let part = msg
            .parts
            .iter()
            .find(|p| {
                use mail_parser::MimeHeaders;
                p.content_type().is_some_and(|ct| {
                    ct.subtype() == Some("calendar") && ct.attribute("method") == Some("REPLY")
                })
            })
            .expect("a text/calendar; method=REPLY part");
        let text = std::str::from_utf8(part.contents()).unwrap();
        assert!(text.split("\r\n").all(|l| l.len() <= 75), "{text}");
        // It reads back: the same UID and SEQUENCE, one attendee, the answer.
        let back = ical::parse(part.contents()).unwrap();
        assert_eq!(back.method, "REPLY");
        assert_eq!((back.uid.as_str(), back.sequence), ("plan-7@x.org", 3));
        assert_eq!(back.attendees.len(), 1);
        assert_eq!(back.attendees[0].addr, "me@example.com");
        assert_eq!(back.attendees[0].partstat, "ACCEPTED");
        assert_eq!(back.organizer.unwrap().addr, "jane@x.org");
        assert!(db.draft(row).unwrap().is_none(), "never sent again");
    }

    #[test]
    fn failed_sent_copy_still_counts_as_delivered() {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 APPEND "Sent""#),
            Step::Send("{tag} NO [OVERQUOTA] mailbox is full\r\n"),
        ]);
        script.extend(logout_steps("a4 LOGOUT"));
        let imap = FakeServer::start(script);
        let addr = imap.addr;
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        db.upsert_folder(inbox.account_id, "Sent", "Sent", 0, Role::Sent)
            .unwrap();
        let row = queued(&db, inbox.account_id, "Hello");
        let (mut w, _) = worker_with(&db, None);
        w.account.save_sent = true;
        w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
        let (events, data) = send_through(SmtpEnd::Accept, &mut db, w, row);
        imap.finish();
        assert!(data.contains("Subject: Hi"));
        // No NeedsSync for Sent: nothing was saved there.
        let [Event::Sent { outcome, .. }] = &events[..] else {
            panic!("{events:?}");
        };
        assert!(
            matches!(outcome, Outcome::DeliveredNotSaved(e) if e.contains("saving a copy to Sent")),
            "{outcome:?}"
        );
        assert!(outcome.delivered());
        assert!(db.draft(row).unwrap().is_none(), "never sent again");
    }

    const UIDPLUS_CAPS: &str = "* CAPABILITY IMAP4rev1 UIDPLUS\r\n{tag} OK done\r\n";

    /// An account with an INBOX and a Drafts folder (UIDVALIDITY 7) and a
    /// saved draft; returns the Drafts folder and the draft's row.
    fn draft_setup(db: &mut Db) -> (Folder, i64) {
        let account_id = setup(db, 7, &[]).account_id;
        let id = db
            .upsert_folder(account_id, "Drafts", "Drafts", 0, Role::Drafts)
            .unwrap();
        db.set_uidvalidity(id, 7).unwrap();
        let row = db
            .insert_draft(&db::DraftRow {
                account_id,
                to: "you@example.com, half@".into(),
                bcc: "hidden@example.com".into(),
                subject: "Plans".into(),
                body: "Draft text".into(),
                message_id: "<d1@example.com>".into(),
                ..Default::default()
            })
            .unwrap();
        (db.folder(id).unwrap().unwrap(), row)
    }

    /// Record `uid` as the server copy of `row` without marking the draft
    /// as copied (it still has changes to upload).
    fn old_copy(db: &Db, drafts: &Folder, row: i64, uid: u32) {
        let copy = db::ServerCopy {
            account_id: drafts.account_id,
            folder_id: drafts.id,
            uidvalidity: 7,
            uid,
        };
        let recorded = db.record_server_copy(row, 0, Some(&copy)).unwrap();
        assert_eq!(recorded, db::Recorded::Replaced(None));
    }

    fn append_steps(line: &'static str, reply: &'static str) -> [Step; 4] {
        [
            Step::Expect(line),
            Step::Send("+ go ahead\r\n"),
            Step::Literal,
            Step::Send(reply),
        ]
    }

    /// After the APPEND (a3): the fresh SELECT of Drafts and the search for
    /// the draft's Message-ID, answered with `found`.
    fn find_steps(found: &'static str) -> [Step; 4] {
        [
            Step::Expect(r#"a4 SELECT "Drafts""#),
            Step::Send(select_reply(2, 14)),
            Step::Expect(r#"a5 UID SEARCH UNDELETED HEADER Message-ID "<d1@example.com>""#),
            Step::Send(found),
        ]
    }

    fn has_expunge(sent: &[String]) -> bool {
        sent.iter().any(|l| l.contains("EXPUNGE"))
    }

    fn save_draft_through(db: &mut Db, script: Vec<Step>, row: i64) -> (Vec<String>, Vec<Event>) {
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(db, imap);
        w.run_job(db, Job::SaveDraft { row }, &mut || None);
        w.imap.take().unwrap().logout();
        let events = events.lock().unwrap().clone();
        (server.finish(), events)
    }

    #[test]
    fn a_new_draft_copy_replaces_only_the_old_uid_with_uidplus() {
        let mut db = Db::in_memory().unwrap();
        let (drafts, row) = draft_setup(&mut db);
        old_copy(&db, &drafts, row, 12);
        let mut script = login_steps(UIDPLUS_CAPS);
        script.extend(append_steps(
            r#"a3 APPEND "Drafts" (\Draft) {"#,
            "{tag} OK [APPENDUID 7 13] APPEND completed\r\n",
        ));
        script.extend(find_steps(
            "* SEARCH 12 13\r\n{tag} OK SEARCH completed\r\n",
        ));
        script.extend([
            Step::Expect(r"a6 UID STORE 12 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
            Step::Expect("a7 UID EXPUNGE 12"),
            Step::Send("* 1 EXPUNGE\r\n{tag} OK EXPUNGE completed\r\n"),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));

        let (lines, events) = save_draft_through(&mut db, script, row);
        assert!(events.is_empty(), "{events:?}");
        // Flagged \Draft only, never \Seen.
        assert!(!lines[2].contains("Seen"), "{}", lines[2]);
        let copy = &lines[3];
        assert!(copy.contains(&format!("X-Komorebi-Draft-Id: {row}\r\n")));
        assert!(copy.contains("Message-ID: <d1@example.com>\r\n"));
        assert!(copy.contains("Bcc: hidden@example.com\r\n"));
        // The half-typed address is left out of the copy, not an error.
        assert!(copy.contains("To: you@example.com\r\n"), "{copy}");
        // Only the old UID is expunged, and only by UID.
        let expunges: Vec<_> = lines.iter().filter(|l| l.contains("EXPUNGE")).collect();
        assert_eq!(expunges, ["a7 UID EXPUNGE 12"]);
        assert!(db.drafts_to_upload(drafts.account_id).unwrap().is_empty());
        let now = db.delete_draft(row).unwrap().unwrap();
        assert_eq!(
            (now.folder_id, now.uidvalidity, now.uid),
            (drafts.id, 7, 13)
        );
    }

    #[test]
    fn without_uidplus_the_old_copy_is_only_flagged() {
        let mut db = Db::in_memory().unwrap();
        let (drafts, row) = draft_setup(&mut db);
        old_copy(&db, &drafts, row, 12);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(append_steps(
            r#"a3 APPEND "Drafts" (\Draft) {"#,
            "{tag} OK APPEND completed\r\n",
        ));
        script.extend(find_steps(
            "* SEARCH 12 13\r\n{tag} OK SEARCH completed\r\n",
        ));
        script.extend([
            Step::Expect(r"a6 UID STORE 12 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));

        let (lines, events) = save_draft_through(&mut db, script, row);
        assert!(events.is_empty(), "{events:?}");
        assert!(!has_expunge(&lines), "never an EXPUNGE: {lines:?}");
        let now = db.delete_draft(row).unwrap().unwrap();
        assert_eq!(now.uid, 13);
    }

    #[test]
    fn a_copy_that_cant_be_found_stops_copies_and_deletes_nothing() {
        let mut db = Db::in_memory().unwrap();
        let (drafts, row) = draft_setup(&mut db);
        old_copy(&db, &drafts, row, 12);
        let mut script = login_steps(UIDPLUS_CAPS);
        script.extend(append_steps(
            r#"a3 APPEND "Drafts" (\Draft) {"#,
            "{tag} OK APPEND completed\r\n",
        ));
        // The search shows only the old copy.
        script.extend(find_steps("* SEARCH 12\r\n{tag} OK SEARCH completed\r\n"));
        script.extend(logout_steps("a6 LOGOUT"));

        let (lines, events) = save_draft_through(&mut db, script, row);
        assert!(!lines.iter().any(|l| l.contains("STORE")), "{lines:?}");
        assert!(
            matches!(&events[..], [Event::Notice { message, .. }] if *message == Notice::NoDraftUid),
            "{events:?}"
        );
        assert_eq!(db.delete_draft(row).unwrap().unwrap().uid, 12);
    }

    #[test]
    fn a_draft_sent_during_its_upload_has_the_new_copy_removed() {
        // Two connections to one in-memory database.
        const URI: &str = "file:komorebi-draft-sent?mode=memory&cache=shared";
        let mut db = Db::open(Path::new(URI)).unwrap();
        let (_, row) = draft_setup(&mut db);
        let mut script = login_steps(UIDPLUS_CAPS);
        script.extend(append_steps(
            r#"a3 APPEND "Drafts" (\Draft) {"#,
            "{tag} OK [APPENDUID 7 13] APPEND completed\r\n",
        ));
        // Send was pressed while the copy uploaded.
        script.insert(
            script.len() - 1,
            Step::Then(Box::new(move || {
                let ui = Db::open(Path::new(URI)).unwrap();
                assert!(ui.outbox_send(row).unwrap());
            })),
        );
        script.extend(find_steps("* SEARCH 13\r\n{tag} OK SEARCH completed\r\n"));
        script.extend([
            Step::Expect(r"a6 UID STORE 13 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
            Step::Expect("a7 UID EXPUNGE 13"),
            Step::Send("{tag} OK EXPUNGE completed\r\n"),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        save_draft_through(&mut db, script, row);
        assert!(db.delete_draft(row).unwrap().is_none(), "nothing recorded");
    }

    #[test]
    fn a_missing_drafts_folder_keeps_the_draft_local() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = db
            .insert_draft(&db::DraftRow {
                account_id: inbox.account_id,
                subject: "Plans".into(),
                message_id: "<d1@example.com>".into(),
                ..Default::default()
            })
            .unwrap();
        // No server is reached (the test connection fails if tried).
        let (mut w, events) = worker_with(&db, None);
        w.run_job(&mut db, Job::SaveDraft { row }, &mut || None);
        w.run_job(&mut db, Job::SaveDraft { row }, &mut || None);
        let events = events.lock().unwrap().clone();
        assert!(
            matches!(&events[..], [Event::Notice { message, .. }] if *message == Notice::NoDraftsFolder),
            "said once: {events:?}"
        );
        assert_eq!(db.drafts_to_upload(inbox.account_id).unwrap(), [(row, 1)]);
    }

    #[test]
    fn an_offline_draft_is_copied_after_the_next_sync_pass() {
        let mut db = Db::in_memory().unwrap();
        let (drafts, row) = draft_setup(&mut db);
        let account_id = drafts.account_id;
        let inbox = db.folder_by_role(account_id, Role::Inbox).unwrap().unwrap();
        for f in [inbox.id, drafts.id] {
            let headers: Vec<_> = (1..=3)
                .map(|uid| NewHeader {
                    uid,
                    ..Default::default()
                })
                .collect();
            db.insert_headers(account_id, f, &headers).unwrap();
            db.finish_folder(f, FP).unwrap();
        }
        let addr = Arc::new(Mutex::new(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            closed_port(),
        ))));
        let (mut w, events) = worker_with(&db, None);
        connect_to(&mut w, &addr);

        // Offline: the save fails quietly and the draft waits.
        w.run_job(&mut db, Job::SaveDraft { row }, &mut || None);
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(db.drafts_to_upload(account_id).unwrap(), [(row, 1)]);

        // Back online: the sync pass (nothing changed) is followed by the copy.
        let mut script = login_steps(
            "* CAPABILITY IMAP4rev1 CONDSTORE LIST-EXTENDED LIST-STATUS UIDPLUS\r\n{tag} OK done\r\n",
        );
        script.extend([
            Step::Expect(leak(format!("a3 {LIST_STATUS}"))),
            Step::Send(list_reply(&["INBOX", "Drafts"], |_| Some(UNCHANGED))),
        ]);
        script.extend(append_steps(
            r#"a4 APPEND "Drafts" (\Draft) {"#,
            "{tag} OK [APPENDUID 7 20] APPEND completed\r\n",
        ));
        script.extend([
            Step::Expect(r#"a5 SELECT "Drafts""#),
            Step::Send(select_reply(4, 21)),
            Step::Expect("a6 UID SEARCH UNDELETED HEADER Message-ID"),
            Step::Send("* SEARCH 20\r\n{tag} OK SEARCH completed\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);
        *addr.lock().unwrap() = server.addr;
        // Not the first pass, which is always full.
        w.passes_since_full = Some(0);
        w.run_job(&mut db, Job::SyncAll { full: false }, &mut || None);
        w.imap.take().unwrap().logout();
        let lines = server.finish();
        assert!(lines[4].contains("Subject: Plans"));
        assert!(sync_errors(&events).is_empty());
        assert!(db.drafts_to_upload(account_id).unwrap().is_empty());
        assert_eq!(db.delete_draft(row).unwrap().unwrap().uid, 20);
    }

    #[test]
    fn delivery_removes_the_draft_copy() {
        let mut db = Db::in_memory().unwrap();
        let (drafts, _) = draft_setup(&mut db);
        let row = queued(&db, drafts.account_id, "Hello");
        let copy = db::ServerCopy {
            account_id: drafts.account_id,
            folder_id: drafts.id,
            uidvalidity: 7,
            uid: 5,
        };
        // The copy saved while it was a draft.
        assert!(db.outbox_unsend(row).unwrap());
        db.record_server_copy(row, 1, Some(&copy)).unwrap();
        assert!(db.outbox_send(row).unwrap());
        let raw = smtp::build_raw(smtp::Outgoing {
            from_email: "me@example.com".into(),
            to: "you@example.com".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(db.store_raw(row, &raw, &AtomicBool::new(false)).unwrap());
        assert!(db.outbox_queue(row).unwrap());
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "Drafts""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a4 UID STORE 5 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let imap = FakeServer::start(script);
        let addr = imap.addr;
        let (mut w, _) = worker_with(&db, None);
        w.connect = Box::new(move |_| Imap::connect_plain(addr, "user", "pass"));
        let (events, _) = send_through(SmtpEnd::Accept, &mut db, w, row);
        assert!(!has_expunge(&imap.finish()));
        assert!(
            matches!(
                &events[..],
                [Event::Sent {
                    outcome: Outcome::Delivered,
                    ..
                }]
            ),
            "{events:?}"
        );
    }

    #[test]
    fn a_copy_of_another_account_goes_to_that_account() {
        let mut db = Db::in_memory().unwrap();
        let (drafts, row) = draft_setup(&mut db);
        let (mut w, events) = worker_with(&db, None);
        let elsewhere = db::ServerCopy {
            account_id: drafts.account_id + 1,
            folder_id: drafts.id,
            uidvalidity: 7,
            uid: 3,
        };
        w.run_job(&mut db, Job::DeleteDraft(elsewhere), &mut || None);
        let events = events.lock().unwrap().clone();
        assert!(matches!(&events[..], [Event::DraftCopyLeft(c)] if *c == elsewhere));
        assert_eq!(db.drafts_to_upload(drafts.account_id).unwrap(), [(row, 1)]);
    }

    #[test]
    fn a_large_delete_is_one_store_and_expunge() {
        let uids: Vec<u32> = (1..=200).collect();
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &uids);
        let mut script = login_steps(UIDPLUS_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a4 UID STORE 1:200 +FLAGS.SILENT (\Deleted)"),
            Step::Send("{tag} OK STORE completed\r\n"),
            Step::Expect("a5 UID EXPUNGE 1:200"),
            Step::Send("{tag} OK EXPUNGE completed\r\n"),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        let job = Job::Delete {
            folder_id: f.id,
            uids,
        };
        // Every message of the batch is pending while the job is queued.
        let keys = job.message_keys();
        assert_eq!(keys.len(), 200);
        assert!(keys.iter().all(|&(folder, _)| folder == f.id));
        w.run_job(&mut db, job, &mut || None);
        assert!(events.lock().unwrap().is_empty());
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn junk_reports_are_moves_and_not_junk_is_read_first() {
        // The jobs the app holds for Report junk and Not junk (`remove_to`):
        // a move to Junk; back from Junk, \Seen and then a move to INBOX.
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[5]);
        let junk = add_folder(&mut db, inbox.account_id, "Junk", &[7], None);
        let mut script = login_steps("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n");
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r#"a4 UID MOVE 5 "Junk""#),
            Step::Send("{tag} OK MOVE completed\r\n"),
            Step::Expect(r#"a5 SELECT "Junk""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a6 UID STORE 7 +FLAGS.SILENT (\Seen)"),
            Step::Send("{tag} OK STORE completed\r\n"),
            Step::Expect(r#"a7 UID MOVE 7 "INBOX""#),
            Step::Send("{tag} OK MOVE completed\r\n"),
        ]);
        script.extend(logout_steps("a8 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        let jobs = [
            Job::Move {
                folder_id: inbox.id,
                uids: vec![5],
                dest: junk.id,
            },
            Job::SetFlags {
                folder_id: junk.id,
                uids: vec![7],
                flag: "\\Seen",
                on: true,
            },
            Job::Move {
                folder_id: junk.id,
                uids: vec![7],
                dest: inbox.id,
            },
        ];
        for job in jobs {
            w.run_job(&mut db, job, &mut || None);
        }
        assert!(matches!(
            events.lock().unwrap()[..],
            [Event::NeedsSync { folder_id: a }, Event::NeedsSync { folder_id: b }]
                if a == junk.id && b == inbox.id
        ));
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn a_large_move_is_one_move_per_folder() {
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &(1..=150).collect::<Vec<_>>());
        let work = add_folder(&mut db, inbox.account_id, "Work", &[3, 4, 5, 9], None);
        let trash = add_folder(&mut db, inbox.account_id, "Trash", &[], None);
        let mut script = login_steps("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n");
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r#"a4 UID MOVE 1:150 "Trash""#),
            Step::Send("{tag} OK MOVE completed\r\n"),
            Step::Expect(r#"a5 SELECT "Work""#),
            Step::Send(SELECT_OK),
            Step::Expect(r#"a6 UID MOVE 3:5,9 "Trash""#),
            Step::Send("{tag} OK MOVE completed\r\n"),
        ]);
        script.extend(logout_steps("a7 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        for (folder_id, uids) in [
            (inbox.id, (1..=150).rev().collect()),
            (work.id, vec![9, 3, 4, 5]),
        ] {
            let job = Job::Move {
                folder_id,
                uids,
                dest: trash.id,
            };
            w.run_job(&mut db, job, &mut || None);
        }
        assert!(matches!(
            events.lock().unwrap()[..],
            [Event::NeedsSync { folder_id: a }, Event::NeedsSync { folder_id: b }]
                if a == trash.id && b == trash.id
        ));
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn mark_all_read_is_one_store_up_to_the_highest_cached_uid() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(
            &mut db,
            7,
            &(1..=300).filter(|u| u % 7 != 0).collect::<Vec<_>>(),
        );
        let (uids, through) = db.mark_folder_read(f.id).unwrap();
        assert_eq!((uids.len(), through), (258, 300));
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            // Never `1:*`: mail that arrived after the last sync stays unread.
            Step::Expect(r"a4 UID STORE 1:300 +FLAGS.SILENT (\Seen)"),
            Step::Send("{tag} OK STORE completed\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        let job = Job::MarkAllRead {
            folder_id: f.id,
            through,
            uids,
        };
        // The messages marked read in the cache stay pending until it ends.
        assert_eq!(job.message_keys().len(), 258);
        w.run_job(&mut db, job, &mut || None);
        assert!(events.lock().unwrap().is_empty());
        w.imap.take().unwrap().logout();
        server.finish();
    }

    // ---- resilience: refusals, gone messages, waiting actions, wake

    #[test]
    fn a_connection_limit_never_latches_the_sign_in() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let (mut w, events) = worker_with(&db, None);
        let addr = Arc::new(Mutex::new(std::net::SocketAddr::from(([127, 0, 0, 1], 1))));
        let attempts = connect_to(&mut w, &addr);
        // Dovecot's limit has no response code; others say [LIMIT] or
        // [UNAVAILABLE] (Temporary authentication failure).
        for reply in [
            "{tag} NO Maximum number of connections from user+IP exceeded (mail_max_userip_connections=10)\r\n",
            "{tag} NO [LIMIT] Too many connections\r\n",
            "{tag} NO [UNAVAILABLE] Temporary authentication failure\r\n",
            "{tag} NO Temporary authentication failure\r\n",
        ] {
            let server = login_refused(reply);
            *addr.lock().unwrap() = server.addr;
            let job = Job::SyncFolder {
                id: f.id,
                full: true,
            };
            w.run_job(&mut db, job, &mut || None);
            server.finish();
            assert!(!w.auth_failed.get(), "{reply}");
            assert!(
                matches!(
                    &events.lock().unwrap().drain(..).collect::<Vec<_>>()[..],
                    [Event::ActionFailed { error, .. }] if error.class == Class::TooManyConnections
                ),
                "{reply}"
            );
        }
        assert_eq!(*attempts.lock().unwrap(), 4);
    }

    #[test]
    fn actions_on_messages_already_gone_count_as_done() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        db.finish_folder(f.id, FP).unwrap();
        let archive = add_folder(&mut db, f.account_id, "Archive", &[], None);
        let mut script = login_steps("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n");
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a4 UID STORE 5 +FLAGS.SILENT (\Seen)"),
            Step::Send("{tag} NO Some of the requested messages no longer exist\r\n"),
            Step::Expect("a5 UID MOVE 5"),
            Step::Send("{tag} NO [EXPUNGEISSUED] Some messages were expunged\r\n"),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);
        let imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let (mut w, events) = worker(&db, imap);

        let flag = Job::SetFlags {
            folder_id: f.id,
            uids: vec![5],
            flag: "\\Seen",
            on: true,
        };
        w.run_job(&mut db, flag, &mut || None);
        let mv = Job::Move {
            folder_id: f.id,
            uids: vec![5],
            dest: archive.id,
        };
        w.run_job(&mut db, mv, &mut || None);
        // No failure: the flag change has nothing left to do, and the move
        // says why nothing moved. Both resync the folder.
        {
            let events = events.lock().unwrap();
            assert!(
                matches!(
                    &events[..],
                    [
                        Event::NeedsSync { folder_id: a },
                        Event::Notice { message, .. },
                        Event::NeedsSync { folder_id: b },
                    ] if *a == f.id && *b == f.id && *message == Notice::MoveGone
                ),
                "{events:?}"
            );
        }
        assert_eq!(db.fingerprint(f.id).unwrap(), None);
        w.imap.take().unwrap().logout();
        server.finish();
    }

    /// A connect closure that fails as if offline while `at` is `None`.
    fn connect_when(w: &mut Worker, at: &Arc<Mutex<Option<std::net::SocketAddr>>>) {
        let at = at.clone();
        w.connect = Box::new(move |_| match *at.lock().unwrap() {
            Some(addr) => Imap::connect_plain(addr, "user", "pass"),
            None => Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into()),
        });
    }

    /// Run `job` as the interactive worker does: its messages pending
    /// meanwhile.
    fn run_action(w: &mut Worker, db: &mut Db, job: Job) {
        let keys = job.message_keys();
        keys.iter().for_each(|&k| w.pending.add(k));
        w.run_job(db, job, &mut || None);
        keys.iter().for_each(|&k| w.pending.done(k));
    }

    #[test]
    fn an_action_cut_off_from_the_server_runs_once_on_the_next_connection() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        db.finish_folder(f.id, FP).unwrap();
        let (mut w, events) = worker_with(&db, None);
        let at = Arc::new(Mutex::new(None));
        connect_when(&mut w, &at);
        let job = Job::SetFlags {
            folder_id: f.id,
            uids: vec![5],
            flag: "\\Flagged",
            on: true,
        };
        run_action(&mut w, &mut db, job);
        // Nothing reported or undone; the message stays pending, so a sync
        // can't show the old flag meanwhile.
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(w.parked.len(), 1);
        assert_eq!(w.pending.uids(f.id), HashSet::from([5]));
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(FP));
        // Still offline: it waits.
        w.retry_parked(&mut db);
        assert_eq!(w.parked.len(), 1);
        assert!(events.lock().unwrap().is_empty());

        // Back online: the one retry goes through.
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a4 UID STORE 5 +FLAGS.SILENT (\Flagged)"),
            Step::Send("{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a5 LOGOUT"));
        let server = FakeServer::start(script);
        *at.lock().unwrap() = Some(server.addr);
        w.retry_parked(&mut db);
        assert!(w.parked.is_empty());
        assert!(w.pending.uids(f.id).is_empty());
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(db.fingerprint(f.id).unwrap(), Some(FP));
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn a_new_action_waits_behind_earlier_ones_on_its_folder() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        db.finish_folder(f.id, FP).unwrap();
        let (mut w, events) = worker_with(&db, None);
        let at = Arc::new(Mutex::new(None));
        connect_when(&mut w, &at);
        let flag = |on| Job::SetFlags {
            folder_id: f.id,
            uids: vec![5],
            flag: "\\Flagged",
            on,
        };
        run_action(&mut w, &mut db, flag(true));
        // Still offline: the next one queues behind it.
        run_action(&mut w, &mut db, flag(false));
        assert_eq!(w.parked.len(), 2);
        assert_eq!(w.pending.uids(f.id), HashSet::from([5]));

        // Back online: the set goes first, so the clear is the last word.
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect(r"a4 UID STORE 5 +FLAGS.SILENT (\Flagged)"),
            Step::Send("{tag} OK done\r\n"),
            Step::Expect(r"a5 UID STORE 5 -FLAGS.SILENT (\Flagged)"),
            Step::Send("{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);
        *at.lock().unwrap() = Some(server.addr);
        w.retry_parked(&mut db);
        assert!(w.parked.is_empty());
        assert!(events.lock().unwrap().is_empty());
        w.imap.take().unwrap().logout();
        server.finish();
    }

    #[test]
    fn a_waiting_action_whose_retry_fails_too_is_undone() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        db.finish_folder(f.id, FP).unwrap();
        let archive = add_folder(&mut db, f.account_id, "Archive", &[], None);
        let (mut w, events) = worker_with(&db, None);
        let at = Arc::new(Mutex::new(None));
        connect_when(&mut w, &at);
        let job = Job::Move {
            folder_id: f.id,
            uids: vec![5],
            dest: archive.id,
        };
        run_action(&mut w, &mut db, job);
        assert_eq!(w.parked.len(), 1);

        // The server is back, but the connection drops during the MOVE.
        let offline = at.clone();
        let mut script = login_steps("* CAPABILITY IMAP4rev1 MOVE\r\n{tag} OK done\r\n");
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID MOVE 5"),
            Step::Then(Box::new(move || *offline.lock().unwrap() = None)),
            Step::Close,
        ]);
        let server = FakeServer::start(script);
        *at.lock().unwrap() = Some(server.addr);
        w.retry_parked(&mut db);
        server.finish();
        assert!(w.parked.is_empty());
        assert!(w.pending.uids(f.id).is_empty());
        // Reported once, and the folder brought back to the server's state.
        {
            let events = events.lock().unwrap();
            assert!(
                matches!(
                    &events[..],
                    [Event::ActionFailed { error, .. }, Event::NeedsSync { folder_id }]
                        if error.class == Class::Offline && *folder_id == f.id
                ),
                "{events:?}"
            );
        }
        assert_eq!(db.fingerprint(f.id).unwrap(), None);
    }

    #[test]
    fn a_waiting_action_is_given_up_after_a_while() {
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[5]);
        db.finish_folder(f.id, FP).unwrap();
        let (mut w, events) = worker_with(&db, None);
        connect_when(&mut w, &Arc::new(Mutex::new(None)));
        let job = Job::Delete {
            folder_id: f.id,
            uids: vec![5],
        };
        run_action(&mut w, &mut db, job);
        assert_eq!(w.parked.len(), 1);
        let Some(long_ago) = Instant::now().checked_sub(PARK_MAX) else {
            return; // The clock started less than PARK_MAX ago.
        };
        w.parked[0].since = long_ago;
        w.retry_parked(&mut db);
        assert!(w.parked.is_empty());
        assert!(w.pending.uids(f.id).is_empty());
        let events = events.lock().unwrap();
        assert!(
            matches!(
                &events[..],
                [Event::ActionFailed { error, .. }, Event::NeedsSync { .. }]
                    if error.class == Class::Offline
            ),
            "{events:?}"
        );
    }

    #[test]
    fn after_a_wake_the_connection_is_replaced_without_a_probe() {
        let mut db = Db::in_memory().unwrap();
        setup(&mut db, 7, &[]);
        let mut script = login_steps(PLAIN_CAPS);
        script.push(Step::ExpectEof);
        let old = FakeServer::start(script);
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(logout_steps("a3 LOGOUT"));
        let new = FakeServer::start(script);
        let imap = Imap::connect_plain(old.addr, "user", "pass").unwrap();
        let (mut w, _) = worker(&db, imap);
        let attempts = connect_to(&mut w, &Arc::new(Mutex::new(new.addr)));
        let resume = w.resume.clone();

        resume.fetch_add(1, Ordering::Relaxed); // Workers::resumed
        w.run_job(&mut db, Job::Warm, &mut || None);
        // The old connection got no NOOP, not even a LOGOUT.
        assert_eq!(old.finish().len(), 2);
        assert_eq!(*attempts.lock().unwrap(), 1);
        // Only once per wake.
        w.run_job(&mut db, Job::Warm, &mut || None);
        assert_eq!(*attempts.lock().unwrap(), 1);
        w.imap.take().unwrap().logout();
        new.finish();
    }

    #[test]
    fn a_long_header_download_reports_its_progress() {
        let reply = (1..=3)
            .map(|uid| header_fetch(uid, uid, "", "Subject: x\r\n\r\n"))
            .collect::<String>()
            + "{tag} OK done\r\n";
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 1:* (UID FLAGS)"),
            Step::Send(concat!(
                "* 1 FETCH (UID 1 FLAGS ())\r\n",
                "* 2 FETCH (UID 2 FLAGS ())\r\n",
                "* 3 FETCH (UID 3 FLAGS ())\r\n",
                "{tag} OK done\r\n",
            )),
            Step::Expect("a5 UID FETCH 1:3 (UID FLAGS INTERNALDATE"),
            Step::Send(leak(reply)),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);
        let mut db = Db::in_memory().unwrap();
        let f = setup(&mut db, 7, &[]);
        let mut imap = Imap::connect_plain(server.addr, "user", "pass").unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let notify: Notify = Arc::new(move |e| sink.lock().unwrap().push(e));

        sync_folder_notify(&mut imap, &mut db, &f, &notify).unwrap();
        let progress: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::Progress { done, total, .. } => Some((*done, *total)),
                _ => None,
            })
            .collect();
        assert_eq!(progress, [(3, 3)]);
        imap.logout();
        server.finish();
    }

    /// The diagnostics log names commands, codes and timings only: a
    /// session whose password, subject and server texts carry sentinels
    /// leaves none of them in it.
    #[test]
    fn the_diagnostics_log_never_carries_the_password_or_the_subject() {
        const PASSWORD: &str = "Sentinel-Pa55word";
        const SUBJECT: &str = "Sentinel subject 7f3a";
        let dir = std::env::temp_dir().join(format!("komorebi-diag-redact-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let log = diag::log_path(&dir.join("cache.sqlite3"));
        diag::set(Some(log.clone()));

        let header = header_fetch(1, 5, "", &format!("Subject: {SUBJECT}\r\n\r\n"));
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect(r#"a3 SELECT "INBOX""#),
            Step::Send(SELECT_OK),
            Step::Expect("a4 UID FETCH 5 (UID FLAGS INTERNALDATE"),
            Step::Send(leak(header + "{tag} OK done\r\n")),
            Step::Expect(r"a5 UID STORE 5 +FLAGS.SILENT (\Flagged)"),
            Step::Send(leak(format!(
                "{{tag}} NO [OVERQUOTA] {SUBJECT} is over quota\r\n"
            ))),
        ]);
        script.extend(logout_steps("a6 LOGOUT"));
        let server = FakeServer::start(script);
        let mut imap = Imap::connect_plain(server.addr, "user", PASSWORD).unwrap();
        imap.set_account(7);
        imap.select("INBOX").unwrap();
        assert_eq!(imap.fetch_headers(&[5]).unwrap()[0].subject, SUBJECT);
        assert!(
            imap.set_flags(("INBOX", 7), &[5], "\\Flagged", true)
                .is_err()
        );
        imap.logout();
        let sent = server.finish();
        assert!(sent[0].contains(PASSWORD), "the fake saw the password");

        // A send whose body carries the sentinel too.
        let mut db = Db::in_memory().unwrap();
        let inbox = setup(&mut db, 7, &[]);
        let row = queued(&db, inbox.account_id, SUBJECT);
        let (w, _) = worker_with(&db, None);
        send_through(SmtpEnd::Accept, &mut db, w, row);
        diag::set(None);

        let text = std::fs::read_to_string(&log).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            !text.contains(PASSWORD) && !text.contains("Sentinel"),
            "{text}"
        );
        for want in [
            "account 7 SELECT OK",
            "account 7 FETCH OK",
            "account 7 STORE NO [OVERQUOTA]",
            "account 7 LOGOUT OK",
            "SMTP SEND OK",
        ] {
            assert!(text.contains(want), "{want} in {text}");
        }
        // Every line: time, account, verb, status, duration.
        for line in text.lines() {
            assert!(
                line.ends_with(" ms") && line.contains(" account "),
                "{line}"
            );
        }
    }

    #[test]
    fn a_temporary_smtp_sign_in_refusal_is_retried() {
        let server = FakeSmtp::start(SmtpEnd::TempRejectAuth);
        let e = smtp::verify_plain(server.port, "me@example.com", "pw").unwrap_err();
        server.finish();
        assert!(
            matches!(send_failure(&e), Outcome::Retry(t) if t.contains("454")),
            "{e:#}"
        );
    }

    fn account_with_logins(imap_user: &str, smtp_user: &str) -> Account {
        let server = Server {
            host: "mail.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        Account {
            id: 1,
            email: "me@example.com".into(),
            name: String::new(),
            imap: server.clone(),
            smtp: server,
            save_sent: false,
            signature: String::new(),
            enabled: true,
            imap_user: imap_user.into(),
            smtp_user: smtp_user.into(),
            imap_pin: None,
            smtp_pin: None,
        }
    }

    /// The LOGIN line an IMAP fake got for `login`, and the user name and
    /// password in the AUTH PLAIN payload an SMTP fake got for `smtp_login`.
    fn logins_seen(login: &str, smtp_login: &str) -> (String, (String, String)) {
        use base64::Engine;
        let mut script = login_steps(PLAIN_CAPS);
        script.extend(logout_steps("a3 LOGOUT"));
        let server = FakeServer::start(script);
        Imap::connect_plain(server.addr, login, "pw")
            .unwrap()
            .logout();
        let login_line = server.finish().remove(0);

        let smtp = FakeSmtp::start(SmtpEnd::Accept);
        smtp::verify_plain(smtp.port, smtp_login, "pw").unwrap();
        let auth = smtp.finish_with_auth();
        let payload = auth
            .iter()
            .find_map(|l| l.strip_prefix("AUTH PLAIN "))
            .expect("an AUTH PLAIN line");
        let plain = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        let plain = String::from_utf8(plain).unwrap();
        let mut parts = plain.split('\0');
        assert_eq!(parts.next(), Some(""), "no authorization identity");
        let user = parts.next().unwrap().to_string();
        let password = parts.next().unwrap().to_string();
        (login_line, (user, password))
    }

    #[test]
    fn imap_and_smtp_sign_in_with_their_own_logins() {
        let a = account_with_logins("alice", "bob");
        let (imap, smtp) = logins_seen(a.imap_login(), a.smtp_login());
        assert_eq!(imap, r#"a1 LOGIN "alice" "pw""#);
        assert_eq!(smtp, ("bob".to_string(), "pw".to_string()));

        // Empty logins are the email address, for both.
        let a = account_with_logins("", "");
        let (imap, smtp) = logins_seen(a.imap_login(), a.smtp_login());
        assert_eq!(imap, r#"a1 LOGIN "me@example.com" "pw""#);
        assert_eq!(smtp, ("me@example.com".to_string(), "pw".to_string()));

        // One of them set: the other stays the address.
        let a = account_with_logins("alice", "");
        assert_eq!(
            (a.imap_login(), a.smtp_login()),
            ("alice", "me@example.com")
        );
    }

    #[test]
    fn a_login_with_quotes_and_backslashes_is_sent_intact() {
        let login = r#"dom\al"ice"#;
        let (imap, smtp) = logins_seen(login, login);
        assert_eq!(imap, r#"a1 LOGIN "dom\\al\"ice" "pw""#);
        assert_eq!(smtp, (login.to_string(), "pw".to_string()));
    }

    // ---- "Test connection": the failing step and its sentence

    use errors::{Special, Step as Check, StepError};

    fn imap_server(host: &str, port: u16, security: Security) -> Server {
        Server {
            host: host.into(),
            port,
            security,
        }
    }

    /// `Imap::connect_steps` as the form calls it, for a failure.
    fn connect_failure(server: &Server) -> StepError {
        let (step, e) = match Imap::connect_steps(server, None, "alice", "pw", Some(&mut |_| {})) {
            Err(failure) => failure,
            Ok(_) => panic!("connected"),
        };
        imap_step_error(step, e, server, false, "alice")
    }

    #[test]
    fn a_name_that_does_not_resolve_fails_the_resolve_step() {
        let e = connect_failure(&imap_server("nonexistent.invalid", 993, Security::Tls));
        assert_eq!((e.step, e.class), (Check::Resolve, Class::Offline));
        assert_eq!(
            e.say(),
            Say::NotFound {
                host: "nonexistent.invalid".into()
            }
        );
    }

    #[test]
    fn a_closed_port_fails_the_connect_step() {
        let port = closed_port();
        let e = connect_failure(&imap_server("127.0.0.1", port, Security::Tls));
        assert_eq!((e.step, e.class), (Check::Connect, Class::Offline));
        assert_eq!(
            e.say(),
            Say::NoAnswer {
                host: "127.0.0.1".into(),
                port
            }
        );
        // The raw chain stays in the error's source, not in the sentence
        // or in what the logs get.
        assert!(format!("{:#}", e.source).contains("connecting to"));
        assert!(!e.to_string().contains("connecting to"), "{e}");
    }

    #[test]
    fn a_refused_starttls_fails_the_secure_step_before_any_password() {
        let server = FakeServer::start(vec![
            Step::Send("* OK fake IMAP ready\r\n"),
            Step::Expect("a0 CAPABILITY"),
            Step::Send("* CAPABILITY IMAP4rev1 STARTTLS\r\na0 OK done\r\n"),
            Step::Expect("a1 STARTTLS"),
            Step::Send("a1 NO not today\r\n"),
        ]);
        let e = connect_failure(&imap_server(
            "127.0.0.1",
            server.addr.port(),
            Security::StartTls,
        ));
        // No LOGIN reached the server.
        assert_eq!(server.finish(), ["a0 CAPABILITY", "a1 STARTTLS"]);
        assert_eq!((e.step, e.special), (Check::Secure, Special::NoStartTls));
        assert_eq!(
            e.say(),
            Say::NoStartTls {
                host: "127.0.0.1".into()
            }
        );
    }

    #[test]
    fn a_plain_answer_on_a_tls_port_is_named() {
        // A server that greets in plain text on every connection.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                std::thread::spawn(move || {
                    let mut stream = stream;
                    use std::io::Write;
                    stream.write_all(b"* OK plain IMAP ready\r\n").ok();
                    std::thread::sleep(Duration::from_millis(800));
                });
            }
        });
        let e = connect_failure(&imap_server("127.0.0.1", port, Security::Tls));
        assert_eq!((e.step, e.special), (Check::Secure, Special::PlainAnswer));
        assert_eq!(e.say(), Say::PlainAnswer);
    }

    #[test]
    fn a_tls_server_is_not_taken_for_a_plain_one() {
        // Silent until the client speaks, as a TLS port is.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _held = listener.accept();
            std::thread::sleep(Duration::from_secs(3));
        });
        assert!(!imap::answers_plain("127.0.0.1", port));
    }

    /// The whole check against the plain fakes: IMAP as `Imap::connect_plain`
    /// gets it (which reports the steps a TLS connection would), SMTP over
    /// plain TCP. Returns the steps reported and the outcome.
    fn check_plain(
        imap_script: Vec<Step>,
        smtp_end: SmtpEnd,
    ) -> (Vec<Check>, Result<(), StepError>) {
        let imap_fake = FakeServer::start(imap_script);
        let smtp_fake = FakeSmtp::start(smtp_end);
        let addr = imap_fake.addr;
        let port = smtp_fake.port;
        let server = imap_server("imap.example.com", 993, Security::Tls);
        let mut seen = Vec::new();
        let result = run_steps(
            &server,
            false,
            "alice",
            |report| {
                let imap =
                    Imap::connect_plain(addr, "alice", "pw").map_err(|e| (Check::ImapSignIn, e))?;
                for step in [
                    Check::Resolve,
                    Check::Connect,
                    Check::Secure,
                    Check::ImapSignIn,
                ] {
                    report(step);
                }
                Ok(imap)
            },
            |report| smtp::verify_plain_steps(port, "relay", "pw", report),
            &mut |step| seen.push(step),
        );
        if result.is_err() {
            // The fakes may not have been reached to the end.
            drop((imap_fake, smtp_fake));
        } else {
            imap_fake.finish();
            smtp_fake.finish_with_auth();
        }
        (seen, result)
    }

    fn list_and_logout() -> Vec<Step> {
        let mut script = login_steps(PLAIN_CAPS);
        script.extend([
            Step::Expect("a3 LIST"),
            Step::Send("* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n{tag} OK done\r\n"),
        ]);
        script.extend(logout_steps("a4 LOGOUT"));
        script
    }

    #[test]
    fn a_working_account_passes_every_step_in_order() {
        let (seen, result) = check_plain(list_and_logout(), SmtpEnd::Accept);
        result.unwrap();
        assert_eq!(seen, Check::ALL);
    }

    #[test]
    fn a_rejected_imap_password_fails_the_imap_sign_in_step() {
        let (seen, result) = check_plain(
            vec![
                Step::Send("* OK fake IMAP ready\r\n"),
                Step::Expect("a1 LOGIN"),
                Step::Send("{tag} NO [AUTHENTICATIONFAILED] bad credentials\r\n"),
            ],
            SmtpEnd::Accept,
        );
        let e = result.unwrap_err();
        assert!(seen.is_empty(), "{seen:?}");
        assert_eq!((e.step, e.class), (Check::ImapSignIn, Class::Auth));
        assert_eq!(
            e.say(),
            Say::Rejected {
                host: "imap.example.com".into(),
                login: "alice".into()
            }
        );
    }

    #[test]
    fn a_rejected_smtp_password_fails_the_smtp_sign_in_step() {
        let (seen, result) = check_plain(list_and_logout(), SmtpEnd::RejectAuth);
        let e = result.unwrap_err();
        // IMAP passed, and so did the SMTP connection.
        assert_eq!(seen.last(), Some(&Check::SmtpConnect));
        assert_eq!((e.step, e.class), (Check::SmtpSignIn, Class::Auth));
        assert_eq!(
            e.say(),
            Say::Rejected {
                host: "127.0.0.1".into(),
                login: "relay".into()
            }
        );
    }

    #[test]
    fn a_busy_smtp_server_gets_the_busy_sentence() {
        let (_, result) = check_plain(list_and_logout(), SmtpEnd::TempRejectAuth);
        let e = result.unwrap_err();
        assert_eq!(
            (e.step, e.class),
            (Check::SmtpSignIn, Class::TooManyConnections)
        );
        assert_eq!(e.say(), Say::Class(Class::TooManyConnections));
    }

    #[test]
    fn sending_failures_are_classified_like_incoming_ones() {
        let smtp = |end| {
            let server = FakeSmtp::start(end);
            let e = smtp::verify_plain(server.port, "me", "pw").unwrap_err();
            drop(server);
            e
        };
        assert_eq!(errors::classify(&smtp(SmtpEnd::RejectAuth)), Class::Auth);
        assert_eq!(
            errors::classify(&smtp(SmtpEnd::TempRejectAuth)),
            Class::TooManyConnections
        );
        // A refused recipient is a message problem, not a sign-in one.
        assert_eq!(errors::classify_smtp(&anyhow!("plain")), None);
    }

    fn verify_server(server: &Server) -> StepError {
        let mut seen = Vec::new();
        let e = smtp::verify_smtp(server, None, "relay", "pw", &mut |s| seen.push(s)).unwrap_err();
        assert!(seen.is_empty(), "{seen:?}");
        e
    }

    #[test]
    fn smtp_failures_before_the_sign_in_name_their_step() {
        // Closed port.
        let e = verify_server(&imap_server("127.0.0.1", closed_port(), Security::Tls));
        assert_eq!((e.step, e.class), (Check::SmtpConnect, Class::Offline));
        assert!(matches!(e.say(), Say::NoAnswer { .. }), "{:?}", e.say());
        // A name that does not resolve is said so, naming the sending server.
        let e = verify_server(&imap_server("nonexistent.invalid", 465, Security::Tls));
        assert_eq!(
            (e.step, e.special),
            (Check::SmtpConnect, Special::SmtpResolve)
        );
        assert_eq!(
            e.say(),
            Say::NotFound {
                host: "nonexistent.invalid".into()
            }
        );
        // STARTTLS wanted, not offered: no AUTH line ever arrives.
        let fake = FakeSmtp::start(SmtpEnd::Accept);
        let e = verify_server(&imap_server("127.0.0.1", fake.port, Security::StartTls));
        assert_eq!(
            (e.step, e.special),
            (Check::SmtpConnect, Special::NoStartTls)
        );
        assert!(matches!(e.say(), Say::NoStartTls { .. }), "{:?}", e.say());
        assert!(fake.finish_with_auth().is_empty());
    }

    #[test]
    fn a_plain_smtp_greeting_on_a_tls_port_is_named() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                std::thread::spawn(move || {
                    let mut stream = stream;
                    use std::io::Write;
                    stream.write_all(b"220 plain SMTP ready\r\n").ok();
                    std::thread::sleep(Duration::from_millis(800));
                });
            }
        });
        let e = verify_server(&imap_server("127.0.0.1", port, Security::Tls));
        assert_eq!(
            (e.step, e.special),
            (Check::SmtpConnect, Special::PlainAnswer)
        );
        assert_eq!(e.say(), Say::PlainAnswer);
    }
}
