//! The Rest scene: a misty lake that fills the calm panel, edge to edge,
//! with the calm line in its open sky, when nothing is unread.
//!
//! Two drawn bands per scheme (day for light, night for dark), RGBA, drawn
//! as they are: the far ridges (`assets/rest-*-dist.png`, 240 x 100, its
//! far waterline on the horizon, centred and repeated across a wide panel)
//! and the near shore (`assets/rest-*-fore.png`, 240 x 240, pines, jetty,
//! boat and reeds, anchored to the bottom left; cropped on the right, or
//! shifted left to keep the jetty's end in a narrow panel). Everything else
//! is drawn here in the scene's greys (`theme::scene`): the sky (stars at
//! night in the dark scheme, a few thin clouds otherwise), the sun or moon
//! on its arc behind the ridges (from the local clock, the moon in its real
//! phase), the water and the body's path on it, the fireflies, the lantern
//! and the birds. `sky.rs` says where, as pure functions.
//!
//! Every art pixel is the whole number of device pixels closest to 2
//! logical px (`layout::scene_scale`), on the device grid from the panel's
//! bottom left corner: the bands are enlarged once on the CPU (nearest
//! neighbour) and painted 1:1, the rest is quads. The still art keeps to
//! the art grid; the few moving lights (the body, the fireflies, the birds,
//! the mist) sit on the nearest device pixel, so they creep a device pixel
//! at a time, never an art pixel at once. The panel's own paper is the
//! ground; its rounded corners clip the scene.
//!
//! Motion (`sky.rs`): calm by design, a pure function of the time in
//! seconds; nothing moves a device pixel or changes its light by more than
//! a few hundredths from one frame to the next. The water glimmers in
//! place, it does not flow, and mirrors the ridges, the jetty and the boat
//! (`sky.rs`, reflections): their reflections' rows are built once per
//! scheme, scale and size ([`mirror_rows`], cached as images); a frame only
//! shifts each row by its slow sway and covers its calm gaps.
//!
//! Cost: [`RestScene`] is its own view, embedded cached, so a frame of it
//! redraws the scene alone. It draws 20 frames a second
//! (`motion::SCENE_FRAME_MS`) on the window's frame callbacks (so frames
//! keep to the display's refreshes), only while it is painted in the
//! active window and motion is allowed; under reduce motion it is redrawn
//! once a minute (the body moves) and everything else holds still. A
//! refresh that finds the last frame asked for was not painted (hidden,
//! minimised, another view) ends the run, so nothing runs while it is out
//! of sight.
//! Decorative: only the line has a name.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui::{
    Bounds, Context, Corners, Hsla, IntoElement, ParentElement as _, Pixels, Render, RenderImage,
    Role, SharedString, Size, Styled, Subscription, WeakEntity, Window, canvas, div, fill, point,
    prelude::*, px, size,
};
use gpui_kit::TestSupportExt as _;

use super::sky::{self, Body};
use crate::theme::scene::{BodyTone, SceneTones, tones};
use crate::theme::{ActiveTokens as _, Scheme, TextStyled as _, Tokens, layout, motion, radius};

/// The bands, built into the exe.
const DAY_DIST: &[u8] = include_bytes!("../../../assets/rest-day-dist.png");
const DAY_FORE: &[u8] = include_bytes!("../../../assets/rest-day-fore.png");
const NIGHT_DIST: &[u8] = include_bytes!("../../../assets/rest-night-dist.png");
const NIGHT_FORE: &[u8] = include_bytes!("../../../assets/rest-night-fore.png");

// ------------------------------------------------------------ the bands

/// A decoded picture: straight RGBA rows, top down.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Master {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Master {
    /// The alpha of pixel `(x, y)`.
    #[cfg(test)]
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
        self.rgba[((y * self.width + x) * 4 + 3) as usize]
    }
}

/// Decode a band into straight RGBA (any PNG colour type).
pub fn decode(bytes: &[u8]) -> Result<Master, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (width, height) = (info.width, info.height);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        let line = &buf[y * info.line_size..];
        for x in 0..width as usize {
            let px = match info.color_type {
                png::ColorType::Grayscale => [line[x], line[x], line[x], 255],
                png::ColorType::GrayscaleAlpha => {
                    let (g, a) = (line[x * 2], line[x * 2 + 1]);
                    [g, g, g, a]
                }
                png::ColorType::Rgb => [line[x * 3], line[x * 3 + 1], line[x * 3 + 2], 255],
                png::ColorType::Rgba => [
                    line[x * 4],
                    line[x * 4 + 1],
                    line[x * 4 + 2],
                    line[x * 4 + 3],
                ],
                png::ColorType::Indexed => return Err("indexed colour was not expanded".into()),
            };
            rgba.extend(px);
        }
    }
    Ok(Master {
        width,
        height,
        rgba,
    })
}

/// Which band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Band {
    Dist,
    Fore,
}

/// A band for a scheme, decoded on first use (empty if it breaks the
/// contract: its test fails first).
fn band(scheme: Scheme, band: Band) -> &'static Master {
    static CELLS: [OnceLock<Master>; 4] = [const { OnceLock::new() }; 4];
    let dark = usize::from(scheme == Scheme::Dark);
    let (dist, fore) = match scheme {
        Scheme::Light => (DAY_DIST, DAY_FORE),
        Scheme::Dark => (NIGHT_DIST, NIGHT_FORE),
    };
    match band {
        Band::Dist => CELLS[dark].get_or_init(|| decode(dist).unwrap_or_default()),
        Band::Fore => CELLS[2 + dark].get_or_init(|| decode(fore).unwrap_or_default()),
    }
}

