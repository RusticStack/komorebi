//! Mail workers.
//!
//! Each account gets two worker threads, each with its own IMAP connection
//! and SQLite connection:
//! - **sync**: folder list, header sync, flag sync and body prefetch. These
//!   jobs can run for a long time.
//! - **interactive**: opening messages, flag changes, moves and sending.
//!   These run while a long sync is in progress, so the UI never waits on a
//!   sync.
//!
//! Workers report back through [`Event`]s, which the UI applies on its own
//! thread.

pub mod html;
pub mod imap;
pub mod mime;
pub mod smtp;

use crate::db::{Account, Db, Folder};
use crate::providers::Server;
use crate::role::Role;
use crate::secrets;
use anyhow::{Context, Result, anyhow};
use imap::Imap;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};

/// How many headers are fetched per round trip during a first sync.
const HEADER_BATCH: usize = 500;
/// How many inbox bodies are prefetched (newest first) for instant opening.
const PREFETCH_COUNT: usize = 300;
/// Bodies larger than this are fetched only when opened.
const PREFETCH_MAX_SIZE: i64 = 512 * 1024;

#[derive(Debug, Clone)]
pub enum Event {
    Syncing {
        account_id: i64,
        active: bool,
        error: Option<String>,
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
        error: String,
    },
    Sent {
        result: Result<(), String>,
        sent_folder: Option<i64>,
    },
    /// A server action failed; the next sync restores the true state.
    ActionFailed {
        account_id: i64,
        error: String,
    },
    /// A folder got new mail from our own action (move/append) and should
    /// be synced.
    NeedsSync {
        folder_id: i64,
    },
}

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
    SyncAll,
    SyncFolder(i64),
    FetchBody(i64),
    SetFlag {
        folder_id: i64,
        uid: u32,
        flag: &'static str,
        on: bool,
    },
    Move {
        folder_id: i64,
        uid: u32,
        dest: i64,
    },
    Delete {
        folder_id: i64,
        uid: u32,
    },
    Send {
        outgoing: Box<smtp::Outgoing>,
        reply_to: Option<i64>,
    },
}

pub struct Workers {
    sync: Sender<Job>,
    interactive: Sender<Job>,
    pending: Pending,
}

impl Workers {
    pub fn spawn(account: Account, db_path: PathBuf, notify: Notify) -> Self {
        let (sync, sync_rx) = channel();
        let (interactive, inter_rx) = channel();
        let pending = Pending::default();
        for (rx, name) in [(sync_rx, "sync"), (inter_rx, "interactive")] {
            let mut w = Worker {
                account: account.clone(),
                db_path: db_path.clone(),
                notify: notify.clone(),
                imap: None,
                pending: pending.clone(),
            };
            std::thread::Builder::new()
                .name(format!("{name}-{}", account.id))
                .spawn(move || w.run(rx))
                .expect("spawning mail worker");
        }
        Self {
            sync,
            interactive,
            pending,
        }
    }

    pub fn send(&self, job: Job) {
        if let Some(key) = job.message_key() {
            self.pending.add(key);
        }
        let tx = match job {
            Job::SyncAll | Job::SyncFolder(_) => &self.sync,
            _ => &self.interactive,
        };
        // A closed channel means the account was removed; nothing to do.
        tx.send(job).ok();
    }
}

impl Job {
    fn message_key(&self) -> Option<(i64, u32)> {
        match *self {
            Job::SetFlag { folder_id, uid, .. }
            | Job::Move { folder_id, uid, .. }
            | Job::Delete { folder_id, uid } => Some((folder_id, uid)),
            _ => None,
        }
    }
}

struct Worker {
    account: Account,
    db_path: PathBuf,
    notify: Notify,
    imap: Option<Imap>,
    pending: Pending,
}

