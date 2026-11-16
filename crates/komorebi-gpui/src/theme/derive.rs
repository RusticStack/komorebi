//! The custom theme's rules: the secondary tokens derived from the main
//! four (accent, background, panes, text), and the legibility checks with
//! their one-click fix.
//!
//! **Derivation.** A user who sets only an accent and a background gets a
//! coherent theme: every other token follows from the panes (`paper`), the
//! text (`ink`) and the accent, in the proportions of the brand's own
//! palette (Komorebi light: ink-2 is 27 percent of the way from the ink to
//! the paper, ink-3 36 percent, wash 7 percent from the paper to the ink,
//! and so on), then nudged until the text passes WCAG AA on the panes, the
//! background and the wash. Only the tokens that depend on the colour that
//! changed are derived again, and never one the user set by hand.
//!
//! **Checks.** Text on the panes, the background and the wash, the muted
//! text, the text on the accent and on the ink need 4.5:1 (WCAG AA for
//! text); the accent as a control on the panes and the focus ring on its
//! gap need 3:1 (WCAG 1.4.11). A failing pair is a quiet warning in the
//! editor; "Fix contrast" moves the offending colour towards black or
//! white, the least that makes it pass. Saving is never blocked.

use super::presets::{ColorSet, Rgb, TokenId, contrast};

/// WCAG AA for text.
pub const TEXT_AA: f32 = 4.5;
/// WCAG 1.4.11 for controls and focus indicators.
pub const UI_AA: f32 = 3.0;

/// How far each derived token sits between the paper and the ink (0 the
/// paper, 1 the ink), from the brand's own palette.
fn share(id: TokenId, dark: bool) -> Option<f32> {
    Some(match id {
        TokenId::Ink2 => 0.28,
        TokenId::Ink3 => 0.40,
        TokenId::Inset => 0.04,
        TokenId::Wash => 0.065,
        TokenId::Fall => 0.045,
        TokenId::Line => 0.10,
        TokenId::Spot => 0.29,
        TokenId::SegmentOn if dark => 0.11,
        TokenId::PillOnInset if dark => 0.10,
        _ => return None,
    })
}

/// The tokens derived from `main` (the others depend on nothing or on more
/// than one main colour in the same way).
pub fn depends_on(id: TokenId, main: TokenId) -> bool {
    use TokenId as T;
    match id {
        T::OnAccent | T::Ring => main == T::Accent,
        T::RingGap => main == T::Paper,
        T::OnInk => main == T::Ink || main == T::Paper,
        T::Ink2 | T::Ink3 => matches!(main, T::Ink | T::Paper | T::Canvas),
        T::Inset | T::Wash | T::Line | T::Spot | T::Fall | T::SegmentOn | T::PillOnInset => {
            main == T::Ink || main == T::Paper
        }
        T::Accent | T::Canvas | T::Paper | T::Ink => false,
    }
}

/// White or near-black, whichever reads better on `ground` (white when it
/// passes, as Fluent's text on brand is white).
pub fn text_on(ground: Rgb) -> Rgb {
    let dark = Rgb(0x141414);
    if contrast(Rgb::WHITE, ground) >= TEXT_AA
        || contrast(Rgb::WHITE, ground) >= contrast(dark, ground)
    {
        Rgb::WHITE
    } else {
        dark
    }
}

/// One derived token's value from the main colours of `c`.
pub fn derived(id: TokenId, c: &ColorSet) -> Rgb {
    let (paper, ink, canvas, accent) = (
        c.get(TokenId::Paper),
        c.get(TokenId::Ink),
        c.get(TokenId::Canvas),
        c.get(TokenId::Accent),
    );
    let dark = !paper.is_light();
    match id {
        TokenId::Accent | TokenId::Canvas | TokenId::Paper | TokenId::Ink => c.get(id),
        TokenId::OnAccent => text_on(accent),
        TokenId::Ring => accent,
        TokenId::RingGap => paper,
        TokenId::OnInk => {
            if contrast(paper, ink) >= TEXT_AA {
                paper
            } else {
                text_on(ink)
            }
        }
        TokenId::SegmentOn | TokenId::PillOnInset if !dark => paper,
        TokenId::Ink2 | TokenId::Ink3 => {
            let v = paper.mix(ink, 1. - share(id, dark).unwrap_or(0.3));
            // The muted text passes on every ground text sits on.
            let grounds = [
                paper,
                canvas,
                paper.mix(ink, share(TokenId::Wash, dark).unwrap_or(0.)),
            ];
            raise(v, &grounds, TEXT_AA)
        }
        _ => paper.mix(ink, share(id, dark).unwrap_or(0.)),
    }
}

