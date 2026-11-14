//! DirectX pre-flight. GPUI draws with Direct3D 11 and, when it cannot create
//! a device, panics ("failed to initialize DirectX"); with `panic = "abort"`
//! the app would vanish without a word. So before GPUI connects, the same
//! device GPUI would make is created here (the first DXGI adapter that gives
//! a D3D11 device at feature level 11.1, 11.0 or 10.1 with structured
//! buffers, or WARP under `KOMOREBI_GPUI_FORCE_WARP`) and kept for GPUI's
//! platform, which takes it rather than creating a second one, and on
//! failure a native message box says so in English or European Portuguese
//! and the app exits with [`EXIT_NO_DIRECTX`].
//!
//! The text is a catalog message (`tr!`) in the Windows display language
//! ([`komorebi::i18n::startup_lang`]), never the saved setting (the cache is
//! not read yet).

use komorebi::i18n::Lang;
use komorebi_i18n::{tr, with_lang};

/// The exit code when no DirectX 11 device can be made.
pub const EXIT_NO_DIRECTX: u8 = 3;

/// Development switch: draw with WARP (the software rasterizer every
/// Windows has); the vendored gpui-pre-windows reads the same variable.
pub const FORCE_WARP: &str = "KOMOREBI_GPUI_FORCE_WARP";

/// Development switch: behave as if no device could be made (shows the
/// message box and exits), to check the dialog without breaking DirectX.
pub const FAIL: &str = "KOMOREBI_GPUI_PREFLIGHT_FAIL";

const TITLE: &str = "Komorebi";

/// The message box text in `lang`, with the technical reason (English
/// machine text) after it.
pub fn message(lang: Lang, reason: &str) -> String {
    with_lang(lang, || {
        tr!(
            "Komorebi can't start: Windows could not create a DirectX 11 graphics device on this PC.\n\nUpdate the graphics driver and try again.\n\n{0}",
            reason
        )
    })
}

/// Whether a development switch is set (set but empty counts as unset).
fn switch(name: &str) -> bool {
    komorebi::dev_var(name).is_some()
}

/// Creates the D3D11 device GPUI will draw with and hands it to GPUI's
/// platform, which takes it instead of making a second one
/// (`gpui_windows::prepare_devices`; the first DXGI adapter that gives a
/// device at feature level 11.1, 11.0 or 10.1 with structured buffers, or
/// WARP under [`FORCE_WARP`]). `Err` holds the reason, in English (machine
/// text).
#[cfg(windows)]
pub fn probe() -> Result<(), String> {
    if switch(FAIL) {
        return Err(format!("{FAIL} is set"));
    }
    probe_device(switch(FORCE_WARP))
}

#[cfg(windows)]
fn probe_device(warp: bool) -> Result<(), String> {
    gpui_windows::prepare_devices(warp)
}

#[cfg(not(windows))]
pub fn probe() -> Result<(), String> {
    if switch(FAIL) {
        return Err(format!("{FAIL} is set"));
    }
    Ok(())
}

/// Shows the failure in a native message box (modal, no owner window).
#[cfg(windows)]
pub fn report(lang: Lang, reason: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    let text = HSTRING::from(message(lang, reason));
    let title = HSTRING::from(TITLE);
    // SAFETY: both strings outlive the modal call.
    unsafe { MessageBoxW(None, &text, &title, MB_OK | MB_ICONERROR) };
}

#[cfg(not(windows))]
pub fn report(lang: Lang, reason: &str) {
    eprintln!("{}", message(lang, reason));
}

/// Says a start-up failure in a native error box (no window yet: the
/// release build has no console).
#[cfg(windows)]
pub fn fail(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    let text = HSTRING::from(text);
    let title = HSTRING::from(TITLE);
    // SAFETY: both strings outlive the modal call.
    unsafe { MessageBoxW(None, &text, &title, MB_OK | MB_ICONERROR) };
}

#[cfg(not(windows))]
pub fn fail(text: &str) {
    eprintln!("{text}");
}

/// Says `text` in a native message box after the window is gone (what
/// Remove all data could not remove).
#[cfg(windows)]
pub fn warn(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    let text = HSTRING::from(text);
    let title = HSTRING::from(TITLE);
    // SAFETY: both strings outlive the modal call.
    unsafe { MessageBoxW(None, &text, &title, MB_OK | MB_ICONWARNING) };
}

#[cfg(not(windows))]
pub fn warn(text: &str) {
    eprintln!("{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_follows_the_language() {
        let en = message(Lang::En, "adapter 0: E_FAIL");
        assert!(en.starts_with("Komorebi can't start"), "{en}");
        assert!(en.ends_with("\n\nadapter 0: E_FAIL"), "{en}");
        let pt = message(Lang::PtPt, "x");
        assert!(pt.starts_with("O Komorebi não consegue iniciar"), "{pt}");
        // European Portuguese (AO90): "controlador gráfico", "Atualize",
        // never the Brazilian "driver" or "placa de vídeo".
        assert!(pt.contains("Atualize o controlador gráfico"), "{pt}");
        assert!(pt.ends_with("\n\nx"), "{pt}");
        assert!(!pt.contains("driver") && !pt.contains("vídeo"), "{pt}");
        for text in [en, pt] {
            assert!(!text.contains('\u{2013}') && !text.contains('\u{2014}'));
        }
    }

    /// WARP is part of every Windows 10 and 11, GPU or not, so the probe
    /// passes on it (CI runners have no GPU). No window is opened.
    #[cfg(windows)]
    #[test]
    fn warp_passes_the_probe() {
        assert_eq!(probe_device(true), Ok(()));
    }
}
