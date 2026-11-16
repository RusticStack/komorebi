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
//! neighbour) and painted 1:1, the rest is whole-pixel quads. The panel's
//! own paper is the ground; its rounded corners clip the scene.
//!
//! Cost: [`RestScene`] is its own view. It steps at about 12 frames a
//! second (`motion::SCENE_STEP_MS`) only while it is painted in the active
//! window and motion is allowed; under reduce motion it is redrawn once a
//! minute (the body moves) and everything else holds still. A step that
//! finds the scene was not painted since the last one (hidden, minimised,
//! another view) ends the task, so nothing runs while it is out of sight.
//! Decorative: only the line has a name.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui::{
    Bounds, Context, Corners, Hsla, IntoElement, ParentElement as _, Pixels, Render, RenderImage,
    Role, SharedString, Size, Styled, Subscription, Window, canvas, div, fill, point, prelude::*,
    px, size,
};
use gpui_kit::TestSupportExt as _;

use super::sky::{self, Body, Streak};
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
    /// The near band without its boat.
    Fore,
    /// The boat, cut from the near band to bob on its own.
    Boat,
}

/// A band for a scheme, decoded on first use (empty if it breaks the
/// contract: its test fails first).
fn band(scheme: Scheme, band: Band) -> &'static Master {
    static CELLS: [OnceLock<Master>; 6] = [const { OnceLock::new() }; 6];
    let dark = usize::from(scheme == Scheme::Dark);
    let (dist, fore) = match scheme {
        Scheme::Light => (DAY_DIST, DAY_FORE),
        Scheme::Dark => (NIGHT_DIST, NIGHT_FORE),
    };
    let (x, y, w, h) = sky::BOAT;
    match band {
        Band::Dist => CELLS[dark].get_or_init(|| decode(dist).unwrap_or_default()),
        Band::Fore => CELLS[2 + dark].get_or_init(|| {
            let mut m = decode(fore).unwrap_or_default();
            clear(&mut m, x, y, w, h);
            m
        }),
        Band::Boat => {
            CELLS[4 + dark].get_or_init(|| crop(&decode(fore).unwrap_or_default(), x, y, w, h))
        }
    }
}

/// The pixels of `m` in the box at `(x, y)`, `w` by `h` (clipped to it).
fn crop(m: &Master, x: i32, y: i32, w: i32, h: i32) -> Master {
    let (x0, y0) = (x.max(0) as u32, y.max(0) as u32);
    let x1 = ((x + w).max(0) as u32).min(m.width);
    let y1 = ((y + h).max(0) as u32).min(m.height);
    if x1 <= x0 || y1 <= y0 {
        return Master::default();
    }
    let mut rgba = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for row in y0..y1 {
        let from = ((row * m.width + x0) * 4) as usize;
        let to = ((row * m.width + x1) * 4) as usize;
        rgba.extend_from_slice(&m.rgba[from..to]);
    }
    Master {
        width: x1 - x0,
        height: y1 - y0,
        rgba,
    }
}