/// Columns `x0..x0 + w` of `master` enlarged `n` times, nearest neighbour,
/// as the straight BGRA a `RenderImage` holds.
fn enlarge(master: &Master, n: u32, x0: u32, w: u32) -> Option<Arc<RenderImage>> {
    let w = w.min(master.width.saturating_sub(x0));
    if w == 0 || master.height == 0 || n == 0 {
        return None;
    }
    let (ow, oh) = (w * n, master.height * n);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        let src = ((y / n) * master.width) as usize * 4;
        let row = (y * ow) as usize * 4;
        for x in 0..ow {
            let s = src + (x0 + x / n) as usize * 4;
            let d = row + x as usize * 4;
            let p = &master.rgba[s..s + 4];
            out[d..d + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    let buf = image::RgbaImage::from_raw(ow, oh, out)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

// ------------------------------------------------------------ the body

/// The moon's craters: (dx, dy, radius) from its centre.
const CRATERS: [(i32, i32, i32); 3] = [(-4, -3, 2), (4, 3, 2), (3, -6, 1)];

/// The halos fall off in this many steps.
const HALO_STEPS: f32 = 4.;

/// The lantern's reflection at its strongest.
const LAMP_GLINT: f32 = 0.85;

/// A firefly's glow: its four neighbours, and fainter corners (the dark
/// scheme's only).
const GLOW: [(i32, i32, f32); 8] = [
    (-1, 0, 1.),
    (1, 0, 1.),
    (0, -1, 1.),
    (0, 1, 1.),
    (-1, -1, 0.5),
    (1, -1, 0.5),
    (-1, 1, 0.5),
    (1, 1, 0.5),
];

/// How finely the moon's phase is drawn (steps a month).
const PHASE_STEPS: f32 = 64.;

/// The body's picture: its disc (the moon lit by `phase`, its dark limb
/// in `limb`) inside its halo, `2 * (r + reach) + 1` pixels square.
fn body_master(body: Body, phase: f32, tone: &BodyTone, limb: Hsla) -> Master {
    let r = match body {
        Body::Sun => sky::SUN_R,
        Body::Moon => sky::MOON_R,
    };
    // A thin moon has a fainter halo.
    let light = match body {
        Body::Sun => 1.,
        Body::Moon => sky::moon_light(phase).max(0.2),
    };
    let reach = tone.halo_reach;
    let half = r + reach;
    let side = (2 * half + 1) as u32;
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    let bytes = |c: Hsla, a: f32| {
        let c = gpui::Rgba::from(c);
        let v = |f: f32| (f * 255.).round().clamp(0., 255.) as u8;
        [v(c.r), v(c.g), v(c.b), v(a)]
    };
    for dy in -half..=half {
        for dx in -half..=half {
            let px = if sky::on_disc(dx, dy, r) {
                let lit = match body {
                    Body::Sun => true,
                    Body::Moon => sky::moon_lit(dx, dy, r, phase),
                };
                if !lit {
                    bytes(limb, 1.)
                } else if body == Body::Moon
                    && CRATERS
                        .iter()
                        .any(|&(cx, cy, cr)| sky::on_disc(dx - cx, dy - cy, cr - 1))
                {
                    bytes(tone.crater, 1.)
                } else {
                    bytes(tone.disc, 1.)
                }
            } else {
                let d = ((dx * dx + dy * dy) as f32).sqrt() - r as f32;
                // A soft falloff in a few even steps (pixel art, no blur).
                let k = (1. - d / reach as f32).clamp(0., 1.);
                let step = (k * HALO_STEPS).ceil() / HALO_STEPS;
                bytes(tone.halo, tone.halo.a * light * step.powf(1.5))
            };
            rgba.extend(px);
        }
    }
    Master {
        width: side,
        height: side,
        rgba,
    }
}

// ------------------------------------------------------------ reflections

/// A reflection row reaches this many art pixels past each side of the
/// scene, so its sideways shift never shows an edge.
const REFLECT_PAD: i32 = 3;

/// How a scheme shows its reflections: their strength times `gain`, their
/// colour lifted towards white by `lift` (the dark scheme's dark greys sit
/// close to its paper, so their mirror images need a little more to read
/// as a lighter band broken by rows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub gain: f32,
    pub lift: f32,
}

impl Look {
    pub fn of(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Light => Look { gain: 1., lift: 0. },
            Scheme::Dark => Look {
                gain: 1.35,
                lift: 0.07,
            },
        }
    }
}

/// A reflection is never stronger than this.
const REFLECT_MOST: f32 = 0.72;

/// One reflection's rows (`sky.rs`, reflections): row `k` is the source
/// row it mirrors about the source's waterline row `line`, its alpha
/// scaled by the row's strength (so it lies on the paper fainter, in lower
/// contrast), one art pixel tall and `w + 2 * REFLECT_PAD` wide (its column
/// 0 is the scene's column `-REFLECT_PAD`); `None` for a row with nothing
/// in it. `column` maps a scene column on reflection row `k` to the
/// source's (`None`: nothing, or land covers the water there). `look`
/// sets how the scheme shows it.
pub fn mirror_rows(
    src: &Master,
    line: i32,
    depth: i32,
    w: i32,
    look: Look,
    column: impl Fn(i32, i32) -> Option<u32>,
) -> Vec<Option<Master>> {
    let width = (w + 2 * REFLECT_PAD).max(0) as u32;
    (0..depth)
        .map(|k| {
            let row = line - 1 - sky::mirror_row(k);
            let strength = (sky::reflect_strength(k, depth) * look.gain).min(REFLECT_MOST);
            if row < 0 || row >= src.height as i32 || strength <= 0. {
                return None;
            }
            let mut rgba = vec![0u8; width as usize * 4];
            let mut any = false;
            for i in 0..width as usize {
                let Some(c) = column(i as i32 - REFLECT_PAD, k).filter(|c| *c < src.width) else {
                    continue;
                };
                let p = ((row as u32 * src.width + c) * 4) as usize;
                let a = (f32::from(src.rgba[p + 3]) * strength).round() as u8;
                if a > 0 {
                    any = true;
                    for ch in 0..3 {
                        let v = f32::from(src.rgba[p + ch]);
                        rgba[i * 4 + ch] = (v + (255. - v) * look.lift).round() as u8;
                    }
                    rgba[i * 4 + 3] = a;
                }
            }
            any.then_some(Master {
                width,
                height: 1,
                rgba,
            })
        })
        .collect()
}

/// A reflection's rows sway and break in bands of this many rows (one
/// picture, one shift and one gap a band: half the drawing of single rows,
/// and the pixel-art water's look).
const REFLECT_BAND: usize = 2;

/// A calm gap's end pixels hide this share of what its middle hides.
const GAP_TAPER: f32 = 0.4;

/// Rows joined into bands of [`REFLECT_BAND`] rows (a missing row clear;
/// a band with nothing in it `None`). Band `j` holds rows `j * REFLECT_BAND`
/// and on.
pub fn bands(rows: Vec<Option<Master>>) -> Vec<Option<Master>> {
    rows.chunks(REFLECT_BAND)
        .map(|chunk| {
            let width = chunk.iter().flatten().map(|m| m.width).max()?;
            let mut rgba = Vec::with_capacity(width as usize * 4 * chunk.len());
            for row in chunk {
                match row {
                    Some(m) if m.width == width => rgba.extend_from_slice(&m.rgba),
                    _ => rgba.extend(std::iter::repeat_n(0, width as usize * 4)),
                }
            }
            Some(Master {
                width,
                height: chunk.len() as u32,
                rgba,
            })
        })
        .collect()
}

/// Whether the near band's own pixel (`c`, `row`) is land (anything drawn
/// there: the reflection only shows on open water, never over the shore).
pub fn on_land(fore: &Master, c: u32, row: i32) -> bool {
    if c >= fore.width || row < 0 || row >= fore.height as i32 {
        return false;
    }
    fore.rgba[((row as u32 * fore.width + c) * 4 + 3) as usize] > 0
}

/// The far band's column under scene column `x` (centred and repeated, as
/// it is drawn).
fn dist_column(w: i32, x: i32) -> u32 {
    let mut x0 = (w - sky::DIST_W) / 2;
    while x0 > 0 {
        x0 -= sky::DIST_W;
    }
    (x - x0).rem_euclid(sky::DIST_W) as u32
}

/// A reflection's bands, enlarged.
type Rows = Rc<Vec<Option<Arc<RenderImage>>>>;

/// What the reflections in use were built for: scheme, scale, scene size.
type ReflectKey = (Scheme, u32, i32, i32);

thread_local! {
    static REFLECTIONS: RefCell<Option<(ReflectKey, Rows, Rows)>> = const { RefCell::new(None) };
}

/// The far and the near reflections' rows for a scene, built once per
/// scheme, scale and size (the old ones are dropped from the window's
/// sprite atlas); a frame only shifts and covers them.
fn reflections(scheme: Scheme, n: u32, l: &sky::Layout, window: &mut Window) -> (Rows, Rows) {
    let key = (scheme, n, l.w, l.h);
    REFLECTIONS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((k, far, near)) = slot.as_ref()
            && *k == key
        {
            return (far.clone(), near.clone());
        }
        let enlarged = |rows: Vec<Option<Master>>| -> Rows {
            Rc::new(
                bands(rows)
                    .into_iter()
                    .map(|m| m.and_then(|m| enlarge(&m, n, 0, m.width)))
                    .collect(),
            )
        };
        let look = Look::of(scheme);
        let far = enlarged(mirror_rows(
            band(scheme, Band::Dist),
            sky::DIST_WATERLINE,
            l.far_reflection().depth,
            l.w,
            look,
            |x, _| Some(dist_column(l.w, x)),
        ));
        let fore = band(scheme, Band::Fore);
        let fore_x = l.fore_x;
        let near = enlarged(mirror_rows(
            fore,
            sky::FORE_WATERLINE,
            l.near_reflection().depth,
            l.w,
            look,
            |x, k| {
                let c = u32::try_from(x - fore_x).ok()?;
                (!on_land(fore, c, sky::FORE_WATERLINE + k)).then_some(c)
            },
        ));
        if let Some((_, old_far, old_near)) = slot.replace((key, far.clone(), near.clone())) {
            for img in old_far.iter().chain(old_near.iter()).flatten() {
                window.drop_image(img.clone()).ok();
            }
        }
        (far, near)
    })
}

