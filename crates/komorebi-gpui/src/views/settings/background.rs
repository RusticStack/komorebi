//! Settings > Appearance > Background (Komorebi 1.1.2): a picture behind
//! the panes (fit, dim, blur, Blur behind panes), how see-through the panes
//! and the sidebar are, and Windows' window material. Everything is off
//! until a picture or a material is chosen; the opacity sliders show only
//! then, since an opaque window has nothing to show through.
//!
//! The pixels are `crate::backdrop`'s, made off the UI thread; a slider
//! applies as it moves (the picture is made again once it rests) and is
//! saved when it is let go.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement, PathPromptOptions,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use komorebi_i18n::tr;

use super::SettingsView;
use super::card::{CARD_MAX_W, Card, Segmented, group_head};
use crate::backdrop::{self, pixels};
use crate::icons::Icon;
use crate::theme::appearance::{self, Appearance};
use crate::theme::backdrop::{
    AsciiInk, Background, Fit, MAX_BLUR, MAX_CELL, MAX_DIM, MIN_CELL, MIN_OPACITY, Material,
    PALETTES, Style, safe_opacity,
};
use crate::theme::presets::Rgb;
use crate::theme::settings::appearance as geo;
use crate::theme::{ActiveTokens as _, TextStyled as _, space};
use crate::widgets::button::{Button, Variant};
use crate::widgets::notice::{Kind, Notice};
use crate::widgets::toggle::Toggle;

/// One background slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knob {
    Dim,
    Blur,
    Cell,
    Vignette,
    VignetteSize,
    /// ASCII: how strongly the characters show.
    Intensity,
    PaneBlur,
    /// Panes and sidebar together.
    Surfaces,
    Panes,
    Sidebar,
    Tint,
}

impl Knob {
    const ALL: [Knob; 11] = [
        Knob::Dim,
        Knob::Blur,
        Knob::Cell,
        Knob::Vignette,
        Knob::VignetteSize,
        Knob::Intensity,
        Knob::PaneBlur,
        Knob::Surfaces,
        Knob::Panes,
        Knob::Sidebar,
        Knob::Tint,
    ];

    fn range(self) -> (u8, u8) {
        match self {
            Knob::Dim => (0, MAX_DIM),
            Knob::Blur | Knob::PaneBlur => (0, MAX_BLUR),
            Knob::Cell => (MIN_CELL, MAX_CELL),
            Knob::Vignette | Knob::VignetteSize | Knob::Intensity => (0, 100),
            _ => (MIN_OPACITY, 100),
        }
    }

    fn read(self, bg: &Background) -> u8 {
        match self {
            Knob::Dim => bg.dim,
            Knob::Blur => bg.blur,
            Knob::Cell => bg.cell,
            Knob::Vignette => bg.vignette,
            Knob::VignetteSize => bg.vignette_size,
            Knob::Intensity => bg.intensity,
            Knob::PaneBlur => bg.pane_blur,
            Knob::Surfaces | Knob::Panes => bg.panes,
            Knob::Sidebar => bg.sidebar,
            Knob::Tint => bg.tint,
        }
    }

    fn write(self, bg: &mut Background, v: u8) {
        match self {
            Knob::Dim => bg.dim = v,
            Knob::Blur => bg.blur = v,
            Knob::Cell => bg.cell = v,
            Knob::Vignette => bg.vignette = v,
            Knob::VignetteSize => bg.vignette_size = v,
            Knob::Intensity => bg.intensity = v,
            Knob::PaneBlur => bg.pane_blur = v,
            Knob::Surfaces => {
                bg.panes = v;
                bg.sidebar = v;
            }
            Knob::Panes => bg.panes = v,
            Knob::Sidebar => bg.sidebar = v,
            Knob::Tint => bg.tint = v,
        }
    }

    fn name(self) -> String {
        match self {
            Knob::Dim => tr!("Dim"),
            Knob::Blur => tr!("Blur image"),
            Knob::Cell => tr!("Cell size"),
            Knob::Vignette => tr!("Vignette"),
            Knob::VignetteSize => tr!("Vignette size"),
            Knob::Intensity => tr!("Intensity"),
            Knob::PaneBlur => tr!("Blur behind panes"),
            Knob::Surfaces => tr!("Opacity"),
            Knob::Panes => tr!("Panes"),
            Knob::Sidebar => tr!("Sidebar"),
            Knob::Tint => tr!("Tint"),
        }
    }

