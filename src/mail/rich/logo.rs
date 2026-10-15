//! A signature's logo: one small PNG or JPEG, stored as it was picked.
//! Nothing is decoded or re-encoded: the size is read from the file's
//! header, and the picture is sent as it is.

use super::model::LogoRef;

/// The largest logo file.
pub const MAX_LOGO_BYTES: usize = 200 * 1024;
/// The logo is shown at most this wide and this high, in CSS pixels.
pub const MAX_LOGO_W: u32 = 400;
pub const MAX_LOGO_H: u32 = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoError {
    /// Over [`MAX_LOGO_BYTES`].
    TooLarge,
    /// Not a PNG or JPEG whose header could be read.
    NotAnImage,
}

impl std::fmt::Display for LogoError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LogoError::TooLarge => write!(f, "the logo is larger than 200 KB"),
            LogoError::NotAnImage => write!(f, "the logo is not a PNG or JPEG file"),
        }
    }
}

impl std::error::Error for LogoError {}

/// A checked logo file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logo {
    pub bytes: Vec<u8>,
    /// `image/png` or `image/jpeg`.
    pub mime: &'static str,
    /// Display size: the picture's, scaled down to fit 400 x 150.
    pub width: u16,
    pub height: u16,
}

impl Logo {
    pub fn from_bytes(bytes: &[u8]) -> Result<Logo, LogoError> {
        if bytes.len() > MAX_LOGO_BYTES {
            return Err(LogoError::TooLarge);
        }
        let (mime, w, h) = probe(bytes).ok_or(LogoError::NotAnImage)?;
        let (width, height) = fit(w, h);
        Ok(Logo {
            bytes: bytes.to_vec(),
            mime,
            width,
            height,
        })
    }

    /// The reference a document holds for this logo.
    pub fn reference(&self) -> LogoRef {
        LogoRef::of(&self.bytes, self.width, self.height)
    }
}

/// Scale `w` x `h` down (never up) to fit the display box.
fn fit(w: u32, h: u32) -> (u16, u16) {
    let scale = f64::min(
        1.0,
        f64::min(
            f64::from(MAX_LOGO_W) / f64::from(w.max(1)),
            f64::from(MAX_LOGO_H) / f64::from(h.max(1)),
        ),
    );
    let s = |v: u32| (f64::from(v) * scale).round().max(1.0) as u16;
    (s(w), s(h))
}

/// MIME type and pixel size from a PNG or JPEG header.
pub fn probe(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    if b.get(..8) == Some(b"\x89PNG\r\n\x1a\n") {
        if b.get(12..16) != Some(b"IHDR") {
            return None;
        }
        let w = be32(b, 16)?;
        let h = be32(b, 20)?;
        return (w > 0 && h > 0 && w <= 65_535 && h <= 65_535).then_some(("image/png", w, h));
    }
    if b.get(..2) == Some(&[0xff, 0xd8]) {
        let mut i = 2usize;
        // Walk the marker segments to the first start-of-frame.
        for _ in 0..4096 {
            if b.get(i) != Some(&0xff) {
                return None;
            }
            let mut m = *b.get(i.checked_add(1)?)?;
            let mut at = i.checked_add(2)?;
            while m == 0xff {
                m = *b.get(at)?;
                at = at.checked_add(1)?;
            }
            if (0xd0..=0xd9).contains(&m) || m == 0x01 {
                i = at;
                continue;
            }
            let len = usize::from(be16(b, at)?);
            if matches!(m, 0xc0..=0xcf) && !matches!(m, 0xc4 | 0xc8 | 0xcc) {
                let h = u32::from(be16(b, at.checked_add(3)?)?);
                let w = u32::from(be16(b, at.checked_add(5)?)?);
                return (w > 0 && h > 0).then_some(("image/jpeg", w, h));
            }
            if len < 2 {
                return None;
            }
            i = at.checked_add(len)?;
        }
    }
    None
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([
        *s.first()?,
        *s.get(1)?,
        *s.get(2)?,
        *s.get(3)?,
    ]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    pub fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
        v.extend([0xff, 0xc0, 0, 11, 8]);
        v.extend(h.to_be_bytes());
        v.extend(w.to_be_bytes());
        v.extend([1, 1, 0x11, 0]);
        v
    }

    #[test]
    fn reads_sizes_from_headers() {
        let l = Logo::from_bytes(&png(200, 60)).unwrap();
        assert_eq!((l.mime, l.width, l.height), ("image/png", 200, 60));
        let l = Logo::from_bytes(&jpeg(800, 300)).unwrap();
        assert_eq!((l.mime, l.width, l.height), ("image/jpeg", 400, 150));
        // Scaled down by the tighter side, never up.
        let l = Logo::from_bytes(&png(1000, 1000)).unwrap();
        assert_eq!((l.width, l.height), (150, 150));
        let l = Logo::from_bytes(&png(10, 10)).unwrap();
        assert_eq!((l.width, l.height), (10, 10));
    }

    #[test]
    fn refuses_big_or_foreign_files() {
        let mut big = png(10, 10);
        big.resize(MAX_LOGO_BYTES + 1, 0);
        assert_eq!(Logo::from_bytes(&big), Err(LogoError::TooLarge));
        let mut ok = png(10, 10);
        ok.resize(MAX_LOGO_BYTES, 0);
        assert!(Logo::from_bytes(&ok).is_ok());
        for junk in [
            &b""[..],
            b"GIF89a",
            b"<svg/>",
            b"\x89PNG\r\n\x1a\n",
            b"\xff\xd8",
            b"\xff\xd8\xff",
        ] {
            assert_eq!(
                Logo::from_bytes(junk),
                Err(LogoError::NotAnImage),
                "{junk:?}"
            );
        }
        assert_eq!(Logo::from_bytes(&png(0, 5)), Err(LogoError::NotAnImage));
    }

    #[test]
    fn jpeg_walk_never_loops_or_panics() {
        let mut v = vec![0xff, 0xd8];
        for _ in 0..10_000 {
            v.extend([0xff, 0xe1, 0, 2]);
        }
        assert!(probe(&v).is_none());
        assert!(probe(&[0xff, 0xd8, 0xff, 0xff, 0xff, 0xff]).is_none());
    }
}
