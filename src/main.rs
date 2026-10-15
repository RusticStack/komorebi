#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod crash;
mod format;
mod rows;
mod tr;

use komorebi::{db, mail, providers, role, secrets, trust};

slint::include_modules!();

use std::path::Path;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let db_path = db::default_path();
    // One app per cache, before the cache opens: two would drain one
    // outbox and double the connections.
    if !single_instance(&db_path) {
        return Ok(());
    }
    WINDOW_MARK.set(window_mark(&db_path));
    let crash_log = crash::log_path(&db_path);
    crash::install(crash_log.clone());
    // Two start-up jobs that need no window run beside `AppWindow::new`
    // (docs/PERF.md 6.1: overlap independent latencies): the cache open and
    // the icon read. Each is joined where it is used, and a lost thread
    // falls back to doing the work inline.
    let opening = {
        let path = db_path.clone();
        std::thread::Builder::new()
            .name("db-open".into())
            .spawn(move || db::Db::open_ui(&path))
            .ok()
    };
    #[cfg(windows)]
    let icon_thread = std::thread::Builder::new()
        .name("app-icon".into())
        .spawn(|| app_icon_pixels(APP_ICON_PX))
        .ok();
    let timing = komorebi::dev_var("KOMOREBI_TIMING").is_some();
    let mark = move |what: &str| {
        if timing {
            eprintln!(
                "[timing] {what}: {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    };

    // The software renderer opens a window in a few milliseconds, while a
    // GPU context costs 100+ ms at startup, so it is the only renderer
    // compiled in. SLINT_BACKEND still overrides the selection.
    if std::env::var_os("SLINT_BACKEND").is_none_or(|v| v.is_empty()) {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()?;
    }
    let ui = AppWindow::new()?;
    // The title bar's window side (src/chrome.rs): the window is subclassed
    // as it is created, so its first frame already has the app-drawn bar.
    #[cfg(windows)]
    komorebi::chrome::hook_creation();
    #[cfg(windows)]
    let icon = icon_thread
        .and_then(|h| h.join().ok())
        .unwrap_or_else(|| app_icon_pixels(APP_ICON_PX));
    #[cfg(not(windows))]
    let icon = None;
    wire_title_bar(&ui, icon);
    // The language of the first frame and of the start-up dialogs, from
    // Windows alone (the settings are not read yet); `app::install` then
    // applies the saved choice. Explicit, as Slint's own pick from the
    // system locale would give a Brazilian Windows the text of Portugal.
    let boot_lang = komorebi::i18n::startup_lang();
    komorebi::i18n::select(boot_lang);
    // Windows' short date for English dates with a year (one call).
    let locale_read = Instant::now();
    format::load_short_date();
    if timing {
        eprintln!(
            "[timing] locale short date read: {:.3} ms",
            locale_read.elapsed().as_secs_f64() * 1000.0
        );
    }
    mark("window created");
    let geometry = match app::install_with(&ui, db_path, &mark, opening) {
        Ok(geometry) => geometry,
        Err(e) => {
            // The release build has no console, so say it on screen.
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Komorebi")
                .set_description(tr::t!(invoke_start_failed, format!("{e:#}").into()))
                .show();
            return Err(e);
        }
    };
    mark("cache loaded");
    // Snapshots always start at the default place and size.
    if let Some(g) = geometry.filter(|_| komorebi::dev_var("KOMOREBI_SNAPSHOT").is_none()) {
        restore_window(&ui, g);
    }
    watch_window_events(&ui, timing.then_some(started));
    if crash::take_unseen(&crash_log) {
        app::notice(&tr::t!(invoke_crash_notice));
    }

    if let Some(path) = komorebi::dev_var("KOMOREBI_SNAPSHOT") {
        snapshot_and_quit(&ui, path.into());
    }

    // Started by Windows at sign-in (`--minimized`, "Start with Windows"):
    // the window stays hidden and the tray icon is the way in (it shows if
    // the tray can't be made). The loop runs until Quit or a real close,
    // so a window hidden to the tray keeps the app alive.
    if !komorebi::notify::launched_minimized(std::env::args().skip(1))
        || komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some()
    {
        ui.show()?;
    }
    slint::run_event_loop_until_quit()?;
    ui.hide().ok();
    if !app::remove_data_if_asked(ui) {
        app::finish_actions();
    }
    Ok(())
}

/// Window events the app follows through winit: files dragged from
/// Explorer (winit reports them, as an OLE drop target, one file at a
/// time), focus (for the taskbar flash), and the window's place while it is
/// neither maximized nor minimized (saved on close). With `started`
/// (`KOMOREBI_TIMING`), also the first frame: winit asks for the first
/// redraw, Slint paints it, and a zero timer then runs on the next pass of
/// the event loop, after the pixels are on screen. The first event (the
/// window exists by then) also sets the window's icons and title bar, and
/// starts listening for Windows appearance changes.
fn watch_window_events(ui: &AppWindow, mut started: Option<Instant>) {
    use slint::ComponentHandle;
    use slint::winit_030::winit::event::WindowEvent;
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    let mut dressed = false;
    let weak = ui.as_weak();
    // Snapshots draw the window active, whatever has the focus.
    let snapshot = komorebi::dev_var("KOMOREBI_SNAPSHOT").is_some();
    ui.window().on_winit_window_event(move |window, event| {
        if !std::mem::replace(&mut dressed, true) {
            window.with_winit_window(mark_winit_window);
            window.with_winit_window(set_icons);
            window.with_winit_window(watch_appearance);
            window.with_winit_window(install_chrome);
            // Slint pushed the bar before the window had its scale.
            push_regions();
        }
        match event {
            WindowEvent::RedrawRequested => {
                if let Some(started) = started.take() {
                    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                        let ms = started.elapsed().as_secs_f64() * 1000.0;
                        eprintln!("[timing] first frame drawn: {ms:.1} ms");
                    });
                }
            }
            WindowEvent::HoveredFile(_) => app::file_hover(true),
            WindowEvent::HoveredFileCancelled => app::file_hover(false),
            WindowEvent::DroppedFile(path) => app::file_dropped(path.clone()),
            WindowEvent::Focused(focused) => {
                app::window_focused(*focused);
                // The title bar dims while another window is in front.
                if let Some(ui) = weak.upgrade().filter(|_| !snapshot) {
                    ui.set_window_active(*focused);
                }
            }
            // The bar's rectangles in physical px follow the new scale.
            WindowEvent::ScaleFactorChanged { .. } => push_regions(),
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                push_regions();
                // The maximize button shows Restore while maximized.
                if let Some(ui) = weak.upgrade() {
                    let maximized = window.with_winit_window(|w| w.is_maximized());
                    ui.set_window_maximized(maximized.unwrap_or(false));
                }
                let placed = window.with_winit_window(|w| {
                    if w.is_maximized() || w.is_minimized() == Some(true) {
                        return None;
                    }
                    let (pos, size) = (w.outer_position().ok()?, w.inner_size());
                    Some(db::Geometry {
                        x: pos.x,
                        y: pos.y,
                        w: i32::try_from(size.width).ok()?,
                        h: i32::try_from(size.height).ok()?,
                        maximized: false,
                    })
                });
                if let Some(g) = placed.flatten().filter(|g| g.w > 0 && g.h > 0) {
                    app::window_placed(g);
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

/// A screen rectangle in physical pixels (`right` and `bottom` exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// Where a saved window goes, given the monitors' work areas (the primary
/// first). While enough of its title bar is on a monitor, it stays there,
/// shrunk to fit and pulled fully onto it; otherwise (its monitor was
/// removed or moved) it is centred on the primary monitor, shrunk to fit.
fn fit_to_monitors(g: db::Geometry, areas: &[Rect]) -> db::Geometry {
    // The part of the title bar that must show to count as visible.
    const GRIP_W: i64 = 120;
    const GRIP_H: i64 = 16;
    let Some(primary) = areas.first() else {
        return g;
    };
    let (x, y, w) = (i64::from(g.x), i64::from(g.y), i64::from(g.w));
    // How much of the title bar is on `a` (none below the grip).
    let grip = |a: &&Rect| -> i64 {
        let across = (x + w).min(i64::from(a.right)) - x.max(i64::from(a.left));
        let down = (y + GRIP_H).min(i64::from(a.bottom)) - y.max(i64::from(a.top));
        if across >= GRIP_W.min(w) && down >= GRIP_H {
            across
        } else {
            0
        }
    };
    let (area, keep) = match areas.iter().max_by_key(grip) {
        Some(a) if grip(&a) > 0 => (a, true),
        _ => (primary, false),
    };
    let w = g.w.min(area.right - area.left);
    let h = g.h.min(area.bottom - area.top);
    let (x, y) = if keep {
        (
            g.x.clamp(area.left, area.right - w),
            g.y.clamp(area.top, area.bottom - h),
        )
    } else {
        (
            area.left + (area.right - area.left - w) / 2,
            area.top + (area.bottom - area.top - h) / 2,
        )
    };
    db::Geometry { x, y, w, h, ..g }
}

/// Put the window back where it was last closed, before it first shows.
fn restore_window(ui: &AppWindow, saved: db::Geometry) {
    use slint::ComponentHandle;
    let g = fit_to_monitors(saved, &work_areas());
    let window = ui.window();
    window.set_position(slint::PhysicalPosition::new(g.x, g.y));
    window.set_size(slint::PhysicalSize::new(g.w as u32, g.h as u32));
    if g.maximized {
        window.set_maximized(true);
    }
    app::window_placed(db::Geometry {
        maximized: false,
        ..g
    });
}

/// The monitors' work areas (without the taskbar), the primary first.
#[cfg(windows)]
fn work_areas() -> Vec<Rect> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    use windows_sys::core::BOOL;
    // MONITORINFOF_PRIMARY
    const PRIMARY: u32 = 1;
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the `Vec` passed below, alive for the call.
        let areas = unsafe { &mut *(data as *mut Vec<Rect>) };
        // SAFETY: MONITORINFO is plain data; all zeroes is a valid value.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        // SAFETY: a monitor handle from the enumeration, and a sized struct.
        if unsafe { GetMonitorInfoW(m, &mut info) } != 0 {
            let r = info.rcWork;
            let rect = Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            };
            if info.dwFlags & PRIMARY != 0 {
                areas.insert(0, rect);
            } else {
                areas.push(rect);
            }
        }
        1
    }
    let mut areas: Vec<Rect> = Vec::new();
    // SAFETY: the callback only runs during the call, while `areas` lives.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut areas as *mut Vec<Rect> as LPARAM,
        );
    }
    areas
}

