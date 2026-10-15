//! Triage on the message list and the reader: the row
//! and More menus with their accelerator text, the rows' quick actions,
//! the Move flyout (filter, recent folders, depth), the Undo offer's own
//! slot and its clock, the Trash / Junk delete question and the Unread
//! filter's opened rows.

use super::*;

// The commands of the row menu and the More menu (`run-command` in
// ui/app.slint dispatches on the same numbers).
pub(super) const CMD_REPLY: i32 = 10;
pub(super) const CMD_REPLY_ALL: i32 = 11;
pub(super) const CMD_FORWARD: i32 = 12;
pub(super) const CMD_FORWARD_ATTACHED: i32 = 13;
pub(super) const CMD_MARK_READ: i32 = 14;
pub(super) const CMD_MARK_UNREAD: i32 = 15;
pub(super) const CMD_FLAG: i32 = 16;
pub(super) const CMD_UNFLAG: i32 = 17;
pub(super) const CMD_ARCHIVE: i32 = 18;
pub(super) const CMD_DELETE: i32 = 19;
pub(super) const CMD_JUNK: i32 = 20;
pub(super) const CMD_NOT_JUNK: i32 = 21;
pub(super) const CMD_SAVE: i32 = 22;
pub(super) const CMD_ORIGINAL: i32 = 23;
pub(super) const CMD_COPY_ID: i32 = 24;
pub(super) const CMD_SHOW_IN_FOLDER: i32 = 25;
pub(super) const CMD_PLAIN: i32 = 26;
pub(super) const CMD_COLOURS: i32 = 27;
pub(super) const CMD_LOAD_IMAGES: i32 = 28;
pub(super) const CMD_STOP_IMAGES: i32 = 29;
/// Open in new tab / in new window (`app::tabs`, `app::windows`).
pub(super) const CMD_OPEN_TAB: i32 = 30;
pub(super) const CMD_OPEN_WINDOW: i32 = 31;

/// The quick actions of a row (`message-quick`).
pub(super) const QUICK_ARCHIVE: i32 = 0;
pub(super) const QUICK_DELETE: i32 = 1;
pub(super) const QUICK_FLAG: i32 = 2;
pub(super) const QUICK_READ: i32 = 3;

/// A held move's undo window at least (it lasts until the next move, or
/// this, whichever is later).
pub(super) const UNDO_WINDOW: Duration = Duration::from_secs(10);
/// What is left of the window once the pointer leaves the offer (or its
/// focus goes) after a pause: at least this.
pub(super) const UNDO_RESUME: Duration = Duration::from_secs(3);
/// How many recently used folders the Move flyout lists first.
const MOVE_RECENT: usize = 3;
/// How many recent folders are kept (of every account).
const MOVE_RECENT_KEPT: usize = 12;

/// What the Undo slot at the bottom of the list offers to take back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum UndoOffer {
    #[default]
    None,
    /// The send of this outbox row, in its undo window.
    Send(i64),
    /// The held move or delete with this sequence number.
    Move(u64),
}

/// The clock of a held move's undo window: due [`UNDO_WINDOW`] after it
/// starts, stopped while the offer is hovered or focused (WCAG 2.2.1), and
/// then due at least [`UNDO_RESUME`] after it goes on.
#[derive(Debug, Default)]
pub(super) struct UndoClock {
    due: Option<Instant>,
    /// Stopped, with this much left.
    paused: Option<Duration>,
}

impl UndoClock {
    /// A new window from `now`: how long until it is due.
    pub(super) fn start(&mut self, now: Instant) -> Duration {
        self.due = Some(now + UNDO_WINDOW);
        self.paused = None;
        UNDO_WINDOW
    }

    /// The offer is hovered or focused: the clock stops.
    pub(super) fn pause(&mut self, now: Instant) {
        if let Some(due) = self.due.take() {
            self.paused = Some(due.saturating_duration_since(now));
        }
    }

