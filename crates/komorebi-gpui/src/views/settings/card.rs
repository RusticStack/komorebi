//! The settings page pieces of the mockup (`settings` in
//! komorebi-brand `final/app/app.html`): the page heading with its lede, a
//! group heading (`.sg`), a card (`.card`: inset, 12 px, 64 px at least,
//! the title with its note under it, the control at the right end), the
//! segmented control (`.seg`) and a labelled field for the forms.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement, RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled,
    Toggled, Window, div, prelude::*, px, svg,
};
use gpui_kit::TestSupportExt as _;

use crate::icons::Icon;
use crate::theme::settings as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::widgets::ring_style;

/// The widest a card is (the mockup's 760 px).
pub const CARD_MAX_W: f32 = 760.;

/// The page heading: the section's name and one line under it.
pub fn page_head(
    title: impl Into<SharedString>,
    lede: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    let t = cx.tokens();
    let c = t.color;
    div()
        .flex()
        .flex_col()
        .max_w(px(CARD_MAX_W))
        .child(
            div()
                .id("page-heading")
                .role(Role::Heading)
                .aria_level(1)
                .typo(&t.text.large)
                .text_color(c.ink)
                .child(title.into()),
        )
        .when_some(lede, |el, lede| {
            el.child(
                div()
                    .mt(px(space::SP_1))
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(lede),
            )
        })
        .into_any_element()
}

/// A group heading inside a page ("Appearance", "Diagnostics").
pub fn group_head(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    let t = cx.tokens();
    let text = text.into();
    div()
        .id(SharedString::from(format!("group-{text}")))
        .role(Role::Heading)
        .aria_level(2)
        .mt(px(geo::GROUP_TOP))
        .mb(px(geo::GROUP_BOTTOM))
        .typo(&t.text.body_strong)
        .text_color(t.color.ink)
        .child(text)
        .into_any_element()
}

/// A field with its label above it (Fluent's forms), and a quiet line
/// under it when there is one.
pub fn labelled(
    label: impl Into<SharedString>,
    field: impl IntoElement,
    hint: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    let t = cx.tokens();
    let c = t.color;
    div()
        .flex()
        .flex_col()
        .gap(px(geo::LABEL_GAP))
        .min_w_0()
        .child(
            div()
                .typo(&t.text.body)
                .text_color(c.ink_2)
                .child(label.into()),
        )
        .child(field)
        .when_some(hint, |el, hint| {
            el.child(div().typo(&t.text.caption).text_color(c.ink_3).child(hint))
        })
        .into_any_element()
}

/// A quiet sentence under a group or inside a card.
pub fn note(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    let t = cx.tokens();
    div()
        .max_w(px(CARD_MAX_W))
        .typo(&t.text.caption)
        .text_color(t.color.ink_3)
        .child(text.into())
        .into_any_element()
}

/// A settings card: an optional lead icon, a title with its note, the
/// control at the right end, and anything that goes under them.
#[derive(IntoElement)]
pub struct Card {
    id: ElementId,
    icon: Option<Icon>,
    /// The icon's column: its own 20 px, or wider to line up with a
    /// column of 32 px spots above it (the accounts' cards).
    icon_w: f32,
    title: SharedString,
    note: Option<SharedString>,
    trailing: Vec<AnyElement>,
    below: Vec<AnyElement>,
}

impl Card {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            icon_w: layout::ICON_LG,
            title: title.into(),
            note: None,
            trailing: Vec::new(),
            below: Vec::new(),
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The icon centred in a column as wide as a sender spot, so the title
    /// lines up with cards that lead with a spot.
    pub fn icon_as_spot(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self.icon_w = layout::SPOT;
        self
    }

    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// A control at the right end (a toggle, a select, a button).
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.trailing.push(control.into_any_element());
        self
    }

    /// Content under the title row, the card's full width.
    pub fn below(mut self, content: impl IntoElement) -> Self {
        self.below.push(content.into_any_element());
        self
    }
}

