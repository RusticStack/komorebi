//! The background picture's pixels, off the UI thread: decode (PNG, JPEG,
//! WebP), cap at the largest monitor, dim, blur, and the tones the
//! legibility warning reads. The blur is Komorebi's own: a Gaussian
//! approximated by three separable box passes (each pass a running sum, so
//! its cost does not grow with the radius), with the edges clamped.

use std::path::Path;

use image::{DynamicImage, GenericImageView as _, RgbaImage, imageops::FilterType};

use crate::theme::backdrop::{ToneGrid, Tones};
use crate::theme::presets::Rgb;

/// The most bytes a chosen picture may have.
pub const MAX_FILE: u64 = 64 * 1024 * 1024;
/// The most pixels a decoded picture may have (16384 x 16384), so a
/// hostile file cannot make the decoder allocate gigabytes.
const MAX_PIXELS: u64 = 16_384 * 16_384;
/// The pane copy is this much smaller than the picture: it is blurred
/// further, so the detail lost is never seen, and it costs a sixteenth.
pub const SOFT_SCALE: u32 = 4;

/// The cap when no monitor answers.
pub const FALLBACK_CAP: (u32, u32) = (1920, 1080);

/// Why a picture was not taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TooLarge,
    Unreadable,
}

/// Decodes `path` and scales it down (never up) to fit `cap` px, keeping
/// its aspect.
pub fn load(path: &Path, cap: (u32, u32)) -> Result<RgbaImage, Refusal> {
    let meta = std::fs::metadata(path).map_err(|_| Refusal::Unreadable)?;
    if meta.len() > MAX_FILE {
        return Err(Refusal::TooLarge);
    }
    let reader = image::ImageReader::open(path)
        .map_err(|_| Refusal::Unreadable)?
        .with_guessed_format()
        .map_err(|_| Refusal::Unreadable)?;
    let format = reader.format().ok_or(Refusal::Unreadable)?;
    if !matches!(
        format,
        image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
    ) {
        return Err(Refusal::Unreadable);
    }
    let (w, h) = reader.into_dimensions().map_err(|_| Refusal::Unreadable)?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(Refusal::TooLarge);
    }
    let image = image::ImageReader::open(path)
        .map_err(|_| Refusal::Unreadable)?
        .with_guessed_format()
        .map_err(|_| Refusal::Unreadable)?
        .decode()
        .map_err(|_| Refusal::Unreadable)?;
    Ok(cap_to(image, cap))
}

/// `image` scaled down to fit `cap` (never up).
pub fn cap_to(image: DynamicImage, (cw, ch): (u32, u32)) -> RgbaImage {
    let (w, h) = image.dimensions();
    if w <= cw.max(1) && h <= ch.max(1) {
        return image.into_rgba8();
    }
    let s = (f64::from(cw.max(1)) / f64::from(w)).min(f64::from(ch.max(1)) / f64::from(h));
    let (nw, nh) = (
        ((f64::from(w) * s).round() as u32).max(1),
        ((f64::from(h) * s).round() as u32).max(1),
    );
    image
        .resize_exact(nw, nh, FilterType::Triangle)
        .into_rgba8()
}

/// Keeps `image` as a PNG at `target` (written beside it, then moved in
/// place, so a crash never leaves half a picture).
pub fn store(image: &RgbaImage, target: &Path) -> Result<(), Refusal> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|_| Refusal::Unreadable)?;
    }
    let part = target.with_extension("part");
    image
        .save_with_format(&part, image::ImageFormat::Png)
        .map_err(|_| Refusal::Unreadable)?;
    std::fs::rename(&part, target).map_err(|_| Refusal::Unreadable)
}

/// Darkens toward black by `dim` (0 to 1).
pub fn dim(image: &mut RgbaImage, dim: f32) {
    let keep = 1. - dim.clamp(0., 1.);
    if keep >= 1. {
        return;
    }
    for p in image.pixels_mut() {
        for c in &mut p.0[..3] {
            *c = (f32::from(*c) * keep).round() as u8;
        }
    }
}

/// The three box widths whose passes approximate a Gaussian of `sigma`
/// (Kovesi, "Fast almost-Gaussian filtering").
fn boxes(sigma: f32) -> [usize; 3] {
    let n = 3.;
    let ideal = ((12. * sigma * sigma / n) + 1.).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let wl_f = wl as f32;
    let m = ((12. * sigma * sigma - n * wl_f * wl_f - 4. * n * wl_f - 3. * n) / (-4. * wl_f - 4.))
        .round() as i64;
    let mut out = [0usize; 3];
    for (i, o) in out.iter_mut().enumerate() {
        *o = if (i as i64) < m { wl } else { wu } as usize;
    }
    out
}

