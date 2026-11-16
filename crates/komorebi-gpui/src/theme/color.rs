//! The semantic colours, one [`Palette`] per [`Scheme`], built from a
//! theme's [`ColorSet`] (`presets.rs`; komorebi-brand final BRAND.md
//! "Colour" and IMPLEMENTATION.md section 1 for the default). With
//! `presets.rs`, the only place in the app that holds colour literals (the
//! token lint, `cargo xtask lint`, fails any outside `src/theme/`).
//!
//! Windows' accent colour is never read. The default theme, Komorebi, has
//! no accent colour of its own (its accent is its ink; docs/BRANDING.md),
//! so its one hue is Windows' Close red; other themes are the user's
//! choice (Settings > Appearance). High contrast is not in 1.x; it becomes
//! one more [`Scheme`] variant whose palette is filled from the Windows
//! system colours (`komorebi::theme::SystemTheme::contrast`), with no
//! change to the views.

use gpui::{Hsla, Rgba, rgb};

use super::backdrop::{Background, Material};
use super::presets::{ColorSet, Preset, TokenId};

/// Which palette the app draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

impl Scheme {
    pub fn is_dark(self) -> bool {
        self == Scheme::Dark
    }
}

/// One scheme's semantic colours. Views read these through
/// `cx.tokens().color` and never name a hex value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Window, navigation and title bar.
    pub canvas: Hsla,
    /// Panes (list, reader, composer), menus, dialogs.
    pub paper: Hsla,
    /// Cards, notices, attachments and event pills inside paper.
    pub inset: Hsla,
    /// Selected row, hover, secondary button, avatar ground.
    pub wash: Hsla,
    /// The large light shape on the welcome screen only.
    pub fall: Hsla,
    /// The one 1 px edge: under composer fields.
    pub line: Hsla,
    /// The dim loader spot and disabled marks (never a control's only cue).
    pub spot: Hsla,
    /// Text, unread spot, primary button, focus ring, selection marker.
    pub ink: Hsla,
    /// Read subjects, icons, secondary text.
    pub ink_2: Hsla,
    /// Times, previews, captions, placeholders, scroll thumbs.
    pub ink_3: Hsla,
    /// Text and glyphs on ink.
    pub on_ink: Hsla,
    /// The selected segment of a segmented control on an inset track.
    pub segment_on: Hsla,
    /// Pills and selects that sit inside an inset card.
    pub pill_on_inset: Hsla,
    /// Close's hover: Windows' own caption red, the one hue in the
    /// app, because it is Windows' control (IMPLEMENTATION.md section 4).
    pub close_hover: Hsla,
    /// The glyph on [`Palette::close_hover`].
    pub on_close: Hsla,
    /// Behind dialogs: black at 0.32, no blur.
    pub overlay: Hsla,
    /// Fully transparent (scroll tracks, Kit fields with no fill).
    pub clear: Hsla,
    /// The theme's accent: the primary button, the selection marker and
    /// place spot, links, toggles and checks, the unread spot, the focused
    /// field's edge, the loader. Komorebi's is its ink.
    pub accent: Hsla,
    /// Text and glyphs on the accent.
    pub on_accent: Hsla,
    /// The keyboard focus ring (2 px) and its gap (2 px), outside the
    /// element (`widgets::focus_ring`).
    pub ring: Hsla,
    pub ring_gap: Hsla,
    /// A pane's fill: paper, or paper at the panes' opacity over a
    /// background picture or window material (Settings > Appearance).
    /// Menus, dialogs and the folder drawer stay paper and canvas.
    pub pane: Hsla,
    /// The window's own fill under everything: canvas; canvas at the tint
    /// over a material with no picture; clear over a material with one.
    pub backdrop: Hsla,
    /// Over the picture, under the panes: canvas at the sidebar's opacity
    /// (only drawn with a picture).
    pub veil: Hsla,
    /// The title bar's fill: canvas, or clear when the window is see-through
    /// (the backdrop shows, as it does beside the sidebar).
    pub chrome: Hsla,
}

const CLOSE_HOVER: u32 = 0xC42B1C;
const ON_CLOSE: u32 = 0xFFFFFF;
/// IMPLEMENTATION.md: dialog overlay, black at 0.32.
const OVERLAY_ALPHA: f32 = 0.32;

fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