    fn value_text(self, v: u8) -> String {
        match self {
            Knob::Blur | Knob::PaneBlur | Knob::Cell => tr!("{0} px", v),
            _ => format!("{v}%"),
        }
    }
}

/// The page's sliders, made the first time the page shows.
pub struct BackgroundUi {
    sliders: Vec<(Knob, Entity<SliderState>)>,
    /// More: the panes and the sidebar apart.
    pub(crate) more: bool,
}

fn fit_label(f: Fit) -> String {
    match f {
        Fit::Fill => tr!("Fill"),
        Fit::Fit => tr!("picture fit" => "Fit"),
        Fit::Tile => tr!("Tile"),
        Fit::Centre => tr!("Centre"),
    }
}

fn style_label(s: Style) -> String {
    match s {
        Style::None => tr!("picture style" => "None"),
        Style::PixelArt => tr!("Pixel art"),
        Style::Ascii => tr!("ASCII"),
    }
}

fn ink_label(i: AsciiInk) -> String {
    match i {
        AsciiInk::Picture => tr!("ASCII colour" => "Picture"),
        AsciiInk::Text => tr!("ASCII colour" => "Text"),
        AsciiInk::Accent => tr!("ASCII colour" => "Accent"),
    }
}

fn material_label(m: Material) -> String {
    match m {
        Material::None => tr!("material" => "None"),
        Material::Mica => tr!("Mica"),
        Material::Acrylic => tr!("Acrylic"),
    }
}

impl SettingsView {
    /// Applies new background settings (tokens, material, pictures) and,
    /// when `save`, keeps them.
    pub fn set_background(
        &mut self,
        bg: Background,
        save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut a = self.appearance(cx);
        let prev = std::mem::replace(&mut a.background, bg.clamped());
        crate::theme::set_appearance(a.clone(), cx);
        self.apply_backdrop(&prev, &a.background, window, cx);
        if save {
            self.save(appearance::KEY, &a.to_setting(), cx);
        }
        self.sync_sliders(&a.background, window, cx);
        cx.notify();
    }

    /// The material and the pictures follow `next`.
    fn apply_backdrop(
        &self,
        prev: &Background,
        next: &Background,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if prev.material != next.material {
            backdrop::sync_material(next, cx);
        }
        let source = self.db.file().map(|f| backdrop::picture(&f));
        let debounce = prev.picture == next.picture;
        backdrop::want(source, next, debounce, window, cx);
    }

