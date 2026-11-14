//! Small marks.
//!
//! [`Kbd`]: a shortcut as quiet text ("Ctrl+E", mockup `.search kbd`):
//! Kit's formatting of a GPUI keystroke, drawn as ink-3 caption text with
//! no keycap box (a box would be a second square).
//!
//! [`Badge`]: an unread count beside a folder (mockup `.ni .n`): ink-2
//! caption digits, no pill and no colour (BRAND.md: no square badge;
//! counts never animate).

use gpui::{
    App, ElementId, IntoElement, Keystroke, ParentElement, RenderOnce, Role, SharedString, Styled,
    Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::kbd::Kbd as KitKbd;

use crate::theme::{ActiveTokens as _, TextStyled as _};

/// "ctrl-e" as Windows writes it ("Ctrl+E"); the source when it does not
/// parse.
pub fn shortcut_text(binding: &str) -> String {
    binding
        .split(' ')
        .map(|stroke| {
            Keystroke::parse(stroke).map_or_else(|_| stroke.to_owned(), |k| KitKbd::format(&k))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A shortcut hint.
#[derive(IntoElement)]
pub struct Kbd {
    text: SharedString,
}

impl Kbd {
    /// From a GPUI binding ("ctrl-e", "ctrl-shift-7").
    pub fn new(binding: &str) -> Self {
        Self {
            text: shortcut_text(binding).into(),
        }
    }
}

impl RenderOnce for Kbd {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        div()
            .flex_none()
            .text_color(t.color.ink_3)
            .typo(&t.text.caption)
            .child(self.text)
    }
}

/// An unread count.
#[derive(IntoElement)]
pub struct Badge {
    id: ElementId,
    count: usize,
    /// "2 unread"
    name: SharedString,
}

impl Badge {
    pub fn new(id: impl Into<ElementId>, count: usize, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            count,
            name: name.into(),
        }
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        div()
            .id(self.id)
            .role(Role::Label)
            .aria_label(self.name)
            .flex_none()
            .text_color(t.color.ink_2)
            .typo(&t.text.caption)
            .when(self.count > 0, |el| el.child(self.count.to_string()))
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_as_windows_writes_them() {
        assert_eq!(shortcut_text("ctrl-e"), "Ctrl+E");
        assert_eq!(shortcut_text("ctrl-shift-7"), "Ctrl+Shift+7");
        assert_eq!(shortcut_text("delete"), "Delete");
    }
}
