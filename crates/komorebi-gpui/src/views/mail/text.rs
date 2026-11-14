//! The mail view's words (docs/GPUI-I18N.md): each the Slint app's catalog
//! entry (`ui/tr.slint`, `ui/app.slint`, `ui/widgets.slint`), so the
//! template only gains file names. Views keep typed values and call these
//! in `render`; a language switch redraws them.

use komorebi::db::{Filter, Folder};
use komorebi::i18n::Lang;
use komorebi::role::Role;
use komorebi::rows::Row;
use komorebi_i18n::{tr, trn};

use super::model::{Empty, Question, Status, View};
use super::triage::Removal;

/// A folder's name: the server's in English, the role's name in other
/// languages for the folders a server marks by role.
pub fn folder_label(f: &Folder) -> String {
    if komorebi_i18n::current() == Lang::En {
        return f.display.clone();
    }
    role_name(f.role).unwrap_or_else(|| f.display.clone())
}

/// A role's own name (`None`: a user folder).
pub fn role_name(role: Role) -> Option<String> {
    Some(match role {
        Role::Inbox => tr!("Inbox"),
        Role::Drafts => tr!("Drafts"),
        Role::Sent => tr!("Sent"),
        Role::Archive => tr!("folder" => "Archive"),
        Role::Junk => tr!("Junk"),
        Role::Trash => tr!("Trash"),
        Role::Other => return None,
    })
}

pub fn all_inboxes() -> String {
    tr!("All inboxes")
}

/// All inboxes under its rail icon.
pub fn rail_all_inboxes() -> String {
    tr!("rail" => "All inboxes")
}

/// The unified line of the drafts saved on this PC (every account's, the
/// ones Komorebi edits): named apart from an account's server Drafts
/// folder, which lists only the drafts saved elsewhere (FEATURES.md
/// "Drafts"), so the pane never shows two lines called "Drafts".
pub fn drafts() -> String {
    tr!("Drafts on this PC")
}

pub fn outbox() -> String {
    tr!("Outbox")
}

/// An all-folders search's row tag: the account and the folder.
pub fn account_folder(account: &str, folder: &str) -> String {
    tr!("tag" => "{0} · {1}", account, folder)
}

/// The view's name (the list's title and the search's placeholder).
pub fn view_name(view: View, folder: Option<&Folder>) -> String {
    match view {
        View::AllInboxes => all_inboxes(),
        View::Drafts => drafts(),
        View::Outbox => outbox(),
        View::Folder(_) => folder.map(folder_label).unwrap_or_default(),
    }
}

pub fn search_results() -> String {
    tr!("Search results")
}

/// The search box's placeholder: the view it searches.
pub fn search_in(view: &str) -> String {
    tr!("Search {0}", view)
}

pub fn search_name() -> String {
    tr!("Search mail")
}

/// What the search found, and where.
pub fn results(n: usize, all: bool, view: &str) -> String {
    if all {
        trn!("{n} result in all folders", "{n} results in all folders", n)
    } else {
        trn!("{n} result in {0}", "{n} results in {0}", n, view)
    }
}

pub fn search_all_folders() -> String {
    tr!("Search all folders")
}

pub fn clear_search() -> String {
    tr!("Clear search")
}

/// The pills' group name (new in 1.0: the Slint bar had none).
pub fn filter_name() -> String {
    tr!("Filter")
}

/// The All / Unread / Important pills.
pub fn filters() -> [String; 3] {
    [tr!("All"), tr!("Unread"), tr!("filter" => "Important")]
}

pub fn filter_index(filter: Filter) -> usize {
    match filter {
        Filter::All => 0,
        Filter::Unread => 1,
        Filter::Flagged => 2,
    }
}

pub fn filter_at(ix: usize) -> Filter {
    match ix {
        1 => Filter::Unread,
        2 => Filter::Flagged,
        _ => Filter::All,
    }
}

/// What an empty list says.
pub fn empty(e: Empty, lang: Lang) -> String {
    match e {
        Empty::NoMatches => tr!("No matches"),
        Empty::NoDrafts => tr!("No drafts. Select New mail to start one."),
        Empty::Outbox => tr!("Outbox is empty. Messages wait here while they are sent."),
        Empty::NoUnread => tr!("No unread messages"),
        Empty::NoFlagged => tr!("No important messages"),
        Empty::DownloadingStart => {
            tr!("Downloading your mail\u{a0}… Newest messages arrive first.")
        }
        Empty::Downloading(done, total) => tr!(
            "Downloading your mail\u{a0}… {0} of {1}. Newest messages arrive first.",
            komorebi::format::thousands(lang, done),
            komorebi::format::thousands(lang, total)
        ),
        Empty::Loading => tr!("Loading\u{a0}…"),
        Empty::Unreachable => tr!("Can't reach the server, so this list may be incomplete"),
        Empty::Failed => tr!("Couldn't check for new mail, so this list may be incomplete"),
        Empty::Folder => tr!("This folder is empty"),
        Empty::Nothing => tr!("Nothing here"),
    }
}