    /// The pointer left (or the focus went): how long until it is due now,
    /// if it was stopped.
    pub(super) fn resume(&mut self, now: Instant) -> Option<Duration> {
        let left = self.paused.take()?.max(UNDO_RESUME);
        self.due = Some(now + left);
        Some(left)
    }

    /// The window is over at `now` (never while stopped).
    pub(super) fn is_due(&self, now: Instant) -> bool {
        self.due.is_some_and(|due| now >= due)
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A menu entry with its accelerator text after a tab (Win32 draws it
/// right-aligned; Slint 1.18 allows `MenuItem.shortcut` only in a menu bar).
fn entry(title: String, keys: &str, enabled: bool, command: i32) -> MenuCommand {
    let title = if keys.is_empty() {
        title
    } else {
        format!("{title}\t{keys}")
    };
    MenuCommand {
        title: title.into(),
        enabled,
        command,
        id: 0,
        checkable: false,
        checked: false,
    }
}

fn check_entry(title: String, enabled: bool, checked: bool, command: i32) -> MenuCommand {
    MenuCommand {
        checkable: true,
        checked,
        ..entry(title, "", enabled, command)
    }
}

/// The row menu's groups (between separators; Move to after the third).
pub(super) struct RowMenu {
    pub groups: [Vec<MenuCommand>; 6],
}

/// The More menu's groups (Move to after the first).
pub(super) struct MoreMenu {
    pub groups: [Vec<MenuCommand>; 5],
}

/// The recent folders setting (`move_recent`: folder ids, the last used
/// first).
pub(super) fn parse_recent(value: &str) -> Vec<i64> {
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
pub(super) fn push_recent(recent: &mut Vec<i64>, id: i64) {
    recent.retain(|&r| r != id);
    recent.insert(0, id);
    recent.truncate(MOVE_RECENT_KEPT);
}

/// The Move flyout's entries: with no filter, the recent folders among
/// `options` first (marked `recent`), then all of them; with a filter, the
/// options whose name holds it (any case), in order.
pub(super) fn move_entries(
    options: &[FolderOption],
    recent: &[i64],
    filter: &str,
) -> Vec<FolderOption> {
    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        return options
            .iter()
            .filter(|o| o.label.to_lowercase().contains(&filter))
            .cloned()
            .collect();
    }
    let mut out: Vec<FolderOption> = recent
        .iter()
        .filter_map(|id| options.iter().find(|o| i64::from(o.id) == *id))
        .take(MOVE_RECENT)
        .map(|o| FolderOption {
            recent: true,
            ..o.clone()
        })
        .collect();
    out.extend(options.iter().cloned());
    out
}

/// The Move flyout's indent of each node of the tree (in its order):
/// how many of its ancestors are listed (`included`), so a folder whose
/// parent is left out (the current folder, a Drafts folder) sits where
/// that parent would, never indented under nothing. Capped like the pane.
pub(super) fn move_levels(nodes: &[tree::TreeNode], included: &[bool]) -> Vec<i32> {
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
            i32::try_from(level).unwrap_or(0)
        })
        .collect()
}

/// A message window's More menu (`app::windows`): Forward as attachment;
/// Report junk or Not junk; the view toggles and images. `actions` is off
/// while its message is gone from the server.
pub(super) fn popout_menu(
    junk: bool,
    html: bool,
    plain: bool,
    original: bool,
    dimmed: bool,
    blocked: bool,
    actions: bool,
) -> [Vec<MenuCommand>; 3] {
    let junk_entry = if junk {
        entry(t!(invoke_menu_not_junk), "", actions, CMD_NOT_JUNK)
    } else {
        entry(t!(invoke_menu_report_junk), "", actions, CMD_JUNK)
    };
    [
        vec![entry(
            t!(invoke_menu_forward_attached),
            "",
            actions,
            CMD_FORWARD_ATTACHED,
        )],
        vec![junk_entry],
        vec![
            check_entry(t!(invoke_menu_plain_text), html || plain, plain, CMD_PLAIN),
            check_entry(
                t!(invoke_menu_original_colours),
                html && !plain && (dimmed || original),
                original,
                CMD_COLOURS,
            ),
            entry(
                t!(invoke_menu_load_images),
                "",
                html && !plain && blocked,
                CMD_LOAD_IMAGES,
            ),
        ],
    ]
}

impl App {
    /// Accelerator text for Backspace in the language shown.
    fn backspace_key(&self) -> String {
        t!(invoke_key_backspace)
    }

