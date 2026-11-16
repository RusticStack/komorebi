//! The Rest scene's layout and living parts, as pure functions of the
//! panel's size in art pixels, the time and a seed (no clock, no window):
//! where the horizon, the bands and the line go ([`layout`]), which body is
//! up and where ([`body_at`], [`arc`]), the moon's phase, the sky's stars
//! and clouds, the water and its rings, the boat's bob, the path on the
//! water, the mist, the fireflies, a shooting star, the birds, the
//! lantern's flicker and the stars' twinkle. `scene.rs` draws what these
//! say. Every coordinate is a whole art pixel, origin top left.

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

/// The body up at a local minute of the day, and how far along its arc
/// (0 rising, 1 setting).
pub fn body_at(minute: u32) -> (Body, f32) {
    let minute = minute % DAY;
    let span = (SUNSET - SUNRISE) as f32;
    if (SUNRISE..SUNSET).contains(&minute) {
        (Body::Sun, (minute - SUNRISE) as f32 / span)
    } else {
        let since = (minute + DAY - SUNSET) % DAY;
        (Body::Moon, since as f32 / (DAY - SUNSET + SUNRISE) as f32)
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

/// The local time the scene shows: (minute of the day, Unix seconds).
pub fn now() -> (u32, i64) {
    use chrono::Timelike as _;
    let local = chrono::Local::now();
    let minute = snapshot_clock().unwrap_or(local.hour() * 60 + local.minute());
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
/// Under the horizon the water starts in the far band's bottom tone and
/// fades into the panel over this many rows.
pub const WATER_BLEND: i32 = 20;
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
/// How quickly it rises and how long it stays high (a power of the sine:
/// under 1, it clears the ridges soon after rising and sinks behind them
/// only in the last half hour or so).
const ARC_RISE: f32 = 0.3;

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
/// to 88 percent of the width, up from under the horizon to the apex.
pub fn arc(progress: f32, l: &Layout) -> (i32, i32) {
    let p = progress.clamp(0., 1.);
    let x = l.w as f32 * (ARC_FROM + (ARC_TO - ARC_FROM) * p);
    let low = (l.horizon + ARC_SINK) as f32;
    // A quick rise: the body clears the ridges soon after sunrise and
    // sinks behind them only near sunset.
    let y = low - (low - l.apex as f32) * (PI * p).sin().max(0.).powf(ARC_RISE);
    (x.round() as i32, y.round() as i32)
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

/// One lit pixel: where, and how bright (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spark {
    pub x: i32,
    pub y: i32,
    pub a: f32,
}

/// A run of pixels on one row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Streak {
    pub x: i32,
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
/// A twinkling star's light steps, dim to full (pixel art: no smooth fade).
const TWINKLE_STEPS: [f32; 3] = [0.3, 0.62, 1.];
/// A twinkling star is at least this bright at full light, so its steps
/// show.
pub const TWINKLE_FLOOR: f32 = 0.55;
/// One cycle in this many ends its brightest step in a glint.
const GLINT_EVERY: u64 = 5;

/// How a star shines at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Twinkle {
    /// Whether this star twinkles at all.
    pub live: bool,
    /// Its light now, as a share of its full light.
    pub light: f32,
    /// A glint: full light with a faint cross round it.
    pub glint: bool,
}

/// Star `i`'s light `t` seconds in: about a quarter twinkle, each on its
/// own slow cycle (2 to 6 s) stepping between three levels, now and then
/// peaking in a glint; the rest, and every star under reduce motion, hold
/// still.
pub fn twinkle(seed: u64, i: usize, t: f32, still: bool) -> Twinkle {
    let r = |k: u64| unit(seed, 1000 + i as u64 * 4 + k);
    let live = r(0) < TWINKLE_SHARE;
    if still || !live {
        return Twinkle {
            live,
            light: 1.,
            glint: false,
        };
    }
    let period = 2. + 4. * r(1);
    let phase = t.max(0.) / period + r(2);
    let cycle = phase.floor();
    // 0 at the cycle's ends, 1 in its middle.
    let wave = 0.5 - 0.5 * (TAU * (phase - cycle)).cos();
    let light = TWINKLE_STEPS[((wave * 3.) as usize).min(2)];
    let glint = wave > 0.9 && mix(seed ^ mix(i as u64) ^ cycle as u64).is_multiple_of(GLINT_EVERY);
    Twinkle { live, light, glint }
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
            out.push(Streak {
                x: start,
                y: y + dy,
                len: end,
                a: a * k * 0.5,
            });
            out.push(Streak {
                x: start + end,
                y: y + dy,
                len: (run - 2 * end).max(0),
                a: a * k,
            });
            out.push(Streak {
                x: start + run - end,
                y: y + dy,
                len: end,
                a: a * k * 0.5,
            });
        }
    }
    out
}