/// The sentence for an account with no folder for a role.
pub fn missing(role: Role) -> String {
    match role {
        Role::Archive => tr!(
            "This account has no Archive folder. You can choose one in Settings: Accounts, Edit."
        ),
        Role::Trash => tr!(
            "This account has no Trash folder, so nothing was deleted. You can choose one in Settings: Accounts, Edit."
        ),
        Role::Junk => tr!(
            "This account has no Junk folder, so nothing was moved. You can choose one in Settings: Accounts, Edit."
        ),
        _ => tr!("This account has no Inbox folder."),
    }
}

/// A status line.
pub fn status(s: &Status) -> String {
    match s {
        Status::Missing(role) => missing(*role),
        Status::UndoWindow => tr!("Undo brought the message back; open it from the list."),
        Status::Error(detail) => detail.clone(),
    }
}

/// The Undo offer after moving `count` messages to the folder `place`
/// (empty: deleted).
pub fn removed_label(place: &str, count: usize) -> String {
    match (count, place) {
        (1, "") => tr!("Deleted"),
        (1, _) => tr!("Moved to {0}", place),
        (_, "") => trn!("{n} message deleted", "{n} messages deleted", count),
        _ => trn!(
            "{n} message moved to {0}",
            "{n} messages moved to {0}",
            count,
            place
        ),
    }
}

/// The Undo slot's sentence for a held move.
pub fn removal(r: &Removal, place: Option<&Folder>) -> String {
    let place = match (r.to, place) {
        (Some(_), Some(f)) => folder_label(f),
        _ => String::new(),
    };
    let label = removed_label(&place, r.count);
    match r.missing {
        Some(role) => tr!("{0}. {1}", label, missing(role)),
        None => label,
    }
}

pub fn undo() -> String {
    tr!("Undo")
}

pub fn close() -> String {
    tr!("Close")
}

/// The confirm dialog of a question: title, sentence, action.
pub fn question(q: Question, n: usize) -> (String, String, String) {
    match q {
        Question::DeleteFrom { trash: true } => (
            trn!(
                "Delete {n} message for good?",
                "Delete {n} messages for good?",
                n
            ),
            trn!(
                "{n} message is deleted from Trash on the server. Once the Undo offer goes, it can't be brought back.",
                "{n} messages are deleted from Trash on the server. Once the Undo offer goes, they can't be brought back.",
                n
            ),
            tr!("Delete"),
        ),
        Question::DeleteFrom { trash: false } => (
            trn!(
                "Delete {n} message from Junk?",
                "Delete {n} messages from Junk?",
                n
            ),
            trn!(
                "{n} message moves to Trash.",
                "{n} messages move to Trash.",
                n
            ),
            tr!("Delete"),
        ),
    }
}

pub fn cancel() -> String {
    tr!("Cancel")
}

/// What a screen reader says for a row (the Slint `MailItem`'s label).
pub fn row_name(r: &Row) -> String {
    let mut parts = Vec::new();
    if r.unread {
        parts.push(tr!("Unread"));
    }
    if r.draft {
        parts.push(tr!("Draft"));
    }
    parts.push(r.sender.clone());
    // What the row shows when there is no subject.
    if r.subject.trim().is_empty() {
        parts.push(no_subject());
    } else {
        parts.push(r.subject.clone());
    }
    parts.push(r.date.clone());
    if r.problem {
        parts.push(tr!("Needs attention"));
    }
    if r.flagged {
        parts.push(tr!("Important"));
    }
    if r.high_importance {
        parts.push(tr!("High importance"));
    }
    if r.low_importance {
        parts.push(tr!("Low importance"));
    }
    if r.attach {
        parts.push(tr!("Has attachment"));
    }
    if r.answered {
        parts.push(tr!("Replied"));
    }
    if !r.tag.is_empty() {
        parts.push(tr!("in {0}", r.tag));
    }
    parts.join(", ")
}

/// A row replied to by a saved draft.
pub fn draft() -> String {
    tr!("Draft")
}

/// A row's subject line when it has none.
pub fn no_subject() -> String {
    tr!("(no subject)")
}

