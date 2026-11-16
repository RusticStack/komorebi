//! Settings > Appearance > Background (Komorebi 1.1.2): a picture behind
//! the panes, how translucent the panes and the window's own background
//! are over it, and Windows' window material (Mica, Acrylic).
//!
//! All of it is off by default, and off costs nothing: no picture is held,
//! no element is added to the window and no token changes (the
//! `the_defaults_add_nothing` test). What is on is decided here as data
//! ([`Background`], saved with the colour theme under the `appearance`
//! key); the pixels are `views::backdrop`'s, made once off the UI thread
//! (decode, cap at the largest monitor, dim, blur) and drawn as one or two
//! images: the picture, and for Blur behind panes a smaller, softer copy
//! each pane draws clipped to its own rounded rectangle.
//!
//! Legibility: the panes' opacity has a floor ([`MIN_OPACITY`]), and the
//! page warns when muted text on a pane over the picture's average, darkest
//! or lightest colour would fall below 4.5:1 ([`safe_opacity`]).

use gpui::{Bounds, Pixels, Point, Size, point, px, size};
use serde_json::{Value, json};

use super::derive::TEXT_AA;
use super::presets::{Rgb, contrast};

/// How the picture covers the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Covers the window, cropped to it (the default).
    #[default]
    Fill,
    /// Whole, inside the window.
    Fit,
    /// At its own size, repeated.
    Tile,
    /// At its own size, in the middle.
    Centre,
}

impl Fit {
    pub const ALL: [Fit; 4] = [Fit::Fill, Fit::Fit, Fit::Tile, Fit::Centre];

    pub fn key(self) -> &'static str {
        match self {
            Fit::Fill => "fill",
            Fit::Fit => "fit",
            Fit::Tile => "tile",
            Fit::Centre => "centre",
        }
    }

    pub fn from_key(key: &str) -> Option<Fit> {
        Fit::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// Windows' material behind a translucent window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Material {
    /// An opaque window (the default).
    #[default]
    None,
    /// Mica: the desktop's colours, softly (Windows 11 22H2 and later).
    Mica,
    /// Acrylic: what is behind the window, blurred.
    Acrylic,
}

impl Material {
    pub const ALL: [Material; 3] = [Material::None, Material::Mica, Material::Acrylic];

    pub fn key(self) -> &'static str {
        match self {
            Material::None => "none",
            Material::Mica => "mica",
            Material::Acrylic => "acrylic",
        }
    }

    pub fn from_key(key: &str) -> Option<Material> {
        Material::ALL.into_iter().find(|m| m.key() == key)
    }
}

/// A style the picture is redrawn in, once, when it is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    /// The picture as it is.
    #[default]
    None,
    /// Square cells of few colours, upscaled without smoothing.
    PixelArt,
    /// Monospace characters chosen by each cell's brightness.
    Ascii,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::None, Style::PixelArt, Style::Ascii];

    pub fn key(self) -> &'static str {
        match self {
            Style::None => "none",
            Style::PixelArt => "pixel",
            Style::Ascii => "ascii",
        }
    }

    pub fn from_key(key: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// What colours the ASCII characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AsciiInk {
    /// Each character the colour of its part of the picture.
    #[default]
    Picture,
    /// All in the theme's text colour.
    Text,
    /// All in the theme's accent.
    Accent,
}

impl AsciiInk {
    pub const ALL: [AsciiInk; 3] = [AsciiInk::Picture, AsciiInk::Text, AsciiInk::Accent];