/// The faded ends of a cloud row.
const CLOUD_END: i32 = 5;

// ------------------------------------------------------------ the water

/// About one water streak for this many water pixels, and at most this
/// many.
const WATER_EVERY: i32 = 110;
const WATER_MAX: i32 = 260;
/// The last share of the water's depth fades into the panel.
const WATER_FOOT: f32 = 0.08;

/// The water's streaks `t` seconds in, over the whole lake: thin runs that
/// drift slowly sideways (the nearer, the faster) and fade in and out over
/// 4 to 8 s, sparser and shorter towards the horizon. `still` (reduce
/// motion): each at the middle of its first life.
pub fn water(seed: u64, t: f32, still: bool, l: &Layout) -> Vec<Streak> {
    let top = l.horizon + 2;
    let rows = l.h - top;
    if rows < 2 || l.w < 2 {
        return Vec::new();
    }
    let n = (l.w * rows / WATER_EVERY).clamp(0, WATER_MAX) as u64;
    (0..n)
        .filter_map(|i| {
            let r = |k: u64| unit(seed, 2000 + i * 8 + k);
            let period = 4. + 4. * r(0);
            let phase = if still {
                0.5
            } else {
                t.max(0.) / period + r(1)
            };
            let cycle = phase.floor();
            let into = phase - cycle;
            // Each life starts somewhere new.
            let life = mix(seed ^ mix(i) ^ (cycle as u64).wrapping_mul(0x9E37));
            let q = |k: u64| unit(life, k);
            // 0 at the horizon, 1 at the bottom; nearer rows are likelier.
            let d = q(0).powf(0.6);
            let y = top + (d * (rows - 1) as f32).round() as i32;
            let len = 2 + (q(1) * (3. + 15. * d)) as i32;
            let speed = (0.4 + 0.6 * q(2)) * (0.3 + 0.9 * d);
            let drift = if still { 0. } else { into * period * speed };
            let x = (q(3) * (l.w + len) as f32 - len as f32 + drift).round() as i32;
            let fade = (PI * into).sin();
            let foot = ((1. - d) / WATER_FOOT).clamp(0., 1.);
            let a = (0.4 + 0.5 * q(4)) * fade * foot;
            (a > 0.03).then_some(Streak { x, y, len, a })
        })
        .collect()
}

/// Where rings spread now and then, in the near band's own pixels (on the
/// water by the jetty's posts and by the boat).
const RIPPLE_SPOTS: [(i32, i32); 5] = [(114, 216), (126, 216), (138, 216), (150, 216), (172, 216)];
/// A ring's life, in seconds, and its widest half-width in art pixels.
const RIPPLE_LIFE: f32 = 2.4;
const RIPPLE_REACH: f32 = 5.;

