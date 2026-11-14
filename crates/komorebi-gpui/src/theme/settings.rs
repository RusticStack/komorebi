//! Settings' and onboarding's geometry (mockups `settings` and `welcome`,
//! komorebi-brand `final/app/app.html` `.set`, `.setc`, `.seg`, `.wel`;
//! phases 6 and 7). Logical px on the 4 px grid where the mockup is.

/// The settings column: 20 px above, 32 px at the sides (`.setc`).
pub const PAGE_PAD_Y: f32 = 20.;
pub const PAGE_PAD_X: f32 = 32.;
/// The compact window's narrower column.
pub const PAGE_PAD_X_COMPACT: f32 = 16.;
/// The section list's width in a compact window (`--setnav-w`).
pub const SETNAV_W_COMPACT: f32 = 176.;
/// A group heading: 28 px above, 8 px under (`.sg`).
pub const GROUP_TOP: f32 = 28.;
pub const GROUP_BOTTOM: f32 = 8.;
/// The widest the account form's fields are.
pub const FORM_W: f32 = 560.;
/// A port field.
pub const PORT_W: f32 = 104.;
/// A label's gap above its field.
pub const LABEL_GAP: f32 = 6.;
/// The gap between the fields of a form.
pub const FIELD_GAP: f32 = 16.;
/// The segmented control (`.seg`): a 2 px inset, 32 px segments, 14 px at
/// their sides.
pub const SEG_PAD: f32 = 2.;
pub const SEG_H: f32 = 32.;
pub const SEG_PAD_X: f32 = 14.;
/// A step row of the account check (its mark and text).
pub const STEP_MARK: f32 = 16.;
/// The trust prompt's fingerprint lines.
pub const FINGERPRINT_W: f32 = 360.;

/// The welcome (`.wel`): the copy's left edge and its distance from the
/// bottom, wide and compact.
pub const WELCOME_X: f32 = 96.;
pub const WELCOME_X_COMPACT: f32 = 64.;
pub const WELCOME_BOTTOM: f32 = 120.;
pub const WELCOME_BOTTOM_COMPACT: f32 = 96.;
/// The copy's width, the lede's, and the seal over the headline.
pub const WELCOME_W: f32 = 470.;
pub const LEDE_W: f32 = 420.;
pub const WELCOME_SEAL: f32 = 40.;
pub const WELCOME_SEAL_GAP: f32 = 28.;
/// The headline (40/48 semibold, the display step) and the lede (16/24).
pub const LEDE_SIZE: f32 = 16.;
pub const LEDE_LINE: f32 = 24.;
pub const LEDE_GAP: f32 = 14.;
/// The call to action: 44 px high, 28 px at its sides, 32 px above.
pub const CTA_H: f32 = 44.;
pub const CTA_PAD_X: f32 = 28.;
pub const CTA_GAP: f32 = 32.;
/// The fine print under it, 12 px below.
pub const FINE_GAP: f32 = 12.;
/// The foot row: 24 px from the bottom, 32 px from the right, 28 px
/// between its links.
pub const FOOT_BOTTOM: f32 = 24.;
pub const FOOT_RIGHT: f32 = 32.;
pub const FOOT_GAP: f32 = 28.;
/// The falling K behind the welcome: its width (wide, compact), and how far
/// it reaches past the right edge and down from the top.
pub const FALL_W: f32 = 700.;
pub const FALL_W_COMPACT: f32 = 520.;
pub const FALL_RIGHT: f32 = -170.;
pub const FALL_RIGHT_COMPACT: f32 = -150.;
pub const FALL_TOP: f32 = 20.;
pub const FALL_TOP_COMPACT: f32 = 30.;

/// The setup flow's card: at most 520 px wide, 32 px inside.
pub const FLOW_W: f32 = 520.;
/// The fall's height for its width (the mark's 130 x 144 view).
pub const FALL_H: f32 = FALL_W * mark::VIEW_H / mark::VIEW_W;
pub const FALL_H_COMPACT: f32 = FALL_W_COMPACT * mark::VIEW_H / mark::VIEW_W;
/// A signature's body, at least (two cards high).
pub const SIG_BODY_H: f32 = 128.;
/// The setup flow's top margin.
pub const FLOW_TOP: f32 = 40.;
pub const FLOW_PAD: f32 = 32.;
/// The step indicator's dots and the gap between them.
pub const STEP_DOT: f32 = 8.;
pub const STEP_GAP: f32 = 8.;

/// The K of the seal (komorebi-brand `logo/mark-mono.svg`, its 256 unit
/// box), drawn alone in the fall colour: the stem, the leaf and the leg.
/// The mockup's view box is (70, 56) to (200, 200).
pub mod mark {
    pub const VIEW_X: f32 = 70.;
    pub const VIEW_Y: f32 = 56.;
    pub const VIEW_W: f32 = 130.;
    pub const VIEW_H: f32 = 144.;
    /// The stem: a 30 unit pill from (76, 64) to (106, 192).
    pub const STEM: (f32, f32, f32, f32) = (76., 64., 106., 192.);
    pub const STEM_R: f32 = 15.;
    /// The leaf: two arcs of radius 86.78 between (120, 132) and (192, 62).
    pub const LEAF_FROM: (f32, f32) = (120., 132.);
    pub const LEAF_TO: (f32, f32) = (192., 62.);
    pub const LEAF_R: f32 = 86.78;
    /// The leg: a 30 unit pill from (146, 146) to (176, 177), its four
    /// corners as in the mark.
    pub const LEG: [(f32, f32); 4] = [
        (135.22, 156.43),
        (165.22, 187.43),
        (186.78, 166.57),
        (156.78, 135.57),
    ];
    pub const LEG_R: f32 = 15.;
}

/// Points to logical pixels (96 / 72), for the compose font sample.
pub const PT_PX: f32 = 4. / 3.;
/// General > Compose's font boxes (the Slint page's 160 and 72 px).
pub const FONT_BOX_W: f32 = 160.;
pub const SIZE_BOX_W: f32 = 72.;
