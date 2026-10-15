//! Image decoding (png, zune-jpeg, gif first frame, image-webp; all already
//! linked by Slint) to premultiplied tiny-skia pixmaps. The format is
//! sniffed from magic bytes, never from the declared MIME type, so SVG (or
//! HTML labelled image/png) is rejected. Dimensions are checked from the
//! header before any pixel buffer is allocated.

use std::io::Cursor;
use tiny_skia::Pixmap;

pub struct ImageData {
    pub pix: Pixmap,
    pub nat_w: u32,
    pub nat_h: u32,
    /// Dark marks on a transparent ground (a black logo): in a dimmed
    /// message it is painted on a light plate, or it would vanish into the
    /// dark page.
    pub dark_on_clear: bool,
}

/// Whether pixels with a clear ground and dark marks need a light plate:
/// at least a tenth of them are see-through (alpha under 128), and the
/// rest are dark on average (luma under 96 of 255). Counts from
/// [`decode`]'s single pass.
pub fn needs_plate(total: u64, clear: u64, solid: u64, luma_sum: u64) -> bool {
    total > 0
        && clear.saturating_mul(10) >= total
        && solid > 0
        && luma_sum < solid.saturating_mul(96)
}

/// Encoded bytes accepted per image.
pub const MAX_ENCODED: usize = 10 * 1024 * 1024;
/// Pixels per image, checked from the header.
pub const MAX_PIXELS: u64 = 16_000_000;
/// Decoded (and resampled) pixels per message.
pub const MAX_TOTAL_PIXELS: u64 = 64_000_000;

/// Decode `bytes`, charging the pixels to `budget`; big images are
/// downscaled once towards `target_w` (0: natural size).
pub fn decode(bytes: &[u8], target_w: u32, budget: &mut u64) -> Option<ImageData> {
    if bytes.len() > MAX_ENCODED {
        return None;
    }
    let mut check = |w: u32, h: u32| -> bool {
        let px = u64::from(w).saturating_mul(u64::from(h));
        let ok = w > 0 && h > 0 && px <= MAX_PIXELS && px <= *budget;
        if ok {
            *budget = budget.saturating_sub(px);
        }
        ok
    };
    let (w, h, rgba) = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        decode_png(bytes, &mut check)?
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(bytes, &mut check)?
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        decode_gif(bytes, &mut check)?
    } else if bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        decode_webp(bytes, &mut check)?
    } else {
        return None;
    };
    let mut pix = Pixmap::new(w, h)?;
    let (mut clear, mut solid, mut luma) = (0u64, 0u64, 0u64);
    for (dst, src) in pix
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_chunks::<4>().0)
    {
        let a = u32::from(src[3]);
        let pm = |c: u8| (u32::from(c).wrapping_mul(a).wrapping_add(127) / 255) as u8;
        *dst = [pm(src[0]), pm(src[1]), pm(src[2]), src[3]];
        // At most 16 M pixels of luma 255 each: the sums never wrap.
        if a < 128 {
            clear = clear.wrapping_add(1);
        } else {
            solid = solid.wrapping_add(1);
            // Rec. 601 luma, in integers (at most 255).
            let y = u32::from(src[0])
                .wrapping_mul(299)
                .wrapping_add(u32::from(src[1]).wrapping_mul(587))
                .wrapping_add(u32::from(src[2]).wrapping_mul(114))
                / 1000;
            luma = luma.wrapping_add(u64::from(y));
        }
    }
    let total = u64::from(w).saturating_mul(u64::from(h));
    let dark_on_clear = needs_plate(total, clear, solid, luma);
    // Pre-scale big images once to their display size (area average).
    if target_w > 0 && u64::from(target_w).saturating_mul(3) / 2 < u64::from(w) {
        let th = u64::from(h)
            .saturating_mul(u64::from(target_w))
            .checked_div(u64::from(w))
            .unwrap_or(1)
            .max(1);
        if let Some(p) = box_down(&pix, target_w, u32::try_from(th).unwrap_or(1)) {
            pix = p;
        }
    }
    Some(ImageData {
        pix,
        nat_w: w,
        nat_h: h,
        dark_on_clear,
    })
}

/// Resample to exactly (tw, th): area average when shrinking, bilinear when
/// growing.
pub fn resample(src: &Pixmap, tw: u32, th: u32) -> Option<Pixmap> {
    if tw == 0 || th == 0 || u64::from(tw).saturating_mul(u64::from(th)) > 4_000_000 {
        return None;
    }
    if tw <= src.width() && th <= src.height() {
        return box_down(src, tw, th);
    }
    let mut out = Pixmap::new(tw, th)?;
    let sx = tw as f32 / src.width() as f32;
    let sy = th as f32 / src.height() as f32;
    let paint = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..Default::default()
    };
    out.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &paint,
        tiny_skia::Transform::from_scale(sx, sy),
        None,
    );
    Some(out)
}