pub fn messages() -> String {
    tr!("Messages")
}

pub fn folders() -> String {
    tr!("Folders")
}

pub fn new_mail() -> String {
    tr!("New mail")
}

pub fn new_mail_tip() -> String {
    tr!("New mail (Ctrl+N)")
}

pub fn more_actions() -> String {
    tr!("More actions")
}

pub fn settings() -> String {
    tr!("Settings")
}

pub fn contacts() -> String {
    tr!("Contacts")
}

/// The pane's fold toggle.
pub fn fold_tip(collapsed: bool) -> String {
    if collapsed {
        tr!("Show folders (Ctrl+Shift+E)")
    } else {
        tr!("Hide folders (Ctrl+Shift+E)")
    }
}

/// How many messages are selected.
pub fn selected(n: usize) -> String {
    trn!("{n} message selected", "{n} messages selected", n)
}

pub fn select_all() -> String {
    tr!("Select all")
}

pub fn clear_selection() -> String {
    tr!("Clear selection")
}

/// The search scope that is the view itself: this folder, or All
/// inboxes' inboxes (the Slint ScopeBox).
pub fn scope_here(view: View) -> String {
    match view {
        View::AllInboxes => tr!("These inboxes"),
        _ => tr!("This folder"),
    }
}

pub fn scope_all() -> String {
    tr!("All folders")
}

pub fn search_scope() -> String {
    tr!("Search scope")
}

/// An account line's badge when its sign-in was rejected (the Slint
/// NavItem's ", sign-in rejected").
pub fn sign_in_rejected() -> String {
    tr!("sign-in rejected")
}

/// The overlay while files are dragged over the mail.
pub fn drop_to_attach() -> String {
    tr!("Drop to attach to a new message")
}

/// The empty list's link that takes the filter away.
pub fn show_all() -> String {
    tr!("Show all")
}

/// What a search looks at, under an empty search.
pub fn search_hint() -> String {
    tr!("Searches subject, people and preview of downloaded mail.")
}

/// The first download's bar.
pub fn downloading_your_mail() -> String {
    tr!("Downloading your mail")
}

/// The one-pane layout's first command.
pub fn back_to_list() -> String {
    tr!("Back to list (Esc)")
}

/// The selection panel's Move button, its tooltip, and why it is off.
pub fn move_label() -> String {
    tr!("Move")
}

pub fn move_tip() -> String {
    tr!("Move (Ctrl+Shift+V)")
}

pub fn move_one_account() -> String {
    tr!("Select messages of one account to move them")
}

/// The Move flyout's filter.
pub fn filter_folders() -> String {
    tr!("Filter folders")
}

/// A folder pane line's Collapse all (an account's: Collapse all folders).
/// The line's menu runs it as a `NavRun`, never as a [`Command`].
pub fn collapse_all(account: bool) -> String {
    if account {
        tr!("Collapse all folders")
    } else {
        tr!("Collapse all")
    }
}

/// An account line's Refresh (a `NavRun` too).
pub fn refresh() -> String {
    tr!("Refresh")
}

/// The commands of the menus and the command bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    OpenTab,
    OpenWindow,
    Reply,
    ReplyAll,
    Forward,
    ForwardAttached,
    MarkRead,
    MarkUnread,
    Flag,
    Unflag,
    Archive,
    Delete,
    MoveTo,
    Junk,
    NotJunk,
    SaveEml,
    ShowOriginal,
    CopyId,
    ShowInFolder,
    MarkAllRead,
    SelectMessages,
    AccountSettings,
    HideFolders,
    ShowFolders,
    /// A Drafts row: open it in the composer, or discard it.
    OpenDraft,
    DiscardDraft,
}

pub fn command(c: Command) -> String {
    match c {
        Command::OpenTab => tr!("Open in new tab"),
        Command::OpenWindow => tr!("Open in new window"),
        Command::Reply => tr!("Reply"),
        Command::ReplyAll => tr!("Reply all"),
        Command::Forward => tr!("Forward"),
        Command::ForwardAttached => tr!("Forward as attachment"),
        Command::MarkRead => tr!("Mark as read"),
        Command::MarkUnread => tr!("Mark as unread"),
        Command::Flag => tr!("Mark as important"),
        Command::Unflag => tr!("Remove importance"),
        Command::Archive => tr!("Archive"),
        Command::Delete => tr!("Delete"),
        Command::MoveTo => tr!("Move to"),
        Command::Junk => tr!("Report junk"),
        Command::NotJunk => tr!("Not junk"),
        Command::SaveEml => tr!("Save as .eml"),
        Command::ShowOriginal => tr!("Show original"),
        Command::CopyId => tr!("Copy Message-ID"),
        Command::ShowInFolder => tr!("Show in folder"),
        Command::MarkAllRead => tr!("Mark all as read"),
        Command::SelectMessages => tr!("Select messages"),
        Command::AccountSettings => tr!("Account settings"),
        Command::HideFolders => tr!("Hide folders"),
        Command::ShowFolders => tr!("Show folders"),
        Command::OpenDraft => tr!("Open"),
        Command::DiscardDraft => tr!("Discard"),
    }
}

