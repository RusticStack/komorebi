//! The reader's tokens (mockup `.rd`, `.hdr`, `.msg`, `.att` in app.html;
//! IMPLEMENTATION.md section 3 "Inbox, message open"): the reading column's
//! padding, the header's spacing, the message text's measure, the
//! attachment pills, and the marks drawn over a page of pixels (the
//! selection and the focused link). Colours come from the palette; this
//! file adds only how strongly they are laid over the page.

use gpui::Hsla;

use super::color::Palette;
use super::space;

/// The reading column's padding: 20 px above, 40 px at the sides, 28 px
/// below (`.rd { padding: 20px 40px 28px }`).
pub const PAD_TOP: f32 = space::SP_5;
pub const PAD_X: f32 = space::SP_10;
pub const PAD_BOTTOM: f32 = space::SP_7;
/// Under 560 px the sides take less (FLUENT2 "Layout and spacing").
pub const PAD_X_NARROW: f32 = space::SP_5;
pub const NARROW_BELOW: f32 = 560.;
/// The header: 20 px under the subject, 24 px above the body, 14 px
/// between the sender spot and the names (`.hdr`).
pub const HEADER_TOP: f32 = space::SP_5;
pub const HEADER_BOTTOM: f32 = space::SP_6;
pub const HEADER_GAP: f32 = space::SP_3_5;
/// The plain text's measure (`.msg { max-width: 640px }`).
pub const TEXT_MAX_W: f32 = 640.;
/// Between paragraphs of plain text (`.msg p + p { margin-top: 14px }`).
pub const PARAGRAPH_GAP: f32 = space::SP_3_5;
/// Attachment pills (`.att`): 40 px, 22 px above, 8 px apart, 180 to 280
/// px wide as the Slint reader lays them out.
pub const ATTACHMENT_H: f32 = 40.;
pub const ATTACHMENT_TOP: f32 = 22.;
pub const ATTACHMENT_GAP: f32 = space::SP_2;
pub const ATTACHMENT_MIN_W: f32 = 180.;
pub const ATTACHMENT_MAX_W: f32 = 280.;
/// Notices and the invite card: 12 px apart in the column.
pub const BLOCK_GAP: f32 = space::SP_3;
/// The page is centred on a 4 px grid, so tiles stay on whole device
/// pixels at 125, 150 and 175 percent.
pub const PAGE_GRID: f32 = 4.;
/// The focused link's ring: 2 px of ink, 2 px corners.
pub const LINK_RING: f32 = 2.;
pub const LINK_RING_RADIUS: f32 = 2.;
/// The link target strip: 8 px in from the reader's bottom left corner.
pub const TARGET_INSET: f32 = space::SP_2;
/// The strip's widest (a long address is cut with an ellipsis).
pub const TARGET_MAX_W: f32 = 640.;
/// The struck line through a cancelled event's title, from its top.
pub const STRIKE_Y: f32 = 11.;

/// What Up and Down scroll the reader by: three lines of message text.
pub const SCROLL_LINE: f32 = 60.;

/// The Show original dialog: wider than a question (the header fields
/// and the source are long lines); its text scrolls under this height.
pub const SOURCE_DIALOG_W: f32 = 640.;
pub const SOURCE_DIALOG_MAX_H: f32 = 420.;

/// How strongly the selection's ink lies over the page: a grey wash that
/// keeps the text readable on light and dark pages. The brand's
/// selection is wash; over pixels the wash itself would hide the text, so
/// the highlight is ink at this strength.
const SELECTION_ALPHA: f32 = 0.16;

/// A page's own background (the message's colour, RGBA; opaque), drawn
/// where a tile is not in yet.
pub fn page_colour([r, g, b, _]: [u8; 4]) -> Hsla {
    gpui::Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    }
    .into()
}

/// The selection highlight over a page.
pub fn selection(p: &Palette) -> Hsla {
    p.ink.opacity(SELECTION_ALPHA)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::color::{Scheme, palette};

    #[test]
    fn the_selection_is_a_light_ink_wash_in_both_schemes() {
        for scheme in [Scheme::Light, Scheme::Dark] {
            let p = palette(scheme);
            let s = selection(&p);
            assert_eq!((s.h, s.s, s.l), (p.ink.h, p.ink.s, p.ink.l));
            assert!(s.a > 0.1 && s.a < 0.3);
        }
    }

    #[test]
    fn the_column_is_on_the_4_px_grid() {
        for v in [
            PAD_TOP,
            PAD_X,
            PAD_BOTTOM,
            HEADER_TOP,
            HEADER_BOTTOM,
            ATTACHMENT_H,
        ] {
            assert_eq!(v % 4., 0., "{v}");
        }
    }
}
