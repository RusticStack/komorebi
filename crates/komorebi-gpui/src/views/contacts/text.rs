//! The Contacts view in words (docs/GPUI-I18N.md): each the Slint app's
//! catalog entry (`ui/contacts.slint`, `ui/tr.slint`), so the template only
//! gains file names. The view keeps typed values (`contacts_page::Problem`,
//! [`super::sync::Note`]) and words them here in `render`.

use komorebi::contacts_page::Problem;
use komorebi_i18n::{tr, trn};

use super::sync::Note;

pub fn contacts() -> String {
    tr!("Contacts")
}

pub fn count(n: usize) -> String {
    trn!("{n} contact", "{n} contacts", n)
}

pub fn new_contact() -> String {
    tr!("New contact")
}

pub fn edit_contact() -> String {
    tr!("Edit contact")
}

pub fn search() -> String {
    tr!("Search contacts")
}

pub fn all_contacts() -> String {
    tr!("All contacts")
}

pub fn groups() -> String {
    tr!("Groups")
}

pub fn none_match() -> String {
    tr!("No contacts match.")
}

pub fn none_yet() -> String {
    tr!("No contacts yet. Add one with New contact, or turn on address book sync.")
}

pub fn back() -> String {
    tr!("Back")
}

pub fn close() -> String {
    tr!("Close")
}

// ---- The details

pub fn new_message() -> String {
    tr!("New message")
}

pub fn new_message_to(addr: &str) -> String {
    tr!("New message to {}", addr)
}

pub fn edit() -> String {
    tr!("Edit")
}

pub fn delete() -> String {
    tr!("Delete")
}

pub fn copy() -> String {
    tr!("Copy")
}

pub fn copied() -> String {
    tr!("Copied")
}

pub fn read_only() -> String {
    tr!("Read only")
}

pub fn read_only_text() -> String {
    tr!(
        "This address book syncs read only. Turn on Read and write in address book sync to edit it."
    )
}

pub fn conflict() -> String {
    tr!("This contact changed on the server while you were editing it")
}

pub fn conflict_text() -> String {
    tr!("The server's version is shown. Your version is kept until you choose.")
}

pub fn keep_mine() -> String {
    tr!("Keep mine")
}

pub fn keep_server() -> String {
    tr!("Keep the server's")
}

pub fn email() -> String {
    tr!("Email")
}

pub fn phone() -> String {
    tr!("Phone")
}

pub fn notes() -> String {
    tr!("Notes")
}

pub fn address_book() -> String {
    tr!("Address book")
}

pub fn on_this_computer() -> String {
    tr!("On this computer")
}

/// A vCard TYPE as a word (the server's own word otherwise).
pub fn label(label: &str) -> String {
    match label {
        "work" => tr!("contact label" => "Work"),
        "home" => tr!("contact label" => "Home"),
        "cell" | "mobile" => tr!("contact label" => "Mobile"),
        other => other.to_string(),
    }
}

/// The job title and the organization under the name.
pub fn org_line(title: &str, org: &str) -> String {
    match (title.is_empty(), org.is_empty()) {
        (false, false) => tr!("{0}, {1}", title, org),
        (false, true) => title.to_string(),
        (true, _) => org.to_string(),
    }
}

/// A list row's accessible name: the name and the address.
pub fn row_name(name: &str, email: &str) -> String {
    if email.is_empty() {
        name.to_string()
    } else {
        tr!("suggestion" => "{0}, {1}", name, email)
    }
}

pub fn delete_question() -> String {
    tr!("Delete this contact?")
}

pub fn delete_question_text(synced: bool) -> String {
    if synced {
        tr!("It is removed from this computer and from the address book on the server.")
    } else {
        tr!("It is removed from this computer.")
    }
}

pub fn discard_question() -> String {
    tr!("Discard your changes?")
}

pub fn discard() -> String {
    tr!("Discard")
}

pub fn cancel() -> String {
    tr!("Cancel")
}

pub fn add_account_write() -> String {
    tr!("Add an account to write messages.")
}

// ---- The editor

pub fn name() -> String {
    tr!("Name")
}

pub fn given_name() -> String {
    tr!("Given name")
}

pub fn family_name() -> String {
    tr!("Family name")
}

pub fn organization() -> String {
    tr!("Organization")
}

pub fn job_title() -> String {
    tr!("Job title")
}

pub fn email_placeholder() -> String {
    tr!("name@example.com")
}

pub fn add_email() -> String {
    tr!("Add email")
}

/// Email line `n` and its remove button (1 is the first line), so
/// Narrator's list of fields and buttons tells the lines apart.
pub fn email_n(n: usize) -> String {
    tr!("Email {0}", n)
}

pub fn remove_email_n(n: usize) -> String {
    tr!("Remove email {0}", n)
}

pub fn add_phone() -> String {
    tr!("Add phone")
}

/// Phone line `n` and its remove button (1 is the first line).
pub fn phone_n(n: usize) -> String {
    tr!("Phone {0}", n)
}

