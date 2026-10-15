//! The folder pane's tree: which folder sits under which, in what order,
//! and which parents are folded. The cache stores no hierarchy delimiter,
//! so it is inferred per account from the folders' names and depths (no
//! migration). Pure and panic-free: any folder list gives a tree.

use crate::db::Folder;
use crate::role::Role;
use std::collections::{HashMap, HashSet};

/// The deepest level drawn; deeper folders show at this level (their
/// tooltip and accessible label still give the full path).
pub const MAX_LEVEL: usize = 5;

/// Hierarchy delimiters, in order of preference.
const DELIMITERS: [char; 5] = ['/', '.', '\\', ':', '|'];

/// One folder in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeNode {
    /// The folder's index in the list given to [`tree`].
    pub index: usize,
    /// The level drawn: 0 for a root, capped at [`MAX_LEVEL`].
    pub level: usize,
    /// The true depth in the tree (uncapped).
    pub depth: usize,
    /// The parent folder's index in the list given to [`tree`].
    pub parent: Option<usize>,
    pub has_children: bool,
}

/// The folders as a tree, in display order, account by account (in the
/// order the accounts first appear in `folders`). Role folders are roots,
/// by role rank; user folders follow by name; each node is followed by
/// its subtree, siblings sorted the same way. A user folder hangs under
/// the folder its name continues (`INBOX/Receipts` under Inbox,
/// `Projects/Alpha` under Projects), or under the nearest listed ancestor
/// when its parent is not listed.
pub fn tree(folders: &[Folder]) -> Vec<TreeNode> {
    let mut accounts: Vec<i64> = Vec::new();
    let mut by_account: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, f) in folders.iter().enumerate() {
        by_account
            .entry(f.account_id)
            .or_insert_with(|| {
                accounts.push(f.account_id);
                Vec::new()
            })
            .push(i);
    }
    let mut out = Vec::with_capacity(folders.len());
    for account in accounts {
        if let Some(members) = by_account.get(&account) {
            account_tree(folders, members, &mut out);
        }
    }
    out
}

/// The delimiter an account's folder names use: the one whose count in
/// each nested user folder's name equals its depth, and which no top-level
/// user folder's name contains. `None`: the account is flat.
pub fn delimiter<'a>(folders: impl Iterator<Item = &'a Folder> + Clone) -> Option<char> {
    let others = folders.filter(|f| f.role == Role::Other);
    let mut nested = others.clone().filter(|f| f.depth > 0).peekable();
    nested.peek()?;
    DELIMITERS.into_iter().find(|&c| {
        nested
            .clone()
            .all(|f| i64::try_from(f.name.matches(c).count()).is_ok_and(|n| n == f.depth))
            && others
                .clone()
                .filter(|f| f.depth == 0)
                .all(|f| !f.name.contains(c))
    })
}

fn account_tree(folders: &[Folder], members: &[usize], out: &mut Vec<TreeNode>) {
    let get = |i: usize| folders.get(i);
    let delim = delimiter(members.iter().filter_map(|&i| get(i)));
    let by_name: HashMap<&str, usize> = members
        .iter()
        .filter_map(|&i| Some((get(i)?.name.as_str(), i)))
        .collect();
    let inbox = members
        .iter()
        .copied()
        .find(|&i| get(i).is_some_and(|f| f.role == Role::Inbox));
    let lookup = |name: &str| -> Option<usize> {
        by_name.get(name).copied().or_else(|| {
            name.eq_ignore_ascii_case("INBOX")
                .then_some(inbox)
                .flatten()
        })
    };
    // Parents: user folders only; role folders stay roots.
    let mut parent: HashMap<usize, usize> = HashMap::new();
    if let Some(d) = delim {
        for &i in members {
            let Some(f) = get(i).filter(|f| f.role == Role::Other) else {
                continue;
            };
            let mut prefix = f.name.as_str();
            while let Some((head, _)) = prefix.rsplit_once(d) {
                prefix = head;
                if let Some(p) = lookup(prefix).filter(|&p| p != i) {
                    parent.insert(i, p);
                    break;
                }
            }
        }
    }
    let key = |i: usize| get(i).map(|f| (f.role.rank(), f.display.to_lowercase(), f.name.as_str()));
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for &i in members {
        match parent.get(&i) {
            Some(&p) => children.entry(p).or_default().push(i),
            None => roots.push(i),
        }
    }
    let sort = |list: &mut Vec<usize>| list.sort_by_cached_key(|&i| key(i));
    sort(&mut roots);
    for list in children.values_mut() {
        sort(list);
    }
    // Depth first, without recursion; `seen` guards against any cycle.
    let mut seen: HashSet<usize> = HashSet::new();
    let mut stack: Vec<(usize, Option<usize>, usize)> =
        roots.iter().rev().map(|&i| (i, None, 0)).collect();
    while let Some((i, p, depth)) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        let kids = children.get(&i).map(Vec::as_slice).unwrap_or_default();
        out.push(TreeNode {
            index: i,
            level: depth.min(MAX_LEVEL),
            depth,
            parent: p,
            has_children: !kids.is_empty(),
        });
        stack.extend(kids.iter().rev().map(|&k| (k, Some(i), depth + 1)));
    }
}

