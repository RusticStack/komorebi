//! The Rest scene's layout and living parts, as pure functions of the
//! panel's size in art pixels, the time and a seed (no clock, no window):
//! where the horizon, the bands and the line go ([`layout`]), which body is
//! up and where ([`body_at`], [`arc`]), the moon's phase, the sky's stars
//! and clouds, the water's glimmer and reflections, the path on the
//! water, the mist, the
//! fireflies, the birds, the lantern's glow and the stars' twinkle.
//! `scene.rs` draws what these say. Coordinates are art pixels, origin top
//! left: the still parts sit on whole art pixels; the few moving lights
//! (the body, the fireflies, the birds, the mist) have fractional ones,
//! which `scene.rs` puts on the nearest device pixel.
//!
//! The motion is calm by design (people get motion sick from large,
//! stepping, sideways motion): every moving part is a smooth function of
//! the time in seconds (`t`, an `f64`), so the frame rate never changes its
//! speed; nothing jumps (a light point moves well under a device pixel a
//! frame, a brightness changes by a few hundredths at most); the water does
//! not travel, its streaks fade in, hold and fade out in place, a few at a
//! time. `tests::nothing_jumps_from_one_frame_to_the_next` holds all of it
//! to that at the scene's frame rate (`motion::SCENE_FRAME_MS`).

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

// ------------------------------------------------------------ the clock

/// The sun is up from this local minute of the day to [`SUNSET`]; the moon
/// the rest of the day. No place is known (none is looked up), so the two
/// hours are fixed, here and only here.
pub const SUNRISE: u32 = 6 * 60;
pub const SUNSET: u32 = 18 * 60;
const DAY: u32 = 24 * 60;

/// A new moon (2000-01-06 18:14 UTC) and the mean synodic month.
const NEW_MOON_UNIX: f64 = 947_182_440.;
const SYNODIC_DAYS: f64 = 29.530_588_853;

/// Which body is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    Sun,
    Moon,
}

/// The body up at a local minute of the day (with its fraction: the body
/// creeps along its arc, it never steps), and how far along its arc (0
/// rising, 1 setting).
pub fn body_at(minute: f32) -> (Body, f32) {
    let day = DAY as f32;
    let (rise, set) = (SUNRISE as f32, SUNSET as f32);
    let minute = minute.rem_euclid(day);
    if (rise..set).contains(&minute) {
        (Body::Sun, (minute - rise) / (set - rise))
    } else {
        let since = (minute + day - set) % day;
        (Body::Moon, since / (day - set + rise))
    }
}

/// The moon's phase at a Unix time: 0 new, 0.5 full, towards 1 new again.
pub fn moon_phase(unix: i64) -> f32 {
    let days = (unix as f64 - NEW_MOON_UNIX) / 86_400.;
    (days / SYNODIC_DAYS).rem_euclid(1.) as f32
}

/// Whether pixel `(dx, dy)` from a disc's centre is on a disc of radius `r`.
pub fn on_disc(dx: i32, dy: i32, r: i32) -> bool {
    let rr = (r as f32 + 0.5).powi(2);
    (dx * dx + dy * dy) as f32 <= rr
}

/// Whether the moon's pixel `(dx, dy)` from its centre is lit at `phase`
/// (the rest of the disc is the dark limb: the sky shows through).
pub fn moon_lit(dx: i32, dy: i32, r: i32, phase: f32) -> bool {
    if !on_disc(dx, dy, r) {
        return false;
    }
    let (dx, dy) = (dx as f32, dy as f32);
    let half = ((r as f32 + 0.5).powi(2) - dy * dy).max(0.).sqrt();
    let terminator = half * (TAU * phase).cos();
    if phase < 0.5 {
        dx > terminator
    } else {
        dx < -terminator
    }
}

/// How much of the moon is lit (0 new, 1 full).
pub fn moon_light(phase: f32) -> f32 {
    (1. - (TAU * phase).cos()) / 2.
}

/// The clock a snapshot pins (`KOMOREBI_SNAPSHOT_CLOCK=HH:MM[+S]`, read
/// only while `KOMOREBI_SNAPSHOT` is set): minutes after midnight, and
/// seconds into the motion.
fn snapshot_pin() -> Option<(u32, f32)> {
    static CLOCK: OnceLock<Option<(u32, f32)>> = OnceLock::new();
    *CLOCK.get_or_init(|| {
        std::env::var_os("KOMOREBI_SNAPSHOT")?;
        let s = std::env::var("KOMOREBI_SNAPSHOT_CLOCK").ok()?;
        crate::snapshot::parse_clock(&s).ok()
    })
}

/// The minute a snapshot pins, if it pins one.
pub fn snapshot_clock() -> Option<u32> {
    snapshot_pin().map(|(minute, _)| minute)
}

/// How many seconds into the motion a snapshot shows the scene (0 when
/// none is pinned).
pub fn snapshot_seconds() -> f32 {
    snapshot_pin().map_or(0., |(_, seconds)| seconds)
}

/// A development switch (docs/ENVIRONMENT.md, dev only): the minute
/// `KOMOREBI_SCENE_CLOCK=HH:MM` pins the Rest scene's clock to in a normal
/// run, to watch or measure the day or the night scene at any hour. The
/// motion runs as usual, and also in an inactive window ([`measuring`]).
fn scene_clock() -> Option<u32> {
    static CLOCK: OnceLock<Option<u32>> = OnceLock::new();
    *CLOCK.get_or_init(|| {
        let s = std::env::var("KOMOREBI_SCENE_CLOCK").ok()?;
        crate::snapshot::parse_clock(&s)
            .ok()
            .map(|(minute, _)| minute)
    })
}

/// Whether the scene runs in an inactive window too: only under the
/// development switch `KOMOREBI_SCENE_CLOCK`, so that a measuring script
/// (whose window Windows does not bring to the front) sees it running.
/// Out of sight (hidden, minimised) it still stops.
pub fn measuring() -> bool {
    scene_clock().is_some()
}

/// The local time the scene shows: (minute of the day with its fraction,
/// Unix seconds).
pub fn now() -> (f32, i64) {
    use chrono::Timelike as _;
    let local = chrono::Local::now();
    let minute = snapshot_clock().or_else(scene_clock).map_or_else(
        || (local.hour() * 60 + local.minute()) as f32 + local.second() as f32 / 60.,
        |m| m as f32,
    );
    (minute, local.timestamp())
}

// ------------------------------------------------------------ the art

/// The far band (`*-dist.png`): misty ridges and small pines, the far
/// waterline on row [`DIST_WATERLINE`], the ridges' highest pixels on
/// [`DIST_TOP`].
pub const DIST_W: i32 = 240;
#[cfg(test)]
pub const DIST_H: i32 = 100;
pub const DIST_WATERLINE: i32 = 92;
const DIST_TOP: i32 = 24;

/// The near band (`*-fore.png`): the shore, tall pines on the left, the
/// jetty, the boat, the reeds.
pub const FORE_W: i32 = 240;
pub const FORE_H: i32 = 240;
/// The tallest pine's top row.
const FORE_PINE_TOP: i32 = 5;
/// The pines take this much of the band's width; on its first
/// [`FORE_PINES_HIGH_ROWS`] rows (their tops), [`FORE_PINES_HIGH_W`].
const FORE_PINES_W: i32 = 90;
const FORE_PINES_HIGH_ROWS: i32 = 100;
const FORE_PINES_HIGH_W: i32 = 58;
/// The shore under the pines starts on this row.
const FORE_SHORE: i32 = 198;
/// The jetty's end; it stays in the panel however narrow.
const FORE_JETTY_END: i32 = 150;
/// The lantern post's lamp pixel.
pub const FORE_LAMP: (i32, i32) = (149, 197);

// ------------------------------------------------------------ the layout

/// The horizon sits at this share of the scene's height...
const HORIZON_SHARE: f32 = 0.58;
/// ...with at least this much water under it to the shore (rows)...
const WATER_MIN: i32 = FORE_H - FORE_SHORE + 16;
/// ...the tallest pine at least this far above it...
const PINE_CLEAR: i32 = 72;
/// ...and the ridges at least this far under the top.
const RIDGE_ROOM: i32 = 6;
/// The line's centre sits at this share of the height, with this much
/// room above the ridges and below the top.
const LINE_SHARE: f32 = 0.24;
const LINE_ROOM: i32 = 6;
/// Room between the line and the body at the top of its arc.
const APEX_ROOM: i32 = 4;
/// Room beside the jetty's end.
const JETTY_ROOM: i32 = 6;
/// The body's arc runs across this share of the width.
const ARC_FROM: f32 = 0.12;
const ARC_TO: f32 = 0.88;
/// The body's centre is this far under the horizon as it rises and sets.
const ARC_SINK: i32 = 8;
/// How quickly it rises and how long it stays high: its height is
/// `1 - (1 - sin)^ARC_RISE`, so it is half way up half an hour after
/// rising and sinks behind the ridges only in the last half hour or so,
/// and never moves fast (the slope is bounded, unlike a power of the sine
/// under 1, whose body leapt up in the first second).
const ARC_RISE: i32 = 5;

