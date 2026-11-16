//! Pictures put in a formatted message (dropped, pasted, picked): read and
//! checked off the UI thread, a large one scaled down, a pasted bitmap
//! saved as PNG. The decoders and encoders are the `image` crate's that
//! GPUI already links (no crate added for this, docs/SCOPE.md's size
//! budget).
//!
//! The downscale rule: a picture wider than [`DOWNSCALE_W`] px (1600) is
//! scaled to 1600 px wide, its aspect ratio kept, when it is put in. A JPEG
//! stays a JPEG (quality [`JPEG_QUALITY`]); a PNG, WebP or bitmap becomes a
//! PNG; a GIF is kept as it is (its frames would be lost). The file as it
//! was is kept in memory beside the draft, and "Use original picture"
//! (More > Picture) puts it back; only the picture shown is saved and sent.

use std::path::{Path, PathBuf};

use komorebi::files::ClipboardPicture;
use komorebi::mail::rich::images::{DOWNSCALE_W, ImageFile, MAX_IMAGE_BYTES, read_small_file};

/// A scaled-down JPEG's quality.
pub const JPEG_QUALITY: u8 = 85;

/// A picture ready to go in: the file the message holds and its size,
/// and the file as it was when it was scaled down.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub file: ImageFile,
    pub iw: u32,
    pub ih: u32,
    pub original: Option<(ImageFile, u32, u32)>,
}

/// Whether a file's name says it is a picture the message can show (PNG,
/// JPEG, GIF, WebP; never SVG). The bytes are checked again when read.
pub fn is_picture_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "webp"
        )
    })
}

/// Split dropped or pasted paths: the pictures (to go in the text) and the
/// rest (attached).
pub fn split_paths(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    paths.into_iter().partition(|p| is_picture_path(p))
}

/// Read a picture file and prepare it (slow: never on the UI thread).
/// `None`: not a picture the message can show, or larger than
/// [`MAX_IMAGE_BYTES`].
pub fn read(path: &Path) -> Option<Prepared> {
    prepare(read_small_file(path)?)
}

/// A picture's bytes prepared: checked, then scaled down when wider than
/// [`DOWNSCALE_W`] (see the module's rule).
pub fn prepare(bytes: Vec<u8>) -> Option<Prepared> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let (file, iw, ih) = ImageFile::new(bytes)?;
    if iw <= DOWNSCALE_W || file.mime == "image/gif" {
        return Some(Prepared {
            file,
            iw,
            ih,
            original: None,
        });
    }
    match downscale(&file, iw, ih) {
        Some((small, w, h)) => Some(Prepared {
            file: small,
            iw: w,
            ih: h,
            original: Some((file, iw, ih)),
        }),
        // It does not decode here: kept as it is (the view says so).
        None => Some(Prepared {
            file,
            iw,
            ih,
            original: None,
        }),
    }
}

fn downscale(file: &ImageFile, iw: u32, ih: u32) -> Option<(ImageFile, u32, u32)> {
    let img = image::load_from_memory(&file.data).ok()?;
    let h = (u64::from(ih) * u64::from(DOWNSCALE_W) / u64::from(iw.max(1))).max(1);
    let small = img.resize_exact(
        DOWNSCALE_W,
        u32::try_from(h).ok()?,
        image::imageops::FilterType::Triangle,
    );
    let mut out = Vec::new();
    if file.mime == "image/jpeg" {
        let rgb = small.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .encode_image(&rgb)
            .ok()?;
    } else {
        small
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .ok()?;
    }
    ImageFile::new(out)
}

/// A clipboard picture as a file the message can hold: its PNG as it is,
/// a bitmap saved as PNG (slow: never on the UI thread).
pub fn from_clipboard(pic: ClipboardPicture) -> Option<Prepared> {
    match pic {
        ClipboardPicture::Png(png) => prepare(png),
        ClipboardPicture::Bmp(bmp) => {
            let img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?;
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .ok()?;
            prepare(out)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn encoded(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 120, 40]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), format)
            .unwrap();
        out
    }

    #[test]
    fn only_pictures_the_message_shows_go_in_the_text() {
        let (pics, rest) = split_paths(
            ["a.PNG", "b.jpeg", "c.gif", "d.webp", "e.svg", "f.pdf", "g"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        );
        assert_eq!(pics.len(), 4);
        assert_eq!(rest, [PathBuf::from("e.svg"), "f.pdf".into(), "g".into()]);
        assert!(prepare(b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec()).is_none());
    }

    /// A picture wider than 1600 px is scaled to 1600, its kind and aspect
    /// kept; the original stays beside it. A small one is kept as it is.
    #[test]
    fn a_large_picture_is_scaled_down_and_its_original_kept() {
        let big = prepare(encoded(3200, 1000, image::ImageFormat::Jpeg)).unwrap();
        assert_eq!((big.iw, big.ih), (1600, 500));
        assert_eq!(big.file.mime, "image/jpeg");
        let (orig, ow, oh) = big.original.unwrap();
        assert_eq!((orig.mime, ow, oh), ("image/jpeg", 3200, 1000));
        let png = prepare(encoded(2000, 2000, image::ImageFormat::Png)).unwrap();
        assert_eq!((png.iw, png.ih, png.file.mime), (1600, 1600, "image/png"));
        let small = prepare(encoded(640, 480, image::ImageFormat::Png)).unwrap();
        assert_eq!((small.iw, small.ih), (640, 480));
        assert!(small.original.is_none());
    }

    /// A screenshot on the clipboard as a bitmap goes in as a PNG.
    #[test]
    fn a_clipboard_bitmap_goes_in_as_png() {
        let bmp = encoded(30, 20, image::ImageFormat::Bmp);
        // A clipboard DIB is the BMP file less its 14-byte header.
        let dib = komorebi::files::dib_to_bmp(&bmp[14..]).unwrap();
        let p = from_clipboard(ClipboardPicture::Bmp(dib)).unwrap();
        assert_eq!((p.iw, p.ih, p.file.mime), (30, 20, "image/png"));
        let png = encoded(5, 5, image::ImageFormat::Png);
        let p = from_clipboard(ClipboardPicture::Png(png)).unwrap();
        assert_eq!((p.iw, p.ih), (5, 5));
    }
}
