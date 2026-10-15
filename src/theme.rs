//! The Windows appearance settings Komorebi follows, as a WinUI 3 app does
//! (docs/WINUI3.md "Following the user's Windows theme"): the accent ramp,
//! light or dark apps, high contrast, text size, animation effects and
//! whether title bars show the accent. Read with plain Win32 calls (well
//! under 0.1 ms in all), once before the first frame and again when Windows
//! says a setting changed; never polled, never stored.

/// An opaque sRGB colour.
pub type Rgb = [u8; 3];

/// The accent ramp WinUI falls back to (`SystemThemingInterop.cpp`), Dark3
/// to Light3; index 3 is the accent itself.
pub const FALLBACK_RAMP: [Rgb; 7] = [
    [0x00, 0x26, 0x42],
    [0x00, 0x42, 0x75],
    [0x00, 0x5a, 0x9e],
    [0x00, 0x78, 0xd7],
    [0x42, 0x9c, 0xe3],
    [0x76, 0xb9, 0xed],
    [0xa6, 0xd8, 0xff],
];

/// The high-contrast colours, in this order: Window, WindowText, Highlight,
/// HighlightText, Hotlight, GrayText, ButtonFace, ButtonText.
pub type Contrast = [Rgb; 8];

/// The contrast themes Windows 11 ships (Settings > Accessibility >
/// Contrast themes), read from `C:\Windows\Resources\Ease of Access
/// Themes\*.theme`, for `KOMOREBI_CONTRAST`.
pub const CONTRAST_PRESETS: [(&str, &str); 4] = [
    (
        "hc1",
        "#2d3236,#ffffff,#a1bfde,#212d3b,#70ebde,#a6a6a6,#2d3236,#b6f6f0",
    ),
    (
        "hc2",
        "#000000,#ffffff,#d6b4fd,#2b2b2b,#8080ff,#a6a6a6,#000000,#ffee32",
    ),
    (
        "hcblack",
        "#202020,#ffffff,#8ee3f0,#263b50,#75e9fc,#a6a6a6,#202020,#ffffff",
    ),
    (
        "hcwhite",
        "#fffaef,#3d3d3d,#903909,#fff5e3,#1c5e75,#676767,#fffaef,#202020",
    ),
];

/// What Windows' appearance settings say.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemTheme {
    /// The accent ramp, Dark3 to Light3 (index 3 is the accent).
    pub ramp: [Rgb; 7],
    /// Apps use the dark theme.
    pub dark: bool,
    /// A contrast theme is on, with its colours.
    pub contrast: Option<Contrast>,
    /// Text size, 1.0 to 2.25 (Settings > Accessibility > Text size).
    pub text_scale: f32,
    /// Animation effects are on.
    pub animate: bool,
    /// "Show accent colour on title bars and window borders" is on.
    pub prevalence: bool,
    /// The text caret's blink: half a cycle in ms (`GetCaretBlinkTime`);
    /// 0 when it does not blink (no blinking set, or animation effects off).
    pub caret_blink_ms: u32,
}

impl Default for SystemTheme {
    fn default() -> Self {
        SystemTheme {
            ramp: FALLBACK_RAMP,
            dark: false,
            contrast: None,
            text_scale: 1.0,
            animate: true,
            prevalence: false,
            caret_blink_ms: DEFAULT_BLINK_MS,
        }
    }
}

/// Windows' default caret blink (half a cycle).
pub const DEFAULT_BLINK_MS: u32 = 530;

/// Windows' appearance settings, with the development switches over them.
pub fn read() -> SystemTheme {
    let mut theme = read_system();
    dev_overrides(&mut theme);
    theme
}

/// The caret's blink from `GetCaretBlinkTime` (`raw`): 0 (steady) when it
/// is INFINITE, when animation effects are off, or when the editor should
/// hold still (a snapshot); a failed read (0) is Windows' default. Kept
/// within 100 to 5000 ms.
pub fn caret_blink(raw: u32, animate: bool) -> u32 {
    if !animate || raw == u32::MAX {
        return 0;
    }
    if raw == 0 {
        return DEFAULT_BLINK_MS;
    }
    raw.clamp(100, 5000)
}

/// The ramp from `AccentPalette`: 32 bytes, 8 entries of `[R, G, B, A]`,
/// Light3 to Dark3 and an unused one. None for anything else.
pub fn parse_accent_palette(bytes: &[u8]) -> Option<[Rgb; 7]> {
    if bytes.len() != 32 {
        return None;
    }
    Some(std::array::from_fn(|i| {
        let c = &bytes[(6 - i) * 4..];
        [c[0], c[1], c[2]]
    }))
}