#[cfg(not(windows))]
fn work_areas() -> Vec<Rect> {
    Vec::new()
}

/// The window's handle (none before it exists).
#[cfg(windows)]
fn hwnd(ui: &AppWindow) -> Option<windows_sys::Win32::Foundation::HWND> {
    use slint::ComponentHandle;
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    ui.window()
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as _),
            _ => None,
        })
        .flatten()
}

/// Mark the window as this cache's (see `WINDOW_PROP`).
#[cfg(windows)]
fn mark_winit_window(w: &slint::winit_030::winit::window::Window) {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Some(RawWindowHandle::Win32(h)) = w.window_handle().ok().map(|h| h.as_raw()) {
        let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        mark_window(hwnd, WINDOW_MARK.get());
    }
}

#[cfg(not(windows))]
fn mark_winit_window(_: &slint::winit_030::winit::window::Window) {}

/// The window's icons, from the .exe's icon resource (assets/komorebi.rc):
/// the title bar's at 16 px and the taskbar's at 32 px, scaled to the
/// window's DPI. Slint leaves them alone while `Window.icon` is unset.
#[cfg(windows)]
fn set_icons(w: &slint::winit_030::winit::window::Window) {
    use slint::winit_030::winit::dpi::PhysicalSize;
    use slint::winit_030::winit::platform::windows::{IconExtWindows, WindowExtWindows};
    use slint::winit_030::winit::window::Icon;
    let icon = |logical: f64| {
        let px = (logical * w.scale_factor()).round() as u32;
        Icon::from_resource(1, Some(PhysicalSize::new(px, px))).ok()
    };
    w.set_window_icon(icon(16.0));
    w.set_taskbar_icon(icon(32.0));
}

#[cfg(not(windows))]
fn set_icons(_: &slint::winit_030::winit::window::Window) {}

/// How the title bar looks (docs/WINUI3.md "Title bar and frame"): dark or
/// light (never set dark under a contrast theme), its caption colour as a
/// COLORREF (None: Windows' own; only the system caption shows it), and
/// whether the app draws it (`custom`) or the system caption stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleBar {
    pub dark: bool,
    pub caption: Option<u32>,
    pub custom: bool,
}

thread_local! {
    /// The title bar asked for, set again whenever Windows resets it.
    static TITLE_BAR: std::cell::Cell<Option<TitleBar>> = const { std::cell::Cell::new(None) };
}

/// Give the window's title bar this look (now, or once the window exists).
/// A change of mode (high contrast turned on or off) switches the bar and
/// the frame in the same pass, so no double bar is ever drawn.
pub fn dress_title_bar(ui: &AppWindow, bar: TitleBar) {
    TITLE_BAR.set(Some(bar));
    ui.set_custom_title_bar(bar.custom);
    #[cfg(windows)]
    {
        let changed = komorebi::chrome::set_custom(bar.custom);
        if let Some(hwnd) = hwnd(ui) {
            set_title_bar(hwnd, bar);
            if changed {
                // SAFETY: this process's live window.
                unsafe { komorebi::chrome::frame_changed(hwnd) };
            }
        }
    }
}

thread_local! {
    /// The window, for the title bar's notifications (the window procedure
    /// never touches it: it posts to the event loop, which does).
    static MAIN_UI: std::cell::RefCell<Option<slint::Weak<AppWindow>>> =
        const { std::cell::RefCell::new(None) };
    /// The title bar's height and interactive rectangles (logical px) and
    /// whether a popup is open, as Slint last pushed them.
    static BAR_REGIONS: std::cell::RefCell<(f32, Vec<[f32; 4]>, bool)> =
        const { std::cell::RefCell::new((0.0, Vec::new(), false)) };
    /// A caption state change is on its way to the event loop.
    static CAPTION_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn main_ui() -> Option<AppWindow> {
    MAIN_UI.with(|u| u.borrow().as_ref().and_then(slint::Weak::upgrade))
}

/// The Slint side of the title bar: its rectangles go to the hit test, its
/// caption buttons' default actions (Narrator) send the system commands,
/// and the window procedure's hover and pressed states come back.
fn wire_title_bar(ui: &AppWindow, icon: Option<(u32, Vec<u8>)>) {
    use slint::{ComponentHandle, Model};
    MAIN_UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
    // A Windows that draws no top border over the custom frame (hand check).
    if komorebi::dev_var("KOMOREBI_TITLEBAR_BORDER").is_some() {
        ui.set_top_border(true);
    }
    if let Some((side, bgra)) = icon {
        ui.set_app_icon(icon_image(side, &bgra));
    }
    ui.on_title_bar_regions(|height, rects, popup| {
        let rects = rects
            .iter()
            .map(|r| [r.x, r.y, r.width, r.height])
            .collect();
        BAR_REGIONS.with(|b| *b.borrow_mut() = (height, rects, popup));
        push_regions();
    });
    let weak = ui.as_weak();
    ui.on_caption_command(move |index| {
        #[cfg(windows)]
        if let (Some(ui), Some(button)) =
            (weak.upgrade(), komorebi::chrome::Button::from_index(index))
            && let Some(hwnd) = hwnd(&ui)
        {
            // SAFETY: this process's live window.
            unsafe { komorebi::chrome::press(hwnd, button) };
        }
        #[cfg(not(windows))]
        let _ = (&weak, index);
    });
    #[cfg(windows)]
    komorebi::chrome::set_notify(caption_changed);
}

/// 32 px: sharp at the bar's 16 px up to 200 %.
const APP_ICON_PX: i32 = 32;

/// BGRA pixels of the icon as a Slint image.
fn icon_image(side: u32, bgra: &[u8]) -> slint::Image {
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(side, side);
    for (to, from) in pixels.make_mut_slice().iter_mut().zip(bgra.chunks_exact(4)) {
        *to = slint::Rgba8Pixel::new(from[2], from[1], from[0], from[3]);
    }
    slint::Image::from_rgba8(pixels)
}

/// The app icon at `px` square as BGRA bytes, read from the .exe's icon
/// resource (assets/komorebi.rc), so no image decoder is linked for the
/// title bar. Pure Win32 and no Slint objects, so it can run off the UI
/// thread.
#[cfg(windows)]
fn app_icon_pixels(px: i32) -> Option<(u32, Vec<u8>)> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
        ReleaseDC,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, ICONINFO, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW,
    };
    let side = u32::try_from(px).ok()?;
    let mut bgra = vec![0u8; (side * side * 4) as usize];
    // SAFETY: this module's own resource 1 (an icon), sized structs, a
    // buffer of the size asked for, and every handle released.
    let lines = unsafe {
        let icon = LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            1 as _,
            IMAGE_ICON,
            px,
            px,
            LR_DEFAULTCOLOR,
        );
        if icon.is_null() {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        let lines = if GetIconInfo(icon as _, &mut info) != 0 {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: px,
                biHeight: -px,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let dc = GetDC(std::ptr::null_mut());
            let lines = GetDIBits(
                dc,
                info.hbmColor,
                0,
                side,
                bgra.as_mut_ptr().cast(),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            ReleaseDC(std::ptr::null_mut(), dc);
            DeleteObject(info.hbmColor as _);
            DeleteObject(info.hbmMask as _);
            lines
        } else {
            0
        };
        DestroyIcon(icon as _);
        lines
    };
    if lines != px || bgra.chunks_exact(4).all(|p| p[3] == 0) {
        return None;
    }
    Some((side, bgra))
}

