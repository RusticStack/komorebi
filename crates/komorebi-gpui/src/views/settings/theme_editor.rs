//! Settings > Appearance > Customise colours: the editor of the custom
//! theme. The main four first (accent, background, panes, text), every
//! other token under More colours; light and dark edited apart (the app
//! shows the variant being edited while the editor is open); each colour a
//! swatch with its hex value that opens Kit's picker (palette, hue,
//! saturation, lightness, a hex field). Every change shows at once on the
//! whole app and is saved.
//!
//! Legibility is guarded, never enforced: a pair below WCAG AA (4.5:1 for
//! text, 3:1 for the accent as a control and the focus ring) is a quiet
//! notice with Fix contrast, which moves the offending colour the least
//! that makes it pass (`theme::derive`).

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, Role, SharedString, Styled,
    Subscription, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState, ColorSelect};
use gpui_kit::component::input::{InputEvent, InputState};
use komorebi_i18n::tr;

use super::card::{CARD_MAX_W, Segmented, group_head, note, page_head};
use crate::icons::Icon;
use crate::theme::appearance::{self, Appearance, CustomTheme, ThemeRef};
use crate::theme::derive::{self, Pair};
use crate::theme::presets::{Preset, Rgb, TokenId};
use crate::theme::settings::appearance as geo;
use crate::theme::{ActiveTokens as _, Scheme, TextStyled as _, radius, space};
use crate::views::Db;
use crate::widgets::button::{Button, Variant};
use crate::widgets::field::TextField;
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::select::Dropdown;
use crate::widgets::toggle::Checkbox;

/// What the editor tells Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeEditorEvent {
    /// Back (or Esc): the Appearance page again.
    Back,
    /// A change was saved.
    Saved,
}

/// A token's name in the editor.
pub fn token_name(id: TokenId) -> String {
    match id {
        TokenId::Accent => tr!("Accent"),
        TokenId::Canvas => tr!("Background"),
        TokenId::Paper => tr!("Panes"),
        TokenId::Ink => tr!("Text"),
        TokenId::OnAccent => tr!("Text on the accent"),
        TokenId::Ring => tr!("Focus ring"),
        TokenId::RingGap => tr!("Focus ring gap"),
        TokenId::Ink2 => tr!("Secondary text and icons"),
        TokenId::Ink3 => tr!("Muted text"),
        TokenId::OnInk => tr!("Text on tooltips"),
        TokenId::Inset => tr!("Cards"),
        TokenId::Wash => tr!("Selection and hover"),
        TokenId::Line => tr!("Lines"),
        TokenId::Spot => tr!("Disabled marks"),
        TokenId::Fall => tr!("Welcome shape"),
        TokenId::SegmentOn => tr!("Chosen segment"),
        TokenId::PillOnInset => tr!("Pills on cards"),
    }
}

/// A failing pair in words ("Muted text on the panes is 3.2:1; text needs
/// 4.5:1.").
pub fn issue_text(issue: &derive::Issue) -> String {
    let ratio = format!("{:.1}", issue.ratio);
    match issue.pair {
        Pair::TextOnPanes => tr!("Text on the panes is {0}:1; text needs 4.5:1.", ratio),
        Pair::TextOnCanvas => tr!("Text on the background is {0}:1; text needs 4.5:1.", ratio),
        Pair::TextOnWash => tr!("Text on a selected row is {0}:1; text needs 4.5:1.", ratio),
        Pair::SecondaryOnPanes => {
            tr!(
                "Secondary text on the panes is {0}:1; text needs 4.5:1.",
                ratio
            )
        }
        Pair::MutedOnPanes => tr!("Muted text on the panes is {0}:1; text needs 4.5:1.", ratio),
        Pair::MutedOnCanvas => {
            tr!(
                "Muted text on the background is {0}:1; text needs 4.5:1.",
                ratio
            )
        }
        Pair::OnAccent => tr!("Text on the accent is {0}:1; text needs 4.5:1.", ratio),
        Pair::AccentOnPanes => {
            tr!(
                "The accent on the panes is {0}:1; controls need 3:1.",
                ratio
            )
        }
        Pair::Ring => tr!("The focus ring on its gap is {0}:1; it needs 3:1.", ratio),
        Pair::OnInk => tr!("Text on tooltips is {0}:1; text needs 4.5:1.", ratio),
    }
}

