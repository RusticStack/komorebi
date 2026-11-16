//! Settings > Appearance (Komorebi 1.1.2): light, dark or as Windows, the
//! text size Windows sets, and the colour theme. Calm on the surface: the
//! presets first, each a card with a small live preview in its own
//! colours; the depth (every token, contrast checks, theme files) behind
//! one button, Customise colours (`theme_editor`).
//!
//! Every choice applies at once to every window through the one theme path
//! (`theme::set_appearance`: tokens, then Kit) and is saved under the
//! settings key `appearance`.

use gpui::{
    AnyElement, App, Context, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement, PathPromptOptions, Role, SharedString, StatefulInteractiveElement as _, Styled,
    Toggled, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi_i18n::tr;

use super::card::{
    CARD_MAX_W, Card, Segmented, group_head, note, page_head, radio_handles, radio_step,
};
use super::choices::THEMES;
use super::general::theme_label;
use super::theme_editor::{ThemeEditor, ThemeEditorEvent};
use super::{Section, SettingsView};
use crate::icons::Icon;
use crate::theme::appearance::{self, Appearance, CustomTheme, ImportError, ThemeRef};
use crate::theme::presets::{ColorSet, Preset, TokenId};
use crate::theme::settings::appearance as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::ring_style;

/// One choice of the theme row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Preset(Preset),
    Custom,
}

impl Choice {
    fn key(self) -> &'static str {
        match self {
            Choice::Preset(p) => p.key(),
            Choice::Custom => "custom",
        }
    }
}

/// The plain words for a refused theme file.
pub fn import_error_text(e: &ImportError) -> String {
    match e {
        ImportError::TooLarge => tr!("That file is too large to be a theme."),
        ImportError::NotJson | ImportError::NotATheme => {
            tr!("That file isn't a Komorebi theme.")
        }
        ImportError::NewerVersion(_) => {
            tr!("That theme was made by a newer Komorebi. Update Komorebi to use it.")
        }
        ImportError::Missing { variant, key } if *variant == "dark" => {
            tr!("That theme's dark colours have no {0}.", key)
        }
        ImportError::Missing { key, .. } => tr!("That theme's light colours have no {0}.", key),
        ImportError::BadColour { variant, key } if *variant == "dark" => {
            tr!("That theme's dark {0} isn't a colour like #0f6cbd.", key)
        }
        ImportError::BadColour { key, .. } => {
            tr!("That theme's light {0} isn't a colour like #0f6cbd.", key)
        }
    }
}

/// Windows' text size as a percentage ("125%").
fn percent(scale: f32) -> String {
    format!("{}%", (scale * 100.).round() as i32)
}

impl SettingsView {
    /// The appearance in use.
    pub fn appearance(&self, cx: &App) -> Appearance {
        crate::theme::appearance(cx)
    }

    /// Applies `a` to every window, then saves it.
    pub fn set_appearance(&mut self, a: Appearance, cx: &mut Context<Self>) -> bool {
        crate::theme::set_appearance(a.clone(), cx);
        self.save(appearance::KEY, &a.to_setting(), cx)
    }

    /// A choice of the theme row.
    pub fn choose(&mut self, choice: Choice, cx: &mut Context<Self>) {
        let mut a = self.appearance(cx);
        a.theme = match choice {
            Choice::Preset(p) => ThemeRef::Preset(p),
            Choice::Custom if a.custom.is_some() => ThemeRef::Custom,
            Choice::Custom => return,
        };
        self.set_appearance(a, cx);
    }

