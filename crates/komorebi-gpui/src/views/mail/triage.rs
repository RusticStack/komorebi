//! Triage on the list and the open message (the Slint app's `src/app.rs`
//! flags and moves, and `src/app/triage.rs`): read and important flags,
//! Archive, Delete, Move, junk, the held move with its Undo window and
//! clock, the Trash / Junk delete question, the row menu's state, and the
//! Move flyout's folders (recent first, a filter, depth kept).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use komorebi::db::{Brief, Folder};
use komorebi::mail::Job;
use komorebi::role::Role;
use komorebi::tree;

use super::model::{Effect, Mail, Question, Status, View};

/// A held move's undo window at least (it lasts until the next move, or
/// this, whichever is later).
pub const UNDO_WINDOW: Duration = Duration::from_secs(10);
/// What is left of the window once the pointer leaves the offer (or its
/// focus goes) after a pause: at least this.
pub const UNDO_RESUME: Duration = Duration::from_secs(3);
/// How many recently used folders the Move flyout lists first.
const MOVE_RECENT: usize = 3;
/// How many recent folders are kept (of every account).
const MOVE_RECENT_KEPT: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Flagged,
}

impl Flag {
    fn name(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
        }
    }

    fn of(self, m: &Brief) -> bool {
        match self {
            Flag::Seen => m.seen,
            Flag::Flagged => m.flagged,
        }
    }
}

/// Where Archive, Delete and Move send messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    /// The account's Archive folder.
    Archive,
    /// The account's Trash folder; from Trash itself, deleted.
    Trash,
    Folder(i64),
    /// The account's Junk folder.
    Junk,
    /// From Junk back to the Inbox, marked read.
    NotJunk,
}

/// An action on one message only, never opened: a row's quick actions, and
/// the commands of a message tab or window (which act on their message,
/// whatever the list has selected).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quick {
    Archive,
    Delete,
    /// Important on or off (the message's own state turned over).
    Flag,
    /// Read or unread (turned over).
    Read,
    /// Move to this folder (of the message's account; noted as recent).
    Move(i64),
    /// Report junk: to the account's Junk folder.
    Junk,
    /// Not junk: from Junk back to the Inbox, marked read.
    NotJunk,
}

/// What the Undo slot says about a held move (worded in `text`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Removal {
    /// The folder they went to (`None`: deleted).
    pub to: Option<i64>,
    pub count: usize,
    /// Some messages stayed: their account has no folder for this role.
    pub missing: Option<Role>,
}

/// A move, delete or junk report in its undo window: the messages are
/// gone from the list but still in the cache, and their server jobs wait.
pub struct Held {
    pub seq: u64,
    pub ids: HashSet<i64>,
    pub jobs: Vec<(i64, Job)>,
    /// Unread messages among them, by folder (left out of the counts).
    pub unread: HashMap<i64, i64>,
    /// All of them, by folder (left out of the server Drafts' total).
    pub total: HashMap<i64, i64>,
    /// The message that was open, reopened by Undo.
    pub open: Option<i64>,
}

/// The clock of a held move's undo window: due [`UNDO_WINDOW`] after it
/// starts, stopped while the offer is hovered or focused (WCAG 2.2.1), and
/// then due at least [`UNDO_RESUME`] after it goes on.
#[derive(Debug, Default)]
pub struct UndoClock {
    due: Option<Instant>,
    /// Stopped, with this much left.
    paused: Option<Duration>,
}

impl UndoClock {
    /// A new window from `now`: how long until it is due.
    pub fn start(&mut self, now: Instant) -> Duration {
        self.due = Some(now + UNDO_WINDOW);
        self.paused = None;
        UNDO_WINDOW
    }

    /// The offer is hovered or focused: the clock stops.
    pub fn pause(&mut self, now: Instant) {
        if let Some(due) = self.due.take() {
            self.paused = Some(due.saturating_duration_since(now));
        }
    }

    /// The pointer left (or the focus went): how long until it is due now,
    /// if it was stopped.
    pub fn resume(&mut self, now: Instant) -> Option<Duration> {
        let left = self.paused.take()?.max(UNDO_RESUME);
        self.due = Some(now + left);
        Some(left)
    }

