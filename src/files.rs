//! Files in and out of the app: the file list of a copy in Explorer
//! (CF_HDROP, for pasting attachments), and attachments opened from the
//! reader: a copy beside the cache marked as downloaded (Mark of the Web),
//! never for types that run code.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Types that run code when opened. Open refuses them (Save still works),
/// whatever comes before the last dot (`invoice.pdf.exe`).
const RISKY: &[&str] = &[
    "exe",
    "com",
    "scr",
    "bat",
    "cmd",
    "ps1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "msi",
    "msp",
    "lnk",
    "hta",
    "cpl",
    "pif",
    "reg",
    "jar",
    "url",
    "scf",
    "msc",
    "application",
    "appref-ms",
    "settingcontent-ms",
];

/// The longest path Open writes (MAX_PATH less its NUL), in UTF-16 units,
/// so every app can open the copy.
const MAX_PATH_UNITS: usize = 259;

/// Whether opening `name` could run a program.
pub fn risky(name: &str) -> bool {
    name.trim_end_matches(['.', ' '])
        .rsplit_once('.')
        .is_some_and(|(_, ext)| RISKY.iter().any(|r| ext.eq_ignore_ascii_case(r)))
}

/// Where a cache keeps the attachments it opened: `cache.open` beside it
/// (`%LOCALAPPDATA%\komorebi\cache.open`), cleared at the next start.
pub fn open_dir(db_path: &Path) -> PathBuf {
    db_path.with_extension("open")
}

/// Remove what an earlier run opened (folders older than `before`, so a
/// copy opened since this start stays). Runs off the UI thread.
pub fn clear_open_dir(dir: &Path, before: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < before);
        if old {
            // A copy still open in another app stays until the next start.
            std::fs::remove_dir_all(entry.path()).ok();
        }
    }
}

/// Write attachment `name` (already sanitized, `mime::sanitize_filename`)
/// to a new folder under `dir`, marked as downloaded from the internet.
pub fn write_open_copy(dir: &Path, name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    let folder = dir.join(format!("{:016x}", random()));
    let used = folder.as_os_str().encode_wide_len() + 1;
    let name = fit_name(name, MAX_PATH_UNITS.saturating_sub(used))
        .ok_or_else(|| std::io::Error::other("the path is too long"))?;
    // Cutting a long name must not leave an extension that runs code.
    if risky(&name) {
        return Err(std::io::Error::other("the name is too long"));
    }
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(name);
    std::fs::write(&path, data)?;
    // Without the mark the copy is still opened, as Save would leave it.
    mark_downloaded(&path).ok();
    Ok(path)
}

/// How opening an attachment ended (see [`open_attachment`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The app for its type was started on the copy.
    Started,
    /// The type runs code: nothing was written. Save still works.
    Refused,
    /// The copy was written, but no app opens this type.
    NoApp,
    /// The copy could not be written (the system's words).
    Failed(String),
}

/// Open attachment `name` (sanitized) with its default app: a copy under
/// `dir` ([`open_dir`]) marked as downloaded, never for types that run
/// code. Writing can take a moment and the shell can block (DDE, a slow
/// handler), so call it off the UI thread; the caller words the result.
/// Both apps use it (the Slint app's open and the GPUI reader's
/// `OpenAttachment`).
pub fn open_attachment(dir: &Path, name: &str, data: &[u8]) -> Opened {
    open_attachment_with(dir, name, data, |path| {
        crate::reader::shell_execute(path.as_os_str())
    })
}

/// [`open_attachment`] with the launcher given (`true`: an app started), so
/// the steps are testable without starting one.
pub fn open_attachment_with(
    dir: &Path,
    name: &str,
    data: &[u8],
    launch: impl FnOnce(&Path) -> bool,
) -> Opened {
    if risky(name) {
        return Opened::Refused;
    }
    match write_open_copy(dir, name, data) {
        Ok(path) if launch(&path) => Opened::Started,
        Ok(_) => Opened::NoApp,
        Err(e) => Opened::Failed(e.to_string()),
    }
}

