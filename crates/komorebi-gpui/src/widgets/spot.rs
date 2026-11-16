//! The sender spot (BRAND.md "The four moments"): a 32 px circle with the
//! sender's initials. Unread: solid ink, on-ink initials at 600. Read: wash
//! with ink-2 initials. On a selected row the read spot sits on paper. In
//! multi-select every spot turns into a round check: an ink-3 ring, then
//! ink with an on-ink tick; unread is then shown by the row's weight alone.
//! Avatars (contacts, recipient chips) are the same circle at other sizes.

use gpui::{
    App, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Window, div,
    prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, TextToken, Tokens, layout, radius};

/// Up to two initials from a display name or an address.
pub fn initials(name: &str) -> String {
    let name = name.trim();
    let base = name.split('@').next().unwrap_or(name);
    let words: Vec<&str> = base
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-')
        .filter(|w| w.chars().next().is_some_and(char::is_alphanumeric))
        .collect();
    let first = |w: &str| {
        w.chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
    };
    match words.as_slice() {
        [] => String::new(),
        [one] => first(one).unwrap_or_default(),
        [a, .., b] => format!(
            "{}{}",
            first(a).unwrap_or_default(),
            first(b).unwrap_or_default()
        ),
    }
}

/// The type step of a spot's initials at diameter `size`: the title step
/// (24 px semibold, the display face) on the large avatar (the mockup's
/// `.cd .av`), body from 40 px, caption below.
pub(crate) fn initials_step(t: &Tokens, size: f32) -> &TextToken {
    if size >= layout::AVATAR_LG {
        &t.text.title
    } else if size >= layout::AVATAR_MD {
        &t.text.body
    } else {
        &t.text.caption
    }
}

/// The read spot's initials colour at diameter `size`: ink on the large
/// avatar (the mockup's `.cd .av`), ink-2 on the smaller spots.
pub(crate) fn read_ink(t: &Tokens, size: f32) -> gpui::Hsla {
    if size >= layout::AVATAR_LG {
        t.color.ink
    } else {
        t.color.ink_2
    }
}

/// What the spot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotState {
    /// Initials, unread or read.
    Initials { unread: bool },
    /// Multi-select: a round check.
    Check { checked: bool },
}

/// A sender spot or avatar.
#[derive(IntoElement)]
pub struct SenderSpot {
    /// The sender's name: the check's accessible name.
    name: SharedString,
    initials: SharedString,
    state: SpotState,
    /// On a selected (wash) row: the read spot moves onto paper.
    on_wash: bool,
    size: f32,
    id: Option<SharedString>,
}

impl SenderSpot {
    pub fn new(name: &str) -> Self {
        Self {
            name: SharedString::from(name.trim().to_owned()),
            initials: initials(name).into(),
            state: SpotState::Initials { unread: false },
            on_wash: false,
            size: layout::SPOT,
            id: None,
        }
    }

    pub fn unread(mut self, unread: bool) -> Self {
        self.state = SpotState::Initials { unread };
        self
    }

    /// Multi-select: the spot is a round check. Give it an id: it is then
    /// a checkbox for screen readers.
    pub fn check(mut self, id: impl Into<SharedString>, checked: bool) -> Self {
        self.state = SpotState::Check { checked };
        self.id = Some(id.into());
        self
    }

    pub fn on_wash(mut self, on_wash: bool) -> Self {
        self.on_wash = on_wash;
        self
    }

    /// An id for the initials spot, so tests can find it (the check takes
    /// its id from [`SenderSpot::check`]).
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Another diameter (`layout::AVATAR_SM`, `AVATAR_MD`, `AVATAR_LG`).
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for SenderSpot {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let text = initials_step(&t, self.size);
        let read_ink = read_ink(&t, self.size);
        let tagged = |el: gpui::Div, id: Option<SharedString>| match id {
            Some(id) => el.id(id).test_support().into_any_element(),
            None => el.into_any_element(),
        };
        let base = div()
            .size(px(self.size))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(radius::FULL))
            .typo(text);
        match self.state {
            SpotState::Initials { unread: true } => tagged(
                base.bg(c.accent)
                    .text_color(c.on_accent)
                    .font_weight(t.text.body_strong.weight)
                    .child(self.initials),
                self.id,
            ),
            SpotState::Initials { unread: false } => tagged(
                base.bg(if self.on_wash { c.paper } else { c.wash })
                    .text_color(read_ink)
                    .child(self.initials),
                self.id,
            ),
            SpotState::Check { checked } => {
                let id = self.id.unwrap_or_else(|| "spot-check".into());
                base.id(id)
                    .role(Role::CheckBox)
                    .aria_label(self.name)
                    .aria_toggled(if checked {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .when(checked, |el| {
                        el.bg(c.accent).child(
                            svg()
                                .path(Icon::Checkmark.path())
                                .size(px(layout::ICON))
                                .text_color(c.on_accent),
                        )
                    })
                    .when(!checked, |el| {
                        el.border(px(layout::HAIRLINE)).border_color(c.ink_3)
                    })
                    .test_support()
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{Context, Render, TestAppContext};
    use gpui_kit::test::TestQueryExt as _;

    #[test]
    fn initials_come_from_names_and_addresses() {
        assert_eq!(initials("Marta Silva"), "MS");
        assert_eq!(initials("João Pedro Teixeira"), "JT");
        assert_eq!(initials("lumen.studio@example.com"), "LS");
        assert_eq!(initials("ana"), "A");
        assert_eq!(initials("  "), "");
        assert_eq!(initials("éva"), "É");
    }

    /// The large avatar's initials are in the title step (the mockup's
    /// `.cd .av`: the contact page and the gallery), the 40 px spot's in
    /// body and the smaller ones' in caption, at every text size.
    #[test]
    fn the_large_spot_has_title_initials() {
        use crate::theme::{ThemeInput, tokens};
        for scale in [1.0, 1.5, 2.25] {
            let t = tokens(&ThemeInput::new(
                komorebi::db::ThemeChoice::Light,
                komorebi::theme::SystemTheme {
                    text_scale: scale,
                    ..Default::default()
                },
            ));
            assert_eq!(initials_step(&t, layout::AVATAR_LG), &t.text.title);
            assert_eq!(initials_step(&t, layout::AVATAR_MD), &t.text.body);
            assert_eq!(initials_step(&t, layout::SPOT), &t.text.caption);
            assert_eq!(initials_step(&t, layout::AVATAR_SM), &t.text.caption);
            assert_eq!(read_ink(&t, layout::AVATAR_LG), t.color.ink);
            assert_eq!(read_ink(&t, layout::AVATAR_MD), t.color.ink_2);
            assert_eq!(read_ink(&t, layout::SPOT), t.color.ink_2);
        }
    }

    struct Demo;
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .child(SenderSpot::new("Marta Silva").unread(true))
                .child(SenderSpot::new("Rui Mendes").check("check-rm", true))
                .child(SenderSpot::new("Ana Rocha").check("check-ar", false))
        }
    }

    #[gpui_kit::test]
    fn checks_are_named_checkboxes(cx: &mut TestAppContext) {
        let (window, _) = test_support::open(cx, |_, _| Demo);
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                window.find_by_label("Rui Mendes").role(),
                Some(Role::CheckBox)
            );
            assert_eq!(
                window.find_by_label("Ana Rocha").role(),
                Some(Role::CheckBox)
            );
        })
        .expect("draw");
    }
}