    /// The window is over at `now` (never while stopped).
    pub fn is_due(&self, now: Instant) -> bool {
        self.due.is_some_and(|due| now >= due)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Messages grouped by (account, folder): one server job each.
pub fn by_folder<'a>(msgs: &[&'a Brief]) -> BTreeMap<(i64, i64), Vec<&'a Brief>> {
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for &m in msgs {
        groups
            .entry((m.account_id, m.folder_id))
            .or_default()
            .push(m);
    }
    groups
}

/// The recent folders setting (`move_recent`: folder ids, the last used
/// first).
pub fn parse_recent(value: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = Vec::new();
    for id in value
        .split(',')
        .filter_map(|t| t.trim().parse::<i64>().ok())
    {
        if id > 0 && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// `id` used now: first in `recent`, which keeps at most
/// [`MOVE_RECENT_KEPT`].
pub fn push_recent(recent: &mut Vec<i64>, id: i64) {
    recent.retain(|&r| r != id);
    recent.insert(0, id);
    recent.truncate(MOVE_RECENT_KEPT);
}

/// A folder the Move flyout offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveOption {
    pub id: i64,
    pub label: String,
    pub level: usize,
    /// In the recent section at the top.
    pub recent: bool,
}

/// The Move flyout's entries: with no filter, the recent folders among
/// `options` first (marked `recent`), then all of them; with a filter, the
/// options whose name holds it (any case), in order.
pub fn move_entries(options: &[MoveOption], recent: &[i64], filter: &str) -> Vec<MoveOption> {
    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        return options
            .iter()
            .filter(|o| o.label.to_lowercase().contains(&filter))
            .cloned()
            .collect();
    }
    let mut out: Vec<MoveOption> = recent
        .iter()
        .filter_map(|id| options.iter().find(|o| o.id == *id))
        .take(MOVE_RECENT)
        .map(|o| MoveOption {
            recent: true,
            ..o.clone()
        })
        .collect();
    out.extend(options.iter().cloned());
    out
}

/// The Move flyout's indent of each node of the tree (in its order): how
/// many of its ancestors are listed (`included`), so a folder whose parent
/// is left out (the current folder, a Drafts folder) sits where that parent
/// would, never indented under nothing. Capped like the pane.
pub fn move_levels(nodes: &[tree::TreeNode], included: &[bool]) -> Vec<usize> {
    // Folder index -> (its level here, listed).
    let mut seen: HashMap<usize, (usize, bool)> = HashMap::new();
    nodes
        .iter()
        .enumerate()
        .map(|(pos, n)| {
            let level = match n.parent.and_then(|p| seen.get(&p)) {
                Some(&(parent, true)) => (parent + 1).min(tree::MAX_LEVEL),
                Some(&(parent, false)) => parent,
                None => 0,
            };
            seen.insert(
                n.index,
                (level, included.get(pos).copied().unwrap_or(false)),
            );
            level
        })
        .collect()
}

/// The folders of `account` among `folders` a message can move to, but
/// `except`: in the folder pane's order, with their depth; never a Drafts
/// folder (the mail view's Move flyout, and a message tab's or window's
/// Move menu, which reads the folders from the cache).
pub fn move_options_in(folders: &[Folder], account: i64, except: &[i64]) -> Vec<MoveOption> {
    let nodes = tree::tree(folders);
    let included: Vec<bool> = nodes
        .iter()
        .map(|n| {
            folders.get(n.index).is_some_and(|f| {
                f.account_id == account && !except.contains(&f.id) && f.role != Role::Drafts
            })
        })
        .collect();
    let levels = move_levels(&nodes, &included);
    nodes
        .iter()
        .zip(levels)
        .zip(included)
        .filter(|(_, included)| *included)
        .filter_map(|((n, level), _)| folders.get(n.index).map(|f| (f, level)))
        .map(|(f, level)| MoveOption {
            id: f.id,
            label: super::text::folder_label(f),
            level,
            recent: false,
        })
        .collect()
}

