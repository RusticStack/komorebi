//! Text fields: a Kit single-line `Input` with `appearance(false)` inside a
//! box the brand draws (cookbook_input_appearance), so the caret, selection,
//! IME and clipboard are Kit's and the look is ours.
//!
//! - [`FieldLook::Pill`]: an inset pill, 40 px (settings, dialogs).
//! - [`FieldLook::Underline`]: the composer's 48 px row with its label and
//!   the one 1 px line under it (BRAND.md "Edge").
//! - [`search_field`]: the title bar's 32 px paper pill with the search
//!   glyph and its shortcut.
//!
//! [`TextField::navigation`] gives Up, Down, Page Up, Page Down, Home, End,
//! Enter and Escape to the caller instead of the caret (Skua's palette:
//! `capture_action` on an ancestor plus `stop_propagation`,
//! cookbook_input_actions): a search box that moves a list's selection.

use std::rc::Rc;

use gpui::{
    App, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, Styled, Window, div, prelude::*, px, svg,
};
use gpui_kit::component::input::{
    Enter, Escape, Input, InputState, MoveDown, MoveEnd, MoveHome, MovePageDown, MovePageUp, MoveUp,
};

use super::focus_ring;
use crate::icons::Icon;
use crate::theme::metrics::scaled;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};

/// A key a navigating field hands to its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    /// Ctrl+Enter.
    SecondaryEnter,
    Escape,
}

type OnNav = Rc<dyn Fn(Nav, &mut Window, &mut App)>;

/// How a field is drawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FieldLook {
    #[default]
    Pill,
    /// The composer's row: a label on the left and a line under the row.
    Underline { label: SharedString },
    /// The title bar's search pill: paper on canvas, 32 px.
    Search,
    /// An inset pill in a 32 px line (a folder's name typed in the folder
    /// pane).
    Inline,
}

/// A single-line text field over a Kit `InputState` the caller owns.
#[derive(IntoElement)]
pub struct TextField {
    state: Entity<InputState>,
    name: SharedString,
    look: FieldLook,
    leading: Option<Icon>,
    trailing: Option<SharedString>,
    on_nav: Option<OnNav>,
    /// Only Enter and Escape go to `on_nav` (the caret keeps the rest).
    edit_keys: bool,
    disabled: bool,
}

impl TextField {
    /// `name` is the accessible name.
    pub fn new(state: &Entity<InputState>, name: impl Into<SharedString>) -> Self {
        Self {
            state: state.clone(),
            name: name.into(),
            look: FieldLook::default(),
            leading: None,
            trailing: None,
            on_nav: None,
            edit_keys: false,
            disabled: false,
        }
    }

    pub fn look(mut self, look: FieldLook) -> Self {
        self.look = look;
        self
    }

    /// A glyph before the text (search).
    pub fn leading(mut self, icon: Icon) -> Self {
        self.leading = Some(icon);
        self
    }

    /// Quiet text after the field (a shortcut, "Cc Bcc").
    pub fn trailing(mut self, text: impl Into<SharedString>) -> Self {
        self.trailing = Some(text.into());
        self
    }

    /// Hands the list keys to `f` instead of the caret.
    pub fn navigation(mut self, f: impl Fn(Nav, &mut Window, &mut App) + 'static) -> Self {
        self.on_nav = Some(Rc::new(f));
        self
    }