/// Save attachment bytes where the person chose (`path`), marked as
/// downloaded as a browser download is. Call it off the UI thread.
pub fn save_attachment(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    // Without the mark the file is still saved (FAT and some shares have no
    // alternate streams).
    mark_downloaded(path).ok();
    Ok(())
}

/// Mark `path` as downloaded from the internet: a `Zone.Identifier`
/// stream (zone 3), so Windows and apps such as Office open it with care.
/// Fails on file systems without alternate streams (FAT, some shares).
pub fn mark_downloaded(path: &Path) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
}

/// `name` cut to at most `max` UTF-16 units, keeping a short extension.
/// `None` if not even a few characters fit.
fn fit_name(name: &str, max: usize) -> Option<String> {
    let units = |s: &str| s.encode_utf16().count();
    if units(name) <= max {
        return Some(name.to_string());
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.len() <= 16 => (s, e),
        _ => (name, ""),
    };
    let room = max.checked_sub(if ext.is_empty() { 0 } else { units(ext) + 1 })?;
    if room < 8 {
        return None;
    }
    let mut cut = String::new();
    let mut used = 0;
    for c in stem.chars() {
        used += c.len_utf16();
        if used > room {
            break;
        }
        cut.push(c);
    }
    let cut = cut.trim_end_matches(['.', ' ']);
    Some(if ext.is_empty() {
        cut.to_string()
    } else {
        format!("{cut}.{ext}")
    })
}

/// A folder name nobody can guess ahead (std's randomly keyed hasher over
/// the time).
fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish()
}

trait WideLen {
    fn encode_wide_len(&self) -> usize;
}

impl WideLen for std::ffi::OsStr {
    fn encode_wide_len(&self) -> usize {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            self.encode_wide().count()
        }
        #[cfg(not(windows))]
        {
            self.to_string_lossy().encode_utf16().count()
        }
    }
}

/// The files in a CF_HDROP clipboard block: a `DROPFILES` header (offset
/// of the list, a point, two flags; the last says UTF-16), then the paths,
/// each NUL-terminated, ending with an empty one. Anything malformed ends
/// the list.
pub fn hdrop_paths(block: &[u8]) -> Vec<PathBuf> {
    let u32_at = |i: usize| {
        block
            .get(i..i + 4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
    };
    let (Some(offset), Some(wide)) = (u32_at(0), u32_at(16)) else {
        return Vec::new();
    };
    let Some(list) = usize::try_from(offset).ok().and_then(|o| block.get(o..)) else {
        return Vec::new();
    };
    if wide != 0 {
        let units: Vec<u16> = list
            .chunks_exact(2)
            .filter_map(|c| <[u8; 2]>::try_from(c).ok())
            .map(u16::from_le_bytes)
            .collect();
        units
            .split(|u| *u == 0)
            .take_while(|p| !p.is_empty())
            .map(|p| PathBuf::from(String::from_utf16_lossy(p)))
            .collect()
    } else {
        // The ANSI form (old programs): only plain ASCII paths are certain.
        list.split(|b| *b == 0)
            .take_while(|p| !p.is_empty())
            .filter(|p| p.is_ascii())
            .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
            .collect()
    }
}

/// The files on the clipboard (copied in Explorer), if any.
#[cfg(windows)]
pub fn clipboard_files() -> Vec<PathBuf> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    /// `CF_HDROP` (its constant lives in the Ole feature).
    const CF_HDROP: u32 = 15;
    struct Close;
    impl Drop for Close {
        fn drop(&mut self) {
            // SAFETY: only made after OpenClipboard succeeded.
            unsafe { CloseClipboard() };
        }
    }
    // SAFETY: the clipboard is opened and closed (by `Close`) on this
    // thread; the handle stays valid while it is open, and the block is
    // copied out between GlobalLock and GlobalUnlock, within GlobalSize.
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP) == 0 || OpenClipboard(std::ptr::null_mut()) == 0 {
            return Vec::new();
        }
        let _close = Close;
        let handle = GetClipboardData(CF_HDROP);
        if handle.is_null() {
            return Vec::new();
        }
        let size = GlobalSize(handle);
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
            return Vec::new();
        }
        let block = std::slice::from_raw_parts(ptr, size).to_vec();
        GlobalUnlock(handle);
        hdrop_paths(&block)
    }
}