/// What the row menu (and the reading pane's command bar) can do for the
/// selection or the open message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowMenu {
    /// Exactly one message.
    pub one: bool,
    /// What is acted on lies in a server Drafts folder (no Reply there).
    pub drafts: bool,
    /// Any of them unread: Mark as read, else Mark as unread.
    pub unread: bool,
    /// Any of them not important: Mark as important, else Remove.
    pub unflagged: bool,
    /// All of them in Junk: Not junk, else Report junk.
    pub junk: bool,
    /// Show in folder (while searching).
    pub show_in_folder: bool,
    /// The Move flyout has folders.
    pub can_move: bool,
}

impl Mail {
    // ------------------------------------------------------------ flags

    /// Set a flag on messages in the cache (those without it already),
    /// refresh the open message, and queue the change for the server: one
    /// job per folder.
    pub fn set_flags(&mut self, targets: &[Brief], flag: Flag, on: bool) {
        let changing: Vec<&Brief> = targets.iter().filter(|m| flag.of(m) != on).collect();
        if changing.is_empty() {
            return;
        }
        let ids: Vec<i64> = changing.iter().map(|m| m.id).collect();
        let result = match flag {
            Flag::Seen => self.db.set_seen_many(&ids, on),
            Flag::Flagged => self.db.set_flagged_many(&ids, on),
        };
        if let Err(e) = result {
            self.status = Some(Status::Error(format!("{e:#}")));
            return;
        }
        self.flags_seq += 1;
        for ((account, folder_id), group) in by_folder(&changing) {
            self.jobs.push((
                account,
                Job::SetFlags {
                    folder_id,
                    uids: group.iter().map(|m| m.uid).collect(),
                    flag: flag.name(),
                    on,
                },
            ));
        }
        if let Some(open) = self.open.as_mut().filter(|o| ids.contains(&o.id)) {
            match flag {
                Flag::Seen => {
                    open.seen = on;
                    // Marked by hand: the delayed mark must not undo it.
                    self.read_wait.cancel();
                }
                Flag::Flagged => open.flagged = on,
            }
        }
        if flag == Flag::Seen {
            self.load_counts();
        }
    }

    /// Ctrl+Q and Ctrl+U, the menus' Mark as read / unread.
    pub fn mark(&mut self, flag: Flag, on: bool) {
        let targets = self.targets();
        self.set_flags(&targets, flag, on);
    }

    /// All read if any is unread, else all unread.
    pub fn toggle_read(&mut self) {
        let targets = self.targets();
        let seen = targets.iter().any(|m| !m.seen);
        self.set_flags(&targets, Flag::Seen, seen);
    }

    /// All important if any isn't, else none (Insert, Ctrl+Shift+G).
    pub fn toggle_flag(&mut self) {
        let targets = self.targets();
        let flagged = targets.iter().any(|m| !m.flagged);
        self.set_flags(&targets, Flag::Flagged, flagged);
    }

    /// The list's Mark all as read: every message of the view.
    pub fn mark_all_read(&mut self) {
        if self.lists_messages() {
            let folders = self.view_folders();
            self.mark_folders_read(&folders);
        }
    }

    /// Mark every message of these folders read (whatever view is open):
    /// in the cache at once, then one job per folder (one STORE, up to the
    /// highest UID cached, so mail that arrived since stays unread).
    pub fn mark_folders_read(&mut self, folders: &[i64]) {
        for &folder_id in folders {
            let Some(account) = self.folder(folder_id).map(|f| f.account_id) else {
                continue;
            };
            let (uids, through) = match self.db.mark_folder_read(folder_id) {
                Ok(marked) => marked,
                Err(e) => {
                    self.status = Some(Status::Error(format!("{e:#}")));
                    break;
                }
            };
            if !uids.is_empty() {
                self.jobs.push((
                    account,
                    Job::MarkAllRead {
                        folder_id,
                        through,
                        uids,
                    },
                ));
            }
        }
        // The open message, when it was among them, is read now.
        if let Some(open) = self
            .open
            .as_mut()
            .filter(|o| folders.contains(&o.folder_id))
        {
            open.seen = true;
            self.read_wait.cancel();
        }
        self.flags_seq += 1;
        self.load_counts();
        // Asked for: under Unread the list empties now.
        self.reload_rows();
    }

    // ------------------------------------------------------------ moves

    pub fn archive(&mut self) {
        self.remove_to(Dest::Archive);
    }

