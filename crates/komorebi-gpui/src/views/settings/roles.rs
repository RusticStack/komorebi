//! Folder roles in Edit account > Folders (the Slint app's
//! `src/app/roles.rs`): the folder Sent mail is copied to, drafts are saved
//! in, and Delete, Archive and Report junk move to, for servers that don't
//! mark them (or mark the wrong one). Each role offers "Automatic" (what the
//! server marks) and the account's folders; the picks are kept in the open
//! form and written when Save succeeds. Nothing is created, renamed or moved
//! on the server.

use komorebi::db::Db;
use komorebi::mail::imap::decode_mutf7;
use komorebi::role::Role;

use super::text;

/// The roles that can be chosen, in the order the form lists them.
pub const CHOOSABLE: [Role; 5] = [
    Role::Sent,
    Role::Drafts,
    Role::Trash,
    Role::Archive,
    Role::Junk,
];

/// The folders offered to the open form, and what is picked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleEdit {
    pub account: i64,
    /// The folder names offered, decoded and sorted (the same list for every
    /// role).
    pub names: Vec<String>,
    /// Per role, the name of the folder the server marks for it.
    pub detected: Vec<Option<String>>,
    /// Per role, the folder id of each option (0: Automatic).
    pub ids: Vec<Vec<i64>>,
    /// Per role, the option shown when the form opened.
    pub first: Vec<usize>,
    /// Per role, the option picked now.
    pub picked: Vec<usize>,
}

impl RoleEdit {
    /// The folders of `account` and the picks in the cache (the Slint app's
    /// `show_role_choices`). Empty when nothing has synced yet: no folder is
    /// offered, and an inbox never is.
    pub fn load(db: &Db, account: i64) -> Self {
        let mut folders: Vec<_> = db
            .folders()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.account_id == account && f.role != Role::Inbox)
            .collect();
        folders.sort_by_key(|f| decode_mutf7(&f.name).to_lowercase());
        if folders.is_empty() {
            return Self::default();
        }
        let mut edit = RoleEdit {
            account,
            names: folders.iter().map(|f| decode_mutf7(&f.name)).collect(),
            ..Default::default()
        };
        for role in CHOOSABLE {
            let detected = db.detected_folder(account, role).ok().flatten();
            let chosen = db.chosen_folder(account, role).ok().flatten();
            let mut ids = vec![0];
            ids.extend(folders.iter().map(|f| f.id));
            let current = chosen
                .and_then(|c| ids.iter().position(|id| *id == c.id))
                .unwrap_or(0);
            edit.detected.push(detected.map(|f| f.display));
            edit.ids.push(ids);
            edit.first.push(current);
            edit.picked.push(current);
        }
        edit
    }

    /// The (role, folder id; 0 for Automatic) picked differently from when
    /// the form opened.
    pub fn changes(&self) -> Vec<(Role, i64)> {
        CHOOSABLE
            .iter()
            .enumerate()
            .filter(|(i, _)| self.picked.get(*i) != self.first.get(*i))
            .filter_map(|(i, role)| Some((*role, *self.ids.get(i)?.get(*self.picked.get(i)?)?)))
            .collect()
    }

    /// The option lines for role `role` (an index of [`CHOOSABLE`]):
    /// "Automatic (...)" and then every folder, in the language shown.
    pub fn options(&self, role: usize) -> Vec<String> {
        let detected = self.detected.get(role).and_then(|d| d.as_deref());
        let mut out = vec![automatic_line(detected)];
        out.extend(self.names.iter().cloned());
        out
    }

    /// The form's pick for a role changed.
    pub fn pick(&mut self, role: usize, option: usize) {
        if self.ids.get(role).is_some_and(|ids| option < ids.len())
            && let Some(picked) = self.picked.get_mut(role)
        {
            *picked = option;
        }
    }

    /// Write the picks that differ from when the form opened (Save went
    /// through). Returns whether any role changed.
    pub fn apply(&mut self, db: &Db, account: i64) -> anyhow::Result<bool> {
        if self.account != account {
            return Ok(false);
        }
        let mut changed = false;
        let mut first_error = None;
        for (role, folder) in self.changes() {
            match db.set_role_override(account, role, (folder != 0).then_some(folder)) {
                Ok(()) => changed = true,
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        *self = Self::default();
        match first_error {
            Some(e) => Err(e),
            None => Ok(changed),
        }
    }
}

/// The first option of a role: what the server marks, or that it marks none.
pub fn automatic_line(detected: Option<&str>) -> String {
    match detected {
        Some(name) => text::role_automatic(name),
        None => text::role_automatic_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ports `only_a_pick_that_differs_is_a_change` (src/app/roles.rs).
    #[test]
    fn only_a_pick_that_differs_is_a_change() {
        let edit = RoleEdit {
            account: 1,
            ids: vec![vec![0, 5, 6]; 5],
            first: vec![0, 0, 2, 0, 0],
            picked: vec![1, 0, 0, 0, 0],
            ..Default::default()
        };
        // Sent: Automatic -> folder 5; Trash: folder 6 -> Automatic (0).
        assert_eq!(edit.changes(), [(Role::Sent, 5), (Role::Trash, 0)]);
        assert!(RoleEdit::default().changes().is_empty());
    }

    /// Ports `the_automatic_line_names_what_the_server_marks`.
    #[test]
    fn the_automatic_line_names_what_the_server_marks() {
        assert_eq!(automatic_line(Some("Sent Items")), "Automatic (Sent Items)");
        assert_eq!(automatic_line(None), "Automatic (none found)");
    }

    #[test]
    fn a_pick_outside_the_options_is_ignored() {
        let mut edit = RoleEdit {
            account: 1,
            ids: vec![vec![0, 5]; 5],
            first: vec![0; 5],
            picked: vec![0; 5],
            ..Default::default()
        };
        edit.pick(0, 1);
        edit.pick(1, 9);
        edit.pick(9, 0);
        assert_eq!(edit.picked, [1, 0, 0, 0, 0]);
    }
}
