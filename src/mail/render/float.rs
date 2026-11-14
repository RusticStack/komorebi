//! Floats (`align=left|right` tables and images, CSS `float`): placed
//! shrink-to-fit at the side of their container; following lines and
//! blocks are narrowed while a float is beside them.

use super::dom::Id;
use super::layout::{BoxInfo, Layouter, px_of, shift};
use super::style::{Float, VAlign};

/// Active floats of one block container: (bottom relative to the
/// container, outer width).
#[derive(Default)]
pub(super) struct Floats {
    left: Vec<(f32, f32)>,
    right: Vec<(f32, f32)>,
    /// The top of the last float placed: a later float may not start above
    /// it (CSS 2 9.5.1).
    top: f32,
}

impl Floats {
    /// Space floats take on the left and right at height `at`.
    fn taken(&self, at: f32) -> (f32, f32) {
        let sum = |v: &[(f32, f32)]| v.iter().filter(|f| f.0 > at).map(|f| f.1).sum::<f32>();
        (sum(&self.left), sum(&self.right))
    }

    /// The first height from `at` down where at least `need` of a container
    /// `w` wide is free of floats, and the space floats take on the left and
    /// right there. What does not fit beside the floats goes below them
    /// rather than over them.
    pub fn fit(&self, at: f32, w: f32, need: f32) -> (f32, f32, f32) {
        let mut y = at;
        loop {
            let (l, r) = self.taken(y);
            if w - l - r >= need || l + r <= 0.0 {
                return (y, l, r);
            }
            // Each step goes down to the next float bottom, so this ends.
            let next = self
                .left
                .iter()
                .chain(&self.right)
                .map(|f| f.0)
                .filter(|&b| b > y)
                .fold(f32::INFINITY, f32::min);
            if !next.is_finite() {
                return (y, 0.0, 0.0);
            }
            y = next;
        }
    }

    /// Where content (lines, blocks) goes from height `at` in a container
    /// `w` wide, and the space floats take beside it there. When floats
    /// leave less than a tenth, content goes below them instead of being
    /// squeezed.
    pub fn band(&self, at: f32, w: f32) -> (f32, f32, f32) {
        self.fit(at, w, w * 0.1)
    }

    /// Where a box that clears these sides (left, right) can start.
    pub fn cleared(&self, (left, right): (bool, bool)) -> f32 {
        let bottom = |on: bool, v: &[(f32, f32)]| {
            if on {
                v.iter().map(|f| f.0).fold(0.0, f32::max)
            } else {
                0.0
            }
        };
        bottom(left, &self.left).max(bottom(right, &self.right))
    }

    pub fn bottom(&self) -> f32 {
        self.left
            .iter()
            .chain(&self.right)
            .map(|f| f.0)
            .fold(0.0, f32::max)
    }
}

impl Layouter<'_> {
    /// Lay out float `c` at relative height `cy` (absolute `y`) of a
    /// container at `x`, `w` wide. A float too wide for the room the floats
    /// before it leave goes down until it fits (CSS 2 9.5.1), so that two
    /// `width=100%` floats stack instead of being drawn over each other.
    pub fn place_float(&mut self, c: Id, floats: &mut Floats, x: f32, y: f32, cy: f32, w: f32) {
        let base = y - cy;
        let (mut at, mut l, mut rr) = floats.band(cy.max(floats.top), w);
        let avail = (w - l - rr).max(0.0);
        let mut bi = self.float_box(c, avail);
        if bi.w > avail + 0.5 {
            let (at2, l2, r2) = floats.fit(at, w, bi.w);
            if at2 > at {
                (at, l, rr) = (at2, l2, r2);
                let room = (w - l - rr).max(0.0);
                // Lower down there is more room: its width may depend on it.
                if room > avail + 0.5 {
                    bi = self.float_box(c, room);
                }
            }
        }
        let right = self.style(c).float == Float::Right;
        let fx = if right { x + w - rr - bi.w } else { x + l };
        shift(&mut bi.items, fx, base + at);
        self.out.append(&mut bi.items);
        let side = if right {
            &mut floats.right
        } else {
            &mut floats.left
        };
        side.push((at + bi.h, bi.w));
        floats.top = at;
    }

    /// A float laid out shrink-to-fit at (0, 0); items are relative.
    fn float_box(&mut self, c: Id, avail: f32) -> BoxInfo {
        let s = self.style(c).clone();
        let (ml, mr, mt, mb) = (
            px_of(s.margin[3]),
            px_of(s.margin[1]),
            px_of(s.margin[0]),
            px_of(s.margin[2]),
        );
        let empty = || BoxInfo {
            w: 0.0,
            h: 0.0,
            items: vec![],
            va: VAlign::Baseline,
            deco: 0,
        };
        if self.dom.tag(c) == "img" {
            let (mut ops, mut boxes, mut vis) = (vec![], vec![], false);
            self.image_box(c, 0, 0, avail, &mut ops, &mut boxes, &mut vis);
            // The image's box holds its margins already.
            let Some(b) = boxes.pop() else {
                return empty();
            };
            return BoxInfo {
                va: VAlign::Baseline,
                deco: 0,
                ..b
            };
        }
        let saved = std::mem::take(&mut self.out);
        // The float's own margins are applied here.
        let r = self.box_at_nomargin(c, ml, mt, avail);
        let items = std::mem::replace(&mut self.out, saved);
        BoxInfo {
            w: ml + r.w + mr,
            h: mt + r.h + mb,
            items,
            va: VAlign::Baseline,
            deco: 0,
        }
    }
}