    /// Delete: from Trash or Junk after asking, else to Trash.
    pub fn delete(&mut self) {
        let targets = self.targets();
        self.delete_targets(targets);
    }

    pub fn report_junk(&mut self) {
        self.remove_to(Dest::Junk);
    }

    pub fn not_junk(&mut self) {
        self.remove_to(Dest::NotJunk);
    }

    pub fn move_to(&mut self, dest: i64) {
        self.note_recent(dest);
        self.remove_to(Dest::Folder(dest));
    }

    fn remove_to(&mut self, dest: Dest) {
        let targets = self.targets();
        self.move_out(dest, targets);
    }

    /// Delete these messages: from Trash (deleted on the server) or Junk,
    /// after asking (a prompt with no default button); else to Trash.
    pub fn delete_targets(&mut self, targets: Vec<Brief>) {
        if targets.is_empty() {
            return;
        }
        let trash = targets
            .iter()
            .any(|m| self.folder_role(m.folder_id) == Role::Trash);
        let junk = targets
            .iter()
            .any(|m| self.folder_role(m.folder_id) == Role::Junk);
        if trash || junk {
            self.delete_wait = targets.iter().map(|m| m.id).collect();
            self.effects
                .push(Effect::Ask(Question::DeleteFrom { trash }));
            return;
        }
        self.move_out(Dest::Trash, targets);
    }

    /// The Trash / Junk delete was confirmed.
    pub fn delete_confirmed(&mut self) {
        let ids = std::mem::take(&mut self.delete_wait);
        let targets = self.db.briefs(&ids).unwrap_or_default();
        if !targets.is_empty() {
            self.move_out(Dest::Trash, targets);
        }
    }

    /// How many messages the Trash / Junk question is about.
    pub fn pending_deletes(&self) -> usize {
        self.delete_wait.len()
    }

    /// The Trash / Junk delete was cancelled.
    pub fn delete_cancelled(&mut self) {
        self.delete_wait.clear();
    }

    /// A row's quick action, on that message only: it is never opened (so
    /// never marked read by opening), and the selection stays.
    pub fn quick_action(&mut self, id: i64, action: Quick) {
        if self.lists_messages() {
            self.act_on_message(id, action);
        }
    }

    /// A message tab's or window's command, on its message only, whatever
    /// the Mail tab lists. A move is held with its Undo as the list's own
    /// is; the rows and their selection change only in a message list (the
    /// Drafts and Outbox rows are not messages: their ids are another
    /// table's).
    pub fn act_on_message(&mut self, id: i64, action: Quick) {
        let targets = match self.db.briefs(&[id]) {
            Ok(t) if !t.is_empty() => t,
            _ => return,
        };
        match action {
            Quick::Archive => self.move_out(Dest::Archive, targets),
            Quick::Delete => self.delete_targets(targets),
            Quick::Flag => {
                let on = !targets[0].flagged;
                self.set_flags(&targets, Flag::Flagged, on);
            }
            Quick::Read => {
                let on = !targets[0].seen;
                self.set_flags(&targets, Flag::Seen, on);
            }
            Quick::Move(dest) => {
                self.note_recent(dest);
                self.move_out(Dest::Folder(dest), targets);
            }
            Quick::Junk => self.move_out(Dest::Junk, targets),
            Quick::NotJunk => self.move_out(Dest::NotJunk, targets),
        }
    }