/// The disc radii.
pub const SUN_R: i32 = 17;
pub const MOON_R: i32 = 13;

/// Where everything goes in a scene `w` x `h` art pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub w: i32,
    pub h: i32,
    /// The far waterline's row.
    pub horizon: i32,
    /// The far band's top row.
    pub dist_y: i32,
    /// The near band's left (negative: shifted left to keep the jetty's
    /// end in) and top.
    pub fore_x: i32,
    pub fore_y: i32,
    /// The line's rows (top, bottom) and its left edge (0, or right of the
    /// pines when they reach the line).
    pub line_top: i32,
    pub line_bottom: i32,
    pub line_left: i32,
    /// The body's centre at the top of its arc.
    pub apex: i32,
}

/// The layout of a scene `w` x `h` art pixels whose line is `line_h` rows
/// tall.
pub fn layout(w: i32, h: i32, line_h: i32) -> Layout {
    let fore_y = h - FORE_H;
    let pine_top = fore_y + FORE_PINE_TOP;
    let lowest = (h - WATER_MIN).max(0);
    let highest = DIST_WATERLINE - DIST_TOP + RIDGE_ROOM;
    let horizon = ((h as f32 * HORIZON_SHARE).round() as i32)
        .max(pine_top + PINE_CLEAR)
        .min(lowest)
        .max(highest.min(lowest));
    let ridge_top = horizon - DIST_WATERLINE + DIST_TOP;
    let half = line_h / 2;
    let centre = ((h as f32 * LINE_SHARE).round() as i32)
        .min(ridge_top - LINE_ROOM - (line_h - half))
        .max(LINE_ROOM + half);
    let (line_top, line_bottom) = (centre - half, centre - half + line_h);
    let fore_x = (w - FORE_JETTY_END - JETTY_ROOM).min(0);
    // Clear of the pines when they reach the line's rows (their tops are
    // narrower than their skirts).
    let line_left = if line_bottom + LINE_ROOM <= pine_top {
        0
    } else if line_bottom - fore_y < FORE_PINES_HIGH_ROWS {
        (fore_x + FORE_PINES_HIGH_W).max(0)
    } else {
        (fore_x + FORE_PINES_W).max(0)
    };
    let apex = (line_bottom + APEX_ROOM + SUN_R).min(horizon - SUN_R);
    Layout {
        w,
        h,
        horizon,
        dist_y: horizon - DIST_WATERLINE,
        fore_x,
        fore_y,
        line_top,
        line_bottom,
        line_left,
        apex,
    }
}

impl Layout {
    /// The lantern's lamp pixel in the scene.
    pub fn lamp(&self) -> (i32, i32) {
        (self.fore_x + FORE_LAMP.0, self.fore_y + FORE_LAMP.1)
    }

    /// The ridges' highest row.
    pub fn ridge_top(&self) -> i32 {
        self.dist_y + DIST_TOP
    }
}

/// The body's centre on its arc (`progress` 0 rising, 1 setting): from 12
/// to 88 percent of the width, up from under the horizon to the apex. Not
/// rounded: the body creeps (a device pixel a minute or so).
pub fn arc(progress: f32, l: &Layout) -> (f32, f32) {
    let p = progress.clamp(0., 1.);
    let x = l.w as f32 * (ARC_FROM + (ARC_TO - ARC_FROM) * p);
    let low = (l.horizon + ARC_SINK) as f32;
    // A quick rise: the body clears the ridges soon after sunrise and
    // sinks behind them only near sunset.
    let up = 1. - (1. - (PI * p).sin().clamp(0., 1.)).powi(ARC_RISE);
    let y = low - (low - l.apex as f32) * up;
    (x, y)
}

// ------------------------------------------------------------ seeds

/// SplitMix64: a small, good hash for seeds.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A number in [0, 1) from a seed and an index.
fn unit(seed: u64, i: u64) -> f32 {
    (mix(seed ^ mix(i)) >> 40) as f32 / (1u64 << 24) as f32
}

// ------------------------------------------------------------ easing

/// Smoothstep: 0 up to 0, 1 from 1, eased between (no jump in the value
/// or its slope).
fn ease(x: f32) -> f32 {
    let x = x.clamp(0., 1.);
    x * x * (3. - 2. * x)
}

/// A life's light from its start (`u` 0) to its end (1): it fades in over
/// the first `fade` share of it, holds, and fades out over the last.
fn envelope(u: f32, fade: f32) -> f32 {
    ease(u / fade) * ease((1. - u) / fade)
}

/// Which cycle of `period` seconds the time `t` is in (shifted by
/// `offset` cycles), and how far into it (0 to 1). In `f64`: hours of
/// seconds keep their fractions.
fn cycle(t: f64, period: f32, offset: f32) -> (u64, f32) {
    let p = t.max(0.) / f64::from(period) + f64::from(offset);
    let whole = p.floor();
    (whole as u64, (p - whole) as f32)
}

/// A slow wave between -1 and 1 (`period` seconds, shifted by `offset`
/// cycles).
fn wave(t: f64, period: f32, offset: f32) -> f32 {
    (TAU * cycle(t, period, offset).1).sin()
}

/// One lit pixel on the art grid: where, and how bright (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spark {
    pub x: i32,
    pub y: i32,
    pub a: f32,
}

/// A moving light: where (art pixels, fractional), and how bright.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub x: f32,
    pub y: f32,
    pub a: f32,
}

/// A run of pixels on one row, from `x` (fractional for the mist and the
/// path, which creep).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Streak {
    pub x: f32,
    pub y: i32,
    pub len: i32,
    pub a: f32,
}

// ------------------------------------------------------------ the sky

/// Room kept clear of stars round the line and in the top corners (the
/// panel's rounded corners).
const STAR_ROOM: i32 = 3;
const CORNER_ROOM: i32 = 8;
/// About one star in this many sky pixels.
const STAR_EVERY: i32 = 110;

/// The night's stars: denser at the top, thinning towards the ridges,
/// clear of the line. `a` is the star's light (0 to 1); about a quarter
/// of them twinkle ([`twinkle`]).
pub fn stars(seed: u64, l: &Layout) -> Vec<Spark> {
    let sky = (l.ridge_top() + DIST_WATERLINE / 4).min(l.horizon).max(1);
    let n = (l.w * sky / STAR_EVERY).max(0) as u64;
    let mut out: Vec<Spark> = (0..n)
        .filter_map(|i| {
            let r = |k: u64| unit(seed, i * 4 + k);
            let x = (r(0) * l.w as f32) as i32;
            let y = (r(1).powf(1.9) * sky as f32) as i32;
            let by_line =
                y >= l.line_top - STAR_ROOM && y <= l.line_bottom + STAR_ROOM && x >= l.line_left;
            let in_corner = y < CORNER_ROOM && (x < CORNER_ROOM || x >= l.w - CORNER_ROOM);
            if by_line || in_corner {
                return None;
            }
            // Most are faint; a few are bright.
            Some(Spark {
                x,
                y,
                a: 0.15 + 0.85 * r(2).powi(3),
            })
        })
        .collect();
    out.sort_by(|a, b| b.a.total_cmp(&a.a));
    out
}

/// About this share of the stars twinkle; the rest hold still.
const TWINKLE_SHARE: f32 = 0.25;
/// A twinkling star dims by this much at most (0.2 to 0.35 of its light)...
const TWINKLE_DEPTH: (f32, f32) = (0.2, 0.15);
/// ...over a slow breath of 5 to 9 s.
const TWINKLE_PERIOD: (f32, f32) = (5., 4.);
/// A twinkling star is at least this bright at full light, so its breath
/// shows at all.
pub const TWINKLE_FLOOR: f32 = 0.45;

/// How a star shines at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Twinkle {
    /// Whether this star twinkles at all.
    pub live: bool,
    /// Its light now, as a share of its full light.
    pub light: f32,
}

