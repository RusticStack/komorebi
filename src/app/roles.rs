//! Folder roles in Edit account > Folders: the folder Sent mail is copied
//! to, drafts are saved in, and Delete, Archive and Report junk move to, for
//! servers that don't mark them (or mark the wrong one). Each role offers
//! "Automatic" (what the server marks) and the account's folders; the choices
//! are kept in the open form and written when Save succeeds. Nothing is
//! created, renamed or moved on the server.

use super::*;
use crate::RoleChoice;
use crate::mail::imap::decode_mutf7;
use crate::role::Role;

/// The roles that can be chosen, in the order the form lists them.
pub(super) const CHOOSABLE: [Role; 5] = [
    Role::Sent,
    Role::Drafts,
    Role::Trash,
    Role::Archive,
    Role::Junk,
];

/// The folders offered to the open form, and what is picked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RoleEdit {
    account: i64,
    /// Per role, the folder id of each option (0: Automatic).
    ids: Vec<Vec<i64>>,
    /// Per role, the option shown when the form opened.
    first: Vec<usize>,
    /// Per role, the option picked now.
    picked: Vec<usize>,
}

impl RoleEdit {
    /// The (role, folder id; 0 for Automatic) picked differently from when
    /// the form opened.
    pub(super) fn changes(&self) -> Vec<(Role, i64)> {
        CHOOSABLE
            .iter()
            .enumerate()
            .filter(|(i, _)| self.picked.get(*i) != self.first.get(*i))
            .filter_map(|(i, role)| Some((*role, *self.ids.get(i)?.get(*self.picked.get(i)?)?)))
            .collect()
    }
}

/// The option lines for a role: "Automatic (…)" and then every folder.
fn automatic_line(detected: Option<&str>) -> String {
    match detected {
        Some(name) => t!(invoke_role_automatic, name.into()),
        None => t!(invoke_role_automatic_none),
    }
}

impl App {
    /// The Folders block of the form for `account`; remembers the folder
    /// behind each option.
    pub(super) fn show_role_choices(&mut self, account: i64) {
        let ui = self.ui();
        let mut folders: Vec<_> = self
            .db
            .folders()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.account_id == account && f.role != Role::Inbox)
            .collect();
        folders.sort_by_key(|f| decode_mutf7(&f.name).to_lowercase());
        // Folders that are the account's inbox are never offered, nor is any
        // folder when the list isn't known yet (nothing synced).
        if folders.is_empty() {
            self.role_edit = RoleEdit::default();
            ui.set_a_roles(ModelRc::default());
            return;
        }
        let names: Vec<String> = folders.iter().map(|f| decode_mutf7(&f.name)).collect();
        let mut edit = RoleEdit {
            account,
            ..Default::default()
        };
        let mut choices = Vec::new();
        for role in CHOOSABLE {
            let detected = self.db.detected_folder(account, role).ok().flatten();
            let chosen = self.db.chosen_folder(account, role).ok().flatten();
            let mut options = vec![automatic_line(
                detected.as_ref().map(|f| f.display.as_str()),
            )];
            options.extend(names.iter().cloned());
            let mut ids = vec![0];
            ids.extend(folders.iter().map(|f| f.id));
            let current = chosen
                .and_then(|c| ids.iter().position(|id| *id == c.id))
                .unwrap_or(0);
            edit.ids.push(ids);
            edit.first.push(current);
            edit.picked.push(current);
            choices.push(RoleChoice {
                options: ModelRc::new(VecModel::from(
                    options
                        .into_iter()
                        .map(SharedString::from)
                        .collect::<Vec<_>>(),
                )),
                current: current as i32,
            });
        }
        self.role_edit = edit;
        ui.set_a_roles(ModelRc::new(VecModel::from(choices)));
    }

    /// The language changed: the options are worded again, and what is
    /// picked stays (the form's choices are not saved yet).
    pub(super) fn refresh_role_options(&mut self) {
        let old = std::mem::take(&mut self.role_edit);
        if old.account == 0 || self.ui().get_a_edit_id() == 0 {
            self.role_edit = old;
            return;
        }
        self.show_role_choices(old.account);
        if self.role_edit.ids != old.ids {
            return;
        }
        self.role_edit.first = old.first;
        self.role_edit.picked = old.picked;
        let model = self.ui().get_a_roles();
        for (i, picked) in self.role_edit.picked.iter().enumerate() {
            if let Some(mut choice) = model.row_data(i) {
                choice.current = *picked as i32;
                model.set_row_data(i, choice);
            }
        }
    }

    /// The form's pick for a role changed.
    pub(super) fn choose_role(&mut self, role: i32, option: i32) {
        let (Ok(role), Ok(option)) = (usize::try_from(role), usize::try_from(option)) else {
            return;
        };
        if self
            .role_edit
            .ids
            .get(role)
            .is_some_and(|ids| option < ids.len())
            && let Some(picked) = self.role_edit.picked.get_mut(role)
        {
            *picked = option;
        }
    }

    /// Write the picks that differ from when the form opened (Save went
    /// through). Returns whether any role changed.
    pub(super) fn apply_role_choices(&mut self, account: i64) -> bool {
        if self.role_edit.account != account {
            return false;
        }
        let changes = self.role_edit.changes();
        let mut changed = false;
        for (role, folder) in changes {
            match self
                .db
                .set_role_override(account, role, (folder != 0).then_some(folder))
            {
                Ok(()) => changed = true,
                Err(e) => self.set_status(&text::detail(&e), true),
            }
        }
        self.role_edit = RoleEdit::default();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pick_that_differs_is_a_change() {
        let edit = RoleEdit {
            account: 1,
            ids: vec![vec![0, 5, 6]; 5],
            first: vec![0, 0, 2, 0, 0],
            picked: vec![1, 0, 0, 0, 0],
        };
        // Sent: Automatic -> folder 5; Trash: folder 6 -> Automatic (0).
        assert_eq!(edit.changes(), [(Role::Sent, 5), (Role::Trash, 0)]);
        assert!(RoleEdit::default().changes().is_empty());
    }

    #[test]
    fn the_automatic_line_names_what_the_server_marks() {
        let _ui = crate::tr::tests::window();
        assert_eq!(automatic_line(Some("Sent Items")), "Automatic (Sent Items)");
        assert_eq!(automatic_line(None), "Automatic (none found)");
    }
}