// ------------------------------------------------------------ images

/// What an enlarged picture was made from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Picture {
    Band(Scheme, Band, u32, u32, u32),
    /// Scheme, body, scale, phase, and the limb's colour (the panel's,
    /// `0xRRGGBB`).
    Body(Scheme, Body, u32, u32, u32),
    /// The night's still stars (those that do not twinkle): scale, scene
    /// size, line height, seed. The dark scheme's only.
    Stars(u32, i32, i32, i32, u64),
    /// The lantern's outer glow at its middle strength: scheme, scale.
    Glow(Scheme, u32),
}

/// The pictures in use, one per slot (dist, fore, body, stars, glow); a
/// replaced one is dropped from the window's sprite atlas.
type Slot = Option<(Picture, Arc<RenderImage>)>;

/// What the cached stars were made for: seed and layout.
type StarsKey = (u64, sky::Layout);

thread_local! {
    static PICTURES: RefCell<[Slot; 5]> =
        const { RefCell::new([None, None, None, None, None]) };
    /// The night's stars for a seed and layout (made once, not each frame).
    static STARS: RefCell<Option<(StarsKey, Rc<Vec<sky::Spark>>)>> =
        const { RefCell::new(None) };
}

/// The stars for a seed and layout, made once.
fn stars_for(seed: u64, l: &sky::Layout) -> Rc<Vec<sky::Spark>> {
    STARS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((key, stars)) = slot.as_ref()
            && *key == (seed, *l)
        {
            return stars.clone();
        }
        let stars = Rc::new(sky::stars(seed, l));
        *slot = Some(((seed, *l), stars.clone()));
        stars
    })
}

/// The lantern's glow inside this reach (art pixels) swells with its
/// light, drawn each frame; the rest is a picture at its middle strength.
const LAMP_CORE: f32 = 2.5;
/// The lantern's middle strength (`sky::lantern` swells 0.81 to 0.99).
const LAMP_MID: f32 = 0.9;

/// The lantern glow's alpha at `(dx, dy)` from the lamp, at full light
/// (0 at the lamp itself and beyond its reach).
fn lamp_glow(dx: i32, dy: i32, tone: Hsla) -> f32 {
    let reach = sky::LAMP_GLOW as f32;
    let d = ((dx * dx + dy * dy) as f32).sqrt();
    if d == 0. || d > reach {
        return 0.;
    }
    let k = 1. - d / reach;
    tone.a * k * k
}

/// A picture of straight RGBA `w` x `h` in one colour, from each pixel's
/// alpha.
fn tinted(w: u32, h: u32, c: Hsla, alpha: impl Fn(i32, i32) -> f32) -> Master {
    let c = gpui::Rgba::from(c);
    let v = |f: f32| (f * 255.).round().clamp(0., 255.) as u8;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            rgba.extend([v(c.r), v(c.g), v(c.b), v(alpha(x, y))]);
        }
    }
    Master {
        width: w,
        height: h,
        rgba,
    }
}

fn picture(slot: usize, key: Picture, window: &mut Window) -> Option<Arc<RenderImage>> {
    PICTURES.with(|cell| {
        let mut slots = cell.borrow_mut();
        if let Some((k, image)) = &slots[slot]
            && *k == key
        {
            return Some(image.clone());
        }
        let made = match key {
            Picture::Band(scheme, b, n, x0, w) => enlarge(band(scheme, b), n, x0, w),
            Picture::Body(scheme, body, n, phase, limb) => {
                let t = tones(scheme);
                let limb = crate::theme::presets::Rgb(limb).colour();
                let tone = match body {
                    Body::Sun => t.sun,
                    Body::Moon => t.moon,
                };
                let m = body_master(body, phase as f32 / PHASE_STEPS, &tone, limb);
                enlarge(&m, n, 0, m.width)
            }
            Picture::Stars(n, w, h, line_h, seed) => {
                let l = sky::layout(w, h, line_h);
                let stars = stars_for(seed, &l);
                let star = tones(Scheme::Dark).star;
                let mut still = vec![0f32; (w.max(1) * l.horizon.max(1)) as usize];
                for (i, s) in stars.iter().enumerate() {
                    if !sky::twinkle(seed, i, 0., false).live
                        && (0..w).contains(&s.x)
                        && (0..l.horizon).contains(&s.y)
                    {
                        still[(s.y * w + s.x) as usize] = s.a;
                    }
                }
                let m = tinted(w.max(1) as u32, l.horizon.max(1) as u32, star, |x, y| {
                    still[(y * w + x) as usize] * star.a
                });
                enlarge(&m, n, 0, m.width)
            }
            Picture::Glow(scheme, n) => {
                let tone = tones(scheme).lamp_glow;
                let reach = sky::LAMP_GLOW;
                let side = (2 * reach + 1) as u32;
                let m = tinted(side, side, tone, |x, y| {
                    let (dx, dy) = (x - reach, y - reach);
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    if d <= LAMP_CORE {
                        0.
                    } else {
                        // As the dots draw it: the tone's own alpha twice.
                        tone.a * lamp_glow(dx, dy, tone) * LAMP_MID
                    }
                });
                enlarge(&m, n, 0, m.width)
            }
        }?;
        if let Some((_, old)) = slots[slot].replace((key, made.clone())) {
            window.drop_image(old).ok();
        }
        Some(made)
    })
}

