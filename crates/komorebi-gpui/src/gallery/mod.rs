//! The component gallery (development only, the `gallery` feature):
//! `KOMOREBI_GALLERY=light` or `dark` opens one window with every widget
//! on the brand's surfaces at 100 percent text size (or
//! `KOMOREBI_TEXT_SCALE`), instead of the app.
//! It reads no cache and no account.
//!
//! `KOMOREBI_GALLERY_SHOT=<file.png>` renders the window's frame to a PNG
//! (GPUI's `Window::render_to_image`: the Direct3D scene drawn to a texture,
//! so it works whether or not the window is on screen) and quits, for
//! comparing with the brand mockups. `KOMOREBI_GALLERY_OPEN=dialog` opens
//! the confirm dialog over it first. Motion is off in shots (the loader at
//! rest, no fades), as Skua's fixtures do.

#[macro_use]
mod sections;
mod settings;

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, div, px,
    size,
};
use gpui_kit::component::input::InputState;
use komorebi::db::ThemeChoice;
use komorebi::theme::SystemTheme;

use crate::icons::AppAssets;
use crate::theme::{self, ActiveTokens as _, ThemeInput, layout, space};
use crate::titlebar::TitleBar;
use crate::widgets::dialog::{self, Confirm};
use crate::widgets::field::search_field;
use crate::widgets::loader::Loader;
use crate::widgets::toast::Toast;

/// The gallery's window size (logical px).
const SIZE: (f32, f32) = (1280., 900.);
/// Frames to wait before a shot (Kit's dialog settles, the first layout
/// of text).
const SHOT_DELAY_MS: u64 = 600;

/// What `KOMOREBI_GALLERY` asks for, if anything.
pub fn requested() -> Option<ThemeChoice> {
    let v = komorebi::dev_var("KOMOREBI_GALLERY")?.into_string().ok()?;
    match v.as_str() {
        "dark" => Some(ThemeChoice::Dark),
        _ => Some(ThemeChoice::Light),
    }
}

/// The gallery view.
pub struct Gallery {
    pub(crate) search: Entity<InputState>,
    pub(crate) field: Entity<InputState>,
    pub(crate) to: Entity<InputState>,
    pub(crate) loader: Entity<Loader>,
    pub(crate) toast: Entity<Toast>,
    pub(crate) state: sections::State,
}

impl Gallery {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search mail"));
        let field = cx.new(|cx| InputState::new(window, cx).placeholder("Display name"));
        field.update(cx, |s, cx| s.set_value("Inês Duarte", window, cx));
        let to = cx.new(|cx| InputState::new(window, cx));
        let loader = cx.new(|cx| {
            let mut l = Loader::new("Syncing");
            l.set_active(true, cx);
            l
        });
        let toast = cx.new(|cx| {
            let mut t = Toast::new();
            t.show("Sent to Marta Silva", cx);
            t
        });
        Self {
            search,
            field,
            to,
            loader,
            toast,
            state: sections::State::default(),
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = cx.tokens().color;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c.canvas)
            .text_color(c.ink)
            .child(
                TitleBar::new()
                    .title("Komorebi gallery")
                    .center(search_field(&self.search, "Search mail", "Ctrl+E")),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .gap(px(space::PANE_GAP))
                    .px(px(space::PANE_GAP))
                    .pb(px(space::PANE_GAP))
                    .child(sections::controls(self, cx))
                    .child(sections::lists(self, cx))
                    .child(settings::settings(self, cx)),
            )
    }
}

/// Runs the gallery app (instead of Komorebi) and returns when it quits.
pub fn run(choice: ThemeChoice) -> std::process::ExitCode {
    let shot = komorebi::dev_var("KOMOREBI_GALLERY_SHOT").map(PathBuf::from);
    let open = komorebi::dev_var("KOMOREBI_GALLERY_OPEN").and_then(|v| v.into_string().ok());
    // Windows' settings with the development switches over them
    // (KOMOREBI_TEXT_SCALE, KOMOREBI_ANIMATE); 100 percent text unless
    // asked for, and no motion in shots.
    let mut system = komorebi::theme::read();
    if komorebi::dev_var("KOMOREBI_TEXT_SCALE").is_none() {
        system.text_scale = SystemTheme::default().text_scale;
    }
    if shot.is_some() {
        system.animate = false;
    }
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(ThemeInput::new(choice, system), cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(SIZE.0), px(SIZE.1)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Komorebi gallery".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(layout::WINDOW_MIN.0), px(layout::WINDOW_MIN.1))),
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| Gallery::new(window, cx));
                if open.as_deref() == Some("dialog") {
                    window.defer(cx, open_dialog);
                }
                view
            });
            let handle = match opened {
                Ok((h, _)) => h,
                Err(e) => {
                    eprintln!("gallery: opening the window: {e:#}");
                    cx.quit();
                    return;
                }
            };
            cx.activate(true);
            let Some(path) = shot else { return };
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(SHOT_DELAY_MS)) // timer-exempt: the dev gallery waits for Kit's dialog to settle before its shot
                    .await;
                let image = handle.update(cx, |_, window, _| window.render_to_image());
                match image {
                    Ok(Ok(img)) => match img.save(&path) {
                        Ok(()) => println!("gallery: wrote {}", path.display()),
                        Err(e) => eprintln!("gallery: saving {}: {e}", path.display()),
                    },
                    Ok(Err(e)) => eprintln!("gallery: rendering: {e:#}"),
                    Err(e) => eprintln!("gallery: window gone: {e:#}"),
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    std::process::ExitCode::SUCCESS
}

fn open_dialog(window: &mut Window, cx: &mut App) {
    dialog::confirm(
        Confirm {
            title: "Delete 3 messages?".into(),
            message: "They move to Deleted. You can restore them from there.".into(),
            confirm: "Delete".into(),
            cancel: "Cancel".into(),
            destructive: true,
        },
        |_, _| {},
        window,
        cx,
    );
}
