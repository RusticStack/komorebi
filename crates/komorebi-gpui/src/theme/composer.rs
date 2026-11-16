//! The composer's geometry (mockup `compose`, app.html `.ch`, `.fld`,
//! `.chip`, `.fmt`, `.ed`, `.saved`; phase 4). Logical px on the 4 px grid
//! where the mockup is; the editor's own sizes come from the document
//! (`mail::rich::view`).

/// The header row: 14 px above, 6 below, 28 px on the left, 12 on the right.
pub const HEADER_TOP: f32 = 14.;
pub const HEADER_BOTTOM: f32 = 6.;
pub const INSET_LEFT: f32 = 28.;
pub const INSET_RIGHT: f32 = 24.;
pub const HEADER_RIGHT: f32 = 12.;
/// The gap between the header's icon buttons, and before Send.
pub const HEADER_GAP: f32 = 4.;
pub const SEND_GAP: f32 = 6.;
/// A field row's vertical padding and the gap between its pieces.
pub const FIELD_PAD_Y: f32 = 8.;
pub const FIELD_GAP: f32 = 8.;
/// The format bar: 8 px above and below, 22 px on the left, 2 px between
/// buttons and 10 px between groups.
pub const BAR_PAD_Y: f32 = 8.;
pub const BAR_PAD_X: f32 = 22.;
pub const BAR_GAP: f32 = 2.;
/// Between the format bar's groups: a 1 px rule in the line colour, 16 px
/// high, with 4 px of room on each side (Gmail's groups; the brand's one
/// edge, no darker than a divider).
pub const BAR_SEP_SPACE: f32 = 4.;
pub const BAR_SEP_H: f32 = 16.;
/// A rule's whole width on the bar: the line and its room.
pub const BAR_SEP_W: f32 = super::layout::HAIRLINE + BAR_SEP_SPACE * 2.;
/// The format bar's boxes, fixed so the bar's width is known before it is
/// drawn (what does not fit goes to More): the font box (a long name ends
/// in an ellipsis), the size box (two digits) and the alignment box (its
/// glyph and chevron).
pub const FONT_BOX_W: f32 = 120.;
pub const SIZE_BOX_W: f32 = 52.;
pub const ALIGN_BOX_W: f32 = 48.;
/// The body: 10 px above, 28 px on the sides, at most 700 px of text.
pub const BODY_PAD_Y: f32 = 10.;
pub const BODY_MAX_W: f32 = 700.;
/// The body's least height before the message scrolls.
pub const BODY_MIN_H: f32 = 160.;
/// The caption at the bottom ("Draft saved"): 16 px under it.
pub const CAPTION_BOTTOM: f32 = 16.;
/// The caret, 1.5 px as the reader's text engine measures it.
pub const CARET_W: f32 = 1.5;
/// The page painted above and below the part in view, in view heights, so
/// a short scroll shows pieces painted already.
pub const MARGIN_VIEWS: f32 = 1.;
/// Room kept between the caret and the edge it is scrolled to.
pub const CARET_ROOM: f32 = 8.;
/// The least width of the text typed after an address field's chips.
pub const CHIP_INPUT_MIN_W: f32 = 120.;
/// The suggestions under an address field.
pub const SUGGEST_W: f32 = 360.;
pub const SUGGEST_ROW_H: f32 = 40.;
/// An attachment pill (mockup `.att`): 40 px, 12 px before its glyph.
pub const ATTACH_GAP: f32 = 8.;
/// The link row's field width.
pub const LINK_W: f32 = 360.;
/// The composition (IME) box's padding around its text.
pub const IME_PAD: f32 = 2.;
/// The composition's underline.
pub const IME_UNDERLINE: f32 = 1.;
/// Kit's own padding of a text area's text (the plain body).
pub const PLAIN_PAD: f32 = 10.;
/// The plain-text body's visible lines before it scrolls.
pub const PLAIN_ROWS: usize = 12;

/// The body step the document's sizes are drawn at 100 percent (a 14 px
/// body is `zoom` 1.0), as the Slint app's `Theme.body / 14`.
pub const ZOOM_BASE: f32 = 14.;

/// A token colour as the text engine's straight RGBA bytes.
pub fn rgba8(c: gpui::Hsla) -> [u8; 4] {
    let c = gpui::Rgba::from(c);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    [byte(c.r), byte(c.g), byte(c.b), byte(c.a)]
}

/// The colours the formatted editor's pages are painted with (the
/// theme's: links in the accent (Komorebi's is its ink), list markers and quote bars in ink-3, table
/// rules in the line colour, on paper). High contrast is not in 1.0.
pub fn editor_theme(t: &super::Tokens) -> komorebi::mail::rich::view::Theme {
    let c = &t.color;
    komorebi::mail::rich::view::Theme {
        text: rgba8(c.ink),
        link: rgba8(c.accent),
        subtle: rgba8(c.ink_3),
        border: rgba8(c.line),
        bg: rgba8(c.paper),
        dark: t.scheme.is_dark(),
        contrast: false,
    }
}
