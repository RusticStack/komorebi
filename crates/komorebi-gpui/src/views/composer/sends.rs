//! The sends in their undo window, shared by every composer of the app (the
//! Slint app's App-level `pending_sends`, `src/app/composer.rs` `send_draft`,
//! `count_down` and `promote`).
//!
//! A draft tab has a composer of its own, and the tab (with its composer)
//! goes as soon as Send closes it. So a send's build, its undo window and
//! its hand-over to the send worker do not belong to the composer that
//! started it: they run here, on the app, and what they have to say
//! (`Queued`, a failed build) is told through the *home* composer, the
//! first one made that is still there (the mail view's own, made with the
//! window, before any tab). Undo works from any composer, for any send.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, AsyncApp, EntityId, Global, WeakEntity};
use komorebi::compose::{BuildJob, PendingSend};

use super::{ComposerEvent, ComposerView, Status};
use crate::views::Db;

/// One send in its undo window: its state, the cache it is in, and the
/// composer that started it.
pub(crate) struct Entry {
    pub(crate) send: PendingSend,
    pub(crate) db: Db,
    pub(crate) origin: EntityId,
}

#[derive(Default)]
struct Desk {
    pending: HashMap<i64, Entry>,
    seq: u64,
    /// Every composer made, oldest first (weak: a closed tab's is gone).
    composers: Vec<WeakEntity<ComposerView>>,
}

/// The app's sends (cheap to clone: one shared desk).
#[derive(Clone, Default)]
pub(crate) struct Sends(Rc<RefCell<Desk>>);

#[derive(Default)]
struct SendsGlobal(Sends);

impl Global for SendsGlobal {}

/// What a finished build or undo window leads to (worked out under the
/// desk's borrow, done after it).
enum Next {
    Nothing,
    Promote(Entry),
    Failed(Entry, String),
}

impl Sends {
    /// The app's desk.
    pub(crate) fn of(cx: &mut App) -> Sends {
        cx.default_global::<SendsGlobal>().0.clone()
    }

    /// A composer was made.
    pub(crate) fn register(&self, composer: WeakEntity<ComposerView>) {
        let mut d = self.0.borrow_mut();
        d.composers.retain(|c| c.upgrade().is_some());
        d.composers.push(composer);
    }

    /// The composer that tells the shell about sends whose own composer
    /// may be gone.
    pub(crate) fn home(&self) -> Option<WeakEntity<ComposerView>> {
        self.0
            .borrow()
            .composers
            .iter()
            .find(|c| c.upgrade().is_some())
            .cloned()
    }

    /// The composer of a draft tab or a composer window (not `except`)
    /// showing saved draft `row`.
    pub(crate) fn holder(
        &self,
        row: i64,
        except: EntityId,
        cx: &App,
    ) -> Option<WeakEntity<ComposerView>> {
        let composers = self.0.borrow().composers.clone();
        composers.into_iter().find(|weak| {
            weak.entity_id() != except
                && weak.upgrade().is_some_and(|c| {
                    let c = c.read(cx);
                    c.own_place && c.composing && c.drafts.find_row(row).is_some()
                })
        })
    }

    /// The outbox rows in their undo window.
    pub(crate) fn rows(&self) -> Vec<i64> {
        let mut rows: Vec<i64> = self.0.borrow().pending.keys().copied().collect();
        rows.sort_unstable();
        rows
    }

    /// The composer that started the send of `row`, while it waits.
    pub(crate) fn origin(&self, row: i64) -> Option<EntityId> {
        self.0.borrow().pending.get(&row).map(|e| e.origin)
    }

    /// Undo's flag first: a build finishing now stores nothing.
    pub(crate) fn cancel(&self, row: i64) {
        if let Some(e) = self.0.borrow().pending.get(&row) {
            e.send.cancel();
        }
    }

    pub(crate) fn remove(&self, row: i64) {
        self.0.borrow_mut().pending.remove(&row);
    }