impl RenderOnce for Card {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        div()
            .id(self.id)
            .w_full()
            .max_w(px(CARD_MAX_W))
            .min_h(px(layout::CARD_H))
            .flex()
            .flex_col()
            .justify_center()
            .mb(px(space::SP_1))
            .px(px(space::SP_4))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .text_color(c.ink)
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(space::SP_4))
                    .when_some(self.icon, |el, icon| {
                        el.child(
                            div()
                                .w(px(self.icon_w))
                                .flex_none()
                                .flex()
                                .justify_center()
                                .child(card_icon(icon, &t)),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py(px(space::SP_3))
                            .child(div().typo(&t.text.body).child(self.title))
                            .when_some(self.note, |el, note| {
                                el.child(
                                    div()
                                        .mt(px(space::SP_0_5))
                                        .typo(&t.text.caption)
                                        .text_color(c.ink_3)
                                        .child(note),
                                )
                            }),
                    )
                    .children(self.trailing),
            )
            .children(self.below.into_iter().map(|el| {
                div()
                    .w_full()
                    .pb(px(space::SP_3))
                    .child(el)
                    .into_any_element()
            }))
    }
}

/// A settings card's icon: Fluent's 20 px glyph beside the 14 px title (WinUI's
/// SettingsCard header icon), in the title's ink.
pub fn card_icon(icon: Icon, t: &crate::theme::Tokens) -> gpui::Svg {
    svg()
        .path(icon.at(layout::ICON_LG))
        .size(px(layout::ICON_LG))
        .flex_none()
        .text_color(t.color.ink)
}

type OnPick = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

/// Where an arrow key moves the choice of a radio group of `count` (Fluent
/// and UI Automation radio groups: one Tab stop, Up and Left go back, Down
/// and Right go on, both wrap; Home and End go to the ends). `None`: not a
/// key of the group.
pub fn radio_step(key: &str, at: usize, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = at.min(count - 1);
    match key {
        "left" | "up" => Some(if at == 0 { count - 1 } else { at - 1 }),
        "right" | "down" => Some(if at + 1 == count { 0 } else { at + 1 }),
        "home" => Some(0),
        "end" => Some(count - 1),
        _ => None,
    }
}

/// The focus handles of a radio group's choices, kept between frames under
/// the group's id: the chosen one is the group's one Tab stop.
pub fn radio_handles(
    id: &SharedString,
    count: usize,
    chosen: usize,
    window: &mut Window,
    cx: &mut App,
) -> Vec<FocusHandle> {
    (0..count)
        .map(|ix| {
            let state = window.use_keyed_state(
                SharedString::from(format!("{id}-focus-{ix}")),
                cx,
                |_, cx| cx.focus_handle(),
            );
            state.read(cx).clone().tab_index(0).tab_stop(ix == chosen)
        })
        .collect()
}

/// The segmented control (`.seg`): a few choices side by side on an inset
/// track, the chosen one on paper and semibold. A radio group to UI
/// Automation; each segment is a radio button named by its label.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    name: SharedString,
    choices: Vec<(SharedString, SharedString)>,
    value: SharedString,
    on_pick: Option<OnPick>,
}

impl Segmented {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            choices: Vec::new(),
            value: value.into(),
            on_pick: None,
        }
    }

    pub fn choices(
        mut self,
        choices: impl IntoIterator<Item = (impl Into<SharedString>, impl Into<SharedString>)>,
    ) -> Self {
        self.choices = choices
            .into_iter()
            .map(|(v, l)| (v.into(), l.into()))
            .collect();
        self
    }

    pub fn on_pick(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }
}

