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
//! the message loop the app already runs. Events reach the app through the
//! callback given to `new`, always on that thread.
//!
//! The window is a hidden top-level one (never shown), not a message-only one,
//! because only top-level windows receive the `TaskbarCreated` broadcast; on
//! it the tray icon is added again after an Explorer restart, with the current
//! tooltip. It also hears `WM_SETTINGCHANGE`, which says when the taskbar's
//! theme changed.
//!
//! Tray icon: an exe that carries the brand tray icons (resources 101 to 103,
//! komorebi.exe through `assets/komorebi.rc`) shows the seal
//! in ink on a light taskbar, white on a dark one (`SystemUsesLightTheme`,
//! not the app's theme) and persimmon while unread mail waits
//! ([`Shell::set_unread`]); its notifications carry the seal (resource 1) as
//! their large icon. An exe without them (the library's tests and examples)
//! shows its own icon 1 in every state. See docs/BRANDING.md.

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
use windows_sys::Win32::System::LibraryLoader::{FindResourceW, GetModuleHandleW};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_DWORD, RegCloseKey,
    RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_LARGE_ICON, NIIF_NONE, NIIF_USER, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, FindWindowExW, GetCursorPos, GetSystemMetrics, HICON, IDI_APPLICATION,
    IMAGE_ICON, LoadIconW, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING, PostMessageW,
    RegisterClassW, RegisterWindowMessageW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_COPYDATA, WM_ENDSESSION, WM_LBUTTONUP, WM_NULL,
    WM_QUERYENDSESSION, WM_RBUTTONUP, WM_SETTINGCHANGE, WNDCLASSW,
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

/// Resource id of the exe's own icon (the seal in komorebi.exe).
const APP_ICON: u16 = 1;

/// The notification area icon for a state of the taskbar and the mail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayIcon {
    /// The ink seal, for a light taskbar.
    Light,
    /// The white seal, for a dark taskbar.
    Dark,
    /// The persimmon seal while unread mail waits, on either taskbar.
    Unread,
}

impl TrayIcon {
    /// Persimmon while mail is unread (it contrasts with both taskbars,
    /// 4.6:1 and 3.2:1); otherwise the glyph that contrasts with the taskbar
    /// (the crossed pairs are 1.1:1, so the theme must decide).
    fn pick(light_taskbar: bool, unread: bool) -> TrayIcon {
        match (unread, light_taskbar) {
            (true, _) => TrayIcon::Unread,
            (false, true) => TrayIcon::Light,
            (false, false) => TrayIcon::Dark,
        }
    }

    /// The icon's resource id in `assets/komorebi.rc`.
    fn resource_id(self) -> u16 {
        match self {
            TrayIcon::Light => 101,
            TrayIcon::Dark => 102,
            TrayIcon::Unread => 103,
        }
    }
}

const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const LIGHT_TASKBAR_VALUE: &str = "SystemUsesLightTheme";

/// The taskbar is light only when `SystemUsesLightTheme` is 1. A missing
/// value (Windows 10 before 1903, whose taskbar is always dark) or any other
/// value means dark.
fn light_taskbar_from(value: Option<u32>) -> bool {
    value == Some(1)
}

/// True when a `WM_SETTINGCHANGE` names the area Windows sends when the
/// light or dark mode (or the accent) changed: "ImmersiveColorSet".
fn is_colour_change(area: &[u16]) -> bool {
    area.iter().copied().eq("ImmersiveColorSet".encode_utf16())
}

/// Reads `SystemUsesLightTheme` for the current user.
fn read_light_taskbar() -> bool {
    let key = wide(PERSONALIZE_KEY);
    let name = wide(LIGHT_TASKBAR_VALUE);
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: both strings are NUL-terminated; data and size are valid for a
    // REG_DWORD read, which RRF_RT_REG_DWORD enforces.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    light_taskbar_from((rc == ERROR_SUCCESS).then_some(data))
}

/// True when this exe carries the brand tray icons (komorebi.exe).
fn has_brand_tray_icons() -> bool {
    // SAFETY: a lookup in this exe's resource table; nothing is loaded.
    unsafe {
        let module = GetModuleHandleW(null());
        !FindResourceW(
            module,
            std::ptr::without_provenance::<u16>(TrayIcon::Light.resource_id() as usize),
            std::ptr::without_provenance::<u16>(RT_GROUP_ICON),
        )
        .is_null()
    }
}

/// RT_GROUP_ICON (MAKEINTRESOURCE(14)).
const RT_GROUP_ICON: usize = 14;

