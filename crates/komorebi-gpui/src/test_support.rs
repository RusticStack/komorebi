//! What the GPUI app's tests share (docs/GPUI-TESTING.md): an app context
//! set up as `main` sets it up, headless Kit windows, a throwaway cache
//! seeded by `komorebi::seed`, and queries by accessible name and role (what
//! UI Automation, and so Narrator, reads). Only GPUI and Kit test APIs from
//! Skua's cookbook (cookbook_test, cookbook_a11y, cookbook_actions,
//! cookbook_window_state) plus `Window::dispatch_action`.
//!
//! No timers or sleeps: move GPUI's test clock (`cx.executor()
//! .advance_clock(d)`, then `cx.run_until_parked()`); real I/O that wakes
//! GPUI tasks from other threads needs `cx.executor().allow_parking()`.

use gpui::{
    Action, AnyWindowHandle, App, AppContext as _, Bounds, Entity, MouseButton, Render, Role,
    TestAppContext, Window, WindowBounds, WindowOptions, point, px, size,
};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use komorebi::db::{Db, ThemeChoice};
use komorebi::i18n::Lang;
use komorebi::seed::{self, Seeded};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::theme::{self, ThemeInput};
use crate::window::{DEFAULT_SIZE, Init, MainWindow};

/// The size test windows open at: the app's default window.
pub const WINDOW_SIZE: (f32, f32) = DEFAULT_SIZE;

/// Windows as the tests see it: light, 100 percent text, animation effects
/// off (Kit animations at rest).
fn test_system() -> komorebi::theme::SystemTheme {
    komorebi::theme::SystemTheme {
        animate: false,
        ..Default::default()
    }
}

/// `gpui_kit::init`, English (crate::i18n), the light tokens and reduce
/// motion (crate::theme), and the app's key bindings, as `main` does before
/// it opens a window.
pub fn init_app(cx: &mut TestAppContext) {
    // The main window reads Windows' settings again when it becomes active
    // (`theme::feeds::refresh_from_windows`): a test sees the same ones
    // everywhere, never this machine's.
    komorebi::theme::pin(test_system());
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::i18n::init(Lang::En, cx);
        theme::install(ThemeInput::new(ThemeChoice::Light, test_system()), cx);
        cx.set_reduce_motion(true);
        crate::tabs::bind_keys(cx);
        crate::views::reader::bind_keys(cx);
        crate::views::composer::bind_keys(cx);
        crate::views::settings::bind_keys(cx);
    });
}

/// Shows the app in `lang` from here on (the global and `tr!`'s value for
/// this test thread), as start-up does with the saved choice.
pub fn set_lang(cx: &mut TestAppContext, lang: Lang) {
    cx.update(|cx| crate::i18n::init(lang, cx));
}

/// Draws the app with the tokens of `choice` (System counts as light).
pub fn set_theme(cx: &mut TestAppContext, choice: ThemeChoice) {
    cx.update(|cx| {
        theme::install(ThemeInput::new(choice, test_system()), cx);
    });
}

/// What the main window needs from start-up, for a test: `db` if any. The
/// language and the theme are the app's ([`init_app`], [`set_lang`],
/// [`set_theme`]).
pub fn init(db: Option<Db>) -> Init {
    Init {
        db,
        timing: None,
        exit_after_first_frame: false,
        snapshot: None,
        exit_code: Arc::new(AtomicU8::new(0)),
        start: Default::default(),
    }
}

/// No cache (English and light after [`init_app`]).
pub fn plain_init() -> Init {
    init(None)
}

/// Opens `build` in a headless Kit window (with the Root that draws
/// dialogs, notifications and tooltips) at [`WINDOW_SIZE`]. Call
/// [`init_app`] first. Test windows are headless (no HWND, nothing on
/// screen) and go with the test's app context.
pub fn open<V: Render>(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> (AnyWindowHandle, Entity<V>) {
    cx.update(|cx| {
        let bounds = Bounds::new(
            point(px(0.), px(0.)),
            size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)),
        );
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            build,
        )
    })
    .expect("open the test window")
}

