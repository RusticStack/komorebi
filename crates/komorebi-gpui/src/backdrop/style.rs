//! The background picture's styles and its vignette, made once off the UI
//! thread at the window's own size in physical px, so a cell is a whole
//! number of device pixels from the window's top-left corner and the image
//! is drawn 1:1 (no smoothing between cells).
//!
//! Pixels here are premultiplied RGBA (a transparent pixel is all zeros):
//! framing leaves what the picture does not cover transparent, and the
//! vignette and the styles' blanks are transparency too, so the window's
//! own background (the theme's canvas, or Mica or Acrylic) shows there.
//!
//! - Pixel art: each cell is the average of its part of the picture,
//!   reduced to a palette made from the picture (median cut), optionally
//!   with ordered (Bayer) dithering between palette colours, then upscaled
//!   without smoothing.
//! - ASCII: each cell (twice as tall as wide, as a monospace character is)
//!   becomes a character from [`RAMP`] by its brightness, drawn from a tiny
//!   built-in bitmap font rasterised once per make, in the cell's own
//!   colour or one ink.
//! - Vignette: the picture fades out toward the window's edges along a
//!   rounded rectangle that follows the window. Without a style it is a
//!   smooth fade; in pixel art whole cells drop out by ordered dithering;
//!   in ASCII the characters thin to lighter ones and then to blanks.

use gpui::{Bounds, Point, px, size};
use image::{Rgba, RgbaImage, imageops::FilterType};

use crate::theme::backdrop::{Fit, place};

/// The ASCII ramp, from blank to densest (each glyph inks at least as much
/// as the one before: `the_ramp_gets_denser`).
pub const RAMP: &[u8] = b" .:-=+*%#@";