    /// Move these messages out of their folders (one job per folder) and
    /// open the message that took the place of the open one. The list
    /// changes at once; the server jobs wait out the undo window. Messages
    /// already where they would go stay.
    pub fn move_out(&mut self, dest: Dest, targets: Vec<Brief>) {
        let mut jobs = Vec::new();
        let mut removed = HashSet::new();
        let mut read_first = Vec::new();
        let mut missing = None;
        let mut place: Option<Option<i64>> = None;
        for ((account, folder_id), group) in by_folder(&targets.iter().collect::<Vec<_>>()) {
            let role_folder = |role| self.db.folder_by_role(account, role).ok().flatten();
            let to = match dest {
                Dest::Archive => match role_folder(Role::Archive) {
                    Some(f) => Some(f.id),
                    None => {
                        missing = Some(Role::Archive);
                        continue;
                    }
                },
                Dest::Trash => match role_folder(Role::Trash) {
                    // From Trash itself: deleted.
                    Some(trash) if trash.id == folder_id => None,
                    Some(trash) => Some(trash.id),
                    None => {
                        missing = Some(Role::Trash);
                        continue;
                    }
                },
                // Never somewhere else instead: without a Junk folder the
                // message stays put.
                Dest::Junk => match role_folder(Role::Junk) {
                    Some(f) => Some(f.id),
                    None => {
                        missing = Some(Role::Junk);
                        continue;
                    }
                },
                Dest::NotJunk => {
                    if self.folder_role(folder_id) != Role::Junk {
                        continue;
                    }
                    match role_folder(Role::Inbox) {
                        Some(f) => {
                            read_first.extend(group.iter().map(|m| **m));
                            Some(f.id)
                        }
                        None => {
                            missing = Some(Role::Inbox);
                            continue;
                        }
                    }
                }
                Dest::Folder(id) => {
                    let same_account = self
                        .folders
                        .iter()
                        .any(|f| f.id == id && f.account_id == account);
                    if !same_account {
                        continue;
                    }
                    Some(id)
                }
            };
            if to == Some(folder_id) {
                continue;
            }
            let uids = group.iter().map(|m| m.uid).collect();
            let job = match to {
                Some(dest) => Job::Move {
                    folder_id,
                    uids,
                    dest,
                },
                None => Job::Delete { folder_id, uids },
            };
            place.get_or_insert(to);
            jobs.push((account, job));
            removed.extend(group.iter().map(|m| m.id));
        }
        // Not junk: marked read at once (not held; nothing is lost by it).
        self.set_flags(&read_first, Flag::Seen, true);
        let Some(to) = place else {
            if let Some(role) = missing {
                self.status = Some(Status::Missing(role));
            }
            return;
        };
        let removal = Removal {
            to,
            count: removed.len(),
            missing,
        };
        self.hold(jobs, removed, removal);
    }

    /// Hide moved or deleted messages and offer Undo; their jobs go to the
    /// server when the undo window ends (or sooner: at the next move, or
    /// when the window closes).
    fn hold(&mut self, jobs: Vec<(i64, Job)>, ids: HashSet<i64>, removal: Removal) {
        self.flush_held();
        let list: Vec<i64> = ids.iter().copied().collect();
        let mut unread = HashMap::new();
        let mut total = HashMap::new();
        for m in self.db.briefs(&list).unwrap_or_default() {
            *total.entry(m.folder_id).or_default() += 1;
            if !m.seen {
                *unread.entry(m.folder_id).or_default() += 1;
            }
        }
        // Drafts and the Outbox list another table's ids: the same number
        // there is not this message, so their rows stay as they are.
        let listed = self.lists_messages();
        // The open message is a message (its id is one) in any view.
        let open = self.open_id().filter(|id| ids.contains(id));
        // What was open, or the whole selection, went (not a quick action
        // on another row): the next row opens in its place. Otherwise the
        // open message and what is left of the selection stay.
        let hit = listed
            && (open.is_some() || {
                let selected = self.rows.selection().ids();
                self.open.is_none()
                    && !selected.is_empty()
                    && selected.iter().all(|id| ids.contains(id))
            });
        let index = if listed {
            self.rows.remove_many(&ids).map(|r| r.first)
        } else {
            None
        };
        if open.is_some() {
            self.open = None;
            self.read_wait.cancel();
        }
        self.held_seq += 1;
        let seq = self.held_seq;
        self.held = Some(Held {
            seq,
            ids,
            jobs,
            unread,
            total,
            open,
        });
        if hit {
            match index.and_then(|i| self.rows.neighbour(i)) {
                Some(id) => {
                    self.select_message(id);
                }
                None => self.close_reader(),
            }
        }
        self.undo = Some(removal);
        let after = self.undo_clock.start(Instant::now());
        self.effects.push(Effect::UndoAfter(seq, after));
    }

    /// The held move `seq`'s timer ran out: it goes to the server if its
    /// window is over at `now` (a paused clock waits for its resume).
    pub fn undo_due(&mut self, seq: u64, now: Instant) {
        if self.held.as_ref().is_some_and(|h| h.seq == seq) && self.undo_clock.is_due(now) {
            self.flush_held();
        }
    }

