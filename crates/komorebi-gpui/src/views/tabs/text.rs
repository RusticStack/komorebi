//! The words of the tabs and the message windows. Each `tr!` is the Slint
//! app's text and context (`ui/app.slint`, `ui/widgets.slint`,
//! `src/app/tabs.rs`), so all are translated already.

use komorebi_i18n::tr;

/// A message's subject as its tab's title, or "(no subject)".
pub fn title_or_none(subject: String) -> String {
    if subject.trim().is_empty() {
        tr!("(no subject)")
    } else {
        subject
    }
}

/// A draft tab's title: its subject, or "New message".
pub fn draft_title(subject: &str) -> String {
    match subject.trim() {
        "" => tr!("tab title" => "New message"),
        s => s.to_string(),
    }
}

/// The Mail tab.
pub fn mail() -> String {
    tr!("tab" => "Mail")
}

/// What a screen reader says for a tab (the Slint `TabHeader`).
pub fn tab_name(title: &str, draft: bool, unread: bool, n: usize, of: usize) -> String {
    if draft {
        tr!("{0}, draft, {1} of {2}", title, n, of)
    } else if unread {
        tr!("{0}, unread, {1} of {2}", title, n, of)
    } else {
        tr!("{0}, {1} of {2}", title, n, of)
    }
}

pub fn strip() -> String {
    tr!("Tabs")
}

pub fn close_tab() -> String {
    tr!("Close tab")
}

pub fn close_tab_tip() -> String {
    tr!("Close tab (Ctrl+W)")
}

pub fn close_others() -> String {
    tr!("Close other tabs")
}

pub fn close_right() -> String {
    tr!("Close tabs to the right")
}

pub fn open_window() -> String {
    tr!("Open in new window")
}

pub fn reopen() -> String {
    tr!("Reopen closed tab")
}

/// A message tab or window whose message left the server.
pub fn gone() -> String {
    tr!("This message was moved or deleted on the server.")
}

/// A message window's OS title: "Subject - Komorebi".
pub fn window_title(subject: &str) -> String {
    tr!("window title" => "{0} - Komorebi", subject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    /// The Slint app's sentences, in English and Portuguese.
    #[test]
    fn the_tab_words_are_the_slint_apps() {
        with_lang(Lang::En, || {
            assert_eq!(mail(), "Mail");
            assert_eq!(draft_title(" "), "New message");
            assert_eq!(
                tab_name("Plans", false, true, 2, 3),
                "Plans, unread, 2 of 3"
            );
            assert_eq!(tab_name("Plans", true, false, 2, 3), "Plans, draft, 2 of 3");
            assert_eq!(window_title("Plans"), "Plans - Komorebi");
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(mail(), "Correio");
            assert_eq!(close_tab(), "Fechar separador");
            assert_eq!(reopen(), "Reabrir o separador fechado");
            assert_eq!(
                tab_name("Plano", false, true, 2, 3),
                "Plano, não lida, 2 de 3"
            );
        });
    }
}