/// The ramp's 5 x 7 bitmaps (a set bit is ink; bit 4 is the left column).
const GLYPHS: [[u8; 7]; 10] = [
    [0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0b00100],
    [0, 0, 0b00100, 0, 0, 0b00100, 0],
    [0, 0, 0, 0b01110, 0, 0, 0],
    [0, 0, 0b01110, 0, 0b01110, 0, 0],
    [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
    [0, 0b00100, 0b10101, 0b01110, 0b10101, 0b00100, 0],
    [
        0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011,
    ],
    [0, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0],
    [
        0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01111,
    ],
];

/// The 8 x 8 ordered-dither (Bayer) threshold of cell (`x`, `y`), in
/// (0, 1): the bits of `x ^ y` and `y`, interleaved and reversed, give each
/// of the 64 cells of the tile its own rank, spread evenly.
pub fn bayer(x: u32, y: u32) -> f32 {
    let (xy, y) = ((x ^ y) & 7, y & 7);
    let mut rank = 0;
    for k in 0..3 {
        rank |= ((xy >> k) & 1) << (2 * (2 - k) + 1);
        rank |= ((y >> k) & 1) << (2 * (2 - k));
    }
    (rank as f32 + 0.5) / 64.
}

/// The picture placed for `fit` in a `w` x `h` frame (the window, in
/// physical px); what it does not cover is transparent.
pub fn frame(src: &RgbaImage, fit: Fit, (w, h): (u32, u32)) -> RgbaImage {
    let mut out = RgbaImage::new(w.max(1), h.max(1));
    let (sw, sh) = src.dimensions();
    if sw == 0 || sh == 0 {
        return out;
    }
    let area = Bounds {
        origin: Point::default(),
        size: size(px(w as f32), px(h as f32)),
    };
    for rect in place(fit, size(sw as f32, sh as f32), area) {
        let seen = rect.intersect(&area);
        let (vx, vy) = (f32::from(seen.origin.x), f32::from(seen.origin.y));
        let (vw, vh) = (f32::from(seen.size.width), f32::from(seen.size.height));
        if vw < 1. || vh < 1. {
            continue;
        }
        // The part of the source this visible part shows.
        let (rx, ry) = (f32::from(rect.origin.x), f32::from(rect.origin.y));
        let (kx, ky) = (
            sw as f32 / f32::from(rect.size.width),
            sh as f32 / f32::from(rect.size.height),
        );
        let sx = (((vx - rx) * kx).floor() as u32).min(sw - 1);
        let sy = (((vy - ry) * ky).floor() as u32).min(sh - 1);
        let cw = ((vw * kx).round() as u32).clamp(1, sw - sx);
        let ch = ((vh * ky).round() as u32).clamp(1, sh - sy);
        let part = image::imageops::crop_imm(src, sx, sy, cw, ch).to_image();
        let (tw, th) = (vw.round() as u32, vh.round() as u32);
        let part = if (cw, ch) == (tw, th) {
            part
        } else {
            image::imageops::resize(&part, tw.max(1), th.max(1), FilterType::Triangle)
        };
        image::imageops::replace(&mut out, &part, vx.round() as i64, vy.round() as i64);
    }
    out
}

/// Premultiplies `image`'s colours by its alpha, in place.
pub fn premultiply(image: &mut RgbaImage) {
    for p in image.pixels_mut() {
        let a = u32::from(p.0[3]);
        if a == 255 {
            continue;
        }
        for c in &mut p.0[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// How much of the picture shows at (`x`, `y`) of a `w` x `h` window, 0 to
/// 1: 1 deep inside, falling toward the edges along a rounded rectangle
/// whose band is `size` (0 to 1) of half the shorter side; `amount` (0 to 1)
/// is how far it falls.
pub fn vignette_at(x: f32, y: f32, (w, h): (f32, f32), amount: f32, size: f32) -> f32 {
    if amount <= 0. {
        return 1.;
    }
    let (hw, hh) = (w / 2., h / 2.);
    let band = (size.clamp(0., 1.) * hw.min(hh)).max(1.);
    let (qx, qy) = ((x - hw).abs() - (hw - band), (y - hh).abs() - (hh - band));
    // The signed distance to the rounded rectangle (corner radius `band`)
    // that touches the window's edges: negative inside.
    let outside = qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - band;
    let t = (-outside / band).clamp(0., 1.);
    let smooth = t * t * (3. - 2. * t);
    1. - amount.clamp(0., 1.) * (1. - smooth)
}

/// The smooth vignette (no style): every pixel fades by [`vignette_at`].
pub fn vignette(image: &mut RgbaImage, amount: f32, size: f32) {
    if amount <= 0. {
        return;
    }
    let (w, h) = image.dimensions();
    let dims = (w as f32, h as f32);
    for (x, y, p) in image.enumerate_pixels_mut() {
        let v = vignette_at(x as f32 + 0.5, y as f32 + 0.5, dims, amount, size);
        if v >= 1. {
            continue;
        }
        for c in &mut p.0 {
            *c = (f32::from(*c) * v).round() as u8;
        }
    }
}

/// The picture in cells of `cw` x `ch` px: each cell's average colour
/// (straight, not premultiplied) and coverage, row by row.
pub struct Cells {
    pub cols: u32,
    pub rows: u32,
    pub cw: u32,
    pub ch: u32,
    /// Straight colour and alpha per cell.
    pub cells: Vec<[u8; 4]>,
}

/// Averages `image` (premultiplied) into cells of `cw` x `ch` from the
/// top-left corner; the last row and column may be partial.
pub fn cells(image: &RgbaImage, cw: u32, ch: u32) -> Cells {
    let (w, h) = image.dimensions();
    let (cw, ch) = (cw.max(1), ch.max(1));
    let (cols, rows) = (w.div_ceil(cw), h.div_ceil(ch));
    let mut sums = vec![[0u64; 5]; (cols * rows) as usize];
    for (x, y, p) in image.enumerate_pixels() {
        let s = &mut sums[((y / ch) * cols + x / cw) as usize];
        for (sum, v) in s.iter_mut().zip(p.0) {
            *sum += u64::from(v);
        }
        s[4] += 1;
    }
    let cells = sums
        .into_iter()
        .map(|s| {
            let n = s[4].max(1);
            let a = s[3];
            let straight = |c: u64| (c * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
            [
                straight(s[0]),
                straight(s[1]),
                straight(s[2]),
                (a / n) as u8,
            ]
        })
        .collect();
    Cells {
        cols,
        rows,
        cw,
        ch,
        cells,
    }
}

/// Up to `n` colours standing for `colours` (median cut: the box with the
/// widest channel is split at its median until there are `n`).
pub fn palette(colours: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    if colours.is_empty() || n == 0 {
        return Vec::new();
    }
    let mut boxes: Vec<Vec<[u8; 3]>> = vec![colours.to_vec()];
    while boxes.len() < n {
        // The box with the widest channel range.
        let widest = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let (c, r) = (0..3)
                    .map(|c| {
                        let (lo, hi) = b
                            .iter()
                            .fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[c]), hi.max(p[c])));
                        (c, hi - lo)
                    })
                    .max_by_key(|(_, r)| *r)
                    .unwrap_or((0, 0));
                (i, c, r)
            })
            .max_by_key(|(_, _, r)| *r);
        let Some((i, c, r)) = widest else { break };
        if r == 0 {
            break;
        }
        let mut b = boxes.swap_remove(i);
        b.sort_unstable_by_key(|p| p[c]);
        let upper = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(upper);
    }
    boxes
        .iter()
        .map(|b| {
            let n = b.len().max(1) as u64;
            let mut s = [0u64; 3];
            for p in b {
                for c in 0..3 {
                    s[c] += u64::from(p[c]);
                }
            }
            [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
        })
        .collect()
}

/// The palette colour nearest `c` (weighted for the eye).
fn nearest(palette: &[[u8; 3]], c: [f32; 3]) -> [u8; 3] {
    let d = |p: &[u8; 3]| {
        let e = |i: usize, w: f32| {
            let k = f32::from(p[i]) - c[i];
            w * k * k
        };
        e(0, 2.) + e(1, 4.) + e(2, 3.)
    };
    palette
        .iter()
        .min_by(|a, b| d(a).total_cmp(&d(b)))
        .copied()
        .unwrap_or([0, 0, 0])
}

/// How far a styled cell is drawn toward a light theme's canvas: a little
/// everywhere, and most where the vignette is fading it out, so the last
/// cells sit near the canvas and text over them stays legible. Dark themes
/// keep the picture's own colours.
pub fn wash_share(shown: f32) -> f32 {
    0.15 + 0.85 * (1. - shown.clamp(0., 1.))
}

fn washed(c: [u8; 3], ground: Option<[u8; 3]>, shown: f32) -> [u8; 3] {
    let Some(g) = ground else {
        return c;
    };
    let w = wash_share(shown);
    [0, 1, 2].map(|i| (f32::from(c[i]) * (1. - w) + f32::from(g[i]) * w).round() as u8)
}

/// Pixel art: cells of `cell` px reduced to `colours` colours from the
/// picture, dithered when `dither`; with a vignette, whole cells drop out
/// toward the edges by ordered dithering. On a light theme's canvas
/// (`wash`) cells are drawn toward it ([`wash_share`]).
pub fn pixel_art(
    image: &RgbaImage,
    cell: u32,
    colours: usize,
    dither: bool,
    (amount, size): (f32, f32),
    wash: Option<[u8; 3]>,
) -> RgbaImage {
    let (w, h) = image.dimensions();
    let grid = cells(image, cell, cell);
    let solid: Vec<[u8; 3]> = grid
        .cells
        .iter()
        .filter(|c| c[3] >= 128)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let pal = palette(&solid, colours);
    // Ordered dithering spreads each cell by about one palette step.
    let spread = if dither {
        255. / (colours.max(2) as f32).cbrt() * 0.6
    } else {
        0.
    };
    let dims = (w as f32, h as f32);
    let mut out = RgbaImage::new(w, h);
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            let c = grid.cells[(row * grid.cols + col) as usize];
            if c[3] < 128 {
                continue;
            }
            let (cx, cy) = (
                (col * cell) as f32 + cell as f32 / 2.,
                (row * cell) as f32 + cell as f32 / 2.,
            );
            let threshold = bayer(col, row);
            let shown = vignette_at(cx, cy, dims, amount, size);
            if shown <= threshold {
                continue;
            }
            let off = (threshold - 0.5) * spread;
            let ink = nearest(
                &pal,
                [
                    f32::from(c[0]) + off,
                    f32::from(c[1]) + off,
                    f32::from(c[2]) + off,
                ],
            );
            let ink = washed(ink, wash, shown);
            let px = Rgba([ink[0], ink[1], ink[2], 255]);
            for y in row * cell..((row + 1) * cell).min(h) {
                for x in col * cell..((col + 1) * cell).min(w) {
                    out.put_pixel(x, y, px);
                }
            }
        }
    }
    out
}

