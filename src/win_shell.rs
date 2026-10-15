//! Windows shell plumbing: tray icon with menu, toast (shell balloon), the
//! Run-key autostart and single-instance hooks. Plain Win32 through
//! `windows-sys` (already a dependency), so no new crate and no size cost.
//!
//! Toasts use the tray icon's `NIF_INFO` notification, which Windows 11
//! presents as a native toast and which reports a click. Nothing here logs or
//! stores message content; a toast's text lives only in the shell.
//!
//! Everything runs on the UI thread: [`Shell::new`] creates a hidden
//! message-only window on the calling thread, whose messages are dispatched by
//! the message loop Slint already runs. Events reach the app through the
//! callback given to `new`, always on that thread.
//!
//! The window is a hidden top-level one (never shown), not a message-only one,
//! because only top-level windows receive the `TaskbarCreated` broadcast; on
//! it the tray icon is added again after an Explorer restart, with the current
//! tooltip.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE, HWND, LPARAM, LRESULT,
    WPARAM,
};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_NONE, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    FindWindowExW, GetCursorPos, IDI_APPLICATION, LoadIconW, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    PostMessageW, RegisterClassW, RegisterWindowMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU,
    WM_COPYDATA, WM_ENDSESSION, WM_LBUTTONUP, WM_NULL, WM_QUERYENDSESSION, WM_RBUTTONUP, WNDCLASSW,
};

use crate::notify::Target;

const CLASS_NAME: &str = "KomorebiShellWindow";
const MSG_TRAY: u32 = WM_APP + 1;
const MSG_ACTIVATE: u32 = WM_APP + 2;
const NIN_BALLOONUSERCLICK: u32 = 0x405;
const NIN_BALLOONTIMEOUT: u32 = 0x404;
const ICON_ID: u32 = 1;

/// Registered id of the `TaskbarCreated` broadcast (0 until registered).
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// True when `msg` is the registered, non-zero `TaskbarCreated` message.
fn is_taskbar_created(msg: u32, registered: u32) -> bool {
    registered != 0 && msg == registered
}

const CMD_OPEN: usize = 1;
const CMD_CHECK: usize = 2;
const CMD_PAUSE: usize = 3;
const CMD_QUIT: usize = 4;

/// What the shell tells the app. Delivered on the UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// Tray left click or the Open menu item.
    Open,
    /// Check mail menu item.
    CheckMail,
    /// Pause/resume notifications menu item; the app flips its setting and
    /// calls [`Shell::set_paused`].
    TogglePause,
    /// Quit menu item: really exit, do not just hide to the tray.
    Quit,
    /// The user clicked a toast: open this message.
    ToastClicked(Target),
    /// A second launch asked this instance to come forward.
    ActivateRequested,
    /// A second launch handed over a `mailto:` URL: open a prefilled
    /// composer (it is never sent).
    Mailto(crate::mail::render::links::Mailto),
}

/// Translated menu labels, supplied by the app from the `Tr` global.
#[derive(Clone, Debug)]
pub struct TrayLabels {
    pub open: String,
    pub check_mail: String,
    pub pause_notifications: String,
    pub quit: String,
}

struct Inner {
    labels: TrayLabels,
    paused: bool,
    tooltip: String,
    last_toast: Option<Target>,
    handler: Rc<dyn Fn(ShellEvent)>,
}

thread_local! {
    static INNER: RefCell<Option<Inner>> = const { RefCell::new(None) };
}

/// The tray icon and its hidden window. Dropping it removes the icon.
pub struct Shell {
    hwnd: HWND,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let max = dst.len().saturating_sub(1);
    let mut n = 0;
    for u in s.encode_utf16().take(max) {
        dst[n] = u;
        n += 1;
    }
    if let Some(slot) = dst.get_mut(n) {
        *slot = 0;
    }
}

fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW is plain data; all-zero is a valid initial value.
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = ICON_ID;
    d
}

fn app_icon() -> *mut c_void {
    // SAFETY: resource id 1 of this exe, falling back to the stock icon.
    unsafe {
        let module = GetModuleHandleW(null());
        let icon = LoadIconW(module, std::ptr::without_provenance::<u16>(1));
        if icon.is_null() {
            LoadIconW(null_mut(), IDI_APPLICATION)
        } else {
            icon
        }
    }
}

