//! SKUA PATCH (G3-F26): the session's display power state.
//!
//! When the power plan turns the display off (Balanced: after 5 min without input), a window
//! that keeps drawing (a blinking cursor, a status tick) spends ~2.5 ms of CPU in every
//! `Present` (measured on the owner machine, AMD RX 9070 XT, driver 32.0.32015.2008: Present
//! 0.6 -> 8.6 Mcyc), for frames nobody sees. The platform window subscribes to
//! `GUID_SESSION_DISPLAY_STATUS`; while it reports "off", windows report `Hidden` and skip
//! their frames. "Dimmed" counts as on.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_util::ResultExt;
use windows::Win32::{
    Foundation::{HANDLE, HWND, LPARAM},
    System::{
        Power::{HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification},
        SystemServices::GUID_SESSION_DISPLAY_STATUS,
    },
    UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
};

static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);

/// Whether the session's display is off (frames would not be shown).
pub(crate) fn is_off() -> bool {
    DISPLAY_OFF.load(Ordering::Acquire)
}

/// Subscribes `hwnd` (the platform window) to display state changes. Windows sends the
/// current state right away.
pub(crate) fn register(hwnd: HWND) -> Option<HPOWERNOTIFY> {
    // SAFETY: `hwnd` is the platform window, which handles `WM_POWERBROADCAST`.
    unsafe {
        RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_SESSION_DISPLAY_STATUS,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    }
    .log_err()
}

/// Reads a `PBT_POWERSETTINGCHANGE` payload. Returns `true` when the off state changed.
pub(crate) fn update(lparam: LPARAM) -> bool {
    let setting = lparam.0 as *const POWERBROADCAST_SETTING;
    if setting.is_null() {
        return false;
    }
    // SAFETY: for `PBT_POWERSETTINGCHANGE`, `lparam` points to a `POWERBROADCAST_SETTING`
    // whose `Data` holds `DataLength` bytes (a DWORD for the display status).
    let off = unsafe {
        let setting = &*setting;
        if setting.PowerSetting != GUID_SESSION_DISPLAY_STATUS || setting.DataLength < 4 {
            return false;
        }
        let state = std::ptr::read_unaligned(setting.Data.as_ptr() as *const u32);
        state == 0
    };
    DISPLAY_OFF.swap(off, Ordering::AcqRel) != off
}
