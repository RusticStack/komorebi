//! One app per cache, and the hand-off to the running one: the per-cache
//! instance mutex, the window mark that finds a cache's window (never by its
//! translated title), the installer's app mutex, and the wait for an app that
//! asked for a restart. Plain Win32 through `windows-sys`, shared by every
//! app binary (the Slint app in `src/main.rs` and the GPUI app in
//! `crates/komorebi-gpui`).

use std::path::Path;

/// A hash (FNV-1a, stable across builds) of the absolute, lower-cased path
/// of the cache at `db_path`, so a throwaway `KOMOREBI_DB` instance runs
/// beside the real one.
fn instance_hash(db_path: &Path) -> u64 {
    let path = std::path::absolute(db_path).unwrap_or_else(|_| db_path.to_path_buf());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The single-instance mutex for the cache at `db_path`: `Local\Komorebi-`
/// and its [`instance_hash`].
pub fn instance_name(db_path: &Path) -> String {
    format!("Local\\Komorebi-{:016x}", instance_hash(db_path))
}

/// The name of the window property that tells which cache a Komorebi window
/// shows. A window is found by this, never by its title: the title is
/// translated and carries the unread count.
#[cfg(windows)]
const WINDOW_PROP: &str = "Komorebi.Instance";

/// The app that asked for a restart (`KOMOREBI_RESTART_AFTER`, its process
/// id; set by `app::cachelock`), waited for up to 15 s.
pub fn wait_for_restart() {
    let Some(pid) = std::env::var("KOMOREBI_RESTART_AFTER")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    else {
        return;
    };
    // SAFETY: plain Win32 calls; the handle is closed once.
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !h.is_null() {
            WaitForSingleObject(h, 15_000);
            CloseHandle(h);
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
}

/// The property value for a cache's window (never 0, which means "none").
pub fn window_mark(db_path: &Path) -> usize {
    (instance_hash(db_path) as usize) | 1
}

/// Mark the window as that of the cache `mark` names.
///
/// # Safety
///
/// `hwnd` is a live window of this process.
#[cfg(windows)]
pub unsafe fn mark_window(hwnd: windows_sys::Win32::Foundation::HWND, mark: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetPropW;
    let name: Vec<u16> = WINDOW_PROP.encode_utf16().chain([0]).collect();
    // SAFETY: a live window of this process (the caller's promise), a
    // NUL-terminated name, and a plain number as the value (no handle to
    // free when the window goes).
    unsafe { SetPropW(hwnd, name.as_ptr(), mark as _) };
}

/// Take the instance mutex for this cache and keep it for the life of the
/// process. False if another Komorebi holds it: its window is brought to
/// the front instead.
#[cfg(windows)]
pub fn single_instance(db_path: &Path) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain([0]).collect() };
    let name = wide(&instance_name(db_path));
    // SAFETY: `name` is NUL-terminated and outlives the call. The handle is
    // never closed on success, so the mutex lives until the process exits.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        // Can't tell (no mutex): start anyway.
        return true;
    }
    // SAFETY: a valid handle from CreateMutexW.
    unsafe { CloseHandle(handle) };
    // A `mailto:` URL goes to the running copy, which opens a composer with
    // it (and comes forward by itself); otherwise it is asked to come
    // forward. A window hidden to the tray is shown by the running copy.
    let handed = crate::launch::handoff_payload(std::env::args().skip(1))
        .is_some_and(|p| crate::win_shell::send_mailto_to_existing_instance(&p));
    if !handed {
        crate::win_shell::signal_existing_instance();
    }
    if let Some(hwnd) = app_window(window_mark(db_path)) {
        // SAFETY: a window handle just found; a stale one fails harmlessly.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }
    false
}

/// The first top-level window marked (`mark_window`) with `mark`.
#[cfg(windows)]
pub fn app_window(mark: usize) -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetPropW};
    use windows_sys::core::BOOL;
    struct Search {
        mark: usize,
        found: Option<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `Search` passed below, alive for the call.
        let search = unsafe { &mut *(data as *mut Search) };
        let name: Vec<u16> = WINDOW_PROP.encode_utf16().chain([0]).collect();
        // SAFETY: a window handle from the enumeration, and a NUL-terminated
        // name; a window without the property gives null.
        if unsafe { GetPropW(hwnd, name.as_ptr()) } as usize != search.mark {
            return 1;
        }
        search.found = Some(hwnd);
        0
    }
    let mut search = Search { mark, found: None };
    // SAFETY: the callback only runs during the call, while `search` lives.
    unsafe { EnumWindows(Some(each), &mut search as *mut Search as LPARAM) };
    search.found
}