/// One box pass along rows (`horizontal`) or columns, radius `r`, edges
/// clamped; `src` and `dst` are premultiplied RGBA, `w` x `h` (all four
/// channels are blurred, so clear parts do not darken their neighbours).
fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (lines, len, step, line_step) = if horizontal {
        (h, w, 4, w * 4)
    } else {
        (w, h, w * 4, 4)
    };
    let div = (2 * r + 1) as u32;
    for line in 0..lines {
        let base = line * line_step;
        let at = |i: usize| base + i.min(len - 1) * step;
        let mut sum = [0u32; 4];
        // The window around index 0, its left edge clamped.
        for k in 0..=2 * r {
            let i = at(k.saturating_sub(r));
            for c in 0..4 {
                sum[c] += u32::from(src[i + c]);
            }
        }
        for i in 0..len {
            let o = base + i * step;
            for c in 0..4 {
                dst[o + c] = ((sum[c] + div / 2) / div) as u8;
            }
            let add = at(i + r + 1);
            let sub = at(i.saturating_sub(r));
            for c in 0..4 {
                sum[c] = sum[c] + u32::from(src[add + c]) - u32::from(src[sub + c]);
            }
        }
    }
}

/// A Gaussian blur of `sigma` px, in place.
pub fn blur(image: &mut RgbaImage, sigma: f32) {
    if sigma < 0.5 {
        return;
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0u8; w * h * 4];
    let buf: &mut [u8] = image.as_mut();
    for width in boxes(sigma) {
        let r = (width - 1) / 2;
        box_pass(buf, &mut tmp, w, h, r, true);
        box_pass(&tmp, buf, w, h, r, false);
    }
}

/// How much smaller a picture blurred by `sigma` px may be kept: a pixel
/// of the smaller picture stays under a third of the blur's sigma, and at
/// most 4 times smaller a side.
pub fn shrink_for(sigma: f32) -> u32 {
    ((sigma / 3.).floor() as u32).clamp(1, 4)
}

/// `image`, `by` times smaller a side.
pub fn shrink(image: &RgbaImage, by: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    image::imageops::resize(
        image,
        (w / by.max(1)).max(1),
        (h / by.max(1)).max(1),
        FilterType::Triangle,
    )
}

/// The pane copy (Blur behind panes), made from the stored picture:
/// `SOFT_SCALE` times smaller, blurred by `sigma` px of the picture.
pub fn soften(image: &RgbaImage, sigma: f32) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (sw, sh) = ((w / SOFT_SCALE).max(1), (h / SOFT_SCALE).max(1));
    let mut small = image::imageops::resize(image, sw, sh, FilterType::Triangle);
    blur(&mut small, sigma / SOFT_SCALE as f32);
    small
}

/// The average, darkest and lightest colours of what is drawn (clear parts
/// skipped), sampled on a grid of about 64 x 64 points.
pub fn tones(image: &RgbaImage) -> Tones {
    let (w, h) = image.dimensions();
    tones_in(image, (0, 0), (w, h), 64).unwrap_or(Tones {
        average: Rgb::BLACK,
        darkest: Rgb::BLACK,
        lightest: Rgb::BLACK,
    })
}

/// [`tones`] over a `cols` x `rows` grid of the picture.
pub fn tone_grid(image: &RgbaImage, cols: usize, rows: usize) -> ToneGrid {
    let (w, h) = image.dimensions();
    let mut cells = Vec::with_capacity(cols * rows);
    for r in 0..rows as u32 {
        for c in 0..cols as u32 {
            let from = (c * w / cols as u32, r * h / rows as u32);
            let to = ((c + 1) * w / cols as u32, (r + 1) * h / rows as u32);
            cells.push(tones_in(image, from, to, 24));
        }
    }
    ToneGrid { cols, rows, cells }
}

/// The tones of the rectangle `from`..`to`, sampled about `n` x `n` times
/// (None: nothing drawn there).
fn tones_in(
    image: &RgbaImage,
    (x0, y0): (u32, u32),
    (x1, y1): (u32, u32),
    n: u32,
) -> Option<Tones> {
    let (sx, sy) = (((x1 - x0) / n).max(1), ((y1 - y0) / n).max(1));
    let (mut sum, mut count) = ([0u64; 3], 0u64);
    let (mut dark, mut light) = ((f32::MAX, Rgb::BLACK), (f32::MIN, Rgb::WHITE));
    for y in (y0..y1).step_by(sy as usize) {
        for x in (x0..x1).step_by(sx as usize) {
            let q = image.get_pixel(x, y).0;
            if q[3] < 128 {
                continue;
            }
            let a = u32::from(q[3]);
            let p = [0, 1, 2].map(|c| (u32::from(q[c]) * 255 / a).min(255) as u8);
            for c in 0..3 {
                sum[c] += u64::from(p[c]);
            }
            count += 1;
            let rgb = Rgb::from_bytes(p[0], p[1], p[2]);
            let l = rgb.luminance();
            if l < dark.0 {
                dark = (l, rgb);
            }
            if l > light.0 {
                light = (l, rgb);
            }
        }
    }
    if count == 0 {
        return None;
    }
    let avg = |c: usize| (sum[c] / count) as u8;
    Some(Tones {
        average: Rgb::from_bytes(avg(0), avg(1), avg(2)),
        darkest: dark.1,
        lightest: light.1,
    })
}