fn emit(ev: ShellEvent) {
    let handler = INNER.with(|i| i.borrow().as_ref().map(|s| s.handler.clone()));
    if let Some(h) = handler {
        h(ev);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        MSG_TRAY => {
            match (lp as u32) & 0xFFFF {
                WM_LBUTTONUP => emit(ShellEvent::Open),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                NIN_BALLOONUSERCLICK => {
                    let target =
                        INNER.with(|i| i.borrow_mut().as_mut().and_then(|s| s.last_toast.take()));
                    if let Some(t) = target {
                        emit(ShellEvent::ToastClicked(t));
                    }
                }
                NIN_BALLOONTIMEOUT => {
                    INNER.with(|i| {
                        if let Some(s) = i.borrow_mut().as_mut() {
                            s.last_toast = None;
                        }
                    });
                }
                _ => {}
            }
            0
        }
        MSG_ACTIVATE => {
            emit(ShellEvent::ActivateRequested);
            0
        }
        // The installer's Restart Manager (or a sign-out) asks the app to
        // end: agree, and when told it is ending, really quit (a window
        // hidden to the tray would otherwise keep the app, and its files,
        // alive).
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wp != 0 {
                emit(ShellEvent::Quit);
            }
            0
        }
        WM_COPYDATA => {
            // SAFETY: for WM_COPYDATA, WM_ENDSESSION, WM_QUERYENDSESSION, lParam points to a COPYDATASTRUCT that
            // stays valid for the call.
            let Some(cds) = (unsafe { (lp as *const COPYDATASTRUCT).as_ref() }) else {
                return 0;
            };
            let len = cds.cbData as usize;
            let bytes = if cds.lpData.is_null() || len > crate::launch::MAX_URL_BYTES {
                &[][..]
            } else {
                // SAFETY: lpData points to cbData readable bytes.
                unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, len) }
            };
            match crate::launch::decode_handoff(cds.dwData, bytes) {
                Some(m) => {
                    emit(ShellEvent::Mailto(m));
                    1
                }
                None => 0,
            }
        }
        m if is_taskbar_created(m, TASKBAR_CREATED.load(Ordering::Relaxed)) => {
            // Explorer restarted and lost our icon; add it again.
            add_icon_for(hwnd);
            0
        }
        // SAFETY: forwarding the unhandled message with its original arguments.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn show_menu(hwnd: HWND) {
    let Some((labels, paused)) =
        INNER.with(|i| i.borrow().as_ref().map(|s| (s.labels.clone(), s.paused)))
    else {
        return;
    };
    // SAFETY: plain menu calls on the UI thread; the menu is destroyed below
    // and the wide strings outlive each call.
    let cmd = unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        AppendMenuW(menu, MF_STRING, CMD_OPEN, wide(&labels.open).as_ptr());
        AppendMenuW(
            menu,
            MF_STRING,
            CMD_CHECK,
            wide(&labels.check_mail).as_ptr(),
        );
        let flags = if paused {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        AppendMenuW(
            menu,
            flags,
            CMD_PAUSE,
            wide(&labels.pause_notifications).as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, CMD_QUIT, wide(&labels.quit).as_ptr());
        let mut pt = std::mem::zeroed();
        GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd
    };
    match cmd as usize {
        CMD_OPEN => emit(ShellEvent::Open),
        CMD_CHECK => emit(ShellEvent::CheckMail),
        CMD_PAUSE => emit(ShellEvent::TogglePause),
        CMD_QUIT => emit(ShellEvent::Quit),
        _ => {}
    }
}

/// Adds the tray icon for `hwnd` with the stored tooltip.
fn add_icon_for(hwnd: HWND) -> bool {
    let tip = INNER
        .with(|i| {
            i.try_borrow()
                .ok()
                .and_then(|b| b.as_ref().map(|s| s.tooltip.clone()))
        })
        .unwrap_or_default();
    let mut d = nid(hwnd);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    d.uCallbackMessage = MSG_TRAY;
    d.hIcon = app_icon();
    copy_wide(&mut d.szTip, &tip);
    // SAFETY: d is fully initialised.
    unsafe { Shell_NotifyIconW(NIM_ADD, &d) != 0 }
}

