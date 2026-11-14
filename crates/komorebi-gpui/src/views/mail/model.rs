//! The mail view's state, without GPUI (docs/GPUI-TESTING.md: controller
//! logic in a state struct, tested with plain `#[test]`s): the view shown,
//! the filter and the search, the list's rows and selection, the open
//! message, and what an action asks of the window ([`Effect`]). The Slint
//! app's controller (`src/app.rs`, `src/app/triage.rs`) is the source of
//! every rule here; triage is in `triage.rs`, the folder pane in `nav.rs`.
//!
//! The library does the work: the cache (`komorebi::db`), the lazy rows and
//! the selection (`komorebi::rows`), the folder tree and folds
//! (`komorebi::tree`). Changes for the server are kept as jobs
//! ([`Mail::take_jobs`]) for the account workers, which the GPUI app does
//! not start yet (docs/GPUI-MIGRATION.md phase 2, not done).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use komorebi::db::{Account, Brief, Filter, Folder, MarkRead};
use komorebi::i18n::Lang;
use komorebi::mail::Job;
use komorebi::role::Role;
use komorebi::rows::{Kind, Rows, Selection};
use komorebi::tree::{self, Fold};

use super::triage::{Held, Removal, UndoClock};
use crate::views::Db;

/// Searching a scope larger than this waits for a pause in typing.
pub const SEARCH_DEBOUNCE_ABOVE: u32 = 5_000;
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(crate::theme::motion::SEARCH_PAUSE_MS);

/// What the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    AllInboxes,
    Folder(i64),
    /// The saved local drafts.
    Drafts,
    /// Sent messages until their send settles.
    Outbox,
}

impl View {
    /// The `last_view` setting.
    pub fn key(self) -> String {
        match self {
            View::AllInboxes => "all".into(),
            View::Folder(id) => format!("f{id}"),
            View::Drafts => "drafts".into(),
            View::Outbox => "outbox".into(),
        }
    }
}

/// The view a start opens on: the one saved in `last` (`last_view`) while
/// it still exists (a folder of an account still here, the Outbox while it
/// holds mail, All inboxes while there is more than one account), else the
/// Inbox: All inboxes, or with one account that account's Inbox.
pub fn start_view(last: &str, accounts: &[Account], folders: &[Folder], outbox: i64) -> View {
    let inbox = || match accounts {
        [one] => folders
            .iter()
            .find(|f| f.account_id == one.id && f.role == Role::Inbox)
            .map_or(View::AllInboxes, |f| View::Folder(f.id)),
        _ => View::AllInboxes,
    };
    match last {
        _ if accounts.is_empty() => View::AllInboxes,
        "drafts" => View::Drafts,
        "outbox" if outbox > 0 => View::Outbox,
        "all" if accounts.len() > 1 => View::AllInboxes,
        f => f
            .strip_prefix('f')
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|&id| {
                folders
                    .iter()
                    .any(|x| x.id == id && accounts.iter().any(|a| a.id == x.account_id))
            })
            .map_or_else(inbox, View::Folder),
    }
}

/// Whether a search over `scope` cached messages waits for a typing pause.
pub fn debounces_search(scope: u32, text: &str) -> bool {
    scope > SEARCH_DEBOUNCE_ABOVE && !text.trim().is_empty()
}

/// An account's last sync (from its workers; the GPUI app has none yet, so
/// the list never says it is loading).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountState {
    Syncing,
    /// Unix seconds of the last sync that went through.
    Checked(i64),
    /// The server could not be reached.
    Unreachable,
    Failed,
}

/// What the accounts behind an empty list are doing.
#[derive(Default, Clone, Debug)]
pub struct ListState {
    /// A sync is running.
    pub syncing: bool,
    /// A shown account's last sync failed.
    pub failed: Option<AccountState>,
    /// A shown account is in its first download: (done, total), (0, 0)
    /// before the first count.
    pub fresh: Option<(usize, usize)>,
    /// The folder's account has completed a sync pass.
    pub synced: bool,
}

/// What an empty list says (typed; `text::empty` words it): no match for
/// the search, no drafts, an empty outbox, nothing under the filter, a new
/// account's first download, still loading, a sync that failed, an empty
/// folder once a sync pass has completed, or nothing (yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Empty {
    NoMatches,
    NoDrafts,
    Outbox,
    NoUnread,
    NoFlagged,
    DownloadingStart,
    Downloading(usize, usize),
    Loading,
    Unreachable,
    Failed,
    Folder,
    Nothing,
}

/// The Slint app's `empty_text` choice, before the words.
pub fn empty_kind(view: View, search: &str, filter: Filter, state: &ListState) -> Empty {
    match filter {
        _ if !search.trim().is_empty() => Empty::NoMatches,
        _ if view == View::Drafts => Empty::NoDrafts,
        _ if view == View::Outbox => Empty::Outbox,
        Filter::Unread => Empty::NoUnread,
        Filter::Flagged => Empty::NoFlagged,
        Filter::All => match (state.fresh, &state.failed) {
            (Some((_, 0)), _) => Empty::DownloadingStart,
            (Some((done, total)), _) => Empty::Downloading(done.min(total), total),
            _ if state.syncing => Empty::Loading,
            (_, Some(AccountState::Unreachable)) => Empty::Unreachable,
            (_, Some(AccountState::Failed)) => Empty::Failed,
            _ if state.synced => Empty::Folder,
            _ => Empty::Nothing,
        },
    }
}