// ------------------------------------------------------------ a frame

/// Everything one frame of the scene needs.
#[derive(Clone, Copy, Debug)]
struct Frame {
    scheme: Scheme,
    tones: SceneTones,
    /// Device pixels per art pixel.
    n: u32,
    line_h: i32,
    /// The local minute of the day, with its fraction.
    minute: f32,
    unix: i64,
    /// Seconds into the motion, and whether it holds still.
    t: f64,
    still: bool,
    seed: u64,
}

/// The scene's size in art pixels for a panel `panel` logical px.
fn art_size(panel: Size<Pixels>, scale_factor: f32, n: u32) -> (i32, i32) {
    let art = |v: Pixels| (v.as_f32() * scale_factor / n as f32).ceil() as i32;
    (art(panel.width), art(panel.height))
}

/// Paints one frame in `bounds` (the panel).
fn paint(bounds: Bounds<Pixels>, f: &Frame, window: &mut Window) {
    let sf = window.scale_factor();
    let n = f.n as f32;
    let (w, h) = art_size(bounds.size, sf, f.n);
    let l = sky::layout(w, h, f.line_h);
    // The art grid in device pixels, hung from the panel's bottom left.
    let left = (bounds.origin.x.as_f32() * sf).round();
    let top = ((bounds.origin.y + bounds.size.height).as_f32() * sf).round() - h as f32 * n;
    // An art position on the device grid: a whole art pixel lands on the
    // art grid, a fractional one (a moving light) on the nearest device
    // pixel, so it creeps a device pixel at a time.
    let dev = |x: f32, y: f32| ((left + x * n).round(), (top + y * n).round());
    let area = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let (a, b) = dev(x0, y0);
        let (c, d) = dev(x1, y1);
        Bounds::new(
            point(px(a / sf), px(b / sf)),
            size(px((c - a) / sf), px((d - b) / sf)),
        )
    };
    let dot = |window: &mut Window, x: f32, y: f32, c: Hsla| {
        if c.a > 0.004 {
            window.paint_quad(fill(area(x, y, x + 1., y + 1.), c));
        }
    };
    let run = |window: &mut Window, s: &sky::Streak, c: Hsla| {
        let (x0, x1) = (s.x.max(0.), (s.x + s.len as f32).min(w as f32));
        if x1 > x0 && s.a > 0.004 {
            let y = s.y as f32;
            window.paint_quad(fill(area(x0, y, x1, y + 1.), c.opacity(s.a)));
        }
    };
    let draw =
        |window: &mut Window, x: f32, y: f32, img: Arc<RenderImage>, corners: Corners<Pixels>| {
            const FIRST: usize = 0;
            let s = img.size(FIRST);
            let (a, b) = dev(x, y);
            let area = Bounds::new(
                point(px(a / sf), px(b / sf)),
                size(px(s.width.0 as f32 / sf), px(s.height.0 as f32 / sf)),
            );
            window.paint_image(area, area, corners, img, 0, false).ok();
        };
    let t = &f.tones;
    let (body, progress) = sky::body_at(f.minute);
    let (bx, by) = sky::arc(progress, &l);
    let night = body == Body::Moon;
    let horizon = l.horizon as f32;

    // The sky.
    let starry = f.scheme == Scheme::Dark && night;
    let stars = if starry {
        stars_for(f.seed, &l)
    } else {
        Rc::default()
    };
    if starry {
        // The still three quarters as one picture, the twinkling ones as
        // dots.
        let key = Picture::Stars(f.n, w, h, f.line_h, f.seed);
        if let Some(img) = picture(3, key, window) {
            draw(window, 0., 0., img, Corners::default());
        }
        for (i, s) in stars.iter().enumerate() {
            let tw = sky::twinkle(f.seed, i, f.t, f.still);
            if tw.live {
                let a = s.a.max(sky::TWINKLE_FLOOR) * tw.light;
                dot(window, s.x as f32, s.y as f32, t.star.opacity(a));
            }
        }
    } else {
        for c in sky::clouds(f.seed, &l) {
            run(window, &c, t.cloud);
        }
    }

    // The body, behind the ridges and hidden under the horizon.
    let (tone, r) = match body {
        Body::Sun => (t.sun, sky::SUN_R),
        Body::Moon => (t.moon, sky::MOON_R),
    };
    let up = by < horizon;
    if by - (r as f32) < horizon {
        let phase = (sky::moon_phase(f.unix) * PHASE_STEPS).round() as u32 % PHASE_STEPS as u32;
        let limb = crate::theme::presets::Rgb::from_colour(f.tones.limb).0;
        let key = Picture::Body(f.scheme, body, f.n, if night { phase } else { 0 }, limb);
        if let Some(img) = picture(2, key, window) {
            let half = (r + tone.halo_reach) as f32;
            let sky_rect = area(0., 0., w as f32, horizon);
            window.with_content_mask(Some(gpui::ContentMask { bounds: sky_rect }), |window| {
                draw(window, bx - half, by - half, img, Corners::default());
            });
        }
    }

    // The far ridges, centred and repeated.
    if let Some(img) = picture(
        0,
        Picture::Band(f.scheme, Band::Dist, f.n, 0, sky::DIST_W as u32),
        window,
    ) {
        let mut x = (w - sky::DIST_W) / 2;
        while x > 0 {
            x -= sky::DIST_W;
        }
        while x < w {
            draw(
                window,
                x as f32,
                l.dist_y as f32,
                img.clone(),
                Corners::default(),
            );
            x += sky::DIST_W;
        }
    }
    // By day, mist drifting along the far shore.
    if !night {
        for s in sky::mist(f.seed, f.t, f.still, &l) {
            run(window, &s, t.mist);
        }
    }

    // The water: a clean row under the horizon, then the reflections
    // (each band of rows shifted a little and swaying, broken by calm gaps
    // that fade in and out), the glimmers, the body's path (its reflection),
    // the bright stars', the lantern's and the fireflies' glints. The near
    // shore covers what falls on land.
    let (far, near) = reflections(f.scheme, f.n, &l, window);
    for (rows, r, id) in [
        (&far, l.far_reflection(), 0u64),
        (&near, l.near_reflection(), 1000),
    ] {
        for (j, img) in rows.iter().enumerate() {
            let Some(img) = img else {
                continue;
            };
            let (band, k) = (id + j as u64, (j * REFLECT_BAND) as i32);
            let off = sky::ripple_offset(f.seed, band, k, r.depth, f.t, f.still);
            let y = (r.top + k) as f32;
            draw(
                window,
                off - REFLECT_PAD as f32,
                y,
                img.clone(),
                Corners::default(),
            );
            // A calm gap: one row of the band (which one, its own), its
            // ends tapered, so it reads as still water, not a bar.
            let gap = sky::calm_gap(f.seed, band, w, f.t, f.still);
            if gap.a > 0.004 {
                let tall = (REFLECT_BAND as i32).min(r.depth - k).max(1);
                let gy = y + (gap.x as i32).rem_euclid(tall) as f32;
                let (x, len) = (gap.x, gap.len as f32);
                for (from, to, share) in [
                    (x, x + 1., GAP_TAPER),
                    (x + 1., x + len - 1., 1.),
                    (x + len - 1., x + len, GAP_TAPER),
                ] {
                    let (x0, x1) = (from.max(0.), to.min(w as f32));
                    if x1 > x0 {
                        let c = t.limb.opacity(gap.a * share);
                        window.paint_quad(fill(area(x0, gy, x1, gy + 1.), c));
                    }
                }
            }
        }
    }
    for s in sky::water(f.seed, f.t, f.still, &l) {
        run(window, &s, t.water);
    }
    if up {
        let light = match body {
            Body::Sun => 1.,
            Body::Moon => sky::moon_light(sky::moon_phase(f.unix)).max(0.3),
        };
        // The column under the body. On paper (the light scheme) it is
        // darker than the water, and every third row over the ridges'
        // reflection is the paper itself: a soft lighter and darker
        // shimmer through it. In the dark it is lighter than the water.
        let far = l.far_reflection();
        let paper = f.scheme == Scheme::Light;
        for s in sky::path(f.seed, bx, f.t, f.still, &l) {
            let lift = paper && (s.y - far.top) % 3 == 1 && s.y < far.top + far.depth;
            let c = if lift { t.limb } else { tone.path };
            run(window, &s, c.opacity(tone.path.a * light));
        }
    }
    for (i, g) in sky::star_glints(&stars, &l) {
        let light = sky::twinkle(f.seed, i, f.t, f.still).light;
        dot(window, g.x as f32, g.y as f32, t.star.opacity(g.a * light));
    }
    let flies = if night {
        sky::fireflies(f.seed, f.t, f.still, f.scheme == Scheme::Dark, &l)
    } else {
        Vec::new()
    };
    if night {
        let glow = sky::lantern(f.t, f.still);
        for s in sky::lamp_glints(f.seed, f.t, f.still, &l) {
            run(window, &s, t.lamp.opacity(LAMP_GLINT * glow));
        }
        for g in flies.iter().filter_map(|s| sky::firefly_glint(s, &l)) {
            dot(window, g.x, g.y, t.firefly.opacity(g.a));
        }
    }

    // The near shore, from the panel's bottom left corner.
    let crop = (-l.fore_x).max(0) as u32;
    let in_panel = ((bounds.size.width.as_f32() * sf) as u32 / f.n).max(1);
    let shown = (sky::FORE_W as u32 - crop).min(in_panel);
    if let Some(img) = picture(
        1,
        Picture::Band(f.scheme, Band::Fore, f.n, crop, shown),
        window,
    ) {
        let round = px(radius::SURFACE);
        let corners = Corners {
            bottom_left: round,
            bottom_right: if shown == in_panel {
                round
            } else {
                Pixels::ZERO
            },
            ..Corners::default()
        };
        draw(window, 0., l.fore_y as f32, img, corners);
    }

    // The night's lantern and fireflies; the day's birds.
    if night {
        let (lx, ly) = l.lamp();
        let glow = sky::lantern(f.t, f.still);
        let reach = sky::LAMP_GLOW;
        // The outer glow as a picture at its middle strength; the core
        // swells with the light.
        if let Some(img) = picture(4, Picture::Glow(f.scheme, f.n), window) {
            let at = (lx - reach) as f32;
            draw(window, at, (ly - reach) as f32, img, Corners::default());
        }
        let core = LAMP_CORE as i32;
        for dy in -core..=core {
            for dx in -core..=core {
                if ((dx * dx + dy * dy) as f32).sqrt() > LAMP_CORE {
                    continue;
                }
                let a = lamp_glow(dx, dy, t.lamp_glow) * glow;
                dot(
                    window,
                    (lx + dx) as f32,
                    (ly + dy) as f32,
                    t.lamp_glow.opacity(a),
                );
            }
        }
        dot(window, lx as f32, ly as f32, t.lamp.opacity(glow));
        // A firefly is a pixel in a soft glow (unlike a star), on and off
        // the pines in the dark scheme, by the shore and reeds only in the
        // light one.
        let dark = f.scheme == Scheme::Dark;
        for s in &flies {
            for (dx, dy, k) in GLOW {
                if k < 1. && !dark {
                    continue;
                }
                let a = t.firefly_glow.a * s.a * k;
                dot(
                    window,
                    s.x + dx as f32,
                    s.y + dy as f32,
                    t.firefly_glow.opacity(a),
                );
            }
            dot(window, s.x, s.y, t.firefly.opacity(s.a));
        }
    } else if !f.still {
        for (x, y) in sky::birds(f.seed, f.t, &l) {
            for (dx, dy) in sky::BIRD {
                dot(window, x + dx as f32, y + dy as f32, t.bird);
            }
        }
    }
}