/// Premultiplied RGBA to the premultiplied BGRA GPUI's images hold, in one
/// buffer.
pub fn to_bgra(image: RgbaImage) -> (u32, u32, std::sync::Arc<[u8]>) {
    let (w, h) = image.dimensions();
    let mut raw = image.into_raw();
    for p in raw.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    (w, h, raw.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(c))
    }

    #[test]
    fn a_blur_keeps_a_flat_picture_flat_and_spreads_an_edge() {
        let mut flat = solid(40, 30, [90, 120, 200, 255]);
        blur(&mut flat, 6.);
        assert!(flat.pixels().all(|p| p.0 == [90, 120, 200, 255]));
        // A black half and a white half: the edge becomes a ramp, the far
        // ends stay put, and the result is symmetric.
        let mut edge = RgbaImage::from_fn(64, 8, |x, _| {
            if x < 32 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        blur(&mut edge, 4.);
        let at = |x| edge.get_pixel(x, 4).0[0];
        assert_eq!(at(0), 0);
        assert_eq!(at(63), 255);
        assert!(at(30) > 0 && at(30) < 128, "{}", at(30));
        assert!(at(33) > 128 && at(33) < 255, "{}", at(33));
        assert!((i32::from(at(31)) + i32::from(at(32)) - 255).abs() <= 2);
        // Monotonic across the ramp.
        assert!((1..64).all(|x| at(x) >= at(x - 1)));
    }

    #[test]
    fn the_boxes_approximate_the_sigma() {
        for sigma in [1.0f32, 3., 8., 20.] {
            let b = boxes(sigma);
            // Three passes of width w have variance sum((w^2 - 1) / 12).
            let var: f32 = b.iter().map(|w| ((w * w - 1) as f32) / 12.).sum();
            assert!((var.sqrt() - sigma).abs() < 0.6, "{sigma}: {b:?}");
            assert!(b.iter().all(|w| w % 2 == 1));
        }
    }

    #[test]
    fn a_strong_blur_keeps_a_smaller_picture() {
        assert_eq!(shrink_for(0.), 1);
        assert_eq!(shrink_for(2.9), 1);
        assert_eq!(shrink_for(6.), 2);
        assert_eq!(shrink_for(10.), 3);
        assert_eq!(shrink_for(40.), 4);
        assert_eq!(
            shrink(&solid(400, 300, [1, 2, 3, 255]), 4).dimensions(),
            (100, 75)
        );
    }

    #[test]
    fn dimming_darkens_toward_black() {
        let mut img = solid(2, 2, [200, 100, 50, 255]);
        dim(&mut img, 0.5);
        assert_eq!(img.get_pixel(0, 0).0, [100, 50, 25, 255]);
        dim(&mut img, 1.);
        assert_eq!(img.get_pixel(1, 1).0, [0, 0, 0, 255]);
    }

    #[test]
    fn the_cap_scales_down_only_and_keeps_the_aspect() {
        let big = DynamicImage::ImageRgba8(solid(4000, 2000, [1, 2, 3, 255]));
        assert_eq!(cap_to(big, (1920, 1080)).dimensions(), (1920, 960));
        let small = DynamicImage::ImageRgba8(solid(300, 200, [1, 2, 3, 255]));
        assert_eq!(cap_to(small, (1920, 1080)).dimensions(), (300, 200));
    }

    #[test]
    fn tones_find_the_average_and_the_extremes() {
        let img = RgbaImage::from_fn(64, 64, |x, _| {
            if x < 32 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let t = tones(&img);
        assert_eq!(t.darkest, Rgb::BLACK);
        assert_eq!(t.lightest, Rgb::WHITE);
        let a = t.average.bytes();
        assert!((126..=129).contains(&a[0]), "{a:?}");
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let (w, h, bytes) = to_bgra(solid(1, 1, [10, 20, 30, 255]));
        assert_eq!((w, h), (1, 1));
        assert_eq!(&bytes[..], &[30, 20, 10, 255]);
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_refused() {
        let dir = std::env::temp_dir().join(format!("komorebi-pixels-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("not.png");
        std::fs::write(&bad, b"hello").unwrap();
        assert_eq!(load(&bad, (100, 100)), Err(Refusal::Unreadable));
        let good = dir.join("good.png");
        solid(300, 100, [9, 9, 9, 255]).save(&good).unwrap();
        let img = load(&good, (150, 150)).unwrap();
        assert_eq!(img.dimensions(), (150, 50));
        // Stored and read back whole.
        let kept = dir.join("kept").join("background.png");
        store(&img, &kept).unwrap();
        assert_eq!(load(&kept, (4000, 4000)).unwrap(), img);
        assert!(!kept.with_extension("part").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