    /// The pointer is over the Undo offer, or it has the focus (`on`): a
    /// move's window stops, and goes on (for at least [`UNDO_RESUME`])
    /// after.
    pub fn undo_hold(&mut self, on: bool, now: Instant) {
        let Some(seq) = self.held.as_ref().map(|h| h.seq) else {
            return;
        };
        if on {
            self.undo_clock.pause(now);
        } else if let Some(left) = self.undo_clock.resume(now) {
            self.effects.push(Effect::UndoAfter(seq, left));
        }
    }

    /// Send the held jobs to the server now and drop their messages from
    /// the cache.
    pub fn flush_held(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        self.undo_clock.clear();
        self.undo = None;
        let ids: Vec<i64> = held.ids.into_iter().collect();
        // A folder reset during the window took its rows (and the meaning
        // of their UIDs) with it: only UIDs still cached are sent.
        let alive: HashSet<(i64, u32)> = self
            .db
            .briefs(&ids)
            .unwrap_or_default()
            .iter()
            .map(|m| (m.folder_id, m.uid))
            .collect();
        for (account, mut job) in held.jobs {
            if let Job::Move {
                folder_id, uids, ..
            }
            | Job::Delete { folder_id, uids } = &mut job
            {
                uids.retain(|u| alive.contains(&(*folder_id, *u)));
                if uids.is_empty() {
                    continue;
                }
            }
            self.jobs.push((account, job));
        }
        self.db.delete_messages(&ids).ok();
        self.load_counts();
    }

    /// Undo (or Ctrl+Z): the held messages come back to the list (they
    /// never left the cache or the server), selected as they were.
    pub fn undo(&mut self) {
        let Some(held) = self.held.take() else {
            return;
        };
        self.undo_clock.clear();
        self.undo = None;
        self.reload_rows();
        // Drafts and the Outbox list another table's ids: nothing of theirs
        // is selected for a message that comes back.
        if !self.lists_messages() {
            return;
        }
        let listed: Vec<i64> = held
            .ids
            .iter()
            .copied()
            .filter(|&id| self.rows.index_of(id).is_some())
            .collect();
        match held.open.filter(|id| listed.contains(id)) {
            Some(id) => {
                self.select_message(id);
            }
            None if !listed.is_empty() => self.select_rows(|s, _| {
                s.clear();
                for id in listed {
                    s.toggle(id);
                }
            }),
            None => {}
        }
    }

    /// Moves on each time the view sets read or important marks in the
    /// cache (the message tabs and windows read theirs again).
    pub fn flags_seq(&self) -> u64 {
        self.flags_seq
    }

    /// The move held in its undo window.
    pub fn held(&self) -> Option<&Held> {
        self.held.as_ref()
    }

    /// What the Undo slot says.
    pub fn removal(&self) -> Option<Removal> {
        self.undo
    }

    // ------------------------------------------------------ the menus

    /// The row menu for what is selected (or open).
    pub fn row_menu(&self) -> RowMenu {
        let targets = self.targets();
        let all = |role| {
            !targets.is_empty()
                && targets
                    .iter()
                    .all(|m| self.folder_role(m.folder_id) == role)
        };
        RowMenu {
            one: self.one_target(),
            drafts: all(Role::Drafts),
            unread: targets.iter().any(|m| !m.seen),
            unflagged: targets.iter().any(|m| !m.flagged),
            junk: all(Role::Junk),
            show_in_folder: self.searching(),
            can_move: !self.move_choices().is_empty(),
        }
    }

    /// The folders of `account` a message can move to, but `except`: in
    /// the folder pane's order, with their depth; never a Drafts folder.
    pub fn move_options(&self, account: i64, except: &[i64]) -> Vec<MoveOption> {
        move_options_in(&self.folders, account, except)
    }

