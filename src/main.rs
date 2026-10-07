#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod format;
mod mail;
mod providers;
mod role;
mod rows;
mod secrets;

slint::include_modules!();

use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let timing = std::env::var_os("KOMOREBI_TIMING").is_some();
    let mark = move |what: &str| {
        if timing {
            eprintln!(
                "[timing] {what}: {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    };

    // The software renderer opens a window in a few milliseconds, while a
    // GPU context costs 100+ ms at startup. SLINT_BACKEND=winit-femtovg
    // opts into the OpenGL renderer (useful on very large displays).
    if std::env::var_os("SLINT_BACKEND").is_none_or(|v| v.is_empty()) {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("software".into())
            .select()?;
    }
    let ui = AppWindow::new()?;
    mark("window created");
    if let Err(e) = app::install(&ui, db::default_path(), &mark) {
        // The release build has no console, so say it on screen.
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Komorebi")
            .set_description(format!("Komorebi couldn't start.\n\n{e:#}"))
            .show();
        return Err(e);
    }
    mark("cache loaded");

    if let Some(path) = std::env::var_os("KOMOREBI_SNAPSHOT") {
        snapshot_and_quit(&ui, path.into());
    }

    ui.run()?;
    Ok(())
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
    // Select the n-th message and optionally trigger an action, so the
    // reader, composer and settings can be checked too.
    let select = std::env::var("KOMOREBI_SNAPSHOT_SELECT")
        .ok()
        .and_then(|s| s.parse::<i32>().ok());
    let action = std::env::var("KOMOREBI_SNAPSHOT_ACTION").unwrap_or_default();
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(600), move || {
        let Some(ui) = weak.upgrade() else { return };
        for _ in 0..=select.unwrap_or(-1) {
            ui.invoke_select_offset(1);
        }
        match action.as_str() {
            "reply" => ui.invoke_reply(),
            "compose" => ui.invoke_compose(),
            "settings" => ui.set_show_settings(true),
            _ => {}
        }
    });
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(1500), move || {
        let Some(ui) = weak.upgrade() else { return };
        let t = Instant::now();
        let snapshot = ui.window().take_snapshot();
        if std::env::var_os("KOMOREBI_TIMING").is_some() {
            eprintln!(
                "[timing] full-frame snapshot: {:.1} ms",
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Ok(buf) = snapshot {
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
            std::fs::write(&path, bmp).ok();
        }
        slint::quit_event_loop().ok();
    });
}
