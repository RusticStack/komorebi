//! Folders made, renamed and deleted from the folder pane (the owner's
//! "Read. Reply. Rest.": no dialog for a name). New folder and New subfolder
//! open a field in the tree where the folder will be, Rename turns the line
//! into one; Enter makes it, Esc leaves it, and a name that can't be used
//! says why under the field. Delete asks first and says what goes with the
//! folder. The server work is the account worker's ([`Job::CreateFolder`]
//! and the others), which tells how it ended on the status line.
//!
//! - [`Mail`]'s part (no GPUI, plain `#[test]`s): what a line allows, the
//!   name's checks, the jobs.
//! - [`MailView`]'s part: the field's state, its keys, the confirm dialog.

use gpui::{
    AnyElement, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement, Role, SharedString, Styled, Subscription, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi::mail::Job;
use komorebi::mail::folders::{self, BadName};
use komorebi::role::Role as FolderRole;
use komorebi::tree::{self, Fold};
use komorebi_i18n::{tr, trn};

use super::model::Mail;
use super::nav::NavKind;
use super::{MailView, sync_text};
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{TextStyled as _, Tokens, layout, space};
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::field::{FieldLook, Nav, TextField};

/// What the field in the pane makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A new folder of `account`, under folder `parent` or at the top.
    New { account: i64, parent: Option<i64> },
    /// A new name for this folder.
    Rename(i64),
}

/// Why the typed name can't be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProblem {
    Bad(BadName),
    /// A folder beside it has this name already.
    Taken,
}

impl NameProblem {
    pub fn text(&self) -> String {
        match self {
            NameProblem::Bad(b) => sync_text::bad_name(b),
            NameProblem::Taken => tr!("There is already a folder with that name here."),
        }
    }
}

/// What a folder line's menu offers to change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FolderMenu {
    /// New folder (an account line) or New subfolder (a folder line).
    pub new: bool,
    /// Rename and Delete: an ordinary folder, never a special one.
    pub change: bool,
}

impl Mail {
    /// The account is on (its workers run): its folders can change.
    fn account_on(&self, account: i64) -> bool {
        self.accounts.iter().any(|a| a.id == account && a.enabled)
    }

    /// What a line's menu offers for folders (`None`: nothing, as All
    /// inboxes, Drafts, the Outbox, an account switched off).
    pub fn folder_menu(&self, kind: NavKind) -> Option<FolderMenu> {
        match kind {
            NavKind::Account(a) if self.account_on(a) => Some(FolderMenu {
                new: true,
                change: false,
            }),
            NavKind::Folder(id) => {
                let f = self.folder(id)?;
                self.account_on(f.account_id).then_some(FolderMenu {
                    new: true,
                    change: f.role == FolderRole::Other,
                })
            }
            _ => None,
        }
    }

    /// The account a new folder goes to from this line (Ctrl+Shift+N): the
    /// line's account, else the open folder's, else the first account on.
    pub fn new_folder_account(&self, kind: Option<NavKind>) -> Option<i64> {
        let of_kind = kind.and_then(|k| match k {
            NavKind::Account(a) => Some(a),
            NavKind::Folder(id) => self.folder(id).map(|f| f.account_id),
            _ => None,
        });
        let open = match self.view() {
            super::model::View::Folder(id) => self.folder(id).map(|f| f.account_id),
            _ => None,
        };
        of_kind
            .or(open)
            .filter(|&a| self.account_on(a))
            .or_else(|| self.accounts.iter().find(|a| a.enabled).map(|a| a.id))
    }

    /// The account a field's folder is in.
    pub fn target_account(&self, target: Target) -> Option<i64> {
        match target {
            Target::New { account, .. } => Some(account),
            Target::Rename(id) => self.folder(id).map(|f| f.account_id),
        }
    }

    /// The folder lines open where the field goes: the account and the
    /// parent unfolded.
    pub fn open_for(&mut self, target: Target) {
        let (account, parent) = match target {
            Target::New { account, parent } => (Some(account), parent),
            Target::Rename(id) => (self.folder(id).map(|f| f.account_id), None),
        };
        let mut changed = false;
        if let Some(a) = account {
            changed |= self.folded.remove(&Fold::Account(a));
        }
        if let Some(p) = parent {
            changed |= self.folded.remove(&Fold::Folder(p));
        }
        if changed {
            self.save_folds();
        }
    }