/// The bar as Slint drew it, in physical px at the window's scale now.
fn bar_in_pixels(scale: f32) -> komorebi::chrome::Bar {
    let (height, rects, popup) = BAR_REGIONS.with(|b| b.borrow().clone());
    komorebi::chrome::Bar {
        height: (height * scale).round() as i32,
        scale,
        passthrough: rects
            .iter()
            .map(|r| komorebi::chrome::PxRect::from_logical(r[0], r[1], r[2], r[3], scale))
            .collect(),
        popup,
    }
}

/// Hand the bar to the hit test (after Slint moved it, or the scale changed).
fn push_regions() {
    use slint::ComponentHandle;
    let Some(ui) = main_ui() else { return };
    let bar = bar_in_pixels(ui.window().scale_factor());
    #[cfg(windows)]
    komorebi::chrome::set_bar(bar);
    #[cfg(not(windows))]
    let _ = bar;
}

/// The window procedure saw the caption buttons' hover or pressed state
/// change: show it from the event loop, once per burst.
#[cfg(windows)]
fn caption_changed() {
    if CAPTION_PENDING.replace(true) {
        return;
    }
    let posted = slint::invoke_from_event_loop(|| {
        CAPTION_PENDING.set(false);
        let (hover, pressed) = komorebi::chrome::caption_state();
        if let Some(ui) = main_ui() {
            ui.set_caption_hover(hover.map_or(0, |b| b.index()));
            ui.set_caption_pressed(pressed.map_or(0, |b| b.index()));
        }
    });
    if posted.is_err() {
        CAPTION_PENDING.set(false);
    }
}

/// The window exists: the creation hook should have subclassed it (then no
/// system caption was ever drawn); if it missed, subclass it now, keeping
/// the client size.
#[cfg(windows)]
fn install_chrome(w: &slint::winit_030::winit::window::Window) {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    komorebi::chrome::unhook_creation();
    let Some(RawWindowHandle::Win32(h)) = w.window_handle().ok().map(|h| h.as_raw()) else {
        return;
    };
    let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let hooked = komorebi::chrome::installed() == hwnd;
    if !hooked {
        // SAFETY: this thread's live window.
        unsafe { komorebi::chrome::install_late(hwnd) };
    }
    if komorebi::dev_var("KOMOREBI_TIMING").is_some() {
        let when = if hooked {
            "at creation, before the first frame"
        } else {
            "after the first frame (a system caption may have shown)"
        };
        eprintln!("[timing] title bar subclass installed {when}");
    }
}

#[cfg(not(windows))]
fn install_chrome(_: &slint::winit_030::winit::window::Window) {}

#[cfg(windows)]
fn set_title_bar(hwnd: windows_sys::Win32::Foundation::HWND, bar: TitleBar) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_CAPTION_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmSetWindowAttribute,
    };
    let dark: windows_sys::core::BOOL = bar.dark.into();
    let caption: u32 = bar.caption.unwrap_or(DWMWA_COLOR_DEFAULT);
    // SAFETY: this process's live window, and values with their sizes. The
    // caption colour fails harmlessly before Windows 11.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as _,
            (&raw const dark).cast(),
            std::mem::size_of_val(&dark) as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR as _,
            (&raw const caption).cast(),
            std::mem::size_of_val(&caption) as u32,
        );
    }
}

/// Set the title bar asked for, and follow Windows appearance changes: the
/// window procedure is subclassed, as winit passes on only posted messages
/// and these are sent (docs/WINUI3.md "Live updates").
#[cfg(windows)]
fn watch_appearance(w: &slint::winit_030::winit::window::Window) {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    let Some(RawWindowHandle::Win32(h)) = w.window_handle().ok().map(|h| h.as_raw()) else {
        return;
    };
    let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    if let Some(bar) = TITLE_BAR.get() {
        set_title_bar(hwnd, bar);
    }
    // SAFETY: this thread's live window, and a procedure that removes
    // itself when the window is destroyed.
    unsafe { SetWindowSubclass(hwnd, Some(appearance_proc), 1, 0) };
}

#[cfg(not(windows))]
fn watch_appearance(_: &slint::winit_030::winit::window::Window) {}

/// After winit's own handling (which resets the title bar on a setting
/// change), on an appearance message: set the title bar again, and read the
/// settings again once per burst of messages, from the event loop. The app
/// state is never touched here: this can run inside a native dialog's
/// message loop.
#[cfg(windows)]
unsafe extern "system" fn appearance_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    id: usize,
    _: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_DWMCOLORIZATIONCOLORCHANGED, WM_NCDESTROY, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE,
        WM_THEMECHANGED,
    };
    thread_local! {
        static PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    // SAFETY: the arguments this subclass procedure was called with.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    match msg {
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
            if let Some(bar) = TITLE_BAR.get() {
                set_title_bar(hwnd, bar);
            }
            if !PENDING.replace(true) {
                let posted = slint::invoke_from_event_loop(|| {
                    PENDING.set(false);
                    app::system_theme_changed();
                });
                if posted.is_err() {
                    PENDING.set(false);
                }
            }
        }
        WM_NCDESTROY => {
            // SAFETY: the subclass installed in `watch_appearance`.
            unsafe { RemoveWindowSubclass(hwnd, Some(appearance_proc), id) };
        }
        _ => {}
    }
    result
}

/// A message window's frame (docs/WINUI3.md "Secondary windows"): the
/// system caption, dark or light and in the accent caption colour as the
/// main window's bar is asked to be (now, or once the window exists).
pub fn dress_popout(ui: &AppWindow) {
    #[cfg(windows)]
    if let (Some(hwnd), Some(bar)) = (hwnd(ui), TITLE_BAR.get()) {
        set_title_bar(hwnd, bar);
    }
    #[cfg(not(windows))]
    let _ = ui;
}