/// Star `i`'s light `t` seconds in: about a quarter breathe, each on its
/// own slow eased cycle, a little dimmer and back (never a flash); the
/// rest, and every star under reduce motion, hold still.
pub fn twinkle(seed: u64, i: usize, t: f64, still: bool) -> Twinkle {
    let r = |k: u64| unit(seed, 1000 + i as u64 * 4 + k);
    let live = r(0) < TWINKLE_SHARE;
    if still || !live {
        return Twinkle { live, light: 1. };
    }
    let depth = TWINKLE_DEPTH.0 + TWINKLE_DEPTH.1 * r(3);
    let period = TWINKLE_PERIOD.0 + TWINKLE_PERIOD.1 * r(1);
    // 0 at the cycle's ends, 1 in its middle, eased (a cosine).
    let dim = 0.5 - 0.5 * (TAU * cycle(t, period, r(2)).1).cos();
    Twinkle {
        live,
        light: 1. - depth * dim,
    }
}

/// Clouds keep this far from the line.
const CLOUD_ROOM: i32 = 10;

/// Two or three soft cloud streaks at different heights, clear of the
/// line: each three rows (the middle one widest), with faded ends.
pub fn clouds(seed: u64, l: &Layout) -> Vec<Streak> {
    let (top, bottom) = (CORNER_ROOM + 4, l.ridge_top() - 12);
    let open: Vec<i32> = (top..bottom)
        .filter(|y| *y < l.line_top - CLOUD_ROOM || *y > l.line_bottom + CLOUD_ROOM)
        .collect();
    if open.len() < 8 {
        return Vec::new();
    }
    let n = 2 + mix(seed) % 2;
    let mut out = Vec::new();
    for i in 0..n {
        let r = |k: u64| unit(seed, 500 + i * 8 + k);
        // Spread over the open rows, a little jittered.
        let at = (i as f32 + 0.25 + 0.5 * r(0)) / n as f32 * open.len() as f32;
        let y = open[(at as usize).min(open.len() - 1)];
        let len = 36 + (r(1) * 40.) as i32;
        let x = (r(2) * (l.w + len) as f32) as i32 - len / 2;
        let a = 0.45 + 0.25 * r(3);
        for (dy, share, k) in [(-1, 0.45, 0.55), (0, 1., 1.), (1, 0.7, 0.75)] {
            let run = (len as f32 * share) as i32;
            let start = x + (len - run) / 2 + ((r((5 + dy) as u64) - 0.5) * 8.) as i32;
            // A soft run: its ends at half strength.
            let end = run.min(CLOUD_END);
            for (from, run_len, k) in [
                (start, end, k * 0.5),
                (start + end, (run - 2 * end).max(0), k),
                (start + run - end, end, k * 0.5),
            ] {
                out.push(Streak {
                    x: from as f32,
                    y: y + dy,
                    len: run_len,
                    a: a * k,
                });
            }
        }
    }
    out
}

/// The faded ends of a cloud row.
const CLOUD_END: i32 = 5;

// ------------------------------------------------------------ the water

/// About one glimmer for this many water pixels, and at most this many
/// (about seven in ten show at a moment: 40 or so on a mid-sized panel;
/// the reflections give the water most of its life).
const WATER_EVERY: i32 = 490;
const WATER_MAX: i32 = 60;
/// A glimmer lives 5 to 9 s and rests 1 to 4 s before the next one of its
/// slot shows somewhere else...
const WATER_LIFE: (f32, f32) = (5., 4.);
const WATER_REST: (f32, f32) = (1., 3.);
/// ...fading in over the first 40 percent of its life and out over the
/// last.
const WATER_FADE: f32 = 0.4;
/// The last share of the water's depth fades into the panel.
const WATER_FOOT: f32 = 0.08;

/// The water's glimmers `t` seconds in, one per slot (in slot order, `a` 0
/// while a slot rests): thin runs that fade in, hold and fade out in place
/// (they never travel), each slot's next life somewhere new, sparser and
/// shorter towards the horizon. `still` (reduce motion): the moment at 0 s.
pub fn water(seed: u64, t: f64, still: bool, l: &Layout) -> Vec<Streak> {
    let t = if still { 0. } else { t };
    let top = l.horizon + 2;
    let rows = l.h - top;
    if rows < 2 || l.w < 2 {
        return Vec::new();
    }
    let n = (l.w * rows / WATER_EVERY).clamp(0, WATER_MAX) as u64;
    (0..n)
        .map(|i| {
            let r = |k: u64| unit(seed, 2000 + i * 8 + k);
            let life = WATER_LIFE.0 + WATER_LIFE.1 * r(0);
            let period = life + WATER_REST.0 + WATER_REST.1 * r(1);
            let (n_life, into) = cycle(t, period, r(2));
            let u = into * period / life;
            let shown = if u < 1. { envelope(u, WATER_FADE) } else { 0. };
            // Each life shows somewhere new.
            let q = |k: u64| unit(mix(seed ^ mix(i) ^ n_life.wrapping_mul(0x9E37)), k);
            // 0 at the horizon, 1 at the bottom; nearer rows are likelier.
            let d = q(0).powf(0.6);
            let y = top + (d * (rows - 1) as f32).round() as i32;
            let len = 2 + (q(1) * (3. + 15. * d)) as i32;
            let x = (q(3) * (l.w + len) as f32 - len as f32).round();
            let foot = ((1. - d) / WATER_FOOT).clamp(0., 1.);
            let a = (0.4 + 0.5 * q(4)) * shown * foot;
            Streak { x, y, len, a }
        })
        .collect()
}

/// Mist by day: a few long, faint runs over the far shore, drifting
/// slowly; this many rows above the far waterline at most.
const MIST_ROWS: i32 = 7;
/// The mist drifts 0.12 to 0.24 art pixels a second (a device pixel every
/// second or so).
const MIST_SPEED: (f32, f32) = (0.12, 0.12);

/// The day's mist along the far shore `t` seconds in: three or four long
/// faint runs just above the far waterline, drifting very slowly right
/// (on the device grid, never an art pixel at once) and coming round
/// again; still under reduce motion.
pub fn mist(seed: u64, t: f64, still: bool, l: &Layout) -> Vec<Streak> {
    let t = if still { 0. } else { t.max(0.) };
    let n = 3 + mix(seed ^ 0x3157) % 2;
    let mut out = Vec::new();
    for i in 0..n {
        let r = |k: u64| unit(seed, 4000 + i * 8 + k);
        let len = 30 + (r(0) * 30.) as i32;
        let y = l.horizon - 1 - (r(1) * (MIST_ROWS - 1) as f32) as i32;
        let speed = MIST_SPEED.0 + MIST_SPEED.1 * r(2);
        let span = f64::from(l.w + len);
        let x = ((f64::from(r(3)) * span + f64::from(speed) * t) % span) as f32 - len as f32;
        let a = 0.5 + 0.25 * r(4);
        // Two rows, the upper one shorter and fainter, with soft ends as
        // the clouds have.
        for (dy, share, row_a) in [(0, 1., a), (-1, 0.6, a * 0.55)] {
            let run_len = (len as f32 * share) as i32;
            let start = x + ((len - run_len) / 2) as f32;
            let end = run_len.min(CLOUD_END * 2) / 2;
            for (from, run, k) in [
                (start, end, 0.5),
                (start + end as f32, run_len - 2 * end, 1.),
                (start + (run_len - end) as f32, end, 0.5),
            ] {
                if run > 0 {
                    out.push(Streak {
                        x: from,
                        y: y + dy,
                        len: run,
                        a: row_a * k,
                    });
                }
            }
        }
    }
    out
}

/// The body's path on the water: each of its rows crossfades between two
/// dashes, each living 5 to 8 s.
const PATH_LIFE: (f32, f32) = (5., 3.);
/// The path reaches this share of the lake's depth (it has faded out by
/// then)...
const PATH_REACH: f32 = 0.66;
/// ...and this many rows at most.
const PATH_ROWS: i32 = 110;
/// A dash's half width (art pixels) at the far waterline and at the
/// path's foot: a narrow column that widens towards the viewer.
const PATH_HALF: (f32, f32) = (1., 9.);
/// The share of a row's lives that are calm (no dash) at the waterline
/// and at the foot: the column is broken, more so further down.
const PATH_CALM: (f32, f32) = (0.12, 0.45);
/// How fast the path fades with depth: the power of the depth left.
const PATH_FALL: f32 = 1.3;