    /// Choose picture: a PNG, JPEG or WebP file, scaled down to the
    /// largest monitor and kept in the app's data folder, off the UI
    /// thread.
    pub fn choose_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr!("Choose a picture").into()),
        });
        let cap = backdrop::monitor_cap(Some(&*window), cx);
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| this.take_picture(path, cap, window, cx));
            });
        })
        .detach();
    }

    /// Takes the picture at `path` (decoded and stored off the UI thread).
    pub fn take_picture(
        &mut self,
        path: std::path::PathBuf,
        cap: (u32, u32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(db_file) = self.db.file() else {
            return;
        };
        let target = backdrop::picture(&db_file);
        let window_handle = window.window_handle();
        let stored = cx.background_executor().spawn(async move {
            let img = pixels::load(&path, cap)?;
            pixels::store(&img, &target)
        });
        cx.spawn(async move |this, cx| {
            let result = stored.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| match result {
                    Ok(()) => {
                        this.theme_note = None;
                        backdrop::invalidate(cx);
                        let mut bg = this.appearance(cx).background;
                        bg.picture = true;
                        this.set_background(bg, true, window, cx);
                    }
                    Err(e) => {
                        this.theme_note = Some(match e {
                            pixels::Refusal::TooLarge => tr!("That picture is too large."),
                            pixels::Refusal::Unreadable => {
                                tr!("That file isn't a PNG, JPEG or WebP picture.")
                            }
                        });
                        cx.notify();
                    }
                });
            });
        })
        .detach();
    }

    /// Remove picture: the picture is deleted from the data folder.
    pub fn remove_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.db.file() {
            std::fs::remove_file(backdrop::picture(&f)).ok();
        }
        let mut bg = self.appearance(cx).background;
        bg.picture = false;
        self.set_background(bg, true, window, cx);
    }

    /// Reset appearance: the default look, with no picture or material.
    pub fn reset_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.db.file() {
            std::fs::remove_file(backdrop::picture(&f)).ok();
        }
        let prev = self.appearance(cx).background;
        let a = Appearance::default();
        crate::theme::set_appearance(a.clone(), cx);
        self.apply_backdrop(&prev, &a.background, window, cx);
        self.save(appearance::KEY, &a.to_setting(), cx);
        self.theme_note = None;
        self.sync_sliders(&a.background, window, cx);
        cx.notify();
    }

    /// The sliders, made once.
    fn background_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.background_ui.is_some() {
            return;
        }
        let bg = self.appearance(cx).background;
        let mut sliders = Vec::new();
        for knob in Knob::ALL {
            let (lo, hi) = knob.range();
            let state = cx.new(|_| {
                SliderState::new()
                    .min(f32::from(lo))
                    .max(f32::from(hi))
                    .step(1.)
                    .default_value(f32::from(knob.read(&bg)))
            });
            self._subscriptions.push(cx.subscribe_in(
                &state,
                window,
                move |this, _, e: &SliderEvent, window, cx| {
                    let (value, save) = match e {
                        SliderEvent::Change(v) => (v.start(), false),
                        SliderEvent::Release(v) => (v.start(), true),
                    };
                    let v = (value.round().clamp(f32::from(lo), f32::from(hi))) as u8;
                    let mut bg = this.appearance(cx).background;
                    if knob.read(&bg) == v && !save {
                        return;
                    }
                    knob.write(&mut bg, v);
                    this.set_background(bg, save, window, cx);
                },
            ));
            sliders.push((knob, state));
        }
        self.background_ui = Some(BackgroundUi {
            sliders,
            more: false,
        });
    }

    /// The sliders follow the settings (after a reset or a file).
    fn sync_sliders(&mut self, bg: &Background, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ui) = &self.background_ui else {
            return;
        };
        for (knob, state) in ui.sliders.clone() {
            let want = f32::from(knob.read(bg));
            if (state.read(cx).value().start() - want).abs() >= 0.5 {
                state.update(cx, |s, cx| s.set_value(want, window, cx));
            }
        }
    }

    fn slider_card(&self, knob: Knob, note: Option<String>, cx: &Context<Self>) -> Option<Card> {
        let t = cx.tokens();
        let ui = self.background_ui.as_ref()?;
        let state = ui.sliders.iter().find(|(k, _)| *k == knob)?.1.clone();
        let value = state.read(cx).value().start().round() as u8;
        let name = knob.name();
        let card = Card::new(
            SharedString::from(format!("appearance-slider-{knob:?}")),
            name.clone(),
        )
        .control(
            div()
                .flex()
                .items_center()
                .gap(px(space::SP_3))
                .child(
                    div()
                        .w(px(geo::SLIDER_W))
                        .child(Slider::new(&state).accessibility_label(name)),
                )
                .child(
                    div()
                        .w(px(geo::VALUE_W))
                        .typo(&t.text.body)
                        .text_color(t.color.ink_2)
                        .child(knob.value_text(value)),
                ),
        );
        Some(match note {
            Some(n) => card.note(n),
            None => card,
        })
    }

    /// Style: the choice, then only the chosen style's controls.
    fn style_cards(&self, bg: &Background, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let pick = Card::new("appearance-style", tr!("Style"))
            .note(tr!(
                "Redraws the picture once, in cells that line up with the screen's pixels."
            ))
            .control(
                Segmented::new("appearance-style-choice", tr!("Style"), bg.style.key())
                    .choices(
                        Style::ALL
                            .iter()
                            .map(|s| (s.key().to_string(), style_label(*s))),
                    )
                    .on_pick(cx.listener(|this, v: &SharedString, window, cx| {
                        if let Some(s) = Style::from_key(v) {
                            let mut bg = this.appearance(cx).background;
                            // Blur behind panes would hide a style: it
                            // starts at 0 with one (and can be raised).
                            if bg.style == Style::None && s != Style::None {
                                bg.pane_blur = 0;
                            }
                            bg.style = s;
                            this.set_background(bg, true, window, cx);
                        }
                    })),
            );
        out.push(pick.into_any_element());
        if bg.style == Style::None {
            return out;
        }
        out.extend(
            self.slider_card(Knob::Cell, None, cx)
                .map(IntoElement::into_any_element),
        );
        match bg.style {
            Style::PixelArt => {
                let colours = Card::new("appearance-colours", tr!("Colours")).control(
                    Segmented::new(
                        "appearance-colours-choice",
                        tr!("Colours"),
                        bg.colours.to_string(),
                    )
                    .choices(PALETTES.iter().map(|n| (n.to_string(), n.to_string())))
                    .on_pick(cx.listener(
                        |this, v: &SharedString, window, cx| {
                            if let Ok(n) = v.parse::<u8>() {
                                let mut bg = this.appearance(cx).background;
                                bg.colours = n;
                                this.set_background(bg, true, window, cx);
                            }
                        },
                    )),
                );
                out.push(colours.into_any_element());
                let view = cx.entity().downgrade();
                let dither = Card::new("appearance-dither", tr!("Dithering"))
                    .note(tr!(
                        "Mixes neighbouring colours in a fixed pattern, as old games did."
                    ))
                    .control(
                        Toggle::new("appearance-dither", tr!("Dithering"), bg.dither)
                            .state_label(tr!("On"), tr!("Off"))
                            .on_change(move |on, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    let mut bg = this.appearance(cx).background;
                                    bg.dither = on;
                                    this.set_background(bg, true, window, cx);
                                });
                            }),
                    );
                out.push(dither.into_any_element());
            }
            Style::Ascii => {
                let ink = Card::new("appearance-ascii-ink", tr!("Character colour")).control(
                    Segmented::new(
                        "appearance-ascii-ink-choice",
                        tr!("Character colour"),
                        bg.ascii_ink.key(),
                    )
                    .choices(
                        AsciiInk::ALL
                            .iter()
                            .map(|i| (i.key().to_string(), ink_label(*i))),
                    )
                    .on_pick(cx.listener(
                        |this, v: &SharedString, window, cx| {
                            if let Some(i) = AsciiInk::from_key(v) {
                                let mut bg = this.appearance(cx).background;
                                bg.ascii_ink = i;
                                this.set_background(bg, true, window, cx);
                            }
                        },
                    )),
                );
                out.push(ink.into_any_element());
                out.extend(
                    self.slider_card(
                        Knob::Intensity,
                        Some(tr!(
                            "Bolder characters with a little of their colour behind them, so the picture reads through the panes."
                        )),
                        cx,
                    )
                    .map(IntoElement::into_any_element),
                );
            }
            Style::None => {}
        }
        out
    }

    pub(crate) fn render_background(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.background_ui(window, cx);
        let t = cx.tokens().clone();
        let bg = self.appearance(cx).background;
        let mut out: Vec<AnyElement> = vec![group_head(tr!("Background"), cx)];

        let picture = Card::new("appearance-picture", tr!("Picture"))
            .icon(Icon::Image)
            .note(tr!(
                "PNG, JPEG or WebP. Kept on this PC, scaled to your largest screen."
            ))
            .control(
                div()
                    .flex()
                    .gap(px(space::SP_2))
                    .child(
                        Button::new("appearance-choose-picture")
                            .label(if bg.picture {
                                tr!("Change picture")
                            } else {
                                tr!("Choose picture")
                            })
                            .variant(Variant::OnInset)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_picture(window, cx)),
                            ),
                    )
                    .when(bg.picture, |el| {
                        el.child(
                            Button::new("appearance-remove-picture")
                                .label(tr!("Remove picture"))
                                .variant(Variant::Subtle)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_picture(window, cx)
                                })),
                        )
                    }),
            );
        out.push(picture.into_any_element());

        if bg.picture {
            let fit = Card::new("appearance-fit", tr!("Fit")).control(
                Segmented::new("appearance-fit-choice", tr!("Fit"), bg.fit.key())
                    .choices(
                        Fit::ALL
                            .iter()
                            .map(|f| (f.key().to_string(), fit_label(*f))),
                    )
                    .on_pick(cx.listener(|this, v: &SharedString, window, cx| {
                        if let Some(f) = Fit::from_key(v) {
                            let mut bg = this.appearance(cx).background;
                            bg.fit = f;
                            this.set_background(bg, true, window, cx);
                        }
                    })),
            );
            out.push(fit.into_any_element());
            out.extend(self.style_cards(&bg, cx));
            for knob in [Knob::Blur, Knob::Dim, Knob::Vignette] {
                out.extend(
                    self.slider_card(knob, None, cx)
                        .map(IntoElement::into_any_element),
                );
            }
            if bg.vignette > 0 {
                out.extend(
                    self.slider_card(
                        Knob::VignetteSize,
                        Some(tr!("How far in from the window's edges the fade starts.")),
                        cx,
                    )
                    .map(IntoElement::into_any_element),
                );
            }
        }

        let materials: Vec<Material> = Material::ALL
            .into_iter()
            .filter(|m| *m != Material::Mica || backdrop::mica_available())
            .collect();
        let material = Card::new("appearance-material", tr!("Window material"))
            .note(tr!(
                "Mica tints the window with your desktop; Acrylic blurs what is behind it."
            ))
            .control(
                Segmented::new(
                    "appearance-material-choice",
                    tr!("Window material"),
                    bg.material.key(),
                )
                .choices(
                    materials
                        .iter()
                        .map(|m| (m.key().to_string(), material_label(*m))),
                )
                .on_pick(cx.listener(|this, v: &SharedString, window, cx| {
                    if let Some(m) = Material::from_key(v) {
                        let mut bg = this.appearance(cx).background;
                        bg.material = m;
                        this.set_background(bg, true, window, cx);
                    }
                })),
            );
        out.push(material.into_any_element());
        if bg.material != Material::None && !bg.picture {
            out.extend(
                self.slider_card(
                    Knob::Tint,
                    Some(tr!(
                        "How much of the background colour covers the material."
                    )),
                    cx,
                )
                .map(IntoElement::into_any_element),
            );
        }

        if bg.is_on() {
            out.push(group_head(tr!("Panes"), cx));
            let more = self.background_ui.as_ref().is_some_and(|u| u.more);
            if more {
                out.extend(
                    self.slider_card(Knob::Panes, None, cx)
                        .map(IntoElement::into_any_element),
                );
                if bg.picture {
                    out.extend(
                        self.slider_card(Knob::Sidebar, None, cx)
                            .map(IntoElement::into_any_element),
                    );
                }
            } else {
                out.extend(
                    self.slider_card(
                        Knob::Surfaces,
                        Some(tr!(
                            "How much the panes and the sidebar let the background through."
                        )),
                        cx,
                    )
                    .map(IntoElement::into_any_element),
                );
            }
            out.push(
                div()
                    .my(px(space::SP_2))
                    .flex()
                    .child(
                        Button::new("appearance-more-opacity")
                            .label(if more {
                                tr!("One opacity for all")
                            } else {
                                tr!("Panes and sidebar apart")
                            })
                            .variant(Variant::Subtle)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(ui) = &mut this.background_ui {
                                    ui.more = !ui.more;
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
            if bg.picture {
                out.extend(
                    self.slider_card(
                        Knob::PaneBlur,
                        Some(match bg.style {
                            Style::PixelArt if bg.pane_blur > 0 => {
                                tr!("Blur hides the pixel art style behind panes.")
                            }
                            Style::Ascii if bg.pane_blur > 0 => {
                                tr!("Blur hides the ASCII style behind panes.")
                            }
                            _ => tr!("Panes blur the picture behind them, like frosted glass."),
                        }),
                        cx,
                    )
                    .map(IntoElement::into_any_element),
                );
            }
            // Legibility over the picture: the panes and the sidebar against
            // its average, darkest and lightest colours.
            if bg.picture
                && let Some(tones) = backdrop::Backdrop::tones(cx)
            {
                let muted = Rgb::from_colour(t.color.ink_3);
                let pane_need = safe_opacity(Rgb::from_colour(t.color.paper), muted, &tones.all());
                let side_need = safe_opacity(Rgb::from_colour(t.color.canvas), muted, &tones.all());
                let low = bg.panes < pane_need || bg.sidebar < side_need;
                if low {
                    let need = pane_need.max(side_need);
                    out.push(
                        div()
                            .w_full()
                            .max_w(px(CARD_MAX_W))
                            .mt(px(space::SP_3))
                            .child(if bg.style == Style::None
                                || !Rgb::from_colour(t.color.canvas).is_light()
                            {
                                Notice::new(
                                    "appearance-opacity-warning",
                                    Kind::Warning,
                                    tr!(
                                        "Muted text may be hard to read over this picture. An opacity of {0}% or more keeps it at 4.5:1.",
                                        need
                                    ),
                                )
                            } else {
                                // A style raises each pane to what its own
                                // part of the picture needs.
                                Notice::new(
                                    "appearance-opacity-raised",
                                    Kind::Info,
                                    tr!(
                                        "With a style in a light theme, each pane and the sidebar stay opaque enough for text to read over the part of the picture behind them."
                                    ),
                                )
                            })
                            .into_any_element(),
                    );
                }
            }
        }

        out.push(
            div()
                .w_full()
                .max_w(px(CARD_MAX_W))
                .mt(px(space::SP_5))
                .flex()
                .child(
                    Button::new("appearance-reset")
                        .label(tr!("Reset appearance"))
                        .icon(Icon::Undo)
                        .variant(Variant::Secondary)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.reset_appearance(window, cx)),
                        ),
                )
                .into_any_element(),
        );
        out
    }
}
