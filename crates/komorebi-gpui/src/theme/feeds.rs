//! The live feeds from Windows into the tokens: never polled.
//!
//! - Light or dark: GPUI reports Windows' app mode change to every window
//!   (`observe_window_appearance`, from `WM_SETTINGCHANGE`); the window
//!   calls [`refresh_from_windows`].
//! - Text size: the Settings slider may only write the registry, so a
//!   thread waits on `HKCU\Software\Microsoft\Accessibility`
//!   (`komorebi::theme::watch_text_scale`) and hands each change to the UI
//!   thread through a channel ([`watch`]).
//! - Animation effects: no message reaches GPUI for it, so the settings
//!   are read again whenever a Komorebi window becomes active (the user
//!   comes back from the Settings app); reading them costs under 0.1 ms.
//!
//! [`sync_window_frames`] keeps the window frame (DWM's immersive dark
//! mode: the border, the system menu, the Snap flyout) on the app's scheme
//! when the theme setting forces light or dark against Windows.

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, Window};

use super::{ActiveTokens as _, system_changed};

/// Set by a snapshot: animations stay at rest whatever Windows says.
struct HeldStill;

impl gpui::Global for HeldStill {}

/// Keeps animation effects off for the rest of the run (a snapshot), also
/// when a window's activation re-reads Windows' settings.
pub fn hold_still(cx: &mut App) {
    cx.set_global(HeldStill);
    let mut system = komorebi::theme::read();
    system.animate = false;
    system_changed(system, cx);
}

/// Re-reads Windows' appearance settings and installs them.
pub fn refresh_from_windows(cx: &mut App) {
    let mut system = komorebi::theme::read();
    if cx.has_global::<HeldStill>() {
        system.animate = false;
    }
    system_changed(system, cx);
}

/// Starts the text size watcher. Once per app.
pub fn watch(cx: &mut App) {
    let (tx, mut rx) = mpsc::unbounded::<()>();
    komorebi::theme::watch_text_scale(move || {
        // A closed channel means the app is gone; the thread ends with it.
        let _ = tx.unbounded_send(());
    });
    cx.spawn(async move |cx| {
        while rx.next().await.is_some() {
            // A burst of registry writes is one refresh.
            while rx.try_recv().is_ok() {}
            cx.update(refresh_from_windows);
        }
    })
    .detach();
}

/// Puts every window's frame on the app's scheme, after GPUI's own
/// appearance handling for the current message has run.
pub fn sync_window_frames(cx: &mut App) {
    let dark = cx.tokens().scheme.is_dark();
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            for handle in cx.windows() {
                // A window being torn down is skipped.
                let _ = handle.update(cx, |_, window, _| set_frame_dark(window, dark));
            }
        });
    })
    .detach();
}

/// DWM's immersive dark mode on one window.
#[cfg(windows)]
pub fn set_frame_dark(window: &Window, dark: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    use windows::core::BOOL;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return;
    };
    let value = BOOL::from(dark);
    // SAFETY: GPUI's live window on its own thread, and a BOOL with its
    // size, as GPUI's `configure_dwm_dark_mode` passes it.
    let result = unsafe {
        DwmSetWindowAttribute(
            HWND(h.hwnd.get() as _),
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const value).cast(),
            std::mem::size_of::<BOOL>() as u32,
        )
    };
    if let Err(e) = result {
        eprintln!("komorebi-gpui: dark window frame: {e}");
    }
}

#[cfg(not(windows))]
pub fn set_frame_dark(_: &Window, _: bool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// A snapshot's reduce motion holds when the window's activation
    /// re-reads Windows' settings (stage A's remaining problem).
    #[gpui::test]
    fn a_held_still_run_keeps_reduce_motion_after_a_refresh(cx: &mut TestAppContext) {
        crate::test_support::init_app(cx);
        cx.update(|cx| {
            hold_still(cx);
            refresh_from_windows(cx);
            assert!(cx.tokens().reduce_motion);
            assert!(cx.reduce_motion());
        });
    }
}