/// `c` with the tokens that depend on `main` derived again, except the
/// ones in `kept` (set by hand).
pub fn rederive(c: &ColorSet, main: TokenId, kept: &[TokenId]) -> ColorSet {
    let mut out = *c;
    for id in TokenId::ALL {
        if depends_on(id, main) && !kept.contains(&id) {
            out.set(id, derived(id, &out));
        }
    }
    out
}

/// Every secondary token derived from the main four of `c`.
pub fn derive_all(c: &ColorSet) -> ColorSet {
    let mut out = *c;
    for id in TokenId::ALL {
        if !id.is_main() {
            out.set(id, derived(id, &out));
        }
    }
    out
}

/// `fg` moved towards black or white (the side with more room) the least
/// that gives `need` against every one of `grounds`.
pub fn raise(fg: Rgb, grounds: &[Rgb], need: f32) -> Rgb {
    let worst = |c: Rgb| {
        grounds
            .iter()
            .map(|g| contrast(c, *g))
            .fold(f32::INFINITY, f32::min)
    };
    if worst(fg) >= need {
        return fg;
    }
    let to_black = worst(Rgb::BLACK);
    let to_white = worst(Rgb::WHITE);
    let target = if to_black >= to_white {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    };
    // 100 steps of 1 percent: the least move that passes.
    for step in 1..=100 {
        let c = fg.mix(target, step as f32 / 100.);
        if worst(c) >= need {
            return c;
        }
    }
    target
}

/// One legibility pair the editor checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pair {
    /// Text on the panes.
    TextOnPanes,
    /// Text on the background (the folder pane, the title bar).
    TextOnCanvas,
    /// Text on the selected row and secondary buttons.
    TextOnWash,
    /// Secondary text (read subjects, icons) on the panes.
    SecondaryOnPanes,
    /// Muted text (times, previews, captions) on the panes.
    MutedOnPanes,
    /// Muted text on the background.
    MutedOnCanvas,
    /// Text on the accent (the primary button).
    OnAccent,
    /// The accent as a control (toggle, selection marker) on the panes.
    AccentOnPanes,
    /// The focus ring on its gap.
    Ring,
    /// Text on ink (tooltips, notices).
    OnInk,
}

impl Pair {
    pub const ALL: [Pair; 10] = [
        Pair::TextOnPanes,
        Pair::TextOnCanvas,
        Pair::TextOnWash,
        Pair::SecondaryOnPanes,
        Pair::MutedOnPanes,
        Pair::MutedOnCanvas,
        Pair::OnAccent,
        Pair::AccentOnPanes,
        Pair::Ring,
        Pair::OnInk,
    ];

    /// The colour drawn on top (the one a fix moves) and its ground.
    pub fn tokens(self) -> (TokenId, TokenId) {
        use TokenId as T;
        match self {
            Pair::TextOnPanes => (T::Ink, T::Paper),
            Pair::TextOnCanvas => (T::Ink, T::Canvas),
            Pair::TextOnWash => (T::Ink, T::Wash),
            Pair::SecondaryOnPanes => (T::Ink2, T::Paper),
            Pair::MutedOnPanes => (T::Ink3, T::Paper),
            Pair::MutedOnCanvas => (T::Ink3, T::Canvas),
            Pair::OnAccent => (T::OnAccent, T::Accent),
            Pair::AccentOnPanes => (T::Accent, T::Paper),
            Pair::Ring => (T::Ring, T::RingGap),
            Pair::OnInk => (T::OnInk, T::Ink),
        }
    }

    /// The ratio the pair needs.
    pub fn need(self) -> f32 {
        match self {
            Pair::AccentOnPanes | Pair::Ring => UI_AA,
            _ => TEXT_AA,
        }
    }
}

/// A pair below its need.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Issue {
    pub pair: Pair,
    pub ratio: f32,
}

/// The pairs of `c` below WCAG AA.
pub fn check(c: &ColorSet) -> Vec<Issue> {
    Pair::ALL
        .into_iter()
        .filter_map(|pair| {
            let (fg, bg) = pair.tokens();
            let ratio = contrast(c.get(fg), c.get(bg));
            (ratio + 0.005 < pair.need()).then_some(Issue { pair, ratio })
        })
        .collect()
}