/// The body's path on the water under `x` `t` seconds in: its reflection,
/// a column of broken horizontal dashes from the far waterline down, narrow
/// and brightest there, wider, sparser and fainter further down, gone by
/// about two thirds of the lake. It shimmers by crossfading: each row holds
/// two dashes half a life apart, one fading out (`sin²`) as the other fades
/// in (`cos²`, so the row's light holds); a dash's next life lies a little
/// to one side, or is calm (`a` 0). Two per row, in row order (`a` 0 at a
/// life's ends). `still`: the moment at 0 s.
pub fn path(seed: u64, x: f32, t: f64, still: bool, l: &Layout) -> Vec<Streak> {
    let t = if still { 0. } else { t };
    let top = l.horizon + 2;
    let lake = (l.h - l.horizon) as f32;
    let bottom = top + ((lake * PATH_REACH).round() as i32).min(PATH_ROWS);
    let span = (bottom - top).max(1) as f32;
    let mut out = Vec::new();
    for (i, y) in (top..bottom).enumerate() {
        let i = i as u64;
        let r = |k: u64| unit(seed, 6000 + i * 4 + k);
        let period = PATH_LIFE.0 + PATH_LIFE.1 * r(0);
        let depth = (y - top) as f32 / span;
        let calm = PATH_CALM.0 + (PATH_CALM.1 - PATH_CALM.0) * depth;
        for slot in 0..2u64 {
            let (n_life, into) = cycle(t, period, r(1) + 0.5 * slot as f32);
            let shown = (PI * into).sin().powi(2);
            let q = |k: u64| {
                unit(
                    mix(seed ^ mix(6000 + i * 2 + slot) ^ n_life.wrapping_mul(0x2545)),
                    k,
                )
            };
            let half = PATH_HALF.0 + (PATH_HALF.1 - PATH_HALF.0) * depth;
            let len = (2. * half * (0.45 + 0.55 * q(1))).round().max(1.);
            let shift = ((q(2) - 0.5) * 2. * (0.5 + 2.5 * depth)).round();
            let lit = if q(4) < calm { 0. } else { 0.6 + 0.4 * q(3) };
            let a = (1. - depth).powf(PATH_FALL) * lit * shown;
            out.push(Streak {
                x: x + shift - (len / 2.).floor(),
                y,
                len: len as i32,
                a,
            });
        }
    }
    out
}

// ------------------------------------------------------------ reflections

/// The lake mirrors what stands on it, in the pixel-art water convention:
/// each row of a reflection is a source row mirrored about its waterline,
/// a little shorter than the source ([`REFLECT_SQUASH`]), fainter towards
/// the panel's paper (about [`REFLECT_TOP`] next to the waterline, nothing
/// at its far end), shifted sideways by a whole art pixel or two that grows
/// with the distance ([`ripple_offset`]), easing back and forth by half an
/// art pixel over several seconds, and broken by calm gaps that fade in and
/// out in place ([`calm_gap`]). Two reflections: the far band's (ridges
/// and small pines, mirrored about the horizon, under a clean row of
/// water) and the near band's (the jetty, its posts and lantern post, the
/// boat; the pines stand on land, which covers theirs), mirrored about
/// [`FORE_WATERLINE`]. The body's reflection is its path ([`path`]); the
/// lantern's a short broken streak ([`lamp_glints`]); bright stars and
/// fireflies by the water add faint glints ([`star_glints`],
/// [`firefly_glint`]).
pub const REFLECT_SQUASH: f32 = 0.8;
/// A reflection's strength next to its waterline (it fades to nothing
/// over its depth).
pub const REFLECT_TOP: f32 = 0.5;
/// How fast it fades: the power of the distance left.
const REFLECT_FALL: f32 = 1.3;
/// The near band's water surface: the boat's keel, where the jetty's posts
/// meet the water (the band's own row).
pub const FORE_WATERLINE: i32 = 215;
/// The most a reflection row is shifted (art pixels, either way) at its
/// far end, held (whole pixels: the art's grid)...
const RIPPLE_REACH: f32 = 1.5;
/// ...and how far it sways (art pixels, either way) at its far end, over
/// 6 to 10 s: half an art pixel, on the device grid.
const RIPPLE_SWAY: f32 = 0.5;
const RIPPLE_PERIOD: (f32, f32) = (6., 4.);
/// A calm gap lives 6 to 10 s, rests 2 to 8 s, and is 4 to 11 pixels long
/// (one row tall, its ends tapered by the caller: never a hard bar).
const GAP_LIFE: (f32, f32) = (6., 4.);
const GAP_REST: (f32, f32) = (2., 6.);
const GAP_LEN: (f32, f32) = (4., 7.);
/// A gap hides this much of the reflection under it at most.
const GAP_DEPTH: f32 = 0.6;

/// Which source row reflection row `k` (0 the first under the waterline)
/// mirrors: how many rows above the waterline (0 the row just above it).
/// Squashed: some source rows are skipped.
pub fn mirror_row(k: i32) -> i32 {
    (k.max(0) as f32 / REFLECT_SQUASH).floor() as i32
}

/// A reflection's strength at its row `k` of `depth` rows: [`REFLECT_TOP`]
/// at the waterline, falling smoothly to 0 at its far end.
pub fn reflect_strength(k: i32, depth: i32) -> f32 {
    if depth <= 0 || k < 0 || k >= depth {
        return 0.;
    }
    REFLECT_TOP * (1. - k as f32 / depth as f32).powf(REFLECT_FALL)
}

/// A reflection: its first row in the scene and how many rows deep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reflection {
    pub top: i32,
    pub depth: i32,
}

impl Layout {
    /// The far band's reflection: under the horizon and a clean row of
    /// water, as deep as the squashed ridges (or the water, if shallower).
    pub fn far_reflection(&self) -> Reflection {
        let top = self.horizon + 2;
        let tall = DIST_WATERLINE - DIST_TOP;
        Reflection {
            top,
            depth: ((tall as f32 * REFLECT_SQUASH) as i32)
                .min(self.h - top)
                .max(0),
        }
    }

    /// The near band's reflection: from its waterline to the panel's foot.
    pub fn near_reflection(&self) -> Reflection {
        let top = self.fore_y + FORE_WATERLINE;
        Reflection {
            top,
            depth: (self.h - top).max(0),
        }
    }
}

/// How far reflection row `k` of `depth` is shifted sideways `t` seconds
/// in (art pixels): a held whole-pixel shift that grows with the distance
/// from the waterline, plus a slow sway of half a pixel at most (out of
/// phase from row to row). `row` names the row (each its own). `still`:
/// the held shift alone.
pub fn ripple_offset(seed: u64, row: u64, k: i32, depth: i32, t: f64, still: bool) -> f32 {
    let u = (k as f32 / depth.max(1) as f32).clamp(0., 1.);
    let r = |j: u64| unit(seed, 7000 + row * 4 + j);
    let held = ((r(0) - 0.5) * 2. * RIPPLE_REACH * u).round();
    if still {
        return held;
    }
    let period = RIPPLE_PERIOD.0 + RIPPLE_PERIOD.1 * r(1);
    held + RIPPLE_SWAY * (0.3 + 0.7 * u) * wave(t, period, r(2))
}

/// The calm gap on reflection row `row` `t` seconds in: a run of still
/// water (the reflection hidden under it by `a` of the paper) that fades
/// in, holds and fades out in place, then rests; `a` 0 while it rests and
/// under reduce motion. Its `y` is 0 (the caller's row).
pub fn calm_gap(seed: u64, row: u64, w: i32, t: f64, still: bool) -> Streak {
    let r = |j: u64| unit(seed, 8000 + row * 4 + j);
    let life = GAP_LIFE.0 + GAP_LIFE.1 * r(0);
    let period = life + GAP_REST.0 + GAP_REST.1 * r(1);
    let (n_life, into) = cycle(t, period, r(2));
    let u = into * period / life;
    let shown = if still || u >= 1. {
        0.
    } else {
        envelope(u, 0.4)
    };
    let q = |j: u64| unit(mix(seed ^ mix(8000 + row) ^ n_life.wrapping_mul(0x51ED)), j);
    let len = (GAP_LEN.0 + GAP_LEN.1 * q(0)) as i32;
    Streak {
        x: (q(1) * (w + len) as f32 - len as f32).round(),
        y: 0,
        len,
        a: GAP_DEPTH * shown,
    }
}

/// The bright stars' faint glints on the water at night: each star high
/// enough to clear the ridges' reflection shows under it, mirrored about
/// the horizon (squashed), at this share of its light...
const STAR_GLINT: f32 = 0.3;
/// ...if it is at least this bright.
const STAR_GLINT_FROM: f32 = 0.5;

/// Where the bright stars glint on the water, and how bright (a share of
/// the star's light; the caller adds its twinkle). The index is the star's
/// in `stars`.
pub fn star_glints(stars: &[Spark], l: &Layout) -> Vec<(usize, Spark)> {
    let far = l.far_reflection();
    let ridges = ((l.horizon - l.ridge_top()) as f32 * REFLECT_SQUASH) as i32;
    stars
        .iter()
        .enumerate()
        .filter(|(_, s)| s.a >= STAR_GLINT_FROM)
        .filter_map(|(i, s)| {
            let k = ((l.horizon - 1 - s.y) as f32 * REFLECT_SQUASH) as i32;
            let y = far.top + k;
            (k > ridges && y < l.fore_y + FORE_WATERLINE - 2).then_some((
                i,
                Spark {
                    x: s.x,
                    y,
                    a: s.a * STAR_GLINT,
                },
            ))
        })
        .collect()
}

