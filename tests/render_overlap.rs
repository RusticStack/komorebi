//! Constructs that drew lines of text over one another in real mail, each
//! rebuilt by hand with made-up text. Every case is laid out at the widths
//! the reader uses and no visible line may cross another.

mod common;

use common::big_stack;
use komorebi::mail::render::{self, Deadline, Doc, Engine, Opts, RenderInput, TextLine};

fn lay_out(html: &str, width: f32) -> Doc {
    let html = html.to_string();
    big_stack(move || {
        let input = RenderInput {
            html: Some(html),
            ..Default::default()
        };
        let opts = Opts {
            width_px: width,
            ..Default::default()
        };
        render::layout(&input, &opts, &mut Engine::new(), &Deadline::none()).unwrap()
    })
}

/// Pairs of lines that cross by more than half a line each way and overlap
/// sideways, as (text, text) for the assertion message.
fn overlaps(lines: &[TextLine]) -> Vec<(String, String)> {
    let mut out = vec![];
    for (i, a) in lines.iter().enumerate() {
        for b in lines.iter().skip(i + 1) {
            let dy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
            let dx = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
            if dy > 0.5 * a.h.min(b.h) && dx > 1.0 {
                out.push((a.text.clone(), b.text.clone()));
            }
        }
    }
    out
}

fn assert_no_overlap(name: &str, html: &str) -> Doc {
    let mut last = None;
    for width in [600.0, 720.0, 960.0] {
        let doc = lay_out(html, width);
        let lines = doc.text_lines();
        assert!(!lines.is_empty(), "{name} at {width}: no text");
        assert_eq!(
            overlaps(&lines),
            [] as [(String, String); 0],
            "{name} at {width}"
        );
        last = Some(doc);
    }
    last.unwrap()
}

/// The top of the first line whose text starts with `prefix`.
fn line_y(doc: &Doc, prefix: &str) -> f32 {
    doc.text_lines()
        .iter()
        .find(|l| l.text.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starting {prefix:?}"))
        .y
}

/// A newsletter column of `<table align="left" width="100%">` blocks, one per
/// paragraph, with a centred button table in flow between them: each float
/// takes the whole width, so the next one goes below it, and so do the
/// table and the lines after them.
#[test]
fn full_width_floated_tables_stack() {
    let para = |t: &str| {
        format!(
            r#"<table align="left" width="100%" cellpadding="0" cellspacing="0"><tr>
<td align="left" style="padding:20px 40px 0 40px;font-family:sans-serif;font-size:16px;line-height:21px">{t}</td>
</tr></table>"#
        )
    };
    let html = format!(
        r#"<table width="100%" cellpadding="0" cellspacing="0"><tr><td align="center">
<table align="center" width="500" style="width:500px" cellpadding="0" cellspacing="0"><tr><td>
<table align="left" width="100%" cellpadding="0" cellspacing="0"><tr>
<td style="padding:20px 40px 0 40px;font-size:36px;line-height:43px;font-weight:700">Alpha heading here</td>
</tr></table>
{}{}{}
<table align="center" width="100%" cellpadding="0" cellspacing="0"><tr><td style="padding:20px 40px 0 40px">
<table width="100%" style="background:#c00"><tr><td align="center" style="padding:12px;color:#fff">Echo button</td></tr></table>
</td></tr></table>
{}
<p>Golf closing line</p>
</td></tr></table>
</td></tr></table>"#,
        para("Bravo first paragraph with enough words to wrap on to a second line in the column."),
        para("Charlie second paragraph, also long enough to wrap at least once in the column."),
        para("<b>Delta bold start.</b> Then plain words follow until the line wraps."),
        para("Foxtrot after the button."),
    );
    let doc = assert_no_overlap("full-width floats", &html);
    let order = [
        "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf",
    ];
    for pair in order.windows(2) {
        assert!(
            line_y(&doc, pair[0]) < line_y(&doc, pair[1]),
            "{} should be above {}",
            pair[0],
            pair[1]
        );
    }
}

/// Two left floats that fit side by side stay side by side.
#[test]
fn narrow_floats_still_sit_side_by_side() {
    let html = r#"<div style="width:500px">
<table align="left" width="200"><tr><td>Left one</td></tr></table>
<table align="left" width="200"><tr><td>Left two</td></tr></table>
<p>Text beside them.</p></div>"#;
    let doc = assert_no_overlap("narrow floats", html);
    assert!((line_y(&doc, "Left one") - line_y(&doc, "Left two")).abs() < 1.0);
}

/// Paragraphs inside an inline element with a `<br>` between them
/// (`<span><p>..</p><br><p>..</p></span>`), as some templates write their
/// body copy: the line after the break holds only the second paragraph's
/// box, and that line counts towards the height, so what follows starts
/// below it.
#[test]
fn a_block_after_a_line_break_inside_an_inline_keeps_its_line() {
    let html = r#"<style>p{display:block;margin:13px 0}</style>
<table width="100%"><tr><td style="font-size:0px">
<div style="font-family:Arial;font-size:15px;line-height:20px"><span><p>Alpha opening paragraph.</p>
<br><p>Bravo middle paragraph.</p>
<br><p>Charlie last paragraph, long enough to wrap on to a second line at the narrowest width used here, and then some more words to be sure.</p></span></div>
</td></tr><tr><td><div style="font-size:18px">Delta next section</div></td></tr></table>"#;
    let doc = assert_no_overlap("block after br in inline", html);
    let order = ["Alpha", "Bravo", "Charlie", "Delta"];
    for pair in order.windows(2) {
        assert!(line_y(&doc, pair[0]) < line_y(&doc, pair[1]));
    }
}