    /// Exactly one message is open or selected (the commands on a single
    /// message apply).
    fn one_target(&self) -> bool {
        self.open.is_some() || self.rows.selection().len() == 1
    }

    /// What is acted on lies in a server Drafts folder (no Reply there).
    fn targets_in_drafts(&self, targets: &[db::Brief]) -> bool {
        !targets.is_empty()
            && targets
                .iter()
                .all(|m| self.folder_role(m.folder_id) == Role::Drafts)
    }

    /// The row menu for what is selected (or open).
    pub(super) fn row_menu(&self) -> RowMenu {
        let targets = self.targets();
        let one = self.one_target();
        let drafts = self.targets_in_drafts(&targets);
        let unread = targets.iter().any(|m| !m.seen);
        let unflagged = targets.iter().any(|m| !m.flagged);
        let junk = !targets.is_empty()
            && targets
                .iter()
                .all(|m| self.folder_role(m.folder_id) == Role::Junk);
        let read_entry = if unread {
            entry(t!(invoke_menu_mark_read), "Ctrl+Q", true, CMD_MARK_READ)
        } else {
            entry(t!(invoke_menu_mark_unread), "Ctrl+U", true, CMD_MARK_UNREAD)
        };
        let flag_entry = if unflagged {
            entry(t!(invoke_menu_flag), "Ctrl+Shift+G", true, CMD_FLAG)
        } else {
            entry(
                t!(invoke_menu_remove_flag),
                "Ctrl+Shift+G",
                true,
                CMD_UNFLAG,
            )
        };
        let junk_entry = if junk {
            entry(t!(invoke_menu_not_junk), "", true, CMD_NOT_JUNK)
        } else {
            entry(t!(invoke_menu_report_junk), "", true, CMD_JUNK)
        };
        let show_in_folder = if self.searching() {
            vec![entry(
                t!(invoke_show_in_folder),
                "",
                one,
                CMD_SHOW_IN_FOLDER,
            )]
        } else {
            Vec::new()
        };
        RowMenu {
            groups: [
                vec![
                    entry(t!(invoke_menu_open_tab), "Shift+Enter", one, CMD_OPEN_TAB),
                    entry(t!(invoke_menu_open_window), "Ctrl+O", one, CMD_OPEN_WINDOW),
                    entry(t!(invoke_menu_reply), "Ctrl+R", one && !drafts, CMD_REPLY),
                    entry(
                        t!(invoke_menu_reply_all),
                        "Ctrl+Shift+R",
                        one && !drafts,
                        CMD_REPLY_ALL,
                    ),
                    entry(t!(invoke_menu_forward), "Ctrl+Shift+F", one, CMD_FORWARD),
                    entry(
                        t!(invoke_menu_forward_attached),
                        "",
                        one,
                        CMD_FORWARD_ATTACHED,
                    ),
                ],
                vec![read_entry, flag_entry],
                vec![
                    entry(
                        t!(invoke_menu_archive),
                        &self.backspace_key(),
                        true,
                        CMD_ARCHIVE,
                    ),
                    entry(t!(invoke_menu_delete), "Del", true, CMD_DELETE),
                ],
                vec![junk_entry],
                vec![
                    entry(t!(invoke_menu_save_eml), "Ctrl+S", one, CMD_SAVE),
                    entry(t!(invoke_menu_show_original), "", one, CMD_ORIGINAL),
                    entry(t!(invoke_menu_copy_message_id), "", one, CMD_COPY_ID),
                ],
                show_in_folder,
            ],
        }
    }