/// The custom theme's editor.
pub struct ThemeEditor {
    db: Db,
    /// The variant being edited (and shown).
    scheme: Scheme,
    pickers: Vec<(TokenId, Entity<ColorPickerState>)>,
    name: Entity<InputState>,
    /// More colours is open.
    more: bool,
    /// The contrast warning's ratios are shown.
    details: bool,
    /// Export theme also writes the background settings (never the
    /// picture).
    with_background: bool,
    /// Why the last save or export failed, or where the export went.
    line: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ThemeEditorEvent> for ThemeEditor {}

impl Focusable for ThemeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ThemeEditor {
    /// The editor over the custom theme in use (Settings made it).
    pub fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let a = crate::theme::appearance(cx);
        let scheme = cx.tokens().scheme;
        let custom = a
            .custom
            .clone()
            .unwrap_or_else(|| CustomTheme::from_preset(a.preset()));
        let colours = custom.variant(scheme).colors;
        let mut subs = Vec::new();
        let pickers: Vec<(TokenId, Entity<ColorPickerState>)> = TokenId::ALL
            .into_iter()
            .map(|id| {
                let state = cx.new(|cx| {
                    ColorPickerState::new(window, cx).default_value(colours.get(id).colour())
                });
                subs.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, e: &ColorPickerEvent, window, cx| {
                        let ColorPickerEvent::Change(Some(c)) = e else {
                            return;
                        };
                        let value = Rgb::from_colour(*c);
                        this.edit(window, cx, |t, scheme| t.variant_mut(scheme).set(id, value));
                    },
                ));
                (id, state)
            })
            .collect();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(custom.name.clone()));
        subs.push(
            cx.subscribe_in(&name, window, |this, input, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.edit(window, cx, |t, _| t.name = value.trim().to_string());
                }
            }),
        );
        Self {
            db,
            scheme,
            pickers,
            name,
            more: false,
            details: false,
            with_background: false,
            line: None,
            focus: cx.focus_handle(),
            _subscriptions: subs,
        }
    }

    /// The first stop: the name field.
    pub fn focus_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.name.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// The picker of one colour.
    pub fn picker(&self, id: TokenId) -> Option<&Entity<ColorPickerState>> {
        self.pickers.iter().find(|(i, _)| *i == id).map(|(_, s)| s)
    }

    /// The variant being edited.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The custom theme as it is now.
    pub fn theme(&self, cx: &App) -> CustomTheme {
        let a = crate::theme::appearance(cx);
        let base = a.preset();
        a.custom.unwrap_or_else(|| CustomTheme::from_preset(base))
    }

    /// Changes the custom theme, shows it on the whole app and saves it.
    pub fn edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CustomTheme, Scheme),
    ) {
        let mut a: Appearance = crate::theme::appearance(cx);
        let mut custom = a
            .custom
            .take()
            .unwrap_or_else(|| CustomTheme::from_preset(a.preset()));
        f(&mut custom, self.scheme);
        a.custom = Some(custom);
        a.theme = ThemeRef::Custom;
        crate::theme::set_appearance(a.clone(), cx);
        self.line = match self.db.set_setting(appearance::KEY, &a.to_setting()) {
            Ok(()) => None,
            Err(e) => Some(super::text::not_saved(&format!("{e:#}"))),
        };
        self.sync(window, cx);
        cx.emit(ThemeEditorEvent::Saved);
        cx.notify();
    }

    /// The pickers follow the theme (after a fix, a reset or a derived
    /// change); setting a picker's value sends no change back.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let colours = self.theme(cx).variant(self.scheme).colors;
        for (id, state) in &self.pickers {
            let want = colours.get(*id);
            let shown = state.read(cx).value().map(Rgb::from_colour);
            if shown != Some(want) {
                state.update(cx, |s, cx| s.set_value(want.colour(), window, cx));
            }
        }
    }

    /// Light or dark: which variant is edited (and shown on the app).
    pub fn set_scheme(&mut self, scheme: Scheme, window: &mut Window, cx: &mut Context<Self>) {
        self.scheme = scheme;
        crate::theme::set_preview(Some(scheme), cx);
        self.sync(window, cx);
        cx.notify();
    }

    /// Sets one colour of the variant edited.
    pub fn set(&mut self, id: TokenId, value: Rgb, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(window, cx, |t, scheme| t.variant_mut(scheme).set(id, value));
    }

    /// Fix contrast: every failing pair of the variant edited.
    pub fn fix_contrast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(window, cx, |t, scheme| {
            let v = t.variant_mut(scheme);
            let (fixed, moved) = derive::fix(&v.colors);
            v.colors = fixed;
            for id in moved {
                if !id.is_main() && !v.edited.contains(&id) {
                    v.edited.push(id);
                }
            }
        });
        // The button goes with the warning: focus stays in the editor.
        window.focus(&self.focus, cx);
    }

    /// Reset to preset: the variant edited back to its base preset.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(window, cx, |t, scheme| t.reset(scheme));
    }

    /// Start from another preset: both variants become its colours.
    pub fn start_from(&mut self, base: Preset, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(window, cx, |t, _| {
            let name = t.name.clone();
            *t = CustomTheme {
                name,
                ..CustomTheme::from_preset(base)
            };
        });
    }

    /// Export theme: a theme file where the user says.
    pub fn export(&mut self, cx: &mut Context<Self>) {
        let theme = self.theme(cx);
        let background = crate::theme::appearance(cx).background;
        let file = appearance::export_with(&theme, self.with_background.then_some(&background));
        let stem: String = theme
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let stem = if stem.trim_matches('-').is_empty() {
            "theme".to_string()
        } else {
            stem
        };
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let answer = cx.prompt_for_new_path(&dir, Some(&format!("{stem}.komorebi-theme.json")));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = answer.await {
                let result = std::fs::write(&path, file);
                let _ = this.update(cx, |this, cx| {
                    this.line = Some(match result {
                        Ok(()) => tr!("Saved as {0}.", path.display()),
                        Err(e) => tr!("The theme was not saved: {0}", e),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn colour_row(
        &self,
        id: TokenId,
        low: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = cx.tokens().clone();
        let hex = self
            .theme(cx)
            .variant(self.scheme)
            .colors
            .get(id)
            .hex()
            .to_uppercase();
        let state = self
            .pickers
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, s)| s.clone());
        let name = token_name(id);
        div()
            .id(SharedString::from(format!("theme-colour-{}", id.key())))
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SP_3))
            .py(px(space::SP_1))
            .child(
                div()
                    .w(px(geo::LABEL_W))
                    .flex_none()
                    .typo(&t.text.body)
                    .text_color(t.color.ink)
                    .child(name.clone()),
            )
            .children(state.map(|s| {
                // The name, the value shown and, when it fails, why: what a
                // screen reader says for the swatch field.
                let spoken = if low {
                    tr!("colour" => "{0}, {1}, low contrast", name, hex)
                } else {
                    tr!("colour" => "{0}, {1}", name, hex)
                };
                div()
                    .w(px(geo::COLOUR_W))
                    .child(ColorSelect::new(&s).accessibility_label(spoken))
            }))
            .when(low, |el| {
                el.child(super::card::card_icon(Icon::Warning, &t).text_color(t.color.ink))
            })
    }

    /// The colours a failing pair would move (the ones drawn on top).
    fn offending(&self, cx: &App) -> Vec<TokenId> {
        let colours = self.theme(cx).variant(self.scheme).colors;
        derive::check(&colours)
            .into_iter()
            .map(|i| i.pair.tokens().0)
            .collect()
    }

    /// The contrast warning: one calm line and Fix contrast; the ratios,
    /// one pair a line, behind Details.
    fn render_issues(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = cx.tokens().clone();
        let colours = self.theme(cx).variant(self.scheme).colors;
        let issues = derive::check(&colours);
        if issues.is_empty() {
            return None;
        }
        let details = self.details;
        let lines: Vec<_> = issues
            .iter()
            .map(|i| {
                div()
                    .typo(&t.text.caption)
                    .text_color(t.color.ink_2)
                    .child(issue_text(i))
            })
            .collect();
        Some(
            div()
                .w_full()
                .max_w(px(CARD_MAX_W))
                .mt(px(space::SP_3))
                .flex()
                .flex_col()
                .gap(px(space::SP_2))
                .child(
                    Notice::new(
                        "theme-contrast",
                        Kind::Warning,
                        tr!("Some text may be hard to read with these colours."),
                    )
                    .action(
                        Button::new("theme-fix-contrast")
                            .label(tr!("Fix contrast"))
                            .variant(Variant::OnInset)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fix_contrast(window, cx)),
                            ),
                    )
                    .action(
                        Button::new("theme-contrast-details")
                            .label(if details {
                                tr!("Hide details")
                            } else {
                                tr!("Details")
                            })
                            .trailing(if details {
                                Icon::ChevronUp
                            } else {
                                Icon::ChevronDown
                            })
                            .variant(Variant::Subtle)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.details = !this.details;
                                cx.notify();
                            })),
                    ),
                )
                .when(details, |el| {
                    el.child(
                        div()
                            .id("theme-contrast-details-list")
                            .role(Role::List)
                            .aria_label(tr!("Contrast details"))
                            .px(px(space::SP_4))
                            .flex()
                            .flex_col()
                            .gap(px(space::SP_1))
                            .children(lines)
                            .test_support(),
                    )
                }),
        )
    }
}