impl Shell {
    /// Creates the hidden window and the tray icon on the calling (UI) thread.
    /// `handler` is called on that thread for every [`ShellEvent`]. Returns
    /// `None` if Windows refuses (the app then simply runs without a tray).
    pub fn new(
        labels: TrayLabels,
        tooltip: &str,
        handler: impl Fn(ShellEvent) + 'static,
    ) -> Option<Shell> {
        // SAFETY: standard window-class registration and creation; every
        // pointer passed outlives the call.
        let hwnd = unsafe {
            let module = GetModuleHandleW(null());
            let class = wide(CLASS_NAME);
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = module;
            wc.lpszClassName = class.as_ptr();
            RegisterClassW(&wc); // already registered is fine
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(), // top-level (hidden, never shown) to hear broadcasts
                null_mut(),
                module,
                null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        INNER.with(|i| {
            *i.borrow_mut() = Some(Inner {
                labels,
                paused: false,
                tooltip: tooltip.to_string(),
                last_toast: None,
                handler: Rc::new(handler),
            });
        });
        // SAFETY: the name is NUL-terminated and lives across the call.
        let tb = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
        TASKBAR_CREATED.store(tb, Ordering::Relaxed);
        let shell = Shell { hwnd };
        if shell.add_icon() {
            Some(shell)
        } else {
            None // Drop removes the window and state
        }
    }

    fn add_icon(&self) -> bool {
        add_icon_for(self.hwnd)
    }

    /// Adds the icon again, for instance after Explorer restarted.
    pub fn readd_icon(&self) {
        let d = nid(self.hwnd);
        // SAFETY: d is fully initialised. Delete first so a live icon is not doubled.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
        self.add_icon();
    }

    /// Sets the tooltip, e.g. "Komorebi - 3 unread" (translated by the caller).
    pub fn set_tooltip(&self, text: &str) {
        INNER.with(|i| {
            if let Some(s) = i.borrow_mut().as_mut() {
                s.tooltip = text.to_string();
            }
        });
        let mut d = nid(self.hwnd);
        d.uFlags = NIF_TIP;
        copy_wide(&mut d.szTip, text);
        // SAFETY: d is fully initialised.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }

    /// Updates the menu's checked state for "Pause notifications".
    pub fn set_paused(&self, paused: bool) {
        INNER.with(|i| {
            if let Some(s) = i.borrow_mut().as_mut() {
                s.paused = paused;
            }
        });
    }

    /// Replaces the menu labels (language switched in Settings).
    pub fn set_labels(&self, labels: TrayLabels) {
        INNER.with(|i| {
            if let Some(s) = i.borrow_mut().as_mut() {
                s.labels = labels;
            }
        });
    }

    /// Shows a toast. `title` and `body` are already translated and already
    /// reduced to the allowed content level by the caller. A click raises
    /// [`ShellEvent::ToastClicked`] with `target`.
    pub fn show_toast(&self, title: &str, body: &str, target: Option<Target>) {
        INNER.with(|i| {
            if let Some(s) = i.borrow_mut().as_mut() {
                s.last_toast = target;
            }
        });
        let mut d = nid(self.hwnd);
        d.uFlags = NIF_INFO;
        copy_wide(&mut d.szInfoTitle, title);
        copy_wide(&mut d.szInfo, body);
        d.dwInfoFlags = NIIF_NONE;
        // SAFETY: d is fully initialised.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let d = nid(self.hwnd);
        // SAFETY: removes our icon and destroys our window on the thread that made it.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &d);
            DestroyWindow(self.hwnd);
        }
        INNER.with(|i| *i.borrow_mut() = None);
    }
}

// ---- Single instance ------------------------------------------------------

/// Held for the life of the primary instance.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handle came from CreateMutexW.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Result of [`acquire_single_instance`].
pub enum Instance {
    /// This is the first instance; keep the guard alive.
    Primary(InstanceGuard),
    /// Another instance runs; call [`signal_existing_instance`] and exit.
    Secondary,
}

/// Claims the per-session single-instance mutex.
pub fn acquire_single_instance() -> Instance {
    let name = wide("Local\\Komorebi.SingleInstance");
    // SAFETY: name is NUL-terminated and lives across the call.
    unsafe {
        let h = CreateMutexW(null(), 0, name.as_ptr());
        if h.is_null() {
            // Cannot tell; behave as the only instance rather than refuse to start.
            return Instance::Primary(InstanceGuard(null_mut()));
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            Instance::Secondary
        } else {
            Instance::Primary(InstanceGuard(h))
        }
    }
}

/// Asks the running instance to come forward (it gets
/// [`ShellEvent::ActivateRequested`]). Returns false if it was not found, for
/// instance because it runs without a tray. Carries no arguments or data.
pub fn signal_existing_instance() -> bool {
    let class = wide(CLASS_NAME);
    // SAFETY: class is NUL-terminated; the posted message carries no pointers.
    unsafe {
        let hwnd = FindWindowExW(null_mut(), null_mut(), class.as_ptr(), null());
        !hwnd.is_null() && PostMessageW(hwnd, MSG_ACTIVATE, 0, 0) != 0
    }
}

/// Hands a `mailto:` URL (UTF-8 `payload`, from `launch::handoff_payload`)
/// to the running instance, which opens a composer with it. True if it was
/// delivered; false if no instance window was found (no tray) or it did not
/// answer in two seconds. The payload is a plain copy; nothing is sent.
pub fn send_mailto_to_existing_instance(payload: &[u8]) -> bool {
    let class = wide(CLASS_NAME);
    let Ok(len) = u32::try_from(payload.len()) else {
        return false;
    };
    let data = COPYDATASTRUCT {
        dwData: crate::launch::COPYDATA_MAILTO,
        cbData: len,
        lpData: payload.as_ptr() as *mut c_void,
    };
    // SAFETY: class is NUL-terminated; `data` and the payload outlive the
    // synchronous SendMessageTimeoutW, which copies the data across.
    unsafe {
        let hwnd = FindWindowExW(null_mut(), null_mut(), class.as_ptr(), null());
        if hwnd.is_null() {
            return false;
        }
        let mut result: usize = 0;
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            0,
            &data as *const COPYDATASTRUCT as LPARAM,
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        ) != 0
            && result == 1
    }
}