/// The faint rings on the water `t` seconds in: now and then one spreads
/// from a jetty post or the boat, wide and flat, fading as it grows. None
/// under reduce motion.
pub fn ripples(seed: u64, t: f32, still: bool, l: &Layout) -> Vec<Spark> {
    if still {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (j, (sx, sy)) in RIPPLE_SPOTS.into_iter().enumerate() {
        let r = |k: u64| unit(seed, 3000 + j as u64 * 8 + k);
        let period = 6. + 5. * r(0);
        let phase = t.max(0.) / period + r(1);
        let cycle = phase.floor();
        let into = (phase - cycle) * period;
        // About two cycles in three bring a ring.
        if into > RIPPLE_LIFE || mix(seed ^ mix(77 + j as u64) ^ cycle as u64).is_multiple_of(3) {
            continue;
        }
        let u = into / RIPPLE_LIFE;
        let rx = 1. + (RIPPLE_REACH - 1.) * u;
        let ry = (rx / 3.).max(1.);
        let a = 0.75 * (1. - u);
        let (cx, cy) = (l.fore_x + sx, l.fore_y + sy);
        let reach = rx.round() as i32;
        for dx in -reach..=reach {
            let k = (dx as f32 / rx).clamp(-1., 1.);
            let dy = (ry * (1. - k * k).sqrt()).round() as i32;
            out.push(Spark {
                x: cx + dx,
                y: cy + dy,
                a,
            });
            if dy != 0 {
                out.push(Spark {
                    x: cx + dx,
                    y: cy - dy,
                    a,
                });
            }
        }
    }
    out
}

/// The boat in the near band's own pixels (x, y, width, height), drawn on
/// its own so it can bob; its mooring line stays in the band.
pub const BOAT: (i32, i32, i32, i32) = (161, 208, 24, 7);
/// The boat's slow bob, in seconds a cycle.
const BOB_CYCLE: f32 = 5.;

/// How far up the boat sits `t` seconds in: one art pixel for a part of
/// each slow cycle, else none (and none under reduce motion).
pub fn boat_bob(t: f32, still: bool) -> i32 {
    if still {
        return 0;
    }
    -i32::from((TAU * t / BOB_CYCLE).sin() > 0.35)
}

/// Mist by day: a few long, faint runs over the far shore, drifting
/// slowly; this many rows above the far waterline at most.
const MIST_ROWS: i32 = 7;

/// The day's mist along the far shore `t` seconds in: three or four long
/// faint runs just above the far waterline, drifting slowly right and
/// coming round again; still under reduce motion.
pub fn mist(seed: u64, t: f32, still: bool, l: &Layout) -> Vec<Streak> {
    let t = if still { 0. } else { t.max(0.) };
    let n = 3 + mix(seed ^ 0x3157) % 2;
    let mut out = Vec::new();
    for i in 0..n {
        let r = |k: u64| unit(seed, 4000 + i * 8 + k);
        let len = 30 + (r(0) * 30.) as i32;
        let y = l.horizon - 1 - (r(1) * (MIST_ROWS - 1) as f32) as i32;
        let speed = 0.25 + 0.25 * r(2);
        let span = (l.w + len) as f32;
        let x = ((r(3) * span + speed * t) % span) as i32 - len;
        let a = 0.5 + 0.25 * r(4);
        // Two rows, the upper one shorter and fainter, with soft ends as
        // the clouds have.
        for (dy, share, row_a) in [(0, 1., a), (-1, 0.6, a * 0.55)] {
            let run_len = (len as f32 * share) as i32;
            let start = x + (len - run_len) / 2;
            let end = run_len.min(CLOUD_END * 2) / 2;
            for (from, run, k) in [
                (start, end, 0.5),
                (start + end, run_len - 2 * end, 1.),
                (start + run_len - end, end, 0.5),
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

/// A shooting star now and then at night: one chance a cycle, in a window
/// that keeps two at least a minute apart.
const SHOOT_CYCLE: f32 = 100.;
const SHOOT_FROM: f32 = 30.;
const SHOOT_WINDOW: f32 = 40.;
const SHOOT_LIFE: f32 = 0.5;
/// How far its head travels, in art pixels across.
const SHOOT_TRAVEL: f32 = 16.;

/// The shooting star `t` seconds in, if one is crossing: a 6 to 10 pixel
/// streak sliding down across the upper sky for half a second, brightest
/// at its head, clear of the line. About two cycles in three bring one,
/// so one every one to three minutes. None under reduce motion.
pub fn shooting_star(seed: u64, t: f32, still: bool, l: &Layout) -> Vec<Spark> {
    if still || t < 0. {
        return Vec::new();
    }
    let cycle = (t / SHOOT_CYCLE).floor();
    let s = mix(seed ^ 0x5407 ^ (cycle as u64).wrapping_mul(0x2545));
    if s.is_multiple_of(3) {
        return Vec::new();
    }
    let start = SHOOT_FROM + SHOOT_WINDOW * unit(s, 1);
    let into = t - cycle * SHOOT_CYCLE - start;
    if !(0. ..SHOOT_LIFE).contains(&into) {
        return Vec::new();
    }
    let len = 6 + (unit(s, 2) * 5.) as i32;
    let fall = (SHOOT_TRAVEL + len as f32) / 2.;
    // The rows it may cross: the upper sky, clear of the line.
    let (top, bottom) = (CORNER_ROOM + 2, l.ridge_top() - 14);
    let open: Vec<i32> = (top..bottom - fall as i32)
        .filter(|y| {
            let end = y + fall as i32;
            end < l.line_top - STAR_ROOM || *y > l.line_bottom + STAR_ROOM
        })
        .collect();
    if open.is_empty() {
        return Vec::new();
    }
    let y0 = open[((unit(s, 3) * open.len() as f32) as usize).min(open.len() - 1)] as f32;
    let x0 = l.w as f32 * (0.15 + 0.7 * unit(s, 4));
    let dir = if unit(s, 5) < 0.5 { 1. } else { -1. };
    let u = into / SHOOT_LIFE;
    let (hx, hy) = (x0 + dir * SHOOT_TRAVEL * u, y0 + SHOOT_TRAVEL / 2. * u);
    let fade = (PI * u).sin().sqrt();
    (0..len)
        .map(|k| Spark {
            x: (hx - dir * k as f32).round() as i32,
            y: (hy - k as f32 / 2.).round() as i32,
            a: fade * (1. - k as f32 / len as f32),
        })
        .collect()
}
/// The body's path on the water under `x`, rolled by `roll` (it changes
/// every `motion::SCENE_PATH_MS` so the water shimmers): short streaks
/// from just under the horizon down, wider and fainter with depth.
pub fn path(x: i32, roll: u64, l: &Layout) -> Vec<Streak> {
    let top = l.horizon + 2;
    let bottom = l.horizon + ((l.h - l.horizon) as f32 * 0.55).round().min(48.) as i32;
    let span = (bottom - top).max(1) as f32;
    (top..=bottom)
        .step_by(2)
        .enumerate()
        .filter_map(|(i, y)| {
            let r = |k: u64| unit(roll, i as u64 * 8 + k);
            if r(0) < 0.25 {
                return None;
            }
            let depth = (y - top) as f32 / span;
            let half = 2. + 10. * depth;
            let len = (2. * half * (0.4 + 0.6 * r(1))).round().max(1.);
            let shift = (r(2) - 0.5) * 2. * (1. + 3. * depth);
            let a = (1. - depth).powf(1.2) * (0.45 + 0.55 * r(3));
            Some(Streak {
                x: (x as f32 + shift - len / 2.).round() as i32,
                y,
                len: len as i32,
                a,
            })
        })
        .collect()
}

// ------------------------------------------------------------ particles

/// Where fireflies live, in the near band's own pixels (x0, y0, x1, y1):
/// round the pines' skirts (where they are dense: over land, not water), along the shore and the jetty, by the reeds.
const FLY_PINES: (i32, i32, i32, i32) = (4, 150, 80, 196);
const FLY_SHORE: (i32, i32, i32, i32) = (88, 186, 152, 214);
const FLY_REEDS: (i32, i32, i32, i32) = (190, 196, 236, 232);

/// The fireflies (8 to 12, by the seed) `t` seconds in, by the shore,
/// the jetty and the reeds, and round the pines when `on_pines` (the dark
/// scheme: the light scheme's dark specks would vanish there). `still`
/// (reduce motion): each at its home, lit, whatever `t` is.
pub fn fireflies(seed: u64, t: f32, still: bool, on_pines: bool, l: &Layout) -> Vec<Spark> {
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
                return Spark {
                    x: home_x.round() as i32,
                    y: home_y.round() as i32,
                    a: 0.75,
                };
            }
            // Slow wandering in its zone on a smooth curve (two waves
            // across, two up and down), along its own clock `m`, which
            // eases to a stop now and then and rests a moment.
            let m = fly_clock(seed, i, t);
            let dx = 6. * (m * (0.12 + 0.1 * r(2)) + TAU * r(3)).sin()
                + 2.5 * (m * (0.31 + 0.1 * r(4)) + TAU * r(5)).sin();
            let dy = 3.5 * (m * (0.17 + 0.1 * r(6)) + TAU * r(7)).sin()
                + 1. * (m * (0.43 + 0.1 * r(11)) + TAU * r(12)).sin();
            let x = (home_x + dx).clamp(x0, x1);
            let y = (home_y + dy).clamp(y0, y1);
            // In and out over 2 to 4 s, each at its own phase.
            let period = 2. + 2. * r(8);
            let a = 0.5 - 0.5 * (TAU * (t / period + r(9))).cos();
            Spark {
                x: x.round() as i32,
                y: y.round() as i32,
                a,
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
pub fn fly_clock(seed: u64, i: u64, t: f32) -> f32 {
    let r = |k: u64| unit(seed, 9000 + i * 4 + k);
    let cycle = FLY_CYCLE + FLY_CYCLE_MORE * r(0);
    let go = cycle - (FLY_REST + FLY_REST_MORE * r(1));
    let phase = t.max(0.) / cycle + r(2);
    let n = phase.floor();
    let s = ((phase - n) * cycle / go).min(1.);
    (n + s * s * (3. - 2. * s)) * go
}
/// A distant bird: five pixels in a "v".
pub const BIRD: [(i32, i32); 5] = [(-2, -2), (-1, -1), (0, 0), (1, -1), (2, -2)];

/// The birds' cycle: now and then a flock crosses in the first part.
const BIRD_CYCLE: f32 = 80.;
const BIRD_CROSSING: f32 = 48.;

/// Where the day's birds are `t` seconds in (their centres; none between
/// crossings): two or three gliding slowly across the upper half, under
/// the line, now and then.
pub fn birds(seed: u64, t: f32, l: &Layout) -> Vec<(i32, i32)> {
    let cycle = (t / BIRD_CYCLE).floor().max(0.) as u64;
    let into = t - cycle as f32 * BIRD_CYCLE;
    let s = mix(seed ^ cycle);
    // About two crossings in three cycles.
    if into > BIRD_CROSSING || s.is_multiple_of(3) {
        return Vec::new();
    }
    let n = 2 + (s >> 8) % 2;
    let top = (l.line_bottom + 10) as f32;
    let bottom = (l.h as f32 * 0.45)
        .min((l.ridge_top() - 8) as f32)
        .max(top + 1.);
    let base_y = top + (bottom - top) * unit(s, 1);
    let (from, to) = (-10., (l.w + 10) as f32);
    let x = from + (to - from) * (into / BIRD_CROSSING);
    (0..n)
        .map(|i| {
            let (ox, oy) = [(0., 0.), (-8., 4.), (7., 6.)][i as usize];
            let bob = (t * 0.9 + i as f32).sin();
            ((x + ox).round() as i32, (base_y + oy + bob).round() as i32)
        })
        .collect()
}

/// The lamp's glow reach, in art pixels.
pub const LAMP_GLOW: i32 = 9;

/// The lantern's brightness `t` seconds in: a slow, slight flicker (1
/// when still).
pub fn lantern(t: f32, still: bool) -> f32 {
    if still {
        return 1.;
    }
    0.86 + 0.08 * (t * 1.7).sin() + 0.06 * (t * 4.3 + 1.).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1280 x 800: the panel is 380 x 744 px, 190 x 372 art pixels.
    fn mid() -> Layout {
        layout(190, 372, 12)
    }

    #[test]
    fn the_sun_rises_at_six_tops_at_noon_and_sets_at_six() {
        let l = mid();
        assert_eq!(body_at(6 * 60), (Body::Sun, 0.));
        // At the left, under the horizon.
        let (x, y) = arc(0., &l);
        assert_eq!(x, (190. * ARC_FROM).round() as i32);
        assert!(y > l.horizon);
        let (body, p) = body_at(12 * 60);
        assert_eq!((body, p), (Body::Sun, 0.5));
        assert_eq!(arc(p, &l), (95, l.apex), "the top of the arc");
        assert_eq!(body_at(18 * 60 - 1).0, Body::Sun);
        assert_eq!(body_at(18 * 60), (Body::Moon, 0.));
        assert_eq!(body_at(0).0, Body::Moon);
        assert_eq!(body_at(5 * 60 + 59).0, Body::Moon);
        // The moon tops out at midnight.
        assert_eq!(body_at(0).1, 0.5);
        assert!(arc(1., &l).1 > l.horizon);
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
            let tw = twinkle(7, i, 40., true);
            assert!(tw.light == 1. && !tw.glint);
        }
        assert_eq!(water(7, 0., true, &l), water(7, 93.5, true, &l));
        assert_eq!(mist(7, 0., true, &l), mist(7, 93.5, true, &l));
        for t in [0., 12., 55.25] {
            assert!(ripples(7, t, true, &l).is_empty());
            assert!(shooting_star(7, t, true, &l).is_empty());
            assert_eq!(boat_bob(t, true), 0);
        }
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

    #[test]
    fn fireflies_stay_low_by_the_shore_and_pulse() {
        let l = mid();
        for seed in 0..20 {
            for t in [0., 1.3, 17., 200.] {
                let flies = fireflies(seed, t, false, seed % 2 == 0, &l);
                assert!((8..=12).contains(&flies.len()));
                for f in flies {
                    assert!(f.y as f32 >= l.h as f32 * 0.55 - 1. && f.y < l.h);
                    assert!((0..l.w).contains(&f.x));
                    assert!((0. ..=1.).contains(&f.a));
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
        let water = water(3, 10., false, &l);
        assert!(!water.is_empty());
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

    /// About a quarter of the stars twinkle, each stepping through its
    /// levels on its own 2 to 6 s cycle, now and then in a glint; the rest
    /// hold still.
    #[test]
    fn a_quarter_of_the_stars_twinkle_in_steps() {
        let n = 400;
        let live = (0..n).filter(|i| twinkle(5, *i, 0., false).live).count();
        assert!((n / 6..n / 3).contains(&live), "{live} of {n}");
        let mut glints = 0;
        for i in 0..n {
            let lights: Vec<Twinkle> = (0..240)
                .map(|k| twinkle(5, i, k as f32 * 0.083, false))
                .collect();
            let levels: std::collections::BTreeSet<u32> =
                lights.iter().map(|tw| (tw.light * 100.) as u32).collect();
            if lights[0].live {
                assert!(levels.len() >= 2, "star {i} changes");
                assert!(levels.len() <= 3, "star {i} steps: {levels:?}");
            } else {
                assert_eq!(levels.len(), 1, "star {i} holds still");
            }
            glints += lights.iter().filter(|tw| tw.glint).count();
        }
        assert!(glints > 0, "a glint now and then");
    }

    /// The water moves everywhere: the streaks drift sideways and come and
    /// go, so two moments a few seconds apart differ across the lake.
    #[test]
    fn the_whole_lake_moves() {
        let l = mid();
        let a = water(3, 10., false, &l);
        let b = water(3, 12., false, &l);
        assert_ne!(a, b);
        // In the far and the near halves alike.
        let half = l.horizon + (l.h - l.horizon) / 2;
        for near in [false, true] {
            let pick = |v: &[Streak]| -> Vec<Streak> {
                v.iter()
                    .filter(|s| (s.y >= half) == near)
                    .copied()
                    .collect()
            };
            assert_ne!(pick(&a), pick(&b), "near {near}");
        }
        // From one frame to the next a streak moves a pixel at most.
        let c = water(3, 10. + 0.083, false, &l);
        let kept = a
            .iter()
            .filter(|s| {
                c.iter()
                    .any(|n| n.y == s.y && n.len == s.len && (n.x - s.x).abs() <= 1)
            })
            .count();
        assert!(kept * 10 >= a.len() * 9, "{kept} of {}", a.len());
    }

    /// Rings now and then by the posts and the boat, flat and fading.
    #[test]
    fn ripples_spread_now_and_then_by_the_jetty_and_boat() {
        let l = mid();
        let mut seen = 0;
        for k in 0..600 {
            let rings = ripples(2, k as f32 * 0.25, false, &l);
            if rings.is_empty() {
                continue;
            }
            seen += 1;
            for p in rings {
                assert!(p.y > l.horizon && p.y < l.h && (0. ..=0.75).contains(&p.a));
                assert!(p.x >= l.fore_x + 108 && p.x <= l.fore_x + 178, "{p:?}");
            }
        }
        // Some of the time, not all of it.
        assert!((60..540).contains(&seen), "{seen} of 600");
    }

    #[test]
    fn the_boat_bobs_one_pixel_slowly() {
        let bobs: Vec<i32> = (0..120)
            .map(|k| boat_bob(k as f32 * 0.083, false))
            .collect();
        assert!(bobs.iter().all(|b| (-1..=0).contains(b)));
        assert!(bobs.contains(&-1) && bobs.contains(&0));
        // Slow: it changes only a few times in ten seconds.
        let changes = bobs.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((1..=6).contains(&changes), "{changes}");
    }

    /// Fireflies follow smooth curves (a pixel a frame at most) and now and
    /// then rest a moment.
    #[test]
    fn fireflies_wander_smoothly_and_rest() {
        let l = mid();
        let frame = 0.083;
        for seed in 0..6 {
            let frames: Vec<Vec<Spark>> = (0..1200)
                .map(|k| fireflies(seed, k as f32 * frame, false, true, &l))
                .collect();
            for i in 0..frames[0].len() {
                let (mut still, mut longest) = (0, 0);
                for w in frames.windows(2) {
                    let (a, b) = (w[0][i], w[1][i]);
                    assert!(
                        (a.x - b.x).abs() <= 1 && (a.y - b.y).abs() <= 1,
                        "seed {seed} fly {i}"
                    );
                    still = if (a.x, a.y) == (b.x, b.y) {
                        still + 1
                    } else {
                        0
                    };
                    longest = longest.max(still);
                }
                // A rest of at least a second in the hundred seconds.
                assert!(longest >= 12, "seed {seed} fly {i} rests: {longest}");
            }
        }
        // Its clock never runs back.
        for k in 0..2000 {
            let t = k as f32 * 0.05;
            assert!(fly_clock(4, 2, t + 0.05) >= fly_clock(4, 2, t));
        }
    }

    /// A shooting star at most once a minute, short and clear of the line.
    #[test]
    fn a_shooting_star_now_and_then() {
        let l = mid();
        let step = 0.1;
        let mut starts = Vec::new();
        let mut was = false;
        for k in 0..12_000 {
            let t = k as f32 * step;
            let s = shooting_star(9, t, false, &l);
            for p in &s {
                assert!(p.y < l.ridge_top() && !(l.line_top..=l.line_bottom).contains(&p.y));
            }
            if !s.is_empty() {
                assert!((6..=10).contains(&s.len()));
                if !was {
                    starts.push(t);
                }
            }
            was = !s.is_empty();
        }
        assert!((4..=12).contains(&starts.len()), "{starts:?}");
        for w in starts.windows(2) {
            assert!(w[1] - w[0] >= 60., "{starts:?}");
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
                let on_pines = f.x < l.fore_x + FLY_PINES.2 && f.y < l.fore_y + FLY_SHORE.1;
                assert!(!on_pines, "{f:?}");
            }
        }
    }

    #[test]
    fn the_path_widens_and_fades_with_depth() {
        let l = mid();
        let s = path(95, 3, &l);
        assert!(!s.is_empty());
        for w in s.windows(2) {
            assert!(w[0].y < w[1].y);
        }
        assert!(s.iter().all(|s| s.y > l.horizon && s.y <= l.horizon + 48));
        assert_ne!(path(95, 3, &l), path(95, 4, &l), "another roll");
    }

    #[test]
    fn birds_come_now_and_then_in_the_upper_half() {
        let l = mid();
        let crossings = (0..30)
            .filter(|c| !birds(1, *c as f32 * BIRD_CYCLE + 10., &l).is_empty())
            .count();
        assert!((5..30).contains(&crossings), "{crossings} of 30");
        for c in 0..30 {
            let t = c as f32 * BIRD_CYCLE;
            assert!(birds(1, t + 60., &l).is_empty(), "nothing between");
            for (_, y) in birds(1, t + 10., &l) {
                assert!(y > l.line_bottom && y < l.h / 2, "{y}");
            }
        }
    }
}
