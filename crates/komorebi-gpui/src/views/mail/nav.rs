//! The folder pane's lines and the rail's tiles (the Slint app's
//! `rebuild_nav`, `nav_*` and `parents_of` in `src/app.rs`): All inboxes
//! with two or more accounts, Drafts, the Outbox while it holds mail, then
//! each account's line and its folder tree (`komorebi::tree`), folded where
//! the user folded it; unread counts, a folded parent's being its whole
//! subtree's. Lines are built when drawn, so their words follow the
//! language; the counts are the model's.

use std::collections::HashMap;

use komorebi::db::Folder;
use komorebi::role::Role;
use komorebi::tree::{self, Fold};

use super::model::{Mail, View};
use super::text;

/// What a line of the folder pane is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavKind {
    AllInboxes,
    Drafts,
    Outbox,
    Account(i64),
    Folder(i64),
}

/// One line of the folder pane (or tile of the rail).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavItem {
    pub kind: NavKind,
    pub label: String,
    /// The folder's role, for its icon (`None`: a line of its own).
    pub role: Option<Role>,
    /// Unread mail (a folded line: its whole subtree's).
    pub unread: i64,
    /// A quiet total (Drafts, the Outbox, a server Drafts folder).
    pub count: i64,
    /// The level drawn (0 to `tree::MAX_LEVEL`).
    pub level: usize,
    pub has_children: bool,
    pub expanded: bool,
    /// The line above it in the tree (Left goes there).
    pub parent: Option<usize>,
    pub account: i64,
    /// The full path of a folder deeper than the levels drawn, or the
    /// account's address under its name.
    pub sub: String,
    /// The account is switched off.
    pub off: bool,
}

/// The pane's lines and where the pill is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    pub items: Vec<NavItem>,
    /// The line of the open view (or the line it is folded into).
    pub pill: Option<usize>,
}

/// What a line's menu offers (`None` from [`Mail::nav_menu`]: no menu).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavMenu {
    /// Go to the account's Inbox, Sent, Archive (a rail tile): the folder,
    /// if the account has one.
    pub open: Vec<(Role, Option<i64>)>,
    /// Mark all as read, and whether anything is unread.
    pub mark_read: Option<bool>,
    /// Collapse all (folders, on an account line), and whether a parent is
    /// open.
    pub collapse: Option<(bool, bool)>,
    /// Refresh and Account settings (an account line), Refresh off while
    /// the account is.
    pub account: Option<bool>,
}

fn line(kind: NavKind, label: String) -> NavItem {
    NavItem {
        kind,
        label,
        role: None,
        unread: 0,
        count: 0,
        level: 0,
        has_children: false,
        expanded: true,
        parent: None,
        account: 0,
        sub: String::new(),
        off: false,
    }
}

impl Mail {
    fn count_of(&self, unread: &HashMap<i64, i64>, f: &Folder) -> i64 {
        if f.role.shows_unread() {
            unread.get(&f.id).copied().unwrap_or(0)
        } else {
            0
        }
    }

    /// The server Drafts' total, quietly, less the copies of local drafts
    /// (the list leaves those out) and the messages a move holds.
    fn drafts_total(&self, f: &Folder) -> i64 {
        let held: i64 = self.held.iter().filter_map(|h| h.total.get(&f.id)).sum();
        let copies = self.db.draft_copies(&[f.id]).map_or(0, |c| c.len() as i64);
        self.db
            .count_messages(f.id)
            .map_or(0, |n| (i64::from(n) - held - copies).max(0))
    }

