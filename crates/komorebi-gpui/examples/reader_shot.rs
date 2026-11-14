//! A snapshot of the reader (docs/GPUI-MIGRATION.md phase 3): one message
//! in the reader's paper pane, captured once its page is laid out and the
//! tiles in view are drawn, through the app's own snapshot code
//! (`komorebi_gpui::snapshot`) and with its safety rules: it runs only with
//! `KOMOREBI_OFFLINE` set and `KOMOREBI_DB` naming a throwaway cache
//! outside the app's cache folder, and `KOMOREBI_SNAPSHOT` the image to
//! write (`KOMOREBI_THEME`, `KOMOREBI_SIZE`, `KOMOREBI_LANG` as for the
//! app). `KOMOREBI_READER_EML` names the message (default: the HTML corpus'
//! `12-personal-applemail.eml`); it is added to the throwaway cache with
//! the seed account. Until the list lane mounts the reader in the mail
//! view, this is how the reader is looked at.
//!
//! ```powershell
//! $env:KOMOREBI_OFFLINE = '1'; $env:KOMOREBI_DB = "$scratch\shot.sqlite3"
//! $env:KOMOREBI_SNAPSHOT = "$scratch\reader.png"; $env:KOMOREBI_SIZE = '632x784'
//! cargo run -p komorebi-gpui --example reader_shot
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, div, px, size,
};
use komorebi::db::Db;
use komorebi::mail::mime;
use komorebi_gpui::theme::{self, ActiveTokens as _, ThemeInput, space};
use komorebi_gpui::views::reader::{self, ReaderView};
use komorebi_gpui::widgets::pane::Pane;
use komorebi_gpui::{i18n, icons, snapshot};

/// The reader in its pane on canvas, as the mail view lays it out.
struct Shot {
    reader: Entity<ReaderView>,
    /// Taken by the capture, once.
    request: Option<snapshot::Request>,
    exit: Arc<AtomicU8>,
    _settled: Subscription,
}

impl Shot {
    fn new(
        reader: Entity<ReaderView>,
        request: snapshot::Request,
        exit: Arc<AtomicU8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settled = cx.observe_in(&reader, window, |this: &mut Shot, reader, window, cx| {
            if !reader.read(cx).settled() {
                return;
            }
            let Some(request) = this.request.take() else {
                return;
            };
            let exit = this.exit.clone();
            // After this update: a frame with the last tile, rendered to a
            // texture (whether or not the window is on screen, as the
            // gallery's shots), written, and the app quits.
            window.defer(cx, move |window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
                let written = window
                    .render_to_image()
                    .and_then(|img| Ok(img.save(&request.path)?));
                match written {
                    Ok(()) => eprintln!("reader_shot: {}", request.path.display()),
                    Err(e) => {
                        eprintln!("reader_shot: {e:#}");
                        exit.store(snapshot::EXIT_FAILED, Ordering::Relaxed);
                    }
                }
                cx.quit();
            });
        });
        Shot {
            reader,
            request: Some(request),
            exit,
            _settled: settled,
        }
    }
}

impl Render for Shot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = cx.tokens().color;
        div()
            .size_full()
            .bg(c.canvas)
            .p(px(space::PANE_GAP))
            .child(Pane::new("reader-pane").child(self.reader.clone()))
    }
}

/// Adds the message in `eml` to the throwaway cache with the seed
/// account: its id.
fn add_message(db: &mut Db, eml: &PathBuf) -> anyhow::Result<i64> {
    let raw = std::fs::read(eml)?;
    let seeded = komorebi::seed::seed(db, 0)?;
    let mut header = mime::parse_header(&raw);
    header.uid = 1;
    db.insert_headers(seeded.account, seeded.inbox, &[header])?;
    let id = *db
        .message_ids(&[seeded.inbox], "")?
        .first()
        .ok_or_else(|| anyhow::anyhow!("the message was not stored"))?;
    db.set_body(id, &raw, "", false, Default::default())?;
    Ok(id)
}

fn main() -> std::process::ExitCode {
    let request = match snapshot::from_env() {
        Ok(Some(r)) => r,
        Ok(None) => {
            eprintln!("reader_shot: set KOMOREBI_SNAPSHOT (and KOMOREBI_OFFLINE, KOMOREBI_DB)");
            return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
        }
        Err(reason) => {
            eprintln!("reader_shot: {reason}");
            return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
        }
    };
    let eml = komorebi::dev_var("KOMOREBI_READER_EML")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/html/12-personal-applemail.eml")
        });
    // snapshot::from_env checked that KOMOREBI_DB names a throwaway cache;
    // never the app's own (no fallback to the default path).
    let Some(db_path) = komorebi::dev_var("KOMOREBI_DB").map(PathBuf::from) else {
        eprintln!("reader_shot: KOMOREBI_DB must name a throwaway cache");
        return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
    };
    let mut db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("reader_shot: opening {}: {e:#}", db_path.display());
            return std::process::ExitCode::from(snapshot::EXIT_FAILED);
        }
    };
    let id = match add_message(&mut db, &eml) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("reader_shot: {}: {e:#}", eml.display());
            return std::process::ExitCode::from(snapshot::EXIT_FAILED);
        }
    };
    let lang = komorebi::i18n::resolve_here(komorebi::i18n::dev_setting().unwrap_or_default());
    let mut system = komorebi::theme::read();
    system.animate = false;
    let theme_input = ThemeInput::new(
        request.theme.unwrap_or(komorebi::db::ThemeChoice::Light),
        system,
    );
    let (w, h) = request.size.unwrap_or((632., 784.));
    let exit = Arc::new(AtomicU8::new(0));
    let code = exit.clone();
    gpui_kit::application()
        .with_assets(icons::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            i18n::init(lang, cx);
            reader::bind_keys(cx);
            theme::install(theme_input, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(w), px(h)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Komorebi reader".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                let reader = cx.new(|cx| ReaderView::new(std::rc::Rc::new(db), window, cx));
                reader.update(cx, |r, cx| r.open(id, window, cx));
                cx.new(|cx| Shot::new(reader, request, exit, window, cx))
            });
            if let Err(e) = opened {
                eprintln!("reader_shot: opening the window: {e:#}");
                cx.quit();
            }
            cx.activate(true);
        });
    std::process::ExitCode::from(code.load(Ordering::Relaxed))
}
