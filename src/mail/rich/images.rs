//! The pictures a document shows: their bytes by Content-ID, the size read
//! from a file's header (PNG, JPEG, GIF and WebP; never SVG or anything
//! else), and the sources a pasted picture may come from without the
//! network (a `data:` address, a file on this PC, a picture the document
//! has already). Nothing here decodes pixels or fetches anything.

use super::model::{InlineImage, image_cid};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The largest picture file kept in a message: what the reader's decoder
/// takes ([`crate::mail::render::img::MAX_ENCODED`]).
pub const MAX_IMAGE_BYTES: usize = crate::mail::render::img::MAX_ENCODED;
/// A picture wider than this (pixels) is scaled down to it when it is put
/// in, unless the original is asked for.
pub const DOWNSCALE_W: u32 = 1600;

/// A picture's bytes and their type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFile {
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub mime: &'static str,
    pub data: Arc<[u8]>,
}

impl ImageFile {
    /// A checked picture: `None` for a file that is not a PNG, JPEG, GIF or
    /// WebP whose header can be read, or is over [`MAX_IMAGE_BYTES`].
    pub fn new(data: impl Into<Arc<[u8]>>) -> Option<(ImageFile, u32, u32)> {
        let data: Arc<[u8]> = data.into();
        if data.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let (mime, w, h) = probe(&data)?;
        Some((ImageFile { mime, data }, w, h))
    }

    pub fn cid(&self) -> String {
        image_cid(&self.data)
    }

    /// The file name it is saved and sent under (`image-1a2b3c4d.png`).
    pub fn file_name(&self, cid: &str) -> String {
        let stem: String = cid
            .split('@')
            .next()
            .unwrap_or("image")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(40)
            .collect();
        format!("{stem}.{}", extension(self.mime))
    }
}

/// The pictures a document can show, by Content-ID. Pictures stay here
/// after they leave the document (undo brings them back); only those the
/// document shows are saved and sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Images {
    map: HashMap<String, ImageFile>,
}

impl Images {
    pub fn get(&self, cid: &str) -> Option<&ImageFile> {
        self.map.get(cid)
    }

    pub fn contains(&self, cid: &str) -> bool {
        self.map.contains_key(cid)
    }

    pub fn insert(&mut self, cid: String, file: ImageFile) {
        self.map.insert(cid, file);
    }

    /// Add a picture under its own Content-ID; that Content-ID.
    pub fn add(&mut self, file: ImageFile) -> String {
        let cid = file.cid();
        self.map.entry(cid.clone()).or_insert(file);
        cid
    }

    pub fn extend(&mut self, other: &Images) {
        for (k, v) in &other.map {
            self.map.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ImageFile)> {
        self.map.iter()
    }

    /// Only the pictures in `cids`, in that order.
    pub fn subset(&self, cids: &[String]) -> Vec<(String, ImageFile)> {
        cids.iter()
            .filter_map(|c| Some((c.clone(), self.map.get(c)?.clone())))
            .collect()
    }
}

/// The file extension of a picture type.
pub fn extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// MIME type and pixel size from a PNG, JPEG, GIF or WebP header.
pub fn probe(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    let ok = |w: u32, h: u32| (w > 0 && h > 0 && w <= 65_535 && h <= 65_535).then_some((w, h));
    if b.get(..8) == Some(b"\x89PNG\r\n\x1a\n") {
        if b.get(12..16) != Some(b"IHDR") {
            return None;
        }
        let (w, h) = ok(be32(b, 16)?, be32(b, 20)?)?;
        return Some(("image/png", w, h));
    }
    if b.get(..2) == Some(&[0xff, 0xd8]) {
        let (w, h) = jpeg_size(b)?;
        let (w, h) = ok(w, h)?;
        return Some(("image/jpeg", w, h));
    }
    if b.get(..6) == Some(b"GIF87a") || b.get(..6) == Some(b"GIF89a") {
        let (w, h) = ok(u32::from(le16(b, 6)?), u32::from(le16(b, 8)?))?;
        return Some(("image/gif", w, h));
    }
    if b.get(..4) == Some(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        let (w, h) = match b.get(12..16)? {
            b"VP8 " => {
                // A key frame: its start code, then 14-bit width and height.
                if b.get(23..26) != Some(&[0x9d, 0x01, 0x2a]) {
                    return None;
                }
                (
                    u32::from(le16(b, 26)? & 0x3fff),
                    u32::from(le16(b, 28)? & 0x3fff),
                )
            }
            b"VP8L" => {
                if b.get(20) != Some(&0x2f) {
                    return None;
                }
                let v = u32::from_le_bytes([*b.get(21)?, *b.get(22)?, *b.get(23)?, *b.get(24)?]);
                ((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1)
            }
            b"VP8X" => {
                let w = u32::from_le_bytes([*b.get(24)?, *b.get(25)?, *b.get(26)?, 0]) + 1;
                let h = u32::from_le_bytes([*b.get(27)?, *b.get(28)?, *b.get(29)?, 0]) + 1;
                (w, h)
            }
            _ => return None,
        };
        let (w, h) = ok(w, h)?;
        return Some(("image/webp", w, h));
    }
    None
}

fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
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
            return Some((w, h));
        }
        if len < 2 {
            return None;
        }
        i = at.checked_add(len)?;
    }
    None
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
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

