//! Komorebi 1.0 on GPUI (docs/GPUI-MIGRATION.md). The start-up of the
//! Slint app (one app per cache, the hand-off to the running one, the
//! installer's mutex, the cache opened beside the window) through the same
//! library code, a DirectX pre-flight, the brand's theme with its live
//! Windows feeds, and the window shell at its saved place.
//! `KOMOREBI_SNAPSHOT` captures the first frame to a file (snapshot.rs); the
//! tests share test_support.rs (docs/GPUI-TESTING.md).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, px, size,
};
use komorebi::{db, instance};
use komorebi_gpui::theme::{self, ThemeInput, feeds};
use komorebi_gpui::window::{self, DEFAULT_SIZE, Init, MIN_SIZE, MainWindow, Start};
use komorebi_gpui::{i18n, icons, preflight, snapshot, tabs, text};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

/// Development builds with `alloc-count` count every allocation (the A/B
/// harness, alloc_count.rs).
#[cfg(feature = "alloc-count")]
#[global_allocator]
static ALLOCATOR: komorebi_gpui::alloc_count::Counting = komorebi_gpui::alloc_count::Counting;

/// The release allocator is mimalloc, not the Windows heap: the interactive
/// paths make 5,000 to 7,000 allocations per frame or key, and heap calls
/// were 10 to 12 percent of their CPU (docs/PERF.md 8).
#[cfg(not(feature = "alloc-count"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Gives free memory back to Windows at once instead of after mimalloc's
/// default 10 ms delay. The opening of a large message allocates and frees
/// 140 MB in a burst; with the delay the freed pages stay resident and the
/// peak working set is 7 MB higher (measured; docs/PERF.md 8).
#[cfg(not(feature = "alloc-count"))]
fn tune_allocator() {
    /// `mi_option_purge_delay` in mimalloc 2.x's `mi_option_e`
    /// (libmimalloc-sys =0.1.49, c_src/mimalloc/v2/include/mimalloc.h); the
    /// crate does not export it.
    const PURGE_DELAY: i32 = 15;
    unsafe extern "C" {
        fn mi_option_set(option: i32, value: std::ffi::c_long);
    }
    // SAFETY: mimalloc is the global allocator, so its symbols are linked;
    // the call sets an integer option and is valid at any time.
    unsafe { mi_option_set(PURGE_DELAY, 0) };
}

