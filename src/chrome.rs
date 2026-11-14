//! Which title bar the window has: app-drawn ("custom") or the system
//! caption. The GPUI app asks [`custom_wanted`] once at start-up
//! (`crates/komorebi-gpui/src/titlebar.rs`); the bar itself, its hit test
//! and the caption buttons are GPUI's (`window_control_area`).

/// The first Windows 11 build number (22000): the app-drawn bar is wanted
/// from here on.
pub const WINDOWS_11: u32 = 22000;

/// Whether the bar is app-drawn ("custom") rather than the system caption:
/// on Windows 11 (`build`), not under high contrast, and not switched off
/// with `KOMOREBI_SYSTEM_TITLEBAR` (`system_switch`). A snapshot draws the
/// custom bar unless under high contrast; `forced` (`custom` or `system`,
/// from `KOMOREBI_SNAPSHOT_TITLEBAR`) wins over everything.
pub fn custom_wanted(
    build: u32,
    high_contrast: bool,
    system_switch: bool,
    snapshot: bool,
    forced: Option<&str>,
) -> bool {
    match forced {
        Some("custom") => return true,
        Some("system") => return false,
        _ => {}
    }
    if high_contrast {
        return false;
    }
    snapshot || (build >= WINDOWS_11 && !system_switch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_on_windows_11_without_high_contrast() {
        assert!(custom_wanted(22631, false, false, false, None));
        assert!(!custom_wanted(19045, false, false, false, None));
        assert!(!custom_wanted(22631, true, false, false, None));
        assert!(!custom_wanted(22631, false, true, false, None));
        // Snapshots draw it on any build, but not under high contrast.
        assert!(custom_wanted(19045, false, false, true, None));
        assert!(!custom_wanted(22631, true, false, true, None));
        assert!(custom_wanted(19045, true, true, true, Some("custom")));
        assert!(!custom_wanted(22631, false, false, true, Some("system")));
        assert!(custom_wanted(22631, false, false, false, Some("other")));
    }
}