// ------------------------------------------------------------ the view

/// A frame is asked for at the first display refresh at least this long
/// after the last one: 5 ms short of the frame, so the frames keep to
/// every third refresh at 60 Hz (every sixth at 120, every seventh at 144)
/// however the refreshes jitter.
const FRAME_SLACK_MS: u64 = 5;

/// The Rest scene and its line, as its own view. The calm panel embeds it
/// cached (`calm::rest`): a frame asked for here redraws the scene alone
/// (GPUI's in-place splice), the mail view and the window around it are
/// replayed.
pub struct RestScene {
    words: SharedString,
    /// The panel's width (its height comes from the last paint).
    width: f32,
    seed: u64,
    /// The motion's time (seconds) of the frame last drawn: the motion
    /// pauses while the scene is not running and goes on from here.
    shown: f64,
    /// While running: the executor's clock and the motion's time when the
    /// run began (the motion is a pure function of this time, so the frame
    /// rate never changes its speed).
    run_from: Option<(Instant, f64)>,
    /// A frame callback is waiting: the motion runs on the window's frames.
    ticking: bool,
    /// When the last frame was asked for, and whether it is not painted
    /// yet (the next refresh then ends the run: out of sight).
    asked_at: Option<Instant>,
    asked: bool,
    /// Frames asked for (tests).
    frames: u64,
    /// Bumped to end the once-a-minute task (reduce motion).
    epoch: u64,
    minute_task: bool,
    /// Set by each paint.
    painted: Rc<Cell<bool>>,
    /// The panel's size at the last paint.
    panel: Rc<Cell<Option<Size<Pixels>>>>,
    activation: Option<Subscription>,
}

