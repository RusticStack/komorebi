//! Colour themes as data (Komorebi 1.1.2 appearance): every token colour a
//! theme sets, for light and for dark, and the three presets. This file and
//! `color.rs` are the only places in the app that hold colour literals (the
//! token lint, `cargo xtask lint`, fails any outside `src/theme/`).
//!
//! A theme is a [`ColorSet`] per [`Scheme`]: the seventeen tokens the views
//! read (`Palette`), minus the four that are Windows' or fixed (Close's
//! red, the glyph on it, the dialog overlay, clear). The presets follow the
//! app's light, dark or system choice; each has both variants.
//!
//! - **Komorebi** (the default): the brand's neutral look, exactly as 1.1.1
//!   drew it (`the_komorebi_preset_is_todays_look` pins every value). Its
//!   accent is its ink: the app has no accent colour of its own
//!   (docs/BRANDING.md).
//! - **Outlook**: Fluent 2's neutral aliases with the communication blue
//!   ramp (docs/FLUENT2.md "Colour"), as current Outlook for Windows draws
//!   them. A colour scheme only: the word, no logo.
//! - **GitHub**: Primer's functional colours (light and dark default
//!   themes), from the neutral primitives.

use gpui::{Hsla, Rgba};

use super::color::Scheme;

/// An opaque colour, `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u32);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0x000000);
    pub const WHITE: Rgb = Rgb(0xFFFFFF);

    pub fn from_bytes(r: u8, g: u8, b: u8) -> Self {
        Rgb(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }

    pub fn bytes(self) -> [u8; 3] {
        let [_, r, g, b] = self.0.to_be_bytes();
        [r, g, b]
    }

    /// Channels from 0 to 1.
    pub fn unit(self) -> [f32; 3] {
        self.bytes().map(|v| f32::from(v) / 255.)
    }

    fn from_unit([r, g, b]: [f32; 3]) -> Self {
        let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        Rgb::from_bytes(byte(r), byte(g), byte(b))
    }

    /// `#rrggbb`, lower case.
    pub fn hex(self) -> String {
        format!("#{:06x}", self.0 & 0xFF_FFFF)
    }

    /// `#rrggbb` or `rrggbb` (any case, spaces around); `#rgb` too.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        match s.len() {
            6 => u32::from_str_radix(s, 16).ok().map(Rgb),
            3 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                let (r, g, b) = ((v >> 8) & 0xF, (v >> 4) & 0xF, v & 0xF);
                Some(Rgb((r * 0x11) << 16 | (g * 0x11) << 8 | (b * 0x11)))
            }
            _ => None,
        }
    }

    pub fn colour(self) -> Hsla {
        let [r, g, b] = self.unit();
        Rgba { r, g, b, a: 1. }.into()
    }

    /// The nearest opaque colour to `c` (its alpha is ignored).
    pub fn from_colour(c: Hsla) -> Self {
        let c = Rgba::from(c);
        Rgb::from_unit([c.r, c.g, c.b])
    }

    /// WCAG 2 relative luminance.
    pub fn luminance(self) -> f32 {
        let f = |v: f32| {
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let [r, g, b] = self.unit().map(f);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// `self` moved `t` (0 to 1) of the way to `to`, channel by channel.
    pub fn mix(self, to: Rgb, t: f32) -> Rgb {
        let (a, b) = (self.unit(), to.unit());
        Rgb::from_unit([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t))
    }

    /// Whether this is a light colour (text on it should be dark).
    pub fn is_light(self) -> bool {
        self.luminance() > 0.18
    }
}

/// WCAG 2 contrast ratio of two opaque colours (1 to 21).
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (x, y) = (a.luminance(), b.luminance());
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The tokens a theme sets, in the order the editor lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenId {
    Accent,
    Canvas,
    Paper,
    Ink,
    OnAccent,
    Ring,
    RingGap,
    Ink2,
    Ink3,
    OnInk,
    Inset,
    Wash,
    Line,
    Spot,
    Fall,
    SegmentOn,
    PillOnInset,
}

impl TokenId {
    pub const COUNT: usize = 17;
    pub const ALL: [TokenId; Self::COUNT] = [
        TokenId::Accent,
        TokenId::Canvas,
        TokenId::Paper,
        TokenId::Ink,
        TokenId::OnAccent,
        TokenId::Ring,
        TokenId::RingGap,
        TokenId::Ink2,
        TokenId::Ink3,
        TokenId::OnInk,
        TokenId::Inset,
        TokenId::Wash,
        TokenId::Line,
        TokenId::Spot,
        TokenId::Fall,
        TokenId::SegmentOn,
        TokenId::PillOnInset,
    ];
    /// The four the editor shows first: accent, background, panes, text.
    pub const MAIN: [TokenId; 4] = [
        TokenId::Accent,
        TokenId::Canvas,
        TokenId::Paper,
        TokenId::Ink,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn is_main(self) -> bool {
        Self::MAIN.contains(&self)
    }

    /// The token's key in a theme file.
    pub fn key(self) -> &'static str {
        match self {
            TokenId::Accent => "accent",
            TokenId::Canvas => "canvas",
            TokenId::Paper => "paper",
            TokenId::Ink => "ink",
            TokenId::OnAccent => "on-accent",
            TokenId::Ring => "ring",
            TokenId::RingGap => "ring-gap",
            TokenId::Ink2 => "ink-2",
            TokenId::Ink3 => "ink-3",
            TokenId::OnInk => "on-ink",
            TokenId::Inset => "inset",
            TokenId::Wash => "wash",
            TokenId::Line => "line",
            TokenId::Spot => "spot",
            TokenId::Fall => "fall",
            TokenId::SegmentOn => "segment-on",
            TokenId::PillOnInset => "pill-on-inset",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// One variant's colours: a value for every [`TokenId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorSet([Rgb; TokenId::COUNT]);

impl ColorSet {
    pub fn get(&self, id: TokenId) -> Rgb {
        self.0[id.index()]
    }

    pub fn set(&mut self, id: TokenId, value: Rgb) {
        self.0[id.index()] = value;
    }

    pub fn with(mut self, id: TokenId, value: Rgb) -> Self {
        self.set(id, value);
        self
    }

    /// Every token and its value, in the editor's order.
    pub fn iter(&self) -> impl Iterator<Item = (TokenId, Rgb)> + '_ {
        TokenId::ALL.into_iter().map(|t| (t, self.get(t)))
    }
}

/// A set from `(token, 0xRRGGBB)` pairs naming every token once.
fn set(values: [(TokenId, u32); TokenId::COUNT]) -> ColorSet {
    let mut out = [Rgb::BLACK; TokenId::COUNT];
    for (id, v) in values {
        out[id.index()] = Rgb(v);
    }
    ColorSet(out)
}

/// The built-in themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Preset {
    #[default]
    Komorebi,
    Outlook,
    GitHub,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Komorebi, Preset::Outlook, Preset::GitHub];

    /// The preset's key in the settings and in a theme file.
    pub fn key(self) -> &'static str {
        match self {
            Preset::Komorebi => "komorebi",
            Preset::Outlook => "outlook",
            Preset::GitHub => "github",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The preset's name (proper names: the same in every language).
    pub fn name(self) -> &'static str {
        match self {
            Preset::Komorebi => "Komorebi",
            Preset::Outlook => "Outlook",
            Preset::GitHub => "GitHub",
        }
    }

    /// The preset's colours for a scheme.
    pub fn colors(self, scheme: Scheme) -> ColorSet {
        match (self, scheme) {
            (Preset::Komorebi, Scheme::Light) => komorebi_light(),
            (Preset::Komorebi, Scheme::Dark) => komorebi_dark(),
            (Preset::Outlook, Scheme::Light) => outlook_light(),
            (Preset::Outlook, Scheme::Dark) => outlook_dark(),
            (Preset::GitHub, Scheme::Light) => github_light(),
            (Preset::GitHub, Scheme::Dark) => github_dark(),
        }
    }
}

use TokenId as T;

/// Komorebi light: the brand's palette as 1.1.1 drew it. The accent, the
/// ring and the text on the accent are the ink and on-ink; the ring's gap
/// is the paper.
fn komorebi_light() -> ColorSet {
    set([
        (T::Accent, 0x171717),
        (T::Canvas, 0xF6F6F4),
        (T::Paper, 0xFFFFFF),
        (T::Ink, 0x171717),
        (T::OnAccent, 0xFFFFFF),
        (T::Ring, 0x171717),
        (T::RingGap, 0xFFFFFF),
        (T::Ink2, 0x555552),
        (T::Ink3, 0x6A6A66),
        (T::OnInk, 0xFFFFFF),
        (T::Inset, 0xF6F6F4),
        (T::Wash, 0xEEEEEB),
        (T::Line, 0xE2E2DE),
        (T::Spot, 0xBEBEB9),
        (T::Fall, 0xF1F1EE),
        (T::SegmentOn, 0xFFFFFF),
        (T::PillOnInset, 0xFFFFFF),
    ])
}

fn komorebi_dark() -> ColorSet {
    set([
        (T::Accent, 0xF2F2F0),
        (T::Canvas, 0x121212),
        (T::Paper, 0x1A1A1A),
        (T::Ink, 0xF2F2F0),
        (T::OnAccent, 0x141414),
        (T::Ring, 0xF2F2F0),
        (T::RingGap, 0x1A1A1A),
        (T::Ink2, 0xB0B0AC),
        (T::Ink3, 0x939390),
        (T::OnInk, 0x141414),
        (T::Inset, 0x232323),
        (T::Wash, 0x262626),
        (T::Line, 0x2C2C2C),
        (T::Spot, 0x5A5A57),
        (T::Fall, 0x202020),
        (T::SegmentOn, 0x333333),
        (T::PillOnInset, 0x303030),
    ])
}

/// Outlook light: Fluent 2's light aliases (docs/FLUENT2.md "Key alias
/// tokens"): canvas NeutralBackground3 (grey96), panes NeutralBackground1
/// (white), text NeutralForeground1/2/3 (grey14, 26, 38), wash the
/// selected subtle background (grey92), the line NeutralStroke2 (grey88),
/// spot NeutralForegroundDisabled (grey74); the accent BrandBackground
/// (communication blue 80, #0F6CBD) with white on it; the focus ring
/// StrokeFocus2 (black) over StrokeFocus1 (white).
fn outlook_light() -> ColorSet {
    set([
        (T::Accent, 0x0F6CBD),
        (T::Canvas, 0xF5F5F5),
        (T::Paper, 0xFFFFFF),
        (T::Ink, 0x242424),
        (T::OnAccent, 0xFFFFFF),
        (T::Ring, 0x000000),
        (T::RingGap, 0xFFFFFF),
        (T::Ink2, 0x424242),
        (T::Ink3, 0x616161),
        (T::OnInk, 0xFFFFFF),
        (T::Inset, 0xF5F5F5),
        (T::Wash, 0xEBEBEB),
        (T::Line, 0xE0E0E0),
        (T::Spot, 0xBDBDBD),
        (T::Fall, 0xF0F0F0),
        (T::SegmentOn, 0xFFFFFF),
        (T::PillOnInset, 0xFFFFFF),
    ])
}

/// Outlook dark: Fluent 2's dark aliases: canvas NeutralBackground2
/// (grey12, #1F1F1F), panes NeutralBackground1 (grey16), cards
/// NeutralCardBackground (grey20), wash the subtle hover (grey22), text
/// white, grey84 and grey68, the line NeutralStroke2 (grey32), spot
/// NeutralForegroundDisabled (grey36); the accent BrandForeground1
/// (communication blue 100, #479EF5), so dark text sits on it (white is
/// 2.8:1 there); the focus ring white over black.
fn outlook_dark() -> ColorSet {
    set([
        (T::Accent, 0x479EF5),
        (T::Canvas, 0x1F1F1F),
        (T::Paper, 0x292929),
        (T::Ink, 0xFFFFFF),
        (T::OnAccent, 0x0A0A0A),
        (T::Ring, 0xFFFFFF),
        (T::RingGap, 0x000000),
        (T::Ink2, 0xD6D6D6),
        (T::Ink3, 0xADADAD),
        (T::OnInk, 0x242424),
        (T::Inset, 0x333333),
        (T::Wash, 0x383838),
        (T::Line, 0x525252),
        (T::Spot, 0x5C5C5C),
        (T::Fall, 0x242424),
        (T::SegmentOn, 0x424242),
        (T::PillOnInset, 0x3D3D3D),
    ])
}

/// GitHub light: Primer's light default. bgColor-muted #f6f8fa (canvas,
/// cards), bgColor-default #ffffff (panes), fgColor-default #1f2328,
/// fgColor-muted #59636e (ink-3), neutral 10 #454c54 (ink-2),
/// borderColor-default #d1d9e0 (line), control-bgColor-hover #eff2f5
/// (wash), neutral 8 #818b98 (spot, fgColor-disabled's colour); the accent
/// fgColor-accent #0969da, which is also Primer's focus outline.
fn github_light() -> ColorSet {
    set([
        (T::Accent, 0x0969DA),
        (T::Canvas, 0xF6F8FA),
        (T::Paper, 0xFFFFFF),
        (T::Ink, 0x1F2328),
        (T::OnAccent, 0xFFFFFF),
        (T::Ring, 0x0969DA),
        (T::RingGap, 0xFFFFFF),
        (T::Ink2, 0x454C54),
        (T::Ink3, 0x59636E),
        (T::OnInk, 0xFFFFFF),
        (T::Inset, 0xF6F8FA),
        (T::Wash, 0xEFF2F5),
        (T::Line, 0xD1D9E0),
        (T::Spot, 0x818B98),
        (T::Fall, 0xF6F8FA),
        (T::SegmentOn, 0xFFFFFF),
        (T::PillOnInset, 0xFFFFFF),
    ])
}

/// GitHub dark: Primer's dark default. bgColor-inset #010409 (canvas),
/// bgColor-default #0d1117 (panes), bgColor-muted #151b23 (cards),
/// control-bgColor-rest #212830 (wash), fgColor-default #f0f6fc,
/// neutral 10 #b7bdc8 (ink-2), fgColor-muted #9198a1 (ink-3),
/// borderColor-default #3d444d (line), neutral 8 #656c76 (spot); the
/// accent fgColor-accent #4493f8, with dark text on it (white is 3.1:1),
/// and the focus outline #1f6feb.
fn github_dark() -> ColorSet {
    set([
        (T::Accent, 0x4493F8),
        (T::Canvas, 0x010409),
        (T::Paper, 0x0D1117),
        (T::Ink, 0xF0F6FC),
        (T::OnAccent, 0x010409),
        (T::Ring, 0x1F6FEB),
        (T::RingGap, 0x0D1117),
        (T::Ink2, 0xB7BDC8),
        (T::Ink3, 0x9198A1),
        (T::OnInk, 0x0D1117),
        (T::Inset, 0x151B23),
        (T::Wash, 0x212830),
        (T::Line, 0x3D444D),
        (T::Spot, 0x656C76),
        (T::Fall, 0x151B23),
        (T::SegmentOn, 0x2A313C),
        (T::PillOnInset, 0x262C36),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        for v in [0x000000, 0xFFFFFF, 0x0F6CBD, 0x1F2328] {
            assert_eq!(Rgb::parse(&Rgb(v).hex()), Some(Rgb(v)));
        }
        assert_eq!(Rgb::parse(" #ABC "), Some(Rgb(0xAABBCC)));
        assert_eq!(Rgb::parse("0f6cbd"), Some(Rgb(0x0F6CBD)));
        for bad in ["", "#", "#12345", "#1234567", "#ggg000", "red", "#12 456"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn hsla_round_trips_to_the_same_bytes() {
        for v in [0x0F6CBD, 0x479EF5, 0xF6F8FA, 0x010409, 0x555552] {
            assert_eq!(Rgb::from_colour(Rgb(v).colour()), Rgb(v));
        }
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast(Rgb::BLACK, Rgb::WHITE) - 21.).abs() < 0.01);
        assert!((contrast(Rgb(0x777777), Rgb::WHITE) - 4.48).abs() < 0.02);
    }

    /// Every preset names every token in both variants (the `set` helper
    /// would leave a forgotten token black; no preset uses black but for
    /// Outlook's light focus ring).
    /// Text on the accent (buttons, the unread pill) reads at 4.5:1 in
    /// every preset, light and dark (GitHub and Outlook dark put dark text
    /// on their light blue accents: white is 3.1:1 and 2.8:1 there).
    #[test]
    fn text_on_the_accent_passes_in_every_preset() {
        for p in Preset::ALL {
            for scheme in [Scheme::Light, Scheme::Dark] {
                let c = p.colors(scheme);
                let r = contrast(c.get(T::OnAccent), c.get(T::Accent));
                assert!(r >= 4.5, "{p:?} {scheme:?}: {r:.2}");
            }
        }
    }

    #[test]
    fn every_preset_sets_every_token_in_both_variants() {
        for preset in Preset::ALL {
            for scheme in [Scheme::Light, Scheme::Dark] {
                let c = preset.colors(scheme);
                for (id, v) in c.iter() {
                    let black_allowed = matches!(
                        (preset, scheme, id),
                        (Preset::Outlook, Scheme::Light, TokenId::Ring)
                            | (Preset::Outlook, Scheme::Dark, TokenId::RingGap)
                    );
                    assert!(
                        v != Rgb::BLACK || black_allowed,
                        "{preset:?} {scheme:?} {id:?} is unset"
                    );
                }
                // A light variant has light panes, a dark one dark panes.
                assert_eq!(
                    c.get(TokenId::Paper).is_light(),
                    scheme == Scheme::Light,
                    "{preset:?} {scheme:?}"
                );
            }
        }
        let keys: std::collections::HashSet<_> = TokenId::ALL.iter().map(|t| t.key()).collect();
        assert_eq!(keys.len(), TokenId::COUNT);
        for t in TokenId::ALL {
            assert_eq!(TokenId::from_key(t.key()), Some(t));
        }
    }

    #[test]
    fn the_brief_values_are_the_presets() {
        let o = Preset::Outlook;
        assert_eq!(o.colors(Scheme::Light).get(TokenId::Accent), Rgb(0x0F6CBD));
        assert_eq!(o.colors(Scheme::Dark).get(TokenId::Accent), Rgb(0x479EF5));
        let g = Preset::GitHub.colors(Scheme::Light);
        assert_eq!(
            [T::Canvas, T::Paper, T::Ink, T::Ink3, T::Line, T::Accent].map(|t| g.get(t)),
            [0xF6F8FA, 0xFFFFFF, 0x1F2328, 0x59636E, 0xD1D9E0, 0x0969DA].map(Rgb)
        );
        let g = Preset::GitHub.colors(Scheme::Dark);
        assert_eq!(
            [T::Canvas, T::Paper, T::Ink, T::Ink3, T::Line, T::Accent].map(|t| g.get(t)),
            [0x010409, 0x0D1117, 0xF0F6FC, 0x9198A1, 0x3D444D, 0x4493F8].map(Rgb)
        );
    }
}
