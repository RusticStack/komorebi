//! The Rest scene's drawn greys (`views/mail/scene.rs`, `sky.rs`): the
//! sky's stars and clouds, the sun or moon, the water's glimmer and the path on it,
//! the fireflies, the lantern, the birds. Neutral greys only, from the same
//! range as the scene's own art (light: about #2E to #EE over the paper,
//! dark: about #0E to #F0), never a hue. The scheme picks the palette; the
//! clock picks the body.

use gpui::{Hsla, Rgba};

use super::Scheme;

/// A grey `v` (0 to 255) at opacity `a` (also for a tone read from the
/// scene's own art).
pub fn grey(v: u8, a: f32) -> Hsla {
    let c = f32::from(v) / 255.;
    Rgba {
        r: c,
        g: c,
        b: c,
        a,
    }
    .into()
}

/// One body (sun or moon) in one scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyTone {
    /// The lit disc.
    pub disc: Hsla,
    /// The craters (the moon's).
    pub crater: Hsla,
    /// The halo's colour at its strongest (its opacity is the most it
    /// reaches next to the disc).
    pub halo: Hsla,
    /// How far the halo falls off, in art pixels.
    pub halo_reach: i32,
    /// The path on the water at its strongest.
    pub path: Hsla,
}

/// The scene's drawn greys for a scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneTones {
    pub sun: BodyTone,
    pub moon: BodyTone,
    /// The moon's dark limb: the sky (the panel's paper) showing through.
    pub limb: Hsla,
    /// A star at full light (fainter ones take less opacity).
    pub star: Hsla,
    /// A cloud streak at its strongest.
    pub cloud: Hsla,
    /// A water streak at its strongest.
    pub water: Hsla,
    /// The day's mist over the far shore: lighter than the ridges in the
    /// light scheme, a little lighter than them in the dark one.
    pub mist: Hsla,
    /// A firefly's pixel and its faint glow: lighter than the dark
    /// scheme's ground, darker than the light scheme's.
    pub firefly: Hsla,
    pub firefly_glow: Hsla,
    /// The lit lantern's lamp and its glow at its strongest.
    pub lamp: Hsla,
    pub lamp_glow: Hsla,
    /// A distant bird.
    pub bird: Hsla,
}

pub fn tones(scheme: Scheme) -> SceneTones {
    match scheme {
        // Over white paper: a soft grey sun, a pale moon, dark fireflies.
        Scheme::Light => SceneTones {
            sun: BodyTone {
                disc: grey(0xDC, 1.),
                crater: grey(0xDC, 1.),
                halo: grey(0xD4, 0.42),
                halo_reach: 30,
                path: grey(0xA0, 0.95),
            },
            moon: BodyTone {
                disc: grey(0xE0, 1.),
                crater: grey(0xCC, 1.),
                halo: grey(0xEC, 0.5),
                halo_reach: 14,
                path: grey(0xA8, 0.9),
            },
            limb: grey(0xFF, 1.),
            star: grey(0xC8, 1.),
            cloud: grey(0xE6, 1.),
            water: grey(0xDA, 1.),
            mist: grey(0xF4, 1.),
            firefly: grey(0x3C, 1.),
            firefly_glow: grey(0x8C, 0.3),
            lamp: grey(0x4A, 1.),
            lamp_glow: grey(0xA8, 0.3),
            bird: grey(0x86, 1.),
        },
        // Over the dark paper: a bright moon, a dim sun, light fireflies.
        Scheme::Dark => SceneTones {
            sun: BodyTone {
                disc: grey(0x5E, 1.),
                crater: grey(0x5E, 1.),
                halo: grey(0x3E, 0.5),
                halo_reach: 30,
                path: grey(0x62, 0.9),
            },
            moon: BodyTone {
                disc: grey(0xE6, 1.),
                crater: grey(0xC4, 1.),
                halo: grey(0xB4, 0.26),
                halo_reach: 18,
                path: grey(0xD2, 0.9),
            },
            limb: grey(0x1A, 1.),
            star: grey(0xE0, 1.),
            cloud: grey(0x2A, 1.),
            water: grey(0x34, 1.),
            mist: grey(0x44, 1.),
            firefly: grey(0xF0, 1.),
            firefly_glow: grey(0xC8, 0.42),
            lamp: grey(0xF0, 1.),
            lamp_glow: grey(0xD8, 0.32),
            bird: grey(0x5A, 1.),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::color::palette;

    /// Greys only, never a hue; the moon's limb is the panel's paper.
    #[test]
    fn the_scene_tones_are_grey() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let t = tones(scheme);
            for c in [
                t.sun.disc,
                t.sun.halo,
                t.sun.path,
                t.moon.disc,
                t.moon.crater,
                t.moon.halo,
                t.moon.path,
                t.limb,
                t.star,
                t.cloud,
                t.water,
                t.mist,
                t.firefly,
                t.firefly_glow,
                t.lamp,
                t.lamp_glow,
                t.bird,
            ] {
                assert_eq!(c.s, 0., "{scheme:?}: {c:?} is a grey");
            }
            let paper = Rgba::from(palette(scheme).paper);
            let limb = Rgba::from(t.limb);
            assert!((paper.r - limb.r).abs() < 0.01, "{scheme:?}");
        }
    }
}