    pub fn key(self) -> &'static str {
        match self {
            AsciiInk::Picture => "picture",
            AsciiInk::Text => "text",
            AsciiInk::Accent => "accent",
        }
    }

    pub fn from_key(key: &str) -> Option<AsciiInk> {
        AsciiInk::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// A style's cell, in logical px.
pub const MIN_CELL: u8 = 2;
pub const MAX_CELL: u8 = 16;
/// The pixel-art palette sizes offered.
pub const PALETTES: [u8; 4] = [8, 16, 32, 64];

/// The first Windows build with Mica as a system backdrop
/// (`DWMWA_SYSTEMBACKDROP_TYPE`).
pub const MICA_BUILD: u32 = 22621;

/// How long a slider rests before the picture is made again (each move
/// would otherwise start a new blur).
pub const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(120);
/// The blur slider is a radius in logical px; the Gaussian's sigma is half
/// of it (a radius of about two sigmas is what the eye reads as the blur).
pub const SIGMA_PER_PX: f32 = 0.5;

/// The least opacity a pane or the sidebar may have, in percent.
pub const MIN_OPACITY: u8 = 40;
/// The most dimming and blur, in percent and px.
pub const MAX_DIM: u8 = 100;
pub const MAX_BLUR: u8 = 40;

/// The background settings (all off by default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Background {
    /// A picture is chosen (kept as `background.png` in the app's data
    /// folder, never in the mail cache).
    pub picture: bool,
    pub fit: Fit,
    /// Darkens the picture, 0 to 100 %.
    pub dim: u8,
    /// Blurs the picture, 0 to 40 px.
    pub blur: u8,
    /// The panes' opacity over the picture or the material, 40 to 100 %.
    pub panes: u8,
    /// The sidebar's (the window's own background's) opacity, 40 to 100 %.
    pub sidebar: u8,
    /// Blur behind panes, 0 (off) to 40 px: each pane shows a copy of the
    /// picture blurred this much (frosted glass).
    pub pane_blur: u8,
    pub material: Material,
    /// The style the picture is redrawn in.
    pub style: Style,
    /// The style's cell, 2 to 16 logical px.
    pub cell: u8,
    /// Pixel art: how many colours (one of [`PALETTES`]).
    pub colours: u8,
    /// Pixel art: ordered (Bayer) dithering between the colours.
    pub dither: bool,
    /// ASCII: what colours the characters.
    pub ascii_ink: AsciiInk,
    /// ASCII: how bold the characters are and how much of their colour
    /// fills their cells, 0 to 100 %.
    pub intensity: u8,
    /// How much the picture fades out toward the window's edges, 0 to 100 %.
    pub vignette: u8,
    /// How far in from the edges the fade starts, 0 to 100 % of the
    /// shorter side's half.
    pub vignette_size: u8,
    /// The window background's opacity over the material, 40 to 100 %.
    pub tint: u8,
}

impl Default for Background {
    fn default() -> Self {
        Background {
            picture: false,
            fit: Fit::Fill,
            dim: 0,
            blur: 0,
            panes: 100,
            sidebar: 100,
            pane_blur: 0,
            material: Material::None,
            tint: 60,
            style: Style::None,
            cell: 6,
            colours: 16,
            dither: true,
            ascii_ink: AsciiInk::Picture,
            intensity: 50,
            vignette: 0,
            vignette_size: 40,
        }
    }
}

impl Background {
    /// As JSON (the settings value and, without the picture, theme files).
    pub fn to_json(&self) -> Value {
        json!({
            "picture": self.picture,
            "fit": self.fit.key(),
            "dim": self.dim,
            "blur": self.blur,
            "panes": self.panes,
            "sidebar": self.sidebar,
            "pane_blur": self.pane_blur,
            "material": self.material.key(),
            "tint": self.tint,
            "style": self.style.key(),
            "cell": self.cell,
            "colours": self.colours,
            "dither": self.dither,
            "ascii_ink": self.ascii_ink.key(),
            "intensity": self.intensity,
            "vignette": self.vignette,
            "vignette_size": self.vignette_size,
        })
    }

    /// From JSON, leniently: what is missing or unreadable is the default,
    /// and numbers are brought into range.
    pub fn from_json(v: &Value) -> Self {
        let d = Background::default();
        let num = |k: &str, or: u8| {
            v.get(k)
                .and_then(Value::as_u64)
                .map_or(or, |n| n.min(u64::from(u8::MAX)) as u8)
        };
        let flag = |k: &str, or: bool| v.get(k).and_then(Value::as_bool).unwrap_or(or);
        let key = |k: &str| v.get(k).and_then(Value::as_str);
        Background {
            picture: flag("picture", d.picture),
            fit: key("fit").and_then(Fit::from_key).unwrap_or(d.fit),
            dim: num("dim", d.dim),
            blur: num("blur", d.blur),
            panes: num("panes", d.panes),
            sidebar: num("sidebar", d.sidebar),
            pane_blur: num("pane_blur", d.pane_blur),
            material: key("material")
                .and_then(Material::from_key)
                .unwrap_or(d.material),
            tint: num("tint", d.tint),
            style: key("style").and_then(Style::from_key).unwrap_or(d.style),
            cell: num("cell", d.cell),
            colours: num("colours", d.colours),
            dither: flag("dither", d.dither),
            ascii_ink: key("ascii_ink")
                .and_then(AsciiInk::from_key)
                .unwrap_or(d.ascii_ink),
            intensity: num("intensity", d.intensity),
            vignette: num("vignette", d.vignette),
            vignette_size: num("vignette_size", d.vignette_size),
        }
        .clamped()
    }