/// A message window's window events: on the first one its icons, frame
/// colours and the appearance listener (the main window's subclass, which
/// also sets its caption again when Windows resets it); `placed` hears its
/// size in logical px whenever it is resized. It is never marked as the
/// app's window (a second start finds the main one) and never gets the
/// app-drawn bar.
pub fn watch_popout(ui: &AppWindow, placed: impl Fn(u32, u32) + 'static) {
    use slint::ComponentHandle;
    use slint::winit_030::winit::event::WindowEvent;
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    let mut dressed = false;
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        if !std::mem::replace(&mut dressed, true) {
            window.with_winit_window(set_icons);
            window.with_winit_window(watch_appearance);
        }
        if let WindowEvent::Resized(_) = event
            && let Some(ui) = weak.upgrade()
        {
            let w = ui.window();
            let maximized = window
                .with_winit_window(|x| x.is_maximized() || x.is_minimized() == Some(true))
                .unwrap_or(false);
            if !maximized {
                let size = w.size().to_logical(w.scale_factor());
                placed(size.width.round() as u32, size.height.round() as u32);
            }
        }
        EventResult::Propagate
    });
}

/// Bring `ui` to the front (a reply from a message window opens in the
/// main one), restoring it if it was minimized.
pub fn bring_to_front(ui: &AppWindow) {
    use slint::ComponentHandle;
    ui.window().set_minimized(false);
    #[cfg(windows)]
    if let Some(hwnd) = hwnd(ui) {
        // SAFETY: this process's live window; Windows may decline (then
        // the taskbar button flashes instead).
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd) };
    }
}

/// Flash the taskbar button until the window comes to the front.
#[cfg(windows)]
pub fn flash_taskbar(ui: &AppWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx,
    };
    let Some(hwnd) = hwnd(ui) else { return };
    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        hwnd: hwnd as _,
        dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: a sized struct naming this process's live window.
    unsafe { FlashWindowEx(&info) };
}

#[cfg(not(windows))]
pub fn flash_taskbar(_: &AppWindow) {}

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
fn instance_name(db_path: &Path) -> String {
    format!("Local\\Komorebi-{:016x}", instance_hash(db_path))
}

/// The name of the window property that tells which cache a Komorebi window
/// shows. A window is found by this, never by its title: the title is
/// translated and carries the unread count.
const WINDOW_PROP: &str = "Komorebi.Instance";

/// The property value for a cache's window (never 0, which means "none").
fn window_mark(db_path: &Path) -> usize {
    (instance_hash(db_path) as usize) | 1
}