    /// The reader's More menu for the open message.
    pub(super) fn more_menu(&self) -> MoreMenu {
        let ui = self.ui();
        let targets = self.targets();
        let junk = !targets.is_empty()
            && targets
                .iter()
                .all(|m| self.folder_role(m.folder_id) == Role::Junk);
        let junk_entry = if junk {
            entry(t!(invoke_menu_not_junk), "", true, CMD_NOT_JUNK)
        } else {
            entry(t!(invoke_menu_report_junk), "", true, CMD_JUNK)
        };
        let (html, plain) = (ui.get_r_html(), ui.get_r_plain());
        let original = ui.get_r_original();
        let mut view = vec![
            check_entry(t!(invoke_menu_plain_text), html || plain, plain, CMD_PLAIN),
            check_entry(
                t!(invoke_menu_original_colours),
                html && !plain && (ui.get_r_dimmed() || original),
                original,
                CMD_COLOURS,
            ),
        ];
        // The images: this message's, or no longer from its sender.
        let trusted = self
            .open
            .as_ref()
            .is_some_and(|o| self.db.remote_allowed(&o.msg.from_addr).unwrap_or(false));
        view.push(if trusted {
            entry(t!(invoke_menu_stop_images), "", true, CMD_STOP_IMAGES)
        } else {
            entry(
                t!(invoke_menu_load_images),
                "",
                html && !plain && ui.get_r_remote_blocked() > 0,
                CMD_LOAD_IMAGES,
            )
        });
        let show_in_folder = if self.searching() {
            vec![entry(
                t!(invoke_show_in_folder),
                "",
                true,
                CMD_SHOW_IN_FOLDER,
            )]
        } else {
            Vec::new()
        };
        MoreMenu {
            groups: [
                vec![
                    entry(t!(invoke_menu_open_window), "Ctrl+O", true, CMD_OPEN_WINDOW),
                    entry(
                        t!(invoke_menu_forward_attached),
                        "",
                        true,
                        CMD_FORWARD_ATTACHED,
                    ),
                ],
                vec![junk_entry],
                view,
                vec![
                    entry(t!(invoke_menu_save_eml), "Ctrl+S", true, CMD_SAVE),
                    entry(t!(invoke_menu_show_original), "", true, CMD_ORIGINAL),
                    entry(t!(invoke_menu_copy_message_id), "", true, CMD_COPY_ID),
                ],
                show_in_folder,
            ],
        }
    }

    /// The row menu or the More menu opens: both are built for what they
    /// act on (and the Move list without a filter).
    pub(super) fn menu_opening(&mut self) {
        self.move_filter.clear();
        self.show_move_targets();
        let row = self.row_menu();
        let more = self.more_menu();
        let ui = self.ui();
        let model = |v: &Vec<MenuCommand>| ModelRc::new(VecModel::from(v.clone()));
        ui.set_row_menu_1(model(&row.groups[0]));
        ui.set_row_menu_2(model(&row.groups[1]));
        ui.set_row_menu_3(model(&row.groups[2]));
        ui.set_row_menu_4(model(&row.groups[3]));
        ui.set_row_menu_5(model(&row.groups[4]));
        ui.set_row_menu_6(model(&row.groups[5]));
        ui.set_more_menu_1(model(&more.groups[0]));
        ui.set_more_menu_2(model(&more.groups[1]));
        ui.set_more_menu_3(model(&more.groups[2]));
        ui.set_more_menu_4(model(&more.groups[3]));
        ui.set_more_menu_5(model(&more.groups[4]));
    }

    // ------------------------------------------------------ quick actions

