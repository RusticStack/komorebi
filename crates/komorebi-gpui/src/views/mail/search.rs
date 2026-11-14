//! The title bar's search (docs/WINUI3.md "AutoSuggestBox", the Slint
//! `SearchBox`): its placeholder names the view it searches; typing filters
//! the list (large folders after a pause, `model::debounces_search`); Down
//! or Enter hands the keys to the list, Esc clears the text, then gives the
//! list the keys back. The scope (this view, or all folders) is under the
//! list's title while a search runs (`filters.rs`).

use gpui::{Context, Entity, Window};

use super::model::Mail;
use super::{MailView, text};
use crate::widgets::field::{Nav, TextField, search_field};

/// The search box's placeholder: "Search" and the view.
pub fn placeholder(mail: &Mail) -> String {
    let folder = match mail.view() {
        super::model::View::Folder(id) => mail.folder(id),
        _ => None,
    };
    text::search_in(&text::view_name(mail.view(), folder))
}

/// The search pill for the title bar's centre slot.
pub fn field(view: &Entity<MailView>, cx: &gpui::App) -> TextField {
    let state = view.read(cx).search_input().clone();
    let target = view.downgrade();
    search_field(&state, text::search_name(), "Ctrl+E").navigation(move |key, window, cx| {
        let _ = target.update(cx, |this, cx| this.search_nav(key, window, cx));
    })
}

/// The scope beside the title bar's search (the Slint `ScopeBox`): a
/// pill naming the scope, with a menu of this folder (or these inboxes)
/// and all folders; off in Drafts and the Outbox. The window places it
/// next to [`field`].
pub fn scope_box(view: &Entity<MailView>, cx: &gpui::App) -> gpui::AnyElement {
    use gpui::IntoElement as _;
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let mail = &view.read(cx).mail;
    let here = text::scope_here(mail.view());
    let all = mail.search_all() && mail.lists_messages();
    let label = if all { text::scope_all() } else { here.clone() };
    let target = view.downgrade();
    crate::widgets::button::Button::new("search-scope")
        .variant(crate::widgets::button::Variant::Subtle)
        .label(label)
        .trailing(crate::icons::Icon::ChevronDown)
        .tooltip(text::search_scope())
        .name(text::search_scope())
        .disabled(!mail.lists_messages())
        .dropdown_menu(move |menu, _window, cx| {
            let menu = crate::widgets::menu::brand(menu, cx);
            let (to_here, to_all) = (target.clone(), target.clone());
            menu.item(PopupMenuItem::new(here.clone()).checked(!all).on_click(
                move |_, window, cx| {
                    let _ = to_here.update(cx, |v, cx| {
                        v.act(window, cx, |m| m.set_search_all(false));
                    });
                },
            ))
            .item(PopupMenuItem::new(text::scope_all()).checked(all).on_click(
                move |_, window, cx| {
                    let _ = to_all.update(cx, |v, cx| {
                        v.act(window, cx, |m| m.set_search_all(true));
                    });
                },
            ))
        })
        .into_any_element()
}

/// A list key pressed in the search box.
pub fn navigate(view: &mut MailView, key: Nav, window: &mut Window, cx: &mut Context<MailView>) {
    match key {
        Nav::Down | Nav::Enter | Nav::SecondaryEnter | Nav::PageDown => {
            // A search with nothing current starts on the first result.
            if view.mail.current_row().is_none() && !view.mail.rows().is_empty() {
                view.act(window, cx, |m| m.select_offset(1));
            }
            window.focus(&view.list_focus, cx);
        }
        Nav::Escape => {
            if view.search.read(cx).value().is_empty() {
                window.focus(&view.list_focus, cx);
            } else {
                view.clear_search(window, cx);
            }
        }
        Nav::Up | Nav::PageUp | Nav::Home | Nav::End => {}
    }
}

impl MailView {
    /// Clear search: the text and the list.
    pub fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        self.act(window, cx, Mail::clear_search);
    }

    /// The folder of the view shown, if it is one.
    pub(super) fn view_folder(&self) -> Option<&komorebi::db::Folder> {
        match self.mail.view() {
            super::model::View::Folder(id) => self.mail.folder(id),
            _ => None,
        }
    }
}