/// Loads icon resource `id` of this exe at the system's small (`large` =
/// false) or large icon size for the current DPI, picking the best image of
/// the ICO. The caller owns the handle (DestroyIcon). Null when missing.
fn load_icon(id: u16, large: bool) -> HICON {
    // SAFETY: plain metric queries and a resource load from this exe.
    unsafe {
        let (cx, cy) = if large {
            (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON))
        } else {
            (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON))
        };
        LoadImageW(
            GetModuleHandleW(null()),
            std::ptr::without_provenance::<u16>(id as usize),
            IMAGE_ICON,
            cx,
            cy,
            0,
        ) as HICON
    }
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
    /// The exe carries the brand tray icons ([`has_brand_tray_icons`]).
    brand: bool,
    /// `SystemUsesLightTheme`, refreshed on `WM_SETTINGCHANGE`.
    light_taskbar: bool,
    /// Unread mail waits ([`Shell::set_unread`]).
    unread: bool,
    /// The brand tray icon the shell shows now, owned here (null without
    /// brand icons, where the shared app icon is used).
    icon: HICON,
    /// The seal for notifications, loaded on the first one (brand only).
    toast_icon: HICON,
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

/// Puts the tray icon for the current state into `d.hIcon`, runs `call`
/// (the Shell_NotifyIconW that shows it), then keeps a loaded brand icon in
/// INNER and destroys the one it replaces. Without brand icons it is the
/// exe's shared icon 1, which is never destroyed.
fn with_tray_icon(d: &mut NOTIFYICONDATAW, call: impl FnOnce(&NOTIFYICONDATAW) -> bool) -> bool {
    let state = INNER.with(|i| {
        i.try_borrow()
            .ok()
            .and_then(|b| b.as_ref().map(|s| (s.brand, s.light_taskbar, s.unread)))
    });
    let owned = match state {
        Some((true, light, unread)) => {
            load_icon(TrayIcon::pick(light, unread).resource_id(), false)
        }
        _ => null_mut(),
    };
    d.hIcon = if owned.is_null() { app_icon() } else { owned };
    let shown = call(d);
    if !owned.is_null() {
        let old = INNER.with(|i| {
            i.try_borrow_mut()
                .ok()
                .and_then(|mut b| b.as_mut().map(|s| std::mem::replace(&mut s.icon, owned)))
        });
        // SAFETY: icons this module loaded with LoadImageW and owns; the
        // shell keeps its own copy of the one it shows.
        unsafe {
            match old {
                Some(old) if !old.is_null() => {
                    DestroyIcon(old);
                }
                Some(_) => {}
                None => {
                    DestroyIcon(owned);
                }
            }
        }
    }
    shown
}

/// Shows the tray icon for the current state (after a theme or unread change).
fn modify_icon(hwnd: HWND) {
    let mut d = nid(hwnd);
    d.uFlags = NIF_ICON;
    // SAFETY: d is fully initialised.
    with_tray_icon(&mut d, |d| unsafe { Shell_NotifyIconW(NIM_MODIFY, d) != 0 });
}

/// Re-reads the taskbar theme and swaps the brand tray icon if it changed.
fn refresh_theme(hwnd: HWND) {
    let brand = INNER.with(|i| {
        i.try_borrow()
            .ok()
            .and_then(|b| b.as_ref().map(|s| s.brand))
    });
    if brand != Some(true) {
        return;
    }
    let light = read_light_taskbar();
    let changed = INNER.with(|i| {
        i.try_borrow_mut().ok().and_then(|mut b| {
            b.as_mut().map(|s| {
                let changed = s.light_taskbar != light;
                s.light_taskbar = light;
                changed
            })
        })
    });
    if changed == Some(true) {
        modify_icon(hwnd);
    }
}