/// The main window in a headless Kit window.
pub fn open_main(cx: &mut TestAppContext, init: Init) -> (AnyWindowHandle, Entity<MainWindow>) {
    open(cx, move |window, cx| {
        cx.new(|cx| MainWindow::new(init, window, cx))
    })
}

/// Runs `f` inside the window after a fresh frame (queries read the last
/// completed frame).
pub fn in_window<R>(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx)
    })
    .expect("the test window is open")
}

/// Every element UI Automation would name in the last frame: its role and
/// name, in screen order (top to bottom, then left to right).
pub fn names(window: &Window) -> Vec<(Option<Role>, String)> {
    window
        .elements()
        .into_iter()
        .filter_map(|e| Some((e.role(), e.label()?.to_owned())))
        .collect()
}

/// Asserts that exactly one element is named `name`, has `role` and is
/// visible.
pub fn assert_named(window: &Window, name: &str, role: Role) {
    let found = window.find_by_label(name);
    assert_eq!(found.role(), Some(role), "the role of {name:?}");
    assert!(found.visible(), "{name:?} is not visible");
}

/// A left click at the centre of the element named `name` (for elements
/// with no stable id, such as popup items; by id, use `window.click(id)`).
pub fn click_named(window: &mut Window, name: &str, cx: &mut App) {
    window.render_frame(cx);
    let centre = window.find_by_label(name).bounds().center();
    window.pointer_move(centre, None, cx);
    window.pointer_down(centre, MouseButton::Left, cx);
    window.pointer_up(centre, MouseButton::Left, cx);
    window.render_frame(cx);
}

/// Presses each key in turn ("ctrl-tab", "shift-f6", "enter"): bindings
/// first, as on Windows, then the focused element's key handlers.
pub fn press(window: &mut Window, keys: &[&str], cx: &mut App) {
    for key in keys {
        window.press(key, cx);
    }
}

/// Types `text` into the focused element, one character at a time.
pub fn type_text(window: &mut Window, text: &str, cx: &mut App) {
    window.input(text, cx);
}

/// Dispatches `action` from the focused element, as its key would.
pub fn dispatch(window: &mut Window, action: &dyn Action, cx: &mut App) {
    window.dispatch_action(action.boxed_clone(), cx);
    window.render_frame(cx);
}

/// A throwaway cache in its own temp folder, removed on drop (the cache,
/// its WAL and the drafts folder with it).
pub struct TempCache {
    dir: PathBuf,
    pub path: PathBuf,
}

/// A folder name no other test in this process can share: the process id,
/// a counter that only goes up, and `name` for whoever reads a leftover
/// folder. Two tests (or two merged lanes) that pick the same `name` get
/// different folders, so one's drop never removes the other's cache.
pub fn unique_dir(prefix: &str, name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{}-{n}-{name}", std::process::id()))
}

impl TempCache {
    /// A cache in a folder of its own: `name` only labels the folder, it
    /// does not identify it ([`unique_dir`]).
    pub fn new(name: &str) -> Self {
        let dir = unique_dir("komorebi-gpui-test", name);
        std::fs::create_dir_all(&dir).expect("create the test cache folder");
        let path = dir.join("cache.sqlite3");
        TempCache { dir, path }
    }