    /// The names beside a field's folder (its siblings in the tree), as
    /// shown, without the folder renamed.
    fn sibling_names(&self, target: Target) -> Vec<String> {
        let Some(account) = self.target_account(target) else {
            return Vec::new();
        };
        let nodes = tree::tree(&self.folders);
        let index_of = |id: i64| self.folders.iter().position(|f| f.id == id);
        let parent = match target {
            Target::New { parent, .. } => parent.and_then(index_of),
            Target::Rename(id) => index_of(id)
                .and_then(|i| nodes.iter().find(|n| n.index == i))
                .and_then(|n| n.parent),
        };
        let renamed = match target {
            Target::Rename(id) => index_of(id),
            Target::New { .. } => None,
        };
        nodes
            .iter()
            .filter(|n| n.parent == parent && Some(n.index) != renamed)
            .filter_map(|n| self.folders.get(n.index))
            .filter(|f| f.account_id == account)
            .flat_map(|f| {
                let mut names = vec![f.display.clone()];
                // "Inbox" and the like as they read too.
                names.extend(super::text::role_name(f.role));
                names
            })
            .collect()
    }

    /// Why `typed` can't be the field's folder name, if it can't. The
    /// server's delimiter is the one the account's names use.
    pub fn name_problem(&self, target: Target, typed: &str) -> Option<NameProblem> {
        let name = match folders::clean(typed) {
            Ok(n) => n,
            Err(b) => return Some(NameProblem::Bad(b)),
        };
        let account = self.target_account(target)?;
        let delimiter = tree::delimiter(self.folders.iter().filter(|f| f.account_id == account));
        if let Some(d) = delimiter.filter(|&d| name.contains(d)) {
            return Some(NameProblem::Bad(BadName::Delimiter(d.to_string())));
        }
        let unchanged = matches!(target, Target::Rename(id)
            if self.folder(id).is_some_and(|f| f.display == name));
        if !unchanged
            && self
                .sibling_names(target)
                .iter()
                .any(|n| n.to_lowercase() == name.to_lowercase())
        {
            return Some(NameProblem::Taken);
        }
        None
    }

    /// Make the field's folder: the job goes to the account's worker.
    /// `Ok(false)`: nothing to do (a rename to the same name).
    pub fn commit_folder(&mut self, target: Target, typed: &str) -> Result<bool, NameProblem> {
        if let Some(problem) = self.name_problem(target, typed) {
            return Err(problem);
        }
        let name = typed.trim().to_string();
        let Some(account) = self.target_account(target) else {
            return Ok(false);
        };
        let job = match target {
            Target::New { parent, .. } => Job::CreateFolder { parent, name },
            Target::Rename(id) => {
                if self.folder(id).is_some_and(|f| f.display == name) {
                    return Ok(false);
                }
                Job::RenameFolder {
                    folder_id: id,
                    name,
                }
            }
        };
        self.jobs.push((account, job));
        Ok(true)
    }

    /// What deleting a folder takes with it, as cached: its messages and
    /// those of its subfolders, and how many subfolders.
    pub fn folder_contents(&self, id: i64) -> (u64, usize) {
        let nodes = tree::tree(&self.folders);
        let Some(start) = self
            .folders
            .iter()
            .position(|f| f.id == id)
            .and_then(|i| nodes.iter().position(|n| n.index == i))
        else {
            return (0, 0);
        };
        let depth = nodes[start].depth;
        let under: Vec<usize> = nodes[start + 1..]
            .iter()
            .take_while(|n| n.depth > depth)
            .map(|n| n.index)
            .collect();
        let messages = std::iter::once(id)
            .chain(
                under
                    .iter()
                    .filter_map(|&i| self.folders.get(i))
                    .map(|f| f.id),
            )
            .map(|f| u64::from(self.db.count_messages(f).unwrap_or(0)))
            .sum();
        (messages, under.len())
    }

    /// Delete an ordinary folder (confirmed): the job goes to the worker.
    pub fn delete_folder(&mut self, id: i64) {
        if let Some(f) = self.folder(id)
            && f.role == FolderRole::Other
        {
            self.jobs
                .push((f.account_id, Job::DeleteFolder { folder_id: id }));
        }
    }
}

pub fn new_folder() -> String {
    tr!("New folder")
}

pub fn new_subfolder() -> String {
    tr!("New subfolder")
}

pub fn rename() -> String {
    tr!("folder" => "Rename")
}

pub fn delete() -> String {
    tr!("Delete folder")
}

/// The field typed in the pane.
pub struct FolderEdit {
    pub target: Target,
    pub input: Entity<InputState>,
    /// Said under the field (the name's problem, as typed so far).
    pub problem: Option<NameProblem>,
    _changes: Subscription,
}