thread_local! {
    /// The mark of this process's window (see `window_mark`), put on the
    /// window as soon as it exists.
    static WINDOW_MARK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Mark the window as that of the cache `mark` names.
#[cfg(windows)]
fn mark_window(hwnd: windows_sys::Win32::Foundation::HWND, mark: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetPropW;
    let name: Vec<u16> = WINDOW_PROP.encode_utf16().chain([0]).collect();
    // SAFETY: a live window of this process, a NUL-terminated name, and a
    // plain number as the value (no handle to free when the window goes).
    unsafe { SetPropW(hwnd, name.as_ptr(), mark as _) };
}

/// Take the instance mutex for this cache and keep it for the life of the
/// process. False if another Komorebi holds it: its window is brought to
/// the front instead.
#[cfg(windows)]
fn single_instance(db_path: &Path) -> bool {
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
    // A window hidden to the tray is shown by the running copy itself.
    komorebi::win_shell::signal_existing_instance();
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
fn app_window(mark: usize) -> Option<windows_sys::Win32::Foundation::HWND> {
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
fn single_instance(_: &Path) -> bool {
    true
}

/// Development aid: render the window (optionally at `KOMOREBI_SIZE=WxH`),
/// save it as a BMP, and exit. Used to check layouts without screen capture.
fn snapshot_and_quit(ui: &AppWindow, path: std::path::PathBuf) {
    use slint::ComponentHandle;
    if let Some((w, h)) = std::env::var("KOMOREBI_SIZE")
        .ok()
        .and_then(|s| {
            s.split_once('x')
                .map(|(w, h)| (w.parse().ok(), h.parse().ok()))
        })
        .and_then(|(w, h)| Some((w?, h?)))
    {
        ui.window().set_size(slint::LogicalSize::new(w, h));
    }
    match std::env::var("KOMOREBI_THEME").as_deref() {
        Ok("light") => ui
            .global::<Palette>()
            .set_color_scheme(slint::language::ColorScheme::Light),
        Ok("dark") => ui
            .global::<Palette>()
            .set_color_scheme(slint::language::ColorScheme::Dark),
        _ => {}
    }
    // Select the n-th message (or rows a to b, as Shift+Down would: the
    // "N selected" panel) and optionally trigger an action, so the reader,
    // composer and settings can be checked too.
    let select = std::env::var("KOMOREBI_SNAPSHOT_SELECT").unwrap_or_default();
    let (select, extend) = match select.split_once('-') {
        Some((a, b)) => (a.parse::<i32>().ok(), b.parse::<i32>().ok()),
        None => (select.parse::<i32>().ok(), None),
    };
    let action = std::env::var("KOMOREBI_SNAPSHOT_ACTION").unwrap_or_default();
    // Type this into the search box first (with the action `search-all`,
    // in the "All folders" scope).
    let search = std::env::var("KOMOREBI_SNAPSHOT_SEARCH").unwrap_or_default();
    // Then show the keyboard focus (see `snapshot_focus`).
    let focus = std::env::var("KOMOREBI_SNAPSHOT_FOCUS").unwrap_or_default();
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(600), move || {
        let Some(ui) = weak.upgrade() else { return };
        if !search.is_empty() {
            ui.set_search_text(search.as_str().into());
            ui.invoke_search_edited(search.as_str().into());
            if action == "search-all" {
                ui.invoke_set_search_all(true);
            }
        }
        for _ in 0..=select.unwrap_or(-1) {
            ui.invoke_select_offset(1);
        }
        // In one pane the arrows only move the cursor: open it, as Enter
        // would (a no-op where the row is already open).
        if select.is_some() {
            ui.invoke_open_current();
        }
        for _ in select.unwrap_or(0)..extend.unwrap_or(0) {
            ui.invoke_select_extend(1);
        }
        match action.as_str() {
            "reply" => ui.invoke_reply(),
            "compose" | "compose-suggest" => ui.invoke_compose(),
            // A new message with three attachments, the counter at 85%.
            "compose-attach" => {
                ui.invoke_compose();
                app::sample_attachments();
            }
            // A new message whose To address is wrong, then Send: the
            // inline error under To, which has the focus (nothing is sent).
            "compose-invalid" => {
                ui.invoke_compose();
                let to = "Ana Silva <ana@>";
                ui.set_c_to(to.into());
                ui.invoke_c_edited(ComposeField::To, to.into());
                ui.invoke_send();
            }
            // A new message with no recipient, then Send: "Add a recipient
            // to send." under To.
            "compose-no-to" => {
                ui.invoke_compose();
                ui.invoke_send();
            }
            // A formatted message (the painted editor) with bold, italic,
            // underline, a colour, a link, both lists and a quote, the
            // focus in its body; `compose-table` adds a 3 x 3 table,
            // `compose-preview` turns Preview on.
            "compose-format"
            | "compose-rich"
            | "compose-rich-compact"
            | "compose-preview"
            | "compose-table" => {
                ui.invoke_compose();
                app::sample_rich(action == "compose-table");
                if action == "compose-preview" {
                    ui.set_c_preview(true);
                }
            }
            // The same message switched to plain text (`compose-plain`), or
            // the question before it (`compose-plain-ask`).
            "compose-plain" | "compose-plain-ask" => {
                ui.invoke_compose();
                app::sample_rich(false);
                if action == "compose-plain" {
                    ui.global::<Rich>().invoke_set_rich(false);
                } else {
                    ui.global::<Rich>().set_plain_ask(true);
                }
            }
            // Settings > Signatures with two made-up signatures (written to
            // the throwaway cache, offline only), and the first one's page.
            "settings-signatures" => app::sample_signatures(false),
            // A formatted message from an account whose default signature
            // has a logo (made up, offline only).
            "compose-signature" => {
                app::sample_signatures(false);
                ui.set_show_settings(false);
                ui.invoke_compose();
                app::sample_rich(false);
            }
            "signature-edit" => app::sample_signatures(true),
            // A file dragged over the window, with a composer open
            // (`drop-hover-compose`) or not: the drop overlay.
            "drop-hover" => app::sample_file_hover(),
            "drop-hover-compose" => {
                ui.invoke_compose();
                app::sample_file_hover();
            }
            // The Outbox with a made-up message whose send failed for good
            // (`outbox:permanent`: 550 5.1.1, Edit leads) or for now
            // (`outbox:transient`: 421, Retry leads), written to the
            // throwaway cache; offline only. Nothing is sent.
            "outbox:permanent" | "outbox:transient"
                if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() =>
            {
                app::sample_outbox(&action["outbox:".len()..]);
            }
            // A reply to the selected message with a line typed, closed:
            // the reader's "You started a reply" bar (offline only, as the
            // draft is saved in the throwaway cache).
            "draft-reply" if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() => {
                ui.invoke_reply();
                ui.invoke_c_edited(ComposeField::Body, "Thanks, see you then.".into());
                ui.invoke_c_close();
            }
            // The Drafts view (always in the pane).
            "drafts" => ui.invoke_nav_clicked(NavItem {
                kind: 3,
                ..Default::default()
            }),
            // The newest saved draft in the composer, then Discard: shows
            // the prompt (nothing is deleted until it is confirmed).
            "discard" => {
                app::open_newest_draft();
                ui.invoke_discard();
            }
            // The newest saved draft in the composer, then Send: shows the
            // send guards' bar for a draft without a subject (nothing is
            // sent until "Send anyway").
            "guard" => {
                app::open_newest_draft();
                ui.invoke_send();
            }
            // The Outbox view, with its newest message selected.
            "outbox" => {
                use slint::Model;
                ui.invoke_nav_clicked(NavItem {
                    kind: 4,
                    ..Default::default()
                });
                if let Some(row) = ui.get_rows().row_data(0) {
                    ui.invoke_message_selected(row.id);
                }
            }
            // The narrow window's folder pane, opened over the content.
            "pane" => ui.set_pane_open(true),
            // The folder pane folded to the rail at any width (Ctrl+Shift+E).
            "nav:collapsed" => ui.invoke_toggle_pane(),
            // The list's first n messages in windows of their own (the
            // snapshot is of the main window; for measuring memory).
            a if a.starts_with("windows:") => {
                let n = a["windows:".len()..].parse::<usize>().unwrap_or(1);
                app::open_windows(n);
            }
            // Tabs: the newest messages in tabs of their own (up to N), the
            // first one shown; the strip over the content.
            a if a.starts_with("tabs") => {
                use slint::Model;
                let n = a
                    .strip_prefix("tabs:")
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(3);
                let rows = ui.get_rows();
                let ids: Vec<i32> = (0..rows.row_count().min(n))
                    .filter_map(|i| rows.row_data(i))
                    .map(|r| r.id)
                    .collect();
                for id in ids.iter().rev() {
                    ui.invoke_open_tab(*id, true);
                }
                if !ids.is_empty() {
                    ui.invoke_tab_activate(1);
                }
            }
            // The last folder opened first, then the pane: it opens
            // scrolled to that folder.
            "pane-last" => {
                use slint::Model;
                let items = ui.get_nav_items();
                if let Some(item) = (0..items.row_count())
                    .rev()
                    .filter_map(|i| items.row_data(i))
                    .find(|r| r.kind == 2)
                {
                    ui.invoke_nav_clicked(item);
                }
                ui.set_pane_open(true);
            }
            "settings" => ui.set_show_settings(true),
            // Settings on one section: `settings:general`, `:privacy`,
            // `:about` (or `:accounts`).
            s if s.starts_with("settings:") => {
                let section = ["accounts", "signatures", "general", "privacy", "about"]
                    .iter()
                    .position(|n| s.ends_with(n))
                    .unwrap_or(0);
                ui.set_settings_section(section as i32);
                ui.set_show_settings(true);
            }
            // Settings > Privacy with the Remove all data prompt (nothing
            // is removed until it is confirmed).
            "remove-data" => {
                ui.set_settings_section(3);
                ui.set_show_settings(true);
                ui.invoke_remove_all_data();
            }
            // The onboarding flow (made up and offline; nothing is looked up,
            // checked or sent): `ob-email`, `ob-email-error`, `ob-finding`,
            // `ob-found-preset`, `ob-found`, `ob-notfound`, `ob-password`,
            // `ob-wait`, `ob-wait-slow`, `ob-error-rejected`,
            // `ob-error-unreachable`, `ob-error-cert`, `ob-manual`, and
            // (offline only, as they add a made-up account to the throwaway
            // cache) `ob-personalise`, `ob-done`, `ob-done-progress`.
            // `ob-embedded` opens it from Settings > Add account (a cache with
            // an account). The old add-form actions are aliases.
            "ob-embedded" => {
                ui.set_settings_section(0);
                ui.set_show_settings(true);
                ui.invoke_open_onboarding();
            }
            s if s.starts_with("ob-")
                || matches!(s, "settings-add" | "signin-wait" | "account-test-failed")
                || s.starts_with("account-find") =>
            {
                let state = match s {
                    "settings-add" => "email",
                    "signin-wait" => "wait",
                    "account-test-failed" => "error-rejected",
                    "account-find" | "account-find-srv" => "found",
                    "account-find-none" | "account-find-foreign" => "notfound",
                    "account-find-rest" => "manual",
                    s => s.trim_start_matches("ob-"),
                };
                app::sample_onboard(state);
            }
            // A reply to the selected message, sent: the Undo offer (the
            // send waits out its undo window, so nothing goes out). The
            // row stays queued in the cache and the next start sends it,
            // so only under KOMOREBI_OFFLINE, on a copy.
            "undo-send" if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() => {
                ui.invoke_reply();
                ui.invoke_c_send_anyway();
            }
            // Settings with the first account's edit form open.
            "settings-edit" => {
                use slint::Model;
                if let Some(a) = ui.get_accounts().row_data(0) {
                    ui.invoke_edit_account(a.id);
                }
            }
            // Settings with the first account's edit form after the secure
            // connection failed on an untrusted certificate (made-up; nothing
            // is sent): the sentence and Review certificate. `account-cert-prompt`
            // opens the Trust this certificate? prompt over it (and
            // `account-cert-again` the one that replaces a forgotten
            // certificate); `account-cert-pinned` the form with a certificate
            // saved for each server.
            "account-cert-failed"
            | "account-cert-prompt"
            | "account-cert-again"
            | "account-cert-pinned" => {
                use slint::Model;
                if let Some(a) = ui.get_accounts().row_data(0) {
                    ui.invoke_edit_account(a.id);
                }
                let action = action.to_string();
                // After the page has built itself (which loads the form).
                slint::Timer::single_shot(std::time::Duration::from_millis(50), move || {
                    app::sample_cert(action.trim_start_matches("account-cert-"))
                });
            }
            // The first account's page scrolled to its Folders (the roles
            // Sent, Drafts, Trash, Archive and Junk, each header above its
            // box).
            "settings-edit-folders" => {
                use slint::Model;
                if let Some(a) = ui.get_accounts().row_data(0) {
                    ui.invoke_edit_account(a.id);
                    ui.set_a_scroll_folders(true);
                }
            }
            // The sender notice's "Why?" dialog over the selected message.
            "notice-why" => ui.set_why_show(true),
            // The first account as when its saved certificate changed: the
            // status line with Review certificate (made up; nothing is
            // sent), and with `cert-changed-settings` its card in Settings.
            "cert-changed" => app::sample_cert_changed(),
            "cert-changed-settings" => {
                app::sample_cert_changed();
                ui.set_show_settings(true);
            }
            // The same with made-up sign-in names (an IMAP username and a
            // separate SMTP one), as for a self-hosted server.
            "settings-edit-logins" => {
                use slint::Model;
                if let Some(a) = ui.get_accounts().row_data(0) {
                    ui.invoke_edit_account(a.id);
                    ui.set_a_edit(AccountForm {
                        imap_user: "alice".into(),
                        smtp_user: "relay@mail.example.net".into(),
                        smtp_own_login: true,
                        ..ui.get_a_edit()
                    });
                }
            }
            // The first account's status as when its server can't be
            // reached (a made-up Offline problem): the status line, and
            // with `offline-settings` its line in Settings.
            "offline" => app::sample_offline(),
            "offline-settings" => {
                app::sample_offline();
                ui.set_show_settings(true);
            }
            // Settings with the first account's mail downloading (made-up
            // numbers; the bar shows after its 1 s delay, so add a
            // `KOMOREBI_SNAPSHOT_HOVER` to take the snapshot later).
            "settings-download" => {
                app::sample_progress();
                ui.set_show_settings(true);
            }
            // A made-up account added to the throwaway cache, as just added:
            // its Inbox in its first download (1,234 of 5,000). Offline
            // only, as it writes the cache.
            "fresh-inbox" if komorebi::dev_var("KOMOREBI_OFFLINE").is_some() => {
                app::sample_fresh_inbox();
            }
            "plain" => ui.set_r_plain(true),
            // Dark theme: "Show original colours" (no dimming) on the
            // message shown.
            "original" => ui.invoke_r_show_original(true),
            // Show original on the selected message: its header fields and
            // source.
            "source" => ui.invoke_show_original(),
            // Accept on the selected invitation: the prompt naming the one
            // recipient (nothing is sent until it is confirmed).
            "rsvp" => ui.invoke_invite(1),
            // Archive or Report junk on what is selected: the Undo offer
            // (the server job is held, so the cache is not changed), or
            // the line saying there is no Junk folder.
            "archive" => ui.invoke_archive(),
            // "Select messages" on: the rows' check boxes.
            "select-mode" => ui.invoke_list_command(2),
            // Delete asked about as in Trash (nothing is deleted unless it
            // is confirmed, which a snapshot never does).
            "delete-confirm" => app::sample_delete_confirm(),
            // The open message as if not downloaded with its server out of
            // reach (in memory only): after 2 s, the sentence and Retry (the
            // snapshot waits for them).
            "not-downloaded" => app::sample_not_downloaded(),
            "junk" => ui.invoke_junk(true),
            // The Unread filter; or Mark all as read first (this changes
            // the cache: use a copy), leaving its empty state.
            // A menu's entries (`menu:list`, `menu:pane`, `menu:rail`)
            // printed to stderr, then a normal exit: no native popup is
            // ever opened in a snapshot run (it would hang the process).
            "menu:row" | "menu:more" | "menu:move" => {}
            s if s.starts_with("menu:") => {
                app::dump_menu(&s["menu:".len()..]);
                slint::quit_event_loop().ok();
            }
            // The teaching tips (`tips`: the series of this layout from its
            // first tip; `tips:folders`, `tips:search`, `tips:help` at that
            // tip, its place counted as in the series), the What's new bar,
            // the Keyboard shortcuts dialog. Nothing is saved as seen.
            s if (s == "tips" || s.starts_with("tips:"))
                && !app::sample_tips(s.strip_prefix("tips:")) =>
            {
                eprintln!("tip not applicable at this width");
                slint::quit_event_loop().ok();
            }
            "whats-new" if !app::sample_whats_new() => {
                eprintln!("What's new is not due on this cache");
            }
            "shortcuts" => ui.invoke_open_shortcuts(),
            "unread" => ui.invoke_set_filter(1),
            // The window as when another one is in front: the title bar dims.
            "inactive" => ui.set_window_active(false),
            "read-all" => {
                ui.invoke_mark_all_read();
                ui.invoke_set_filter(1);
            }
            _ => {}
        }
        // Once the page is up: select all of it and copy it, focus its
        // first link (Tab), scroll it sideways to its right edge, or click
        // its first web link (the confirm prompt shows; nothing is opened).
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(600), move || {
            let Some(ui) = weak.upgrade() else { return };
            match action.as_str() {
                // The Move flyout (Ctrl+Shift+V) on what is selected, once
                // the reader shows it: its filter, recent folders and the
                // tree (a popup drawn in the window, not a native menu).
                "move" => ui.invoke_open_move(),
                // The row and More menus (the page rendered by now).
                "menu:row" | "menu:more" | "menu:move" => {
                    app::dump_menu(&action["menu:".len()..]);
                    slint::quit_event_loop().ok();
                }
                "select-all" => {
                    ui.invoke_r_select_all();
                    ui.invoke_r_copy();
                }
                "link-focus" => {
                    ui.invoke_r_focus_link(false);
                }
                // The composer's link field, with an address typed.
                "compose-format" => {
                    use slint::platform::{Key, WindowEvent};
                    let send = |text: slint::SharedString| {
                        let w = ui.window();
                        w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                        w.dispatch_event(WindowEvent::KeyReleased { text });
                    };
                    let w = ui.window();
                    w.dispatch_event(WindowEvent::KeyPressed {
                        text: Key::Control.into(),
                    });
                    send("k".into());
                    w.dispatch_event(WindowEvent::KeyReleased {
                        text: Key::Control.into(),
                    });
                    send("example.org".into());
                }
                // A name prefix typed into To: the recipient suggestions
                // from the address index show under it.
                "compose-suggest" => {
                    use slint::platform::WindowEvent;
                    for text in ["a", "l"] {
                        let w = ui.window();
                        w.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
                        w.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
                    }
                }
                // Scroll a page wider than the column to its right edge.
                "scroll-right" => {
                    ui.set_r_page_x((ui.get_r_body_width() - ui.get_r_page_width()).min(0.0));
                }
                // Load the remote images (refused under KOMOREBI_OFFLINE),
                // once or always for the sender (then show Settings).
                "remote-once" => ui.invoke_r_load_remote(false),
                "remote-always" => {
                    ui.invoke_r_load_remote(true);
                    ui.set_show_settings(true);
                }
                // The same, staying in the reader: the bar with Undo and
                // Privacy settings.
                "remote-always-bar" => ui.invoke_r_load_remote(true),
                "link-confirm" => {
                    use slint::Model;
                    if let Some(i) = ui
                        .get_r_links()
                        .iter()
                        .position(|l| l.href.starts_with("http"))
                    {
                        ui.invoke_r_activate(i as i32);
                    }
                }
                _ => {}
            }
            if !focus.is_empty() {
                snapshot_focus(&ui, &focus);
            }
        });
    });
    // Scroll the reader down this many logical px once the message is up.
    if let Some(y) = std::env::var("KOMOREBI_SNAPSHOT_SCROLL")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    {
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(1000), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_r_content_y(-y);
            }
        });
    }
    // Then open the next n messages, 250 ms apart (each one renders before
    // the next replaces it), to measure memory after a reading session.
    let walk = std::env::var("KOMOREBI_SNAPSHOT_WALK")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map_or(0, |n| n.min(1000));
    for i in 1..=walk {
        let weak = ui.as_weak();
        let at = std::time::Duration::from_millis(600 + 250 * i);
        slint::Timer::single_shot(at, move || {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_select_offset(1);
            }
        });
    }
    // Last, point at x,y (logical px), to show a hover; `x,y:press` also
    // holds the left button down there, `x,y:click` clicks there. The
    // snapshot follows 700 ms later, after a tooltip's 500 ms delay.
    let hover = komorebi::dev_var("KOMOREBI_SNAPSHOT_HOVER").and_then(|v| v.into_string().ok());
    // `not-downloaded` says so after 2 s: the snapshot waits for it.
    let late = if std::env::var("KOMOREBI_SNAPSHOT_ACTION").as_deref() == Ok("not-downloaded") {
        1500
    } else {
        0
    };
    let at = std::time::Duration::from_millis(1500 + 250 * walk + late);
    if let Some(hover) = &hover {
        let (weak, hover) = (ui.as_weak(), hover.clone());
        slint::Timer::single_shot(at, move || {
            if let Some(ui) = weak.upgrade() {
                snapshot_hover(&ui, &hover);
            }
        });
    }
    let at = at + std::time::Duration::from_millis(if hover.is_some() { 700 } else { 0 });
    let weak = ui.as_weak();
    slint::Timer::single_shot(at, move || {
        let Some(ui) = weak.upgrade() else { return };
        dump_title_bar(&ui);
        let t = Instant::now();
        let snapshot = ui.window().take_snapshot();
        if komorebi::dev_var("KOMOREBI_TIMING").is_some() {
            eprintln!(
                "[timing] full-frame snapshot: {:.1} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Ok(buf) = &snapshot {
            write_bmp(buf, &path);
        }
        // CI's boot check: the list and the reader must show something.
        if komorebi::dev_var("KOMOREBI_SNAPSHOT_CHECK").is_some() {
            let blank = match &snapshot {
                Ok(buf) => blank_panes(buf, ui.window().scale_factor()),
                Err(_) => vec!["window"],
            };
            if !blank.is_empty() {
                eprintln!("snapshot check failed: blank {}", blank.join(", "));
                std::process::exit(1);
            }
            eprintln!("snapshot check passed");
        }
        slint::quit_event_loop().ok();
    });
}