#[cfg(not(windows))]
pub fn clipboard_files() -> Vec<PathBuf> {
    Vec::new()
}

/// The fragment of a CF_HTML ("HTML Format") clipboard block: the UTF-8
/// bytes between its `StartFragment` and `EndFragment` offsets (the
/// header's), else between the `<!--StartFragment-->` comments. `None` if
/// the header is malformed.
pub fn cf_html_fragment(block: &[u8]) -> Option<String> {
    let block = block.split(|b| *b == 0).next().unwrap_or(block);
    let head = String::from_utf8_lossy(block.get(..block.len().min(1024))?);
    let field = |name: &str| -> Option<usize> {
        let at = head.find(name)? + name.len();
        let digits: String = head
            .get(at..)?
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    };
    let html = match (field("StartFragment:"), field("EndFragment:")) {
        (Some(a), Some(b)) if a <= b && b <= block.len() => block.get(a..b)?,
        _ => {
            let text = block;
            let find = |pat: &[u8]| text.windows(pat.len()).position(|w| w == pat);
            let a = find(b"<!--StartFragment-->")? + "<!--StartFragment-->".len();
            let b = find(b"<!--EndFragment-->")?;
            text.get(a..b.max(a))?
        }
    };
    Some(String::from_utf8_lossy(html).into_owned())
}

/// The largest CF_HTML block read: pictures copied with the HTML (as
/// `data:` addresses) make it large.
pub const MAX_CLIPBOARD_HTML: usize = 48 * 1024 * 1024;
/// The largest bitmap read from the clipboard (a 4K screenshot as a DIB is
/// about 33 MB).
pub const MAX_CLIPBOARD_PICTURE: usize = 64 * 1024 * 1024;

/// A clipboard format's block (`format` a standard one, or `name`
/// registered), if it is there and at most `max` bytes.
#[cfg(windows)]
fn clipboard_block(format: u32, name: Option<&str>, max: usize) -> Option<Vec<u8>> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        RegisterClipboardFormatW,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    struct Close;
    impl Drop for Close {
        fn drop(&mut self) {
            // SAFETY: only made after OpenClipboard succeeded.
            unsafe { CloseClipboard() };
        }
    }
    let wide: Option<Vec<u16>> = name.map(|n| n.encode_utf16().chain([0]).collect());
    // SAFETY: `wide` is NUL-terminated; the clipboard is opened and closed
    // (by `Close`) on this thread; the handle stays valid while it is open,
    // and the block is copied out between GlobalLock and GlobalUnlock,
    // within GlobalSize.
    unsafe {
        let format = match &wide {
            Some(w) => RegisterClipboardFormatW(w.as_ptr()),
            None => format,
        };
        if format == 0
            || IsClipboardFormatAvailable(format) == 0
            || OpenClipboard(std::ptr::null_mut()) == 0
        {
            return None;
        }
        let _close = Close;
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        if size > max {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let block = std::slice::from_raw_parts(ptr, size).to_vec();
        GlobalUnlock(handle);
        Some(block)
    }
}

/// The HTML fragment on the clipboard (copied from a browser, Word or
/// Outlook), if any; at most [`MAX_CLIPBOARD_HTML`].
#[cfg(windows)]
pub fn clipboard_html() -> Option<String> {
    let block = clipboard_block(0, Some("HTML Format"), MAX_CLIPBOARD_HTML)?;
    cf_html_fragment(&block)
}

#[cfg(not(windows))]
pub fn clipboard_html() -> Option<String> {
    None
}