/// The confirm dialog's words for deleting `name`.
pub fn delete_question(name: &str, messages: u64, subfolders: usize) -> (String, String) {
    let title = tr!("Delete \u{201c}{0}\u{201d}?", name);
    let n = messages as usize;
    let mail = trn!(
        "The folder and the {n} message in it are deleted on the server.",
        "The folder and the {n} messages in it are deleted on the server.",
        n
    );
    let mail = if messages == 0 {
        tr!("The folder is deleted on the server.")
    } else {
        mail
    };
    let subs = (subfolders > 0).then(|| {
        trn!(
            "Its {n} subfolder goes with it.",
            "Its {n} subfolders go with it.",
            subfolders
        )
    });
    let gone = tr!("Messages in it don't go to Trash and can't be brought back.");
    let message = match subs {
        Some(s) => format!("{mail} {s} {gone}"),
        None => format!("{mail} {gone}"),
    };
    (title, message)
}

impl MailView {
    /// Open the field where `target`'s folder goes (the keyboard in it).
    pub fn start_folder_edit(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match target {
            Target::Rename(id) => match self.mail.folder(id) {
                Some(f) if f.role == FolderRole::Other => f.display.clone(),
                _ => return,
            },
            Target::New { account, .. }
                if self.mail.folder_menu(NavKind::Account(account)).is_none() =>
            {
                return;
            }
            Target::New { .. } => String::new(),
        };
        self.act(window, cx, |m| m.open_for(target));
        // A rail shows no field: the full pane opens over the content.
        if self.is_rail(window) {
            self.pane_open = true;
        }
        let placeholder = match target {
            Target::New {
                parent: Some(_), ..
            } => new_subfolder(),
            _ => new_folder(),
        };
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(placeholder);
            state.set_value(current, window, cx);
            state.select_all(window, cx);
            state
        });
        let changes = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            match event {
                InputEvent::Change => {
                    let typed = input.read(cx).value().to_string();
                    if let Some(edit) = &mut this.folder_edit {
                        // An empty name is said on Enter only.
                        edit.problem = this
                            .mail
                            .name_problem(edit.target, &typed)
                            .filter(|p| *p != NameProblem::Bad(BadName::Empty));
                    }
                    cx.notify();
                }
                // Out of the field: a good name is kept, as in Explorer.
                InputEvent::Blur => this.end_folder_edit(true, false, window, cx),
                _ => {}
            }
        });
        let focus = input.read(cx).focus_handle(cx);
        self.folder_edit = Some(FolderEdit {
            target,
            input,
            problem: None,
            _changes: changes,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Enter (`commit`) or Esc on the field, or the focus left it
    /// (`!keyed`: a name that can't be used is left, silently).
    pub fn end_folder_edit(
        &mut self,
        commit: bool,
        keyed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = &self.folder_edit else {
            return;
        };
        let target = edit.target;
        if commit {
            let typed = edit.input.read(cx).value().to_string();
            let blank_new = matches!(target, Target::New { .. }) && typed.trim().is_empty();
            // A new folder left blank by a click away is simply left.
            if !blank_new || keyed {
                let mut result = Ok(false);
                self.act(window, cx, |m| result = m.commit_folder(target, &typed));
                if let Err(problem) = result
                    && keyed
                {
                    if let Some(edit) = &mut self.folder_edit {
                        edit.problem = Some(problem);
                    }
                    cx.notify();
                    return;
                }
            }
        }
        self.folder_edit = None;
        if keyed {
            window.focus(&self.nav_focus, cx);
        }
        cx.notify();
    }

    /// The field's keys: Enter makes the folder, Esc leaves it.
    fn folder_edit_key(&mut self, key: Nav, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            Nav::Enter | Nav::SecondaryEnter => self.end_folder_edit(true, true, window, cx),
            Nav::Escape => self.end_folder_edit(false, true, window, cx),
            _ => {}
        }
    }

    /// Ask before a folder is deleted: what goes with it, and that it can't
    /// be brought back. Never a special folder.
    pub fn ask_delete_folder(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(f) = self.mail.folder(id) else {
            return;
        };
        if self
            .mail
            .folder_menu(NavKind::Folder(id))
            .is_none_or(|m| !m.change)
        {
            return;
        }
        let name = f.display.clone();
        let (messages, subfolders) = self.mail.folder_contents(id);
        let (title, message) = delete_question(&name, messages, subfolders);
        let view = cx.entity().downgrade();
        dialog::confirm(
            Confirm {
                title: title.into(),
                message: message.into(),
                confirm: tr!("Delete folder").into(),
                cancel: super::text::cancel().into(),
                destructive: true,
            },
            move |window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.act(window, cx, |m| m.delete_folder(id));
                    window.focus(&this.nav_focus, cx);
                });
            },
            window,
            cx,
        );
    }

    /// The keyboard's folder line (in the full pane).
    fn cursor_kind(&self, window: &Window) -> Option<NavKind> {
        if self.keys_on_rail(window) {
            return None;
        }
        self.mail.nav().items.get(self.nav_cursor).map(|i| i.kind)
    }

    /// Ctrl+Shift+N: a new folder at the top of the keyboard line's account
    /// (or the open folder's).
    pub fn new_folder_key(
        &mut self,
        _: &super::commands::NewFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = self
            .nav_focus
            .contains_focused(window, cx)
            .then(|| self.cursor_kind(window))
            .flatten();
        if let Some(account) = self.mail.new_folder_account(kind) {
            self.start_folder_edit(
                Target::New {
                    account,
                    parent: None,
                },
                window,
                cx,
            );
        }
    }

    /// F2 on a folder line: rename it in place.
    pub fn rename_folder_key(
        &mut self,
        _: &super::commands::RenameFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(NavKind::Folder(id)) = self.cursor_kind(window) {
            self.start_folder_edit(Target::Rename(id), window, cx);
        }
    }

    /// Delete on a folder line: ask to delete it.
    pub fn delete_folder_key(
        &mut self,
        _: &super::commands::DeleteFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(NavKind::Folder(id)) = self.cursor_kind(window) {
            self.ask_delete_folder(id, window, cx);
        }
    }
}