/// The panes of a snapshot (taken at `scale` physical pixels per logical
/// one) that show nothing: "list" or "reader", the inner parts of the
/// panes at the window's widths (see app.slint) with less than 1% of their
/// pixels unlike their background.
fn blank_panes(buf: &slint::SharedPixelBuffer<slint::Rgba8Pixel>, scale: f32) -> Vec<&'static str> {
    let px = |logical: f32| (logical * scale).round() as u32;
    let (w, h) = (buf.width(), buf.height());
    let compact = (w as f32) < 1040.0 * scale;
    let (nav, list) = if compact {
        // The rail: 64 px, wider only for a long word of its names.
        (64.0, 300.0)
    } else {
        (248.0, 380.0)
    };
    let rows = h / 5..h * 9 / 10;
    let panes = [
        ("list", px(nav + 8.0)..px(nav + list - 8.0)),
        ("reader", px(nav + list + 16.0)..w.saturating_sub(px(16.0))),
    ];
    panes
        .into_iter()
        .filter(|(_, cols)| ink(buf, cols.clone(), rows.clone()) < 0.01)
        .map(|(name, _)| name)
        .collect()
}

/// The share of the pixels in a region (clipped to the buffer) that differ
/// from its most common colour; 0 for an empty region.
fn ink(
    buf: &slint::SharedPixelBuffer<slint::Rgba8Pixel>,
    cols: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> f64 {
    let (w, h) = (buf.width(), buf.height());
    let clip = |r: std::ops::Range<u32>, max: u32| r.start.min(r.end).min(max)..r.end.min(max);
    let (cols, rows) = (clip(cols, w), clip(rows, h));
    let pixels = buf.as_slice();
    let region = || {
        rows.clone().flat_map(|y| {
            let line = &pixels[(y * w) as usize..][..w as usize];
            line[cols.start as usize..cols.end as usize].iter()
        })
    };
    let mut counts = std::collections::HashMap::new();
    for p in region() {
        *counts.entry((p.r, p.g, p.b)).or_insert(0usize) += 1;
    }
    let total: usize = counts.values().sum();
    let background = counts.values().copied().max().unwrap_or(0);
    if total == 0 {
        return 0.0;
    }
    (total - background) as f64 / total as f64
}

/// Move the keyboard focus with real key presses, from the list:
/// `toolbar` (Tab: the reader's first button) or `toolbar:n` (then Right n
/// times), `nav` or `nav:n` (Shift+F6 to the folders, then Down n times;
/// `nav:n:keys` then presses each of `keys`: `l` Left, `r` Right, `u` Up,
/// `d` Down, `e` Enter), `list` (Tab and Shift+Tab: the list's current row), `tab:n` (n Tabs)
/// or `press:keys` (those letters, plus `n` PageDown, `p` PageUp, `h` Home,
/// `z` End, `f` F6, `t` Tab, pressed where the focus already is: a dialog's
/// keys, or a Settings page's Tab stops, which `tab:n` would leave).
fn snapshot_focus(ui: &AppWindow, focus: &str) {
    use slint::ComponentHandle;
    use slint::platform::{Key, WindowEvent};
    let press = |shift: bool, key: Key| {
        let w = ui.window();
        if shift {
            w.dispatch_event(WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            });
        }
        w.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        w.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        if shift {
            w.dispatch_event(WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            });
        }
    };
    let (what, n) = match focus.split_once(':') {
        Some((what, n)) => (what, n),
        None => (focus, ""),
    };
    let (n, keys) = n.split_once(':').unwrap_or((n, ""));
    let n: u32 = n.parse().unwrap_or(0);
    let letter = |k: char| match k {
        'l' => Some(Key::LeftArrow),
        'r' => Some(Key::RightArrow),
        'u' => Some(Key::UpArrow),
        'd' => Some(Key::DownArrow),
        'e' => Some(Key::Return),
        'n' => Some(Key::PageDown),
        'p' => Some(Key::PageUp),
        'h' => Some(Key::Home),
        'z' => Some(Key::End),
        'f' => Some(Key::F6),
        // Tab where the focus is (Settings keeps its page, unlike `tab:n`,
        // which starts from the list).
        't' => Some(Key::Tab),
        _ => None,
    };
    if what == "press" {
        // `press:keys`: the part after the first colon, letters only.
        for key in focus
            .split_once(':')
            .map_or("", |(_, k)| k)
            .chars()
            .filter_map(letter)
        {
            press(false, key);
        }
        return;
    }
    ui.invoke_focus_list();
    match what {
        "toolbar" => {
            press(false, Key::Tab);
            (0..n).for_each(|_| press(false, Key::RightArrow));
        }
        "nav" => {
            press(true, Key::F6);
            (0..n).for_each(|_| press(false, Key::DownArrow));
            for key in keys.chars().filter_map(letter) {
                press(false, key);
            }
        }
        "list" => {
            press(false, Key::Tab);
            press(true, Key::Tab);
        }
        "tab" => (0..n).for_each(|_| press(false, Key::Tab)),
        _ => {}
    }
}