impl Worker {
    fn run(&mut self, rx: Receiver<Job>) {
        let mut db = match Db::open(&self.db_path) {
            Ok(db) => db,
            Err(e) => {
                (self.notify)(Event::ActionFailed {
                    account_id: self.account.id,
                    error: format!("{e:#}"),
                });
                return;
            }
        };
        'jobs: while let Ok(job) = rx.recv() {
            let job = match job {
                // Coalesce queued sync requests into one pass.
                Job::SyncAll => loop {
                    match rx.try_recv() {
                        Ok(Job::SyncAll) | Ok(Job::SyncFolder(_)) => continue,
                        Ok(other) => {
                            self.handle(&mut db, Job::SyncAll);
                            break other;
                        }
                        Err(TryRecvError::Empty) => break Job::SyncAll,
                        // Account removed: stop, but still log out below.
                        Err(TryRecvError::Disconnected) => break 'jobs,
                    }
                },
                other => other,
            };
            self.handle(&mut db, job);
        }
        if let Some(imap) = self.imap.take() {
            imap.logout();
        }
    }

    fn handle(&mut self, db: &mut Db, job: Job) {
        let key = job.message_key();
        self.run_job(db, job);
        if let Some(key) = key {
            self.pending.done(key);
        }
    }

    fn run_job(&mut self, db: &mut Db, job: Job) {
        let id = self.account.id;
        match job {
            Job::SyncAll => {
                (self.notify)(Event::Syncing {
                    account_id: id,
                    active: true,
                    error: None,
                });
                let error = self.sync_all(db).err().map(|e| format!("{e:#}"));
                (self.notify)(Event::Syncing {
                    account_id: id,
                    active: false,
                    error,
                });
            }
            Job::SyncFolder(folder_id) => {
                let result = db.folder(folder_id).and_then(|f| {
                    let f = f.ok_or_else(|| anyhow!("folder no longer exists"))?;
                    let pending = self.pending.clone();
                    self.with_imap(|imap, notify| sync_folder(imap, db, &f, notify, &pending))
                });
                if let Err(e) = result {
                    self.fail(e);
                }
            }
            Job::FetchBody(message_id) => {
                if let Err(e) = self.fetch_body(db, message_id) {
                    (self.notify)(Event::BodyFailed {
                        message_id,
                        error: format!("{e:#}"),
                    });
                }
            }
            Job::SetFlag {
                folder_id,
                uid,
                flag,
                on,
            } => {
                let result = folder_name(db, folder_id).and_then(|folder| {
                    self.with_imap(|imap, _| imap.set_flag(&folder, uid, flag, on))
                });
                if let Err(e) = result {
                    self.fail(e);
                }
            }
            Job::Move {
                folder_id,
                uid,
                dest,
            } => {
                let result = folder_name(db, folder_id).and_then(|folder| {
                    let dest_name = folder_name(db, dest)?;
                    self.with_imap(|imap, _| imap.move_message(&folder, uid, &dest_name))
                });
                match result {
                    Ok(()) => (self.notify)(Event::NeedsSync { folder_id: dest }),
                    Err(e) => {
                        self.fail(e);
                        // Bring the message back if the server kept it.
                        (self.notify)(Event::NeedsSync { folder_id });
                    }
                }
            }
            Job::Delete { folder_id, uid } => {
                let result = folder_name(db, folder_id)
                    .and_then(|folder| self.with_imap(|imap, _| imap.delete_message(&folder, uid)));
                if let Err(e) = result {
                    self.fail(e);
                    (self.notify)(Event::NeedsSync { folder_id });
                }
            }
            Job::Send { outgoing, reply_to } => {
                let sent_folder = if self.account.save_sent {
                    db.folder_by_role(id, Role::Sent).ok().flatten()
                } else {
                    None
                };
                let result = self
                    .send(db, *outgoing, reply_to, sent_folder.as_ref())
                    .map_err(|e| format!("{e:#}"));
                (self.notify)(Event::Sent {
                    result,
                    sent_folder: sent_folder.map(|f| f.id),
                });
            }
        }
    }

    fn fail(&self, e: anyhow::Error) {
        (self.notify)(Event::ActionFailed {
            account_id: self.account.id,
            error: format!("{e:#}"),
        });
    }

    /// Run `f` on a live connection, reconnecting once if the old
    /// connection went stale (servers drop idle sessions).
    fn with_imap<T>(&mut self, mut f: impl FnMut(&mut Imap, &Notify) -> Result<T>) -> Result<T> {
        if let Some(imap) = self.imap.as_mut() {
            match f(imap, &self.notify) {
                Ok(v) => return Ok(v),
                Err(e) if !is_connection_error(&e) => return Err(e),
                Err(_) => self.imap = None,
            }
        }
        let password = secrets::get_password(self.account.id)?;
        let imap = self.imap.insert(Imap::connect(
            &self.account.imap,
            &self.account.email,
            &password,
        )?);
        f(imap, &self.notify)
    }

    fn sync_all(&mut self, db: &mut Db) -> Result<()> {
        let id = self.account.id;
        let remote = self.with_imap(|imap, _| imap.list_folders())?;
        let mut keep = Vec::new();
        for r in &remote {
            keep.push(db.upsert_folder(id, &r.name, &r.display, r.depth, r.role)?);
        }
        db.retain_folders(id, &keep)?;
        (self.notify)(Event::FolderChanged { folder_id: None });

        let folders: Vec<Folder> = db
            .folders()?
            .into_iter()
            .filter(|f| f.account_id == id)
            .collect();
        let mut errors = Vec::new();
        let pending = self.pending.clone();
        for f in &folders {
            if let Err(e) =
                self.with_imap(|imap, notify| sync_folder(imap, db, f, notify, &pending))
            {
                errors.push(format!("{}: {e:#}", f.display));
            }
        }
        // Prefetch once every folder is listed, so other folders never wait.
        for f in folders.iter().filter(|f| f.role == Role::Inbox) {
            self.prefetch(db, f).ok();
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(errors.join("; ")))
        }
    }

    fn prefetch(&mut self, db: &mut Db, folder: &Folder) -> Result<()> {
        let missing = db.missing_bodies(folder.id, PREFETCH_COUNT, PREFETCH_MAX_SIZE)?;
        for chunk in missing.chunks(25) {
            let uids: Vec<u32> = chunk.iter().map(|(_, uid)| *uid).collect();
            let bodies = self.with_imap(|imap, _| imap.fetch_bodies(&folder.name, &uids))?;
            for (uid, raw) in bodies {
                if let Some((id, _)) = chunk.iter().find(|(_, u)| *u == uid) {
                    store_body(db, *id, &raw)?;
                }
            }
            (self.notify)(Event::FolderChanged {
                folder_id: Some(folder.id),
            });
        }
        Ok(())
    }

    fn fetch_body(&mut self, db: &mut Db, message_id: i64) -> Result<()> {
        let m = db
            .message(message_id)?
            .ok_or_else(|| anyhow!("message is gone"))?;
        if m.raw.is_none() {
            let folder = folder_name(db, m.folder_id)?;
            let mut bodies = self.with_imap(|imap, _| imap.fetch_bodies(&folder, &[m.uid]))?;
            let (_, raw) = bodies
                .pop()
                .ok_or_else(|| anyhow!("the server no longer has this message"))?;
            store_body(db, message_id, &raw)?;
        }
        (self.notify)(Event::BodyReady { message_id });
        Ok(())
    }

    fn send(
        &mut self,
        db: &mut Db,
        out: smtp::Outgoing,
        reply_to: Option<i64>,
        sent_folder: Option<&Folder>,
    ) -> Result<()> {
        let msg = smtp::build(out)?;
        let password = secrets::get_password(self.account.id)?;
        smtp::send(&self.account.smtp, &self.account.email, &password, &msg)?;
        // The message is out; failures below are reported but do not undo it.
        if let Some(folder) = sent_folder {
            let raw = msg.formatted();
            self.with_imap(|imap, _| imap.append(&folder.name, &raw))
                .context("sent, but saving a copy to Sent failed")?;
        }
        if let Some(original) = reply_to
            && let Some(m) = db.message(original)?
        {
            db.set_answered(original)?;
            let folder = folder_name(db, m.folder_id)?;
            self.with_imap(|imap, _| imap.set_flag(&folder, m.uid, "\\Answered", true))
                .ok();
        }
        Ok(())
    }
}