/// The NUL-terminated UTF-16 string at `p` (at most 256 units), or empty.
///
/// # Safety
/// `p` is null or points to a NUL-terminated UTF-16 string that stays valid
/// for the returned lifetime.
unsafe fn wide_at<'a>(p: *const u16) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller guarantees a NUL within the string; the cap stops a
    // runaway read on a malformed one at a length no area name reaches.
    unsafe {
        while n < 256 && *p.add(n) != 0 {
            n += 1;
        }
        std::slice::from_raw_parts(p, n)
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
        WM_SETTINGCHANGE => {
            // SAFETY: for WM_SETTINGCHANGE, lParam is null or a NUL-terminated
            // area name valid for the call.
            let area = unsafe { wide_at(lp as *const u16) };
            if is_colour_change(area) {
                // The light or dark mode changed: the taskbar may have too.
                refresh_theme(hwnd);
            }
            // SAFETY: forwarding the message with its original arguments.
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
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
    copy_wide(&mut d.szTip, &tip);
    // SAFETY: d is fully initialised.
    with_tray_icon(&mut d, |d| unsafe { Shell_NotifyIconW(NIM_ADD, d) != 0 })
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
        let brand = has_brand_tray_icons();
        INNER.with(|i| {
            *i.borrow_mut() = Some(Inner {
                labels,
                paused: false,
                tooltip: tooltip.to_string(),
                last_toast: None,
                handler: Rc::new(handler),
                brand,
                light_taskbar: brand && read_light_taskbar(),
                unread: false,
                icon: null_mut(),
                toast_icon: null_mut(),
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

    /// Says whether unread mail waits: the brand tray seal turns persimmon
    /// while it does. Nothing changes in an exe without the brand icons.
    pub fn set_unread(&self, unread: bool) {
        let changed = INNER.with(|i| {
            i.borrow_mut().as_mut().map(|s| {
                let changed = s.brand && s.unread != unread;
                s.unread = unread;
                changed
            })
        });
        if changed == Some(true) {
            modify_icon(self.hwnd);
        }
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
        // With the brand icons, the notification carries the seal (icon 1 at
        // the large icon size) as its image; otherwise Windows' default.
        let toast_icon = INNER.with(|i| {
            i.borrow_mut().as_mut().and_then(|s| {
                s.last_toast = target;
                if s.brand && s.toast_icon.is_null() {
                    s.toast_icon = load_icon(APP_ICON, true);
                }
                (s.brand && !s.toast_icon.is_null()).then_some(s.toast_icon)
            })
        });
        let mut d = nid(self.hwnd);
        d.uFlags = NIF_INFO;
        copy_wide(&mut d.szInfoTitle, title);
        copy_wide(&mut d.szInfo, body);
        match toast_icon {
            Some(icon) => {
                d.hBalloonIcon = icon;
                d.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
            }
            None => d.dwInfoFlags = NIIF_NONE,
        }
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
        if let Some(s) = INNER.with(|i| i.borrow_mut().take()) {
            for icon in [s.icon, s.toast_icon] {
                if !icon.is_null() {
                    // SAFETY: icons this module loaded and owns; the shell
                    // no longer shows them.
                    unsafe { DestroyIcon(icon) };
                }
            }
        }
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
    fn tray_icon_follows_the_taskbar_and_turns_persimmon_while_unread() {
        assert_eq!(TrayIcon::pick(true, false), TrayIcon::Light);
        assert_eq!(TrayIcon::pick(false, false), TrayIcon::Dark);
        assert_eq!(TrayIcon::pick(true, true), TrayIcon::Unread);
        assert_eq!(TrayIcon::pick(false, true), TrayIcon::Unread);
    }

    #[test]
    fn tray_icon_resource_ids_match_the_brand_rc() {
        let ids = [TrayIcon::Light, TrayIcon::Dark, TrayIcon::Unread].map(TrayIcon::resource_id);
        assert_eq!(ids, [101, 102, 103]);
        assert!(!ids.contains(&APP_ICON));
        let rc = include_str!("../assets/komorebi.rc");
        for (id, file) in ids.iter().zip(["tray-light", "tray-dark", "tray-unread"]) {
            assert!(
                rc.contains(&format!("{id} ICON \"tray/{file}.ico\"")),
                "{id} {file}"
            );
        }
        assert!(rc.contains(&format!("{APP_ICON} ICON \"komorebi.ico\"")));
    }

    #[test]
    fn light_taskbar_only_when_the_value_is_one() {
        assert!(light_taskbar_from(Some(1)));
        assert!(!light_taskbar_from(Some(0)));
        assert!(!light_taskbar_from(Some(2)));
        // Windows 10 before 1903 has no value and a dark taskbar.
        assert!(!light_taskbar_from(None));
    }

    #[test]
    fn colour_change_is_the_immersive_color_set_area() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert!(is_colour_change(&w("ImmersiveColorSet")));
        assert!(!is_colour_change(&w("immersivecolorset ")));
        assert!(!is_colour_change(&w("WindowsThemeElement")));
        assert!(!is_colour_change(&w("Policy")));
        assert!(!is_colour_change(&[]));
    }

    #[test]
    fn wide_at_reads_up_to_the_nul_and_accepts_null() {
        let s = wide("ImmersiveColorSet");
        // SAFETY: `s` is NUL-terminated and outlives the slice.
        assert!(is_colour_change(unsafe { wide_at(s.as_ptr()) }));
        // SAFETY: null is allowed.
        assert!(unsafe { wide_at(null()) }.is_empty());
    }

    #[test]
    fn this_test_exe_has_no_brand_tray_icons() {
        // The library's tests carry no resources 101 to
        // 103: the tray keeps icon 1 and notifications keep Windows' image.
        assert!(!has_brand_tray_icons());
    }

    #[test]
    fn copy_wide_truncates_and_terminates() {
        let mut b = [1u16; 4];
        copy_wide(&mut b, "abcdef");
        assert_eq!(b, [97, 98, 99, 0]);
    }
}