/// Relative brightness of a straight colour, 0 to 1.
fn brightness(c: [u8; 4]) -> f32 {
    (0.2126 * f32::from(c[0]) + 0.7152 * f32::from(c[1]) + 0.0722 * f32::from(c[2])) / 255.
}

/// The ramp index for a cell of `brightness` (0 to 1) shown `shown` (the
/// vignette, 0 to 1): on a dark ground bright cells are dense; on a light
/// ground (`light`) dark cells are.
pub fn ramp_index(brightness: f32, shown: f32, light: bool) -> usize {
    let ink = if light { 1. - brightness } else { brightness };
    let level = (ink.clamp(0., 1.) * shown.clamp(0., 1.)) * (RAMP.len() - 1) as f32;
    (level.round() as usize).min(RAMP.len() - 1)
}

/// Glyph `g`'s coverage in a `cw` x `ch` cell (4 x 4 samples a pixel; the
/// glyph sits in a 6 x 9 unit box: one unit of spacing right, one above and
/// one below).
fn raster(g: usize, cw: u32, ch: u32) -> Vec<u8> {
    let bits = GLYPHS[g];
    let mut out = Vec::with_capacity((cw * ch) as usize);
    for y in 0..ch {
        for x in 0..cw {
            let mut hit = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let ux = (x as f32 + (sx as f32 + 0.5) / 4.) / cw as f32 * 6.;
                    let uy = (y as f32 + (sy as f32 + 0.5) / 4.) / ch as f32 * 9. - 1.;
                    if (0. ..5.).contains(&ux) && (0. ..7.).contains(&uy) {
                        let (col, row) = (ux as usize, uy as usize);
                        if bits[row] & (0b10000 >> col) != 0 {
                            hit += 1;
                        }
                    }
                }
            }
            out.push((hit * 255 / 16) as u8);
        }
    }
    out
}