    /// Anything is on: a picture or a material.
    pub fn is_on(&self) -> bool {
        self.picture || self.material != Material::None
    }

    /// Values from a file or the settings, brought into range.
    pub fn clamped(mut self) -> Self {
        self.dim = self.dim.min(MAX_DIM);
        self.blur = self.blur.min(MAX_BLUR);
        self.panes = self.panes.clamp(MIN_OPACITY, 100);
        self.sidebar = self.sidebar.clamp(MIN_OPACITY, 100);
        self.tint = self.tint.clamp(MIN_OPACITY, 100);
        self.pane_blur = self.pane_blur.min(MAX_BLUR);
        self.cell = self.cell.clamp(MIN_CELL, MAX_CELL);
        // The nearest palette size offered.
        let c = self.colours;
        self.colours = PALETTES
            .into_iter()
            .min_by_key(|p| p.abs_diff(c))
            .unwrap_or(16);
        self.vignette = self.vignette.min(100);
        self.intensity = self.intensity.min(100);
        self.vignette_size = self.vignette_size.min(100);
        self
    }

    /// The panes' alpha (1 when nothing is on: an opaque window shows no
    /// translucency).
    pub fn pane_alpha(&self) -> f32 {
        if self.is_on() {
            f32::from(self.panes.clamp(MIN_OPACITY, 100)) / 100.
        } else {
            1.
        }
    }

    /// The window background's alpha over the picture (the sidebar slider)
    /// or, with no picture, over the material (the tint).
    pub fn veil_alpha(&self) -> f32 {
        if self.picture {
            f32::from(self.sidebar.clamp(MIN_OPACITY, 100)) / 100.
        } else if self.material != Material::None {
            f32::from(self.tint.clamp(MIN_OPACITY, 100)) / 100.
        } else {
            1.
        }
    }
}

/// Where a picture of `image` logical px is drawn in `area` for `fit`:
/// one rectangle, or for Tile as many as cover the area (from its top
/// left). Fill and Fit keep the picture's aspect and centre it.
pub fn place(fit: Fit, image: Size<f32>, area: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let (iw, ih) = (image.width.max(1.), image.height.max(1.));
    let (aw, ah) = (f32::from(area.size.width), f32::from(area.size.height));
    let centred = |w: f32, h: f32| {
        vec![Bounds {
            origin: point(
                area.origin.x + px((aw - w) / 2.),
                area.origin.y + px((ah - h) / 2.),
            ),
            size: size(px(w), px(h)),
        }]
    };
    match fit {
        Fit::Fill => {
            let s = (aw / iw).max(ah / ih);
            centred(iw * s, ih * s)
        }
        Fit::Fit => {
            let s = (aw / iw).min(ah / ih);
            centred(iw * s, ih * s)
        }
        Fit::Centre => centred(iw, ih),
        Fit::Tile => {
            let (cols, rows) = ((aw / iw).ceil() as i32, (ah / ih).ceil() as i32);
            let mut out = Vec::with_capacity((cols.max(0) * rows.max(0)) as usize);
            for r in 0..rows {
                for c in 0..cols {
                    out.push(Bounds {
                        origin: Point {
                            x: area.origin.x + px(c as f32 * iw),
                            y: area.origin.y + px(r as f32 * ih),
                        },
                        size: size(px(iw), px(ih)),
                    });
                }
            }
            out
        }
    }
}

/// What a picture is made of, for the legibility warning: its average,
/// darkest and lightest colours (as baked: dimmed and blurred).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tones {
    pub average: Rgb,
    pub darkest: Rgb,
    pub lightest: Rgb,
}