    /// Hands Enter and Escape to `f` (commit, cancel); the caret keeps
    /// the arrows, Home and End (a name typed in place).
    pub fn edit_keys(mut self, f: impl Fn(Nav, &mut Window, &mut App) + 'static) -> Self {
        self.on_nav = Some(Rc::new(f));
        self.edit_keys = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Captures one Kit input action, hands it over and stops it.
macro_rules! take {
    ($el:expr, $nav:expr, $($action:ty => $key:expr),*) => {
        $el$(.capture_action({
            let nav = $nav.clone();
            move |_: &$action, window: &mut Window, cx: &mut App| {
                nav($key, window, cx);
                cx.stop_propagation();
            }
        }))*
    };
}

impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let ring = focused && window.last_input_was_keyboard();
        // Kit sizes its text and padding by its own scale; the brand's body
        // step and the box's padding win (the style is refined last).
        let input = Input::new(&self.state)
            .appearance(false)
            .disabled(self.disabled)
            .aria_label(self.name)
            .typo(&t.text.body)
            .px(Pixels::ZERO);
        let height = scaled::control(
            &t,
            match self.look {
                FieldLook::Pill => layout::CONTROL_H_LG,
                FieldLook::Underline { .. } => layout::FIELD_H,
                FieldLook::Search => layout::SEARCH_H,
                FieldLook::Inline => layout::CONTROL_H,
            },
        );
        let mut row = div()
            .h(px(height))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::SP_2_5))
            .text_color(c.ink)
            .typo(&t.text.body);
        row = match &self.look {
            FieldLook::Pill => row
                .px(px(space::SP_3))
                .rounded(px(radius::FULL))
                .bg(c.inset)
                .when(ring, |el| el.shadow(focus_ring(&t))),
            FieldLook::Inline => row
                .px(px(space::SP_2))
                .rounded(px(radius::FULL))
                .bg(c.inset)
                .when(ring, |el| el.shadow(focus_ring(&t))),
            FieldLook::Search => row
                .w(px(layout::SEARCH_W))
                .px(px(space::SP_3))
                .rounded(px(radius::FULL))
                .bg(c.paper)
                .when(ring, |el| el.shadow(focus_ring(&t))),
            // The ink line says where the focus is; the keyboard's focus
            // gets the brand ring too, as every focusable widget.
            FieldLook::Underline { label } => row
                .border_b(px(layout::HAIRLINE))
                .border_color(if focused { c.ink } else { c.line })
                .when(ring, |el| {
                    el.rounded(px(radius::CARD)).shadow(focus_ring(&t))
                })
                .child(
                    div()
                        .w(px(layout::FIELD_LABEL_W))
                        .flex_none()
                        .text_color(c.ink_3)
                        .child(label.clone()),
                ),
        };
        row = row
            .when_some(self.leading, |el, icon| {
                el.child(
                    svg()
                        .path(icon.path())
                        .size(px(layout::ICON))
                        .flex_none()
                        .text_color(c.ink_3),
                )
            })
            .child(div().flex_1().min_w_0().child(Styled::h(input, px(height))))
            .when_some(self.trailing, |el, text| {
                el.child(
                    div()
                        .flex_none()
                        .text_color(c.ink_3)
                        .typo(&t.text.caption)
                        .child(text),
                )
            });
        match self.on_nav {
            Some(nav) if self.edit_keys => take!(row, nav, Escape => Nav::Escape)
                .capture_action(move |_: &Enter, window, cx| {
                    nav(Nav::Enter, window, cx);
                    cx.stop_propagation();
                })
                .into_any_element(),
            Some(nav) => take!(row, nav,
                MoveUp => Nav::Up, MoveDown => Nav::Down, MovePageUp => Nav::PageUp,
                MovePageDown => Nav::PageDown, MoveHome => Nav::Home, MoveEnd => Nav::End,
                Escape => Nav::Escape)
            .capture_action(move |action: &Enter, window, cx| {
                let key = if action.secondary {
                    Nav::SecondaryEnter
                } else {
                    Nav::Enter
                };
                nav(key, window, cx);
                cx.stop_propagation();
            })
            .into_any_element(),
            None => row.into_any_element(),
        }
    }
}

/// The title bar's search pill: the search glyph, a placeholder (on the
/// state) and the shortcut hint.
pub fn search_field(
    state: &Entity<InputState>,
    name: impl Into<SharedString>,
    shortcut: impl Into<SharedString>,
) -> TextField {
    TextField::new(state, name)
        .look(FieldLook::Search)
        .leading(Icon::Search)
        .trailing(shortcut)
}

type OnRemove = Rc<dyn Fn(&mut Window, &mut App)>;

/// One recipient chip of a [`ChipField`].
pub struct ChipEntry {
    pub id: SharedString,
    /// The name shown (or the address when there is none).
    pub label: SharedString,
    /// The dismiss button's accessible name ("Remove Marta Silva").
    pub remove_name: SharedString,
    /// A valid address (an invalid one is also said by the field's error).
    pub valid: bool,
    pub on_remove: OnRemove,
}

/// The composer's address row (mockup `.fld` with `.chip`s): its label,
/// the chips, a Kit field for the address being typed, all in one wrapping
/// row with the one 1 px line under it (ink while the field has the focus),
/// an optional quiet trailing button ("Cc  Bcc") and an inline error.
#[derive(IntoElement)]
pub struct ChipField {
    state: Entity<InputState>,
    name: SharedString,
    id: SharedString,
    chips: Vec<ChipEntry>,
    trailing: Option<(SharedString, SharedString, OnRemove)>,
    error: Option<SharedString>,
    /// Whether a list of suggestions under the field is open.
    expanded: Option<bool>,
}

impl ChipField {
    /// `name` is the label and the field's accessible name.
    pub fn new(state: &Entity<InputState>, name: impl Into<SharedString>) -> Self {
        Self {
            state: state.clone(),
            name: name.into(),
            id: SharedString::default(),
            chips: Vec::new(),
            trailing: None,
            error: None,
            expanded: None,
        }
    }

    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    pub fn chips(mut self, chips: Vec<ChipEntry>) -> Self {
        self.chips = chips;
        self
    }