/// Whether the clipboard has HTML or a picture (a cheap check, the
/// clipboard not opened): a formatted paste then reads them off the UI
/// thread.
#[cfg(windows)]
pub fn clipboard_has_rich() -> bool {
    use windows_sys::Win32::System::DataExchange::{
        IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    /// `CF_BITMAP`, `CF_DIB`, `CF_DIBV5`.
    const BITMAPS: [u32; 3] = [2, 8, 17];
    let named = ["HTML Format", "PNG"].map(|n| n.encode_utf16().chain([0]).collect::<Vec<u16>>());
    // SAFETY: each name is NUL-terminated; neither call opens the clipboard.
    unsafe {
        named.iter().any(|n| {
            let f = RegisterClipboardFormatW(n.as_ptr());
            f != 0 && IsClipboardFormatAvailable(f) != 0
        }) || BITMAPS.iter().any(|f| IsClipboardFormatAvailable(*f) != 0)
    }
}

#[cfg(not(windows))]
pub fn clipboard_has_rich() -> bool {
    false
}

/// A picture on the clipboard (a screenshot, an image copied in an app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPicture {
    /// The registered "PNG" format (browsers, Office, the Snipping Tool).
    Png(Vec<u8>),
    /// A device-independent bitmap (CF_DIBV5 or CF_DIB; Windows makes one
    /// from a CF_BITMAP), as a BMP file ([`dib_to_bmp`]).
    Bmp(Vec<u8>),
}

/// The picture on the clipboard, if any: its PNG when it has one, else its
/// bitmap.
#[cfg(windows)]
pub fn clipboard_picture() -> Option<ClipboardPicture> {
    /// `CF_DIB` and `CF_DIBV5` (their constants live in the Ole feature).
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;
    if let Some(png) = clipboard_block(0, Some("PNG"), MAX_CLIPBOARD_PICTURE) {
        return Some(ClipboardPicture::Png(png));
    }
    let dib = clipboard_block(CF_DIBV5, None, MAX_CLIPBOARD_PICTURE)
        .or_else(|| clipboard_block(CF_DIB, None, MAX_CLIPBOARD_PICTURE))?;
    dib_to_bmp(&dib).map(ClipboardPicture::Bmp)
}

#[cfg(not(windows))]
pub fn clipboard_picture() -> Option<ClipboardPicture> {
    None
}