    /// The folder pane's lines.
    pub fn nav(&self) -> Nav {
        if self.accounts.is_empty() {
            return Nav::default();
        }
        let unread = self.shown_unread();
        let inbox_unread: i64 = self
            .folders
            .iter()
            .filter(|f| f.role == Role::Inbox)
            .map(|f| self.count_of(&unread, f))
            .sum();
        let mut items = Vec::new();
        if self.accounts.len() > 1 {
            items.push(NavItem {
                unread: inbox_unread,
                ..line(NavKind::AllInboxes, text::all_inboxes())
            });
        }
        items.push(NavItem {
            count: self.draft_count,
            ..line(NavKind::Drafts, text::drafts())
        });
        if self.outbox_count > 0 || self.view == View::Outbox {
            items.push(NavItem {
                count: self.outbox_count,
                ..line(NavKind::Outbox, text::outbox())
            });
        }
        let nodes = tree::tree(&self.folders);
        let n = self.folders.len();
        // Each folder's unread with its subfolders' (shown on a folded
        // parent): children come after their parents.
        let mut sum: Vec<i64> = self
            .folders
            .iter()
            .map(|f| self.count_of(&unread, f))
            .collect();
        for node in nodes.iter().rev() {
            if let Some(p) = node.parent
                && p < n
                && let Some(&own) = sum.get(node.index)
            {
                sum[p] = sum[p].saturating_add(own);
            }
        }
        // The line showing each folder: its own, or while it is folded
        // away, its nearest shown line's; and whether its subfolders show.
        let mut row = vec![0usize; n];
        let mut reach = vec![false; n];
        for a in &self.accounts {
            let head = items.len();
            let open = !self.folded.contains(&Fold::Account(a.id));
            items.push(NavItem {
                unread: self
                    .folders
                    .iter()
                    .filter(|f| f.account_id == a.id && f.role == Role::Inbox)
                    .map(|f| self.count_of(&unread, f))
                    .sum(),
                expanded: open,
                has_children: true,
                account: a.id,
                sub: a.email.clone(),
                off: !a.enabled,
                ..line(NavKind::Account(a.id), Mail::account_label(a).to_string())
            });
            for node in &nodes {
                let Some(f) = self
                    .folders
                    .get(node.index)
                    .filter(|f| f.account_id == a.id)
                else {
                    continue;
                };
                let (above, shown) = match node.parent {
                    Some(p) if p < n => (row[p], reach[p]),
                    _ => (head, open),
                };
                if !shown {
                    row[node.index] = above;
                    continue;
                }
                let folded = node.has_children && self.folded.contains(&Fold::Folder(f.id));
                row[node.index] = items.len();
                reach[node.index] = !folded;
                items.push(NavItem {
                    kind: NavKind::Folder(f.id),
                    label: text::folder_label(f),
                    role: Some(f.role),
                    unread: if folded {
                        sum.get(node.index).copied().unwrap_or(0)
                    } else {
                        self.count_of(&unread, f)
                    },
                    count: if f.role == Role::Drafts {
                        self.drafts_total(f)
                    } else {
                        0
                    },
                    level: node.level,
                    has_children: node.has_children,
                    expanded: node.has_children && !folded,
                    parent: Some(above),
                    account: a.id,
                    sub: if node.depth > tree::MAX_LEVEL {
                        self.folder_path(&nodes, node.index)
                    } else {
                        String::new()
                    },
                    off: !a.enabled,
                });
            }
        }
        let pill = match self.view {
            View::AllInboxes => items.iter().position(|r| r.kind == NavKind::AllInboxes),
            View::Drafts => items.iter().position(|r| r.kind == NavKind::Drafts),
            View::Outbox => items.iter().position(|r| r.kind == NavKind::Outbox),
            View::Folder(id) => self.folders.iter().position(|f| f.id == id).map(|i| row[i]),
        };
        Nav { items, pill }
    }