impl Default for RestScene {
    fn default() -> Self {
        Self::new()
    }
}

impl RestScene {
    pub fn new() -> Self {
        // The same sky all day; a pinned one for snapshots.
        let seed = if sky::snapshot_clock().is_some() {
            1
        } else {
            (sky::now().1 / 86_400) as u64
        };
        Self {
            words: SharedString::default(),
            width: 0.,
            seed,
            shown: 0.,
            run_from: None,
            ticking: false,
            asked_at: None,
            asked: false,
            frames: 0,
            epoch: 0,
            minute_task: false,
            painted: Rc::new(Cell::new(false)),
            panel: Rc::new(Cell::new(None)),
            activation: None,
        }
    }

    /// The line and the panel's width (from the mail view's render). The
    /// scene is a cached view: a change redraws it.
    pub fn set(&mut self, words: SharedString, width: f32, cx: &mut Context<Self>) {
        if self.words != words || self.width != width {
            self.words = words;
            self.width = width;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.ticking
    }

    /// Waits for the window's next frame (a display refresh) to run
    /// [`Self::tick`]; the chain ends when a tick says so.
    fn tick_next(this: WeakEntity<Self>, window: &Window) {
        window.on_next_frame(move |window, cx| {
            let active = window.is_window_active() || sky::measuring();
            let again = this
                .update(cx, |scene, cx| scene.tick(active, cx))
                .unwrap_or(false);
            if again {
                Self::tick_next(this, window);
            }
        });
    }

    /// One display refresh while the motion runs: asks for a frame when one
    /// is due. Ends the run when the window is not active, or when the last
    /// frame asked for was not painted (the scene is out of sight: another
    /// view, a hidden panel, a minimised window).
    fn tick(&mut self, active: bool, cx: &mut Context<Self>) -> bool {
        if !self.ticking {
            return false;
        }
        if !active || (self.asked && !self.painted.get()) {
            self.ticking = false;
            self.run_from = None;
            return false;
        }
        self.asked = false;
        let now = cx.background_executor().now();
        let due = self.asked_at.is_none_or(|at| {
            now.duration_since(at) >= Duration::from_millis(motion::SCENE_FRAME_MS - FRAME_SLACK_MS)
        });
        if due {
            self.asked_at = Some(now);
            self.asked = true;
            self.painted.set(false);
            self.frames += 1;
            cx.notify();
        }
        true
    }

    /// Starts or ends the once-a-minute redraw (reduce motion: the body
    /// moves); it ends by itself when the scene was not painted since.
    fn minute(&mut self, wanted: bool, cx: &mut Context<Self>) {
        if self.minute_task == wanted {
            return;
        }
        self.epoch += 1;
        self.minute_task = wanted;
        if !wanted {
            return;
        }
        let epoch = self.epoch;
        self.painted.set(true);
        cx.spawn(async move |this, cx| {
            loop {
                let into =
                    (chrono::Local::now().timestamp_millis() as u64) % motion::SCENE_MINUTE_MS;
                let wait = Duration::from_millis(motion::SCENE_MINUTE_MS - into);
                cx.background_executor().timer(wait).await; // timer-exempt: the Rest scene's minute under reduce motion, only while it is painted in the active window
                let alive = this.update(cx, |scene, cx| {
                    if scene.epoch != epoch {
                        return false;
                    }
                    if !scene.painted.replace(false) {
                        scene.minute_task = false;
                        return false;
                    }
                    cx.notify();
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// The line's height in art pixels: one line, or two when it is wider
    /// than the room beside the pines (the layout is tried with one first).
    fn line_rows(&self, t: &Tokens, panel: Size<Pixels>, n: u32, window: &mut Window) -> i32 {
        let sf = window.scale_factor();
        let (w, h) = art_size(panel, sf, n);
        let line = t.text.message.line.as_f32() * sf / n as f32;
        let one = line.ceil() as i32;
        let l = sky::layout(w, h, one);
        let room = panel.width.as_f32() - l.line_left as f32 * n as f32 / sf;
        let wide = crate::widgets::text_width(&self.words, &t.text.message, window);
        if wide <= room - layout::SCENE_LINE_INSET - layout::SCENE_LINE_INSET {
            one
        } else {
            (line * 2.).ceil() as i32
        }
    }
}

impl Render for RestScene {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.activation = Some(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        }
        let t = cx.tokens().clone();
        // A snapshot holds motion still, or draws the one moment it pins
        // (`KOMOREBI_SNAPSHOT_CLOCK=HH:MM+S`).
        let pinned = sky::snapshot_seconds() > 0.;
        let still = cx.reduce_motion() && !pinned;
        let active = window.is_window_active() || sky::measuring();
        let moving = active && !still && !pinned;
        self.minute(active && still, cx);
        if moving && !self.ticking {
            self.ticking = true;
            self.asked = false;
            self.asked_at = None;
            Self::tick_next(cx.entity().downgrade(), window);
        }
        let time = if pinned {
            f64::from(sky::snapshot_seconds())
        } else if moving {
            let now = cx.background_executor().now();
            let (from, base) = *self.run_from.get_or_insert((now, self.shown));
            base + now.duration_since(from).as_secs_f64()
        } else {
            // Paused (the waiting frame callback ends the run).
            self.ticking = false;
            self.run_from = None;
            self.shown
        };
        self.shown = time;

        let sf = window.scale_factor();
        let panel = self.panel.get().unwrap_or_else(|| {
            let view = window.viewport_size();
            size(px(self.width), view.height - px(layout::SCENE_GUESS_CHROME))
        });
        let n = layout::scene_scale(sf, panel.width.as_f32() * sf, panel.height.as_f32() * sf);
        let line_h = self.line_rows(&t, panel, n, window);
        let (w, h) = art_size(panel, sf, n);
        let l = sky::layout(w, h, line_h);
        let u = n as f32 / sf;
        // The art grid hangs from the panel's bottom edge.
        let grid_top = panel.height.as_f32() - h as f32 * u;
        let line_top = grid_top + l.line_top as f32 * u;
        let line_left = l.line_left as f32 * u;

        let (minute, unix) = sky::now();
        let frame = Frame {
            // The panel's colour picks day or night art (a theme's panes
            // may be light in the dark scheme or the other way round).
            scheme: crate::theme::scene::art_scheme(t.color.paper),
            tones: crate::theme::scene::tones_on(t.color.paper),
            n,
            line_h,
            minute,
            unix,
            t: time,
            still,
            seed: self.seed,
        };
        let painted = self.painted.clone();
        let size_cell = self.panel.clone();
        div()
            .id("calm-rest")
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        painted.set(true);
                        if size_cell.get() != Some(bounds.size) {
                            size_cell.set(Some(bounds.size));
                            // The line follows the panel's height.
                            window.request_animation_frame();
                        }
                        paint(bounds, &frame, window);
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .id("calm-line")
                    .role(Role::Label)
                    .aria_label(self.words.clone())
                    .absolute()
                    .top(px(line_top))
                    .left(px(line_left))
                    .right_0()
                    .px(px(layout::SCENE_LINE_INSET))
                    .text_center()
                    .typo(&t.text.message)
                    .text_color(t.color.ink)
                    .child(self.words.clone())
                    .test_support(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bands keep the contract: their sizes, they decode, the far
    /// band's waterline row is solid across, and the near band's top
    /// corners are clear (it floats in the sky).
    #[test]
    fn the_bands_keep_the_contract() {
        for (name, bytes, (w, h)) in [
            ("day dist", DAY_DIST, (sky::DIST_W, sky::DIST_H)),
            ("night dist", NIGHT_DIST, (sky::DIST_W, sky::DIST_H)),
            ("day fore", DAY_FORE, (sky::FORE_W, sky::FORE_H)),
            ("night fore", NIGHT_FORE, (sky::FORE_W, sky::FORE_H)),
        ] {
            let m = decode(bytes).expect(name);
            assert_eq!((m.width as i32, m.height as i32), (w, h), "{name}");
            assert_eq!(m.alpha(0, 0), 0, "{name}: top left is clear");
            assert_eq!(m.alpha(m.width - 1, 0), 0, "{name}: top right is clear");
            if h == sky::DIST_H {
                let row = sky::DIST_WATERLINE as u32;
                assert!(
                    (0..m.width).all(|x| m.alpha(x, row) > 0),
                    "{name}: the waterline"
                );
                assert!(
                    (0..m.width).all(|x| m.alpha(x, row + 1) == 0),
                    "{name}: clear under it"
                );
            }
        }
        let fore = decode(NIGHT_FORE).unwrap();
        let (lx, ly) = sky::FORE_LAMP;
        assert!(fore.alpha(lx as u32, ly as u32 + 1) > 0, "the lantern post");
    }

    /// Runs on the window's frames, 20 a second at a 60 Hz refresh, only
    /// while painted in the active window; the motion's time follows the
    /// clock (not the frame count); nothing runs in an inactive window or
    /// under reduce motion, and the motion pauses there (it goes on from
    /// where it stopped, no jump).
    #[gpui::test]
    fn the_scene_runs_on_frames_only_while_seen(cx: &mut gpui::TestAppContext) {
        crate::widgets::test_support::init(cx, false);
        cx.update(|cx| cx.set_reduce_motion(false));
        let (scene, cx) = cx.add_window_view(|_, _| RestScene::new());
        scene.update(cx, |s, cx| s.set("Rest".into(), 380., cx));
        let frames = |cx: &mut gpui::VisualTestContext| scene.read_with(cx, |s, _| s.frames);
        let shown = |cx: &mut gpui::VisualTestContext| scene.read_with(cx, |s, _| s.shown);
        let refresh = |cx: &mut gpui::VisualTestContext, n: usize| {
            for _ in 0..n {
                cx.executor().advance_clock(Duration::from_micros(16_667));
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                });
                cx.run_until_parked();
            }
        };
        cx.run_until_parked();
        assert!(!scene.read_with(cx, |s, _| s.is_running()), "inactive");
        refresh(cx, 60);
        assert_eq!(frames(cx), 0);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(scene.read_with(cx, |s, _| s.is_running()), "active: runs");
        let before = (frames(cx), shown(cx));
        refresh(cx, 60);
        let ran = frames(cx) - before.0;
        assert!((19..=21).contains(&ran), "20 a second, got {ran}");
        let moved = shown(cx) - before.1;
        assert!(
            (0.9..=1.05).contains(&moved),
            "a second of motion, got {moved}"
        );
        // Reduce motion: still, and the time holds.
        cx.update(|_, cx| cx.set_reduce_motion(true));
        scene.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        refresh(cx, 3);
        assert!(!scene.read_with(cx, |s, _| s.is_running()), "still");
        let held = (frames(cx), shown(cx));
        cx.executor().advance_clock(Duration::from_secs(5));
        refresh(cx, 30);
        assert_eq!((frames(cx), shown(cx)), held, "held");
        // Motion again: it goes on from where it stopped.
        cx.update(|_, cx| cx.set_reduce_motion(false));
        scene.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        refresh(cx, 3);
        assert!(scene.read_with(cx, |s, _| s.is_running()), "runs again");
        let resumed = shown(cx) - held.1;
        assert!((0. ..0.1).contains(&resumed), "no jump: {resumed}");
    }

    #[test]
    fn an_art_pixel_is_about_two_logical_px() {
        // 1280 x 800 (panel 380 x 744): 2 device px at 100 percent, 3 at
        // 150, 4 at 200.
        let at = |sf: f32| layout::scene_scale(sf, 380. * sf, 744. * sf);
        assert_eq!((at(1.0), at(1.5), at(2.0)), (2, 3, 4));
        assert_eq!(layout::scene_scale(0.4, 152., 298.), 1, "one at least");
        // 1920 x 1080 maximised (panel 480 x 1024): 3, the scene 342 tall.
        assert_eq!(layout::scene_scale(1.0, 480., 1024.), 3);
        // A tall narrow panel (320 x 984): 3 would leave it 106 wide.
        assert_eq!(layout::scene_scale(1.0, 320., 984.), 2);
        // 1280 x 800 at 100 percent: the panel (380 x 744) is 190 x 372.
        assert_eq!(art_size(size(px(380.), px(744.)), 1.0, 2), (190, 372));
    }

    /// A reflection row mirrors the source row about the waterline
    /// (squashed), fainter with depth, and maps the scene's columns.
    #[test]
    fn reflections_mirror_fade_and_follow_the_columns() {
        // Four columns, six rows: row r's pixels are grey 10 * r, opaque;
        // the waterline is row 6 (under the picture).
        let src = Master {
            width: 4,
            height: 6,
            rgba: (0..6u8)
                .flat_map(|r| (0..4).flat_map(move |_| [10 * r, 10 * r, 10 * r, 255]))
                .collect(),
        };
        let plain = Look::of(Scheme::Light);
        let rows = mirror_rows(&src, 6, 5, 2, plain, |x, _| u32::try_from(x).ok());
        assert_eq!(rows.len(), 5);
        let first = rows[0].as_ref().expect("the first row");
        assert_eq!(first.width, 2 + 2 * REFLECT_PAD as u32);
        // Its first scene column is the source's row 5, the one above the
        // waterline; the pad left of the scene is clear.
        fn px(m: &Master, x: usize) -> &[u8] {
            &m.rgba[x * 4..x * 4 + 4]
        }
        let col0 = REFLECT_PAD as usize;
        assert_eq!(px(first, col0)[0], 50);
        assert_eq!(px(first, col0 - 1)[3], 0);
        // About the strength next to the waterline, fading row by row.
        let alpha = |k: usize| rows[k].as_ref().map_or(0, |m| px(m, col0)[3]);
        assert_eq!(alpha(0), (255. * sky::reflect_strength(0, 5)).round() as u8);
        for k in 1..5 {
            assert!(alpha(k) <= alpha(k - 1), "row {k} fades");
        }
        // Row k mirrors the row `mirror_row(k)` above the line.
        for (k, row) in rows.iter().enumerate() {
            if let Some(m) = row {
                let want = 10 * (5 - sky::mirror_row(k as i32)) as u8;
                assert_eq!(px(m, col0)[0], want, "row {k}");
            }
        }
        // Nothing to mirror: no row.
        let clear = Master {
            width: 4,
            height: 6,
            rgba: vec![0; 4 * 6 * 4],
        };
        assert!(
            mirror_rows(&clear, 6, 5, 2, plain, |x, _| u32::try_from(x).ok())
                .iter()
                .all(Option::is_none)
        );
        // The dark scheme's reflection is a little stronger and lighter,
        // never past its most.
        let dark = mirror_rows(&src, 6, 5, 2, Look::of(Scheme::Dark), |x, _| {
            u32::try_from(x).ok()
        });
        let d0 = px(dark[0].as_ref().unwrap(), col0);
        assert!(d0[3] > alpha(0) && f32::from(d0[3]) <= 255. * REFLECT_MOST + 1.);
        assert!(d0[0] > 50);
        // Covered columns (land) stay clear.
        let open = mirror_rows(&src, 6, 5, 2, plain, |x, k| {
            u32::try_from(x).ok().filter(|_| !(x == 0 && k < 2))
        });
        assert_eq!(px(open[0].as_ref().unwrap(), col0)[3], 0);
        assert!(px(open[2].as_ref().unwrap(), col0)[3] > 0);
        // The far band repeats from its centred tile.
        assert_eq!(dist_column(240, 0), 0);
        assert_eq!(dist_column(200, 0), 20);
        assert_eq!(dist_column(200, -1), 19);
    }

    /// The near band's reflection lies on open water only (never over the
    /// shore), and the lantern post shows straight below itself.
    #[test]
    fn the_near_reflection_shows_on_open_water_only() {
        let post = (sky::FORE_LAMP.0 + REFLECT_PAD) as usize;
        for scheme in [Scheme::Light, Scheme::Dark] {
            let fore = band(scheme, Band::Fore);
            let depth = fore.height as i32 - sky::FORE_WATERLINE;
            let rows = mirror_rows(
                fore,
                sky::FORE_WATERLINE,
                depth,
                fore.width as i32,
                Look::of(scheme),
                |x, k| {
                    let c = u32::try_from(x).ok()?;
                    (!on_land(fore, c, sky::FORE_WATERLINE + k)).then_some(c)
                },
            );
            let mut shown = 0;
            for (k, row) in rows.iter().enumerate() {
                let Some(m) = row else { continue };
                for i in 0..m.width as usize {
                    if m.rgba[i * 4 + 3] > 0 {
                        shown += 1;
                        let c = (i as i32 - REFLECT_PAD) as u32;
                        assert!(!on_land(fore, c, sky::FORE_WATERLINE + k as i32));
                    }
                }
            }
            assert!(shown > 0, "{scheme:?}: the jetty and boat reflect");
            // The lantern post shows straight below itself.
            let under = (0..16)
                .filter_map(|k| rows[k].as_ref())
                .filter(|m| (post - 1..=post + 1).any(|i| m.rgba[i * 4 + 3] > 0))
                .count();
            assert!(under > 0, "{scheme:?}: the lantern post reflects");
        }
    }

    /// Rows join in bands of two: a missing row is clear, an empty band is
    /// none, an odd last row stands alone.
    #[test]
    fn reflection_rows_join_in_bands() {
        let row = |v: u8| {
            Some(Master {
                width: 2,
                height: 1,
                rgba: vec![v; 8],
            })
        };
        let joined = bands(vec![row(1), None, None, None, row(3)]);
        assert_eq!(joined.len(), 3);
        let first = joined[0].as_ref().expect("a band");
        assert_eq!((first.width, first.height), (2, 2));
        assert_eq!(first.rgba[..8], [1; 8]);
        assert_eq!(first.rgba[8..], [0; 8], "the missing row is clear");
        assert!(joined[1].is_none(), "nothing in it");
        assert_eq!(joined[2].as_ref().map(|m| m.height), Some(1));
    }

    #[test]
    fn enlarging_repeats_crops_and_swaps_to_bgra() {
        let m = Master {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 40, 50, 60, 70, 80],
        };
        let image = enlarge(&m, 2, 1, 5).expect("an image");
        let bytes = image.as_bytes(0).expect("frame 0");
        assert_eq!(bytes, [70, 60, 50, 80].repeat(4).as_slice());
    }

    #[test]
    fn the_moon_shows_its_phase_and_the_sun_its_halo() {
        let t = tones(Scheme::Dark);
        let full = body_master(Body::Moon, 0.5, &t.moon, t.limb);
        let new = body_master(Body::Moon, 0., &t.moon, t.limb);
        let c = full.width / 2;
        let at = |m: &Master, x: u32| m.rgba[((c * m.width + x) * 4) as usize];
        // The disc's left edge: lit when full, the limb (paper) when new.
        let edge = c - sky::MOON_R as u32 + 1;
        assert!(at(&full, edge) > 0xA0);
        assert_eq!(at(&new, edge), 0x1A);
        let sun = body_master(Body::Sun, 0., &t.sun, t.limb);
        let side = sun.width;
        assert_eq!(side as i32, 2 * (sky::SUN_R + t.sun.halo_reach) + 1);
        assert_eq!(sun.alpha(0, 0), 0, "the halo fades out");
        assert!(
            sun.alpha(side / 2, side / 2 - sky::SUN_R as u32 - 2) > 0,
            "the halo"
        );
    }
}