// ---- Start with Windows ---------------------------------------------------

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Komorebi";
const ERROR_FILE_NOT_FOUND: u32 = 2;

fn open_run_key(access: u32) -> Option<HKEY> {
    let sub = wide(RUN_KEY);
    let mut key: HKEY = null_mut();
    // SAFETY: sub is NUL-terminated; key receives the handle.
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut key) };
    (rc == ERROR_SUCCESS).then_some(key)
}

/// True when the HKCU Run key has a Komorebi entry.
pub fn autostart_enabled() -> bool {
    let Some(key) = open_run_key(KEY_QUERY_VALUE) else {
        return false;
    };
    let name = wide(RUN_VALUE);
    // SAFETY: a size-only query; no buffer is written.
    let rc = unsafe {
        let rc = RegQueryValueExW(
            key,
            name.as_ptr(),
            null(),
            null_mut(),
            null_mut(),
            null_mut(),
        );
        RegCloseKey(key);
        rc
    };
    rc == ERROR_SUCCESS
}

/// Adds (`true`) or removes (`false`) the Run entry that starts `exe`
/// minimised to the tray at sign-in. Opt-in only; returns whether it worked.
pub fn set_autostart(enabled: bool, exe: &Path) -> bool {
    let Some(key) = open_run_key(KEY_SET_VALUE) else {
        return false;
    };
    let name = wide(RUN_VALUE);
    // SAFETY: buffers are NUL-terminated and sized in bytes as REG_SZ requires.
    let rc = unsafe {
        let rc = if enabled {
            let data = wide(&crate::notify::run_command(exe));
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        } else {
            let r = RegDeleteValueW(key, name.as_ptr());
            if r == ERROR_FILE_NOT_FOUND {
                ERROR_SUCCESS
            } else {
                r
            }
        };
        RegCloseKey(key);
        rc
    };
    rc == ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taskbar_created_matches_only_when_registered() {
        assert!(is_taskbar_created(0xC123, 0xC123));
        assert!(!is_taskbar_created(0xC124, 0xC123));
        assert!(!is_taskbar_created(0, 0));
    }

    #[test]
    fn copy_wide_truncates_and_terminates() {
        let mut b = [1u16; 4];
        copy_wide(&mut b, "abcdef");
        assert_eq!(b, [97, 98, 99, 0]);
    }
}