/// How much ink glyph `g` puts in a cell (tests: the ramp gets denser).
pub fn ink_of(g: usize) -> u32 {
    raster(g, 12, 24).iter().map(|&c| u32::from(c)).sum()
}

/// ASCII: characters in cells `cell` px wide and twice as tall, chosen by
/// brightness; coloured from the picture (`ink` None, brightened on a dark
/// ground and darkened on a light one) or all in `ink`; with a vignette the
/// characters thin out toward the edges. `intensity` (0 to 1) makes the
/// strokes bolder and lays a little of each character's colour under it, so
/// the picture's shapes still read behind a translucent pane. The ground is
/// transparent; on a light theme's canvas (`wash`) the characters are drawn
/// toward it.
pub fn ascii(
    image: &RgbaImage,
    cell: u32,
    ink: Option<[u8; 3]>,
    intensity: f32,
    (amount, size): (f32, f32),
    wash: Option<[u8; 3]>,
) -> RgbaImage {
    let light = wash.is_some();
    let (w, h) = image.dimensions();
    let (cw, ch) = (cell.max(5), cell.max(5) * 2);
    let grid = cells(image, cw, ch);
    let atlas: Vec<Vec<u8>> = (0..RAMP.len()).map(|g| raster(g, cw, ch)).collect();
    let dims = (w as f32, h as f32);
    let levels = levels(&grid);
    let intensity = intensity.clamp(0., 1.);
    // Coverage gain: 1 (as drawn) to 3 (antialiased edges filled in).
    let gain = 1. + 2. * intensity;
    let mut out = RgbaImage::new(w, h);
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            let c = grid.cells[(row * grid.cols + col) as usize];
            if c[3] < 128 {
                continue;
            }
            let shown = vignette_at(
                (col * cw) as f32 + cw as f32 / 2.,
                (row * ch) as f32 + ch as f32 / 2.,
                dims,
                amount,
                size,
            );
            let g = ramp_index(levels(brightness(c)), shown, light);
            if g == 0 {
                continue;
            }
            let colour = washed(
                ink.unwrap_or_else(|| vivid([c[0], c[1], c[2]], light)),
                wash,
                shown,
            );
            let glyph = &atlas[g];
            // The cell's own wash of colour, more for denser characters.
            let back = intensity * 0.45 * g as f32 / (RAMP.len() - 1) as f32;
            for y in 0..ch {
                let py = row * ch + y;
                if py >= h {
                    break;
                }
                for x in 0..cw {
                    let px = col * cw + x;
                    if px >= w {
                        break;
                    }
                    let a = (f32::from(glyph[(y * cw + x) as usize]) / 255. * gain).min(1.);
                    // The character over its cell's wash (premultiplied).
                    let a = a + back * (1. - a);
                    if a <= 0. {
                        continue;
                    }
                    let pre = |v: u8| (f32::from(v) * a).round() as u8;
                    out.put_pixel(
                        px,
                        py,
                        Rgba([
                            pre(colour[0]),
                            pre(colour[1]),
                            pre(colour[2]),
                            (a * 255.).round() as u8,
                        ]),
                    );
                }
            }
        }
    }
    out
}