/// A Win32 COLORREF (`0x00BBGGRR`) as RGB.
pub fn colorref(c: u32) -> Rgb {
    [c as u8, (c >> 8) as u8, (c >> 16) as u8]
}

/// RGB as a Win32 COLORREF.
pub fn to_colorref(c: Rgb) -> u32 {
    u32::from(c[0]) | u32::from(c[1]) << 8 | u32::from(c[2]) << 16
}

/// The Fluent 2 type ramp's Display size (the onboarding welcome's
/// headline), in px at 100 % text size; scaled as the rest of the ramp.
pub const DISPLAY: f32 = 40.0;

/// A font size scaled for the text size `factor` (1.0 to 2.25), as WinUI's
/// `TextFormatting::GetScaledFontSize` does: small text grows the most.
pub fn scaled(size: f32, factor: f32) -> f32 {
    let s = size.max(1.0);
    s + (18.0 - std::f32::consts::E * s.ln()).max(0.0) * (factor - 1.0)
}

/// An approximate ramp around `accent`, for snapshots of accents whose real
/// ramp is unknown (Windows' algorithm is not public): the light steps mix
/// it towards white, the dark ones towards black.
pub fn approx_ramp(accent: Rgb) -> [Rgb; 7] {
    let mix = |to: u8, t: f32| -> Rgb {
        accent.map(|c| (f32::from(c) + (f32::from(to) - f32::from(c)) * t).round() as u8)
    };
    [
        mix(0, 0.69),
        mix(0, 0.45),
        mix(0, 0.22),
        accent,
        mix(255, 0.25),
        mix(255, 0.45),
        mix(255, 0.65),
    ]
}

/// `N` comma-separated `#rrggbb` colours.
pub fn parse_colors<const N: usize>(s: &str) -> Option<[Rgb; N]> {
    let colors: Vec<Rgb> = s
        .split(',')
        .map(|c| parse_hex(c.trim()))
        .collect::<Option<_>>()?;
    colors.try_into().ok()
}

fn parse_hex(s: &str) -> Option<Rgb> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// WCAG 2 relative luminance (0 black to 1 white).
pub fn luminance(c: Rgb) -> f32 {
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}

/// The title bar's caption colour (None: Windows' own), so caption and
/// window read as one surface (docs/WINUI3.md "Title bar and frame"):
/// Windows' own under high contrast and when title bars show the accent,
/// else the window's `layer` colour.
pub fn caption_color(theme: &SystemTheme, layer: Rgb) -> Option<Rgb> {
    (theme.contrast.is_none() && !theme.prevalence).then_some(layer)
}

/// Development switches for snapshots: `KOMOREBI_ACCENT` (`#rrggbb`, an
/// approximate ramp, or 7 colours Dark3 to Light3), `KOMOREBI_THEME=contrast`
/// with `KOMOREBI_CONTRAST` (a preset name or 8 colours; hcblack if unset),
/// `KOMOREBI_TEXT_SCALE` (1.0 to 2.25) and `KOMOREBI_ANIMATE=0`.
fn dev_overrides(theme: &mut SystemTheme) {
    let var = |name: &str| crate::dev_var(name).and_then(|v| v.into_string().ok());
    if let Some(accent) = var("KOMOREBI_ACCENT") {
        if let Some(ramp) = parse_colors::<7>(&accent) {
            theme.ramp = ramp;
        } else if let Some([c]) = parse_colors::<1>(&accent) {
            theme.ramp = approx_ramp(c);
        }
    }
    if var("KOMOREBI_THEME").as_deref() == Some("contrast") {
        let pick = var("KOMOREBI_CONTRAST").unwrap_or_else(|| "hcblack".into());
        let list = CONTRAST_PRESETS
            .iter()
            .find(|(name, _)| *name == pick)
            .map_or(pick.as_str(), |(_, list)| list);
        theme.contrast = parse_colors(list).or(theme.contrast);
    }
    if let Some(f) = var("KOMOREBI_TEXT_SCALE").and_then(|s| s.parse::<f32>().ok()) {
        theme.text_scale = f.clamp(1.0, 2.25);
    }
    if var("KOMOREBI_ANIMATE").as_deref() == Some("0") {
        theme.animate = false;
        theme.caret_blink_ms = 0;
    }
    // A snapshot shows the caret, never a blink's off half.
    if crate::dev_var("KOMOREBI_SNAPSHOT").is_some() {
        theme.caret_blink_ms = 0;
    }
}

