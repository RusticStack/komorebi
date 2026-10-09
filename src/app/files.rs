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
pub(super) fn risky(name: &str) -> bool {
    name.trim_end_matches(['.', ' '])
        .rsplit_once('.')
        .is_some_and(|(_, ext)| RISKY.iter().any(|r| ext.eq_ignore_ascii_case(r)))
}

/// Where a cache keeps the attachments it opened: `cache.open` beside it
/// (`%LOCALAPPDATA%\komorebi\cache.open`), cleared at the next start.
pub(super) fn open_dir(db_path: &Path) -> PathBuf {
    db_path.with_extension("open")
}

/// Remove what an earlier run opened (folders older than `before`, so a
/// copy opened since this start stays). Runs off the UI thread.
pub(super) fn clear_open_dir(dir: &Path, before: SystemTime) {
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
pub(super) fn write_open_copy(dir: &Path, name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
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

/// Mark `path` as downloaded from the internet: a `Zone.Identifier`
/// stream (zone 3), so Windows and apps such as Office open it with care.
/// Fails on file systems without alternate streams (FAT, some shares).
pub(super) fn mark_downloaded(path: &Path) -> std::io::Result<()> {
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
pub(super) fn hdrop_paths(block: &[u8]) -> Vec<PathBuf> {
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
pub(super) fn clipboard_files() -> Vec<PathBuf> {
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
pub(super) fn clipboard_files() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

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
        let dir =
            std::env::temp_dir().join(format!("komorebi-files-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
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