/// A packed DIB (a BITMAPINFOHEADER or a later header, the colour table or
/// masks, the pixels) as a BMP file: the 14-byte file header in front,
/// pointing at the pixels. `None` for a block too short or malformed.
pub fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let le32 = |at: usize| -> Option<u32> {
        let s = dib.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([
            *s.first()?,
            *s.get(1)?,
            *s.get(2)?,
            *s.get(3)?,
        ]))
    };
    let le16 = |at: usize| -> Option<u16> {
        let s = dib.get(at..at.checked_add(2)?)?;
        Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
    };
    let header = usize::try_from(le32(0)?).ok()?;
    if !(40..=124).contains(&header) || dib.len() < header {
        return None;
    }
    let bits = le16(14)?;
    let compression = le32(16)?;
    let used = usize::try_from(le32(32)?).ok()?;
    let palette = if used > 0 {
        used
    } else if bits <= 8 {
        1usize << bits
    } else {
        0
    };
    // BI_BITFIELDS with the plain header: three masks follow it.
    let masks = if header == 40 && compression == 3 {
        12
    } else {
        0
    };
    let offset = 14usize
        .checked_add(header)?
        .checked_add(masks)?
        .checked_add(palette.checked_mul(4)?)?;
    let total = dib.len().checked_add(14)?;
    if offset > total {
        return None;
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&u32::try_from(total).ok()?.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&u32::try_from(offset).ok()?.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// The CF_HTML header (Version 0.9, four ten-digit offsets).
const CF_HEADER: &str =
    "Version:0.9\r\nStartHTML:{a}\r\nEndHTML:{b}\r\nStartFragment:{c}\r\nEndFragment:{d}\r\n";

/// A CF_HTML ("HTML Format") clipboard block for the HTML document `html`
/// (its `<body>` content is the fragment): the header's byte offsets, then
/// the document with the fragment comments, NUL-terminated.
pub fn cf_html_block(html: &str) -> Vec<u8> {
    const START: &str = "<!--StartFragment-->";
    const END: &str = "<!--EndFragment-->";
    let (head, body, tail) = match (html.find("<body>"), html.rfind("</body>")) {
        (Some(a), Some(b)) if a + 6 <= b => (
            html.get(..a + 6).unwrap_or_default(),
            html.get(a + 6..b).unwrap_or_default(),
            html.get(b..).unwrap_or_default(),
        ),
        _ => ("<html><body>", html, "</body></html>"),
    };
    let doc = format!("{head}{START}{body}{END}{tail}");
    // Each `{x}` placeholder (3 bytes) becomes ten digits.
    let header_len = CF_HEADER.len() + 4 * 7;
    let start_frag = header_len + head.len() + START.len();
    let end_frag = start_frag + body.len();
    let end_html = header_len + doc.len();
    let mut out = CF_HEADER
        .replace("{a}", &format!("{header_len:010}"))
        .replace("{b}", &format!("{end_html:010}"))
        .replace("{c}", &format!("{start_frag:010}"))
        .replace("{d}", &format!("{end_frag:010}"));
    out.push_str(&doc);
    let mut out = out.into_bytes();
    out.push(0);
    out
}

/// Put `text` (plain) and `html` (a whole HTML document) on the clipboard
/// together, for Ctrl+C and Cut in the composer. False: the clipboard
/// would not take the text (another program holds it).
#[cfg(windows)]
pub fn set_clipboard_rich(text: &str, html: &str) -> bool {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    /// `CF_UNICODETEXT` (its constant lives in the Ole feature).
    const CF_UNICODETEXT: u32 = 13;
    struct Close;
    impl Drop for Close {
        fn drop(&mut self) {
            // SAFETY: only made after OpenClipboard succeeded.
            unsafe { CloseClipboard() };
        }
    }
    // Windows text: CRLF line ends, UTF-16, NUL-terminated.
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let wide: Vec<u8> = crlf
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_ne_bytes)
        .collect();
    let block = cf_html_block(html);
    let name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
    let put = |fmt: u32, bytes: &[u8]| -> bool {
        // SAFETY: the block is allocated, filled within its size between
        // GlobalLock and GlobalUnlock, then handed to the clipboard (which
        // owns it from then on) or freed if it is refused.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if h.is_null() {
                return false;
            }
            let ptr = GlobalLock(h).cast::<u8>();
            if ptr.is_null() {
                GlobalFree(h);
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            GlobalUnlock(h);
            if SetClipboardData(fmt, h).is_null() {
                GlobalFree(h);
                return false;
            }
            true
        }
    };
    // SAFETY: `name` is NUL-terminated; the clipboard is opened on this
    // thread, owned by the active window (so EmptyClipboard gives it an
    // owner and SetClipboardData is accepted), and closed by `Close`.
    unsafe {
        let format = RegisterClipboardFormatW(name.as_ptr());
        if OpenClipboard(GetActiveWindow()) == 0 {
            return false;
        }
        let _close = Close;
        if EmptyClipboard() == 0 {
            return false;
        }
        let plain = put(CF_UNICODETEXT, &wide);
        if format != 0 {
            put(format, &block);
        }
        plain
    }
}

