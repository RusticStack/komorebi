//! The Komorebi 1.0 brand assets (docs/BRANDING.md): the ICO files carry the
//! sizes Windows asks for in the formats it expects and load through the
//! Windows API, and the GPUI view assets in assets/brand stay small.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One ICO directory entry: its size in px and whether it is a PNG.
#[derive(Debug, PartialEq)]
struct Entry {
    size: u32,
    png: bool,
}

/// Parses an ICO, checking the container and every image's own header.
fn entries(path: &Path) -> Vec<Entry> {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    assert_eq!(
        (u16_at(0), u16_at(2)),
        (0, 1),
        "{}: not an icon",
        path.display()
    );
    let mut out = Vec::new();
    for i in 0..u16_at(4) as usize {
        let o = 6 + i * 16;
        let size = if b[o] == 0 { 256 } else { u32::from(b[o]) };
        assert_eq!(b[o], b[o + 1], "square");
        assert_eq!(u16_at(o + 6), 32, "32 bits per pixel");
        let (len, off) = (u32_at(o + 8) as usize, u32_at(o + 12) as usize);
        assert!(off + len <= b.len(), "image inside the file");
        let img = &b[off..off + len];
        let png = img.starts_with(b"\x89PNG\r\n\x1a\n");
        if png {
            let w = u32::from_be_bytes([img[16], img[17], img[18], img[19]]);
            let h = u32::from_be_bytes([img[20], img[21], img[22], img[23]]);
            assert_eq!((w, h), (size, size), "PNG size");
        } else {
            let dib = |o: usize| i32::from_le_bytes([img[o], img[o + 1], img[o + 2], img[o + 3]]);
            assert_eq!(dib(0), 40, "BITMAPINFOHEADER");
            assert_eq!((dib(4), dib(8)), (size as i32, 2 * size as i32), "DIB size");
            let mask_row = size.div_ceil(32) as usize * 4;
            assert_eq!(
                len,
                40 + (size * size * 4) as usize + mask_row * size as usize
            );
        }
        out.push(Entry { size, png });
    }
    out
}

/// Loads every size of `path` through LoadImageW, as the shell and the
/// window class do.
#[cfg(windows)]
fn loads(path: &Path, sizes: &[u32]) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, HICON, IMAGE_ICON, LR_LOADFROMFILE, LoadImageW,
    };
    use windows::core::HSTRING;
    let name = HSTRING::from(path.as_os_str());
    for &s in sizes {
        // SAFETY: a file load into a fresh icon that is destroyed below.
        let icon =
            unsafe { LoadImageW(None, &name, IMAGE_ICON, s as i32, s as i32, LR_LOADFROMFILE) }
                .unwrap_or_else(|e| panic!("{} at {s}: {e}", path.display()));
        // SAFETY: the icon came from LoadImageW above and is not shared.
        unsafe { DestroyIcon(HICON(icon.0)) }.expect("DestroyIcon");
    }
}

#[test]
fn app_icon_has_every_size_with_a_png_256() {
    let path = repo().join("assets/komorebi.ico");
    let got = entries(&path);
    let want: Vec<Entry> = [16, 20, 24, 32, 40, 48, 64, 128, 256]
        .into_iter()
        .map(|size| Entry {
            size,
            png: size == 256,
        })
        .collect();
    assert_eq!(got, want);
    #[cfg(windows)]
    loads(&path, &[16, 20, 24, 32, 40, 48, 64, 128, 256]);
}

#[test]
fn tray_icons_have_the_small_icon_sizes() {
    for state in ["light", "dark", "unread-light", "unread-dark"] {
        let path = repo().join(format!("assets/tray/tray-{state}.ico"));
        let got = entries(&path);
        let want: Vec<Entry> = [16, 20, 24, 32]
            .into_iter()
            .map(|size| Entry { size, png: false })
            .collect();
        assert_eq!(got, want, "{state}");
        #[cfg(windows)]
        loads(&path, &[16, 20, 24, 32]);
    }
}

#[test]
fn brand_view_assets_are_listed_present_and_small() {
    let dir = repo().join("assets/brand");
    let index = std::fs::read_to_string(dir.join("index.json")).expect("index.json");
    let mut listed = 0;
    for key in ["\"file\": \"", "\"x1\": \"", "\"x2\": \""] {
        for part in index.split(key).skip(1) {
            let file = &part[..part.find('"').expect("closing quote")];
            assert!(
                dir.join(file).is_file(),
                "{file} is listed in index.json but missing"
            );
            listed += 1;
        }
    }
    assert!(listed >= 7 + 2 * 9, "index.json lists {listed} files");
    let mut total = 0;
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).expect("read_dir") {
            let e = e.expect("entry");
            if e.file_type().expect("type").is_dir() {
                stack.push(e.path());
            } else {
                total += e.metadata().expect("metadata").len();
            }
        }
    }
    assert!(
        total < 150 * 1024,
        "assets/brand is {total} bytes, over 150 KB"
    );
}

#[test]
fn installer_images_have_inno_setup_sizes() {
    for (file, w, h) in [
        ("wizard-image.bmp", 164, 314),
        ("wizard-image-2x.bmp", 328, 628),
        ("wizard-small.bmp", 55, 58),
        ("wizard-small-2x.bmp", 110, 116),
    ] {
        let b = std::fs::read(repo().join("installer").join(file)).expect(file);
        let i32_at = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        assert_eq!(&b[..2], b"BM", "{file}");
        assert_eq!((i32_at(18), i32_at(22)), (w, h), "{file}");
        assert_eq!(u16::from_le_bytes([b[28], b[29]]), 24, "{file}: 24-bit");
    }
}