/// Whether the chosen segment needs an outline to stand out: its pill is
/// barely different from the track (under 1.5:1) in a theme other than
/// Komorebi's own presets, whose look is pinned (there the pill and the
/// stronger text weight mark it).
pub fn outline_selected(cx: &App) -> bool {
    use crate::theme::appearance::ThemeRef;
    use crate::theme::presets::{Preset, Rgb, contrast};
    let c = cx.tokens().color;
    let faint = contrast(Rgb::from_colour(c.segment_on), Rgb::from_colour(c.inset)) < 1.5;
    let pinned = matches!(
        crate::theme::appearance(cx).theme,
        ThemeRef::Default | ThemeRef::Preset(Preset::Komorebi)
    );
    faint && !pinned
}

impl RenderOnce for Segmented {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let count = self.choices.len();
        let chosen = self
            .choices
            .iter()
            .position(|(v, _)| *v == self.value)
            .unwrap_or(0);
        let handles = radio_handles(&self.id, count, chosen, window, cx);
        let outline = outline_selected(cx);
        let values: Vec<SharedString> = self.choices.iter().map(|(v, _)| v.clone()).collect();
        let segments = self
            .choices
            .into_iter()
            .enumerate()
            .map(|(ix, (value, label))| {
                let on = value == self.value;
                let pick = self.on_pick.clone();
                div()
                    .id(SharedString::from(format!("{}-{ix}", self.id)))
                    .track_focus(&handles[ix])
                    .role(Role::RadioButton)
                    .aria_label(label.clone())
                    .aria_toggled(if on { Toggled::True } else { Toggled::False })
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(count)
                    .h(px(geo::SEG_H))
                    .px(px(geo::SEG_PAD_X))
                    .flex()
                    .items_center()
                    .rounded(px(radius::FULL))
                    .typo(if on {
                        &t.text.body_strong
                    } else {
                        &t.text.body
                    })
                    .text_color(if on { c.ink } else { c.ink_2 })
                    .when(on, |el| el.bg(c.segment_on))
                    .when(on && outline, |el| el.border_1().border_color(c.ink_3))
                    .when(!on, |el| el.hover(move |s| s.text_color(c.ink)))
                    .focus_visible(ring_style(&t))
                    .child(label)
                    .when_some(pick, |el, pick| {
                        el.on_click(move |_, window, cx| pick(&value, window, cx))
                    })
                    .test_support()
            });
        let pick = self.on_pick.clone();
        let key_handles = handles.clone();
        div()
            .id(self.id.clone())
            .role(Role::RadioGroup)
            .aria_label(self.name)
            .flex()
            .flex_none()
            .p(px(geo::SEG_PAD))
            .rounded(px(radius::FULL))
            .bg(c.inset)
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                if e.keystroke.modifiers.modified() {
                    return;
                }
                let at = key_handles
                    .iter()
                    .position(|h| h.is_focused(window))
                    .unwrap_or(chosen);
                let Some(to) = radio_step(e.keystroke.key.as_str(), at, count) else {
                    return;
                };
                cx.stop_propagation();
                window.focus(&key_handles[to], cx);
                if let Some(pick) = &pick {
                    pick(&values[to], window, cx);
                }
            })
            .children(segments)
            .test_support()
    }
}

/// Radio buttons with a line under each label (the Slint app's
/// `RadioButtons` with descriptions): a radio group to UI Automation, one
/// Tab stop (the chosen choice, else the first), the arrows move the choice.
/// A choice that changes the view ([`RadioList::navigates`]) only takes the
/// focus from the arrows; a click, Space or Enter picks it.
#[derive(IntoElement)]
pub struct RadioList {
    id: SharedString,
    name: SharedString,
    choices: Vec<(SharedString, SharedString, SharedString)>,
    value: SharedString,
    on_pick: Option<OnPick>,
    navigates: Vec<SharedString>,
}