/// Move the pointer to `x,y` (logical px), and with `x,y:press` press the
/// left button there (`x,y:click` releases it again).
fn snapshot_hover(ui: &AppWindow, hover: &str) {
    use slint::ComponentHandle;
    use slint::platform::{PointerEventButton, WindowEvent};
    let (at, how) = hover.split_once(':').unwrap_or((hover, ""));
    // The caption buttons get the pointer as Windows reports it, not
    // through Slint: `caption-min`, `caption-max`, `caption-close`, with
    // `:press` held down.
    if let Some(which) = at.strip_prefix("caption-") {
        let index = match which {
            "min" => 1,
            "max" => 2,
            "close" => 3,
            _ => 0,
        };
        ui.set_caption_hover(index);
        if how == "press" {
            ui.set_caption_pressed(index);
        }
        return;
    }
    let Some((x, y)) = at
        .split_once(',')
        .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
    else {
        return;
    };
    let position = slint::LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    // Moved again a moment later, as Windows' next mouse move would: what
    // the first one made appear under the pointer (a row's quick actions)
    // gets it too. Then the press or click.
    let (weak, how) = (ui.as_weak(), how.to_string());
    slint::Timer::single_shot(std::time::Duration::from_millis(50), move || {
        let Some(ui) = weak.upgrade() else { return };
        let w = ui.window();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        if how == "press" || how == "click" {
            w.dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        }
        if how == "click" {
            w.dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        }
    });
}