    /// Hold `entry` (its `seq` is set here) and return its number.
    fn add(&self, row: i64, mut entry: Entry) -> u64 {
        let mut d = self.0.borrow_mut();
        d.seq += 1;
        entry.send.seq = d.seq;
        let seq = d.seq;
        d.pending.insert(row, entry);
        seq
    }

    /// Hold the send of outbox row `row` (`entry`), build `job` off the UI thread,
    /// and once `delay` (the undo window) has passed and the build is done
    /// queue it. Returns the send's number.
    pub(crate) fn start(
        &self,
        row: i64,
        entry: Entry,
        mut job: BuildJob,
        delay: Duration,
        cx: &mut App,
    ) -> u64 {
        let seq = self.add(row, entry);
        job.seq = seq;
        let build = cx.background_spawn(async move { job.run() });
        let sends = self.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = build.await;
            sends.built(row, seq, result, cx);
        })
        .detach();
        if !delay.is_zero() {
            let sends = self.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                cx.background_executor().timer(delay).await; // timer-exempt: the undo-send window the user set
                sends.due(row, seq, cx);
            })
            .detach();
        }
        seq
    }

    /// The undo window of a send ended.
    fn due(&self, row: i64, seq: u64, cx: &mut AsyncApp) {
        let next = {
            let mut d = self.0.borrow_mut();
            match d.pending.get_mut(&row) {
                Some(e) if e.send.seq == seq => {
                    e.send.due = true;
                    if e.send.built {
                        d.pending.remove(&row).map_or(Next::Nothing, Next::Promote)
                    } else {
                        Next::Nothing
                    }
                }
                _ => Next::Nothing,
            }
        };
        self.follow(row, next, cx);
    }

    /// A build ended: `Ok(true)` stored the message, `Ok(false)` found the
    /// send undone. A result for a send undone since is dropped.
    fn built(&self, row: i64, seq: u64, result: Result<bool, String>, cx: &mut AsyncApp) {
        let next = {
            let mut d = self.0.borrow_mut();
            match d.pending.get_mut(&row) {
                Some(e) if e.send.seq == seq && !e.send.cancelled() => match result {
                    Ok(true) => {
                        e.send.built = true;
                        if e.send.due {
                            d.pending.remove(&row).map_or(Next::Nothing, Next::Promote)
                        } else {
                            Next::Nothing
                        }
                    }
                    Ok(false) => {
                        d.pending.remove(&row);
                        Next::Nothing
                    }
                    Err(err) => d
                        .pending
                        .remove(&row)
                        .map_or(Next::Nothing, |e| Next::Failed(e, err)),
                },
                _ => Next::Nothing,
            }
        };
        self.follow(row, next, cx);
    }

    /// Queue a built send past its window, or put a failed one back in
    /// Drafts, and tell the shell.
    fn follow(&self, row: i64, next: Next, cx: &mut AsyncApp) {
        let events = match next {
            Next::Nothing => return,
            // Built and past its undo window: queue the row and hand it to
            // its account's send worker (offline, it waits in the queue).
            Next::Promote(e) => match e.db.outbox_queue(row) {
                Ok(true) => vec![ComposerEvent::Queued {
                    row,
                    account: e.send.account,
                }],
                Ok(false) => return,
                Err(err) => vec![ComposerEvent::Status(Status::Error(format!("{err:#}")))],
            },
            Next::Failed(e, err) => match e.db.outbox_unsend(row) {
                Err(unsend) => vec![ComposerEvent::Status(Status::Error(format!("{unsend:#}")))],
                Ok(_) => vec![
                    ComposerEvent::Status(Status::Error(err)),
                    ComposerEvent::Unsent { row },
                    ComposerEvent::DraftsChanged {
                        reply_to: None,
                        count: true,
                    },
                ],
            },
        };
        if let Some(home) = self.home() {
            let _ = home.update(cx, |c, cx| {
                for event in events {
                    c.relay(event, cx);
                }
            });
        }
    }
}