/// `c` with every failing pair's top colour nudged until it passes (each
/// against every ground it is checked on), and what was changed.
pub fn fix(c: &ColorSet) -> (ColorSet, Vec<TokenId>) {
    let mut out = *c;
    let mut moved = Vec::new();
    // Two rounds: moving the ink can move what the muted text sits on.
    for _ in 0..2 {
        for issue in check(&out) {
            let (fg, _) = issue.pair.tokens();
            let grounds: Vec<Rgb> = Pair::ALL
                .into_iter()
                .filter(|p| p.tokens().0 == fg && p.need() <= issue.pair.need())
                .map(|p| out.get(p.tokens().1))
                .collect();
            let next = raise(out.get(fg), &grounds, issue.pair.need());
            if next != out.get(fg) {
                out.set(fg, next);
                if !moved.contains(&fg) {
                    moved.push(fg);
                }
            }
        }
    }
    (out, moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::color::Scheme;
    use crate::theme::presets::Preset;

    #[test]
    fn every_preset_passes_every_check() {
        for preset in Preset::ALL {
            for scheme in [Scheme::Light, Scheme::Dark] {
                let issues = check(&preset.colors(scheme));
                assert!(issues.is_empty(), "{preset:?} {scheme:?}: {issues:?}");
            }
        }
    }

    /// Accent and background alone make a coherent theme: every check
    /// passes and the derived greys keep their order between paper and ink.
    #[test]
    fn accent_and_background_alone_derive_a_coherent_theme() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let base = Preset::Komorebi.colors(scheme);
            // A background of the variant's own lightness (a dark one under
            // light panes and text cannot pass with any one text colour).
            let pairs = match scheme {
                Scheme::Light => [
                    (0x8A3FFC, 0xF2EEF8),
                    (0x0F6CBD, 0xE8EEF6),
                    (0xE3B341, 0xFAFAF7),
                ],
                Scheme::Dark => [
                    (0x8A3FFC, 0x1B1B2F),
                    (0x0F6CBD, 0x101820),
                    (0x00A36C, 0x101010),
                ],
            };
            for (accent, canvas) in pairs {
                let mut c = base;
                c.set(TokenId::Accent, Rgb(accent));
                c = rederive(&c, TokenId::Accent, &[]);
                c.set(TokenId::Canvas, Rgb(canvas));
                c = rederive(&c, TokenId::Canvas, &[]);
                let (c, _) = fix(&c);
                assert!(
                    check(&c).is_empty(),
                    "{scheme:?} {accent:06x}: {:?}",
                    check(&c)
                );
                let to_ink = |id| contrast(c.get(id), c.get(TokenId::Paper));
                assert!(to_ink(TokenId::Ink) > to_ink(TokenId::Ink2));
                assert!(to_ink(TokenId::Wash) < to_ink(TokenId::Spot));
            }
        }
    }

    #[test]
    fn a_derived_ring_and_text_follow_the_accent() {
        let mut c = Preset::Komorebi.colors(Scheme::Light);
        c.set(TokenId::Accent, Rgb(0x0F6CBD));
        let c = rederive(&c, TokenId::Accent, &[]);
        assert_eq!(c.get(TokenId::Ring), Rgb(0x0F6CBD));
        assert_eq!(c.get(TokenId::OnAccent), Rgb::WHITE);
        // A light accent takes dark text.
        let mut d = c;
        d.set(TokenId::Accent, Rgb(0xFFD54F));
        let d = rederive(&d, TokenId::Accent, &[]);
        assert!(contrast(d.get(TokenId::OnAccent), Rgb(0xFFD54F)) >= TEXT_AA);
    }

    #[test]
    fn a_hand_set_token_is_kept_when_its_main_colour_changes() {
        let mut c = Preset::Komorebi.colors(Scheme::Light);
        c.set(TokenId::Ink3, Rgb(0x404040));
        c.set(TokenId::Paper, Rgb(0xFAF7F0));
        let kept = rederive(&c, TokenId::Paper, &[TokenId::Ink3]);
        assert_eq!(kept.get(TokenId::Ink3), Rgb(0x404040));
        assert_ne!(kept.get(TokenId::Wash), c.get(TokenId::Wash));
        let free = rederive(&c, TokenId::Paper, &[]);
        assert_ne!(free.get(TokenId::Ink3), Rgb(0x404040));
    }

    #[test]
    fn the_checks_find_and_fix_poor_contrast() {
        let mut c = Preset::Komorebi.colors(Scheme::Light);
        c.set(TokenId::Ink3, Rgb(0xC8C8C8));
        c.set(TokenId::Accent, Rgb(0xFFE680));
        c.set(TokenId::OnAccent, Rgb::WHITE);
        let pairs: Vec<Pair> = check(&c).into_iter().map(|i| i.pair).collect();
        assert!(pairs.contains(&Pair::MutedOnPanes));
        assert!(pairs.contains(&Pair::OnAccent));
        assert!(pairs.contains(&Pair::AccentOnPanes));
        let (fixed, moved) = fix(&c);
        assert!(check(&fixed).is_empty(), "{:?}", check(&fixed));
        assert!(moved.contains(&TokenId::Ink3));
        assert!(moved.contains(&TokenId::OnAccent));
        // The least move: just over the line, not black.
        let r = contrast(fixed.get(TokenId::Ink3), fixed.get(TokenId::Paper));
        assert!((TEXT_AA..TEXT_AA + 0.6).contains(&r), "{r}");
        // A passing set is left alone.
        let ok = Preset::Outlook.colors(Scheme::Dark);
        assert_eq!(fix(&ok), (ok, Vec::new()));
    }
}