    /// The cache opened (and created) at [`TempCache::path`].
    pub fn open(&self) -> Db {
        Db::open(&self.path).expect("open the test cache")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempCache {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A throwaway cache with the seed account and `count` generated messages
/// (`komorebi::seed`, the data of `cargo run --example seed`), in English.
pub fn seeded(name: &str, count: u32) -> (TempCache, Db, Seeded) {
    let cache = TempCache::new(name);
    let mut db = cache.open();
    let made = seed::seed(&mut db, count).expect("seed the test cache");
    db.set_setting("language", "en").expect("set the language");
    (cache, db, made)
}

/// The harness checks itself: a field and a button inside the main
/// window's key context, as the composer's fields will be.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::{self, NextTab};
    use gpui::{
        Context, FocusHandle, Focusable, IntoElement, ParentElement, Styled, div, prelude::*,
    };
    use gpui_kit::component::button::Button;
    use gpui_kit::component::input::{Input, InputState};

    struct Form {
        focus: FocusHandle,
        name: Entity<InputState>,
        saved: Vec<String>,
        next_tab: usize,
    }

    impl Form {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Form {
                focus: cx.focus_handle(),
                name: cx.new(|cx| InputState::new(window, cx)),
                saved: Vec::new(),
                next_tab: 0,
            }
        }
    }

    impl Focusable for Form {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context(tabs::CONTEXT)
                .track_focus(&self.focus)
                .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                    this.next_tab += 1;
                    cx.notify();
                }))
                .size_full()
                .flex()
                .flex_col()
                .child(Input::new(&self.name).aria_label("Name"))
                .child(
                    Button::new("save")
                        .label("Save")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.saved.push(this.name.read(cx).value().to_string());
                            cx.notify();
                        })),
                )
        }
    }

    #[gpui_kit::test]
    fn names_clicks_typing_and_keys_reach_the_view(cx: &mut TestAppContext) {
        init_app(cx);
        let (window, form) = open(cx, |window, cx| cx.new(|cx| Form::new(window, cx)));
        let name = form.read_with(cx, |f, _| f.name.clone());
        in_window(cx, window, |window, cx| {
            assert_named(window, "Name", Role::TextInput);
            assert_named(window, "Save", Role::Button);
            assert!(names(window).contains(&(Some(Role::Button), "Save".to_owned())));
            let focus = name.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            type_text(window, "Ana", cx);
            // A window key wins over the focused field (cookbook_actions).
            press(window, &["ctrl-tab"], cx);
            click_named(window, "Save", cx);
        });
        form.read_with(cx, |f, _| {
            assert_eq!(f.saved, ["Ana"]);
            assert_eq!(f.next_tab, 1);
        });
    }

    /// The main window reads Windows' settings again when it becomes active
    /// (`theme::feeds::refresh_from_windows`). A test must get its own, not
    /// this machine's: with animation effects on, Kit's dialogs slide in
    /// over real time and a click lands on a moving button, only under load
    /// (`views::mail::tests::deleting_from_trash_or_junk_asks_first`).
    #[gpui_kit::test]
    fn a_window_reading_windows_again_keeps_the_tests_settings(cx: &mut TestAppContext) {
        init_app(cx);
        let (window, _) = open_main(cx, plain_init());
        in_window(cx, window, |_, cx| {
            crate::theme::feeds::refresh_from_windows(cx);
            assert!(cx.reduce_motion(), "animation effects stay off");
        });
        assert!(!komorebi::theme::read().animate);
    }

    /// Two caches given the same name (as two merged lanes once did with
    /// "tab-commands" and "tab-flush") are two folders: dropping one leaves
    /// the other's cache in place.
    #[test]
    fn a_name_shared_by_two_tests_is_two_caches() {
        let (first, first_db, _) = seeded("shared", 1);
        let second = TempCache::new("shared");
        assert_ne!(first.dir(), second.dir());
        assert_ne!(first.path, second.path);
        let (third, ..) = seeded("shared", 1);
        assert_ne!(first.dir(), third.dir());
        assert_ne!(second.dir(), third.dir());
        drop(second);
        assert!(first.path.exists(), "the first is untouched");
        assert_eq!(first_db.accounts().unwrap().len(), 1);
    }

    #[test]
    fn a_seeded_cache_goes_with_its_folder() {
        let (cache, db, seeded) = seeded("selftest", 5);
        assert_eq!(db.message_ids(&[seeded.inbox], "").unwrap().len(), 5);
        assert_eq!(db.settings().unwrap().language.key(), "en");
        let dir = cache.dir().to_path_buf();
        assert!(cache.path.starts_with(&dir));
        drop(db);
        drop(cache);
        assert!(!dir.exists());
    }
}