    /// A quiet button at the row's end: its text, accessible name, action.
    pub fn trailing(
        mut self,
        text: impl Into<SharedString>,
        name: impl Into<SharedString>,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.trailing = Some((text.into(), name.into(), Rc::new(f)));
        self
    }

    /// The inline error under the row (none: no error).
    pub fn error(mut self, error: Option<SharedString>) -> Self {
        self.error = error;
        self
    }

    /// Tells assistive technology whether the suggestions under the field
    /// are open (the field's expanded state).
    pub fn expanded(mut self, open: bool) -> Self {
        self.expanded = Some(open);
        self
    }
}

impl RenderOnce for ChipField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use super::pill::Chip;
        use crate::theme::composer as geo;
        use gpui::{ElementId, Role};
        use gpui_kit::TestSupportExt as _;

        let t = cx.tokens().clone();
        let c = t.color;
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let ring = focused && window.last_input_was_keyboard();
        let input = Input::new(&self.state)
            .appearance(false)
            .aria_label(self.name.clone())
            .typo(&t.text.body)
            .px(Pixels::ZERO);
        let input = match self.expanded {
            Some(open) => input.aria_expanded(open),
            None => input,
        };
        let chips = self.chips.into_iter().map(|e| {
            let f = e.on_remove.clone();
            Chip::new(e.id, e.label, e.remove_name).on_remove(move |w, cx| f(w, cx))
        });
        let row = div()
            .w_full()
            .min_h(px(layout::FIELD_H))
            .py(px(geo::FIELD_PAD_Y))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(geo::FIELD_GAP))
            .border_b(px(layout::HAIRLINE))
            .border_color(if focused { c.ink } else { c.line })
            // Keyboard focus: the brand ring as well as the ink line.
            .when(ring, |el| {
                el.rounded(px(radius::CARD)).shadow(focus_ring(&t))
            })
            .text_color(c.ink)
            .typo(&t.text.body)
            .child(
                div()
                    .w(px(layout::FIELD_LABEL_W))
                    .flex_none()
                    .text_color(c.ink_3)
                    .child(self.name.clone()),
            )
            // The chips and the text typed wrap between the label and the
            // trailing button.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(geo::FIELD_GAP))
                    .children(chips)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(geo::CHIP_INPUT_MIN_W))
                            .child(Styled::h(input, px(scaled::control(&t, layout::CHIP_H)))),
                    ),
            )
            .when_some(self.trailing, |el, (text, name, f)| {
                el.child(
                    div()
                        .id(ElementId::Name(format!("{}-more", self.id).into()))
                        .role(Role::Button)
                        .aria_label(name)
                        .tab_index(0)
                        .flex_none()
                        .px(px(space::SP_1))
                        .rounded(px(radius::FULL))
                        .text_color(c.ink_3)
                        .typo(&t.text.preview)
                        .hover(move |s| s.text_color(c.ink))
                        .focus_visible(super::ring_style(&t))
                        .child(text)
                        .on_click(move |_, w, cx| f(w, cx))
                        .test_support(),
                )
            });
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(row)
            .when_some(self.error, |el, error| {
                el.child(
                    div()
                        .id(ElementId::Name(format!("{}-error", self.id).into()))
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .pl(px(layout::FIELD_LABEL_W + geo::FIELD_GAP))
                        .pt(px(space::SP_1))
                        .typo(&t.text.caption)
                        .text_color(c.ink)
                        .child(error)
                        .test_support(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::test_support;
    use gpui::{Context, Render, TestAppContext};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    struct Demo {
        search: Entity<InputState>,
        to: Entity<InputState>,
        keys: Vec<Nav>,
    }

    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let view = cx.entity().downgrade();
            div()
                .flex()
                .flex_col()
                .gap(px(space::SP_4))
                .child(
                    search_field(&self.search, "Search mail", "Ctrl+E").navigation(
                        move |key, _, cx| {
                            let _ = view.update(cx, |v, _| v.keys.push(key));
                        },
                    ),
                )
                .child(
                    TextField::new(&self.to, "To")
                        .look(FieldLook::Underline { label: "To".into() }),
                )
        }
    }

    struct Recipients {
        to: Entity<InputState>,
    }

    impl Render for Recipients {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().p(px(space::SP_4)).child(
                ChipField::new(&self.to, "To")
                    .id("to")
                    .error(Some("not an address".into())),
            )
        }
    }

    /// Whether `shadow` paints at `p` (scaled px), as the renderer draws it:
    /// a drop shadow fills its rect, an inset one its element minus its rect
    /// (corners ignored: `p` is far from them).
    fn paints(shadow: &gpui::Shadow, p: gpui::Point<gpui::ScaledPixels>) -> bool {
        let mask = shadow.content_mask.bounds;
        if !mask.contains(&p) {
            return false;
        }
        if shadow.inset == 0 {
            shadow.bounds.contains(&p)
        } else {
            shadow.element_bounds.contains(&p) && !shadow.bounds.contains(&p)
        }
    }

    /// The owner's bug: typing in the composer's To field (keyboard focus)
    /// drew the focus ring's ink over the whole row, light in dark mode and
    /// dark in light mode, and the ink text vanished. The ring is two crisp
    /// spread shadows on a row with no background; it must paint outside
    /// the row only, never under the text.
    fn the_ring_stays_outside_a_typed_in_field(cx: &mut TestAppContext, dark: bool) {
        let (window, view) = test_support::open(cx, |window, cx| Recipients {
            to: cx.new(|cx| InputState::new(window, cx)),
        });
        if dark {
            cx.update(|cx| {
                crate::theme::install(
                    crate::theme::ThemeInput::new(
                        komorebi::db::ThemeChoice::Dark,
                        komorebi::theme::SystemTheme::default(),
                    ),
                    cx,
                );
            });
        }
        let to = view.read_with(cx, |v, _| v.to.clone());
        cx.update_window(window.into(), |_, window, cx| {
            let handle = to.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            window.input("aaasdasdAaaa", cx);
            window.draw(cx).clear(cx);
            let ink = cx.tokens().color.ink;
            let shadows = window.painted_shadows();
            assert!(
                shadows.iter().any(|s| s.color == ink),
                "the keyboard focus draws the ink ring: {shadows:?}"
            );
            let text = window.find_by_label("To").bounds();
            let centre = text.center().scale(window.scale_factor());
            for s in &shadows {
                assert!(
                    !paints(s, centre),
                    "a shadow paints under the typed text (dark {dark}): {s:?}"
                );
            }
            // Outside the row: the 2 px paper gap, then the 2 px ink ring
            // (the shadows are in the order they are drawn, the last on top).
            let row = shadows
                .iter()
                .find(|s| s.color == ink)
                .expect("the ink ring")
                .bounds;
            let scale = window.scale_factor();
            let top = |dx: f32| {
                let p = gpui::point(row.left() - gpui::ScaledPixels(dx * scale), row.center().y);
                shadows.iter().rev().find(|s| paints(s, p)).map(|s| s.color)
            };
            let paper = cx.tokens().color.paper;
            assert_eq!(top(1.), Some(paper), "the gap next to the row");
            assert_eq!(top(3.), Some(ink), "the ring outside the gap");
            assert_eq!(top(5.), None, "nothing past the ring");
        })
        .expect("type");
        assert_eq!(
            to.read_with(cx, |s, _| s.value().to_string()),
            "aaasdasdAaaa"
        );
    }

    #[gpui_kit::test]
    fn the_ring_stays_outside_a_typed_in_field_in_dark_mode(cx: &mut TestAppContext) {
        the_ring_stays_outside_a_typed_in_field(cx, true);
    }

    #[gpui_kit::test]
    fn the_ring_stays_outside_a_typed_in_field_in_light_mode(cx: &mut TestAppContext) {
        the_ring_stays_outside_a_typed_in_field(cx, false);
    }

    #[gpui_kit::test]
    fn the_search_field_hands_list_keys_over_and_keeps_typing(cx: &mut TestAppContext) {
        let (window, view) = test_support::open(cx, |window, cx| Demo {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search mail")),
            to: cx.new(|cx| InputState::new(window, cx)),
            keys: Vec::new(),
        });
        let search = view.read_with(cx, |v, _| v.search.clone());
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            assert!(window.try_find_by_label("Search mail").is_some());
            assert!(window.try_find_by_label("To").is_some());
            let handle = search.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
            window.draw(cx).clear(cx);
            for key in ["down", "up", "home", "end", "enter", "escape"] {
                window.press(key, cx);
            }
            window.input("invoice", cx);
        })
        .expect("type");
        cx.run_until_parked();
        view.read_with(cx, |v, cx| {
            assert_eq!(
                v.keys,
                [
                    Nav::Down,
                    Nav::Up,
                    Nav::Home,
                    Nav::End,
                    Nav::Enter,
                    Nav::Escape
                ]
            );
            assert_eq!(v.search.read(cx).value().to_string(), "invoice");
        });
    }
}