/// The field's line in the tree, at `level`, with its problem under it.
/// It keeps the folder lines' columns: `icon` (the folder's) in their icon
/// box after `lead` and the indent, the field from their labels' edge, and
/// their right padding, so the focus ring stays in the pane.
pub fn edit_line(
    view: &MailView,
    level: usize,
    lead: f32,
    icon: Icon,
    t: &Tokens,
    cx: &mut Context<MailView>,
) -> Option<AnyElement> {
    let edit = view.folder_edit.as_ref()?;
    let c = t.color;
    let name: SharedString = match edit.target {
        Target::Rename(_) => tr!("Folder name").into(),
        Target::New {
            parent: Some(_), ..
        } => tr!("New subfolder name").into(),
        Target::New { .. } => tr!("New folder name").into(),
    };
    let entity = cx.entity().downgrade();
    let field = TextField::new(&edit.input, name)
        .look(FieldLook::Inline)
        .edit_keys(move |key, window, cx| {
            let _ = entity.update(cx, |this, cx| this.folder_edit_key(key, window, cx));
        });
    let problem = edit.problem.as_ref().map(NameProblem::text);
    let indent = lead + scaled::indent(level);
    // The pill's own padding (`FieldLook::Inline`) before its text.
    let pill_pad = space::SP_2;
    // The labels' edge, from the line's left: the icon box and its gap.
    let label_at = layout::ICON_LG + super::folders::ICON_GAP;
    Some(
        div()
            .id("folder-edit")
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::SP_1))
            .pl(px(indent))
            // The lines' right padding: the focus ring stays in the pane.
            .pr(px(space::SP_2))
            .py(px(space::SP_1))
            .child(
                div()
                    .h(px(scaled::nav_item(t)))
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .path(icon.at(layout::ICON_LG))
                            .size(px(layout::ICON_LG))
                            .flex_none()
                            .mr(px(label_at - layout::ICON_LG))
                            .text_color(c.ink),
                    )
                    .child(div().flex_1().min_w_0().child(field)),
            )
            .when_some(problem, |el, text| {
                el.child(
                    div()
                        .id("folder-edit-problem")
                        .role(Role::Alert)
                        .aria_label(text.clone())
                        .a11y_synthetic_children(|b| {
                            b.parent_node().set_live(gpui::accesskit::Live::Assertive)
                        })
                        .flex()
                        .items_start()
                        .gap(px(space::SP_1))
                        // Under the typed name.
                        .pl(px(label_at + pill_pad))
                        .pr(px(pill_pad))
                        .typo(&t.text.caption)
                        .text_color(c.ink_2)
                        .child(
                            svg()
                                .path(Icon::ErrorCircle.at(layout::ICON_SM))
                                .size(px(layout::ICON_SM))
                                .flex_none()
                                .mt(px(layout::HAIRLINE))
                                .text_color(c.ink_2),
                        )
                        .child(div().flex_1().min_w_0().child(text))
                        .test_support(),
                )
            })
            .into_any_element(),
    )
}