    /// Customise colours: the editor over the custom theme, made from the
    /// preset in use when there is none yet. The app switches to it (it
    /// looks the same until a colour changes).
    pub fn open_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut a = self.appearance(cx);
        if a.custom.is_none() {
            a.custom = Some(CustomTheme::from_preset(a.preset()));
        }
        a.theme = ThemeRef::Custom;
        self.set_appearance(a, cx);
        let db = self.db.clone();
        let editor = cx.new(|cx| ThemeEditor::new(db, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &editor,
            window,
            |this, _, e: &ThemeEditorEvent, window, cx| match e {
                ThemeEditorEvent::Back => {
                    this.close_theme_editor(cx);
                    window.focus(&this.focus, cx);
                }
                ThemeEditorEvent::Saved => this.reload(cx),
            },
        ));
        editor.update(cx, |e, cx| e.focus_first(window, cx));
        self.customising = Some(editor);
        self.section = Section::Appearance;
        self.message = None;
        self.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    /// Closes the colour editor (and ends its preview of the other
    /// variant).
    pub(crate) fn close_theme_editor(&mut self, cx: &mut Context<Self>) {
        if self.customising.take().is_some() {
            crate::theme::set_preview(None, cx);
            cx.notify();
        }
    }

    /// The colour editor, while it shows.
    pub fn theme_editor(&self) -> Option<&Entity<ThemeEditor>> {
        self.customising.as_ref()
    }