fn sync_folder(
    imap: &mut Imap,
    db: &mut Db,
    f: &Folder,
    notify: &Notify,
    pending: &Pending,
) -> Result<()> {
    let sel = imap.select(&f.name)?;
    let changed = |notify: &Notify| {
        notify(Event::FolderChanged {
            folder_id: Some(f.id),
        })
    };
    if db.set_uidvalidity(f.id, sel.uidvalidity)? {
        changed(notify);
    }
    let remote = if sel.exists == 0 {
        HashMap::new()
    } else {
        imap.uid_flags()?
    };
    let known = db.known_uids(f.id)?;
    let gone: Vec<u32> = known
        .iter()
        .filter(|u| !remote.contains_key(u))
        .copied()
        .collect();
    if !gone.is_empty() {
        db.delete_uids(f.id, &gone)?;
        changed(notify);
    }
    let mut new: Vec<u32> = remote
        .keys()
        .filter(|u| !known.contains(u))
        .copied()
        .collect();
    new.sort_unstable_by(|a, b| b.cmp(a)); // newest first
    for chunk in new.chunks(HEADER_BATCH) {
        let mut headers = imap.fetch_headers(chunk)?;
        // A message the user moved or deleted locally still exists on the
        // server until the interactive worker gets to it; don't re-add it.
        let busy = pending.uids(f.id);
        headers.retain(|h| !busy.contains(&h.uid));
        db.insert_headers(f.account_id, f.id, &headers)?;
        changed(notify);
    }
    if !known.is_empty() && !remote.is_empty() {
        let busy = pending.uids(f.id);
        if db.apply_flags(f.id, &remote, &busy)? > 0 {
            changed(notify);
        }
    }
    Ok(())
}

fn store_body(db: &mut Db, id: i64, raw: &[u8]) -> Result<()> {
    let body = mime::parse_body(raw);
    db.set_body(
        id,
        raw,
        &mime::preview(&body.text),
        !body.attachments.is_empty(),
    )
}

fn folder_name(db: &Db, folder_id: i64) -> Result<String> {
    Ok(db
        .folder(folder_id)?
        .ok_or_else(|| anyhow!("folder no longer exists"))?
        .name)
}

fn is_connection_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>().is_some()
            || c.downcast_ref::<::imap::Error>().is_some_and(|ie| {
                matches!(ie, ::imap::Error::Io(_) | ::imap::Error::ConnectionLost)
            })
    })
}

/// Check that an account's settings and password work before saving it.
pub fn verify(imap_server: &Server, username: &str, password: &str) -> Result<()> {
    let mut imap = Imap::connect(imap_server, username, password)?;
    imap.noop()?;
    imap.logout();
    Ok(())
}