/// The lit lantern's reflection: a short broken streak under it, each of
/// its rows crossfading between two dashes (as the path does).
pub fn lamp_glints(seed: u64, t: f64, still: bool, l: &Layout) -> Vec<Streak> {
    let t = if still { 0. } else { t };
    let (lx, ly) = l.lamp();
    let line = l.fore_y + FORE_WATERLINE;
    let centre = line + ((line - 1 - ly) as f32 * REFLECT_SQUASH) as i32;
    let mut out = Vec::new();
    for (i, y) in (centre - 4..=centre + 6).step_by(2).enumerate() {
        if y < line + 1 || y >= l.h {
            continue;
        }
        let i = i as u64;
        let r = |k: u64| unit(seed, 9500 + i * 4 + k);
        let period = PATH_LIFE.0 + PATH_LIFE.1 * r(0);
        let fall = 1. - ((y - centre) as f32 / 7.).abs().min(1.);
        for slot in 0..2u64 {
            let (n_life, into) = cycle(t, period, r(1) + 0.5 * slot as f32);
            let shown = (PI * into).sin().powi(2);
            let q = |k: u64| {
                unit(
                    mix(seed ^ mix(9500 + i * 2 + slot) ^ n_life.wrapping_mul(0x2545)),
                    k,
                )
            };
            let len = 1 + (q(0) * 3.) as i32;
            let shift = (q(1) - 0.5) * 3.;
            out.push(Streak {
                x: (lx as f32 + shift - len as f32 / 2. + 0.5).round(),
                y,
                len,
                a: (0.35 + 0.65 * fall) * (0.5 + 0.5 * q(2)) * shown,
            });
        }
    }
    out
}

/// A firefly this close above the near waterline (rows) glints under it...
const FLY_GLINT_REACH: f32 = 14.;
/// ...at this share of its light next to the water.
const FLY_GLINT: f32 = 0.35;

/// The faint glint under a firefly near the water: mirrored about the near
/// waterline (squashed), fading with its height above it; none for one
/// higher up or under the waterline.
pub fn firefly_glint(f: &Light, l: &Layout) -> Option<Light> {
    let line = (l.fore_y + FORE_WATERLINE) as f32;
    let above = line - f.y;
    if !(0. ..FLY_GLINT_REACH).contains(&above) {
        return None;
    }
    let y = line + above * REFLECT_SQUASH;
    (y < (l.h - 1) as f32).then_some(Light {
        x: f.x,
        y,
        a: f.a * FLY_GLINT * (1. - above / FLY_GLINT_REACH),
    })
}

// ------------------------------------------------------------ particles

/// Where fireflies live, in the near band's own pixels (x0, y0, x1, y1):
/// round the pines' skirts (where they are dense: over land, not water), along the shore and the jetty, by the reeds.
const FLY_PINES: (i32, i32, i32, i32) = (4, 150, 80, 196);
const FLY_SHORE: (i32, i32, i32, i32) = (88, 186, 152, 214);
const FLY_REEDS: (i32, i32, i32, i32) = (190, 196, 236, 232);
/// A firefly's light breathes between this share and full over 4 to 7 s.
const FLY_DIM: f32 = 0.3;
const FLY_PULSE: (f32, f32) = (4., 3.);

/// The fireflies (8 to 12, by the seed) `t` seconds in, by the shore,
/// the jetty and the reeds, and round the pines when `on_pines` (the dark
/// scheme: the light scheme's dark specks would vanish there). Each
/// wanders slowly on a smooth curve (2.5 art pixels a second at most) and
/// its light breathes, eased. `still` (reduce motion): each at its home,
/// lit, whatever `t` is.
pub fn fireflies(seed: u64, t: f64, still: bool, on_pines: bool, l: &Layout) -> Vec<Light> {
    let zones: Vec<(f32, f32, f32, f32)> = [
        Some(FLY_PINES).filter(|_| on_pines),
        Some(FLY_SHORE),
        Some(FLY_REEDS),
    ]
    .into_iter()
    .flatten()
    .filter_map(|(x0, y0, x1, y1)| {
        let x0 = (l.fore_x + x0).max(2);
        let x1 = (l.fore_x + x1).min(l.w - 3);
        let y0 = (l.fore_y + y0).max(l.horizon + 4);
        let y1 = (l.fore_y + y1).min(l.h - 4);
        (x1 - x0 >= 4 && y1 - y0 >= 4).then_some((x0 as f32, y0 as f32, x1 as f32, y1 as f32))
    })
    .collect();
    if zones.is_empty() {
        return Vec::new();
    }
    let area: f32 = zones.iter().map(|z| (z.2 - z.0) * (z.3 - z.1)).sum();
    let n = 8 + mix(seed) % 5;
    (0..n)
        .map(|i| {
            let r = |k: u64| unit(seed, i * 16 + k);
            // A zone by its area.
            let mut pick = r(10) * area;
            let zone = *zones
                .iter()
                .find(|z| {
                    pick -= (z.2 - z.0) * (z.3 - z.1);
                    pick <= 0.
                })
                .unwrap_or(&zones[zones.len() - 1]);
            let (x0, y0, x1, y1) = zone;
            let home_x = x0 + (x1 - x0) * r(0);
            let home_y = y0 + (y1 - y0) * r(1);
            if still {
                return Light {
                    x: home_x.round(),
                    y: home_y.round(),
                    a: 0.75,
                };
            }
            // Slow wandering in its zone on a smooth curve (two waves
            // across, two up and down), along its own clock `m`, which
            // eases to a stop now and then and rests a moment. The clock
            // runs 1.5 s a second at most; the waves' speeds (amplitude
            // times rate) add up to 1.66 art pixels a second at most.
            let m = fly_clock(seed, i, t);
            let at = |amp: f32, rate: f32, phase: f32| {
                amp * ((m * f64::from(rate)).rem_euclid(std::f64::consts::TAU) as f32 + TAU * phase)
                    .sin()
            };
            let dx = at(6., 0.07 + 0.06 * r(2), r(3)) + at(2.5, 0.19 + 0.06 * r(4), r(5));
            let dy = at(3.5, 0.10 + 0.06 * r(6), r(7)) + at(1., 0.26 + 0.06 * r(11), r(12));
            let x = (home_x + dx).clamp(x0, x1);
            let y = (home_y + dy).clamp(y0, y1);
            // In and out, eased, each at its own pace and phase.
            let period = FLY_PULSE.0 + FLY_PULSE.1 * r(8);
            let pulse = 0.5 - 0.5 * (TAU * cycle(t, period, r(9)).1).cos();
            Light {
                x,
                y,
                a: FLY_DIM + (1. - FLY_DIM) * pulse,
            }
        })
        .collect()
}

/// A firefly's rhythm: it moves for most of a 9 to 15 s cycle and rests
/// for 1.5 to 4 s of it.
const FLY_CYCLE: f32 = 9.;
const FLY_CYCLE_MORE: f32 = 6.;
const FLY_REST: f32 = 1.5;
const FLY_REST_MORE: f32 = 2.5;

/// Firefly `i`'s own clock `t` seconds in: it runs while the firefly
/// moves, easing out to a stop and in again (no jump), and stands still
/// while it rests.
pub fn fly_clock(seed: u64, i: u64, t: f64) -> f64 {
    let r = |k: u64| unit(seed, 9000 + i * 4 + k);
    let length = FLY_CYCLE + FLY_CYCLE_MORE * r(0);
    let go = length - (FLY_REST + FLY_REST_MORE * r(1));
    let (n, into) = cycle(t, length, r(2));
    let s = (into * length / go).min(1.);
    (n as f64 + f64::from(ease(s))) * f64::from(go)
}

/// A distant bird: five pixels in a "v".
pub const BIRD: [(i32, i32); 5] = [(-2, -2), (-1, -1), (0, 0), (1, -1), (2, -2)];

/// The birds glide this many art pixels a second...
const BIRD_SPEED: f32 = 3.;
/// ...now and then: a crossing starts a cycle of at least this long, and
/// at least this long passes between two.
const BIRD_CYCLE: f32 = 90.;
const BIRD_REST: f32 = 30.;
/// They rise and fall this much (art pixels) over this many seconds.
const BIRD_BOB: f32 = 0.6;
const BIRD_BOB_PERIOD: f32 = 7.;

/// The birds' crossing time and cycle in a scene, in seconds (a wider
/// scene takes longer to cross, at the same speed).
fn bird_cycle(l: &Layout) -> (f32, f32) {
    let crossing = (l.w + 20) as f32 / BIRD_SPEED;
    (crossing, BIRD_CYCLE.max(crossing + BIRD_REST))
}