    /// A row's quick action, on that message only: it is never opened
    /// (so never marked read by opening), and the selection stays.
    pub(super) fn quick_action(&mut self, id: i64, action: i32) {
        if !self.reading() || !self.lists_messages() {
            return;
        }
        let targets = match self.db.briefs(&[id]) {
            Ok(t) if !t.is_empty() => t,
            Ok(_) => return,
            Err(e) => return self.set_status(&text::detail(&e), true),
        };
        match action {
            QUICK_ARCHIVE => self.remove_targets(Dest::Archive, targets),
            QUICK_DELETE => self.delete_targets(targets),
            QUICK_FLAG => {
                let on = !targets[0].flagged;
                self.set_flags(&targets, Flag::Flagged, on);
            }
            QUICK_READ => {
                let on = !targets[0].seen;
                self.set_flags(&targets, Flag::Seen, on);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------ delete

    /// Delete these messages: from Trash (deleted on the server) or Junk,
    /// after asking (a prompt with no default button); else to Trash.
    pub(super) fn delete_targets(&mut self, targets: Vec<db::Brief>) {
        if !self.reading() || targets.is_empty() {
            return;
        }
        let role = |m: &db::Brief| self.folder_role(m.folder_id);
        let trash = targets.iter().any(|m| role(m) == Role::Trash);
        let junk = targets.iter().any(|m| role(m) == Role::Junk);
        if trash || junk {
            self.delete_wait = targets.iter().map(|m| m.id).collect();
            return self.ask(Question::DeleteFrom { trash });
        }
        self.remove_targets(Dest::Trash, targets);
    }

    /// The Trash / Junk delete was confirmed.
    pub(super) fn delete_confirmed(&mut self) {
        let ids = std::mem::take(&mut self.delete_wait);
        let targets = self.db.briefs(&ids).unwrap_or_default();
        if !targets.is_empty() {
            self.remove_targets(Dest::Trash, targets);
        }
    }

    // ------------------------------------------------------ Move flyout

    /// The folders of `account` a message can move to, but `except`: in
    /// the folder pane's order, with their depth; never a Drafts folder.
    pub(super) fn move_options(&self, account: i64, except: &[i64]) -> Vec<FolderOption> {
        let nodes = tree::tree(&self.folders);
        let included: Vec<bool> = nodes
            .iter()
            .map(|n| {
                self.folders.get(n.index).is_some_and(|f| {
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
            .filter_map(|((n, level), _)| self.folders.get(n.index).map(|f| (f, level)))
            .map(|(f, level)| FolderOption {
                id: f.id as i32,
                label: text::folder_label(self.lang, f).into(),
                level,
                recent: false,
            })
            .collect()
    }

    /// The folders the open message, or the selection, can move to, before
    /// the flyout's filter (empty: Move is off).
    pub(super) fn move_choices(&self) -> Vec<FolderOption> {
        if let Some(open) = &self.open {
            return self.move_options(open.msg.account_id, &[open.msg.folder_id]);
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

    /// Show the Move targets: recent first, filtered by the flyout's box.
    pub(super) fn show_move_targets(&self) {
        let choices = self.move_choices();
        let entries = move_entries(&choices, &self.move_recent, &self.move_filter);
        let ui = self.ui();
        ui.set_move_possible(!choices.is_empty());
        ui.set_move_filter(self.move_filter.as_str().into());
        ui.set_move_targets(ModelRc::new(VecModel::from(entries)));
    }

    /// The flyout's filter changed.
    pub(super) fn move_filter_edited(&mut self, text: &str) {
        self.move_filter = text.to_string();
        self.show_move_targets();
    }

    /// A folder was used as a Move target: first among the recent ones
    /// (saved; a failed write only loses the order).
    pub(super) fn note_recent(&mut self, id: i64) {
        push_recent(&mut self.move_recent, id);
        let value = self
            .move_recent
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.db.set_setting("move_recent", &value).ok();
    }

    // ------------------------------------------------------ Undo slot

    /// Offer Undo at the bottom of the list (its own slot: a status line
    /// never replaces it).
    pub(super) fn offer_undo(&self, text: &str, offer: UndoOffer) {
        self.undo.set(offer);
        self.ui().set_undo_text(text.into());
    }

    /// The offer's text, while `offer` is still the one shown.
    pub(super) fn update_undo(&self, text: &str, offer: UndoOffer) {
        if self.undo.get() == offer {
            self.ui().set_undo_text(text.into());
        }
    }

    /// Take `offer` away, if it is the one shown.
    pub(super) fn clear_undo(&self, offer: UndoOffer) {
        if self.undo.get() == offer {
            self.undo.set(UndoOffer::None);
            self.ui().set_undo_text(SharedString::new());
        }
    }

    /// Undo (or Ctrl+Z) on the offer shown.
    pub(super) fn undo_clicked(&mut self) {
        match self.undo.get() {
            UndoOffer::None => {}
            UndoOffer::Send(row) => self.undo_send(row),
            UndoOffer::Move(seq) => self.undo_held(seq),
        }
    }

    /// Start a held move's undo window (`seq`).
    pub(super) fn start_undo_clock(&mut self, seq: u64) {
        let left = self.undo_clock.start(Instant::now());
        self.arm_undo_timer(seq, left);
    }

    fn arm_undo_timer(&self, seq: u64, after: Duration) {
        self.undo_timer
            .start(TimerMode::SingleShot, after, move || {
                with_app(|a| {
                    if a.held.as_ref().is_some_and(|h| h.seq == seq)
                        && a.undo_clock.is_due(Instant::now())
                    {
                        a.flush_held();
                    }
                });
            });
    }

    /// The pointer is over the offer, or it has the focus (`on`): a move's
    /// window stops, and goes on (for at least [`UNDO_RESUME`]) after.
    pub(super) fn undo_hold(&mut self, on: bool) {
        let UndoOffer::Move(seq) = self.undo.get() else {
            return;
        };
        let now = Instant::now();
        if on {
            self.undo_clock.pause(now);
            self.undo_timer.stop();
        } else if let Some(left) = self.undo_clock.resume(now) {
            self.arm_undo_timer(seq, left);
        }
    }

    // ------------------------------------------------------ the rest

    /// Ctrl+Shift+I: All inboxes with two or more accounts, else the
    /// account's Inbox.
    pub(super) fn go_inbox(&mut self) {
        if self.accounts.len() > 1 {
            return self.set_view(View::AllInboxes);
        }
        if let Some(inbox) = self.folders.iter().find(|f| f.role == Role::Inbox) {
            self.set_view(View::Folder(inbox.id));
        }
    }

    /// "Select messages" in the list's "..." menu: check boxes on or off.
    pub(super) fn set_select_mode(&mut self, on: bool) {
        self.select_mode = on && self.lists_messages();
        self.ui().set_select_mode(self.select_mode);
    }

    /// The "N selected" panel's Clear selection.
    pub(super) fn clear_selection(&mut self) {
        self.select_rows(|s, _| s.clear());
    }

    /// The open message's server can't be reached now (no worker for its
    /// account, or its last sync couldn't reach it).
    pub(super) fn unreachable(&self, account: i64) -> bool {
        !self.workers.contains_key(&account)
            || matches!(
                self.account_state.get(&account),
                Some(AccountState::Unreachable | AccountState::Failed(_))
            )
    }

    /// Retry on a message not downloaded yet.
    pub(super) fn retry_body(&mut self) {
        let Some(open) = self.open.as_ref().filter(|o| o.msg.raw.is_none()) else {
            return;
        };
        let (id, account) = (open.msg.id, open.msg.account_id);
        if let Some(w) = self.workers.get(&account) {
            w.send(Job::FetchBody(id));
        }
        self.ui().set_r_unreachable(self.unreachable(account));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn option(id: i32, label: &str, level: i32) -> FolderOption {
        FolderOption {
            id,
            label: label.into(),
            level,
            recent: false,
        }
    }

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
        let ids: Vec<(i32, bool)> = got.iter().map(|o| (o.id, o.recent)).collect();
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

    #[test]
    fn menu_entries_carry_their_accelerator_after_a_tab() {
        let e = entry("Reply".into(), "Ctrl+R", true, CMD_REPLY);
        assert_eq!(e.title, "Reply\tCtrl+R");
        assert!(!e.checkable);
        assert_eq!(
            entry("Show original".into(), "", false, CMD_ORIGINAL).title,
            "Show original"
        );
        let c = check_entry("View as plain text".into(), true, true, CMD_PLAIN);
        assert!(c.checkable && c.checked && c.enabled);
    }
}