    /// The folders the open message, or the selection, can move to, before
    /// the flyout's filter (empty: Move is off).
    pub fn move_choices(&self) -> Vec<MoveOption> {
        if let Some(open) = self.open {
            return self.move_options(open.account_id, &[open.folder_id]);
        }
        // The selection moves within one account: the view's, if it has
        // only one (in a search of all folders, the selection's).
        let folders = if self.searching_all() {
            let ids = self.rows.selection().ids();
            let mut folders: Vec<i64> = self
                .db
                .briefs(&ids)
                .unwrap_or_default()
                .iter()
                .map(|m| m.folder_id)
                .collect();
            folders.sort_unstable();
            folders.dedup();
            folders
        } else {
            self.view_folders()
        };
        let mut accounts = self
            .folders
            .iter()
            .filter(|f| folders.contains(&f.id))
            .map(|f| f.account_id);
        let account = accounts.next().filter(|&a| accounts.all(|b| b == a));
        account.map_or_else(Vec::new, |a| self.move_options(a, &folders))
    }

    /// The folders message `row` alone could move to (its account's, never
    /// the one it is in): a drag of a row outside the selection checks
    /// these before it touches the selection.
    pub fn drop_choices(&self, row: i64) -> Vec<MoveOption> {
        self.db
            .briefs(&[row])
            .ok()
            .and_then(|b| b.first().map(|m| (m.account_id, m.folder_id)))
            .map_or_else(Vec::new, |(account, folder)| {
                self.move_options(account, &[folder])
            })
    }

    /// The Move flyout's entries under its filter.
    pub fn move_targets(&self, filter: &str) -> Vec<MoveOption> {
        move_entries(&self.move_choices(), &self.move_recent, filter)
    }

    /// The Move menu of one message in `folder` of `account` (a message tab
    /// or window): the pane's entries with no filter, recent ones first.
    pub fn message_move_targets(&self, account: i64, folder: i64) -> Vec<MoveOption> {
        move_entries(
            &self.move_options(account, &[folder]),
            &self.move_recent,
            "",
        )
    }

    /// A folder was used as a Move target: first among the recent ones
    /// (saved; a failed write only loses the order).
    pub fn note_recent(&mut self, id: i64) {
        push_recent(&mut self.move_recent, id);
        let value = self
            .move_recent
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.db.set_setting("move_recent", &value).ok();
    }

    /// "Show in folder" (the row menu while searching): the search clears
    /// and the message opens in its own folder, among its neighbours.
    pub fn show_in_folder(&mut self) {
        if !self.lists_messages() {
            return;
        }
        let [target] = self.targets()[..] else {
            return;
        };
        self.search.clear();
        self.search_all = false;
        let view = View::Folder(target.folder_id);
        if self.view == view {
            self.reload_rows();
        } else {
            self.set_view(view);
        }
        self.select_message(target.id);
    }