/// Where the day's birds are `t` seconds in (their centres; none between
/// crossings): two or three gliding slowly across the upper half, under
/// the line, now and then. They come in and leave outside the panel.
pub fn birds(seed: u64, t: f64, l: &Layout) -> Vec<(f32, f32)> {
    let (crossing, length) = bird_cycle(l);
    let (n_cycle, into) = cycle(t, length, 0.);
    let into = into * length;
    let s = mix(seed ^ n_cycle);
    // About two crossings in three cycles.
    if into > crossing || s.is_multiple_of(3) {
        return Vec::new();
    }
    let n = 2 + (s >> 8) % 2;
    let top = (l.line_bottom + 10) as f32;
    let bottom = (l.h as f32 * 0.45)
        .min((l.ridge_top() - 8) as f32)
        .max(top + 1.);
    let base_y = top + (bottom - top) * unit(s, 1);
    let x = -10. + BIRD_SPEED * into;
    (0..n)
        .map(|i| {
            let (ox, oy) = [(0., 0.), (-8., 4.), (7., 6.)][i as usize];
            let bob = BIRD_BOB * wave(t, BIRD_BOB_PERIOD, i as f32 / 3.);
            (x + ox, base_y + oy + bob)
        })
        .collect()
}

/// The lamp's glow reach, in art pixels.
pub const LAMP_GLOW: i32 = 9;

