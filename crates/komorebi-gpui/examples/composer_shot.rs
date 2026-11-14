//! A development snapshot of the composer (phase 4), for comparing it with
//! the brand's `compose` mockup until the mail view mounts it in the app
//! (`KOMOREBI_SNAPSHOT_ACTION=compose` then, an integration item).
//!
//! Safety as the app's snapshots (`snapshot::from_env`): it refuses to run
//! without `KOMOREBI_OFFLINE` and a throwaway `KOMOREBI_DB` outside the
//! app's cache folder, and quits once the frame is captured. The cache is
//! seeded if empty; nothing goes to the network.
//!
//! `KOMOREBI_COMPOSE_STATE=formatted|plain|reply|empty` picks the draft
//! (default `formatted`: the mockup's message).

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, div, px,
    size,
};
use komorebi::compose::{ComposeDefaults, Draft, Kind};
use komorebi::db::Db;
use komorebi::mail::rich::{self as model, Block, ListKind, Para, Run, Style};
use komorebi_gpui::theme::{self, ActiveTokens as _, ThemeInput, layout, radius, space};
use komorebi_gpui::titlebar::TitleBar;
use komorebi_gpui::views::composer::{self, ComposerView};
use komorebi_gpui::{i18n, icons, snapshot};

struct Shell {
    composer: Entity<ComposerView>,
    /// The capture, started after the first render (as the app's
    /// snapshot: it waits for DWM to compose the frame).
    capture: Option<(snapshot::Request, Arc<AtomicU8>)>,
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((request, exit)) = self.capture.take() {
            // GPUI's `render_to_image` (test-support, a dev-dependency):
            // the scene drawn to a texture, on screen or not.
            let handle = window.window_handle();
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(400))
                    .await;
                // The display may be off (no frames are drawn then): draw
                // twice by hand, so the body's width change lays it out.
                for _ in 0..2 {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                    });
                }
                let image = handle.update(cx, |_, window, _| window.render_to_image());
                let saved = match image {
                    Ok(Ok(img)) => img.save(&request.path).map_err(|e| e.to_string()),
                    Ok(Err(e)) => Err(format!("{e:#}")),
                    Err(e) => Err(format!("{e:#}")),
                };
                match saved {
                    Ok(()) => eprintln!("composer_shot: wrote {}", request.path.display()),
                    Err(e) => {
                        eprintln!("composer_shot: {e}");
                        exit.store(snapshot::EXIT_FAILED, Ordering::Relaxed);
                    }
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
        let c = cx.tokens().color;
        // The mockup's three columns: the nav and the list (empty here),
        // then the composer's pane.
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c.canvas)
            .child(TitleBar::new().always_active(true))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .gap(px(space::PANE_GAP))
                    .pr(px(space::PANE_GAP))
                    .pb(px(space::PANE_GAP))
                    .child(div().w(px(layout::NAV_W)).flex_none())
                    .child(
                        div()
                            .w(px(layout::LIST_W))
                            .flex_none()
                            .rounded(px(radius::SURFACE))
                            .bg(c.paper),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(px(radius::SURFACE))
                            .overflow_hidden()
                            .bg(c.paper)
                            .child(self.composer.clone()),
                    ),
            )
    }
}

/// The mockup's message, formatted.
fn mockup_doc() -> model::Doc {
    let styled = |text: &str, f: &dyn Fn(&mut Style)| {
        let mut style = Style::default();
        f(&mut style);
        Run::styled(text, style)
    };
    let para = |runs: Vec<Run>| Para {
        runs,
        ..Para::default()
    };
    let item = |text: &str| Para {
        list: Some((ListKind::Bullet, 0)),
        ..Para::text(text)
    };
    let mut doc = komorebi::compose::rich::doc_from_text("", ComposeDefaults::default().doc());
    doc.blocks = vec![
        Block::Para(Para::text("Hi both,")),
        Block::Para(Para::default()),
        Block::Para(para(vec![
            Run::new("Marta's photos from "),
            styled("Gerês", &|s| s.b = true),
            Run::new(" are in the shared folder, and the best ones are "),
            styled("in this album", &|s| {
                s.link = Some("https://example.org/album".into())
            }),
            Run::new(". Two things for "),
            styled("Thursday", &|s| s.i = true),
            Run::new(":"),
        ])),
        Block::Para(item("Bring the USB stick for the full-size files")),
        Block::Para(item("Decide on the coast weekend")),
        Block::Para(Para::text("See you then.")),
        Block::Para(Para::default()),
        Block::Para(Para::text("Inês")),
    ];
    doc
}

