//! The type ramp (BRAND.md "Type", IMPLEMENTATION.md `type_scale.rs`),
//! scaled by Windows' text size the way WinUI scales it
//! (`komorebi::theme::scaled`: small text grows the most). Segoe UI
//! Variable on Windows 11 (Text below 20 px, Display from 20 px), Segoe UI
//! on Windows 10. No UI font is bundled: Segoe may not be redistributed,
//! and DirectWrite has it on every supported system.

use gpui::{FontWeight, Pixels, SharedString, Styled, px};

/// The Windows 11 text face (below 20 px).
pub const FAMILY_TEXT: &str = "Segoe UI Variable Text";
/// The Windows 11 display face (20 px and up).
pub const FAMILY_DISPLAY: &str = "Segoe UI Variable Display";
/// Windows 10.
pub const FAMILY_FALLBACK: &str = "Segoe UI";
/// From this size up the display face is used.
const DISPLAY_FROM: f32 = 20.;
/// The first Windows 11 build.
const WINDOWS_11: u32 = 22000;

/// One step of the ramp, scaled and resolved to a family.
#[derive(Debug, Clone, PartialEq)]
pub struct TextToken {
    pub family: SharedString,
    pub size: Pixels,
    pub line: Pixels,
    pub weight: FontWeight,
}

/// A step at 100 percent text size: (size, line height, weight).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step(pub f32, pub f32, pub FontWeight);

pub const CAPTION: Step = Step(12., 16., FontWeight::NORMAL);
/// Fluent's Caption 1 Strong: a count of unread mail.
pub const CAPTION_STRONG: Step = Step(12., 16., FontWeight::SEMIBOLD);
pub const BODY: Step = Step(14., 20., FontWeight::NORMAL);
pub const BODY_STRONG: Step = Step(14., 20., FontWeight::SEMIBOLD);
pub const PREVIEW: Step = Step(13., 18., FontWeight::NORMAL);
pub const MESSAGE: Step = Step(15., 24., FontWeight::NORMAL);
pub const SUBTITLE: Step = Step(20., 28., FontWeight::SEMIBOLD);
pub const TITLE: Step = Step(24., 32., FontWeight::SEMIBOLD);
pub const LARGE: Step = Step(28., 36., FontWeight::SEMIBOLD);
pub const DISPLAY: Step = Step(40., 48., FontWeight::SEMIBOLD);

/// The whole ramp at one text size.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeRamp {
    pub caption: TextToken,
    pub caption_strong: TextToken,
    pub body: TextToken,
    pub body_strong: TextToken,
    pub preview: TextToken,
    pub message: TextToken,
    pub subtitle: TextToken,
    pub title: TextToken,
    pub large: TextToken,
    pub display: TextToken,
    /// The face Kit's components use (`Theme::font_family`).
    pub ui_family: SharedString,
}

/// Is this Windows 11 (Segoe UI Variable present)?
pub fn has_variable_face(windows_build: u32) -> bool {
    windows_build >= WINDOWS_11
}

/// One step at `factor` (Windows' text size, 1.0 to 2.25).
pub fn token(step: Step, factor: f32, variable: bool) -> TextToken {
    let Step(size, line, weight) = step;
    let scaled = komorebi::theme::scaled(size, factor);
    let family = match (variable, size >= DISPLAY_FROM) {
        (false, _) => FAMILY_FALLBACK,
        (true, false) => FAMILY_TEXT,
        (true, true) => FAMILY_DISPLAY,
    };
    TextToken {
        family: family.into(),
        size: px(scaled),
        // The line keeps its proportion to the size, on whole pixels.
        line: px((line * scaled / size).round()),
        weight,
    }
}

/// The ramp for a text size and a Windows build.
pub fn ramp(factor: f32, windows_build: u32) -> TypeRamp {
    let v = has_variable_face(windows_build);
    let t = |step| token(step, factor, v);
    TypeRamp {
        caption: t(CAPTION),
        caption_strong: t(CAPTION_STRONG),
        body: t(BODY),
        body_strong: t(BODY_STRONG),
        preview: t(PREVIEW),
        message: t(MESSAGE),
        subtitle: t(SUBTITLE),
        title: t(TITLE),
        large: t(LARGE),
        display: t(DISPLAY),
        ui_family: if v { FAMILY_TEXT } else { FAMILY_FALLBACK }.into(),
    }
}

/// `.typo(&t.text.body)` on any styled element: family, size, line height
/// and weight in one call.
pub trait TextStyled: Styled + Sized {
    fn typo(self, token: &TextToken) -> Self {
        self.font_family(token.family.clone())
            .text_size(token.size)
            .line_height(token.line)
            .font_weight(token.weight)
    }
}

impl<T: Styled> TextStyled for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_100_percent_the_ramp_is_the_brand_ramp() {
        let r = ramp(1.0, 26300);
        assert_eq!((r.caption.size, r.caption.line), (px(12.), px(16.)));
        assert_eq!((r.body.size, r.body.line), (px(14.), px(20.)));
        assert_eq!(r.body_strong.weight, FontWeight::SEMIBOLD);
        assert_eq!((r.message.size, r.message.line), (px(15.), px(24.)));
        assert_eq!((r.display.size, r.display.line), (px(40.), px(48.)));
        assert_eq!(r.body.family.as_ref(), FAMILY_TEXT);
        assert_eq!(r.subtitle.family.as_ref(), FAMILY_DISPLAY);
    }

    #[test]
    fn windows_10_uses_segoe_ui() {
        let r = ramp(1.0, 19045);
        assert_eq!(r.body.family.as_ref(), FAMILY_FALLBACK);
        assert_eq!(r.display.family.as_ref(), FAMILY_FALLBACK);
        assert_eq!(r.ui_family.as_ref(), FAMILY_FALLBACK);
    }

    #[test]
    fn text_size_grows_small_text_most_and_keeps_line_proportion() {
        let r = ramp(2.25, 26300);
        assert!((r.caption.size.as_f32() - 26.1).abs() < 0.05);
        assert!(r.caption.size.as_f32() / 12. > r.display.size.as_f32() / 40.);
        // 16/12 of the size, rounded.
        assert_eq!(
            r.caption.line,
            px((16. * r.caption.size.as_f32() / 12.).round())
        );
    }
}