#[cfg(windows)]
fn read_system() -> SystemTheme {
    use windows_sys::Win32::Graphics::Gdi::{
        COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
        COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
    };
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SystemParametersInfoW,
    };
    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let mut palette = [0u8; 33];
    let ramp = reg_binary(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent",
        "AccentPalette",
        &mut palette,
    )
    .and_then(parse_accent_palette)
    .unwrap_or(FALLBACK_RAMP);
    let light = reg_dword(HKEY_CURRENT_USER, PERSONALIZE, "AppsUseLightTheme")
        .or_else(|| reg_dword(HKEY_LOCAL_MACHINE, PERSONALIZE, "SystemUsesLightTheme"))
        .unwrap_or(1);
    let factor = reg_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Accessibility",
        "TextScaleFactor",
    )
    .unwrap_or(100)
    .clamp(100, 225);
    let prevalence = reg_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\DWM",
        "ColorPrevalence",
    );
    let mut hc = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        dwFlags: 0,
        lpszDefaultScheme: std::ptr::null_mut(),
    };
    // SAFETY: a sized HIGHCONTRASTW the call fills in.
    let hc_ok =
        unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&raw mut hc).cast(), 0) };
    let contrast = (hc_ok != 0 && hc.dwFlags & HCF_HIGHCONTRASTON != 0).then(|| {
        [
            COLOR_WINDOW,
            COLOR_WINDOWTEXT,
            COLOR_HIGHLIGHT,
            COLOR_HIGHLIGHTTEXT,
            COLOR_HOTLIGHT,
            COLOR_GRAYTEXT,
            COLOR_BTNFACE,
            COLOR_BTNTEXT,
        ]
        // SAFETY: plain reads of system colour indexes.
        .map(|i| colorref(unsafe { GetSysColor(i) }))
    });
    let mut animate: windows_sys::core::BOOL = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL.
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0) };
    // SAFETY: a plain read with no arguments.
    let blink = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime() };
    SystemTheme {
        ramp,
        dark: light == 0,
        contrast,
        text_scale: factor as f32 / 100.0,
        animate: animate != 0,
        prevalence: prevalence == Some(1),
        caret_blink_ms: caret_blink(blink, animate != 0),
    }
}

#[cfg(not(windows))]
fn read_system() -> SystemTheme {
    SystemTheme::default()
}

/// A registry value under HKCU, read into `buf`: the bytes written.
#[cfg(windows)]
fn reg_binary<'a>(key: &str, value: &str, buf: &'a mut [u8]) -> Option<&'a [u8]> {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW,
    };
    let (key, value) = (wide(key), wide(value));
    let mut len = buf.len() as u32;
    // SAFETY: NUL-terminated names, and a buffer of `len` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (status == 0).then(|| &buf[..(len as usize).min(buf.len())])
}

/// A DWORD registry value.
#[cfg(windows)]
fn reg_dword(
    root: windows_sys::Win32::System::Registry::HKEY,
    key: &str,
    value: &str,
) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_DWORD, RegGetValueW};
    let (key, value) = (wide(key), wide(value));
    let (mut data, mut len) = (0u32, 4u32);
    // SAFETY: NUL-terminated names, and a DWORD with its size.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut data).cast(),
            &mut len,
        )
    };
    (status == 0).then_some(data)
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The Windows build number (`CurrentBuildNumber`; 22000 and up is
/// Windows 11), 0 when it can't be read. Read once (about 5 µs).
pub fn windows_build() -> u32 {
    static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *BUILD.get_or_init(read_build)
}

#[cfg(windows)]
fn read_build() -> u32 {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let value = wide("CurrentBuildNumber");
    let mut buf = [0u16; 16];
    let mut len = std::mem::size_of_val(&buf) as u32;
    // SAFETY: NUL-terminated names and a buffer with its size in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if status != 0 {
        return 0;
    }
    let chars = (len as usize / 2).min(buf.len());
    String::from_utf16_lossy(&buf[..chars])
        .trim_end_matches('\0')
        .trim()
        .parse()
        .unwrap_or(0)
}

#[cfg(not(windows))]
fn read_build() -> u32 {
    0
}

/// The uxtheme `PreferredAppMode` for native menus (the tray menu, the
/// window's system menu): ForceDark (2) or ForceLight (3).
pub fn menu_mode(dark: bool) -> i32 {
    if dark { 2 } else { 3 }
}