impl Render for ThemeEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let theme = self.theme(cx);
        let back = Button::new("theme-back")
            .label(tr!("Back"))
            .icon(Icon::ChevronLeft)
            .variant(Variant::Subtle)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ThemeEditorEvent::Back)));
        let variant = Segmented::new(
            "theme-variant",
            tr!("Colours for"),
            if self.scheme.is_dark() {
                "dark"
            } else {
                "light"
            },
        )
        .choices([
            ("light".to_string(), tr!("Light")),
            ("dark".to_string(), tr!("Dark")),
        ])
        .on_pick(cx.listener(|this, value: &SharedString, window, cx| {
            let scheme = if value.as_ref() == "dark" {
                Scheme::Dark
            } else {
                Scheme::Light
            };
            this.set_scheme(scheme, window, cx);
        }));
        let start = Dropdown::new("theme-base", tr!("Start from"), theme.base.key())
            .choices(Preset::ALL.iter().map(|p| (p.key(), p.name())))
            .on_pick(cx.listener(|this, value: &SharedString, window, cx| {
                if let Some(p) = Preset::from_key(value) {
                    this.start_from(p, window, cx);
                }
            }));
        let head_row = div()
            .w_full()
            .max_w(px(CARD_MAX_W))
            .mt(px(space::SP_4))
            .flex()
            .flex_wrap()
            .items_start()
            .gap(px(space::SP_4))
            .child(super::card::labelled(
                tr!("Name"),
                div()
                    .w(px(geo::NAME_W))
                    .child(TextField::new(&self.name, tr!("Name"))),
                None,
                cx,
            ))
            .child(super::card::labelled(tr!("Colours for"), variant, None, cx))
            .child(super::card::labelled(tr!("Start from"), start, None, cx));
        let low = self.offending(cx);
        let main: Vec<_> = TokenId::MAIN
            .into_iter()
            .map(|id| {
                self.colour_row(id, low.contains(&id), cx)
                    .into_any_element()
            })
            .collect();
        let rest: Vec<_> = if self.more {
            TokenId::ALL
                .into_iter()
                .filter(|id| !id.is_main())
                .map(|id| {
                    self.colour_row(id, low.contains(&id), cx)
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        let more = Button::new("theme-more")
            .label(tr!("More colours"))
            .trailing(if self.more {
                Icon::ChevronUp
            } else {
                Icon::ChevronDown
            })
            .variant(Variant::Subtle)
            .on_click(cx.listener(|this, _, _, cx| {
                this.more = !this.more;
                cx.notify();
            }));
        let footer = div()
            .w_full()
            .max_w(px(CARD_MAX_W))
            .mt(px(space::SP_5))
            .flex()
            .flex_wrap()
            .gap(px(space::SP_2))
            .child(
                Button::new("theme-reset")
                    .label(tr!("Reset to preset"))
                    .icon(Icon::Undo)
                    .variant(Variant::Secondary)
                    .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
            )
            .child(
                Button::new("theme-export")
                    .label(tr!("Export theme"))
                    .icon(Icon::Save)
                    .variant(Variant::Subtle)
                    .on_click(cx.listener(|this, _, _, cx| this.export(cx))),
            )
            .child({
                let view = cx.entity().downgrade();
                div().flex().items_center().child(
                    Checkbox::new(
                        "theme-export-background",
                        tr!("Include background settings"),
                        self.with_background,
                    )
                    .on_change(move |on, _, cx| {
                        let _ = view.update(cx, |this, cx| {
                            this.with_background = on;
                            cx.notify();
                        });
                    }),
                )
            });
        div()
            .id("theme-editor")
            .track_focus(&self.focus)
            .role(Role::Group)
            .aria_label(tr!("Customise colours"))
            .flex()
            .flex_col()
            .child(div().mb(px(space::SP_2)).flex().child(back))
            .child(page_head(
                tr!("Customise colours"),
                Some(
                    tr!(
                        "Changes show at once and are kept. Light and dark have their own colours."
                    )
                    .into(),
                ),
                cx,
            ))
            .child(head_row)
            .children(self.render_issues(cx))
            .child(group_head(tr!("Main colours"), cx))
            .child(
                div()
                    .w_full()
                    .max_w(px(CARD_MAX_W))
                    .p(px(space::SP_4))
                    .rounded(px(radius::CARD))
                    .bg(c.inset)
                    .flex()
                    .flex_col()
                    .children(main),
            )
            .child(div().mt(px(space::SP_3)).flex().child(more))
            .when(self.more, |el| {
                el.child(
                    div()
                        .w_full()
                        .max_w(px(CARD_MAX_W))
                        .mt(px(space::SP_2))
                        .p(px(space::SP_4))
                        .rounded(px(radius::CARD))
                        .bg(c.inset)
                        .flex()
                        .flex_col()
                        .child(note(
                            tr!("These follow the main colours until you change one."),
                            cx,
                        ))
                        .children(rest),
                )
            })
            .child(footer)
            .when_some(self.line.clone(), |el, line| {
                el.child(
                    div()
                        .mt(px(space::SP_3))
                        .typo(&t.text.caption)
                        .text_color(c.ink_2)
                        .child(line),
                )
            })
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_and_pair_has_words() {
        for id in TokenId::ALL {
            assert!(!token_name(id).is_empty());
        }
        for pair in Pair::ALL {
            let text = issue_text(&derive::Issue { pair, ratio: 2.04 });
            assert!(text.contains("2.0"), "{text}");
        }
    }
}