/// The picture's own brightness range (2nd to 98th percentile of its
/// cells) stretched over the whole ramp, so a dark photograph still uses
/// every character; a flat picture is left as it is.
fn levels(grid: &Cells) -> impl Fn(f32) -> f32 + use<> {
    let mut b: Vec<f32> = grid
        .cells
        .iter()
        .filter(|c| c[3] >= 128)
        .map(|c| brightness(*c))
        .collect();
    b.sort_unstable_by(f32::total_cmp);
    let at = |q: f32| b.get(((b.len() as f32 - 1.) * q) as usize).copied();
    let (lo, hi) = match (at(0.02), at(0.98)) {
        (Some(lo), Some(hi)) if hi - lo > 0.05 => (lo, hi),
        _ => (0., 1.),
    };
    move |v| ((v - lo) / (hi - lo)).clamp(0., 1.)
}

/// A cell's colour made to read as ink: brightened on a dark ground (up to
/// four times, its strongest channel at full), darkened on a light one.
fn vivid(c: [u8; 3], light: bool) -> [u8; 3] {
    let top = f32::from(c[0].max(c[1]).max(c[2])).max(1.);
    let k = if light {
        (140. / top).min(1.)
    } else {
        (255. / top).min(4.)
    };
    c.map(|v| (f32::from(v) * k).round().min(255.) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 120, 255])
        })
    }

    #[test]
    fn pixel_art_cells_are_whole_and_few_coloured() {
        for (cell, n) in [(6u32, 8usize), (9, 16), (4, 32)] {
            let out = pixel_art(&ramp(120, 90), cell, n, true, (0., 0.), None);
            // Every cell is one colour, aligned to the top-left corner.
            for (x, y, p) in out.enumerate_pixels() {
                let first = out.get_pixel(x / cell * cell, y / cell * cell);
                assert_eq!(p, first, "cell {cell} at {x},{y}");
            }
            let mut seen: Vec<[u8; 4]> = out.pixels().map(|p| p.0).collect();
            seen.sort_unstable();
            seen.dedup();
            assert!(seen.len() <= n, "{n} colours asked, {} used", seen.len());
            assert!(seen.len() > 1);
        }
    }

    #[test]
    fn the_palette_has_at_most_the_size_asked() {
        let colours: Vec<[u8; 3]> = (0..=255u8).map(|v| [v, 255 - v, v / 2]).collect();
        for n in [8, 16, 32, 64] {
            assert_eq!(palette(&colours, n).len(), n);
        }
        // Fewer distinct colours than asked: no more than there are.
        assert_eq!(palette(&[[1, 2, 3], [1, 2, 3]], 16).len(), 1);
        assert!(palette(&[], 8).is_empty());
    }

    #[test]
    fn the_ramp_gets_denser() {
        let ink: Vec<u32> = (0..RAMP.len()).map(ink_of).collect();
        assert_eq!(ink[0], 0);
        assert!(ink.windows(2).all(|w| w[1] > w[0]), "{ink:?}");
    }

    #[test]
    fn brightness_picks_the_character() {
        assert_eq!(RAMP[ramp_index(0., 1., false)], b' ');
        assert_eq!(RAMP[ramp_index(1., 1., false)], b'@');
        // On a light ground dark is dense.
        assert_eq!(RAMP[ramp_index(0., 1., true)], b'@');
        assert_eq!(RAMP[ramp_index(1., 1., true)], b' ');
        // Brighter never picks a lighter character.
        let picks: Vec<usize> = (0..=20)
            .map(|i| ramp_index(i as f32 / 20., 1., false))
            .collect();
        assert!(picks.windows(2).all(|w| w[1] >= w[0]));
        // Fading out thins the character, then blanks it.
        assert!(ramp_index(1., 0.5, false) < ramp_index(1., 1., false));
        assert_eq!(ramp_index(1., 0., false), 0);
    }

    #[test]
    fn the_vignette_falls_toward_the_edges_along_the_window() {
        let dims = (400., 200.);
        // From the middle out to each edge, never rising.
        for (dx, dy) in [(1., 0.), (0., 1.), (2., 1.)] {
            let mut last = f32::MAX;
            for i in 0..=100 {
                let t = i as f32 / 100.;
                let (x, y) = (200. + dx * 100. * t, 100. + dy * 100. * t * 0.999);
                let v = vignette_at(x.min(400.), y.min(200.), dims, 1., 0.5);
                assert!(v <= last + 1e-6, "rises at {x},{y}");
                last = v;
            }
        }
        assert!((vignette_at(200., 100., dims, 1., 0.5) - 1.).abs() < 1e-6);
        assert!(vignette_at(0., 100., dims, 1., 0.5) < 0.01);
        // The fade follows the window: as deep from the left as from the
        // top at the same distance in (not a circle).
        let left = vignette_at(20., 100., dims, 1., 0.5);
        let top = vignette_at(200., 20., dims, 1., 0.5);
        assert!((left - top).abs() < 1e-4, "{left} {top}");
        // Off is off.
        assert_eq!(vignette_at(0., 0., dims, 0., 0.5), 1.);
    }

    #[test]
    fn pixel_art_cells_drop_out_whole_toward_the_edges() {
        let flat = RgbaImage::from_pixel(240, 160, Rgba([200, 120, 60, 255]));
        let out = pixel_art(&flat, 8, 8, false, (1., 0.6), None);
        // No partial alpha: a cell is there or it is not.
        assert!(out.pixels().all(|p| p.0[3] == 0 || p.0[3] == 255));
        // Sparser toward the edge: the share of cells kept in rings.
        let share = |ring: u32| {
            let (mut kept, mut all) = (0, 0);
            for row in 0..20u32 {
                for col in 0..30u32 {
                    let d = col.min(29 - col).min(row.min(19 - row));
                    if d == ring {
                        all += 1;
                        if out.get_pixel(col * 8 + 4, row * 8 + 4).0[3] == 255 {
                            kept += 1;
                        }
                    }
                }
            }
            kept as f32 / all as f32
        };
        assert!(
            share(0) < share(3) && share(3) <= share(8),
            "{} {} {}",
            share(0),
            share(3),
            share(8)
        );
        assert_eq!(share(9), 1.);
    }

    #[test]
    fn ascii_thins_toward_the_edges_on_a_clear_ground() {
        let flat = RgbaImage::from_pixel(300, 200, Rgba([255, 255, 255, 255]));
        let out = ascii(&flat, 6, None, 0., (1., 0.8), None);
        let ink = |x0: u32, y0: u32| -> u32 {
            (0..6)
                .flat_map(|x| (0..12).map(move |y| (x, y)))
                .map(|(x, y)| u32::from(out.get_pixel(x0 + x, y0 + y).0[3]))
                .sum()
        };
        // The middle cell is the densest; the corner cell is blank.
        let (mid, edge) = (ink(150, 96), ink(0, 0));
        assert_eq!(edge, 0);
        assert!(mid > ink(36, 96) && ink(36, 96) >= ink(6, 96), "{mid}");
        // Between the glyphs the ground is clear.
        assert!(out.pixels().any(|p| p.0[3] == 0));
    }

    /// Intensity makes the characters bolder and washes their cells with
    /// their colour, so they still read behind a translucent pane.
    #[test]
    fn intensity_makes_the_characters_read_stronger() {
        let flat = RgbaImage::from_pixel(120, 120, Rgba([200, 120, 60, 255]));
        let total = |i: f32| -> u64 {
            ascii(&flat, 8, None, i, (0., 0.), None)
                .pixels()
                .map(|p| u64::from(p.0[3]))
                .sum()
        };
        assert!(total(0.5) > total(0.) && total(1.) > total(0.5));
        // At full intensity every pixel of a dense cell carries colour.
        let out = ascii(&flat, 8, None, 1., (0., 0.), None);
        assert!((0..8).all(|x| (0..16).all(|y| out.get_pixel(40 + x, 32 + y).0[3] > 0)));
        // Still premultiplied: no channel above the alpha.
        assert!(out.pixels().all(|p| p.0[..3].iter().all(|&v| v <= p.0[3])));
    }

    #[test]
    fn framing_fills_and_crops_to_the_window() {
        let src = RgbaImage::from_pixel(400, 100, Rgba([10, 20, 30, 255]));
        let filled = frame(&src, Fit::Fill, (200, 200));
        assert_eq!(filled.dimensions(), (200, 200));
        assert!(filled.pixels().all(|p| p.0 == [10, 20, 30, 255]));
        // Fit leaves bands above and below clear.
        let fitted = frame(&src, Fit::Fit, (200, 200));
        assert_eq!(fitted.get_pixel(100, 5).0[3], 0);
        assert_eq!(fitted.get_pixel(100, 100).0, [10, 20, 30, 255]);
    }

    #[test]
    fn light_themes_draw_the_fading_cells_toward_the_canvas() {
        let flat = RgbaImage::from_pixel(240, 160, Rgba([40, 40, 60, 255]));
        let canvas = [244, 244, 244];
        let out = pixel_art(&flat, 8, 8, false, (1., 0.6), Some(canvas));
        let lum = |p: &Rgba<u8>| u32::from(p.0[0]) + u32::from(p.0[1]) + u32::from(p.0[2]);
        let middle = lum(out.get_pixel(120, 80));
        // A kept cell near the edge is nearer the canvas than the middle.
        let edge = (0..30u32)
            .flat_map(|c| [12, 20, 28].map(|y| (c * 8 + 4, y)))
            .map(|(x, y)| out.get_pixel(x, y))
            .filter(|p| p.0[3] == 255)
            .map(lum)
            .max()
            .expect("a kept cell");
        assert!(edge > middle + 60, "{edge} {middle}");
        // Dark themes keep the picture's own colour.
        let dark = pixel_art(&flat, 8, 8, false, (1., 0.6), None);
        assert_eq!(dark.get_pixel(120, 80).0, [40, 40, 60, 255]);
        assert!(wash_share(1.) < wash_share(0.5) && wash_share(0.5) < wash_share(0.));
    }

    #[test]
    fn the_bayer_tile_ranks_each_cell_once() {
        let mut ranks: Vec<u32> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (bayer(x, y) * 64.) as u32))
            .collect();
        assert_eq!(bayer(0, 0), 0.5 / 64.);
        assert_eq!(bayer(8, 3), bayer(0, 3));
        ranks.sort_unstable();
        assert_eq!(ranks, (0..64).collect::<Vec<_>>());
    }

    #[test]
    fn premultiplying_scales_by_alpha() {
        let mut img = RgbaImage::from_pixel(1, 1, Rgba([200, 100, 50, 128]));
        premultiply(&mut img);
        assert_eq!(img.get_pixel(0, 0).0, [100, 50, 25, 128]);
    }
}