// ---- sources ---------------------------------------------------------------

/// A `data:` address of a picture.
pub fn data_uri(file: &ImageFile) -> String {
    use base64::Engine as _;
    format!(
        "data:{};base64,{}",
        file.mime,
        base64::engine::general_purpose::STANDARD.encode(&file.data)
    )
}

/// The bytes of a base64 `data:` address of a picture type (`None` for
/// any other address, or one that does not decode).
pub fn from_data_uri(src: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let rest = src.trim().strip_prefix("data:")?;
    let (head, body) = rest.split_once(',')?;
    let head = head.to_ascii_lowercase();
    if !head.starts_with("image/") || !head.ends_with(";base64") || head.contains("svg") {
        return None;
    }
    if body.len() > MAX_IMAGE_BYTES / 3 * 4 + 8 {
        return None;
    }
    let clean: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .ok()
}

/// The file on this PC a pasted picture names (`file:///C:/...`, or a
/// drive path as Word writes it). `None` for any other address, and for a
/// network share (reading one could reach a server).
pub fn local_path(src: &str) -> Option<PathBuf> {
    let s = src.trim();
    let path = if let Some(rest) = s
        .strip_prefix("file:///")
        .or_else(|| s.strip_prefix("FILE:///"))
    {
        percent_decode(rest)?.replace('/', "\\")
    } else if s.len() > 3
        && s.as_bytes().get(1) == Some(&b':')
        && matches!(s.as_bytes().get(2), Some(b'\\' | b'/'))
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
    {
        s.replace('/', "\\")
    } else {
        return None;
    };
    let b = path.as_bytes();
    let drive = b.len() > 3
        && b.first().is_some_and(u8::is_ascii_alphabetic)
        && b.get(1) == Some(&b':')
        && b.get(2) == Some(&b'\\');
    (drive && !path.contains("\\\\") && !path.contains("..")).then(|| PathBuf::from(path))
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0usize;
    while let Some(&c) = b.get(i) {
        if c == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A picture found in pasted HTML: its source and the size and text its
/// element gave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub src: String,
    /// The `width` attribute or CSS width, in CSS px.
    pub width: Option<u32>,
    pub alt: String,
}

/// Resolves the pictures of pasted HTML without the network: a `data:`
/// address, a file on this PC (read by `read`), or a `cid:` the document
/// has already. Remote (`http`, `https`) and every other address are
/// dropped (PRIVACY.md: nothing is fetched).
pub struct Resolver<'a> {
    pub known: &'a Images,
    pub read: &'a dyn Fn(&std::path::Path) -> Option<Vec<u8>>,
    /// The pictures read so far, to add to the document's.
    pub found: Images,
}

impl<'a> Resolver<'a> {
    pub fn new(known: &'a Images, read: &'a dyn Fn(&std::path::Path) -> Option<Vec<u8>>) -> Self {
        Resolver {
            known,
            read,
            found: Images::default(),
        }
    }

    /// The picture `f` names, or `None` (it is left out).
    pub fn resolve(&mut self, f: &Found) -> Option<InlineImage> {
        let src = f.src.trim();
        let (cid, iw, ih) = if let Some(cid) = src
            .strip_prefix("cid:")
            .or_else(|| src.strip_prefix("CID:"))
        {
            let file = self.known.get(cid).or_else(|| self.found.get(cid))?;
            let (_, w, h) = probe(&file.data)?;
            (cid.to_string(), w, h)
        } else {
            let bytes = match from_data_uri(src) {
                Some(b) => b,
                None => (self.read)(&local_path(src)?)?,
            };
            return self.resolve_bytes(bytes, f);
        };
        Some(Self::sized(InlineImage::new(cid, iw, ih, f.alt.clone()), f))
    }

    /// The picture `f` names, its bytes already read by the caller (a
    /// signature's picture beside its file): `None` when they are not a
    /// picture this app keeps.
    pub fn resolve_bytes(&mut self, bytes: Vec<u8>, f: &Found) -> Option<InlineImage> {
        let (file, w, h) = ImageFile::new(bytes)?;
        let cid = file.cid();
        if !self.known.contains(&cid) {
            self.found.insert(cid.clone(), file);
        }
        Some(Self::sized(InlineImage::new(cid, w, h, f.alt.clone()), f))
    }