/// The opened message waiting to be marked read: one at a time, so a
/// message passed on the way (arrowing through the list) stays unread.
#[derive(Debug, Default)]
pub struct ReadWait(Option<i64>);

impl ReadWait {
    /// Message `id` opened: how long until it is marked read (zero: at
    /// once), or `None` when it isn't (already read, or `Never`).
    pub fn opened(&mut self, id: i64, unread: bool, mode: MarkRead) -> Option<Duration> {
        self.0 = None;
        match mode {
            _ if !unread => None,
            MarkRead::Now => Some(Duration::ZERO),
            MarkRead::After(delay) => {
                self.0 = Some(id);
                Some(delay)
            }
            MarkRead::Never => None,
        }
    }

    pub fn cancel(&mut self) {
        self.0 = None;
    }

    /// The delay ran out with `open` showing: the message to mark read,
    /// if it is still the one that waited.
    pub fn due(&mut self, open: Option<i64>) -> Option<i64> {
        self.0.take().filter(|&id| Some(id) == open)
    }
}

/// What a typed status line says (`text::status` words it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The account has no folder for this role (Archive, Trash, Junk,
    /// Inbox), so nothing moved.
    Missing(Role),
    /// Undo brought a message back whose window had closed.
    UndoWindow,
    /// The cache failed: the error's own words.
    Error(String),
}

/// A question the window asks before acting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Question {
    /// Delete the messages waiting from Trash (for good on the server) or,
    /// `trash` false, from Junk (to Trash).
    DeleteFrom { trash: bool },
}

/// What an action asks of the window (timers, the composer, the focus).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Mark message `id` read after this long, if it is still open then.
    ReadAfter(i64, Duration),
    /// The held move `seq` is due after this long (its undo window).
    UndoAfter(u64, Duration),
    /// Run the search once typing pauses (its sequence number).
    SearchAfter(u64, Duration),
    /// Open the saved draft (a `drafts` row) in the composer.
    OpenDraft(i64),
    /// The composer closes (another row opened).
    CloseComposer,
    /// Ask this question.
    Ask(Question),
    /// The list takes the keyboard focus.
    FocusList,
}

/// What Esc did in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escaped {
    ClearedSearch,
    ClosedStatus,
    LeftSelectMode,
    ClosedMessage,
    Nothing,
}

pub struct Mail {
    pub(super) db: Db,
    pub(super) accounts: Vec<Account>,
    pub(super) folders: Vec<Folder>,
    /// Unread messages by folder (the cache's count, before held moves).
    pub(super) unread: HashMap<i64, i64>,
    pub(super) draft_count: i64,
    pub(super) outbox_count: i64,
    /// The folder pane's folded accounts and parent folders (`nav_folded`).
    pub(super) folded: HashSet<Fold>,
    /// The accounts whose folders have had their default folds
    /// (`nav_folds_init`).
    pub(super) folds_init: HashSet<i64>,
    /// The default folds are applied but not yet written (after the first
    /// frame, docs/PERF.md 6.3).
    pub(super) folds_unsaved: bool,
    /// The folder pane is folded to the rail at every width
    /// (`nav_collapsed`).
    pub(super) nav_collapsed: bool,
    pub(super) view: View,
    pub(super) search: String,
    pub(super) search_seq: u64,
    pub(super) filter: Filter,
    /// The "All folders" search scope (it applies while searching).
    pub(super) search_all: bool,
    pub(super) rows: Rows,
    /// The open message.
    pub(super) open: Option<Brief>,
    /// A saved draft open in the composer (Drafts view).
    pub(super) draft_open: Option<i64>,
    pub(super) mark_read: MarkRead,
    pub(super) read_wait: ReadWait,
    /// "Select messages": the rows show check boxes.
    pub(super) select_mode: bool,
    /// Messages opened in the Unread filter: they stay listed (read by
    /// now) until the view or the filter changes, as in Outlook.
    pub(super) kept_unread: HashSet<i64>,
    /// The last move or delete, while it can be undone.
    pub(super) held: Option<Held>,
    pub(super) held_seq: u64,
    /// Counts the read and important marks the view set in the cache: the
    /// message tabs and windows follow when it moves on.
    pub(super) flags_seq: u64,
    /// What the Undo slot says about the held move.
    pub(super) undo: Option<Removal>,
    pub(super) undo_clock: UndoClock,
    /// The messages a confirmed Trash / Junk delete applies to.
    pub(super) delete_wait: Vec<i64>,
    /// The folders Move went to last, the last first (`move_recent`).
    pub(super) move_recent: Vec<i64>,
    pub(super) status: Option<Status>,
    /// Each account's last sync (none until the GPUI app has workers).
    pub(super) account_state: HashMap<i64, AccountState>,
    /// Accounts in their first download: (done, total) (the hub's).
    pub(super) first_downloads: HashMap<i64, (usize, usize)>,
    /// Changes for the server, by account, for the workers.
    pub(super) jobs: Vec<(i64, Job)>,
    pub(super) effects: Vec<Effect>,
}