impl RadioList {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            choices: Vec::new(),
            value: value.into(),
            on_pick: None,
            navigates: Vec::new(),
        }
    }

    /// The choice `value` leaves the list (Other server opens manual
    /// setup): the arrows move the focus onto it without picking it, so a
    /// keyboard or Narrator user can step past it.
    pub fn navigates(mut self, value: impl Into<SharedString>) -> Self {
        self.navigates.push(value.into());
        self
    }

    /// (value, label, description) of each choice.
    pub fn choices(mut self, choices: impl IntoIterator<Item = (String, String, String)>) -> Self {
        self.choices = choices
            .into_iter()
            .map(|(v, l, d)| (v.into(), l.into(), d.into()))
            .collect();
        self
    }

    pub fn on_pick(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for RadioList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let count = self.choices.len();
        let chosen = self
            .choices
            .iter()
            .position(|(v, _, _)| *v == self.value)
            .unwrap_or(0);
        let handles = radio_handles(&self.id, count, chosen, window, cx);
        let values: Vec<SharedString> = self.choices.iter().map(|(v, _, _)| v.clone()).collect();
        let rows: Vec<AnyElement> = self
            .choices
            .into_iter()
            .enumerate()
            .map(|(ix, (value, label, description))| {
                let on = value == self.value;
                let pick = self.on_pick.clone();
                div()
                    .id(SharedString::from(format!("{}-{ix}", self.id)))
                    .track_focus(&handles[ix])
                    .role(Role::RadioButton)
                    .aria_label(label.clone())
                    .aria_description(description.clone())
                    .aria_toggled(if on { Toggled::True } else { Toggled::False })
                    .aria_position_in_set(ix + 1)
                    .aria_size_of_set(count)
                    .flex()
                    .gap(px(space::SP_3))
                    .p(px(space::SP_2))
                    .rounded(px(radius::CARD))
                    .when(on, |el| el.bg(c.inset))
                    .hover(move |s| s.bg(c.wash))
                    .focus_visible(ring_style(&t))
                    .cursor_pointer()
                    .child(
                        div()
                            .mt(px(space::SP_0_5))
                            .size(px(layout::ICON))
                            .flex_none()
                            .rounded(px(radius::FULL))
                            .border(px(layout::FOCUS_RING))
                            .border_color(if on { c.accent } else { c.ink_2 })
                            .when(on, |el| el.bg(c.accent)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().typo(&t.text.body).text_color(c.ink).child(label))
                            .child(
                                div()
                                    .typo(&t.text.caption)
                                    .text_color(c.ink_3)
                                    .child(description),
                            ),
                    )
                    .when_some(pick, |el, pick| {
                        el.on_click(move |_, window, cx| pick(&value, window, cx))
                    })
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let pick = self.on_pick.clone();
        let navigates = self.navigates;
        div()
            .id(self.id.clone())
            .role(Role::RadioGroup)
            .aria_label(self.name)
            .flex()
            .flex_col()
            .gap(px(space::SP_1))
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                if e.keystroke.modifiers.modified() {
                    return;
                }
                let at = handles
                    .iter()
                    .position(|h| h.is_focused(window))
                    .unwrap_or(chosen);
                let Some(to) = radio_step(e.keystroke.key.as_str(), at, count) else {
                    return;
                };
                cx.stop_propagation();
                window.focus(&handles[to], cx);
                if navigates.contains(&values[to]) {
                    return;
                }
                if let Some(pick) = &pick {
                    pick(&values[to], window, cx);
                }
            })
            .children(rows)
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::radio_step;

    #[test]
    fn arrows_move_a_radio_choice_and_wrap() {
        assert_eq!(radio_step("right", 0, 3), Some(1));
        assert_eq!(radio_step("down", 2, 3), Some(0));
        assert_eq!(radio_step("left", 0, 3), Some(2));
        assert_eq!(radio_step("up", 1, 3), Some(0));
        assert_eq!(radio_step("home", 2, 3), Some(0));
        assert_eq!(radio_step("end", 0, 3), Some(2));
        assert_eq!(radio_step("tab", 0, 3), None);
        assert_eq!(radio_step("right", 0, 0), None);
    }
}