    /// The rail's tiles: the unified lines (All inboxes, Drafts, the
    /// Outbox), then with one account its top-level folders (the brand's
    /// compact mockup), with several one tile per account that opens its
    /// Inbox (the Slint app's rail).
    pub fn rail(&self) -> Nav {
        let nav = self.nav();
        let mut items: Vec<NavItem> = nav
            .items
            .iter()
            .filter(|r| {
                matches!(
                    r.kind,
                    NavKind::AllInboxes | NavKind::Drafts | NavKind::Outbox
                )
            })
            .cloned()
            .collect();
        if let [only] = self.accounts.as_slice() {
            items.extend(
                nav.items
                    .iter()
                    .filter(|r| {
                        r.account == only.id && r.level == 0 && matches!(r.kind, NavKind::Folder(_))
                    })
                    .cloned(),
            );
        } else {
            for a in &self.accounts {
                let inbox = self
                    .folders
                    .iter()
                    .find(|f| f.account_id == a.id && f.role == Role::Inbox);
                let unread = self.shown_unread();
                items.push(NavItem {
                    unread: inbox.map_or(0, |f| self.count_of(&unread, f)),
                    account: a.id,
                    sub: a.email.clone(),
                    off: !a.enabled,
                    ..line(NavKind::Account(a.id), Mail::account_label(a).to_string())
                });
            }
        }
        let pill = match self.view {
            View::AllInboxes => items.iter().position(|r| r.kind == NavKind::AllInboxes),
            View::Drafts => items.iter().position(|r| r.kind == NavKind::Drafts),
            View::Outbox => items.iter().position(|r| r.kind == NavKind::Outbox),
            View::Folder(id) => items
                .iter()
                .position(|r| r.kind == NavKind::Folder(id))
                .or_else(|| {
                    let account = self.folder(id)?.account_id;
                    items
                        .iter()
                        .position(|r| r.kind == NavKind::Account(account))
                }),
        };
        Nav { items, pill }
    }

    /// A folder's full path in the pane ("Projects / Alpha / ..."), for
    /// folders deeper than the levels drawn.
    fn folder_path(&self, nodes: &[tree::TreeNode], index: usize) -> String {
        let parent: HashMap<usize, usize> = nodes
            .iter()
            .filter_map(|n| Some((n.index, n.parent?)))
            .collect();
        let mut parts = Vec::new();
        let mut at = Some(index);
        while let Some(i) = at.filter(|_| parts.len() <= nodes.len()) {
            if let Some(f) = self.folders.get(i) {
                parts.push(text::folder_label(f));
            }
            at = parent.get(&i).copied();
        }
        parts.reverse();
        parts.join(" / ")
    }

    /// A line was clicked (or Enter or Space on it): a view opens, an
    /// account line folds or opens. A rail tile of an account opens its
    /// Inbox.
    pub fn nav_clicked(&mut self, kind: NavKind, rail: bool) {
        match kind {
            NavKind::AllInboxes => self.set_view(View::AllInboxes),
            NavKind::Drafts => self.set_view(View::Drafts),
            NavKind::Outbox => self.set_view(View::Outbox),
            NavKind::Account(id) if rail => {
                if let Some(inbox) = self
                    .folders
                    .iter()
                    .find(|f| f.account_id == id && f.role == Role::Inbox)
                {
                    let inbox = inbox.id;
                    self.set_view(View::Folder(inbox));
                }
            }
            NavKind::Account(_) => self.nav_toggle(kind),
            NavKind::Folder(id) => self.set_view(View::Folder(id)),
        }
    }

    /// Fold or open an account line or a parent folder. The view stays.
    pub fn nav_toggle(&mut self, kind: NavKind) {
        let fold = match kind {
            NavKind::Account(id) => Fold::Account(id),
            NavKind::Folder(id) if self.parents_of(None).contains(&id) => Fold::Folder(id),
            _ => return,
        };
        if !self.folded.remove(&fold) {
            self.folded.insert(fold);
        }
        self.save_folds();
    }

    /// Every parent folder, of every account or of one.
    pub fn parents_of(&self, account: Option<i64>) -> Vec<i64> {
        tree::tree(&self.folders)
            .iter()
            .filter(|n| n.has_children)
            .filter_map(|n| self.folders.get(n.index))
            .filter(|f| account.is_none_or(|a| f.account_id == a))
            .map(|f| f.id)
            .collect()
    }