#[cfg(not(windows))]
pub fn set_clipboard_rich(_: &str, _: &str) -> bool {
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// A screenshot's CF_DIB (a 2 x 1, 32-bit BI_BITFIELDS bitmap, and a
    /// 1 x 1, 8-bit one with its colour table) becomes a BMP whose header
    /// points at its pixels.
    #[test]
    fn a_clipboard_bitmap_becomes_a_bmp_file() {
        let header = |w: i32, h: i32, bits: u16, compression: u32, used: u32| {
            let mut v = Vec::new();
            v.extend(40u32.to_le_bytes());
            v.extend(w.to_le_bytes());
            v.extend(h.to_le_bytes());
            v.extend(1u16.to_le_bytes());
            v.extend(bits.to_le_bytes());
            v.extend(compression.to_le_bytes());
            v.extend([0; 12]);
            v.extend(used.to_le_bytes());
            v.extend(0u32.to_le_bytes());
            v
        };
        let mut dib = header(2, 1, 32, 3, 0);
        dib.extend([0, 0, 0xff, 0, 0, 0xff, 0, 0, 0xff, 0, 0, 0]);
        dib.extend([1, 2, 3, 0, 4, 5, 6, 0]);
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize,
            bmp.len()
        );
        assert_eq!(
            u32::from_le_bytes(bmp[10..14].try_into().unwrap()),
            14 + 40 + 12
        );
        let mut pal = header(1, 1, 8, 0, 0);
        pal.extend(vec![0; 256 * 4]);
        pal.extend([0, 0, 0, 0]);
        let bmp = dib_to_bmp(&pal).unwrap();
        assert_eq!(
            u32::from_le_bytes(bmp[10..14].try_into().unwrap()),
            14 + 40 + 1024
        );
        // Junk is refused.
        assert!(dib_to_bmp(b"").is_none());
        assert!(dib_to_bmp(&[9, 0, 0, 0]).is_none());
        assert!(
            dib_to_bmp(&header(1, 1, 8, 0, 0)).is_none(),
            "no room for the palette"
        );
    }

    #[test]
    fn opening_refuses_what_runs_code_and_writes_nothing() {
        let dir = temp("attachment-refuse");
        let launched = std::cell::Cell::new(false);
        for name in ["setup.exe", "invoice.pdf.exe", "a.PS1", "x.lnk."] {
            let said = open_attachment_with(&dir, name, b"MZ", |_| {
                launched.set(true);
                true
            });
            assert_eq!(said, Opened::Refused, "{name}");
        }
        assert!(!launched.get());
        assert!(!dir.exists(), "no copy was written");
    }

    #[test]
    fn opening_writes_a_copy_in_its_own_folder_and_starts_the_app() {
        let dir = temp("attachment-open");
        let mut copy = None;
        let said = open_attachment_with(&dir, "plan.pdf", b"%PDF", |path| {
            copy = Some(path.to_path_buf());
            true
        });
        assert_eq!(said, Opened::Started);
        let copy = copy.unwrap();
        assert!(copy.starts_with(&dir) && copy.parent() != Some(dir.as_path()));
        assert_eq!(std::fs::read(&copy).unwrap(), b"%PDF");
        assert_eq!(copy.file_name().unwrap(), "plan.pdf");
        // No app for the type: the copy stays, and the caller says so.
        let said = open_attachment_with(&dir, "plan.xyz", b"?", |_| false);
        assert_eq!(said, Opened::NoApp);
        // A name too long for a path is cut, keeping its extension.
        let long = format!("{}.txt", "a".repeat(400));
        let mut cut = None;
        let said = open_attachment_with(&dir, &long, b"?", |path| {
            cut = Some(path.to_path_buf());
            true
        });
        assert_eq!(said, Opened::Started);
        let cut = cut.unwrap();
        let name = cut.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with(".txt") && name.len() < long.len());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_writes_the_bytes_where_chosen() {
        let dir = temp("attachment-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.pdf");
        save_attachment(&path, b"%PDF-1").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1");
        // A folder that is not there: the error, not a panic.
        assert!(save_attachment(&dir.join("gone").join("a.pdf"), b"x").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cf_html_block_round_trips_its_fragment() {
        let html = "<html><head></head><body><div><b>Olá</b> mundo</div></body></html>";
        let block = cf_html_block(html);
        assert_eq!(block.last(), Some(&0));
        assert_eq!(
            cf_html_fragment(&block).as_deref(),
            Some("<div><b>Olá</b> mundo</div>")
        );
        let head = String::from_utf8_lossy(&block[..40]).into_owned();
        assert!(
            head.starts_with("Version:0.9\r\nStartHTML:0000000105\r\n"),
            "{head}"
        );
        // Not a whole document: wrapped in one.
        let block = cf_html_block("<i>x</i>");
        assert_eq!(cf_html_fragment(&block).as_deref(), Some("<i>x</i>"));
    }

    #[test]
    fn copied_formatting_pastes_back() {
        // What Ctrl+C puts on the clipboard, read back as Ctrl+V reads it.
        let mut doc = crate::mail::rich::Doc::from_plain("Olá mundo");
        if let Some(crate::mail::rich::Block::Para(p)) = doc.blocks.first_mut() {
            p.runs.iter_mut().for_each(|r| r.style.b = true);
        }
        let html = crate::mail::rich::emit::html(&doc, crate::mail::rich::emit::Mode::Send);
        let frag = cf_html_fragment(&cf_html_block(&html)).unwrap();
        let back = crate::mail::compose_html::sanitize(&frag)
            .and_then(|h| crate::mail::rich::import::lossy(&h))
            .unwrap();
        assert_eq!(
            crate::mail::rich::emit::plain(&back).trim_end(),
            "Olá mundo"
        );
        assert!(
            crate::mail::rich::emit::needs_html(&back),
            "the bold is kept"
        );
    }

    #[test]
    fn cf_html_gives_its_fragment() {
        let html = "<html><body><!--StartFragment--><b>Hi</b> ação<!--EndFragment--></body></html>";
        let head = |a: usize, b: usize| {
            format!(
                "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:{:010}\r\nStartFragment:{a:010}\r\nEndFragment:{b:010}\r\n",
                105 + html.len()
            )
        };
        let probe = head(0, 0);
        let start = probe.len() + html.find("<b>").unwrap();
        let end = probe.len() + html.find("<!--End").unwrap();
        let block = format!("{}{html}\0", head(start, end));
        assert_eq!(
            cf_html_fragment(block.as_bytes()).as_deref(),
            Some("<b>Hi</b> ação")
        );
        // Offsets out of range: the fragment comments.
        let block = format!("{}{html}", head(9999, 99999));
        assert_eq!(
            cf_html_fragment(block.as_bytes()).as_deref(),
            Some("<b>Hi</b> ação")
        );
        // Neither: nothing.
        assert_eq!(cf_html_fragment(b"Version:0.9\r\n<b>x</b>"), None);
    }

    fn dropfiles(wide: bool, list: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&20u32.to_le_bytes());
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&u32::from(wide).to_le_bytes());
        b.extend_from_slice(list);
        b
    }

    fn utf16(paths: &[&str]) -> Vec<u8> {
        let mut b = Vec::new();
        for p in paths {
            for u in p.encode_utf16().chain([0]) {
                b.extend_from_slice(&u.to_le_bytes());
            }
        }
        b.extend_from_slice(&[0, 0]);
        b
    }

    #[test]
    fn reads_the_files_of_a_copy() {
        let paths = [r"C:\Users\Ana\Relatório final.pdf", r"D:\fotos\😀.jpg"];
        assert_eq!(
            hdrop_paths(&dropfiles(true, &utf16(&paths))),
            paths.map(PathBuf::from)
        );
        // ANSI: plain ASCII paths only.
        let ansi = b"C:\\a.txt\0C:\\b\xe9.txt\0C:\\c.txt\0\0";
        assert_eq!(
            hdrop_paths(&dropfiles(false, ansi)),
            [r"C:\a.txt", r"C:\c.txt"].map(PathBuf::from)
        );
        // Malformed blocks: nothing, or the paths before the damage.
        assert!(hdrop_paths(&[]).is_empty());
        assert!(hdrop_paths(&[1, 2, 3]).is_empty());
        let mut far = dropfiles(true, &utf16(&["C:\\a"]));
        far[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(hdrop_paths(&far).is_empty());
        let mut cut = dropfiles(true, &utf16(&["C:\\a", "C:\\bb"]));
        cut.truncate(cut.len() - 5);
        assert_eq!(
            hdrop_paths(&cut),
            [PathBuf::from("C:\\a"), PathBuf::from("C:\\b")]
        );
        assert!(hdrop_paths(&dropfiles(true, &[0, 0])).is_empty());
        // Every truncation of a valid block is safe.
        let full = dropfiles(true, &utf16(&paths));
        for n in 0..full.len() {
            hdrop_paths(&full[..n]);
        }
    }

    #[test]
    fn refuses_to_open_types_that_run_code() {
        for name in [
            "setup.exe",
            "invoice.pdf.exe",
            "INVOICE.PDF.EXE",
            "run.bat",
            "x.cmd",
            "a.ps1",
            "a.vbs",
            "a.js",
            "a.jse",
            "a.wsf",
            "a.msi",
            "a.lnk",
            "a.hta",
            "a.cpl",
            "a.scr",
            "a.com",
            "a.url",
            "a.scf",
            "a.appref-ms",
            "photo.jpg.scr",
            "trailing.exe.",
            "trailing.exe ",
        ] {
            assert!(risky(name), "{name}");
        }
        for name in [
            "report.pdf",
            "photo.jpg",
            "exe",
            "notes.txt",
            "archive.zip",
            "exe.pdf",
            "a.json",
            "a.jsx.txt",
            "",
        ] {
            assert!(!risky(name), "{name}");
        }
    }

    #[test]
    fn fits_names_to_the_path_limit() {
        assert_eq!(fit_name("a.pdf", 10).unwrap(), "a.pdf");
        let long = format!("{}.pdf", "é".repeat(100));
        let cut = fit_name(&long, 50).unwrap();
        assert!(
            cut.encode_utf16().count() <= 50 && cut.ends_with(".pdf"),
            "{cut}"
        );
        // Surrogate pairs count as two units and are never split.
        let emoji = format!("{}.txt", "😀".repeat(40));
        let cut = fit_name(&emoji, 21).unwrap();
        assert_eq!(cut, format!("{}.txt", "😀".repeat(8)));
        assert_eq!(fit_name("ab.   x.docx", 12).unwrap(), "ab.   x.docx");
        assert_eq!(
            fit_name(&format!("{}.docx", "a".repeat(20)), 13).unwrap(),
            "aaaaaaaa.docx"
        );
        assert!(fit_name("long name.pdf", 6).is_none());
        assert_eq!(fit_name(&"x".repeat(30), 10).unwrap(), "x".repeat(10));
    }

    fn temp(name: &str) -> PathBuf {
        crate::testdir::unique("komorebi-files", name)
    }

    #[test]
    fn open_copies_are_marked_and_cleared_next_time() {
        let dir = temp("open");
        let path = write_open_copy(&dir, "Relatório.pdf", b"%PDF").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF");
        assert_eq!(path.file_name().unwrap(), "Relatório.pdf");
        assert_eq!(path.parent().unwrap().parent().unwrap(), dir);
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        match std::fs::read_to_string(&stream) {
            Ok(mark) => assert_eq!(mark, "[ZoneTransfer]\r\nZoneId=3\r\n"),
            // A temp folder without alternate streams (not NTFS).
            Err(e) => eprintln!("no Zone.Identifier stream: {e}"),
        }
        // A cut that leaves a risky extension is refused.
        for n in 100..200 {
            let long = format!("{}.exe{}", "a".repeat(n), "z".repeat(20));
            if let Ok(path) = write_open_copy(&dir, &long, b"x") {
                assert!(!risky(&path.file_name().unwrap().to_string_lossy()), "{n}");
            }
        }
        // Two opens of one name never share a folder.
        let again = write_open_copy(&dir, "Relatório.pdf", b"2").unwrap();
        assert_ne!(again, path);

        // Opened before the start: removed; opened since: kept.
        let start = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let fresh = write_open_copy(&dir, "new.txt", b"n").unwrap();
        clear_open_dir(&dir, start);
        assert!(!path.exists() && !again.exists());
        assert!(fresh.exists());
        clear_open_dir(&dir.join("missing"), start);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn marks_saved_files_as_downloaded() {
        let dir = temp("mark");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("saved.txt");
        std::fs::write(&file, "x").unwrap();
        match mark_downloaded(&file) {
            Ok(()) => {
                let mut stream = file.as_os_str().to_owned();
                stream.push(":Zone.Identifier");
                let mark = std::fs::read_to_string(stream).unwrap();
                assert!(mark.contains("ZoneId=3"), "{mark}");
                // The file itself is unchanged.
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
            }
            Err(e) => eprintln!("skipped, no alternate streams here: {e}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