    /// Import theme: a theme file chosen in the file dialog.
    pub fn import_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let window_handle = window.window_handle();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!("Choose a theme file").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    let _ = this.update(cx, |this, cx| this.import_theme_from(&path, window, cx));
                });
            }
        })
        .detach();
    }

    /// The theme file at `path` as the custom theme (and the one in use),
    /// with its background settings if it has them (the picture chosen on
    /// this PC stays), or why not.
    pub fn import_theme_from(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let too_large =
            std::fs::metadata(path).is_ok_and(|m| m.len() > appearance::MAX_FILE as u64);
        let read = if too_large {
            Err(ImportError::TooLarge)
        } else {
            std::fs::read_to_string(path)
                .map_err(|_| ImportError::NotATheme)
                .and_then(|text| {
                    appearance::import(&text).map(|t| (t, appearance::import_background(&text)))
                })
        };
        match read {
            Ok((custom, background)) => {
                let mut a = self.appearance(cx);
                a.custom = Some(custom);
                a.theme = ThemeRef::Custom;
                self.theme_note = None;
                self.set_appearance(a.clone(), cx);
                if let Some(bg) = background {
                    let bg = crate::theme::backdrop::Background {
                        picture: a.background.picture,
                        ..bg
                    };
                    self.set_background(bg, true, window, cx);
                }
            }
            Err(e) => self.theme_note = Some(import_error_text(&e)),
        }
        cx.notify();
    }

    /// A theme card: a small window in the theme's own colours, its name and
    /// a radio mark.
    #[allow(clippy::too_many_arguments)] // a card's look, its place and its view
    fn theme_card(
        &self,
        choice: Choice,
        name: String,
        colours: ColorSet,
        on: bool,
        handle: &gpui::FocusHandle,
        (ix, count): (usize, usize),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let c = t.color;
        let k = |id: TokenId| -> Hsla { colours.get(id).colour() };
        let line = |w: f32, colour: Hsla| {
            div()
                .w(gpui::relative(w))
                .h(px(geo::LINE_H))
                .rounded(px(radius::FULL))
                .bg(colour)
        };
        let rail = div()
            .w(px(geo::RAIL_W))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(geo::PREVIEW_PAD))
            .child(
                div()
                    .w(px(geo::PILL_W))
                    .h(px(geo::PILL_H))
                    .rounded(px(radius::FULL))
                    .bg(k(TokenId::Accent)),
            )
            .child(line(geo::RAIL_LINE, k(TokenId::Ink2)))
            .child(line(geo::RAIL_LINE, k(TokenId::Ink3)));
        let selected_row = div()
            .relative()
            .w_full()
            .h(px(geo::ROW_H))
            .flex()
            .items_center()
            .gap(px(geo::PREVIEW_PAD))
            .px(px(geo::PREVIEW_PAD))
            .rounded(px(geo::ROW_RADIUS))
            .bg(k(TokenId::Wash))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .w(px(geo::MARKER_W))
                    .h(px(geo::MARKER_H))
                    .rounded(px(radius::FULL))
                    .bg(k(TokenId::Accent)),
            )
            .child(
                div()
                    .size(px(geo::DOT))
                    .flex_none()
                    .rounded(px(radius::FULL))
                    .bg(k(TokenId::Accent)),
            )
            .child(line(geo::MID, k(TokenId::Ink)));
        let pane = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(geo::PREVIEW_PAD))
            .p(px(geo::PREVIEW_PAD))
            .rounded(px(geo::PANE_RADIUS))
            .bg(k(TokenId::Paper))
            .child(line(geo::LONG, k(TokenId::Ink)))
            .child(selected_row)
            .child(line(geo::MID, k(TokenId::Ink2)))
            .child(line(geo::SHORT, k(TokenId::Ink3)));
        let preview = div()
            .w_full()
            .h(px(geo::PREVIEW_H))
            .flex()
            .gap(px(geo::PREVIEW_PAD))
            .p(px(geo::PREVIEW_PAD))
            .rounded(px(radius::CARD))
            .bg(k(TokenId::Canvas))
            .child(rail)
            .child(pane);
        let mark = div()
            .size(px(layout::ICON))
            .flex_none()
            .rounded(px(radius::FULL))
            .border(px(layout::FOCUS_RING))
            .border_color(if on { c.accent } else { c.ink_2 })
            .when(on, |el| el.bg(c.accent));
        div()
            .id(SharedString::from(format!(
                "appearance-theme-{}",
                choice.key()
            )))
            .track_focus(handle)
            .role(Role::RadioButton)
            .aria_label(name.clone())
            .aria_toggled(if on { Toggled::True } else { Toggled::False })
            .aria_position_in_set(ix + 1)
            .aria_size_of_set(count)
            .w(px(geo::CARD_W))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::SP_2))
            .p(px(space::SP_2))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .when(on, |el| el.bg(c.wash))
            .hover(move |s| s.bg(c.wash))
            .focus_visible(ring_style(&t))
            .cursor_pointer()
            .child(preview)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SP_2))
                    .px(px(space::SP_1))
                    .child(mark)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .typo(if on {
                                &t.text.body_strong
                            } else {
                                &t.text.body
                            })
                            .text_color(c.ink)
                            .child(name),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.choose(choice, cx)))
            .test_support()
    }

    pub(crate) fn render_appearance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let scheme = t.scheme;
        let a = self.appearance(cx);
        let s = self.s.clone();

        // Light, dark or as Windows (the mockup's segmented control).
        let mode = Card::new("settings-theme", tr!("Theme"))
            .icon(Icon::Color)
            .note(tr!("System follows your Windows app mode."))
            .control(
                Segmented::new("settings-theme-choice", tr!("Theme"), s.theme.key())
                    .choices(
                        THEMES
                            .iter()
                            .map(|t| (t.key().to_string(), theme_label(*t))),
                    )
                    .on_pick(cx.listener(|this, value: &SharedString, _, cx| {
                        if let Some(t) = THEMES.iter().find(|t| t.key() == value.as_ref()) {
                            this.set_theme(*t, cx);
                        }
                    })),
            );
        let text_size = Card::new("settings-text-size", tr!("Text size"))
            .icon(Icon::TextSize)
            .note(tr!(
                "Komorebi follows the text size in Windows Settings > Accessibility > Text size."
            ))
            .control(
                div()
                    .id("settings-text-size-value")
                    .role(Role::Label)
                    .aria_label(percent(t.text_scale))
                    .typo(&t.text.body)
                    .text_color(t.color.ink_2)
                    .child(percent(t.text_scale))
                    .test_support(),
            );

        // The theme row: the presets, then the custom theme when there is
        // one. One Tab stop; the arrows move the choice.
        let mut choices: Vec<(Choice, String, ColorSet)> = Preset::ALL
            .iter()
            .map(|p| (Choice::Preset(*p), p.name().to_string(), p.colors(scheme)))
            .collect();
        if let Some(custom) = &a.custom {
            choices.push((
                Choice::Custom,
                if custom.name.trim().is_empty() {
                    tr!("Custom")
                } else {
                    custom.name.clone()
                },
                custom.variant(scheme).colors,
            ));
        }
        let current = if a.is_custom() {
            Choice::Custom
        } else {
            Choice::Preset(a.preset())
        };
        let count = choices.len();
        let chosen = choices
            .iter()
            .position(|(ch, _, _)| *ch == current)
            .unwrap_or(0);
        let group_id: SharedString = "appearance-themes".into();
        let handles = radio_handles(&group_id, count, chosen, window, cx);
        let order: Vec<Choice> = choices.iter().map(|(ch, _, _)| *ch).collect();
        let cards: Vec<AnyElement> = choices
            .into_iter()
            .enumerate()
            .map(|(ix, (choice, name, colours))| {
                self.theme_card(
                    choice,
                    name,
                    colours,
                    choice == current,
                    &handles[ix],
                    (ix, count),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let view = cx.entity().downgrade();
        let key_handles = handles.clone();
        let row = div()
            .id(group_id)
            .role(Role::RadioGroup)
            .aria_label(tr!("Colour theme"))
            .w_full()
            .max_w(px(CARD_MAX_W))
            .flex()
            .flex_wrap()
            .gap(px(space::SP_2))
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
                let choice = order[to];
                let _ = view.update(cx, |this, cx| this.choose(choice, cx));
            })
            .children(cards)
            .test_support();

        let customise = Button::new("appearance-customise")
            .label(tr!("Customise colours"))
            .icon(Icon::Edit)
            .variant(Variant::Secondary)
            .on_click(cx.listener(|this, _, window, cx| this.open_theme_editor(window, cx)));
        let import = Button::new("appearance-import")
            .label(tr!("Import theme"))
            .icon(Icon::Open)
            .variant(Variant::Subtle)
            .on_click(cx.listener(|this, _, window, cx| this.import_theme(window, cx)));
        let actions = div()
            .w_full()
            .max_w(px(CARD_MAX_W))
            .mt(px(space::SP_3))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SP_2))
            .child(customise)
            .child(import);

        div()
            .flex()
            .flex_col()
            .child(page_head(
                Section::Appearance.name(),
                Some(tr!("How Komorebi looks.").into()),
                cx,
            ))
            .child(group_head(tr!("Mode and text"), cx))
            .child(mode)
            .child(text_size)
            .child(group_head(tr!("Colours"), cx))
            .child(row)
            .child(actions)
            .when_some(self.theme_note.clone(), |el, text| {
                el.child(
                    div()
                        .w_full()
                        .max_w(px(CARD_MAX_W))
                        .mt(px(space::SP_3))
                        .child(crate::widgets::notice::Notice::new(
                            "appearance-import-note",
                            crate::widgets::notice::Kind::Error,
                            text,
                        )),
                )
            })
            .child(div().mt(px(space::SP_3)).child(note(
                tr!("Presets follow light, dark or System above. Your own colours stay on this PC; export them from Customise colours to share them."),
                cx,
            )))
            .children(self.render_background(window, cx))
    }
}

use gpui::Entity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_import_error_has_words() {
        for e in [
            ImportError::TooLarge,
            ImportError::NotJson,
            ImportError::NotATheme,
            ImportError::NewerVersion(2),
            ImportError::Missing {
                variant: "dark",
                key: "ink".into(),
            },
            ImportError::BadColour {
                variant: "light",
                key: "accent".into(),
            },
        ] {
            assert!(!import_error_text(&e).is_empty());
        }
        assert_eq!(percent(1.25), "125%");
    }
}
