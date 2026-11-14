//! What real mail needs from the native renderer, checked against what a
//! browser shows for the same HTML (headless Edge, compared by eye and by
//! image difference; see docs/PERF.md section 7). Assertions read painted
//! pixels of backgrounds and boxes, not glyphs, because system fonts change
//! between Windows builds. Nothing here touches the network.

mod common;

use common::big_stack;
use komorebi::mail::render::{self, Deadline, Doc, Engine, Item, Opts, RenderInput};

/// A laid-out page and its pixels at scale 1.
struct Page {
    doc: Doc,
    pix: tiny_skia::Pixmap,
}

impl Page {
    /// The colour at (x, y) in CSS px.
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let p = self.pix.pixel(x, y).unwrap();
        [p.red(), p.green(), p.blue()]
    }

    /// The topmost row at column `x` holding `color`.
    fn first_row(&self, x: u32, color: [u8; 3]) -> Option<u32> {
        (0..self.pix.height()).find(|&y| self.at(x, y) == color)
    }

    /// The leftmost column at row `y` holding `color`.
    fn first_col(&self, y: u32, color: [u8; 3]) -> Option<u32> {
        (0..self.pix.width()).find(|&x| self.at(x, y) == color)
    }

    /// The top of the first line of text starting with `text`.
    fn line(&self, text: &str) -> (f32, f32) {
        let l = self
            .doc
            .text_lines()
            .into_iter()
            .find(|l| l.text.starts_with(text))
            .unwrap_or_else(|| panic!("no line {text:?}"));
        (l.x + l.chars.first().map_or(0.0, |c| c.0), l.y)
    }
}

/// `body` as an HTML message in standards mode, as webmail shows it.
fn page(body: &str, width: f32) -> Page {
    raw_page(
        format!("<!DOCTYPE html><html><body style=\"margin:0\">{body}</body></html>"),
        width,
    )
}