/// The lantern's brightness `t` seconds in: a slow, slight swell (two
/// slow waves, 0.81 to 0.99; 1 when still), never a flicker.
pub fn lantern(t: f64, still: bool) -> f32 {
    if still {
        return 1.;
    }
    0.9 + 0.06 * wave(t, 6., 0.) + 0.03 * wave(t, 11., 0.17)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::motion;

    /// 1280 x 800: the panel is 380 x 744 px, 190 x 372 art pixels.
    fn mid() -> Layout {
        layout(190, 372, 12)
    }

    /// One frame at the scene's rate, in seconds.
    const FRAME: f64 = motion::SCENE_FRAME_MS as f64 / 1000.;
    /// Device pixels per art pixel at 200 percent (`layout::scene_scale`).
    const AT_2X: f32 = 4.;

    #[test]
    fn the_sun_rises_at_six_tops_at_noon_and_sets_at_six() {
        let l = mid();
        assert_eq!(body_at(6. * 60.), (Body::Sun, 0.));
        // At the left, under the horizon.
        let (x, y) = arc(0., &l);
        assert_eq!(x, 190. * ARC_FROM);
        assert!(y > l.horizon as f32);
        let (body, p) = body_at(12. * 60.);
        assert_eq!((body, p), (Body::Sun, 0.5));
        let (x, y) = arc(p, &l);
        assert!(
            (x - 95.).abs() < 0.01 && (y - l.apex as f32).abs() < 0.01,
            "the top"
        );
        assert_eq!(body_at(18. * 60. - 0.5).0, Body::Sun);
        assert_eq!(body_at(18. * 60.), (Body::Moon, 0.));
        assert_eq!(body_at(0.).0, Body::Moon);
        assert_eq!(body_at(5. * 60. + 59.9).0, Body::Moon);
        // The moon tops out at midnight.
        assert_eq!(body_at(0.).1, 0.5);
        assert!(arc(1., &l).1 > l.horizon as f32);
    }

    #[test]
    fn known_full_and_new_moons() {
        // New moon 2024-04-08 18:21 UTC; full moon 2024-04-23 23:49 UTC.
        let new = moon_phase(1_712_600_460);
        assert!(!(0.03..=0.97).contains(&new), "new: {new}");
        let full = moon_phase(1_713_916_140);
        assert!((full - 0.5).abs() < 0.03, "full: {full}");
        assert!(moon_light(full) > 0.99);
        assert!(moon_light(new) < 0.01);
        // Full: the whole disc lit; new: none of it.
        assert!(moon_lit(-MOON_R, 0, MOON_R, 0.5) && moon_lit(MOON_R, 0, MOON_R, 0.5));
        assert!(!moon_lit(0, 0, MOON_R, 0.));
        // Waxing first quarter: the right half.
        assert!(moon_lit(5, 0, MOON_R, 0.25) && !moon_lit(-5, 0, MOON_R, 0.25));
    }

    #[test]
    fn reduce_motion_gives_a_stable_frame() {
        let l = mid();
        assert_eq!(
            fireflies(7, 0., true, true, &l),
            fireflies(7, 93.5, true, true, &l)
        );
        assert_eq!(lantern(12., true), 1.);
        for i in 0..40 {
            assert_eq!(twinkle(7, i, 40., true).light, 1.);
        }
        assert_eq!(water(7, 0., true, &l), water(7, 93.5, true, &l));
        assert_eq!(mist(7, 0., true, &l), mist(7, 93.5, true, &l));
        assert_eq!(path(7, 95., 0., true, &l), path(7, 95., 93.5, true, &l));
    }

    /// The composition holds from the shortest panel to a tall one, from
    /// the narrowest to the widest.
    #[test]
    fn the_layout_holds_at_every_size() {
        for (w, h) in [(160, 204), (190, 372), (240, 500), (160, 560), (300, 300)] {
            let l = layout(w, h, 12);
            let ctx = format!("{w}x{h}: {l:?}");
            // The ridges stay in the panel, under the line.
            assert!(l.ridge_top() >= RIDGE_ROOM, "{ctx}");
            assert!(l.line_bottom + LINE_ROOM <= l.ridge_top(), "{ctx}");
            assert!(l.line_top >= LINE_ROOM - 1, "{ctx}");
            // The tallest pine rises clearly above the horizon, and water
            // shows between the horizon and the shore.
            assert!(l.fore_y + FORE_PINE_TOP + PINE_CLEAR <= l.horizon, "{ctx}");
            assert!(l.horizon + WATER_MIN <= h, "{ctx}");
            // The jetty's end is in the panel.
            assert!(l.fore_x + FORE_JETTY_END < w, "{ctx}");
            // The body tops out under the line.
            assert!(l.apex - SUN_R >= l.line_bottom, "{ctx}");
            assert!(l.apex < l.horizon, "{ctx}");
        }
        // 1280 x 800: the horizon at 58 percent, the line at 24.
        let l = mid();
        assert_eq!(l.horizon, (372. * HORIZON_SHARE).round() as i32);
        assert_eq!(
            (l.line_top + l.line_bottom) / 2,
            (372. * LINE_SHARE).round() as i32
        );
        assert_eq!(l.fore_x, 0);
        assert_eq!(layout(150, 372, 12).fore_x, -6, "shifted to keep the jetty");
    }

    /// The motion sickness test: at the scene's frame rate, from one frame
    /// to the next, no moving part (fireflies, birds, mist, the body, the
    /// path) moves a device pixel at 200 percent, and no light (water,
    /// path, stars, fireflies, the lantern) changes by more than a few
    /// hundredths; a water glimmer or a path streak shows somewhere new
    /// only while it is dark. Over a minute from the start and a minute
    /// after an hour (`t` keeps its fractions).
    #[test]
    fn nothing_jumps_from_one_frame_to_the_next() {
        const MOST_LIGHT: f32 = 0.05;
        const DARK: f32 = 0.01;
        let mut most_move = (0f32, "");
        let mut most_light = 0f32;
        let mut moved = |what: &'static str, a: (f32, f32), b: (f32, f32)| {
            let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() * AT_2X;
            if d > most_move.0 {
                most_move = (d, what);
            }
        };
        let mut lit = |a: f32, b: f32| most_light = most_light.max((a - b).abs());
        let mut swaps = 0;
        for l in [mid(), layout(300, 300, 12), layout(160, 560, 9)] {
            for seed in 0..3 {
                let stars = stars(seed, &l);
                for start in [0., 3600.] {
                    for k in 0..(60. / FRAME) as usize {
                        let (a, b) = (start + k as f64 * FRAME, start + (k + 1) as f64 * FRAME);
                        for (p, q) in fireflies(seed, a, false, true, &l)
                            .iter()
                            .zip(&fireflies(seed, b, false, true, &l))
                        {
                            moved("a firefly", (p.x, p.y), (q.x, q.y));
                            lit(p.a, q.a);
                        }
                        let (ba, bb) = (birds(seed, a, &l), birds(seed, b, &l));
                        if ba.len() == bb.len() {
                            ba.iter()
                                .zip(&bb)
                                .for_each(|(p, q)| moved("a bird", *p, *q));
                        } else {
                            // A flock comes or goes only off the panel (a
                            // bird's wings reach two pixels either side).
                            for (x, _) in ba.iter().chain(&bb) {
                                assert!(*x <= -2.5 || *x >= l.w as f32 + 1.5, "{x}");
                            }
                        }
                        for (p, q) in mist(seed, a, false, &l)
                            .iter()
                            .zip(&mist(seed, b, false, &l))
                        {
                            // Coming round again happens off the panel.
                            if (p.x - q.x).abs() < 1. {
                                moved("the mist", (p.x, 0.), (q.x, 0.));
                            } else {
                                assert!(p.x >= l.w as f32 - 1. && q.x + (q.len as f32) <= 1.);
                            }
                        }
                        let glimmers = |v: &[Streak], w: &[Streak], swaps: &mut i32| {
                            let mut most = 0f32;
                            for (p, q) in v.iter().zip(w) {
                                if (p.x, p.y, p.len) == (q.x, q.y, q.len) {
                                    most = most.max((p.a - q.a).abs());
                                } else {
                                    assert!(p.a < DARK && q.a < DARK, "{p:?} -> {q:?}");
                                    *swaps += 1;
                                }
                            }
                            most
                        };
                        lit(
                            glimmers(
                                &water(seed, a, false, &l),
                                &water(seed, b, false, &l),
                                &mut swaps,
                            ),
                            0.,
                        );
                        lit(
                            glimmers(
                                &path(seed, 95., a, false, &l),
                                &path(seed, 95., b, false, &l),
                                &mut swaps,
                            ),
                            0.,
                        );
                        for (i, s) in stars.iter().enumerate().take(80) {
                            let light =
                                |t| s.a.max(TWINKLE_FLOOR) * twinkle(seed, i, t, false).light;
                            lit(light(a), light(b));
                        }
                        lit(lantern(a, false), lantern(b, false));
                    }
                }
            }
            // The body creeps: a frame's share of a minute along its arc.
            for minute in 0..DAY {
                let m = minute as f32;
                let frame = FRAME as f32 / 60.;
                let (ba, pa) = body_at(m);
                let (bb, pb) = body_at(m + frame);
                if ba == bb {
                    moved("the body", arc(pa, &l), arc(pb, &l));
                }
            }
        }
        assert!(
            most_move.0 < 1.,
            "{most_move:?}: device px in a frame at 2x"
        );
        assert!(most_light < MOST_LIGHT, "{most_light} of light in a frame");
        assert!(swaps > 0, "glimmers do show somewhere new");
    }

    #[test]
    fn fireflies_stay_low_by_the_shore_and_pulse() {
        let l = mid();
        for seed in 0..20 {
            for t in [0., 1.3, 17., 200.] {
                let flies = fireflies(seed, t, false, seed % 2 == 0, &l);
                assert!((8..=12).contains(&flies.len()));
                for f in flies {
                    assert!(f.y >= l.h as f32 * 0.55 - 1. && f.y < l.h as f32);
                    assert!(f.x >= 0. && f.x < l.w as f32);
                    assert!((FLY_DIM..=1.).contains(&f.a));
                }
            }
        }
    }

    #[test]
    fn the_sky_and_water_keep_their_places() {
        let l = mid();
        let stars = stars(3, &l);
        assert!(stars.len() > 50, "{}", stars.len());
        for s in &stars {
            assert!(s.y < l.horizon && (0..l.w).contains(&s.x));
            assert!(!(l.line_top..=l.line_bottom).contains(&s.y), "{s:?}");
        }
        // Denser at the top.
        let third = l.ridge_top() / 3;
        let high = stars.iter().filter(|s| s.y < third).count();
        assert!(high * 3 > stars.len(), "{high} of {}", stars.len());
        for c in clouds(3, &l) {
            assert!(c.y < l.ridge_top());
        }
        // Many lives, to see where glimmers land.
        let water: Vec<Streak> = (0..40)
            .flat_map(|k| water(3, k as f64 * 3.7, false, &l))
            .filter(|s| s.a > 0.)
            .collect();
        assert!(water.iter().all(|s| s.y > l.horizon && s.y < l.h));
        // Sparser and shorter towards the horizon.
        let rows = l.h - l.horizon;
        let (far, near): (Vec<&Streak>, Vec<&Streak>) =
            water.iter().partition(|s| s.y < l.horizon + rows / 3);
        assert!(
            near.len() > far.len() * 2,
            "{} near, {} far",
            near.len(),
            far.len()
        );
        let mean =
            |v: &[&Streak]| v.iter().map(|s| s.len as f32).sum::<f32>() / v.len().max(1) as f32;
        assert!(mean(&near) > mean(&far), "near runs are longer");
    }

    /// About a quarter of the stars breathe, each dimming a little and back
    /// on its own slow eased cycle (many levels: no steps, no glint); the
    /// rest hold still.
    #[test]
    fn a_quarter_of_the_stars_breathe_slowly() {
        let n = 400;
        let live = (0..n).filter(|i| twinkle(5, *i, 0., false).live).count();
        assert!((n / 6..n / 3).contains(&live), "{live} of {n}");
        for i in 0..n {
            let lights: Vec<f32> = (0..400)
                .map(|k| twinkle(5, i, k as f64 * FRAME, false).light)
                .collect();
            let levels: std::collections::BTreeSet<u32> =
                lights.iter().map(|l| (l * 1000.) as u32).collect();
            if twinkle(5, i, 0., false).live {
                assert!(levels.len() > 20, "star {i} breathes smoothly");
                assert!(lights.iter().all(|l| (0.64..=1.).contains(l)), "star {i}");
            } else {
                assert_eq!(levels.len(), 1, "star {i} holds still");
            }
        }
    }

    /// The water glimmers in place: about 25 to 60 streaks show at a time
    /// on a mid-sized panel, each where it began, and the lake changes from
    /// one moment to another a few seconds on.
    #[test]
    fn the_lake_glimmers_in_place() {
        let l = mid();
        for k in 0..50 {
            let t = k as f64 * 1.9;
            let shown = water(3, t, false, &l).iter().filter(|s| s.a > 0.01).count();
            assert!((25..=60).contains(&shown), "{shown} at {t}");
        }
        assert_ne!(water(3, 10., false, &l), water(3, 12., false, &l));
        // A glimmer lit at two moments closer than a rest has not moved.
        let (a, b) = (water(3, 10., false, &l), water(3, 10.9, false, &l));
        for (p, q) in a.iter().zip(&b) {
            if p.a > 0.01 && q.a > 0.01 {
                assert_eq!((p.x, p.y, p.len), (q.x, q.y, q.len));
            }
        }
    }

    /// Fireflies follow smooth curves and now and then rest a moment.
    #[test]
    fn fireflies_wander_smoothly_and_rest() {
        let l = mid();
        for seed in 0..6 {
            let frames: Vec<Vec<Light>> = (0..2000)
                .map(|k| fireflies(seed, k as f64 * FRAME, false, true, &l))
                .collect();
            for i in 0..frames[0].len() {
                let (mut still, mut longest) = (0, 0);
                for w in frames.windows(2) {
                    let (a, b) = (w[0][i], w[1][i]);
                    still = if (a.x, a.y) == (b.x, b.y) {
                        still + 1
                    } else {
                        0
                    };
                    longest = longest.max(still);
                }
                // A rest of at least a second in the hundred seconds.
                let second = (1. / FRAME) as usize;
                assert!(longest >= second, "seed {seed} fly {i} rests: {longest}");
            }
        }
        // Its clock never runs back.
        for k in 0..2000 {
            let t = k as f64 * 0.05;
            assert!(fly_clock(4, 2, t + 0.05) >= fly_clock(4, 2, t));
        }
    }

    /// Mist lies along the far shore and drifts.
    #[test]
    fn mist_drifts_along_the_far_shore() {
        let l = mid();
        let a = mist(4, 0., false, &l);
        assert!(!a.is_empty());
        for s in &a {
            assert!(s.y < l.horizon && s.y >= l.horizon - MIST_ROWS - 1);
        }
        assert_ne!(a, mist(4, 20., false, &l));
    }

    #[test]
    fn clouds_are_few_soft_and_clear_of_the_line() {
        for seed in 0..20 {
            for l in [mid(), layout(240, 342, 9), layout(160, 272, 12)] {
                let clouds = clouds(seed, &l);
                let rows: std::collections::BTreeSet<i32> = clouds.iter().map(|c| c.y).collect();
                // Two or three clouds of three rows each.
                assert!(rows.len() <= 9, "{rows:?}");
                for c in clouds {
                    assert!(
                        c.y < l.line_top - CLOUD_ROOM + 1 || c.y > l.line_bottom + CLOUD_ROOM - 1
                    );
                    assert!(c.y < l.ridge_top());
                }
            }
        }
    }

    /// The light scheme's fireflies keep off the pines, where they vanish.
    #[test]
    fn light_fireflies_keep_off_the_pines() {
        let l = mid();
        for seed in 0..20 {
            for f in fireflies(seed, 3., false, false, &l) {
                let on_pines =
                    f.x < (l.fore_x + FLY_PINES.2) as f32 && f.y < (l.fore_y + FLY_SHORE.1) as f32;
                assert!(!on_pines, "{f:?}");
            }
        }
    }

    /// The path widens and fades with depth, and its light holds as it
    /// shimmers (each row's two streaks crossfade).
    #[test]
    fn the_path_widens_fades_and_shimmers_in_place() {
        let l = mid();
        let s = path(3, 95., 4., false, &l);
        assert!(!s.is_empty());
        for w in s.windows(2) {
            assert!(w[0].y <= w[1].y);
        }
        assert!(s.iter().all(|s| s.y > l.horizon + 1));
        // Down to about two thirds of the lake, narrow at the top and
        // wider below, brightest at the top.
        let lake = (l.h - l.horizon) as f32;
        let last = s.last().unwrap().y;
        assert!((last - l.horizon) as f32 <= lake * PATH_REACH + 3.);
        assert!((last - l.horizon) as f32 >= (lake * PATH_REACH).min(PATH_ROWS as f32) - 3.);
        let widest = |rows: &[Streak]| rows.iter().map(|s| s.len).max().unwrap();
        let rows = s.len() / 2;
        assert!(widest(&s[..8]) <= 3, "narrow at the far waterline");
        assert!(widest(&s[rows..]) > widest(&s[..8]));
        let mut light = vec![0f32; rows];
        for t in 0..40 {
            for (i, pair) in path(3, 95., t as f64 * 0.7, false, &l)
                .chunks(2)
                .enumerate()
            {
                light[i] += pair[0].a + pair[1].a;
            }
        }
        let third = rows / 3;
        let sum = |r: &[f32]| r.iter().sum::<f32>();
        assert!(sum(&light[..third]) > 2. * sum(&light[2 * third..]));
        // Broken: some lives are calm.
        let calm = s
            .chunks(2)
            .filter(|pair| pair[0].a + pair[1].a < 0.02)
            .count();
        assert!(calm > 0, "the column is broken");
        assert_ne!(path(3, 95., 4., false, &l), path(3, 95., 6., false, &l));
        // A row's two lives' light adds up to the same share of their own
        // strengths at any moment: sin² + cos².
        for t in [0., 1.1, 7.3, 42.] {
            for pair in path(3, 95., t, false, &l).chunks(2) {
                assert!(pair.iter().all(|s| (0. ..=1.).contains(&s.a)));
            }
        }
    }

    /// A reflection mirrors about its waterline, a fifth shorter than its
    /// source, at about 0.4 next to the line and fading to nothing.
    #[test]
    fn reflections_mirror_squashed_and_fade() {
        assert_eq!(mirror_row(0), 0);
        assert_eq!(mirror_row(40), 50, "four fifths of the height");
        for k in 1..100 {
            let step = mirror_row(k) - mirror_row(k - 1);
            assert!((1..=2).contains(&step), "row {k}: {step}");
        }
        assert_eq!(reflect_strength(0, 50), REFLECT_TOP);
        assert!((0.35..=0.55).contains(&REFLECT_TOP));
        for k in 1..50 {
            assert!(reflect_strength(k, 50) < reflect_strength(k - 1, 50));
        }
        assert_eq!(reflect_strength(50, 50), 0.);
        assert!(reflect_strength(48, 50) < 0.01);
        // Where they lie: the far one under the horizon and a clean row,
        // the near one from the near waterline to the foot.
        for l in [
            mid(),
            layout(240, 342, 9),
            layout(160, 204, 12),
            layout(160, 560, 12),
        ] {
            let far = l.far_reflection();
            assert_eq!(far.top, l.horizon + 2);
            assert!(far.depth > 0 && far.top + far.depth <= l.h, "{l:?}");
            let near = l.near_reflection();
            assert_eq!(near.top + near.depth, l.h);
        }
    }

    /// The reflections' motion keeps the rules: a row's shift moves well
    /// under a device pixel a frame at 2x, held shifts are whole pixels
    /// that grow with the distance, a calm gap fades in and out in place
    /// (it moves only while hidden), and the glints never jump.
    #[test]
    fn reflections_never_jump() {
        let l = mid();
        let depth = l.far_reflection().depth;
        let mut most_move = 0f32;
        let mut most_light = 0f32;
        for k in 0..depth {
            let held = ripple_offset(3, k as u64, k, depth, 0., true);
            assert_eq!(held, held.round());
            assert!(held.abs() <= RIPPLE_REACH * k as f32 / depth as f32 + 0.5);
            for f in 0..(60. / FRAME) as usize {
                let (a, b) = (f as f64 * FRAME, (f + 1) as f64 * FRAME);
                let (p, q) = (
                    ripple_offset(3, k as u64, k, depth, a, false),
                    ripple_offset(3, k as u64, k, depth, b, false),
                );
                assert!(
                    (p - held).abs() <= RIPPLE_SWAY + 0.001,
                    "sways half a pixel"
                );
                most_move = most_move.max((p - q).abs() * AT_2X);
                let (g, h) = (
                    calm_gap(3, k as u64, l.w, a, false),
                    calm_gap(3, k as u64, l.w, b, false),
                );
                if (g.x, g.len) == (h.x, h.len) {
                    most_light = most_light.max((g.a - h.a).abs());
                } else {
                    assert!(g.a < 0.01 && h.a < 0.01, "{g:?} -> {h:?}");
                }
            }
        }
        let mut shown = 0;
        for f in 0..(60. / FRAME) as usize {
            let (a, b) = (f as f64 * FRAME, (f + 1) as f64 * FRAME);
            for (p, q) in lamp_glints(3, a, false, &l)
                .iter()
                .zip(&lamp_glints(3, b, false, &l))
            {
                if (p.x, p.y, p.len) == (q.x, q.y, q.len) {
                    most_light = most_light.max((p.a - q.a).abs());
                } else {
                    assert!(p.a < 0.01 && q.a < 0.01);
                }
            }
            let flies = |t| fireflies(3, t, false, true, &l);
            for (p, q) in flies(a).iter().zip(&flies(b)) {
                match (firefly_glint(p, &l), firefly_glint(q, &l)) {
                    (Some(g), Some(h)) => {
                        shown += 1;
                        let d = ((g.x - h.x).powi(2) + (g.y - h.y).powi(2)).sqrt();
                        most_move = most_move.max(d * AT_2X);
                        most_light = most_light.max((g.a - h.a).abs());
                    }
                    // A glint comes and goes only where it is faint.
                    (Some(g), None) | (None, Some(g)) => assert!(g.a < 0.05, "{g:?}"),
                    (None, None) => {}
                }
            }
        }
        assert!(most_move < 1., "{most_move} device px in a frame at 2x");
        assert!(most_light < 0.05, "{most_light} of light in a frame");
        assert!(shown > 0, "some fireflies glint");
        // Reduce motion: a still reflection, no gaps.
        assert_eq!(
            ripple_offset(3, 4, 10, depth, 0., true),
            ripple_offset(3, 4, 10, depth, 77., true)
        );
        assert_eq!(calm_gap(3, 4, l.w, 77., true).a, 0.);
    }

    /// Only bright stars high enough to clear the ridges' reflection glint,
    /// faintly, on open water.
    #[test]
    fn bright_stars_glint_on_open_water() {
        let l = mid();
        let stars = stars(3, &l);
        let glints = star_glints(&stars, &l);
        assert!(!glints.is_empty());
        let far = l.far_reflection();
        let ridges = ((l.horizon - l.ridge_top()) as f32 * REFLECT_SQUASH) as i32;
        for (i, g) in glints {
            assert!(stars[i].a >= STAR_GLINT_FROM);
            assert!(g.y > far.top + ridges && g.y < l.fore_y + FORE_WATERLINE);
            assert!(g.a <= stars[i].a * STAR_GLINT);
        }
    }

    #[test]
    fn birds_come_now_and_then_in_the_upper_half() {
        let l = mid();
        let (crossing, length) = bird_cycle(&l);
        let crossings = (0..30)
            .filter(|c| !birds(1, f64::from(*c as f32 * length) + 10., &l).is_empty())
            .count();
        assert!((5..30).contains(&crossings), "{crossings} of 30");
        for c in 0..30 {
            let t = f64::from(c as f32 * length);
            assert!(
                birds(1, t + f64::from(crossing) + 5., &l).is_empty(),
                "nothing between"
            );
            for (_, y) in birds(1, t + 10., &l) {
                assert!(y > l.line_bottom as f32 && y < l.h as f32 / 2., "{y}");
            }
        }
    }
}