fn main() -> std::process::ExitCode {
    let request = match snapshot::from_env() {
        Ok(Some(r)) => r,
        Ok(None) => {
            eprintln!("composer_shot: set KOMOREBI_SNAPSHOT, KOMOREBI_OFFLINE and KOMOREBI_DB");
            return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
        }
        Err(reason) => {
            eprintln!("composer_shot: {reason}");
            return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
        }
    };
    let Some(db_path) = komorebi::dev_var("KOMOREBI_DB").map(std::path::PathBuf::from) else {
        return std::process::ExitCode::from(snapshot::EXIT_REFUSED);
    };
    let mut db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("composer_shot: {e:#}");
            return std::process::ExitCode::from(snapshot::EXIT_FAILED);
        }
    };
    if db.accounts().map(|a| a.is_empty()).unwrap_or(true)
        && let Err(e) = komorebi::seed::seed(&mut db, 20)
    {
        eprintln!("composer_shot: seeding: {e:#}");
        return std::process::ExitCode::from(snapshot::EXIT_FAILED);
    }
    let account = db
        .accounts()
        .ok()
        .and_then(|a| a.first().map(|a| a.id))
        .unwrap_or_default();
    // The newest message with a body, for the reply.
    let first = db
        .folders()
        .ok()
        .map(|f| f.iter().map(|f| f.id).collect::<Vec<_>>())
        .and_then(|ids| db.message_ids(&ids, "").ok())
        .and_then(|ids| {
            ids.into_iter().find_map(|id| {
                let m = db.message(id).ok().flatten()?;
                let body = komorebi::mail::mime::parse_body(m.raw.as_deref()?);
                Some((m, body))
            })
        });
    let state = komorebi::dev_var("KOMOREBI_COMPOSE_STATE")
        .and_then(|v| v.into_string().ok())
        .unwrap_or_else(|| "formatted".into());
    let lang = komorebi::i18n::resolve_here(komorebi::i18n::dev_setting().unwrap_or_default());
    let mut system = komorebi::theme::read();
    system.animate = false;
    let theme_input = ThemeInput::new(
        request.theme.unwrap_or(komorebi::db::ThemeChoice::Light),
        system,
    );
    let (w, h) = request.size.unwrap_or(layout::WINDOW_DEFAULT);
    let exit = Arc::new(AtomicU8::new(0));
    let exit_code = exit.clone();
    gpui_kit::application()
        .with_assets(icons::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            i18n::init(lang, cx);
            theme::install(theme_input, cx);
            composer::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(w), px(h)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Komorebi".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            };
            let request = request.clone();
            let exit = exit.clone();
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                let composer = cx.new(|cx| ComposerView::new(std::rc::Rc::new(db), window, cx));
                composer.update(cx, |v, cx| show(v, &state, account, first, window, cx));
                cx.new(|_| Shell {
                    composer,
                    capture: Some((request, exit)),
                })
            });
            if let Err(e) = opened {
                eprintln!("composer_shot: {e:#}");
                cx.quit();
            }
        });
    std::process::ExitCode::from(exit_code.load(Ordering::Relaxed))
}

/// Starts the draft the snapshot shows.
fn show(
    v: &mut ComposerView,
    state: &str,
    account: i64,
    first: Option<(komorebi::db::Message, komorebi::mail::mime::Body)>,
    window: &mut Window,
    cx: &mut Context<ComposerView>,
) {
    match state {
        "empty" => v.compose_new(None, window, cx),
        "plain" => {
            v.set_defaults(ComposeDefaults {
                rich: false,
                ..ComposeDefaults::default()
            });
            v.start(
                Draft {
                    to: "Marta Silva <marta@example.com>, Rui Mendes <rui@example.com>, ".into(),
                    subject: "Saturday, and the photos".into(),
                    body: "Hi both,\n\nSee you then.\n\nInês".into(),
                    ..Draft::new(account)
                },
                window,
                cx,
            );
        }
        "reply" => {
            if let Some((m, body)) = first {
                v.reply(&m, Some(&body), false, window, cx);
            }
        }
        "attach" => {
            // Two made-up files beside the throwaway cache.
            let dir = komorebi::dev_var("KOMOREBI_DB")
                .map(std::path::PathBuf::from)
                .and_then(|p| p.parent().map(|p| p.join("composer-shot-files")))
                .unwrap_or_default();
            let _ = std::fs::create_dir_all(&dir);
            let files = [("Q3 report.pdf", 2_400_000), ("Site photos.jpg", 900_000)];
            let mut paths = Vec::new();
            for (name, len) in files {
                let path = dir.join(name);
                if std::fs::write(&path, vec![0u8; len]).is_ok() {
                    paths.push(path);
                }
            }
            v.start(
                Draft {
                    to: "Marta Silva <marta@example.com>, ".into(),
                    subject: "The report".into(),
                    touched: true,
                    ..Draft::new(account)
                },
                window,
                cx,
            );
            v.attach_paths(paths, cx);
        }
        _ => {
            let mut d = Draft {
                kind: Kind::New,
                to: "Marta Silva <marta@example.com>, Rui Mendes <rui@example.com>, ana.r".into(),
                subject: "Saturday, and the photos".into(),
                touched: true,
                ..Draft::new(account)
            };
            d.rich = Some(komorebi::mail::rich::edit::Editor::new(mockup_doc()));
            v.start(d, window, cx);
            if state == "preview" {
                v.toggle_preview(window, cx);
            }
        }
    }
}