/// Print the title bar as the hit test has it (snapshots): its mode, the
/// height drawn and the height pushed (which must match), and the
/// passthrough rectangles, in physical px.
fn dump_title_bar(ui: &AppWindow) {
    use slint::ComponentHandle;
    let scale = ui.window().scale_factor();
    let drawn = (ui.get_bar_height() * scale).round() as i32;
    #[cfg(windows)]
    let pushed = komorebi::chrome::bar();
    #[cfg(not(windows))]
    let pushed = bar_in_pixels(scale);
    let rects: Vec<String> = pushed
        .passthrough
        .iter()
        .map(|r| format!("{}x{}@{},{}", r.w, r.h, r.x, r.y))
        .collect();
    eprintln!(
        "[titlebar] mode={} scale={scale} drawn={drawn} pushed={} {} passthrough=[{}] popup={}",
        if ui.get_custom_title_bar() {
            "custom"
        } else {
            "system"
        },
        pushed.height,
        if pushed.height == drawn {
            "match"
        } else {
            "MISMATCH"
        },
        rects.join(" "),
        pushed.popup,
    );
    // The layout breakpoint in effect (docs/WINUI3.md "Layout
    // breakpoints"), in logical px.
    let size = ui.window().size().to_logical(scale);
    eprintln!(
        "[layout] width={} mode={} list={} reader={}",
        size.width,
        if ui.get_one_pane() {
            "one-pane"
        } else if ui.get_compact() {
            "compact"
        } else {
            "wide"
        },
        ui.get_show_list(),
        ui.get_show_reader(),
    );
}

/// Render the window and save it as a BMP (development aid).
#[cfg(debug_assertions)]
fn save_bmp(ui: &AppWindow, path: &std::path::Path) {
    use slint::ComponentHandle;
    if let Ok(buf) = ui.window().take_snapshot() {
        write_bmp(&buf, path);
    }
}

fn write_bmp(buf: &slint::SharedPixelBuffer<slint::Rgba8Pixel>, path: &std::path::Path) {
    {
        {
            let (w, h) = (buf.width(), buf.height());
            let mut bmp = Vec::with_capacity(54 + (w * h * 4) as usize);
            let size = 54 + w * h * 4;
            bmp.extend_from_slice(b"BM");
            bmp.extend_from_slice(&size.to_le_bytes());
            bmp.extend_from_slice(&[0; 4]);
            bmp.extend_from_slice(&54u32.to_le_bytes());
            bmp.extend_from_slice(&40u32.to_le_bytes());
            bmp.extend_from_slice(&(w as i32).to_le_bytes());
            bmp.extend_from_slice(&(-(h as i32)).to_le_bytes());
            bmp.extend_from_slice(&1u16.to_le_bytes());
            bmp.extend_from_slice(&32u16.to_le_bytes());
            bmp.extend_from_slice(&[0; 24]);
            for p in buf.as_slice() {
                bmp.extend_from_slice(&[p.b, p.g, p.r, 255]);
            }
            std::fs::write(path, bmp).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_counts_pixels_unlike_the_background() {
        let white = slint::Rgba8Pixel::new(255, 255, 255, 255);
        let black = slint::Rgba8Pixel::new(0, 0, 0, 255);
        let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(100, 50);
        buf.make_mut_slice().fill(white);
        assert_eq!(ink(&buf, 0..100, 0..50), 0.0);
        // A 10×5 "word" at (20, 10).
        for y in 10..15 {
            buf.make_mut_slice()[y * 100 + 20..y * 100 + 30].fill(black);
        }
        assert_eq!(ink(&buf, 0..100, 0..50), 50.0 / 5000.0);
        assert_eq!(ink(&buf, 20..30, 10..15), 0.0, "all black: no background");
        assert_eq!(ink(&buf, 0..40, 0..20), 50.0 / 800.0);
        assert_eq!(ink(&buf, 50..100, 0..50), 0.0);
        // Clipped to the buffer; empty regions count as blank.
        assert_eq!(ink(&buf, 90..500, 40..500), 0.0);
        assert_eq!(ink(&buf, 200..300, 0..50), 0.0);
        let reversed = std::ops::Range { start: 30, end: 20 };
        assert_eq!(ink(&buf, reversed, 0..50), 0.0);
    }

    #[test]
    fn blank_panes_names_the_empty_ones() {
        let white = slint::Rgba8Pixel::new(250, 250, 250, 255);
        let grey = slint::Rgba8Pixel::new(40, 40, 40, 255);
        // Ink in the middle of each pane, as stripes of text.
        let draw = |w: usize, h: usize, cols: &[(usize, usize)]| {
            let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w as u32, h as u32);
            buf.make_mut_slice().fill(white);
            for y in (h / 4..h * 3 / 4).step_by(8) {
                for &(a, b) in cols {
                    buf.make_mut_slice()[y * w + a..y * w + b].fill(grey);
                }
            }
            buf
        };
        let wide = draw(1280, 800, &[(300, 600), (700, 1200)]);
        assert!(blank_panes(&wide, 1.0).is_empty());
        assert_eq!(
            blank_panes(&draw(1280, 800, &[(300, 600)]), 1.0),
            ["reader"]
        );
        assert_eq!(blank_panes(&draw(1280, 800, &[]), 1.0), ["list", "reader"]);
        // At 150%, the panes are 1.5 times as far in.
        let scaled = draw(1920, 1200, &[(450, 900), (1050, 1800)]);
        assert!(blank_panes(&scaled, 1.5).is_empty());
        // Below 1040 px the rail is narrow: 900 px wide, the list is at 64-348.
        assert!(blank_panes(&draw(900, 700, &[(100, 300), (400, 850)]), 1.0).is_empty());
        assert_eq!(blank_panes(&draw(900, 700, &[(400, 850)]), 1.0), ["list"]);
    }

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
            mark_window(pt, window_mark(real));
            mark_window(other, window_mark(throwaway));
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
    fn saved_windows_stay_on_a_visible_monitor() {
        let primary = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let left = Rect {
            left: -2560,
            top: -200,
            right: 0,
            bottom: 1240,
        };
        let at = |x, y, w, h| db::Geometry {
            x,
            y,
            w,
            h,
            maximized: false,
        };
        // On a monitor: kept as it was.
        let g = at(100, 80, 1280, 820);
        assert_eq!(fit_to_monitors(g, &[primary, left]), g);
        assert_eq!(
            fit_to_monitors(at(-2000, 0, 1280, 820), &[primary, left]),
            at(-2000, 0, 1280, 820)
        );
        // Its monitor was removed: centred on the primary one.
        assert_eq!(
            fit_to_monitors(at(-2000, 0, 1280, 820), &[primary]),
            at(320, 110, 1280, 820)
        );
        // Larger than the monitor left: shrunk to it.
        assert_eq!(
            fit_to_monitors(at(-2500, -150, 2400, 1400), &[primary]),
            at(0, 0, 1920, 1040)
        );
        // Partly off the edge with the title bar showing: pulled onto it.
        assert_eq!(
            fit_to_monitors(at(1500, 600, 1280, 820), &[primary]),
            at(640, 220, 1280, 820)
        );
        // The title bar above the screen (only the bottom shows): centred.
        assert_eq!(
            fit_to_monitors(at(100, -700, 1280, 820), &[primary]),
            at(320, 110, 1280, 820)
        );
        // A sliver of the title bar is not enough.
        assert_eq!(
            fit_to_monitors(at(1900, 80, 1280, 820), &[primary]),
            at(320, 110, 1280, 820)
        );
        // Maximized stays maximized, wherever it goes.
        let max = db::Geometry {
            maximized: true,
            ..at(3000, 0, 1280, 820)
        };
        assert!(fit_to_monitors(max, &[primary]).maximized);
        // No monitor known: left alone.
        assert_eq!(fit_to_monitors(g, &[]), g);
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