/// Makes the box at `(x, y)`, `w` by `h`, of `m` clear.
fn clear(m: &mut Master, x: i32, y: i32, w: i32, h: i32) {
    for row in y.max(0)..(y + h).min(m.height as i32) {
        for col in x.max(0)..(x + w).min(m.width as i32) {
            let p = ((row as u32 * m.width + col as u32) * 4 + 3) as usize;
            m.rgba[p] = 0;
        }
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

/// A star's glint: its four neighbours at this opacity.
const GLINT: f32 = 0.4;

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

/// The far band's bottom tone (the mean of its waterline row), where the
/// water under the horizon starts.
fn band_bottom(scheme: Scheme) -> Hsla {
    static CELLS: [OnceLock<Hsla>; 2] = [const { OnceLock::new() }; 2];
    let i = usize::from(scheme == Scheme::Dark);
    *CELLS[i].get_or_init(|| {
        let m = band(scheme, Band::Dist);
        let row = sky::DIST_WATERLINE as usize;
        let (mut v, mut a, mut n) = (0u32, 0u32, 0u32);
        for x in 0..m.width as usize {
            let p = (row * m.width as usize + x) * 4;
            if let Some(px) = m.rgba.get(p..p + 4) {
                v += u32::from(px[0]);
                a += u32::from(px[3]);
                n += 1;
            }
        }
        let n = n.max(1);
        crate::theme::scene::grey((v / n) as u8, (a / n) as f32 / 255.)
    })
}

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

// ------------------------------------------------------------ images

/// What an enlarged picture was made from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Picture {
    Band(Scheme, Band, u32, u32, u32),
    Body(Scheme, Body, u32, u32),
}

/// The pictures in use, one per slot (dist, fore, body, boat); a replaced
/// one is dropped from the window's sprite atlas.
type Slot = Option<(Picture, Arc<RenderImage>)>;

thread_local! {
    static PICTURES: RefCell<[Slot; 4]> = const { RefCell::new([None, None, None, None]) };
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
            Picture::Body(scheme, body, n, phase) => {
                let t = tones(scheme);
                let tone = match body {
                    Body::Sun => t.sun,
                    Body::Moon => t.moon,
                };
                let m = body_master(body, phase as f32 / PHASE_STEPS, &tone, t.limb);
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
    minute: u32,
    unix: i64,
    /// Seconds into the motion, and whether it holds still.
    t: f32,
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
    let snap = |v: f32| (v * sf).round() / sf;
    let u = f.n as f32 / sf;
    let (w, h) = art_size(bounds.size, sf, f.n);
    let l = sky::layout(w, h, f.line_h);
    let left = snap(bounds.origin.x.as_f32());
    let top = snap((bounds.origin.y + bounds.size.height).as_f32()) - h as f32 * u;
    let at = |x: i32, y: i32| point(px(left + x as f32 * u), px(top + y as f32 * u));
    let rect = |x: i32, y: i32, w: i32, h: i32| {
        Bounds::new(at(x, y), size(px(w as f32 * u), px(h as f32 * u)))
    };
    let dot = |window: &mut Window, x: i32, y: i32, c: Hsla| {
        if c.a > 0.004 {
            window.paint_quad(fill(rect(x, y, 1, 1), c));
        }
    };
    let run = |window: &mut Window, s: &sky::Streak, c: Hsla| {
        let (x0, x1) = (s.x.max(0), (s.x + s.len).min(w));
        if x1 > x0 && s.a > 0.004 {
            window.paint_quad(fill(rect(x0, s.y, x1 - x0, 1), c.opacity(s.a)));
        }
    };
    let draw =
        |window: &mut Window, x: i32, y: i32, img: Arc<RenderImage>, corners: Corners<Pixels>| {
            const FIRST: usize = 0;
            let s = img.size(FIRST);
            let area = Bounds::new(
                at(x, y),
                size(px(s.width.0 as f32 / sf), px(s.height.0 as f32 / sf)),
            );
            window.paint_image(area, area, corners, img, 0, false).ok();
        };
    let t = &f.tones;
    let (body, progress) = sky::body_at(f.minute);
    let (bx, by) = sky::arc(progress, &l);
    let night = body == Body::Moon;

    // The sky.
    if f.scheme == Scheme::Dark && night {
        for (i, s) in sky::stars(f.seed, &l).iter().enumerate() {
            let tw = sky::twinkle(f.seed, i, f.t, f.still);
            if tw.glint {
                // Full light, with a faint cross round it.
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    dot(window, s.x + dx, s.y + dy, t.star.opacity(GLINT));
                }
                dot(window, s.x, s.y, t.star);
            } else if tw.live {
                let a = s.a.max(sky::TWINKLE_FLOOR) * tw.light;
                dot(window, s.x, s.y, t.star.opacity(a));
            } else {
                dot(window, s.x, s.y, t.star.opacity(s.a));
            }
        }
        for p in sky::shooting_star(f.seed, f.t, f.still, &l) {
            dot(window, p.x, p.y, t.star.opacity(p.a));
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
    let up = by < l.horizon;
    if by - r < l.horizon {
        let phase = (sky::moon_phase(f.unix) * PHASE_STEPS).round() as u32 % PHASE_STEPS as u32;
        let key = Picture::Body(f.scheme, body, f.n, if night { phase } else { 0 });
        if let Some(img) = picture(2, key, window) {
            let half = r + tone.halo_reach;
            let sky_rect = Bounds::new(at(0, 0), size(px(w as f32 * u), px(l.horizon as f32 * u)));
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
            draw(window, x, l.dist_y, img.clone(), Corners::default());
            x += sky::DIST_W;
        }
    }
    // By day, mist drifting along the far shore.
    if !night {
        for s in sky::mist(f.seed, f.t, f.still, &l) {
            run(window, &s, t.mist);
        }
    }

    // The water: under the horizon it starts in the far band's bottom
    // tone and fades into the panel (no hard edge), then faint streaks,
    // then the body's path while it is up.
    let shore = band_bottom(f.scheme);
    for k in 1..=sky::WATER_BLEND {
        let fade = 1. - (k - 1) as f32 / sky::WATER_BLEND as f32;
        let row = Streak {
            x: 0,
            y: l.horizon + k,
            len: w,
            a: fade * fade,
        };
        run(window, &row, shore);
    }
    for s in sky::water(f.seed, f.t, f.still, &l) {
        run(window, &s, t.water);
    }
    for p in sky::ripples(f.seed, f.t, f.still, &l) {
        dot(window, p.x, p.y, t.ripple.opacity(p.a));
    }
    if up {
        let roll = if f.still {
            u64::from(f.minute)
        } else {
            (f.t * 1000.) as u64 / motion::SCENE_PATH_MS
        };
        let light = match body {
            Body::Sun => 1.,
            Body::Moon => sky::moon_light(sky::moon_phase(f.unix)).max(0.3),
        };
        for s in sky::path(bx, roll, &l) {
            run(window, &s, tone.path.opacity(tone.path.a * light));
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
        draw(window, 0, l.fore_y, img, corners);
    }
    // The boat, bobbing on its own.
    if let Some(img) = picture(
        3,
        Picture::Band(f.scheme, Band::Boat, f.n, 0, sky::BOAT.2 as u32),
        window,
    ) {
        let (bx, by) = (l.fore_x + sky::BOAT.0, l.fore_y + sky::BOAT.1);
        draw(
            window,
            bx,
            by + sky::boat_bob(f.t, f.still),
            img,
            Corners::default(),
        );
    }

    // The night's lantern and fireflies; the day's birds.
    if night {
        let (lx, ly) = l.lamp();
        let flicker = sky::lantern(f.t, f.still);
        let reach = sky::LAMP_GLOW;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d == 0. || d > reach as f32 {
                    continue;
                }
                let k = 1. - d / reach as f32;
                dot(
                    window,
                    lx + dx,
                    ly + dy,
                    t.lamp_glow.opacity(t.lamp_glow.a * flicker * k * k),
                );
            }
        }
        dot(window, lx, ly, t.lamp.opacity(flicker));
        // A firefly is a pixel in a soft glow (unlike a star), on and off
        // the pines in the dark scheme, by the shore and reeds only in the
        // light one.
        let dark = f.scheme == Scheme::Dark;
        for s in sky::fireflies(f.seed, f.t, f.still, dark, &l) {
            for (dx, dy, k) in GLOW {
                if k < 1. && !dark {
                    continue;
                }
                let a = t.firefly_glow.a * s.a * k;
                dot(window, s.x + dx, s.y + dy, t.firefly_glow.opacity(a));
            }
            dot(window, s.x, s.y, t.firefly.opacity(s.a));
        }
    } else if !f.still {
        for (x, y) in sky::birds(f.seed, f.t, &l) {
            for (dx, dy) in sky::BIRD {
                dot(window, x + dx, y + dy, t.bird);
            }
        }
    }
}

// ------------------------------------------------------------ the view

/// How the scene is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    /// About 12 frames a second.
    Frames,
    /// Once a minute (reduce motion).
    Minute,
}

/// The Rest scene and its line, as its own view so its steps redraw it
/// alone.
pub struct RestScene {
    words: SharedString,
    /// The panel's width (its height comes from the last paint).
    width: f32,
    seed: u64,
    /// Bumped to end a running task.
    epoch: u64,
    running: Option<Run>,
    step: u64,
    /// Set by each paint, taken by each step: no paint since the last
    /// step means the scene is out of sight, and the task ends.
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
            epoch: 0,
            running: None,
            step: 0,
            painted: Rc::new(Cell::new(false)),
            panel: Rc::new(Cell::new(None)),
            activation: None,
        }
    }

    /// The line and the panel's width (from the mail view's render; the
    /// scene renders with it, so nothing to notify).
    pub fn set(&mut self, words: SharedString, width: f32) {
        self.words = words;
        self.width = width;
    }

    #[cfg(test)]
    pub fn is_stepping(&self) -> bool {
        self.running == Some(Run::Frames)
    }

    /// Starts, swaps or stops the task to match `wanted` (called from
    /// `render`, so activity and reduce motion are re-read every time).
    fn sync(&mut self, wanted: Option<Run>, cx: &mut Context<Self>) {
        if self.running == wanted {
            return;
        }
        self.epoch += 1;
        self.running = wanted;
        let Some(run) = wanted else {
            return;
        };
        let epoch = self.epoch;
        cx.spawn(async move |this, cx| {
            loop {
                let wait = match run {
                    Run::Frames => Duration::from_millis(motion::SCENE_STEP_MS),
                    Run::Minute => {
                        let into = (chrono::Local::now().timestamp_millis() as u64)
                            % motion::SCENE_MINUTE_MS;
                        Duration::from_millis(motion::SCENE_MINUTE_MS - into)
                    }
                };
                cx.background_executor().timer(wait).await; // timer-exempt: the Rest scene's steps, only while it is painted in the active window (12 a second, or once a minute under reduce motion)
                let alive = this.update(cx, |scene, cx| {
                    if scene.epoch != epoch {
                        return false;
                    }
                    if !scene.painted.replace(false) {
                        scene.running = None;
                        return false;
                    }
                    if run == Run::Frames {
                        scene.step += 1;
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
        // A snapshot holds motion still, unless it pins a moment of it
        // (`KOMOREBI_SNAPSHOT_CLOCK=HH:MM+S`).
        let still = cx.reduce_motion() && sky::snapshot_seconds() <= 0.;
        let wanted = match (window.is_window_active(), still) {
            (false, _) => None,
            (true, true) => Some(Run::Minute),
            (true, false) => Some(Run::Frames),
        };
        self.sync(wanted, cx);

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
            scheme: t.scheme,
            tones: tones(t.scheme),
            n,
            line_h,
            minute,
            unix,
            t: sky::snapshot_seconds() + (self.step * motion::SCENE_STEP_MS) as f32 / 1000.,
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

    /// Steps about 12 times a second while painted in the active window;
    /// nothing runs in an inactive window, and reduce motion holds still.
    #[gpui::test]
    fn the_scene_steps_only_while_seen(cx: &mut gpui::TestAppContext) {
        crate::widgets::test_support::init(cx, false);
        cx.update(|cx| cx.set_reduce_motion(false));
        let (scene, cx) = cx.add_window_view(|_, _| {
            let mut s = RestScene::new();
            s.set("Rest".into(), 380.);
            s
        });
        let step = |cx: &mut gpui::VisualTestContext| scene.read_with(cx, |s, _| s.step);
        cx.run_until_parked();
        assert!(
            !scene.read_with(cx, |s, _| s.is_stepping()),
            "inactive: no task"
        );
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert_eq!(step(cx), 0);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(scene.read_with(cx, |s, _| s.is_stepping()), "active: steps");
        for _ in 0..12 {
            cx.executor()
                .advance_clock(Duration::from_millis(motion::SCENE_STEP_MS));
            cx.run_until_parked();
        }
        let ran = step(cx);
        assert!((11..=12).contains(&ran), "about 12 a second, got {ran}");
        cx.update(|_, cx| cx.set_reduce_motion(true));
        scene.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(
            !scene.read_with(cx, |s, _| s.is_stepping()),
            "reduce motion: still"
        );
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        assert_eq!(step(cx), ran, "held");
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