/// Native menus dark or light with the app (docs/WINUI3.md "MenuFlyout / ToolTip / Flyout"): the
/// undocumented uxtheme `SetPreferredAppMode` (ordinal 135) then
/// `FlushMenuThemes` (ordinal 136), as Explorer and the Windows Terminal
/// do. Only on build 18362 and later (1809's ordinal 135 takes a BOOL);
/// anything missing is skipped quietly.
#[cfg(windows)]
pub fn set_menu_theme(dark: bool) {
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };
    if windows_build() < 18362 {
        return;
    }
    let name = wide("uxtheme.dll");
    // SAFETY: a NUL-terminated system DLL name; the module stays loaded
    // for the process (uxtheme is loaded by every themed window anyway).
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return;
    }
    // SAFETY: lookups by ordinal (a pointer value below 0x10000).
    let (set, flush) = unsafe {
        (
            GetProcAddress(module, 135 as *const u8),
            GetProcAddress(module, 136 as *const u8),
        )
    };
    let (Some(set), Some(flush)) = (set, flush) else {
        return;
    };
    // SAFETY: the signatures these ordinals have had since build 18362:
    // `PreferredAppMode SetPreferredAppMode(PreferredAppMode)` and
    // `void FlushMenuThemes()`.
    unsafe {
        let set: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set);
        let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
        set(menu_mode(dark));
        flush();
    }
}

#[cfg(not(windows))]
pub fn set_menu_theme(_: bool) {}

/// Call `changed` (from a background thread) whenever a value under
/// `HKCU\Software\Microsoft\Accessibility` changes: the Text size slider
/// writes `TextScaleFactor` there without always sending
/// `WM_SETTINGCHANGE`. One thread waiting in the kernel, never polling;
/// it ends quietly if the key cannot be watched.
#[cfg(windows)]
pub fn watch_text_scale(changed: impl Fn() + Send + 'static) {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegCloseKey,
        RegNotifyChangeKeyValue, RegOpenKeyExW,
    };
    let spawned = std::thread::Builder::new()
        .name("text-scale".into())
        .spawn(move || {
            let path = wide(r"Software\Microsoft\Accessibility");
            let mut key: HKEY = std::ptr::null_mut();
            // SAFETY: a NUL-terminated key name and an out-pointer.
            let status =
                unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_NOTIFY, &mut key) };
            if status != 0 {
                return;
            }
            loop {
                // SAFETY: an open key; synchronous, so it returns on a change.
                let status = unsafe {
                    RegNotifyChangeKeyValue(
                        key,
                        0,
                        REG_NOTIFY_CHANGE_LAST_SET,
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if status != 0 {
                    break;
                }
                changed();
            }
            // SAFETY: the key opened above.
            unsafe { RegCloseKey(key) };
        });
    let _ = spawned;
}