impl Mail {
    /// The mail view over the cache: its accounts, folders and settings,
    /// and the list of the view saved last.
    pub fn new(db: Db) -> Self {
        let settings = db.settings().unwrap_or_default();
        // Draft rows show the text typed above the quote, as the Slint app.
        let rows = Rows::new(db.clone()).with_above_quote(komorebi::compose::above_quote);
        let mut mail = Mail {
            db,
            accounts: Vec::new(),
            folders: Vec::new(),
            unread: HashMap::new(),
            draft_count: 0,
            outbox_count: 0,
            folded: tree::parse_folds(&settings.nav_folded),
            folds_init: tree::parse_ids(&settings.nav_folds_init),
            folds_unsaved: false,
            nav_collapsed: settings.nav_collapsed,
            view: View::AllInboxes,
            search: String::new(),
            search_seq: 0,
            filter: Filter::All,
            search_all: false,
            rows,
            open: None,
            draft_open: None,
            mark_read: settings.mark_read,
            read_wait: ReadWait::default(),
            select_mode: false,
            kept_unread: HashSet::new(),
            held: None,
            held_seq: 0,
            flags_seq: 0,
            undo: None,
            undo_clock: UndoClock::default(),
            delete_wait: Vec::new(),
            move_recent: super::triage::parse_recent(&settings.move_recent),
            status: None,
            account_state: HashMap::new(),
            first_downloads: HashMap::new(),
            jobs: Vec::new(),
            effects: Vec::new(),
        };
        mail.load_folders();
        mail.view = start_view(
            &settings.last_view,
            &mail.accounts,
            &mail.folders,
            mail.outbox_count,
        );
        mail.reload_rows();
        mail
    }

    /// Read the accounts, folders and counts again (a sync changed them);
    /// the view leaves a folder that is gone for the Inbox.
    pub fn load_folders(&mut self) {
        match (self.db.accounts(), self.db.folders()) {
            (Ok(accounts), Ok(folders)) => {
                self.accounts = accounts;
                self.folders = folders;
            }
            (Err(e), _) | (_, Err(e)) => self.status = Some(Status::Error(format!("{e:#}"))),
        }
        self.load_counts();
        let ids: Vec<i64> = self.accounts.iter().map(|a| a.id).collect();
        if tree::prune_folds(&mut self.folded, &ids, &self.folders) {
            self.save_folds();
        }
        let unread = self.unread.clone();
        self.apply_default_folds(&unread);
        if let View::Folder(id) = self.view
            && !self.folders.iter().any(|f| f.id == id)
        {
            let view = start_view("", &self.accounts, &self.folders, self.outbox_count);
            self.set_view(view);
        }
    }

    /// The unread, Drafts and Outbox counts.
    pub fn load_counts(&mut self) {
        self.unread = self.db.unread_counts().unwrap_or_default();
        self.draft_count = self.db.count_drafts().unwrap_or(0);
        self.outbox_count = self.db.count_outbox().unwrap_or(0);
    }

    // ------------------------------------------------------------ reads

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    pub fn search_all(&self) -> bool {
        self.search_all
    }

    pub fn rows(&self) -> &Rows {
        &self.rows
    }