fn raw_page(html: String, width: f32) -> Page {
    big_stack(move || {
        let input = RenderInput {
            html: Some(html),
            ..Default::default()
        };
        let opts = Opts {
            width_px: width,
            ..Default::default()
        };
        let mut engine = Engine::new();
        let doc = render::layout(&input, &opts, &mut engine, &Deadline::none()).unwrap();
        let pix = render::rasterize(&doc, &mut engine, 0, doc.height.ceil() as u32, 1.0).unwrap();
        // The pixmap holds no links to the engine; the doc is moved out whole.
        Page { doc, pix }
    })
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn an_inline_element_paints_its_background_and_padding() {
    let p = page(
        "<p style=\"margin:20px 0\"><a href=\"https://example.com\" style=\"background:#ff0000;\
         color:#fff;padding:10px 20px;font:16px Arial\">Pay now</a></p>",
        300.0,
    );
    // The padding reaches 20 px past the text on the left and 10 px above it.
    let (x, y) = p.line("Pay now");
    let left = p.first_col((y + 8.0) as u32, RED).unwrap();
    assert_eq!(left, 0, "starts at the padding's left edge");
    assert!(x >= 19.0, "text after the padding: {x}");
    let top = p.first_row(5, RED).unwrap();
    assert!(top as f32 <= y - 9.0, "padding above the text: {top} {y}");
    // Vertical padding does not move the line (the 20 px margin stands).
    assert!((y - 20.0).abs() < 2.0, "line stays put: {y}");
    let items = p
        .doc
        .items
        .iter()
        .filter(|i| matches!(i, Item::Rect { .. }));
    assert_eq!(items.count(), 1);
}

#[test]
fn an_inline_border_and_a_nested_span_share_the_box() {
    let p = page(
        "<span style=\"border:2px solid #0000ff;background:#ff0000;padding:0 4px;\
         font:16px Arial\">a <b>bold</b> b</span>",
        300.0,
    );
    let (_, y) = p.line("a bold b");
    let mid = (y + 9.0) as u32;
    assert_eq!(p.at(0, mid), BLUE, "left border");
    assert_eq!(p.at(3, mid), RED, "padding");
    assert_eq!(
        p.doc
            .items
            .iter()
            .filter(|i| matches!(i, Item::Rect { color, .. } if color[..3] == RED))
            .count(),
        1,
        "the bold run has no box of its own"
    );
}

#[test]
fn hidden_inline_boxes_stay_hidden() {
    let p = page(
        "<p><span style=\"visibility:hidden;background:#ff0000\">secret</span>&nbsp;</p>",
        200.0,
    );
    assert!(!(0..p.pix.width()).any(|x| (0..p.pix.height()).any(|y| p.at(x, y) == RED)));
}

#[test]
fn a_mark_is_yellow() {
    let p = page("<mark style=\"font:16px Arial\">Marked</mark>", 200.0);
    assert!((0..200).any(|x| p.at(x, 16) == [255, 255, 0]));
}

#[test]
fn a_declared_cell_height_leaves_the_content_free_to_align() {
    for (valign, low, high) in [
        ("top", 0.0, 4.0),
        ("middle", 35.0, 45.0),
        ("bottom", 75.0, 85.0),
    ] {
        let p = page(
            &format!(
                "<table cellspacing=0 cellpadding=0><tr><td height=100 valign={valign} \
                 style=\"font:16px/20px Arial\">Label</td></tr></table>"
            ),
            300.0,
        );
        let (_, y) = p.line("Label");
        assert!((low..=high).contains(&y), "{valign}: {y}");
    }
}

#[test]
fn a_cell_height_counts_the_content_box_unless_border_box() {
    let cell = |extra: &str| {
        page(
            &format!(
                "<table cellspacing=0 cellpadding=0><tr><td bgcolor=#ff0000 \
                 style=\"height:40px;padding:10px;{extra}\"></td></tr></table>"
            ),
            300.0,
        )
        .doc
        .height
    };
    assert_eq!(cell(""), 60.0);
    assert_eq!(cell("box-sizing:border-box"), 40.0);
}

#[test]
fn a_block_keeps_its_declared_height_whatever_the_content() {
    let p = page(
        "<div style=\"height:0\">gone</div><div style=\"height:10px;font:16px/40px Arial\">x</div>\
         <div style=\"background:#ff0000;height:20px\"></div>",
        300.0,
    );
    assert_eq!(p.doc.height, 30.0);
    assert_eq!(p.first_row(100, RED), Some(10));
}

#[test]
fn min_height_and_border_box_size_a_block() {
    let p = page(
        "<div style=\"background:#ff0000;min-height:30px\"></div>\
         <div style=\"background:#0000ff;width:100px;padding:10px;box-sizing:border-box;height:20px\"></div>",
        300.0,
    );
    assert_eq!(p.doc.height, 50.0);
    assert_eq!(p.at(99, 40), BLUE);
    assert_eq!(p.at(101, 40), [255, 255, 255]);
}

#[test]
fn overflow_hidden_cuts_what_does_not_fit() {
    let p = page(
        "<div style=\"height:30px;overflow:hidden;font:16px/20px Arial\">one<br>two<br>three</div>\
         <div style=\"height:1px\"></div>",
        300.0,
    );
    let Some(item) = p.doc.items.iter().find_map(|i| match i {
        Item::Text { clip_h, .. } => Some(*clip_h),
        _ => None,
    }) else {
        panic!("no text");
    };
    assert!((item - 30.0).abs() < 1.0, "{item}");
}

#[test]
fn legacy_align_moves_a_child_table_but_stops_there() {
    // `td align=center`: the table inside is centred, its cells are not.
    let p = page(
        "<table width=400 cellspacing=0 cellpadding=0><tr><td align=center>\
         <table width=200 cellspacing=0 cellpadding=0><tr><td bgcolor=#ff0000 style=\"font:16px Arial\">\
         Cell<table width=50 cellspacing=0 cellpadding=0><tr><td bgcolor=#0000ff height=4></td></tr></table>\
         </td></tr></table></td></tr></table>",
        400.0,
    );
    assert_eq!(p.first_col(1, RED), Some(100));
    let (x, _) = p.line("Cell");
    assert!(x < 102.0, "text starts at the left of the cell: {x}");
    let y = p.first_row(101, BLUE).unwrap();
    assert_eq!(
        p.first_col(y, BLUE),
        Some(100),
        "and so does the nested table"
    );
}

#[test]
fn td_align_right_moves_a_child_table() {
    let p = page(
        "<table width=400 cellspacing=0 cellpadding=0><tr><td align=right>\
         <table width=100 cellspacing=0 cellpadding=0><tr><td bgcolor=#ff0000 height=10></td></tr></table>\
         </td></tr></table>",
        400.0,
    );
    assert_eq!(p.first_col(5, RED), Some(300));
}

#[test]
fn css_text_align_goes_into_tables() {
    let p = page(
        "<div style=\"text-align:center;font:16px Arial\"><table width=400 cellspacing=0 cellpadding=0><tr>\
         <td>Mid</td></tr></table></div>",
        400.0,
    );
    let (x, _) = p.line("Mid");
    assert!(x > 150.0, "{x}");
}

#[test]
fn align_on_a_block_leaves_the_block_where_it_is() {
    let p = page(
        "<div align=center style=\"width:100px;background:#ff0000;height:10px\"></div>",
        400.0,
    );
    assert_eq!(p.first_col(5, RED), Some(0));
}

#[test]
fn collapsed_borders_resolve_per_edge_and_the_table_does_not_double_them() {
    // The row below draws the rule the row above does not have ("Total").
    let p = page(
        "<table width=100 style=\"border-collapse:collapse\"><tr><td height=20></td></tr>\
         <tr><td height=20 style=\"border-top:2px solid #ff0000\"></td></tr></table>",
        200.0,
    );
    let y = p.first_row(50, RED).unwrap();
    assert!((20..=24).contains(&y), "{y}");
    assert_eq!(p.at(50, y + 1), RED);
    assert_eq!(p.at(50, y + 2), [255, 255, 255]);
    // A table and cell border of 1 px share one 1 px line.
    let p = page(
        "<table width=101 style=\"border-collapse:collapse;border:1px solid #ff0000\"><tr>\
         <td width=50 height=20 style=\"border:1px solid #ff0000;padding:0\"></td>\
         <td width=50 style=\"border:1px solid #ff0000;padding:0\"></td></tr></table>",
        200.0,
    );
    assert_eq!(p.at(0, 10), RED);
    assert_eq!(p.at(1, 10), [255, 255, 255]);
    assert_eq!(p.at(100, 10), RED);
    assert_eq!(p.at(50, 10), RED);
    assert_eq!(p.at(49, 10), [255, 255, 255]);
    assert_eq!(p.doc.height, 22.0);
}

#[test]
fn a_row_border_shows_only_when_collapsed() {
    let row = |collapse: &str| {
        page(
            &format!(
                "<style>tr {{border-bottom:2px solid #ff0000}}</style>\
                 <div style=\"height:1px;background:#0000ff\"></div>\
                 <table width=100 cellspacing=0 style=\"border-collapse:{collapse}\">\
                 <tr><td height=20></td></tr><tr><td height=20></td></tr></table>"
            ),
            200.0,
        )
        .first_row(50, RED)
    };
    assert!(row("collapse").is_some());
    assert_eq!(row("separate"), None);
}

#[test]
fn table_padding_takes_room_from_the_columns() {
    let p = page(
        "<table width=200 cellspacing=0 cellpadding=0 style=\"padding:10px;background:#0000ff\">\
         <tr><td height=10 bgcolor=#ff0000></td></tr></table>",
        300.0,
    );
    assert_eq!(p.first_col(15, RED), Some(10));
    assert_eq!(p.at(189, 15), RED);
    assert_eq!(p.at(190, 15), BLUE);
    assert_eq!(p.doc.height, 30.0);
}

#[test]
fn structural_selectors_reach_table_rows() {
    let p = page(
        "<style>tr:nth-child(even) td {background:#ff0000} tr:nth-child(2n+1) td {background:#0000ff}\
         td {height:10px;padding:0} tr:last-of-type td:not(.keep) {background:#00ff00}\
         </style><table width=100 cellspacing=0 cellpadding=0><tr><td></td></tr><tr><td></td></tr>\
         <tr><td></td></tr><tr><td></td></tr><tr><td class=keep></td><td></td></tr></table>",
        200.0,
    );
    assert_eq!(p.at(10, 5), BLUE);
    assert_eq!(p.at(10, 15), RED);
    assert_eq!(p.at(10, 25), BLUE);
    assert_eq!(p.at(10, 35), RED);
    assert_eq!(p.at(10, 45), BLUE, "the keep cell is left out");
    assert_eq!(p.at(90, 45), [0, 255, 0]);
}

#[test]
fn the_link_pseudo_classes_style_links_and_hover_does_not() {
    let p = page(
        "<style>a:link {background:#ff0000} a:hover {background:#0000ff} a:visited {background:#0000ff}</style>\
         <a href=\"https://example.com\" style=\"font:16px Arial\">link</a>",
        200.0,
    );
    assert!((0..20).any(|x| p.at(x, 8) == RED));
}

#[test]
fn only_child_and_first_of_type() {
    let p = page(
        "<style>div {height:10px} div:only-child {background:#ff0000} span + div:first-of-type {background:#0000ff}\
         </style><section><div></div></section><section><span></span><div></div><div></div></section>",
        50.0,
    );
    assert_eq!(p.at(5, 5), RED);
    assert_eq!(p.at(5, 15), BLUE);
    assert_eq!(p.at(5, 25), [255, 255, 255]);
}

#[test]
fn a_childs_margin_collapses_through_its_parent() {
    let p = page(
        "<div style=\"background:#ff0000\"><p style=\"margin:20px 0 30px;font:16px/20px Arial\">x</p></div>\
         <div style=\"background:#0000ff;height:10px;margin-top:10px\"></div>",
        100.0,
    );
    // The margin is outside the red box, at both ends.
    assert_eq!(p.first_row(50, RED), Some(20));
    assert_eq!(p.first_row(50, BLUE), Some(70));
    assert_eq!(p.doc.height, 80.0);
}

#[test]
fn padding_or_a_cell_keeps_the_margin_inside() {
    let p = page(
        "<div style=\"background:#ff0000;padding-top:1px\"><p style=\"margin:20px 0;font:16px/20px Arial\">x</p></div>",
        100.0,
    );
    assert_eq!(p.first_row(50, RED), Some(0));
    let p = page(
        "<table cellspacing=0 cellpadding=0 width=50><tr><td bgcolor=#ff0000><p style=\"margin:20px 0;\
         font:16px/20px Arial\">x</p></td></tr></table>",
        100.0,
    );
    assert_eq!(p.first_row(45, RED), Some(0));
    assert_eq!(p.doc.height, 60.0);
}

#[test]
fn an_empty_block_collapses_through() {
    let p = page(
        "<div style=\"height:10px;margin-bottom:10px\"></div><p style=\"margin:15px 0\"></p>\
         <div style=\"margin:8px 0\"><div></div></div>\
         <div style=\"background:#ff0000;height:10px;margin-top:5px\"></div>",
        100.0,
    );
    assert_eq!(p.first_row(50, RED), Some(25));
}

/// Runs of `color` along row `y`.
fn runs_of(p: &Page, y: u32, color: [u8; 3]) -> u32 {
    let mut runs = 0;
    let mut inside = false;
    for x in 0..p.pix.width() {
        let on = p.at(x, y) == color;
        if on && !inside {
            runs += 1;
        }
        inside = on;
    }
    runs
}

#[test]
fn dashed_and_double_borders_are_not_solid() {
    let p = page(
        "<div style=\"height:4px;border-bottom:2px dashed #ff0000\"></div>",
        120.0,
    );
    let n = runs_of(&p, 5, RED);
    assert!((8..=12).contains(&n), "{n} dashes");
    assert_eq!(p.at(0, 5), RED, "starts with a dash");
    assert_eq!(p.at(119, 5), RED, "and ends with one");
    let p = page(
        "<div style=\"height:4px;border-top:6px double #ff0000\"></div>",
        50.0,
    );
    assert_eq!(p.at(5, 0), RED);
    assert_eq!(p.at(5, 3), [255, 255, 255]);
    assert_eq!(p.at(5, 5), RED);
}

#[test]
fn a_border_needs_a_style_and_a_style_alone_is_medium() {
    let h = |css: &str| {
        // Something painted besides, or a message with none is no message.
        page(
            &format!(
                "<div style=\"{css};height:10px\"></div>                 <div style=\"height:1px;background:#0000ff\"></div>"
            ),
            50.0,
        )
        .doc
        .height
            - 1.0
    };
    assert_eq!(h("border:1px #ccc"), 10.0);
    assert_eq!(h("border-width:4px"), 10.0);
    assert_eq!(h("border-style:solid"), 16.0);
    assert_eq!(h("border-style:solid;border-width:1px 0"), 12.0);
    assert_eq!(h("border-top:1px solid #000;border-top-style:none"), 10.0);
    assert_eq!(h("border-bottom-style:solid;border-bottom-width:2px"), 12.0);
}

/// A `data:` PNG of `w` by `h` pixels from a colour function.
fn data_png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> String {
    let mut bytes = vec![];
    {
        let mut enc = png::Encoder::new(&mut bytes, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        let mut pixels = vec![];
        for y in 0..h {
            for x in 0..w {
                pixels.extend(f(x, y));
            }
        }
        wr.write_image_data(&pixels).unwrap();
    }
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::from("data:image/png;base64,");
    for c in bytes.chunks(3) {
        let n = u32::from(c[0]) << 16
            | u32::from(c.get(1).copied().unwrap_or(0)) << 8
            | u32::from(c.get(2).copied().unwrap_or(0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

const RED_PNG_SIZE: u32 = 4;

fn red_png() -> String {
    data_png(RED_PNG_SIZE, RED_PNG_SIZE, |_, _| RED)
}

#[test]
fn a_background_image_tiles_by_default_and_obeys_repeat_position_and_size() {
    // Red on a blue checker of 8 px squares: 4 px of red in the corner of each.
    let tile = data_png(8, 8, |x, y| if x < 4 && y < 4 { RED } else { BLUE });
    let p = page(
        &format!("<div style=\"height:16px;background:url({tile})\"></div>"),
        40.0,
    );
    assert_eq!(p.at(1, 1), RED);
    assert_eq!(p.at(5, 1), BLUE);
    assert_eq!(p.at(9, 1), RED, "repeats");
    assert_eq!(p.at(1, 9), RED, "in both directions");
    let red = red_png();
    let p = page(
        &format!(
            "<div style=\"height:20px;background:#00ff00 url({red}) no-repeat center\"></div>"
        ),
        40.0,
    );
    assert_eq!(p.at(1, 1), [0, 255, 0]);
    assert_eq!(p.at(20, 10), RED, "centred, once");
    let p = page(
        &format!(
            "<div style=\"height:20px;background-image:url({red});background-repeat:repeat-x;\
             background-position:0 100%;background-color:#00ff00\"></div>"
        ),
        40.0,
    );
    assert_eq!(p.at(30, 18), RED);
    assert_eq!(p.at(30, 5), [0, 255, 0]);
    let p = page(
        &format!(
            "<div style=\"height:20px;background:url({red}) no-repeat left top/10px 5px;\
             background-color:#00ff00\"></div>"
        ),
        40.0,
    );
    assert_eq!(p.at(9, 4), RED);
    assert_eq!(p.at(11, 4), [0, 255, 0]);
    assert_eq!(p.at(9, 6), [0, 255, 0]);
    let p = page(
        &format!(
            "<div style=\"height:20px;background:url({red}) no-repeat;background-size:cover\"></div>"
        ),
        40.0,
    );
    assert_eq!(p.at(39, 19), RED);
}

#[test]
fn a_linear_gradient_runs_between_its_stops() {
    let p = page(
        "<div style=\"height:10px;background:linear-gradient(to right,#000000,#ffffff)\"></div>\
         <div style=\"height:100px;width:10px;background-image:linear-gradient(#ff0000 50%,#0000ff 50%)\"></div>",
        100.0,
    );
    let [a, ..] = p.at(5, 5);
    let [b, ..] = p.at(50, 5);
    let [c, ..] = p.at(95, 5);
    assert!(a < 30 && (110..150).contains(&b) && c > 225, "{a} {b} {c}");
    assert_eq!(p.at(5, 40), RED);
    assert_eq!(p.at(5, 80), BLUE);
}

#[test]
fn dir_rtl_swaps_table_columns_and_inline_boxes() {
    let p = page(
        "<table width=400 dir=rtl cellspacing=0 cellpadding=0><tr><td width=100 bgcolor=#ff0000 height=10></td>\
         <td width=300 bgcolor=#0000ff></td></tr></table>\
         <div dir=rtl style=\"font-size:0;text-align:left\"><div style=\"display:inline-block;width:100px;\
         height:10px;background:#ff0000\"></div><div style=\"display:inline-block;width:100px;height:10px;\
         background:#0000ff\"></div></div>",
        400.0,
    );
    assert_eq!(p.first_col(5, RED), Some(300));
    assert_eq!(p.first_col(5, BLUE), Some(0));
    assert!(
        p.at(399, 15) == RED || p.at(250, 15) == RED,
        "the first box on the right"
    );
}

#[test]
fn clear_goes_below_the_floats_it_names() {
    let fl = "<div style=\"float:left;width:40px;height:30px;background:#0000ff\"></div>";
    let p = page(
        &format!("{fl}<div style=\"clear:both;height:10px;background:#ff0000\"></div>"),
        100.0,
    );
    assert_eq!(p.first_row(50, RED), Some(30));
    let p = page(
        &format!("{fl}<div style=\"clear:right;height:10px;background:#ff0000\"></div>"),
        100.0,
    );
    assert_eq!(p.first_row(50, RED), Some(0));
    let p = page(
        &format!("{fl}x<br clear=all><div style=\"height:10px;background:#ff0000\"></div>"),
        100.0,
    );
    assert_eq!(p.first_row(50, RED), Some(30));
}

#[test]
fn an_image_keeps_its_border_padding_and_spaces() {
    let red = red_png();
    let p = page(
        &format!(
            "<img src=\"{red}\" width=20 height=20 hspace=5 vspace=3 style=\"display:block\">\
             <span style=\"color:#0000ff\"><img src=\"{red}\" width=20 height=20 border=2 style=\"padding:1px;float:left\"></span>"
        ),
        100.0,
    );
    assert_eq!(p.first_col(10, RED), Some(5), "hspace");
    assert_eq!(p.first_row(10, RED), Some(3), "vspace");
    let y = p.first_row(0, BLUE).unwrap();
    assert_eq!(p.at(1, y + 10), BLUE, "border");
    assert_eq!(p.at(2, y + 10), [255, 255, 255], "padding");
    assert_eq!(p.at(3, y + 10), RED, "content");
}

#[test]
fn a_percentage_radius_makes_an_avatar_round() {
    let red = red_png();
    let p = page(
        &format!(
            "<img src=\"{red}\" width=40 height=40 style=\"display:block;border-radius:50%\">"
        ),
        100.0,
    );
    assert_eq!(p.at(20, 20), RED);
    assert_eq!(p.at(1, 1), [255, 255, 255], "the corner is cut");
}

#[test]
fn a_card_rounds_only_the_corners_it_names() {
    let p = page(
        "<div style=\"height:40px;background:#ff0000;border-radius:16px 16px 0 0\"></div>",
        100.0,
    );
    assert_eq!(p.at(1, 1), [255, 255, 255]);
    assert_eq!(p.at(98, 1), [255, 255, 255]);
    assert_eq!(p.at(1, 38), RED);
    assert_eq!(p.at(98, 38), RED);
    let p = page(
        "<div style=\"height:40px;background:#ff0000;border-bottom-left-radius:16px\"></div>",
        100.0,
    );
    assert_eq!(p.at(1, 38), [255, 255, 255]);
    assert_eq!(p.at(98, 38), RED);
}

#[test]
fn every_css_colour_name_is_known() {
    for (name, rgb) in [
        ("cornflowerblue", [100, 149, 237]),
        ("rebeccapurple", [102, 51, 153]),
        ("lightgoldenrodyellow", [250, 250, 210]),
    ] {
        let p = page(
            &format!("<div style=\"height:4px;background:{name}\"></div>"),
            20.0,
        );
        assert_eq!(p.at(5, 2), rgb, "{name}");
    }
}

#[test]
fn a_semibold_is_wider_than_regular_and_narrower_than_bold() {
    let width = |w: &str| {
        let p = page(
            &format!("<span style=\"font:{w} 40px 'Segoe UI'\">Statement of account</span>"),
            1000.0,
        );
        p.doc.text_lines()[0].w
    };
    assert!(width("400") < width("600"));
    assert!(width("600") < width("700"));
}

#[test]
fn a_caption_shows_over_its_table() {
    let p = page(
        "<table width=100 cellspacing=0 cellpadding=0><caption style=\"font:16px/20px Arial\">Title</caption>\
         <tr><td height=10 bgcolor=#ff0000></td></tr></table>",
        200.0,
    );
    let (x, y) = p.line("Title");
    assert!(y < 2.0 && x > 10.0, "{x} {y}");
    assert_eq!(p.first_row(50, RED), Some(20));
}

#[test]
fn text_indent_hangs_a_word_list_item() {
    let p = page(
        "<p style=\"margin:0 0 0 40px;text-indent:-20px;font:16px/20px Arial\">One two three four five six \
         seven eight nine ten eleven twelve</p>",
        200.0,
    );
    let lines = p.doc.text_lines();
    assert!(lines.len() > 1);
    let first = lines[0].x + lines[0].chars[0].0;
    let next = lines[1].x + lines[1].chars[0].0;
    assert!((first - 20.0).abs() < 1.5, "{first}");
    assert!((next - 40.0).abs() < 1.5, "{next}");
}

#[test]
fn hr_attributes_and_a_cell_that_does_not_wrap() {
    let p = page(
        "<hr size=6 color=#ff0000 width=50 align=left noshade>",
        200.0,
    );
    let y = p.first_row(10, RED).unwrap();
    assert_eq!(p.at(10, y + 5), RED);
    assert_eq!(p.at(60, y + 2), [255, 255, 255]);
    let p = page(
        "<table width=20 cellspacing=0><tr><td nowrap style=\"font:16px Arial\">one two three</td></tr></table>",
        300.0,
    );
    assert_eq!(p.doc.text_lines().len(), 1);
}

#[test]
fn object_fit_cover_fills_without_stretching() {
    // Red on the left half, blue on the right: 40 by 20.
    let wide = data_png(40, 20, |x, _| if x < 20 { RED } else { BLUE });
    let p = page(
        &format!(
            "<img src=\"{wide}\" width=20 height=20 style=\"display:block;object-fit:cover\">\
             <div style=\"height:1px\"></div>"
        ),
        100.0,
    );
    // The middle 20 px of the picture show: half red, half blue, unstretched.
    assert_eq!(p.at(5, 10), RED);
    assert_eq!(p.at(15, 10), BLUE);
    assert_eq!(p.at(25, 10), [255, 255, 255], "nothing past the box");
}

#[test]
fn opacity_fades_boxes_images_and_what_is_inside() {
    let red = red_png();
    let p = page(
        &format!(
            "<div style=\"opacity:.5\"><div style=\"height:10px;background:#ff0000\"></div>\
             <div style=\"opacity:.5;height:10px;background:#ff0000\"></div>\
             <img src=\"{red}\" width=10 height=10 style=\"display:block\"></div>"
        ),
        50.0,
    );
    let g = |y| p.at(5, y)[1] as i32;
    assert!((g(5) - 128).abs() < 4, "{}", g(5));
    assert!((g(15) - 191).abs() < 4, "{}", g(15));
    assert!((g(25) - 128).abs() < 4, "{}", g(25));
}

#[test]
fn col_elements_set_column_widths() {
    let p = page(
        "<table width=400 cellspacing=0 cellpadding=0><colgroup><col width=100><col style=\"width:300px\"></colgroup>\
         <tr><td bgcolor=#ff0000 height=10></td><td bgcolor=#0000ff></td></tr></table>",
        400.0,
    );
    assert_eq!(p.first_col(5, BLUE), Some(100));
}

#[test]
fn a_fixed_table_ignores_its_content() {
    let p = page(
        "<table width=300 cellspacing=0 cellpadding=0 style=\"table-layout:fixed\"><tr>\
         <td width=100 bgcolor=#ff0000 height=10></td><td bgcolor=#0000ff style=\"font:40px Arial\">\
         <div style=\"width:500px;height:1px\"></div></td><td></td></tr></table>",
        600.0,
    );
    assert_eq!(p.first_col(5, BLUE), Some(100));
    assert_eq!(p.at(199, 5), BLUE);
    assert_ne!(
        p.at(201, 5),
        BLUE,
        "the two free columns share what is left"
    );
}

#[test]
fn a_cell_width_is_its_content_box() {
    let p = page(
        "<table width=400 cellspacing=0 cellpadding=0><tr><td width=100 style=\"padding:0 10px;\
         background:#ff0000\" height=10></td><td bgcolor=#0000ff></td></tr></table>",
        400.0,
    );
    assert_eq!(p.first_col(5, BLUE), Some(120));
}

#[test]
fn list_markers_follow_type_start_and_value() {
    let marks = |html: &str| -> Vec<String> {
        let p = page(html, 300.0);
        p.doc
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Text {
                    layout,
                    selectable: false,
                    ..
                } => p.doc.texts.get(*layout).cloned(),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        marks("<ol start=3><li>a</li><li>b</li><li value=10>c</li><li>d</li></ol>"),
        ["3.", "4.", "10.", "11."]
    );
    assert_eq!(marks("<ol type=a><li>a</li><li>b</li></ol>"), ["a.", "b."]);
    assert_eq!(marks("<ol type=I><li value=4>a</li></ol>"), ["IV."]);
    assert_eq!(
        marks("<ul style=\"list-style-type:square\"><li>a</li></ul>"),
        ["\u{25AA}"]
    );
    assert_eq!(
        marks("<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li></ul>"),
        ["\u{25AA}", "\u{25E6}", "\u{2022}"],
        "innermost first: each item's marker comes after its content"
    );
    assert_eq!(
        marks("<ol style=\"list-style:lower-alpha\"><li>a</li></ol>"),
        ["a."]
    );
    assert!(marks("<ul style=\"list-style:none\"><li>a</li></ul>").is_empty());
}

#[test]
fn an_image_keeps_only_the_pixels_it_shows_and_stays_sharp_on_a_scaled_page() {
    // No declared size, as a phone sends it: shown at the width of the page.
    let big = data_png(1200, 800, |x, _| if x % 2 == 0 { RED } else { BLUE });
    let doc = |scale: f32| {
        let html = format!(
            "<!DOCTYPE html><body style=\"margin:0\"><img src=\"{big}\" style=\"display:block;max-width:100%\">\
             <div style=\"display:none\"><img src=\"{}\"></div></body>",
            data_png(50, 50, |_, _| RED)
        );
        big_stack(move || {
            let input = RenderInput {
                html: Some(html),
                ..Default::default()
            };
            let opts = Opts {
                width_px: 300.0,
                scale,
                ..Default::default()
            };
            let mut engine = Engine::new();
            render::layout(&input, &opts, &mut engine, &Deadline::none()).unwrap()
        })
    };
    let d = doc(1.0);
    assert_eq!(d.loaded.len(), 1, "what is not shown is not decoded");
    let pixels: u64 = d
        .images
        .iter()
        .map(|i| u64::from(i.pix.width()) * u64::from(i.pix.height()))
        .sum();
    assert!(pixels < 300 * 200 + 16, "{pixels}");
    // A 2x display: the picture is held at 600 px, not grown from 300.
    let d = doc(2.0);
    assert!(d.images.iter().any(|i| i.pix.width() == 600), "at 2x");
}

#[test]
fn space_at_the_edge_of_an_inline_element_is_kept_and_a_final_br_adds_no_line() {
    // "<b>From:</b> Ana": one space between, as in a browser.
    let p = page(
        "<p style=\"font:16px Arial;margin:0\"><b>From:</b> Ana <b>Sent: </b>Monday</p>",
        400.0,
    );
    let text = p
        .doc
        .text_lines()
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("|");
    assert_eq!(text.trim_end(), "From: Ana Sent: Monday");
    // A paragraph holding only a line break is one line tall; a final line
    // break ends the line before it.
    let h = |body: &str| {
        page(
            &format!("<div style=\"font:16px/20px Arial\">{body}</div><div style=\"height:1px;background:#0000ff\"></div>"),
            300.0,
        )
        .doc
        .height
            - 1.0
    };
    assert_eq!(h("<p style=\"margin:0\"><br></p>"), 20.0);
    assert_eq!(h("<span>x<br></span>"), 20.0);
    assert_eq!(h("x<br><br>"), 40.0, "the second break makes an empty line");
    // A paragraph of only a no-break space (Outlook's empty line) is a line.
    assert_eq!(h("<p style=\"margin:0\">&nbsp;</p>"), 20.0);
}
