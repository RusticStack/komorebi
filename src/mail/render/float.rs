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
}

impl Floats {
    /// Space taken on the left and right at height `at` in a container `w`
    /// wide. When floats leave less than a tenth, content goes below them
    /// instead of being squeezed.
    pub fn band(&self, at: f32, w: f32) -> (f32, f32) {
        let sum = |v: &[(f32, f32)]| v.iter().filter(|f| f.0 > at).map(|f| f.1).sum::<f32>();
        let (l, r) = (sum(&self.left), sum(&self.right));
        if l + r > w * 0.9 { (0.0, 0.0) } else { (l, r) }
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
    /// container at `x`, `w` wide.
    pub fn place_float(&mut self, c: Id, floats: &mut Floats, x: f32, y: f32, cy: f32, w: f32) {
        let (l, rr) = floats.band(cy, w);
        let mut bi = self.float_box(c, (w - l - rr).max(0.0));
        let right = self.style(c).float == Float::Right;
        let fx = if right { x + w - rr - bi.w } else { x + l };
        shift(&mut bi.items, fx, y);
        self.out.append(&mut bi.items);
        let side = if right {
            &mut floats.right
        } else {
            &mut floats.left
        };
        side.push((cy + bi.h, bi.w));
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
        };
        if self.dom.tag(c) == "img" {
            let (mut ops, mut boxes, mut vis) = (vec![], vec![], false);
            self.image_box(c, 0, avail, &mut ops, &mut boxes, &mut vis);
            let Some(mut b) = boxes.pop() else {
                return empty();
            };
            shift(&mut b.items, ml, mt);
            return BoxInfo {
                w: b.w + ml + mr,
                h: b.h + mt + mb,
                items: b.items,
                va: VAlign::Baseline,
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
        }
    }
}