    /// The folders a line's Mark all as read covers: every Inbox (All
    /// inboxes), the account's Inbox (as its folded count), or the folder.
    pub fn nav_menu_folders(&self, kind: NavKind) -> Vec<i64> {
        let inboxes = |account: Option<i64>| {
            self.folders
                .iter()
                .filter(|f| f.role == Role::Inbox && account.is_none_or(|a| f.account_id == a))
                .map(|f| f.id)
                .collect()
        };
        match kind {
            NavKind::AllInboxes => inboxes(None),
            NavKind::Account(id) => inboxes(Some(id)),
            NavKind::Folder(id) => vec![id],
            NavKind::Drafts | NavKind::Outbox => Vec::new(),
        }
    }

    /// A line's menu (none for Drafts and the Outbox): a rail tile goes to
    /// its account's Inbox, Sent or Archive; other lines have Mark all as
    /// read (no prompt, no Undo, as in Windows Mail) and Collapse all; an
    /// account line also Refresh and Account settings. Never "Empty
    /// folder" (it would expunge on the server).
    pub fn nav_menu(&self, kind: NavKind, rail: bool) -> Option<NavMenu> {
        if rail && let NavKind::Account(account) = kind {
            let go = |role: Role| {
                let folder = self
                    .folders
                    .iter()
                    .find(|f| f.account_id == account && f.role == role);
                (role, folder.map(|f| f.id))
            };
            return Some(NavMenu {
                open: vec![go(Role::Inbox), go(Role::Sent), go(Role::Archive)],
                ..NavMenu::default()
            });
        }
        if matches!(kind, NavKind::Drafts | NavKind::Outbox) {
            return None;
        }
        let unread = self.shown_unread();
        // A server Drafts folder's rows are never shown unread.
        let drafts = matches!(kind, NavKind::Folder(id) if self.folder_role(id) == Role::Drafts);
        let has_unread = !drafts
            && self
                .nav_menu_folders(kind)
                .iter()
                .any(|f| unread.get(f).is_some_and(|&n| n > 0));
        let account = match kind {
            NavKind::Account(id) => Some(id),
            _ => None,
        };
        // Nothing to collapse under a folded account line.
        let open_parent = account.is_none_or(|a| !self.folded.contains(&Fold::Account(a)))
            && self
                .parents_of(account)
                .iter()
                .any(|id| !self.folded.contains(&Fold::Folder(*id)));
        let off = account
            .and_then(|a| self.accounts.iter().find(|x| x.id == a))
            .is_some_and(|a| !a.enabled);
        Some(NavMenu {
            open: Vec::new(),
            mark_read: Some(has_unread),
            collapse: Some((account.is_some(), open_parent)),
            account: account.map(|_| !off),
        })
    }

    /// Mark all as read from a line's menu.
    pub fn nav_mark_read(&mut self, kind: NavKind) {
        let folders = self.nav_menu_folders(kind);
        self.mark_folders_read(&folders);
    }

    /// Collapse all: every parent folder (of the account, on its line).
    pub fn collapse_all(&mut self, kind: NavKind) {
        let only = match kind {
            NavKind::Account(id) => Some(id),
            _ => None,
        };
        for id in self.parents_of(only) {
            self.folded.insert(Fold::Folder(id));
        }
        self.save_folds();
    }

    /// Hide folders / Show folders: the pane folds to the rail at every
    /// width (`nav_collapsed`).
    pub fn set_nav_collapsed(&mut self, collapsed: bool) {
        self.nav_collapsed = collapsed;
        self.db
            .set_setting("nav_collapsed", if collapsed { "1" } else { "0" })
            .ok();
    }

    pub fn is_folded(&self, fold: Fold) -> bool {
        self.folded.contains(&fold)
    }
}
