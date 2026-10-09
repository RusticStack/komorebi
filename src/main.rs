#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod crash;
mod format;
mod rows;

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
    let crash_log = crash::log_path(&db_path);
    crash::install(crash_log.clone());
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
    mark("window created");
    let geometry = match app::install(&ui, db_path, &mark) {
        Ok(geometry) => geometry,
        Err(e) => {
            // The release build has no console, so say it on screen.
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Komorebi")
                .set_description(format!("Komorebi couldn't start.\n\n{e:#}"))
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
        app::notice("Komorebi closed unexpectedly last time; see crash.log");
    }

    if let Some(path) = komorebi::dev_var("KOMOREBI_SNAPSHOT") {
        snapshot_and_quit(&ui, path.into());
    }

    ui.run()?;
    if !app::remove_data_if_asked(ui) {
        app::finish_actions();
    }
    Ok(())
}

/// Window events the app follows through winit: files dragged from
/// Explorer (winit reports them, as an OLE drop target, one file at a
/// time), focus (for the taskbar flash), the Windows theme (for the System
/// theme choice), and the window's place while it is neither maximized nor
/// minimized (saved on close). With `started` (`KOMOREBI_TIMING`), also the
/// first frame: winit asks for the first redraw, Slint paints it, and a
/// zero timer then runs on the next pass of the event loop, after the
/// pixels are on screen. The first event (the window exists by then) also
/// sets the window's icons and its title bar's theme.
fn watch_window_events(ui: &AppWindow, mut started: Option<Instant>) {
    use slint::ComponentHandle;
    use slint::winit_030::winit::{event::WindowEvent, window::Theme};
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    let mut dressed = false;
    ui.window().on_winit_window_event(move |window, event| {
        if !std::mem::replace(&mut dressed, true) {
            window.with_winit_window(set_icons);
            app::refresh_title_bar();
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
            WindowEvent::Focused(focused) => app::window_focused(*focused),
            WindowEvent::ThemeChanged(theme) => app::system_theme(*theme == Theme::Dark),
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
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

/// "Inbox (7) - Komorebi" while the inboxes hold unread mail, else
/// "Komorebi".
pub fn window_title(unread: i32) -> String {
    if unread > 0 {
        format!("Inbox ({unread}) - Komorebi")
    } else {
        "Komorebi".into()
    }
}

/// A window title `window_title` gives.
fn is_app_title(title: &str) -> bool {
    title == "Komorebi" || (title.starts_with("Inbox (") && title.ends_with(") - Komorebi"))
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

/// A dark (or light) title bar, to match the app's theme. winit already
/// follows the Windows theme, so this matters when Settings picks Light or
/// Dark over it.
#[cfg(windows)]
pub fn dark_title_bar(ui: &AppWindow, dark: bool) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    let Some(hwnd) = hwnd(ui) else { return };
    let value: windows_sys::core::BOOL = dark.into();
    // SAFETY: this process's live window, and a BOOL with its size.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as _,
            (&raw const value).cast(),
            std::mem::size_of_val(&value) as u32,
        )
    };
}

#[cfg(not(windows))]
pub fn dark_title_bar(_: &AppWindow, _: bool) {}

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

/// The single-instance mutex for the cache at `db_path`: `Local\Komorebi-`
/// and a hash (FNV-1a, stable across builds) of its absolute, lower-cased
/// path, so a throwaway `KOMOREBI_DB` instance runs beside the real one.
fn instance_name(db_path: &Path) -> String {
    let path = std::path::absolute(db_path).unwrap_or_else(|_| db_path.to_path_buf());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_string_lossy().to_lowercase().bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("Local\\Komorebi-{hash:016x}")
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
    if let Some(hwnd) = app_window() {
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

/// The first top-level window of winit's class titled as this app (the
/// title carries the unread count, so it is matched by `is_app_title`).
#[cfg(windows)]
fn app_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowTextW};
    use windows_sys::core::BOOL;
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
        let mut buf = [0u16; 128];
        let len = buf.len() as i32;
        // SAFETY: each call writes at most `len` characters into `buf`.
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), len) };
        if String::from_utf16_lossy(&buf[..n.max(0) as usize]) != "Window Class" {
            return 1;
        }
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), len) };
        if !is_app_title(&String::from_utf16_lossy(&buf[..n.max(0) as usize])) {
            return 1;
        }
        // SAFETY: `data` is the `Option` passed below, alive for the call.
        unsafe { *(data as *mut Option<HWND>) = Some(hwnd) };
        0
    }
    let mut found: Option<HWND> = None;
    // SAFETY: the callback only runs during the call, while `found` lives.
    unsafe { EnumWindows(Some(each), &mut found as *mut Option<HWND> as LPARAM) };
    found
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
            // A new message whose To address is wrong, after leaving the
            // field: shows the inline error and why Send is off.
            "compose-invalid" => {
                ui.invoke_compose();
                let to = "Ana Silva <ana@>";
                ui.set_c_to(to.into());
                ui.invoke_c_edited(ComposeField::To, to.into());
                ui.invoke_c_left(ComposeField::To);
            }
            // A new message with formatting markers in the body, which has
            // the focus; Ctrl+K and an address follow below.
            "compose-format" => {
                ui.invoke_compose();
                let body = "Hi Ana,\n\nThe **report** is _ready_. Notes:\n\n- totals checked\n\
                    - [the sheet](https://example.org/sheet)\n\n> Can you send it today?\n\nThanks";
                ui.set_c_body(body.into());
                ui.invoke_c_edited(ComposeField::Body, body.into());
                ui.set_c_focus(ComposeField::Body);
                ui.set_c_focus_seq(ui.get_c_focus_seq().wrapping_add(1));
            }
            // The Drafts view (listed once the saved drafts are counted,
            // 250 ms after start).
            "drafts" => ui.invoke_nav_clicked(NavItem {
                kind: 3,
                ..Default::default()
            }),
            // The newest saved draft in the composer, then Discard: shows
            // the prompt (nothing is deleted until it is confirmed).
            "discard" => {
                ui.invoke_status_action_clicked();
                ui.invoke_discard();
            }
            // The newest saved draft in the composer, then Send: shows the
            // send guards' bar for a draft without a subject (nothing is
            // sent until "Send anyway").
            "guard" => {
                ui.invoke_status_action_clicked();
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
            "settings" => ui.set_show_settings(true),
            // Settings on one section: `settings:general`, `:privacy`,
            // `:about` (or `:accounts`).
            s if s.starts_with("settings:") => {
                let section = ["accounts", "general", "privacy", "about"]
                    .iter()
                    .position(|n| s.ends_with(n))
                    .unwrap_or(0);
                ui.set_settings_section(section as i32);
                ui.set_show_settings(true);
            }
            // Settings > Privacy with the Remove all data prompt (nothing
            // is removed until it is confirmed).
            "remove-data" => {
                ui.set_settings_section(2);
                ui.set_show_settings(true);
                ui.invoke_remove_all_data();
            }
            // Settings with the first account's edit form open.
            "settings-edit" => {
                use slint::Model;
                if let Some(a) = ui.get_accounts().row_data(0) {
                    ui.invoke_edit_account(a.id);
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
            "junk" => ui.invoke_junk(true),
            // The Unread filter; or Mark all as read first (this changes
            // the cache: use a copy), leaving its empty state.
            "unread" => ui.invoke_set_filter(1),
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
    let weak = ui.as_weak();
    let at = std::time::Duration::from_millis(1500 + 250 * walk);
    slint::Timer::single_shot(at, move || {
        let Some(ui) = weak.upgrade() else { return };
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
        (56.0, 300.0)
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
/// times), `nav` or `nav:n` (Shift+F6 to the folders, then Down n times),
/// `list` (Tab and Shift+Tab: the list's current row) or `tab:n` (n Tabs).
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
        Some((what, n)) => (what, n.parse().unwrap_or(0)),
        None => (focus, 0),
    };
    ui.invoke_focus_list();
    match what {
        "toolbar" => {
            press(false, Key::Tab);
            (0..n).for_each(|_| press(false, Key::RightArrow));
        }
        "nav" => {
            press(true, Key::F6);
            (0..n).for_each(|_| press(false, Key::DownArrow));
        }
        "list" => {
            press(false, Key::Tab);
            press(true, Key::Tab);
        }
        "tab" => (0..n).for_each(|_| press(false, Key::Tab)),
        _ => {}
    }
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
    fn window_titles_carry_the_unread_count() {
        assert_eq!(window_title(0), "Komorebi");
        assert_eq!(window_title(-1), "Komorebi");
        assert_eq!(window_title(7), "Inbox (7) - Komorebi");
        assert_eq!(window_title(12_345), "Inbox (12345) - Komorebi");
        // The single-instance check finds the window by any of them.
        for n in [0, 1, 7, 12_345] {
            assert!(is_app_title(&window_title(n)), "{n}");
        }
        for other in ["", "Komorebi - Notepad", "Inbox (3) - Outlook", "komorebi"] {
            assert!(!is_app_title(other), "{other}");
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