#[cfg(not(windows))]
pub fn single_instance(_: &Path) -> bool {
    true
}

/// The fixed mutex every running Komorebi holds (whatever its cache), which
/// the installer's `AppMutex` names: an install waits for it to go.
#[cfg(windows)]
const APP_MUTEX: &str = "Komorebi.App";

/// The handle of [`APP_MUTEX`] (0: none).
static APP_MUTEX_HANDLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Take [`APP_MUTEX`] for the life of the process (or until
/// [`release_app_mutex`]). Shared: a second cache's Komorebi opens the same
/// one.
#[cfg(windows)]
pub fn hold_app_mutex() {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = APP_MUTEX.encode_utf16().chain([0]).collect();
    // SAFETY: `name` is NUL-terminated and outlives the call; the handle is
    // kept until `release_app_mutex` or the process exits.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    APP_MUTEX_HANDLE.store(handle as usize, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(windows))]
pub fn hold_app_mutex() {}

/// Let go of [`APP_MUTEX`] just before the self-update starts the installer
/// (which would otherwise refuse to run while this process is exiting).
pub fn release_app_mutex() {
    let handle = APP_MUTEX_HANDLE.swap(0, std::sync::atomic::Ordering::Relaxed);
    #[cfg(windows)]
    if handle != 0 {
        // SAFETY: a handle from CreateMutexW, closed once (swapped out).
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle as _) };
    }
    #[cfg(not(windows))]
    let _ = handle;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_found_by_its_mark_not_its_title() {
        let real = Path::new(r"C:\Users\Ana\AppData\Local\komorebi\cache.sqlite3");
        let throwaway = Path::new(r"C:\Temp\komorebi-test.sqlite3");
        assert_ne!(window_mark(real), window_mark(throwaway));
        assert_ne!(window_mark(real), 0);
        let upper = Path::new(r"C:\USERS\ANA\APPDATA\LOCAL\KOMOREBI\CACHE.SQLITE3");
        assert_eq!(window_mark(real), window_mark(upper));
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};
            let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain([0]).collect() };
            // Hidden windows with titles in either language: only the mark
            // makes one this app's.
            let make = |title: &str| {
                // SAFETY: a built-in class and a NUL-terminated title.
                unsafe {
                    CreateWindowExW(
                        0,
                        wide("STATIC").as_ptr(),
                        wide(title).as_ptr(),
                        0,
                        0,
                        0,
                        10,
                        10,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    )
                }
            };
            let en = make("Inbox (3) - Komorebi");
            let pt = make("Caixa de entrada (3) - Komorebi");
            let other = make("Komorebi");
            assert!(!en.is_null() && !pt.is_null() && !other.is_null());
            assert_eq!(app_window(window_mark(real)), None, "titles do not count");
            // SAFETY: windows created above on this thread.
            unsafe {
                mark_window(pt, window_mark(real));
                mark_window(other, window_mark(throwaway));
            }
            assert_eq!(app_window(window_mark(real)), Some(pt));
            assert_eq!(app_window(window_mark(throwaway)), Some(other));
            for hwnd in [en, pt, other] {
                // SAFETY: windows created above on this thread.
                unsafe { DestroyWindow(hwnd) };
            }
            assert_eq!(app_window(window_mark(real)), None);
        }
    }

    #[test]
    fn one_instance_name_per_cache_file() {
        let real = Path::new(r"C:\Users\Ana\AppData\Local\komorebi\cache.sqlite3");
        let name = instance_name(real);
        assert!(name.starts_with(r"Local\Komorebi-"), "{name}");
        assert_eq!(name.len(), r"Local\Komorebi-".len() + 16);
        // No backslash after the namespace (it would name a directory).
        assert_eq!(name.matches('\\').count(), 1);
        // Paths compare without case, as Windows does.
        let upper = Path::new(r"C:\USERS\ANA\APPDATA\LOCAL\KOMOREBI\CACHE.SQLITE3");
        assert_eq!(instance_name(upper), name);
        // A throwaway cache gets its own instance.
        let throwaway = Path::new(r"C:\Temp\komorebi-test.sqlite3");
        assert_ne!(instance_name(throwaway), name);
        // A relative path names the same file as its absolute form.
        let relative = Path::new("seed.sqlite3");
        let absolute = std::path::absolute(relative).unwrap();
        assert_eq!(instance_name(relative), instance_name(&absolute));
        // Stable across builds: the hash is FNV-1a, not std's hasher.
        assert_eq!(
            instance_name(Path::new(r"C:\a")),
            r"Local\Komorebi-7f89de91ebb8dc77"
        );
    }
}