    /// `img` at the width its element gave, if any.
    fn sized(img: InlineImage, f: &Found) -> InlineImage {
        let mut img = img;
        if let Some(w) = f.width.filter(|w| *w > 0) {
            img = img.sized(u16::try_from(w).unwrap_or(u16::MAX));
        }
        img
    }
}

/// Reads a file of at most [`MAX_IMAGE_BYTES`] (the resolver's `read` for
/// a real paste).
pub fn read_small_file(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut data = Vec::new();
    f.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= MAX_IMAGE_BYTES).then_some(data)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
pub(crate) mod tests {
    use super::*;

    pub fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    pub fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
        v.extend([0xff, 0xc0, 0, 11, 8]);
        v.extend(h.to_be_bytes());
        v.extend(w.to_be_bytes());
        v.extend([1, 1, 0x11, 0]);
        v
    }

    #[test]
    fn reads_sizes_from_headers_of_the_four_types() {
        assert_eq!(probe(&png(200, 60)), Some(("image/png", 200, 60)));
        assert_eq!(probe(&jpeg(800, 300)), Some(("image/jpeg", 800, 300)));
        let mut gif = b"GIF89a".to_vec();
        gif.extend(320u16.to_le_bytes());
        gif.extend(240u16.to_le_bytes());
        assert_eq!(probe(&gif), Some(("image/gif", 320, 240)));
        // VP8X: 24-bit width - 1 and height - 1.
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        webp.extend([99, 0, 0, 49, 0, 0]);
        assert_eq!(probe(&webp), Some(("image/webp", 100, 50)));
    }

    #[test]
    fn refuses_svg_and_junk() {
        for junk in [
            &b""[..],
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            b"<?xml version='1.0'?><svg/>",
            b"\x89PNG\r\n\x1a\n",
            b"\xff\xd8",
            b"GIF89a",
            b"RIFF\0\0\0\0WEBP",
        ] {
            assert_eq!(probe(junk), None, "{junk:?}");
        }
        assert_eq!(probe(&png(0, 5)), None);
        assert!(from_data_uri("data:image/svg+xml;base64,PHN2Zy8+").is_none());
        assert!(from_data_uri("data:text/html;base64,PHN2Zy8+").is_none());
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

    #[test]
    fn data_uris_round_trip_and_local_paths_stay_local() {
        let (file, _, _) = ImageFile::new(png(4, 2)).unwrap();
        let uri = data_uri(&file);
        assert!(uri.starts_with("data:image/png;base64,"));
        assert_eq!(from_data_uri(&uri).unwrap(), &file.data[..]);
        assert_eq!(
            local_path("file:///C:/Users/Someone/AppData/Local/Temp/msohtml1/01/clip_image002.png"),
            Some(PathBuf::from(
                "C:\\Users\\Someone\\AppData\\Local\\Temp\\msohtml1\\01\\clip_image002.png"
            ))
        );
        assert_eq!(
            local_path("file:///C:/My%20Pictures/a.png"),
            Some(PathBuf::from("C:\\My Pictures\\a.png"))
        );
        assert_eq!(
            local_path("C:\\a\\b.jpg"),
            Some(PathBuf::from("C:\\a\\b.jpg"))
        );
        for remote in [
            "https://example.com/a.png",
            "http://example.com/a.png",
            "file://server/share/a.png",
            "\\\\server\\share\\a.png",
            "file:///C:/a/../../b.png",
            "//example.com/a.png",
        ] {
            assert_eq!(local_path(remote), None, "{remote}");
        }
    }

    #[test]
    fn the_resolver_reads_only_what_needs_no_network() {
        let known = Images::default();
        let disk = png(30, 10);
        let read = move |p: &std::path::Path| {
            (p == std::path::Path::new("C:\\x\\a.png")).then(|| disk.clone())
        };
        let mut r = Resolver::new(&known, &read);
        let (file, _, _) = ImageFile::new(png(8, 8)).unwrap();
        let inline = r
            .resolve(&Found {
                src: data_uri(&file),
                width: Some(4),
                alt: "dot".into(),
            })
            .unwrap();
        assert_eq!(
            (inline.iw, inline.width, inline.alt.as_str()),
            (8, 8, "dot")
        );
        let local = r
            .resolve(&Found {
                src: "file:///C:/x/a.png".into(),
                ..Found::default()
            })
            .unwrap();
        assert_eq!((local.iw, local.ih, local.width), (30, 10, 30));
        assert!(
            r.resolve(&Found {
                src: "https://example.com/tracker.png".into(),
                ..Found::default()
            })
            .is_none()
        );
        assert_eq!(r.found.len(), 2);
        // A cid the document already has, by its own name.
        let again = r
            .resolve(&Found {
                src: format!("cid:{}", local.cid),
                ..Found::default()
            })
            .unwrap();
        assert_eq!(again.cid, local.cid);
    }
}
