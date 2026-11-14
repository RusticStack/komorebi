//! The token layer (docs/GPUI-MIGRATION.md phase 1): one struct of semantic
//! tokens, [`Tokens`], built by one function, [`tokens`], from what
//! `komorebi::theme` reads of Windows and the theme setting. Views read it
//! with `cx.tokens()`; nothing outside `src/theme/` names a colour or a
//! size literal (`tests/token_lint.rs`).
//!
//! Inputs, as the brand decides them (IMPLEMENTATION.md section 4):
//! light or dark from the setting or Windows; Windows' text size scales the
//! type ramp; animation effects off turns motion off; the accent ramp,
//! accent title bars and high contrast are not read (high contrast is a
//! later [`color::Scheme`] variant fed by `SystemTheme::contrast`).
//!
//! [`install`] is the one writer: it stores the tokens as a GPUI global,
//! writes GPUI Kit's theme (`kit::apply`, Skua's contract: one
//! `Theme::change`, then one `Theme::update` with every write), sets GPUI's
//! reduce-motion flag and refreshes the windows. It runs at start and when
//! an input changes, and does nothing when the tokens come out the same.

pub mod color;
pub mod composer;
pub mod feeds;
pub mod kit;
pub mod metrics;
pub mod reader;
pub mod settings;
pub mod text;

use std::time::Duration;

use gpui::{App, Global};
use komorebi::db::ThemeChoice;
use komorebi::theme::SystemTheme;

pub use color::{Palette, Scheme};
pub use metrics::{layout, motion, radius, space};
pub use text::{TextStyled, TextToken, TypeRamp};

/// Everything the tokens are built from.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeInput {
    /// The app's theme setting (System, Light, Dark).
    pub choice: ThemeChoice,
    /// Windows' appearance settings (`komorebi::theme::read`).
    pub system: SystemTheme,
    /// `komorebi::theme::windows_build` (Windows 10 has no Segoe UI
    /// Variable).
    pub windows_build: u32,
}

impl ThemeInput {
    pub fn new(choice: ThemeChoice, system: SystemTheme) -> Self {
        Self {
            choice,
            system,
            windows_build: komorebi::theme::windows_build(),
        }
    }
}

/// The semantic tokens of one moment: colours, type, motion.
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub scheme: Scheme,
    pub color: Palette,
    pub text: TypeRamp,
    /// Windows' text size, 1.0 to 2.25.
    pub text_scale: f32,
    /// Windows' animation effects are off: every change is instant and the
    /// loader holds still.
    pub reduce_motion: bool,
}

impl Tokens {
    /// A motion duration, or zero under reduce motion.
    pub fn duration(&self, ms: u64) -> Duration {
        if self.reduce_motion {
            Duration::ZERO
        } else {
            Duration::from_millis(ms)
        }
    }
}

/// Light or dark: the setting, or Windows' app mode for System.
pub fn scheme_for(choice: ThemeChoice, system_dark: bool) -> Scheme {
    let dark = match choice {
        ThemeChoice::System => system_dark,
        ThemeChoice::Light => false,
        ThemeChoice::Dark => true,
    };
    if dark { Scheme::Dark } else { Scheme::Light }
}

/// The one function that builds the tokens.
pub fn tokens(input: &ThemeInput) -> Tokens {
    let scheme = scheme_for(input.choice, input.system.dark);
    let text_scale = input.system.text_scale.clamp(1.0, 2.25);
    Tokens {
        scheme,
        color: color::palette(scheme),
        text: text::ramp(text_scale, input.windows_build),
        text_scale,
        reduce_motion: !input.system.animate,
    }
}

/// The installed input and tokens.
struct ThemeState {
    input: ThemeInput,
    tokens: Tokens,
}

impl Global for ThemeState {}

/// `cx.tokens()` from any context.
pub trait ActiveTokens {
    fn tokens(&self) -> &Tokens;
    fn theme_input(&self) -> Option<&ThemeInput>;
}

/// Before [`install`] (only in tests that skip it) views get the light
/// tokens at 100 percent.
fn fallback() -> &'static Tokens {
    static FALLBACK: std::sync::OnceLock<Tokens> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(|| tokens(&ThemeInput::new(ThemeChoice::Light, SystemTheme::default())))
}

impl ActiveTokens for App {
    fn tokens(&self) -> &Tokens {
        match self.try_global::<ThemeState>() {
            Some(state) => &state.tokens,
            None => fallback(),
        }
    }