pub fn remove_phone_n(n: usize) -> String {
    tr!("Remove phone {0}", n)
}

pub fn groups_note() -> String {
    tr!("Separate groups with commas.")
}

pub fn save() -> String {
    tr!("Save")
}

pub fn problem(p: &Problem) -> String {
    match p {
        Problem::Empty => tr!("Add a name, an email address or a phone number."),
        Problem::Email(addr) => tr!("{0} is not an email address.", addr),
    }
}

/// A cache failure behind its lead-in sentence (the Slint app's
/// `text::detail`).
pub fn detail(e: &str) -> String {
    tr!("Something went wrong. {0}", e)
}

// ---- The address-book sync pane

pub fn sync() -> String {
    tr!("Sync")
}

pub fn sync_title() -> String {
    tr!("Address book sync")
}

pub fn sync_intro() -> String {
    tr!(
        "Sync each account's CardDAV address books. Sync is off until you turn it on, and runs only while Komorebi is open."
    )
}

pub fn add_account_first() -> String {
    tr!("Add a mail account first.")
}

pub fn account() -> String {
    tr!("Account")
}

pub fn sync_books() -> String {
    tr!("Sync address books")
}

pub fn mode(m: i64) -> String {
    match m {
        1 => tr!("Read only"),
        2 => tr!("Read and write"),
        _ => tr!("sync" => "Off"),
    }
}

pub fn server_address() -> String {
    tr!("Server address")
}

pub fn server_note() -> String {
    tr!("For Purelymail, copy it from purelymail.com/manage/cardDavUrl.")
}

pub fn find() -> String {
    tr!("Find")
}

pub fn user_name() -> String {
    tr!("User name")
}

pub fn same_password() -> String {
    tr!("Same as the mail password")
}

pub fn password() -> String {
    tr!("Password")
}

pub fn password_note() -> String {
    tr!("Kept in Windows Credential Manager.")
}

pub fn find_books() -> String {
    tr!("Find address books")
}

pub fn sync_now() -> String {
    tr!("Sync now")
}

pub fn working() -> String {
    tr!("Working with the address book server...")
}

/// "12 contacts on this computer. Last synced 14:05." (two sentences, as
/// the Slint app joins them).
pub fn status(count: i64, at: i64) -> String {
    format!(
        "{} {}",
        trn!(
            "{n} contact on this computer.",
            "{n} contacts on this computer.",
            count
        ),
        tr!("Last synced {0}.", komorebi::format::time(at))
    )
}

pub fn remove_question() -> String {
    tr!("Remove the synced contacts from this computer?")
}

pub fn remove_question_text() -> String {
    tr!("The address books on the server are not changed.")
}

pub fn remove() -> String {
    tr!("Remove")
}

pub fn keep() -> String {
    tr!("Keep")
}

/// What a sync request ended with.
pub fn note(n: &Note) -> String {
    match n {
        Note::Offline => tr!("Address book sync is off while Komorebi is offline."),
        Note::Auth => tr!("The address book server refused the user name or password."),
        Note::Redirect => {
            tr!("The server sent Komorebi to another server. Enter that server's address instead.")
        }
        Note::NotFound => tr!("No address book was found at this address."),
        Note::Tls => tr!("The address book server's certificate couldn't be checked."),
        Note::Failed(detail) => tr!("Address book sync failed ({0}).", detail),
        Note::NoneFound => tr!("No address books were found at this address."),
        Note::Detail(e) => detail(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    #[test]
    fn the_details_speak_as_the_slint_app() {
        with_lang(Lang::En, || {
            assert_eq!(org_line("Editor", "Lumen"), "Editor, Lumen");
            assert_eq!(org_line("", "Lumen"), "Lumen");
            assert_eq!(label("cell"), "Mobile");
            assert_eq!(label("other"), "other");
            assert_eq!(count(1), "1 contact");
            assert_eq!(count(9), "9 contacts");
            assert_eq!(
                problem(&Problem::Email("nope".into())),
                "nope is not an email address."
            );
        });
        with_lang(Lang::PtPt, || {
            assert_ne!(count(9), "9 contacts");
            assert_ne!(label("work"), "Work");
            assert_ne!(mode(0), "Off");
        });
    }

    /// Each email and phone line has its own name (stage D).
    #[test]
    fn the_editor_lines_are_numbered() {
        with_lang(Lang::En, || {
            assert_eq!(email_n(2), "Email 2");
            assert_eq!(remove_email_n(2), "Remove email 2");
            assert_eq!(phone_n(1), "Phone 1");
            assert_eq!(remove_phone_n(1), "Remove phone 1");
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(email_n(2), "E-mail 2");
            assert_eq!(remove_email_n(2), "Remover e-mail 2");
            assert_eq!(phone_n(1), "Telefone 1");
            assert_eq!(remove_phone_n(1), "Remover telefone 1");
        });
    }
}