fn box_down(src: &Pixmap, tw: u32, th: u32) -> Option<Pixmap> {
    let mut out = Pixmap::new(tw, th)?;
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (tw, th) = (tw as usize, th as usize);
    let s = src.data().as_chunks::<4>().0;
    let d = out.data_mut().as_chunks_mut::<4>().0;
    let span = |i: usize, src_len: usize, dst_len: usize| {
        let a = i.saturating_mul(src_len).checked_div(dst_len).unwrap_or(0);
        let b = i
            .saturating_add(1)
            .saturating_mul(src_len)
            .div_ceil(dst_len.max(1))
            .min(src_len)
            .max(a.saturating_add(1));
        a..b
    };
    for (y, row) in d.chunks_exact_mut(tw).enumerate().take(th) {
        let ys = span(y, sh, th);
        for (x, px) in row.iter_mut().enumerate() {
            let xs = span(x, sw, tw);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in ys.clone() {
                let start = yy.saturating_mul(sw);
                let line = s
                    .get(start.saturating_add(xs.start)..start.saturating_add(xs.end))
                    .unwrap_or(&[]);
                for p in line {
                    for (a, &c) in acc.iter_mut().zip(p) {
                        *a = a.saturating_add(u32::from(c));
                    }
                    n = n.saturating_add(1);
                }
            }
            *px = acc.map(|a| a.checked_div(n).unwrap_or(0) as u8);
        }
    }
    Some(out)
}

type Checked<'a> = &'a mut dyn FnMut(u32, u32) -> bool;

fn decode_png(bytes: &[u8], check: Checked) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let (w, h) = (r.info().width, r.info().height);
    if !check(w, h) {
        return None;
    }
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let data = buf.get(..info.buffer_size())?;
    let mut out = Vec::with_capacity((w as usize).saturating_mul(h as usize).saturating_mul(4));
    match info.color_type {
        png::ColorType::Rgba => out.extend_from_slice(data),
        png::ColorType::Rgb => {
            for p in data.as_chunks::<3>().0 {
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &g in data {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in data.as_chunks::<2>().0 {
                out.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        png::ColorType::Indexed => return None,
    }
    Some((info.width, info.height, out))
}

fn decode_jpeg(bytes: &[u8], check: Checked) -> Option<(u32, u32, Vec<u8>)> {
    use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    d.decode_headers().ok()?;
    let info = d.info()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    if !check(w, h) {
        return None;
    }
    let px = d.decode().ok()?;
    let need = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    (px.len() >= need).then_some((w, h, px))
}

fn decode_gif(bytes: &[u8], check: Checked) -> Option<(u32, u32, Vec<u8>)> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let limit = std::num::NonZeroU64::new(MAX_PIXELS.saturating_mul(4))?;
    opts.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    let mut d = opts.read_info(Cursor::new(bytes)).ok()?;
    let (sw, sh) = (u32::from(d.width()), u32::from(d.height()));
    if !check(sw, sh) {
        return None;
    }
    // First frame only (no animation).
    let f = d.read_next_frame().ok()??;
    let (sw, sh) = (sw as usize, sh as usize);
    let mut out = vec![0u8; sw.saturating_mul(sh).saturating_mul(4)];
    let (fw, fl, ft) = (
        usize::from(f.width),
        usize::from(f.left),
        usize::from(f.top),
    );
    if fw == 0 {
        return None;
    }
    for (y, row) in f.buffer.chunks(fw.saturating_mul(4)).enumerate() {
        let dy = ft.saturating_add(y);
        if dy >= sh {
            break;
        }
        let dx = fl.min(sw);
        let n = row.len().min(sw.saturating_sub(dx).saturating_mul(4));
        let at = dy.saturating_mul(sw).saturating_add(dx).saturating_mul(4);
        if let (Some(dst), Some(src)) = (out.get_mut(at..at.saturating_add(n)), row.get(..n)) {
            dst.copy_from_slice(src);
        }
    }
    Some((sw as u32, sh as u32, out))
}

fn decode_webp(bytes: &[u8], check: Checked) -> Option<(u32, u32, Vec<u8>)> {
    let mut d = image_webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (w, h) = d.dimensions();
    if !check(w, h) {
        return None;
    }
    let mut buf = vec![0u8; d.output_buffer_size()?];
    d.read_image(&mut buf).ok()?;
    if d.has_alpha() {
        return Some((w, h, buf));
    }
    let mut out = Vec::with_capacity((buf.len() / 3).saturating_mul(4));
    for p in buf.as_chunks::<3>().0 {
        out.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    Some((w, h, out))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_plate_only_for_dark_marks_on_a_clear_ground() {
        // A black logo on a transparent ground.
        assert!(needs_plate(100, 60, 40, 40 * 10));
        // A white logo on a transparent ground: readable on a dark page.
        assert!(!needs_plate(100, 60, 40, 40 * 250));
        // An opaque photo, and a dark one with only a few clear corners.
        assert!(!needs_plate(100, 0, 100, 100 * 20));
        assert!(!needs_plate(100, 4, 96, 96 * 20));
        // All see-through: nothing to show.
        assert!(!needs_plate(100, 100, 0, 0));
        assert!(!needs_plate(0, 0, 0, 0));
    }

    #[test]
    fn rejects_non_images_and_oversized_headers() {
        let mut budget = MAX_TOTAL_PIXELS;
        assert!(decode(b"<svg onload=x>", 0, &mut budget).is_none());
        assert!(decode(b"<html>", 0, &mut budget).is_none());
        // A PNG header claiming 20000 x 20000 is refused before decoding.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&20000u32.to_be_bytes());
        png.extend_from_slice(&20000u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&[0; 4]);
        assert!(decode(&png, 0, &mut budget).is_none());
        assert_eq!(budget, MAX_TOTAL_PIXELS);
    }
}