impl Tones {
    pub fn all(&self) -> [Rgb; 3] {
        [self.average, self.darkest, self.lightest]
    }
}

/// The picture's darkest and lightest colours over a coarse grid, so a pane
/// is checked against what is behind it, not the whole picture.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToneGrid {
    pub cols: usize,
    pub rows: usize,
    /// Row by row; None where nothing is drawn.
    pub cells: Vec<Option<Tones>>,
}

impl ToneGrid {
    /// The darkest and lightest colours under the part of the picture from
    /// `(x0, y0)` to `(x1, y1)`, as shares of its width and height (0 to 1).
    pub fn under(&self, (x0, y0): (f32, f32), (x1, y1): (f32, f32)) -> Option<[Rgb; 2]> {
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        let span = |a: f32, b: f32, n: usize| {
            let lo = ((a.clamp(0., 1.) * n as f32).floor() as usize).min(n - 1);
            let hi = ((b.clamp(0., 1.) * n as f32).ceil() as usize).clamp(lo + 1, n);
            lo..hi
        };
        let (cols, rows) = (span(x0, x1, self.cols), span(y0, y1, self.rows));
        let mut out: Option<[Rgb; 2]> = None;
        for r in rows {
            for c in cols.clone() {
                let Some(t) = self.cells[r * self.cols + c] else {
                    continue;
                };
                out = Some(match out {
                    None => [t.darkest, t.lightest],
                    Some([d, l]) => [
                        if t.darkest.luminance() < d.luminance() {
                            t.darkest
                        } else {
                            d
                        },
                        if t.lightest.luminance() > l.luminance() {
                            t.lightest
                        } else {
                            l
                        },
                    ],
                });
            }
        }
        out
    }
}

/// `top` at `alpha` over `under`.
pub fn over(top: Rgb, alpha: f32, under: Rgb) -> Rgb {
    under.mix(top, alpha.clamp(0., 1.))
}

/// The least pane opacity, in percent, at which `text` on `pane` over each
/// of `grounds` still reads at 4.5:1 (never under [`MIN_OPACITY`]; 100 when
/// even an opaque pane does not).
pub fn safe_opacity(pane: Rgb, text: Rgb, grounds: &[Rgb]) -> u8 {
    (MIN_OPACITY..=100)
        .find(|p| {
            let a = f32::from(*p) / 100.;
            grounds
                .iter()
                .all(|g| contrast(text, over(pane, a, *g)) >= TEXT_AA)
        })
        .unwrap_or(100)
}