fn overlay() -> Hsla {
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: OVERLAY_ALPHA,
    }
    .into()
}

fn clear() -> Hsla {
    Hsla::transparent_black()
}

impl Palette {
    /// A theme's colours with the ones every theme shares (Close's red, the
    /// overlay, clear).
    pub fn from_set(set: &ColorSet) -> Self {
        let c = |id: TokenId| set.get(id).colour();
        Palette {
            canvas: c(TokenId::Canvas),
            paper: c(TokenId::Paper),
            inset: c(TokenId::Inset),
            wash: c(TokenId::Wash),
            fall: c(TokenId::Fall),
            line: c(TokenId::Line),
            spot: c(TokenId::Spot),
            ink: c(TokenId::Ink),
            ink_2: c(TokenId::Ink2),
            ink_3: c(TokenId::Ink3),
            on_ink: c(TokenId::OnInk),
            segment_on: c(TokenId::SegmentOn),
            pill_on_inset: c(TokenId::PillOnInset),
            accent: c(TokenId::Accent),
            on_accent: c(TokenId::OnAccent),
            ring: c(TokenId::Ring),
            ring_gap: c(TokenId::RingGap),
            close_hover: hex(CLOSE_HOVER),
            on_close: hex(ON_CLOSE),
            overlay: overlay(),
            clear: clear(),
            pane: c(TokenId::Paper),
            backdrop: c(TokenId::Canvas),
            veil: c(TokenId::Canvas),
            chrome: c(TokenId::Canvas),
        }
    }

    /// The surfaces over a background picture or window material. Off (the
    /// default) changes nothing.
    pub fn with_background(mut self, bg: &Background) -> Self {
        if !bg.is_on() {
            return self;
        }
        self.pane = self.paper.opacity(bg.pane_alpha());
        self.veil = self.canvas.opacity(bg.veil_alpha());
        self.backdrop = match (bg.picture, bg.material) {
            // An opaque window: the canvas under the picture, for what a
            // Fit or Centre picture leaves uncovered.
            (_, Material::None) => self.canvas,
            (false, _) => self.canvas.opacity(bg.veil_alpha()),
            (true, _) => self.clear,
        };
        self.chrome = self.clear;
        self
    }
}