#[cfg(not(windows))]
pub fn watch_text_scale(_: impl Fn() + Send + 'static) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_blink_follows_windows() {
        assert_eq!(caret_blink(530, true), 530);
        assert_eq!(caret_blink(1200, true), 1200);
        // "No blinking" (INFINITE) and animation effects off: steady.
        assert_eq!(caret_blink(u32::MAX, true), 0);
        assert_eq!(caret_blink(530, false), 0);
        // A failed read is the default; odd values are kept in range.
        assert_eq!(caret_blink(0, true), DEFAULT_BLINK_MS);
        assert_eq!(caret_blink(5, true), 100);
        assert_eq!(caret_blink(60_000, true), 5000);
    }

    #[test]
    fn menu_mode_forces_dark_or_light() {
        assert_eq!(menu_mode(true), 2);
        assert_eq!(menu_mode(false), 3);
    }

    #[test]
    fn accent_palette_is_reversed_into_dark3_to_light3() {
        // Light3, Light2, Light1, Accent, Dark1, Dark2, Dark3, unused.
        let mut bytes = Vec::new();
        for i in 0..8u8 {
            bytes.extend_from_slice(&[i, i + 10, i + 20, 0xff]);
        }
        let ramp = parse_accent_palette(&bytes).unwrap();
        assert_eq!(ramp[0], [6, 16, 26], "Dark3");
        assert_eq!(ramp[3], [3, 13, 23], "Accent");
        assert_eq!(ramp[6], [0, 10, 20], "Light3");
        assert_eq!(parse_accent_palette(&bytes[..31]), None);
        bytes.push(0);
        assert_eq!(parse_accent_palette(&bytes), None);
        assert_eq!(parse_accent_palette(&[]), None);
    }

    #[test]
    fn text_scale_grows_small_text_most() {
        for (size, want) in [
            (12.0, 26.1),
            (14.0, 27.5),
            (16.0, 29.1),
            (20.0, 32.3),
            (28.0, 39.2),
            (DISPLAY, 49.97),
        ] {
            let got = scaled(size, 2.25);
            assert!((got - want).abs() < 0.05, "{size}: {got}");
            assert_eq!(scaled(size, 1.0), size);
        }
        // Large text grows by its size alone once 18 − e·ln(s) is negative.
        assert_eq!(scaled(1000.0, 2.25), 1000.0);
    }

    #[test]
    fn colorref_is_bgr() {
        assert_eq!(colorref(0x00F0_E38E), [0x8e, 0xe3, 0xf0]);
        assert_eq!(colorref(0xFF12_3456), [0x56, 0x34, 0x12]);
        assert_eq!(to_colorref([0xf3, 0xf3, 0xf3]), 0x00F3_F3F3);
        assert_eq!(to_colorref([0x20, 0x30, 0x40]), 0x0040_3020);
        assert_eq!(colorref(to_colorref([1, 2, 3])), [1, 2, 3]);
    }

    #[test]
    fn color_lists_parse_exactly() {
        let green = "#0c1c14,#173525,#1e4732,#26593f,#2e6a4b,#357c58,#40956a";
        let ramp = parse_colors::<7>(green).unwrap();
        assert_eq!(ramp[2], [0x1e, 0x47, 0x32]);
        assert_eq!(ramp[5], [0x35, 0x7c, 0x58]);
        assert_eq!(
            parse_colors::<7>(" #0C1C14 , #173525,#1e4732,#26593f,#2e6a4b,#357c58,#40956a")
                .map(|r| r[0]),
            Some([0x0c, 0x1c, 0x14])
        );
        // Wrong count, bad hex.
        assert_eq!(parse_colors::<8>(green), None);
        assert_eq!(parse_colors::<6>(green), None);
        assert_eq!(parse_colors::<1>("#12345"), None);
        assert_eq!(parse_colors::<1>("123456"), None);
        assert_eq!(parse_colors::<1>("#12345g"), None);
        assert_eq!(parse_colors::<1>("#1234567"), None);
        assert_eq!(parse_colors::<1>(""), None);
        for (name, list) in CONTRAST_PRESETS {
            assert!(parse_colors::<8>(list).is_some(), "{name}");
        }
    }

    #[test]
    fn approximate_ramp_keeps_the_accent_in_the_middle() {
        for accent in [[0xe8, 0x11, 0x23], [0xff, 0xb9, 0x00], [0x88, 0x17, 0x98]] {
            let ramp = approx_ramp(accent);
            assert_eq!(ramp[3], accent);
            // Darker below, lighter above.
            for i in 0..6 {
                assert!(
                    luminance(ramp[i]) < luminance(ramp[i + 1]),
                    "{accent:?} {i}"
                );
            }
        }
    }

    #[test]
    fn contrast_window_tells_dark_from_light() {
        let window = |name: &str| {
            let (_, list) = CONTRAST_PRESETS.iter().find(|(n, _)| *n == name).unwrap();
            parse_colors::<8>(list).unwrap()[0]
        };
        for (name, dark) in [
            ("hc1", true),
            ("hc2", true),
            ("hcblack", true),
            ("hcwhite", false),
        ] {
            assert_eq!(luminance(window(name)) < 0.5, dark, "{name}");
        }
        assert!((luminance([255, 255, 255]) - 1.0).abs() < 1e-6);
        assert_eq!(luminance([0, 0, 0]), 0.0);
    }

    #[test]
    fn caption_takes_the_layer_unless_windows_colours_it() {
        let layer = [0xf3, 0xf3, 0xf3];
        let mut theme = SystemTheme::default();
        assert_eq!(caption_color(&theme, layer), Some(layer));
        theme.prevalence = true;
        assert_eq!(caption_color(&theme, layer), None);
        theme.prevalence = false;
        theme.contrast = parse_colors(CONTRAST_PRESETS[2].1);
        assert_eq!(caption_color(&theme, layer), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_build_is_read() {
        // Windows 10 1507 was build 10240; nothing older runs the app.
        assert!(windows_build() >= 10240, "{}", windows_build());
    }
}