/// `KOMOREBI_SNAPSHOT_BACKDROP`: `key=value` pairs, comma separated, over
/// the defaults (`fit=tile,dim=20,blur=12,panes=70,sidebar=60,
/// paneblur=24,material=mica,tint=50,style=pixel,cell=6,colours=16,dither=1,
/// ink=picture,vignette=60,vsize=40`).
pub fn parse_switch(text: &str) -> Result<Background, String> {
    let mut bg = Background::default();
    for pair in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("{pair}: expected key=value"))?;
        let (k, v) = (k.trim(), v.trim());
        let num = |max: u8| {
            v.parse::<u8>()
                .ok()
                .filter(|n| *n <= max)
                .ok_or_else(|| format!("{k}={v}: expected 0 to {max}"))
        };
        match k {
            "fit" => {
                bg.fit =
                    Fit::from_key(v).ok_or_else(|| format!("fit={v}: fill, fit, tile or centre"))?
            }
            "dim" => bg.dim = num(MAX_DIM)?,
            "blur" => bg.blur = num(MAX_BLUR)?,
            "panes" => bg.panes = num(100)?,
            "sidebar" => bg.sidebar = num(100)?,
            "tint" => bg.tint = num(100)?,
            "paneblur" => bg.pane_blur = num(MAX_BLUR)?,
            "style" => {
                bg.style =
                    Style::from_key(v).ok_or_else(|| format!("style={v}: none, pixel or ascii"))?
            }
            "cell" => bg.cell = num(MAX_CELL)?,
            "colours" => bg.colours = num(64)?,
            "dither" => bg.dither = num(1)? == 1,
            "ink" => {
                bg.ascii_ink = AsciiInk::from_key(v)
                    .ok_or_else(|| format!("ink={v}: picture, text or accent"))?
            }
            "vignette" => bg.vignette = num(100)?,
            "intensity" => bg.intensity = num(100)?,
            "vsize" => bg.vignette_size = num(100)?,
            "material" => {
                bg.material = Material::from_key(v)
                    .ok_or_else(|| format!("material={v}: none, mica or acrylic"))?
            }
            _ => return Err(format!("{k}: not a backdrop setting")),
        }
    }
    Ok(bg.clamped())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(w), px(h)),
        }
    }

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }

    #[test]
    fn fill_covers_and_crops_centred() {
        // A 2:1 picture in a 4:3 window: scaled to the height, cropped at
        // the sides.
        let got = place(Fit::Fill, size(200., 100.), area(400., 300.));
        assert_eq!(got, vec![b(-100., 0., 600., 300.)]);
        // A tall picture in a wide window: scaled to the width.
        let got = place(Fit::Fill, size(100., 200.), area(400., 300.));
        assert_eq!(got, vec![b(0., -250., 400., 800.)]);
    }

    #[test]
    fn fit_shows_it_whole_centred() {
        let got = place(Fit::Fit, size(200., 100.), area(400., 300.));
        assert_eq!(got, vec![b(0., 50., 400., 200.)]);
    }

    #[test]
    fn centre_keeps_its_size() {
        let got = place(Fit::Centre, size(100., 50.), area(400., 300.));
        assert_eq!(got, vec![b(150., 125., 100., 50.)]);
        // Larger than the window: still centred, overflowing evenly.
        let got = place(Fit::Centre, size(500., 400.), area(400., 300.));
        assert_eq!(got, vec![b(-50., -50., 500., 400.)]);
    }

    #[test]
    fn tile_covers_from_the_top_left() {
        let got = place(Fit::Tile, size(150., 100.), area(400., 250.));
        // 3 columns (the last cut), 3 rows (the last cut).
        assert_eq!(got.len(), 9);
        assert_eq!(got[0], b(0., 0., 150., 100.));
        assert_eq!(got[8], b(300., 200., 150., 100.));
        // An empty window: nothing.
        assert!(place(Fit::Tile, size(150., 100.), area(0., 0.)).is_empty());
    }

    #[test]
    fn a_zero_sized_picture_does_not_divide_by_zero() {
        let got = place(Fit::Fill, size(0., 0.), area(400., 300.));
        assert!(got[0].size.width.is_finite_px());
    }

    trait Finite {
        fn is_finite_px(&self) -> bool;
    }
    impl Finite for Pixels {
        fn is_finite_px(&self) -> bool {
            f32::from(*self).is_finite()
        }
    }

    #[test]
    fn opacity_is_bounded_and_only_matters_when_on() {
        let mut bg = Background::default();
        assert!(!bg.is_on());
        assert_eq!(bg.pane_alpha(), 1.);
        assert_eq!(bg.veil_alpha(), 1.);
        bg.panes = 10;
        bg.sidebar = 0;
        // Off: still opaque, whatever the sliders say.
        assert_eq!(bg.pane_alpha(), 1.);
        bg.picture = true;
        assert_eq!(bg.pane_alpha(), f32::from(MIN_OPACITY) / 100.);
        assert_eq!(bg.veil_alpha(), f32::from(MIN_OPACITY) / 100.);
        let c = bg.clamped();
        assert_eq!((c.panes, c.sidebar), (MIN_OPACITY, MIN_OPACITY));
        // A material with no picture: the tint is the veil.
        let m = Background {
            material: Material::Mica,
            tint: 50,
            ..Background::default()
        };
        assert!((m.veil_alpha() - 0.5).abs() < 1e-6);
        // The tint has the same floor: text never sits on bare material.
        let bare = Background { tint: 0, ..m };
        assert_eq!(bare.veil_alpha(), f32::from(MIN_OPACITY) / 100.);
    }

    #[test]
    fn the_safe_opacity_follows_the_picture() {
        let (paper, muted) = (Rgb(0xFFFFFF), Rgb(0x6A6A66));
        // A white picture: any opacity reads.
        assert_eq!(safe_opacity(paper, muted, &[Rgb::WHITE]), MIN_OPACITY);
        // A black one: the pane must be nearly opaque.
        let dark = safe_opacity(paper, muted, &[Rgb::BLACK]);
        assert!(dark > 80 && dark <= 100, "{dark}");
        // Darker grounds need more.
        let mid = safe_opacity(paper, muted, &[Rgb(0x808080)]);
        assert!(mid < dark);
        // The check is the worst of the grounds.
        assert_eq!(safe_opacity(paper, muted, &[Rgb::WHITE, Rgb::BLACK]), dark);
    }

    /// A pane is checked against the part of the picture behind it.
    #[test]
    fn the_tone_grid_reads_only_what_is_under_a_pane() {
        let t = |v: u32| {
            let c = Rgb(v);
            Some(Tones {
                average: c,
                darkest: c,
                lightest: c,
            })
        };
        // Left half black, right half white, a clear cell in a corner.
        let grid = ToneGrid {
            cols: 2,
            rows: 2,
            cells: vec![t(0x000000), t(0xFFFFFF), t(0x000000), None],
        };
        assert_eq!(
            grid.under((0., 0.), (0.4, 1.)),
            Some([Rgb::BLACK, Rgb::BLACK])
        );
        assert_eq!(
            grid.under((0.6, 0.), (1., 0.4)),
            Some([Rgb::WHITE, Rgb::WHITE])
        );
        assert_eq!(
            grid.under((0., 0.), (1., 1.)),
            Some([Rgb::BLACK, Rgb::WHITE])
        );
        assert_eq!(grid.under((0.6, 0.6), (1., 1.)), None);
        // So the floor is local: a pane over the black half needs less
        // than one across both halves.
        let (paper, muted) = (Rgb(0xFFFFFF), Rgb(0x707070));
        let left = safe_opacity(paper, muted, &grid.under((0., 0.), (0.4, 1.)).unwrap());
        let all = safe_opacity(paper, muted, &grid.under((0., 0.), (1., 1.)).unwrap());
        assert!(left <= all);
    }

    #[test]
    fn the_snapshot_switch_reads_its_pairs() {
        let bg =
            parse_switch("fit=tile, dim=20,blur=12,panes=70,paneblur=24,material=acrylic,style=ascii,cell=8,vignette=50").unwrap();
        assert_eq!(
            (bg.fit, bg.dim, bg.blur, bg.panes, bg.pane_blur, bg.material),
            (Fit::Tile, 20, 12, 70, 24, Material::Acrylic)
        );
        assert_eq!((bg.style, bg.cell, bg.vignette), (Style::Ascii, 8, 50));
        assert!(parse_switch("style=oil").is_err());
        assert!(parse_switch("blur=99").is_err());
        assert!(parse_switch("hue=3").is_err());
        assert!(parse_switch("fit").is_err());
        assert_eq!(parse_switch("").unwrap(), Background::default());
    }

    #[test]
    fn settings_round_trip_and_are_lenient() {
        let bg = Background {
            picture: true,
            fit: Fit::Tile,
            dim: 20,
            blur: 12,
            panes: 70,
            sidebar: 80,
            pane_blur: 24,
            material: Material::Acrylic,
            tint: 40,
            style: Style::PixelArt,
            cell: 9,
            colours: 32,
            dither: false,
            ascii_ink: AsciiInk::Accent,
            intensity: 80,
            vignette: 70,
            vignette_size: 55,
        };
        assert_eq!(Background::from_json(&bg.to_json()), bg);
        // Missing fields take their defaults; junk is the default.
        let partial = Background::from_json(&json!({"dim": 30, "fit": "sideways"}));
        assert_eq!(partial.fit, Fit::Fill);
        assert_eq!(Background::from_json(&json!("nope")), Background::default());
        assert_eq!(partial.dim, 30);
        assert_eq!(partial.panes, 100);
        assert_eq!(Fit::from_key("centre"), Some(Fit::Centre));
        assert_eq!(Material::from_key("mica"), Some(Material::Mica));
        // Out-of-range style values come back in range.
        let odd = Background::from_json(&json!({"cell": 99, "colours": 20, "vignette": 300}));
        assert_eq!((odd.cell, odd.colours, odd.vignette), (MAX_CELL, 16, 100));
    }
}