    pub fn selection(&self) -> std::cell::Ref<'_, Selection> {
        self.rows.selection()
    }

    /// The open message's id.
    pub fn open_id(&self) -> Option<i64> {
        self.open.map(|o| o.id)
    }

    pub fn open(&self) -> Option<Brief> {
        self.open
    }

    /// The saved draft the composer shows (Drafts view).
    pub fn draft_open(&self) -> Option<i64> {
        self.draft_open
    }

    pub fn select_mode(&self) -> bool {
        self.select_mode
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    pub fn nav_collapsed(&self) -> bool {
        self.nav_collapsed
    }

    /// What the workers report of each account's sync: the lists' states
    /// and the first downloads' progress.
    pub fn set_sync(
        &mut self,
        states: HashMap<i64, AccountState>,
        fresh: HashMap<i64, (usize, usize)>,
    ) {
        self.account_state = states;
        self.first_downloads = fresh;
    }

    /// The first download of an account shown in the list, if any.
    fn fresh_download(&self, shown: &[i64]) -> Option<(usize, usize)> {
        self.folders
            .iter()
            .filter(|f| shown.contains(&f.id))
            .find_map(|f| self.first_downloads.get(&f.account_id).copied())
    }

    /// Changes for the server since the last call, by account.
    pub fn take_jobs(&mut self) -> Vec<(i64, Job)> {
        std::mem::take(&mut self.jobs)
    }

    /// What the window should do since the last call.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// The account's name as the app shows it: its name, else its address.
    pub fn account_label(a: &Account) -> &str {
        match a.name.trim() {
            "" => a.email.as_str(),
            name => name,
        }
    }

    pub fn folder(&self, id: i64) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn folder_role(&self, id: i64) -> Role {
        self.folder(id).map_or(Role::Other, |f| f.role)
    }

    /// The unread mail shown per folder: the cache's, less the messages
    /// a held move hides.
    pub fn shown_unread(&self) -> HashMap<i64, i64> {
        let mut unread = self.unread.clone();
        for (folder, n) in self.held.iter().flat_map(|h| &h.unread) {
            if let Some(count) = unread.get_mut(folder) {
                *count = (*count - n).max(0);
            }
        }
        unread
    }

    /// The inboxes' unread total (the window title's count).
    pub fn inbox_unread(&self) -> i64 {
        let unread = self.shown_unread();
        self.folders
            .iter()
            .filter(|f| f.role == Role::Inbox)
            .map(|f| unread.get(&f.id).copied().unwrap_or(0))
            .sum()
    }

    /// The views whose rows are messages (and so can be multi-selected).
    pub fn lists_messages(&self) -> bool {
        matches!(self.view, View::AllInboxes | View::Folder(_))
    }

    /// A query is in the search box.
    pub fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    /// A search of every cached folder is running.
    pub fn searching_all(&self) -> bool {
        self.search_all && self.lists_messages() && self.searching()
    }

    pub fn view_folders(&self) -> Vec<i64> {
        match self.view {
            View::AllInboxes => self
                .folders
                .iter()
                .filter(|f| f.role == Role::Inbox)
                .map(|f| f.id)
                .collect(),
            View::Folder(id) => vec![id],
            View::Drafts | View::Outbox => Vec::new(),
        }
    }

    /// The folders the list shows: the view's, or every folder while
    /// searching all of them.
    pub fn list_folders(&self) -> Vec<i64> {
        if self.searching_all() {
            self.folders.iter().map(|f| f.id).collect()
        } else {
            self.view_folders()
        }
    }

    pub fn view_role(&self) -> Role {
        match self.view {
            View::AllInboxes => Role::Inbox,
            View::Drafts => Role::Drafts,
            View::Outbox => Role::Sent,
            View::Folder(id) => self.folder_role(id),
        }
    }

    /// A server Drafts folder: its messages are drafts, not mail to
    /// triage (no filters, no unread bold).
    pub fn list_drafts(&self) -> bool {
        matches!(self.view, View::Folder(_)) && self.view_role() == Role::Drafts
    }

    /// The folder's account, for the list's subtitle (several accounts can
    /// each have an Inbox).
    pub fn view_account(&self) -> Option<&Account> {
        match self.view {
            View::Folder(id) if !self.searching() => {
                let f = self.folder(id)?;
                self.accounts.iter().find(|a| a.id == f.account_id)
            }
            _ => None,
        }
    }

    /// What the list says while it is empty.
    pub fn empty(&self) -> Empty {
        let folders = self.view_folders();
        let failed = self
            .folders
            .iter()
            .filter(|f| folders.contains(&f.id))
            .filter_map(|f| self.account_state.get(&f.account_id))
            .find(|s| matches!(s, AccountState::Unreachable | AccountState::Failed))
            .cloned();
        let synced = matches!(self.view, View::Folder(_))
            && self
                .folders
                .iter()
                .filter(|f| folders.contains(&f.id))
                .any(|f| {
                    matches!(
                        self.account_state.get(&f.account_id),
                        Some(AccountState::Checked(_))
                    )
                });
        let state = ListState {
            syncing: self
                .account_state
                .values()
                .any(|s| *s == AccountState::Syncing),
            failed,
            fresh: self.fresh_download(&folders),
            synced,
        };
        empty_kind(self.view, &self.search, self.filter, &state)
    }

    // ------------------------------------------------------- the list

    /// Read the view's ids again (one indexed query), keeping the
    /// selection of rows still listed.
    pub fn reload_rows(&mut self) {
        let (ids, kind) = match self.view {
            View::Drafts => (self.db.draft_ids(&self.search), Kind::Drafts),
            View::Outbox => (self.db.outbox_ids(&self.search), Kind::Outbox),
            _ => (
                self.list_ids(),
                if self.searching_all() {
                    let tags = self
                        .folders
                        .iter()
                        .map(|f| (f.id, (self.folder_tag(f), f.role.is_outgoing())))
                        .collect();
                    self.rows.set_folders(tags);
                    Kind::Mixed
                } else if self.view_role().is_outgoing() {
                    Kind::Outgoing
                } else {
                    Kind::Received
                },
            ),
        };
        let mut ids = ids.unwrap_or_else(|e| {
            self.status = Some(Status::Error(format!("{e:#}")));
            Vec::new()
        });
        if let Some(held) = self.held.as_ref().filter(|_| self.lists_messages()) {
            ids.retain(|id| !held.ids.contains(id));
        }
        // The server's copies of local drafts: the Drafts entry lists the
        // drafts themselves, so none shows twice.
        if self.lists_messages() {
            let shown = self.list_folders();
            let drafts: Vec<i64> = self
                .folders
                .iter()
                .filter(|f| f.role == Role::Drafts && shown.contains(&f.id))
                .map(|f| f.id)
                .collect();
            if let Ok(copies) = self.db.draft_copies(&drafts)
                && !copies.is_empty()
            {
                ids.retain(|id| !copies.contains(id));
            }
        }
        // With more than one account, All inboxes tags each row with its
        // account's name.
        let tags = if self.view == View::AllInboxes && self.accounts.len() > 1 {
            self.accounts
                .iter()
                .map(|a| (a.id, Self::account_label(a).to_string()))
                .collect()
        } else {
            HashMap::new()
        };
        self.rows.set_account_tags(tags);
        self.rows.set(ids, kind);
    }

    /// The view's message ids under the filter and the search; in the
    /// Unread filter, the messages opened there stay (read by now).
    fn list_ids(&self) -> anyhow::Result<Vec<i64>> {
        let folders = self.list_folders();
        if self.filter != Filter::Unread || self.kept_unread.is_empty() {
            return self
                .db
                .message_ids_filtered(&folders, &self.search, self.filter);
        }
        let unread: HashSet<i64> = self
            .db
            .message_ids_filtered(&folders, &self.search, Filter::Unread)?
            .into_iter()
            .collect();
        let mut ids = self
            .db
            .message_ids_filtered(&folders, &self.search, Filter::All)?;
        ids.retain(|id| unread.contains(id) || self.kept_unread.contains(id));
        Ok(ids)
    }

    /// A row's tag in a search of all folders: "Account · Folder".
    fn folder_tag(&self, f: &Folder) -> String {
        let folder = super::text::folder_label(f);
        match self.accounts.iter().find(|a| a.id == f.account_id) {
            Some(a) => super::text::account_folder(Self::account_label(a), &folder),
            None => folder,
        }
    }

    pub fn set_lang(&self, lang: Lang) {
        self.rows.set_lang(lang);
    }

    /// Show `view`: the filter, the scope and the selection start over,
    /// the query stays and runs there (docs/WINUI3.md "AutoSuggestBox").
    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.search_seq += 1;
        self.filter = Filter::All;
        self.search_all = false;
        self.kept_unread.clear();
        self.select_mode = false;
        self.close_reader();
        if self.draft_open.take().is_some() {
            self.effects.push(Effect::CloseComposer);
        }
        // The next start opens here (a convenience: a failed write only
        // loses it).
        self.db.set_setting("last_view", &view.key()).ok();
        self.rows.select(|s, _| s.clear());
        self.reload_rows();
        self.effects.push(Effect::FocusList);
    }

    /// The All / Unread / Important pills.
    pub fn set_filter(&mut self, filter: Filter) {
        if !self.lists_messages() || self.list_drafts() {
            return;
        }
        self.filter = filter;
        self.kept_unread.clear();
        self.reload_rows();
    }

    /// The search scope: every cached folder, or the view shown. It stays
    /// until the view changes.
    pub fn set_search_all(&mut self, all: bool) {
        if !self.lists_messages() {
            return;
        }
        self.search_all = all;
        self.search_seq += 1;
        self.reload_rows();
    }

    /// The search text changed. Large scopes are searched once typing
    /// pauses ([`Effect::SearchAfter`]); small ones and a cleared search at
    /// once.
    pub fn search_edited(&mut self, text: &str) {
        if self.search == text {
            return;
        }
        self.search = text.to_string();
        self.search_seq += 1;
        let scope = self
            .list_folders()
            .iter()
            .map(|&f| self.db.count_messages(f).unwrap_or(0))
            .sum();
        if debounces_search(scope, text) {
            self.effects
                .push(Effect::SearchAfter(self.search_seq, SEARCH_DEBOUNCE));
        } else {
            self.reload_rows();
        }
    }

    /// A paused search's time came: it runs if nothing changed since.
    pub fn search_due(&mut self, seq: u64) {
        if seq == self.search_seq {
            self.reload_rows();
        }
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        self.search_seq += 1;
        self.reload_rows();
    }

    /// Esc in the list: clears the search, else the status line (when
    /// nothing is open or selected), else Select messages, else the
    /// selection and the message open.
    pub fn escape(&mut self) -> Escaped {
        if !self.search.is_empty() {
            self.clear_search();
            return Escaped::ClearedSearch;
        }
        let selected = self.rows.selection().len();
        if self.open.is_none() && selected == 0 && self.status.is_some() {
            self.status = None;
            return Escaped::ClosedStatus;
        }
        if self.select_mode && self.open.is_none() && selected == 0 {
            self.select_mode = false;
            return Escaped::LeftSelectMode;
        }
        if self.open.is_none() && selected == 0 {
            return Escaped::Nothing;
        }
        self.close_reader();
        Escaped::ClosedMessage
    }

    pub fn dismiss_status(&mut self) {
        self.status = None;
    }

    /// Undo brought back messages whose windows had closed: the status
    /// line says to open them from the list.
    pub fn say_undo_window(&mut self) {
        self.status = Some(Status::UndoWindow);
    }

    /// "Select messages" in the list's menu: check boxes on or off.
    pub fn set_select_mode(&mut self, on: bool) {
        self.select_mode = on && self.lists_messages();
    }

    /// The rows show check boxes: Select messages, or two or more rows
    /// selected.
    pub fn checks(&self) -> bool {
        self.lists_messages() && (self.select_mode || self.rows.selection().len() > 1)
    }

    // -------------------------------------------------- the selection

    /// A row was clicked: a message opens in the reader, a saved draft in
    /// the composer (Outbox rows are selected; their pane comes with the
    /// Outbox view, phase 4).
    pub fn row_clicked(&mut self, id: i64) {
        match self.view {
            View::Drafts => self.open_draft(id),
            View::Outbox => {
                self.rows.select(|s, _| s.single(id));
            }
            _ => {
                self.select_message(id);
            }
        }
    }

    /// A click with Ctrl (or any click with the check boxes on): the row
    /// joins the selection or leaves it.
    pub fn row_toggled(&mut self, id: i64) {
        if !self.lists_messages() {
            return self.row_clicked(id);
        }
        self.select_rows(|s, _| s.toggle(id));
    }

    /// Shift+click: the range from the anchor to the row.
    pub fn row_extended(&mut self, id: i64) {
        if !self.lists_messages() {
            return self.row_clicked(id);
        }
        self.select_rows(|s, rows| s.extend_to(rows, id));
    }

    /// Shift+Up/Down.
    pub fn extend_by(&mut self, delta: isize) {
        self.select_rows(|s, rows| s.extend_by(rows, delta));
    }

    /// Ctrl+A.
    pub fn select_all(&mut self) {
        self.select_rows(|s, rows| s.all(rows));
    }

    /// The selection panel's Clear selection.
    pub fn clear_selection(&mut self) {
        self.select_rows(|s, _| s.clear());
    }

    /// Change the selection. The open message stays only while it is all
    /// that is selected; otherwise the reading pane shows the selection.
    pub fn select_rows(&mut self, f: impl FnOnce(&mut Selection, &[i64])) {
        if !self.lists_messages() {
            return;
        }
        self.rows.select(f);
        let keep = {
            let selection = self.rows.selection();
            self.open
                .is_some_and(|o| selection.len() == 1 && selection.contains(o.id))
        };
        if !keep && self.open.take().is_some() {
            self.read_wait.cancel();
            self.effects.push(Effect::FocusList);
        }
    }

    /// A right press on row `id`: selected first unless it already is
    /// (without opening it, so reporting junk never marks it read). False
    /// where rows have no menu.
    pub fn row_context(&mut self, id: i64) -> bool {
        if !self.lists_messages() {
            self.rows.select(|s, _| s.single(id));
            return true;
        }
        self.select_rows(|s, _| s.right_click(id));
        true
    }

    /// Up/Down and Page Up/Down: open the message `delta` rows away (with
    /// none open, the first).
    pub fn select_offset(&mut self, delta: isize) {
        self.select_by(|current, last| match current {
            Some(i) => i.saturating_add_signed(delta).min(last),
            None => 0,
        });
    }

    /// Home / End: open the first or the last message.
    pub fn select_edge(&mut self, end: bool) {
        self.select_by(|_, last| if end { last } else { 0 });
    }

    fn select_by(&mut self, pick: impl FnOnce(Option<usize>, usize) -> usize) {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return;
        };
        let current = self.current_row().and_then(|id| self.rows.index_of(id));
        let next = pick(current, last);
        if Some(next) == current {
            return;
        }
        let Some(id) = self.rows.id_at(next) else {
            return;
        };
        match self.view {
            // A draft opens in the composer, which takes the keys: the
            // arrows only move to it, and Enter opens it.
            View::Drafts | View::Outbox => {
                self.rows.select(|s, _| s.single(id));
            }
            _ => {
                self.select_message(id);
            }
        }
    }

    /// The open message, else the row Shift+Up/Down move from.
    pub fn current_row(&self) -> Option<i64> {
        self.open.map(|o| o.id).or(self.rows.selection().cursor())
    }

    /// The row the list's keyboard is on (shown with the focus ring).
    pub fn current_index(&self) -> Option<usize> {
        self.current_row().and_then(|id| self.rows.index_of(id))
    }

    /// Enter in the list: open the current row (with none, the first).
    /// True when a message shows, so the page can take the focus.
    pub fn open_current(&mut self) -> bool {
        let listed = self
            .current_row()
            .filter(|&id| self.rows.index_of(id).is_some());
        let Some(id) = listed.or_else(|| self.rows.id_at(0)) else {
            return false;
        };
        if !self.lists_messages() {
            self.row_clicked(id);
            return false;
        }
        if self.open.is_none_or(|o| o.id != id) {
            self.select_message(id);
        }
        self.open.is_some()
    }

    /// Message `id` opens: selected alone, its read delay started. False
    /// when it is not cached.
    pub fn select_message(&mut self, id: i64) -> bool {
        let brief = match self.db.briefs(&[id]) {
            Ok(b) => b.into_iter().next(),
            Err(e) => {
                self.status = Some(Status::Error(format!("{e:#}")));
                None
            }
        };
        let Some(brief) = brief else {
            return false;
        };
        // Always asks: a New mail has no `draft_open`, and the pane's dirty
        // draft still has to go to a tab.
        self.draft_open = None;
        self.effects.push(Effect::CloseComposer);
        if self.filter == Filter::Unread && self.lists_messages() {
            self.kept_unread.insert(id);
        }
        self.open = Some(brief);
        self.rows.select(|s, _| s.single(id));
        if let Some(delay) = self.read_wait.opened(id, !brief.seen, self.mark_read) {
            if delay.is_zero() {
                self.read_wait.cancel();
                self.set_flags(&[brief], super::triage::Flag::Seen, true);
            } else {
                self.effects.push(Effect::ReadAfter(id, delay));
            }
        }
        true
    }

    /// The open message has been open long enough: it is marked read.
    pub fn read_due(&mut self) {
        let open = self.open_id();
        if self.read_wait.due(open).is_some()
            && let Some(brief) = self.open
        {
            self.set_flags(&[brief], super::triage::Flag::Seen, true);
        }
    }

    /// Nothing open, nothing selected.
    pub fn close_reader(&mut self) {
        self.open = None;
        self.read_wait.cancel();
        self.rows.select(|s, _| s.clear());
    }

    /// A saved draft opens in the composer.
    fn open_draft(&mut self, id: i64) {
        self.rows.select(|s, _| s.single(id));
        self.draft_open = Some(id);
        self.effects.push(Effect::OpenDraft(id));
    }

    /// What an action applies to: the open message, or with none open the
    /// selected rows.
    pub fn targets(&self) -> Vec<Brief> {
        if let Some(open) = self.open {
            return vec![open];
        }
        let ids = self.rows.selection().ids();
        self.db.briefs(&ids).unwrap_or_default()
    }

    /// Exactly one message is open or selected.
    pub fn one_target(&self) -> bool {
        self.open.is_some() || self.rows.selection().len() == 1
    }

    // ----------------------------------------------------- the folds

    pub(super) fn save_folds(&self) {
        self.db
            .set_setting("nav_folded", &tree::format_folds(&self.folded))
            .ok();
    }

    /// Fold the long parts of each account's tree the first time its
    /// folders are here, once per account (`nav_folds_init`). Written by
    /// [`Mail::flush_default_folds`] after the first frame.
    fn apply_default_folds(&mut self, unread: &HashMap<i64, i64>) {
        let ids: HashSet<i64> = self.accounts.iter().map(|a| a.id).collect();
        let mut changed = self.folds_init.iter().any(|id| !ids.contains(id));
        self.folds_init.retain(|id| ids.contains(id));
        let n = self.accounts.len();
        for (position, a) in self.accounts.iter().enumerate() {
            if self.folds_init.contains(&a.id) || !self.folders.iter().any(|f| f.account_id == a.id)
            {
                continue;
            }
            self.folded.extend(tree::default_folds(
                a.id,
                position,
                n,
                &self.folders,
                unread,
            ));
            self.folds_init.insert(a.id);
            changed = true;
        }
        self.folds_unsaved |= changed;
    }

    /// Write the default folds (after the first frame: two commits off the
    /// launch path). True when something was written.
    pub fn flush_default_folds(&mut self) -> bool {
        if !std::mem::take(&mut self.folds_unsaved) {
            return false;
        }
        self.save_folds();
        self.db
            .set_setting("nav_folds_init", &tree::format_ids(&self.folds_init))
            .ok();
        true
    }

    /// The default folds wait to be written.
    pub fn folds_unsaved(&self) -> bool {
        self.folds_unsaved
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use komorebi::db::Account;
    use komorebi::providers::{Security, Server};

    pub fn account(id: i64) -> Account {
        let server = Server {
            host: "imap.example.invalid".into(),
            port: 993,
            security: Security::Tls,
        };
        Account {
            id,
            email: format!("a{id}@example.invalid"),
            name: String::new(),
            imap: server.clone(),
            smtp: server,
            save_sent: true,
            signature: String::new(),
            enabled: true,
            imap_user: String::new(),
            smtp_user: String::new(),
            imap_pin: None,
            smtp_pin: None,
            smtp_no_auth: false,
        }
    }

    pub fn folder(id: i64, account_id: i64, role: Role) -> Folder {
        Folder {
            id,
            account_id,
            name: format!("f{id}"),
            display: format!("f{id}"),
            depth: 0,
            role,
            uidvalidity: 0,
        }
    }

    /// Port of the Slint app's `start_view_falls_back_to_the_inbox`
    /// (src/app.rs).
    #[test]
    fn start_view_falls_back_to_the_inbox() {
        let one = [account(1)];
        let two = [account(1), account(2)];
        let folders = [
            folder(10, 1, Role::Inbox),
            folder(11, 1, Role::Other),
            folder(20, 2, Role::Inbox),
        ];
        // A folder saved last time and still here opens again.
        assert_eq!(start_view("f11", &one, &folders, 0), View::Folder(11));
        assert_eq!(View::Folder(11).key(), "f11");
        // Gone (or of an account that is gone): the Inbox, or with more
        // than one account All inboxes.
        assert_eq!(start_view("f99", &one, &folders, 0), View::Folder(10));
        assert_eq!(start_view("f20", &one, &folders, 0), View::Folder(10));
        assert_eq!(start_view("f99", &two, &folders, 0), View::AllInboxes);
        // All inboxes needs two accounts; one opens on its Inbox.
        assert_eq!(start_view("all", &one, &folders, 0), View::Folder(10));
        assert_eq!(start_view("all", &two, &folders, 0), View::AllInboxes);
        // Drafts always; the Outbox only while it holds mail.
        assert_eq!(start_view("drafts", &one, &folders, 0), View::Drafts);
        assert_eq!(start_view("outbox", &one, &folders, 0), View::Folder(10));
        assert_eq!(start_view("outbox", &one, &folders, 2), View::Outbox);
        // Nothing saved; no Inbox yet; no account.
        assert_eq!(start_view("", &one, &folders, 0), View::Folder(10));
        assert_eq!(start_view("", &one, &[], 0), View::AllInboxes);
        assert_eq!(start_view("drafts", &[], &[], 0), View::AllInboxes);
    }

    /// Port of `only_large_scopes_debounce_search` (src/app.rs).
    #[test]
    fn only_large_scopes_debounce_search() {
        assert!(!debounces_search(1_200, "inv"));
        assert!(!debounces_search(SEARCH_DEBOUNCE_ABOVE, "inv"));
        assert!(debounces_search(SEARCH_DEBOUNCE_ABOVE + 1, "inv"));
        // Clearing the search always reloads at once.
        assert!(!debounces_search(50_000, ""));
        assert!(!debounces_search(50_000, "  "));
    }

    /// Port of `the_read_delay_is_cancelled_by_moving_on` (src/app.rs).
    #[test]
    fn the_read_delay_is_cancelled_by_moving_on() {
        let delay = Duration::from_secs(1);
        let mut wait = ReadWait::default();
        // Arrowing through 1, 2, 3: only 3, still open, is marked.
        assert_eq!(wait.opened(1, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.opened(2, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.opened(3, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.due(Some(3)), Some(3));
        // It fires once only.
        assert_eq!(wait.due(Some(3)), None);
        // A read message never waits, and opening it drops an earlier wait.
        wait.opened(4, true, MarkRead::After(delay));
        assert_eq!(wait.opened(5, false, MarkRead::After(delay)), None);
        assert_eq!(wait.due(Some(4)), None);
        // Closed (or another message open) when the delay ends: nothing.
        wait.opened(6, true, MarkRead::After(delay));
        assert_eq!(wait.due(None), None);
        wait.opened(7, true, MarkRead::After(delay));
        wait.cancel();
        assert_eq!(wait.due(Some(7)), None);
        // At once, or never.
        assert_eq!(wait.opened(8, true, MarkRead::Now), Some(Duration::ZERO));
        assert_eq!(wait.due(Some(8)), None);
        assert_eq!(wait.opened(9, true, MarkRead::Never), None);
    }

    /// Port of `empty_list_texts` (src/app.rs): the choice here, the words
    /// in `text::tests`.
    #[test]
    fn empty_list_kinds() {
        let st = |syncing, failed| ListState {
            syncing,
            failed,
            ..Default::default()
        };
        let inbox = View::AllInboxes;
        let cases = [
            ("x", Filter::Unread, st(true, None), Empty::NoMatches),
            ("", Filter::Unread, st(true, None), Empty::NoUnread),
            (
                "",
                Filter::Flagged,
                st(false, Some(AccountState::Failed)),
                Empty::NoFlagged,
            ),
            (
                "",
                Filter::All,
                st(true, Some(AccountState::Failed)),
                Empty::Loading,
            ),
            ("  ", Filter::All, st(false, None), Empty::Nothing),
            (
                "",
                Filter::All,
                st(false, Some(AccountState::Failed)),
                Empty::Failed,
            ),
            (
                "",
                Filter::All,
                st(false, Some(AccountState::Unreachable)),
                Empty::Unreachable,
            ),
        ];
        for (search, filter, state, want) in cases {
            assert_eq!(empty_kind(inbox, search, filter, &state), want);
        }
        // Drafts and the Outbox say what they hold (a search still finds
        // nothing first).
        let none = st(false, None);
        assert_eq!(
            empty_kind(View::Drafts, "", Filter::All, &st(true, None)),
            Empty::NoDrafts
        );
        assert_eq!(
            empty_kind(View::Outbox, "", Filter::All, &none),
            Empty::Outbox
        );
        assert_eq!(
            empty_kind(View::Outbox, "x", Filter::All, &none),
            Empty::NoMatches
        );
        // A new account's first download, over Loading (with its count once
        // known), and an empty folder once a sync pass has completed.
        let folder = View::Folder(7);
        let fresh = |f| ListState {
            syncing: true,
            fresh: Some(f),
            ..Default::default()
        };
        assert_eq!(
            empty_kind(folder, "", Filter::All, &fresh((0, 0))),
            Empty::DownloadingStart
        );
        assert_eq!(
            empty_kind(folder, "", Filter::All, &fresh((6000, 5000))),
            Empty::Downloading(5000, 5000)
        );
        let synced = ListState {
            synced: true,
            ..Default::default()
        };
        assert_eq!(empty_kind(folder, "", Filter::All, &synced), Empty::Folder);
        assert_eq!(
            empty_kind(folder, "", Filter::Unread, &synced),
            Empty::NoUnread
        );
    }
}
