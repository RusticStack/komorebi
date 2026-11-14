//! The Keyboard shortcuts dialog (F1, Ctrl+/, Settings > About; the Slint
//! app's `ui/shortcuts.slint`): every key by group. Each row's keys are a
//! key cell of the `docs/USAGE.md` "Keyboard" table, word for word, and every
//! key cell there is a row here: the test `the_dialog_lists_the_readme_keys`
//! keeps the two the same.

use gpui::{
    App, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use gpui_kit::component::WindowExt as _;
use komorebi_i18n::tr;

use super::text;
use crate::theme::{ActiveTokens as _, TextStyled as _, radius, space};
use crate::widgets::button::Button;

/// A row of the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A group's heading.
    Heading(String),
    Key {
        keys: String,
        action: String,
    },
}

/// The rows, in the language shown.
pub fn rows() -> Vec<Row> {
    vec![
        Row::Heading(tr!("shortcut group" => "Anywhere")),
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+N"),
            action: tr!("shortcut" => "New mail"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+E, F3 or Ctrl+F"),
            action: tr!("shortcut" => "Search"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+I"),
            action: tr!("shortcut" => "Go to the Inbox"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "F5"),
            action: tr!("shortcut" => "Refresh"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+,"),
            action: tr!("shortcut" => "Open or close Settings"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "F1 or Ctrl+/"),
            action: tr!("shortcut" => "Keyboard shortcuts"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "F6 / Shift+F6"),
            action: tr!("shortcut" => "Next / previous pane"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Tab / Shift+Tab"),
            action: tr!("shortcut" => "Next / previous control"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Space or Enter"),
            action: tr!("shortcut" => "Press the focused button"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Esc"),
            action: tr!("shortcut" => "Close the folder pane, clear the search or the selection"),
        },
        Row::Heading(tr!("shortcut group" => "Tabs and windows")),
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+E"),
            action: tr!("shortcut" => "Show / hide the folders"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Double-click, Shift+Enter"),
            action: tr!("shortcut" => "Open in new tab"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Middle-click"),
            action: tr!("shortcut" => "Open in a background tab; on a tab, close it"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+O"),
            action: tr!("shortcut" => "Open in new window"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+W or Ctrl+F4"),
            action: tr!("shortcut" => "Close the tab or message window"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+PageDown / Ctrl+PageUp"),
            action: tr!("shortcut" => "Next / previous tab"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+1 to Ctrl+8, Ctrl+9"),
            action: tr!("shortcut" => "Go to tab 1-8 / last tab"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+PageUp / Ctrl+Shift+PageDown"),
            action: tr!("shortcut" => "Move tab left / right"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+T"),
            action: tr!("shortcut" => "Reopen closed tab"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Esc (in a message window)"),
            action: tr!("shortcut" => "Close the window"),
        },
        Row::Heading(tr!("shortcut group" => "Folders")),
        Row::Key {
            keys: tr!("shortcut key" => "Up / Down, Home / End, Enter or Space"),
            action: tr!("shortcut" => "Move through the folders and open one"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Shift+F10 or Menu"),
            action: tr!("shortcut" => "Folder menu"),
        },
        Row::Heading(tr!("shortcut group" => "Messages")),
        Row::Key {
            keys: tr!("shortcut key" => "Up / Down"),
            action: tr!("shortcut" => "Previous / next message"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Home / End"),
            action: tr!("shortcut" => "First / last message"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "PageUp / PageDown"),
            action: tr!("shortcut" => "Up / down a page"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Enter"),
            action: tr!("shortcut" => "Open the message"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+click / Shift+click"),
            action: tr!("shortcut" => "Add to the selection / select a range"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Shift+Up / Shift+Down"),
            action: tr!("shortcut" => "Extend the selection"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+A"),
            action: tr!("shortcut" => "Select all messages"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+R / Ctrl+Shift+R"),
            action: tr!("shortcut" => "Reply / Reply all"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+F"),
            action: tr!("shortcut" => "Forward"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+V"),
            action: tr!("shortcut" => "Move to a folder"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Q / Ctrl+U"),
            action: tr!("shortcut" => "Mark as read / Mark as unread"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Insert or Ctrl+Shift+G"),
            action: tr!("shortcut" => "Mark as important / Remove importance"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Backspace"),
            action: tr!("shortcut" => "Archive"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Delete"),
            action: tr!("shortcut" => "Delete"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Shift+F10 or Menu"),
            action: tr!("shortcut" => "Message menu"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+S"),
            action: tr!("shortcut" => "Save as a .eml file"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Z"),
            action: tr!("shortcut" => "Undo the last archive, delete, move or send"),
        },
        Row::Heading(tr!("shortcut group" => "Writing")),
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Enter or Alt+S"),
            action: tr!("shortcut" => "Send"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Esc"),
            action: tr!("shortcut" => "Close, keeping the draft"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+C / Ctrl+Shift+B"),
            action: tr!("shortcut" => "Show Cc / Bcc"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+A"),
            action: tr!("shortcut" => "Attach files"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+V"),
            action: tr!("shortcut" => "Attach the files copied in File Explorer"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Up / Down, Enter or Tab, Esc"),
            action: tr!("shortcut" => "Pick, take or close a suggested address"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Enter (in the contact editor's Name box)"),
            action: tr!("shortcut" => "Save the contact"),
        },
        Row::Heading(tr!("shortcut group" => "Formatting (message body)")),
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+B / Ctrl+I / Ctrl+U"),
            action: tr!("shortcut" => "Bold / italic / underline"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+X"),
            action: tr!("shortcut" => "Strikethrough"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+K"),
            action: tr!("shortcut" => "Link to an address"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+8 / Ctrl+Shift+7"),
            action: tr!("shortcut" => "Bulleted / numbered list"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+Q"),
            action: tr!("shortcut" => "Quote"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+L / Ctrl+E / Ctrl+R / Ctrl+J"),
            action: tr!("shortcut" => "Align left / center / right / justify"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+] / Ctrl+["),
            action: tr!("shortcut" => "Larger / smaller text"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+M / Ctrl+Shift+M"),
            action: tr!("shortcut" => "Increase / decrease indent"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Tab / Shift+Tab (in a list or table)"),
            action: tr!("shortcut" => "List level, or the next / previous cell"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Space"),
            action: tr!("shortcut" => "Clear formatting"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Z / Ctrl+Y"),
            action: tr!("shortcut" => "Undo / redo"),
        },
        Row::Key {
            keys: tr!("shortcut key" => "Ctrl+Shift+V"),
            action: tr!("shortcut" => "Paste as plain text"),
        },
    ]
}

/// The widest the dialog is.
const WIDTH: f32 = 640.;
/// The tallest the list is before it scrolls.
const LIST_MAX_H: f32 = 480.;

/// Show the dialog; Esc and Close close it.
pub fn open(window: &mut Window, cx: &mut App) {
    let t = cx.tokens().clone();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let c = t.color;
        let list = rows().into_iter().fold(
            div()
                .id("shortcuts-list")
                .max_h(px(LIST_MAX_H))
                .overflow_y_scroll()
                .flex()
                .flex_col(),
            |list, row| match row {
                Row::Heading(name) => list.child(
                    div()
                        .pt(px(space::SP_4))
                        .pb(px(space::SP_1))
                        .typo(&t.text.body_strong)
                        .child(name),
                ),
                Row::Key { keys, action } => list.child(
                    div()
                        .flex()
                        .gap(px(space::SP_4))
                        .py(px(space::SP_1))
                        .typo(&t.text.body)
                        .child(
                            div()
                                .w(px(WIDTH / 2.5))
                                .flex_none()
                                .text_color(c.ink_2)
                                .child(keys),
                        )
                        .child(div().flex_1().child(action)),
                ),
            },
        );
        dialog
            .w(px(WIDTH))
            .bg(c.paper)
            .text_color(c.ink)
            .rounded(px(radius::CARD))
            .shadow(Vec::new())
            .close_button(false)
            .p(px(space::SP_6))
            .title(
                div()
                    .typo(&t.text.subtitle)
                    .child(text::keyboard_shortcuts_title()),
            )
            .child(list)
            .footer(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .pt(px(space::SP_4))
                    .child(
                        Button::new("shortcuts-close")
                            .label(text::close())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use komorebi::i18n::Lang;

    use super::*;

    /// The first cell of each row of the table under `## Keyboard`.
    fn readme_keys() -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/USAGE.md");
        let readme = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
            .replace("\r\n", "\n");
        let section = readme
            .split("\n## Keyboard\n")
            .nth(1)
            .expect("a Keyboard section in docs/USAGE.md");
        let section = section.split("\n## ").next().unwrap();
        section
            .lines()
            .filter(|l| l.starts_with('|'))
            .skip(2) // the header and its rule
            .map(|l| {
                l.trim_start_matches('|')
                    .split(" |")
                    .next()
                    .unwrap()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// Ports `the_dialog_lists_the_readme_keys` (tests/shortcuts_table.rs),
    /// reading the GPUI table instead of the Slint source.
    #[test]
    fn the_dialog_lists_the_readme_keys() {
        let mut readme = readme_keys();
        let mut dialog: Vec<String> = komorebi_i18n::with_lang(Lang::En, rows)
            .into_iter()
            .filter_map(|r| match r {
                Row::Key { keys, .. } => Some(keys),
                Row::Heading(_) => None,
            })
            .collect();
        assert!(
            readme.len() > 30,
            "only {} USAGE.md rows found",
            readme.len()
        );
        readme.sort();
        dialog.sort();
        let missing: Vec<_> = readme.iter().filter(|k| !dialog.contains(k)).collect();
        let extra: Vec<_> = dialog.iter().filter(|k| !readme.contains(k)).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "USAGE.md keys missing from the dialog: {missing:?}; dialog keys not in USAGE.md: {extra:?}"
        );
        assert_eq!(readme, dialog, "a key appears a different number of times");
    }

    #[test]
    fn the_rows_are_translated() {
        let pt = komorebi_i18n::with_lang(Lang::PtPt, rows);
        assert!(
            matches!(&pt[0], Row::Heading(h) if h == "Em qualquer lado"),
            "{:?}",
            pt[0]
        );
    }
}