/// The default (Komorebi) palette of a scheme.
pub fn palette(scheme: Scheme) -> Palette {
    Palette::from_set(&Preset::Komorebi.colors(scheme))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2 contrast ratio of two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let l = |c: Hsla| {
            let c = c.to_rgb();
            let f = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
        };
        let (x, y) = (l(a), l(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// No accent colour (docs/BRANDING.md): every colour but Close's hover
    /// red is a grey, its channels within 8 of each other out of 255 (the
    /// warm greys differ by at most 5, spot #BEBEB9; the former persimmon
    /// seal #B3503A by 121).
    #[test]
    fn the_palette_has_no_accent_colour() {
        let spread = |c: Hsla| {
            let c = c.to_rgb();
            let (hi, lo) = (c.r.max(c.g).max(c.b), c.r.min(c.g).min(c.b));
            ((hi - lo) * 255.).round()
        };
        for scheme in [Scheme::Light, Scheme::Dark] {
            let p = palette(scheme);
            for (name, c) in [
                ("canvas", p.canvas),
                ("paper", p.paper),
                ("inset", p.inset),
                ("wash", p.wash),
                ("fall", p.fall),
                ("line", p.line),
                ("spot", p.spot),
                ("ink", p.ink),
                ("ink-2", p.ink_2),
                ("ink-3", p.ink_3),
                ("on-ink", p.on_ink),
                ("segment-on", p.segment_on),
                ("pill-on-inset", p.pill_on_inset),
                ("on-close", p.on_close),
                ("accent", p.accent),
                ("on-accent", p.on_accent),
                ("ring", p.ring),
                ("ring-gap", p.ring_gap),
            ] {
                assert!(spread(c) <= 8., "{scheme:?} {name} has a hue");
            }
        }
    }

    /// The default theme is 1.1.1's look, pixel for pixel: every token of
    /// both schemes pinned to the value it had before themes existed (the
    /// accent, the ring and the text on the accent are the ink, the ring's
    /// gap the paper, as `widgets::focus_ring` drew them).
    #[test]
    fn the_komorebi_preset_is_todays_look() {
        let today = |scheme| match scheme {
            Scheme::Light => [
                0xF6F6F4, 0xFFFFFF, 0xF6F6F4, 0xEEEEEB, 0xF1F1EE, 0xE2E2DE, 0xBEBEB9, 0x171717,
                0x555552, 0x6A6A66, 0xFFFFFF, 0xFFFFFF, 0xFFFFFF, 0x171717, 0xFFFFFF, 0x171717,
                0xFFFFFF,
            ],
            Scheme::Dark => [
                0x121212, 0x1A1A1A, 0x232323, 0x262626, 0x202020, 0x2C2C2C, 0x5A5A57, 0xF2F2F0,
                0xB0B0AC, 0x939390, 0x141414, 0x333333, 0x303030, 0xF2F2F0, 0x141414, 0xF2F2F0,
                0x1A1A1A,
            ],
        };
        for scheme in [Scheme::Light, Scheme::Dark] {
            let p = palette(scheme);
            let got = [
                p.canvas,
                p.paper,
                p.inset,
                p.wash,
                p.fall,
                p.line,
                p.spot,
                p.ink,
                p.ink_2,
                p.ink_3,
                p.on_ink,
                p.segment_on,
                p.pill_on_inset,
                p.accent,
                p.on_accent,
                p.ring,
                p.ring_gap,
            ];
            for (ix, (g, want)) in got.iter().zip(today(scheme)).enumerate() {
                assert_eq!(*g, hex(want), "{scheme:?} token {ix}");
            }
            assert_eq!(p.close_hover, hex(0xC42B1C));
            assert_eq!(p.on_close, hex(0xFFFFFF));
            assert_eq!(p.overlay, overlay());
            assert_eq!(p.clear, Hsla::transparent_black());
        }
    }

    /// BRAND.md's contrast claims hold for the text ramp on every ground
    /// text sits on.
    #[test]
    fn the_text_ramp_meets_the_brand_contrast() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let p = palette(scheme);
            for ground in [p.paper, p.canvas, p.wash] {
                assert!(contrast(p.ink, ground) >= 13.0, "{scheme:?} ink");
                assert!(contrast(p.ink_2, ground) >= 6.0, "{scheme:?} ink-2");
                assert!(contrast(p.ink_3, ground) >= 4.5, "{scheme:?} ink-3");
            }
            assert!(contrast(p.on_ink, p.ink) >= 13.0, "{scheme:?} on-ink");
            // Scroll thumbs are ink-3: 3:1 for a control part.
            assert!(contrast(p.ink_3, p.paper) >= 3.0);
        }
    }

    #[test]
    fn the_overlay_is_flat_black_at_a_third() {
        let o = palette(Scheme::Dark).overlay;
        assert!((o.a - OVERLAY_ALPHA).abs() < 1e-6);
        assert_eq!(o.l, 0.);
    }

    /// Off, the surfaces are today's: the panes paper, the window and the
    /// title bar canvas (no added per-frame work: nothing else is drawn).
    #[test]
    fn the_defaults_add_nothing() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let p = palette(scheme);
            let q = p.with_background(&Background::default());
            assert_eq!(p, q);
            assert_eq!(
                (q.pane, q.backdrop, q.veil, q.chrome),
                (p.paper, p.canvas, p.canvas, p.canvas)
            );
            // Sliders moved with nothing on: still opaque.
            let moved = Background {
                panes: 50,
                sidebar: 50,
                ..Background::default()
            };
            assert_eq!(p.with_background(&moved), p);
        }
    }

    #[test]
    fn a_picture_makes_the_panes_translucent() {
        let p = palette(Scheme::Light);
        let bg = Background {
            picture: true,
            panes: 70,
            sidebar: 50,
            ..Background::default()
        };
        let q = p.with_background(&bg);
        assert!((q.pane.a - 0.7).abs() < 1e-6);
        assert!((q.veil.a - 0.5).abs() < 1e-6);
        assert_eq!(q.backdrop, p.canvas);
        assert_eq!(q.chrome, p.clear);
        // Menus and dialogs keep the opaque paper.
        assert_eq!(q.paper, p.paper);
        let mica = Background {
            material: Material::Mica,
            tint: 50,
            ..Background::default()
        };
        let m = p.with_background(&mica);
        assert!((m.backdrop.a - 0.5).abs() < 1e-6);
    }
}