    fn theme_input(&self) -> Option<&ThemeInput> {
        self.try_global::<ThemeState>().map(|s| &s.input)
    }
}

/// Builds and installs the tokens for `input`: the global, GPUI Kit's
/// theme, reduce motion, and a refresh of every window. Returns whether
/// anything changed (an unchanged input costs nothing).
pub fn install(input: ThemeInput, cx: &mut App) -> bool {
    let next = tokens(&input);
    if let Some(state) = cx.try_global::<ThemeState>()
        && state.tokens == next
    {
        cx.global_mut::<ThemeState>().input = input;
        return false;
    }
    kit::apply(&next, cx);
    cx.set_reduce_motion(next.reduce_motion);
    // Native menus (the tray's) follow the app's light or dark (the Slint
    // app's `apply_theme`); never from a test, as it is process-wide.
    #[cfg(not(test))]
    komorebi::theme::set_menu_theme(next.scheme.is_dark());
    cx.set_global(ThemeState {
        input,
        tokens: next,
    });
    feeds::sync_window_frames(cx);
    cx.refresh_windows();
    true
}

/// The theme setting changed (Settings, or the cache's value at start).
pub fn set_choice(choice: ThemeChoice, cx: &mut App) -> bool {
    let input = cx
        .theme_input()
        .cloned()
        .unwrap_or_else(|| ThemeInput::new(choice, komorebi::theme::read()));
    install(ThemeInput { choice, ..input }, cx)
}

/// Windows' appearance settings changed (or may have: they are re-read).
pub fn system_changed(system: SystemTheme, cx: &mut App) -> bool {
    let input = cx
        .theme_input()
        .cloned()
        .unwrap_or_else(|| ThemeInput::new(ThemeChoice::System, system.clone()));
    install(ThemeInput { system, ..input }, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    fn system(dark: bool, text_scale: f32, animate: bool) -> SystemTheme {
        SystemTheme {
            dark,
            text_scale,
            animate,
            ..SystemTheme::default()
        }
    }

    #[test]
    fn the_theme_setting_wins_over_windows() {
        assert_eq!(scheme_for(ThemeChoice::System, true), Scheme::Dark);
        assert_eq!(scheme_for(ThemeChoice::System, false), Scheme::Light);
        assert_eq!(scheme_for(ThemeChoice::Light, true), Scheme::Light);
        assert_eq!(scheme_for(ThemeChoice::Dark, false), Scheme::Dark);
    }

    #[test]
    fn the_accent_ramp_and_contrast_are_ignored() {
        let plain = ThemeInput::new(ThemeChoice::System, system(false, 1.0, true));
        let mut accented = plain.clone();
        accented.system.ramp = [[0xe8, 0x11, 0x23]; 7];
        accented.system.prevalence = true;
        assert_eq!(tokens(&plain), tokens(&accented));
    }

    #[test]
    fn text_size_and_animation_effects_feed_the_tokens() {
        let t = tokens(&ThemeInput::new(
            ThemeChoice::System,
            system(true, 1.5, false),
        ));
        assert_eq!(t.scheme, Scheme::Dark);
        assert!(t.text.body.size.as_f32() > 14.);
        assert!(t.reduce_motion);
        assert_eq!(t.duration(motion::PANE_MS), Duration::ZERO);
        let t = tokens(&ThemeInput::new(
            ThemeChoice::Light,
            system(true, 9.0, true),
        ));
        assert_eq!(t.text_scale, 2.25, "clamped");
        assert_eq!(t.duration(motion::PANE_MS), Duration::from_millis(250));
    }

    #[gpui::test]
    fn install_writes_once_and_follows_changes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let input = ThemeInput::new(ThemeChoice::System, system(false, 1.0, true));
            assert!(install(input.clone(), cx));
            assert!(!install(input, cx), "same tokens: nothing to do");
            assert_eq!(cx.tokens().scheme, Scheme::Light);
            assert!(set_choice(ThemeChoice::Dark, cx));
            assert_eq!(cx.tokens().scheme, Scheme::Dark);
            // Windows going dark changes nothing while the setting forces it.
            assert!(!system_changed(system(true, 1.0, true), cx));
            assert!(system_changed(system(true, 1.0, false), cx));
            assert!(cx.reduce_motion());
            // Back to System: Windows is dark now, so nothing to redraw.
            assert!(!set_choice(ThemeChoice::System, cx));
            assert_eq!(cx.tokens().scheme, Scheme::Dark);
        });
    }
}