    /// Ctrl+Shift+I: All inboxes with two or more accounts, else the
    /// account's Inbox.
    pub fn go_inbox(&mut self) {
        if self.accounts.len() > 1 {
            return self.set_view(View::AllInboxes);
        }
        if let Some(inbox) = self.folders.iter().find(|f| f.role == Role::Inbox) {
            let id = inbox.id;
            self.set_view(View::Folder(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `the_undo_clock_waits_ten_seconds_and_stops_while_held`
    /// (src/app/triage.rs).
    #[test]
    fn the_undo_clock_waits_ten_seconds_and_stops_while_held() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut c = UndoClock::default();
        assert!(!c.is_due(t0 + s(60)), "nothing held");
        assert_eq!(c.start(t0), s(10));
        assert!(!c.is_due(t0 + Duration::from_millis(9_999)));
        assert!(c.is_due(t0 + s(10)));
        // Hovered at 9 s: never due while it lasts.
        c.pause(t0 + s(9));
        assert!(!c.is_due(t0 + s(600)));
        // Left at 600 s: 1 s was left, but it gets 3 s.
        assert_eq!(c.resume(t0 + s(600)), Some(s(3)));
        assert!(!c.is_due(t0 + s(602)));
        assert!(c.is_due(t0 + s(603)));
        // Hovered early: the rest of the window stays.
        c.start(t0);
        c.pause(t0 + s(2));
        assert_eq!(c.resume(t0 + s(30)), Some(s(8)));
        // Resume without a pause changes nothing; clear ends it.
        assert_eq!(c.resume(t0 + s(31)), None);
        c.clear();
        assert!(!c.is_due(t0 + s(600)));
    }

    fn option(id: i64, label: &str, level: usize) -> MoveOption {
        MoveOption {
            id,
            label: label.into(),
            level,
            recent: false,
        }
    }

    /// Port of `move_entries_list_recent_folders_first_and_filter_by_name`
    /// (src/app/triage.rs).
    #[test]
    fn move_entries_list_recent_folders_first_and_filter_by_name() {
        let options = [
            option(1, "Inbox", 0),
            option(2, "Archive", 0),
            option(3, "Projects", 0),
            option(4, "Komorebi", 1),
            option(5, "Receipts", 0),
        ];
        let got = move_entries(&options, &[4, 99, 2, 5, 1], "");
        let ids: Vec<(i64, bool)> = got.iter().map(|o| (o.id, o.recent)).collect();
        assert_eq!(
            ids,
            [
                (4, true),
                (2, true),
                (5, true),
                (1, false),
                (2, false),
                (3, false),
                (4, false),
                (5, false)
            ]
        );
        // The depth stays (the recent copies are drawn flush).
        assert_eq!(got[0].level, 1);
        // A filter: any case, no recent section.
        let got = move_entries(&options, &[4], " PRO");
        assert_eq!(got.iter().map(|o| o.id).collect::<Vec<_>>(), [3]);
        assert!(move_entries(&options, &[], "zz").is_empty());
    }

    /// Port of `move_levels_collapse_the_ancestors_left_out`
    /// (src/app/triage.rs).
    #[test]
    fn move_levels_collapse_the_ancestors_left_out() {
        let node = |index, parent, depth| tree::TreeNode {
            index,
            level: depth,
            depth,
            parent,
            has_children: false,
        };
        // Inbox > Receipts > 2024; Projects > Alpha.
        let nodes = [
            node(0, None, 0),
            node(1, Some(0), 1),
            node(2, Some(1), 2),
            node(3, None, 0),
            node(4, Some(3), 1),
        ];
        // Everything listed: the tree's own depths.
        assert_eq!(move_levels(&nodes, &[true; 5]), [0, 1, 2, 0, 1]);
        // The Inbox is the current folder: its children come up a step.
        assert_eq!(
            move_levels(&nodes, &[false, true, true, true, true]),
            [0, 0, 1, 0, 1]
        );
        // A middle ancestor left out: its child sits where it would.
        assert_eq!(
            move_levels(&nodes, &[true, false, true, true, true]),
            [0, 1, 1, 0, 1]
        );
        // A short `included` lists nothing more.
        assert_eq!(move_levels(&nodes, &[true]), [0, 1, 1, 0, 0]);
    }

    /// Port of `recent_folders_round_trip` (src/app/triage.rs).
    #[test]
    fn recent_folders_round_trip() {
        let mut r = parse_recent(" 3, x,5,3,-1,");
        assert_eq!(r, [3, 5]);
        push_recent(&mut r, 5);
        assert_eq!(r, [5, 3]);
        for id in 10..30 {
            push_recent(&mut r, id);
        }
        assert_eq!(r.len(), MOVE_RECENT_KEPT);
        assert_eq!(r[0], 29);
    }

    /// Port of `batches_are_one_job_per_folder` (src/app.rs).
    #[test]
    fn batches_are_one_job_per_folder() {
        let brief = |id: i64, account_id, folder_id| Brief {
            id,
            account_id,
            folder_id,
            uid: id as u32,
            seen: false,
            flagged: false,
        };
        // 200 messages from two inboxes of two accounts, interleaved.
        let msgs: Vec<Brief> = (1..=200)
            .map(|id| match id % 4 {
                0 => brief(id, 2, 20),
                _ => brief(id, 1, 10),
            })
            .collect();
        let refs: Vec<&Brief> = msgs.iter().collect();
        let groups = by_folder(&refs);
        assert_eq!(
            groups.keys().copied().collect::<Vec<_>>(),
            [(1, 10), (2, 20)]
        );
        assert_eq!(groups[&(1, 10)].len(), 150);
        assert_eq!(groups[&(2, 20)].len(), 50);
        assert!(groups[&(2, 20)].iter().all(|m| m.uid % 4 == 0));
        assert!(by_folder(&[]).is_empty());
    }
}