/// A command bar button's tooltip, with its key.
pub fn command_tip(c: Command) -> String {
    match c {
        Command::MarkRead => tr!("Mark as read (Ctrl+Q)"),
        Command::MarkUnread => tr!("Mark as unread (Ctrl+U)"),
        Command::Archive => tr!("Archive (Backspace)"),
        Command::Delete => tr!("Delete (Del)"),
        other => command(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi_i18n::with_lang;

    /// Port of the Slint app's `empty_list_texts` (src/app.rs): the words.
    #[test]
    fn empty_list_texts() {
        with_lang(Lang::En, || {
            let cases = [
                (Empty::NoMatches, "No matches"),
                (Empty::NoUnread, "No unread messages"),
                (Empty::NoFlagged, "No important messages"),
                (Empty::Loading, "Loading\u{a0}…"),
                (Empty::Nothing, "Nothing here"),
                (Empty::NoDrafts, "No drafts. Select New mail to start one."),
                (
                    Empty::Outbox,
                    "Outbox is empty. Messages wait here while they are sent.",
                ),
                (
                    Empty::DownloadingStart,
                    "Downloading your mail\u{a0}… Newest messages arrive first.",
                ),
                (
                    Empty::Downloading(1234, 5000),
                    "Downloading your mail\u{a0}… 1,234 of 5,000. Newest messages arrive first.",
                ),
                (Empty::Folder, "This folder is empty"),
            ];
            for (e, want) in cases {
                assert_eq!(empty(e, Lang::En), want);
            }
            assert!(empty(Empty::Failed, Lang::En).starts_with("Couldn't check for new mail"));
            assert!(empty(Empty::Unreachable, Lang::En).starts_with("Can't reach the server"));
        });
    }

    /// Port of `undo_offers_name_the_folder` (src/app.rs).
    #[test]
    fn undo_offers_name_the_folder() {
        with_lang(Lang::En, || {
            assert_eq!(removed_label("Archive", 1), "Moved to Archive");
            assert_eq!(removed_label("Junk", 3), "3 messages moved to Junk");
            assert_eq!(removed_label("", 1), "Deleted");
            assert_eq!(removed_label("", 200), "200 messages deleted");
        });
    }

    #[test]
    fn the_words_are_portuguese_in_portuguese() {
        with_lang(Lang::PtPt, || {
            assert_eq!(empty(Empty::NoMatches, Lang::PtPt), "Sem resultados");
            assert_ne!(all_inboxes(), "All inboxes");
            assert_ne!(removed_label("", 2), "2 messages deleted");
            let f = Folder {
                id: 1,
                account_id: 1,
                name: "INBOX".into(),
                display: "INBOX".into(),
                depth: 0,
                role: Role::Inbox,
                uidvalidity: 0,
            };
            assert_eq!(folder_label(&f), "Caixa de entrada");
        });
    }

    #[test]
    fn a_row_is_named_as_the_slint_row_was() {
        let row = Row {
            sender: "Marta Silva".into(),
            subject: "Photos".into(),
            date: "09:04".into(),
            unread: true,
            attach: true,
            ..Row::default()
        };
        with_lang(Lang::En, || {
            assert_eq!(
                row_name(&row),
                "Unread, Marta Silva, Photos, 09:04, Has attachment"
            );
        });
    }

    /// The tag is said once ("in Work"), a row without a subject says what
    /// it shows, and an Outbox problem is named as its mark shows it.
    #[test]
    fn a_row_name_says_the_tag_once_and_the_problem() {
        let row = Row {
            sender: "To: ana@example.org".into(),
            subject: " ".into(),
            date: "09:04".into(),
            tag: "Work".into(),
            problem: true,
            high_importance: true,
            ..Row::default()
        };
        with_lang(Lang::En, || {
            assert_eq!(
                row_name(&row),
                "To: ana@example.org, (no subject), 09:04, Needs attention, High importance, in Work"
            );
        });
    }
}