fn main() -> std::process::ExitCode {
    #[cfg(not(feature = "alloc-count"))]
    tune_allocator();
    // Development builds with the `gallery` feature: the component gallery
    // instead of the app (no cache, no accounts).
    #[cfg(feature = "gallery")]
    if let Some(choice) = komorebi_gpui::gallery::requested() {
        return komorebi_gpui::gallery::run(choice);
    }
    let started = Instant::now();
    // A snapshot request is checked before anything opens: offline, on a
    // throwaway cache, or not at all (snapshot.rs).
    let snapshot = match snapshot::from_env() {
        Ok(request) => request,
        Err(reason) => {
            eprintln!("komorebi-gpui: {reason}");
            return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
        }
    };
    // The A/B harness's script (bench.rs): off unless KOMOREBI_BENCH is set,
    // and refused on a real cache.
    if let Err(reason) = komorebi_gpui::bench::from_env() {
        eprintln!("komorebi-gpui: {reason}");
        return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
    }
    // The sampling profiler, from the start of the process (a bench run only).
    #[cfg(feature = "sampler")]
    if komorebi_gpui::bench::enabled()
        && let Some(name) =
            komorebi::dev_var("KOMOREBI_PROF_START").and_then(|v| v.into_string().ok())
    {
        komorebi_gpui::sampler::start(&name);
    }
    let timing = komorebi::dev_var("KOMOREBI_TIMING").is_some();
    let mark = move |what: &str| {
        if timing {
            eprintln!(
                "[timing] {what}: {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    };
    let db_path = db::default_path();
    // Started by Restart now or Start a fresh cache: wait for the app that
    // started this one to close (it holds the cache and the one-app lock).
    instance::wait_for_restart();
    // One app per cache, before the cache opens; a second launch hands its
    // mail link over or brings the running window forward.
    if !instance::single_instance(&db_path) {
        return std::process::ExitCode::SUCCESS;
    }
    instance::hold_app_mutex();
    // A panic leaves one line in crash.log; the next launch says so once.
    let crash_log = komorebi::crash::log_path(&db_path);
    komorebi::crash::install(crash_log.clone());
    let window_mark = instance::window_mark(&db_path);
    // The language of start-up text, from Windows alone (the saved choice
    // is in the cache, not read yet).
    let lang = komorebi::i18n::startup_lang();
    komorebi_i18n::set_current(lang);

    // Beside GPUI's platform start (OLE, DirectWrite, about 30 ms): the
    // DirectX probe and the cache open. Each is joined where it is needed;
    // a lost thread does the work inline.
    let probe = std::thread::Builder::new()
        .name("dx-probe".into())
        .spawn(preflight::probe)
        .ok();
    let opening = {
        let path = db_path.clone();
        std::thread::Builder::new()
            .name("db-open".into())
            .spawn(move || db::Db::open_ui(&path))
            .ok()
    };
    let app = gpui_kit::application().with_assets(icons::AppAssets);
    mark("platform created");
    let probed = probe
        .and_then(|h| h.join().ok())
        .unwrap_or_else(preflight::probe);
    mark("directx probed");
    if let Err(reason) = probed {
        eprintln!("komorebi-gpui: no DirectX 11 device: {reason}");
        preflight::report(lang, &reason);
        return std::process::ExitCode::from(preflight::EXIT_NO_DIRECTX);
    }
    let opened = opening
        .and_then(|h| h.join().ok())
        .unwrap_or_else(|| db::Db::open_ui(&db_path));
    // A cache this Windows user cannot unlock: its own dialog (Start a
    // fresh cache or Quit) over the empty window. Any other failure (a
    // cache made by a newer Komorebi, a full disk) is said in a native box,
    // and the app exits, as the Slint app does.
    let mut start = Start::default();
    let db = match opened {
        Ok((db, aside)) => {
            start.damaged =
                aside.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            Some(db)
        }
        Err(e) if window::cache_is_locked(&e) => {
            eprintln!("komorebi-gpui: the cache is locked: {e:#}");
            start.locked = Some(db_path.clone());
            None
        }
        Err(e) => {
            eprintln!("komorebi-gpui: opening the cache: {e:#}");
            if snapshot.is_none() {
                preflight::fail(&komorebi_i18n::with_lang(lang, || {
                    text::start_failed(&format!("{e:#}"))
                }));
            }
            return std::process::ExitCode::FAILURE;
        }
    };
    // Marked seen only when it can be said: the locked-cache window has no
    // status line (the Slint app reads it after the cache has loaded).
    start.crashed = db.is_some() && komorebi::crash::take_unseen(&crash_log);
    // Started by Windows at sign-in (`--minimized`, Start with Windows):
    // the window stays hidden and the tray icon is the way in. A mail link
    // (`komorebi-gpui.exe "mailto:..."`) opens a prefilled composer (never
    // sent) and shows the window even then.
    start.mailto = komorebi::launch::mailto_from_args(std::env::args().skip(1));
    start.minimized =
        snapshot.is_none() && komorebi::notify::launched_minimized(std::env::args().skip(1));
    let hidden = start.hidden();
    let settings = db
        .as_ref()
        .and_then(|db| db.settings().ok())
        .unwrap_or_default();
    // The diagnostics log, when Settings > Privacy switched it on.
    if settings.diagnostics_log {
        komorebi::diag::set(Some(komorebi::diag::log_path(&db_path)));
    }
    mark("cache loaded");
    // The saved choice (or KOMOREBI_LANG) now that the cache is read, as
    // the Slint app does.
    let lang =
        komorebi::i18n::resolve_here(komorebi::i18n::dev_setting().unwrap_or(settings.language));
    let mut system = komorebi::theme::read();
    // Kit animations rest in a snapshot.
    if snapshot.is_some() {
        system.animate = false;
    }
    let theme = snapshot
        .as_ref()
        .and_then(|r| r.theme)
        .unwrap_or(settings.theme);
    let theme_input = ThemeInput::new(theme, system);
    // Snapshots start at their own size, never the saved place.
    let size_asked = snapshot.as_ref().map(|r| r.size.unwrap_or(DEFAULT_SIZE));
    let saved = settings.gpui_window.filter(|_| snapshot.is_none());
    let exit_code = Arc::new(AtomicU8::new(0));
    let init = Init {
        db,
        timing: timing.then_some(started),
        exit_after_first_frame: komorebi::dev_var("KOMOREBI_EXIT_AFTER_FIRST_FRAME").is_some(),
        snapshot,
        exit_code: exit_code.clone(),
        start,
    };
    let still = init.snapshot.is_some();

    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        // The language global and tr!'s value (komorebi_gpui::i18n).
        i18n::init(lang, cx);
        tabs::bind_keys(cx);
        komorebi_gpui::access::bind_keys(cx);
        komorebi_gpui::views::reader::bind_keys(cx);
        komorebi_gpui::views::composer::bind_keys(cx);
        komorebi_gpui::views::settings::bind_keys(cx);
        // The tokens, Kit's theme and reduce motion, before any window;
        // then the text size watcher (the other feeds are per window).
        theme::install(theme_input, cx);
        // A snapshot keeps animations at rest even when the window's
        // activation re-reads Windows' settings.
        if still {
            feeds::hold_still(cx);
        }
        feeds::watch(cx);
        // GPUI does not quit when the last window closes; the app decides.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let (w, h) = size_asked.unwrap_or(DEFAULT_SIZE);
        let default = WindowBounds::Windowed(Bounds::centered(None, size(px(w), px(h)), cx));
        let bounds = saved
            .map(window::from_geometry)
            .filter(|b| window::fits_a_display(b.get_bounds(), cx))
            .unwrap_or(default);
        let options = WindowOptions {
            window_bounds: Some(bounds),
            // `appears_transparent` hides the system caption; `title` stays
            // the window's title (taskbar, Alt+Tab, UI Automation).
            titlebar: Some(TitlebarOptions {
                title: Some(text::window_title(0).into()),
                appears_transparent: komorebi_gpui::titlebar::app_drawn(),
                traffic_light_position: None,
            }),
            window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
            // No Mica or Acrylic: the paper is paper (IMPLEMENTATION.md).
            window_background: WindowBackgroundAppearance::Opaque,
            // Start with Windows: hidden until the tray icon brings it.
            show: !hidden,
            focus: !hidden,
            ..Default::default()
        };
        let dark = cx_dark(cx);
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            mark_window(window, window_mark);
            feeds::set_frame_dark(window, dark);
            cx.new(|cx| MainWindow::new(init, window, cx))
        });
        if let Err(e) = opened {
            eprintln!("komorebi-gpui: opening the window: {e:#}");
            cx.quit();
        }
        mark("window created");
        if !hidden {
            cx.activate(true);
        }
    });
    // Remove all data (Settings > Privacy): once the cache is closed.
    window::remove_data_if_asked();
    std::process::ExitCode::from(exit_code.load(Ordering::Relaxed))
}

/// Whether the app draws dark (the window frame follows it).
fn cx_dark(cx: &App) -> bool {
    use theme::ActiveTokens as _;
    cx.tokens().scheme.is_dark()
}

/// Marks the window as this cache's, so a second launch finds it
/// (`komorebi::instance::single_instance`), never by its translated title.
#[cfg(windows)]
fn mark_window(window: &gpui::Window, mark: usize) {
    if let Some(hwnd) = snapshot::window_handle(window) {
        // SAFETY: GPUI's live window, on the thread that made it.
        unsafe { instance::mark_window(hwnd as _, mark) };
    }
}

#[cfg(not(windows))]
fn mark_window(_: &gpui::Window, _: usize) {}