/// A folded line of the folder pane: an account or a parent folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Fold {
    Account(i64),
    Folder(i64),
}

/// The folded lines from the `nav_folded` setting (`a2,f117,f230`).
/// Unreadable tokens are skipped.
pub fn parse_folds(value: &str) -> HashSet<Fold> {
    value
        .split(',')
        .filter_map(|t| {
            let t = t.trim();
            let id = |s: &str| s.parse::<i64>().ok().filter(|&n| n > 0);
            if let Some(n) = t.strip_prefix('a') {
                id(n).map(Fold::Account)
            } else {
                id(t.strip_prefix('f')?).map(Fold::Folder)
            }
        })
        .collect()
}

/// The folded lines as the setting stores them, sorted.
pub fn format_folds(folds: &HashSet<Fold>) -> String {
    let mut list: Vec<Fold> = folds.iter().copied().collect();
    list.sort();
    list.iter()
        .map(|f| match f {
            Fold::Account(id) => format!("a{id}"),
            Fold::Folder(id) => format!("f{id}"),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Drop the folds of accounts and folders that are gone. A folder's fold
/// is only dropped once every account has its folder list (a new account
/// before its first sync has none), so nothing is lost on a cold start.
/// True when something was dropped.
pub fn prune_folds(folds: &mut HashSet<Fold>, accounts: &[i64], folders: &[Folder]) -> bool {
    let before = folds.len();
    let complete = accounts
        .iter()
        .all(|a| folders.iter().any(|f| f.account_id == *a));
    let ids: HashSet<i64> = folders.iter().map(|f| f.id).collect();
    folds.retain(|f| match f {
        Fold::Account(a) => accounts.contains(a),
        Fold::Folder(id) => !complete || ids.contains(id),
    });
    folds.len() != before
}

/// The lines folded for an account when its folders first arrive, so a
/// long tree starts short: every user parent
/// folder (any depth under the account line; role folders such as an
/// Inbox with subfolders stay open) without unread mail of its own, and,
/// with more than one account, the account line itself after the first
/// (`position` is its place in the account order). A folded parent shows
/// its subtree's unread, so nothing unread is hidden without a count.
pub fn default_folds(
    account: i64,
    position: usize,
    accounts: usize,
    folders: &[Folder],
    unread: &HashMap<i64, i64>,
) -> Vec<Fold> {
    let mut folds: Vec<Fold> = tree(folders)
        .iter()
        .filter(|n| n.has_children)
        .filter_map(|n| folders.get(n.index))
        .filter(|f| f.account_id == account && f.role == Role::Other)
        .filter(|f| unread.get(&f.id).is_none_or(|&n| n == 0))
        .map(|f| Fold::Folder(f.id))
        .collect();
    if accounts > 1 && position > 0 {
        folds.push(Fold::Account(account));
    }
    folds
}

/// Account ids from a comma-separated setting (`nav_folds_init`).
/// Unreadable tokens are skipped.
pub fn parse_ids(value: &str) -> HashSet<i64> {
    value
        .split(',')
        .filter_map(|t| t.trim().parse::<i64>().ok().filter(|&n| n > 0))
        .collect()
}

/// The setting back, sorted.
pub fn format_ids(ids: &HashSet<i64>) -> String {
    let mut list: Vec<i64> = ids.iter().copied().collect();
    list.sort_unstable();
    list.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: i64, account: i64, name: &str, depth: i64, role: Role) -> Folder {
        let display = if role == Role::Inbox {
            "Inbox".to_string()
        } else {
            name.rsplit(['/', '.']).next().unwrap_or(name).to_string()
        };
        Folder {
            id,
            account_id: account,
            name: name.into(),
            display,
            depth,
            role,
            uidvalidity: 0,
        }
    }

    fn other(id: i64, name: &str, depth: i64) -> Folder {
        folder(id, 1, name, depth, Role::Other)
    }

    /// (name, level, parent name) in display order.
    fn shape(folders: &[Folder]) -> Vec<(String, usize, Option<String>)> {
        tree(folders)
            .iter()
            .map(|n| {
                (
                    folders[n.index].name.clone(),
                    n.level,
                    n.parent.map(|p| folders[p].name.clone()),
                )
            })
            .collect()
    }

    fn names(folders: &[Folder]) -> Vec<String> {
        shape(folders).into_iter().map(|(n, ..)| n).collect()
    }

    #[test]
    fn a_sibling_sorting_between_parent_and_child_follows_the_subtree() {
        let f = [
            other(1, "Projects-old", 0),
            other(2, "Projects/Alpha", 1),
            other(3, "Projects", 0),
        ];
        assert_eq!(
            shape(&f),
            [
                ("Projects".into(), 0, None),
                ("Projects/Alpha".into(), 1, Some("Projects".into())),
                ("Projects-old".into(), 0, None),
            ]
        );
        let t = tree(&f);
        assert!(t[0].has_children);
        assert!(!t[1].has_children && !t[2].has_children);
    }

    #[test]
    fn children_of_role_folders_hang_under_them() {
        let f = [
            folder(1, 1, "INBOX", 0, Role::Inbox),
            folder(2, 1, "Trash", 0, Role::Trash),
            other(3, "INBOX/Receipts", 1),
            folder(4, 1, "Archive", 0, Role::Archive),
            other(5, "Archive/2024", 1),
            other(6, "Notes", 0),
        ];
        assert_eq!(
            shape(&f),
            [
                ("INBOX".into(), 0, None),
                ("INBOX/Receipts".into(), 1, Some("INBOX".into())),
                ("Archive".into(), 0, None),
                ("Archive/2024".into(), 1, Some("Archive".into())),
                ("Trash".into(), 0, None),
                ("Notes".into(), 0, None),
            ]
        );
    }

    #[test]
    fn a_dot_delimiter_and_courier_style_names() {
        let f = [
            folder(1, 1, "INBOX", 0, Role::Inbox),
            folder(2, 1, "INBOX.Archive", 0, Role::Archive),
            other(3, "INBOX.Archive.2024", 2),
            other(4, "INBOX.Clients", 1),
            folder(5, 1, "INBOX.Sent", 0, Role::Sent),
        ];
        assert_eq!(delimiter(f.iter()), Some('.'));
        assert_eq!(
            shape(&f),
            [
                ("INBOX".into(), 0, None),
                ("INBOX.Clients".into(), 1, Some("INBOX".into())),
                ("INBOX.Sent".into(), 0, None),
                ("INBOX.Archive".into(), 0, None),
                ("INBOX.Archive.2024".into(), 1, Some("INBOX.Archive".into())),
            ]
        );
    }

    #[test]
    fn a_dot_in_a_top_level_name_rules_out_the_dot() {
        let f = [
            other(1, "Q1.2024", 0),
            other(2, "Work", 0),
            other(3, "Work/v1.2", 1),
        ];
        assert_eq!(delimiter(f.iter()), Some('/'));
        assert_eq!(
            shape(&f),
            [
                ("Q1.2024".into(), 0, None),
                ("Work".into(), 0, None),
                ("Work/v1.2".into(), 1, Some("Work".into())),
            ]
        );
    }

    #[test]
    fn an_orphan_hangs_under_its_nearest_listed_ancestor() {
        let f = [
            other(1, "A", 0),
            // "A/B" is not listed (a \Noselect parent).
            other(2, "A/B/C", 2),
            // Nothing above it is listed: a root.
            other(3, "X/Y", 1),
        ];
        assert_eq!(
            shape(&f),
            [
                ("A".into(), 0, None),
                ("A/B/C".into(), 1, Some("A".into())),
                ("X/Y".into(), 0, None),
            ]
        );
    }

    #[test]
    fn a_flat_server_has_one_level() {
        let f = [
            folder(1, 1, "INBOX", 0, Role::Inbox),
            other(2, "a.b", 0),
            other(3, "c/d", 0),
        ];
        assert_eq!(delimiter(f.iter()), None);
        assert!(tree(&f).iter().all(|n| n.level == 0 && n.parent.is_none()));
    }

    #[test]
    fn siblings_sort_without_regard_to_case() {
        let f = [
            other(1, "beta", 0),
            other(2, "Alpha", 0),
            other(3, "Gamma", 0),
        ];
        assert_eq!(names(&f), ["Alpha", "beta", "Gamma"]);
    }

    #[test]
    fn three_levels_and_the_level_cap() {
        let f = [
            other(1, "a", 0),
            other(2, "a/b", 1),
            other(3, "a/b/c", 2),
            other(4, "a/b/c/d", 3),
            other(5, "a/b/c/d/e", 4),
            other(6, "a/b/c/d/e/f", 5),
            other(7, "a/b/c/d/e/f/g", 6),
        ];
        let t = tree(&f);
        let levels: Vec<usize> = t.iter().map(|n| n.level).collect();
        assert_eq!(levels, [0, 1, 2, 3, 4, 5, 5]);
        assert_eq!(t[6].depth, 6);
        assert_eq!(t[2].parent, Some(1));
    }

    #[test]
    fn accounts_keep_their_order_and_their_own_folders() {
        let f = [
            folder(1, 7, "INBOX", 0, Role::Inbox),
            folder(2, 3, "INBOX", 0, Role::Inbox),
            folder(3, 3, "INBOX/x", 1, Role::Other),
            folder(4, 7, "INBOX/y", 1, Role::Other),
        ];
        let t = tree(&f);
        let order: Vec<i64> = t.iter().map(|n| f[n.index].id).collect();
        assert_eq!(order, [1, 4, 2, 3]);
        assert_eq!(t[1].parent, Some(0));
        assert_eq!(t[3].parent, Some(1));
    }

    #[test]
    fn five_hundred_folders() {
        let mut f = vec![folder(1, 1, "INBOX", 0, Role::Inbox)];
        for i in 0..500 {
            let depth = i % 4;
            let name = (0..=depth)
                .map(|d| format!("n{}", i - d))
                .collect::<Vec<_>>()
                .join("/");
            f.push(other(i as i64 + 2, &name, depth as i64));
        }
        let t = tree(&f);
        assert_eq!(t.len(), f.len());
        let mut seen: Vec<usize> = t.iter().map(|n| n.index).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), f.len());
        // Every parent comes before its children.
        let pos: HashMap<usize, usize> = t.iter().enumerate().map(|(k, n)| (n.index, k)).collect();
        assert!(
            t.iter()
                .all(|n| n.parent.is_none_or(|p| pos[&p] < pos[&n.index]))
        );
    }

    #[test]
    fn folds_round_trip_and_skip_unreadable_tokens() {
        let folds = parse_folds("a2, f117,x9,f,a-1,f230,a2");
        assert_eq!(
            folds,
            HashSet::from([Fold::Account(2), Fold::Folder(117), Fold::Folder(230)])
        );
        assert_eq!(format_folds(&folds), "a2,f117,f230");
        assert!(parse_folds("").is_empty());
        assert!(parse_folds("garbage").is_empty());
    }

    #[test]
    fn pruning_waits_for_every_account_s_folders() {
        let f = [folder(1, 1, "INBOX", 0, Role::Inbox), other(2, "P", 0)];
        let mut folds = HashSet::from([
            Fold::Account(1),
            Fold::Account(9),
            Fold::Folder(2),
            Fold::Folder(50),
        ]);
        // Account 3 has no folders yet: folder folds stay.
        assert!(prune_folds(&mut folds, &[1, 3], &f));
        assert_eq!(
            folds,
            HashSet::from([Fold::Account(1), Fold::Folder(2), Fold::Folder(50)])
        );
        assert!(prune_folds(&mut folds, &[1], &f));
        assert_eq!(folds, HashSet::from([Fold::Account(1), Fold::Folder(2)]));
        assert!(!prune_folds(&mut folds, &[1], &f));
    }
    #[test]
    fn long_trees_start_folded() {
        let folders = vec![
            folder(1, 1, "INBOX", 0, Role::Inbox),
            folder(2, 1, "INBOX/Receipts", 1, Role::Other),
            folder(3, 1, "Projects", 0, Role::Other),
            folder(4, 1, "Projects/Alpha", 1, Role::Other),
            folder(5, 1, "Projects/Alpha/Q1", 2, Role::Other),
            folder(6, 1, "News", 0, Role::Other),
            folder(7, 1, "News/Daily", 1, Role::Other),
            folder(8, 2, "INBOX", 0, Role::Inbox),
            folder(9, 2, "Team", 0, Role::Other),
            folder(10, 2, "Team/Standups", 1, Role::Other),
        ];
        // News holds unread mail of its own: it stays open; Alpha's
        // subfolder's unread doesn't keep Alpha (or Projects) open.
        let unread = HashMap::from([(6, 2), (5, 4)]);
        let mut first = default_folds(1, 0, 2, &folders, &unread);
        first.sort();
        // The Inbox (a role folder) stays open; the first account too.
        assert_eq!(first, [Fold::Folder(3), Fold::Folder(4)]);
        let mut second = default_folds(2, 1, 2, &folders, &unread);
        second.sort();
        assert_eq!(second, [Fold::Account(2), Fold::Folder(9)]);
        // One account: never folded itself.
        assert_eq!(default_folds(2, 1, 1, &folders, &unread), [Fold::Folder(9)]);
        // The setting that records which accounts had them.
        let ids = parse_ids(" 3,1,x,-2,");
        assert_eq!(ids, HashSet::from([1, 3]));
        assert_eq!(format_ids(&ids), "1,3");
    }
}
