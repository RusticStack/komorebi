//! The brand's colours (komorebi-brand final BRAND.md "Colour" and
//! IMPLEMENTATION.md section 1), one [`Palette`] per [`Scheme`]. The only
//! place in the app that holds colour literals (the token lint,
//! `tests/token_lint.rs`, fails any outside `src/theme/`).
//!
//! Windows' accent colour is never read (BRAND.md: the brand owns one
//! colour). High contrast is not in 1.0; it becomes one more [`Scheme`]
//! variant whose palette is filled from the Windows system colours
//! (`komorebi::theme::SystemTheme::contrast`), with no change to the views.

use gpui::{Hsla, Rgba, rgb};

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
    /// The persimmon seal: the seal path only (BRAND.md persimmon rule).
    pub seal: Hsla,
    /// The selected segment of a segmented control on an inset track.
    pub segment_on: Hsla,
    /// Pills and selects that sit inside an inset card.
    pub pill_on_inset: Hsla,
    /// Close's hover: Windows' own caption red, the one hue besides the
    /// seal, because it is Windows' control (IMPLEMENTATION.md section 4).
    pub close_hover: Hsla,
    /// The glyph on [`Palette::close_hover`].
    pub on_close: Hsla,
    /// Behind dialogs: black at 0.32, no blur.
    pub overlay: Hsla,
    /// Fully transparent (scroll tracks, Kit fields with no fill).
    pub clear: Hsla,
}

const SEAL: u32 = 0xB3503A;
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

/// The palette of a scheme. A later high contrast scheme is one more arm.
pub fn palette(scheme: Scheme) -> Palette {
    match scheme {
        Scheme::Light => Palette {
            canvas: hex(0xF6F6F4),
            paper: hex(0xFFFFFF),
            inset: hex(0xF6F6F4),
            wash: hex(0xEEEEEB),
            fall: hex(0xF1F1EE),
            line: hex(0xE2E2DE),
            spot: hex(0xBEBEB9),
            ink: hex(0x171717),
            ink_2: hex(0x555552),
            ink_3: hex(0x6A6A66),
            on_ink: hex(0xFFFFFF),
            seal: hex(SEAL),
            segment_on: hex(0xFFFFFF),
            pill_on_inset: hex(0xFFFFFF),
            close_hover: hex(CLOSE_HOVER),
            on_close: hex(ON_CLOSE),
            overlay: overlay(),
            clear: clear(),
        },
        Scheme::Dark => Palette {
            canvas: hex(0x121212),
            paper: hex(0x1A1A1A),
            inset: hex(0x232323),
            wash: hex(0x262626),
            fall: hex(0x202020),
            line: hex(0x2C2C2C),
            spot: hex(0x5A5A57),
            ink: hex(0xF2F2F0),
            ink_2: hex(0xB0B0AC),
            ink_3: hex(0x939390),
            on_ink: hex(0x141414),
            seal: hex(SEAL),
            segment_on: hex(0x333333),
            pill_on_inset: hex(0x303030),
            close_hover: hex(CLOSE_HOVER),
            on_close: hex(ON_CLOSE),
            overlay: overlay(),
            clear: clear(),
        },
    }
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

    #[test]
    fn the_seal_is_the_same_persimmon_in_both_schemes() {
        assert_eq!(palette(Scheme::Light).seal, palette(Scheme::Dark).seal);
        assert_eq!(palette(Scheme::Light).seal, hex(0xB3503A));
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
}
