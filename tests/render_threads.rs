//! Long business reply chains (synthetic, tests/fixtures/threads/, made by
//! gen_threads.py there): every quoting style real clients produce, 20 to 60
//! levels deep. No line of text may be drawn over another, and the whole
//! thread is laid out.

mod common;

use common::big_stack;
use komorebi::mail::render::{self, Deadline, Doc, Engine, Opts, RenderInput, TextLine};

fn fixture(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/threads")
        .join(name);
    std::fs::read_to_string(p).unwrap()
}

fn lay_out(html: String, width: f32, scale: f32) -> Doc {
    big_stack(move || {
        let input = RenderInput {
            html: Some(html),
            ..Default::default()
        };
        let opts = Opts {
            width_px: width,
            scale,
            ..Default::default()
        };
        render::layout(&input, &opts, &mut Engine::new(), &Deadline::none()).unwrap()
    })
}

/// Pairs of lines whose boxes cross by more than a pixel each way.
fn overlaps(lines: &[TextLine]) -> Vec<(usize, usize)> {
    let mut out = vec![];
    for (i, a) in lines.iter().enumerate() {
        for (j, b) in lines.iter().enumerate().skip(i + 1) {
            if b.y > a.y + a.h + 200.0 {
                // Lines come in document order; later groups may sit beside
                // (floats, cells), never far above.
                continue;
            }
            let dy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
            let dx = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
            if dy > 0.5 * a.h.min(b.h)
                && dx > 1.0
                && !a.text.trim().is_empty()
                && !b.text.trim().is_empty()
            {
                out.push((i, j));
            }
        }
    }
    out
}

const THREADS: [&str; 5] = [
    "t20-thread.html",
    "t40-thread.html",
    "t60-thread.html",
    "t60-big-thread.html",
    "t60-wide-thread.html",
];

#[test]
fn no_line_of_a_long_thread_is_drawn_over_another() {
    for name in THREADS {
        for width in [480.0, 720.0, 960.0] {
            let doc = lay_out(fixture(name), width, 1.0);
            let lines = doc.text_lines();
            assert!(lines.len() > 300, "{name} at {width}");
            assert_eq!(overlaps(&lines), [], "{name} at {width}");
            // The whole thread is there, its oldest message too.
            assert!(!doc.truncated, "{name} at {width}: cut at {}", doc.height);
        }
    }
}

#[test]
fn quoted_headers_keep_the_space_after_the_label() {
    // Outlook's "<b>From:</b> Name": the space is at the edge of an inline
    // element, where it used to be dropped ("From:Ana").
    let doc = lay_out(fixture("t20-thread.html"), 720.0, 1.0);
    let lines = doc.text_lines();
    let from: Vec<&str> = lines
        .iter()
        .map(|l| l.text.as_str())
        .filter(|t| t.starts_with("From:"))
        .collect();
    assert!(from.len() >= 3);
    assert!(from.iter().all(|t| t.starts_with("From: ")), "{from:?}");
}

#[test]
#[ignore]
fn scan() {
    for name in [
        "t20-thread.html",
        "t40-thread.html",
        "t60-thread.html",
        "t60-big-thread.html",
    ] {
        for width in [360.0, 600.0, 720.0, 960.0] {
            let doc = lay_out(fixture(name), width, 1.0);
            let lines = doc.text_lines();
            let o = overlaps(&lines);
            eprintln!(
                "{name} {width}: h {} lines {} overlaps {}",
                doc.height,
                lines.len(),
                o.len()
            );
            for &(i, j) in o.iter().take(4) {
                let (a, b) = (&lines[i], &lines[j]);
                eprintln!(
                    "   {:.1},{:.1} {:.1}x{:.1} {:?}\n   {:.1},{:.1} {:.1}x{:.1} {:?}",
                    a.x,
                    a.y,
                    a.w,
                    a.h,
                    a.text.get(..40.min(a.text.len())),
                    b.x,
                    b.y,
                    b.w,
                    b.h,
                    b.text.get(..40.min(b.text.len()))
                );
            }
        }
    }
}

#[test]
#[ignore]
fn scan_reuse() {
    use komorebi::mail::render::{NoRemote, Reuse};
    let names = ["t20-thread.html", "t40-thread.html", "t60-thread.html"];
    big_stack(move || {
        let mut engine = Engine::new();
        let mut earlier: Option<Doc> = None;
        for name in names {
            for (w1, w2, scale) in [
                (720.0, 600.0, 1.0),
                (600.0, 960.0, 1.25),
                (360.0, 720.0, 1.5),
            ] {
                let input = RenderInput {
                    html: Some(fixture(name)),
                    ..Default::default()
                };
                let mut reuse = Reuse::new(&input);
                let mut opts = Opts {
                    width_px: w1,
                    scale,
                    ..Default::default()
                };
                let a = render::layout_reusing(
                    &mut reuse,
                    &opts,
                    &mut engine,
                    &Deadline::none(),
                    &mut NoRemote,
                    earlier.as_ref(),
                )
                .unwrap();
                opts.width_px = w2;
                opts.dark = true;
                let b = render::layout_reusing(
                    &mut reuse,
                    &opts,
                    &mut engine,
                    &Deadline::none(),
                    &mut NoRemote,
                    Some(&a),
                )
                .unwrap();
                let fresh = lay_out(fixture(name), w2, scale);
                let (lb, lf) = (b.text_lines(), fresh.text_lines());
                let same = lb.len() == lf.len()
                    && lb
                        .iter()
                        .zip(&lf)
                        .all(|(x, y)| (x.y - y.y).abs() < 0.01 && x.text == y.text);
                eprintln!(
                    "{name} {w1}->{w2} @{scale}: overlaps first {} second {} fresh {}; second == fresh: {same} (h {} vs {})",
                    overlaps(&a.text_lines()).len(),
                    overlaps(&lb).len(),
                    overlaps(&lf).len(),
                    b.height,
                    fresh.height
                );
                earlier = Some(b);
            }
        }
    });
}

#[test]
#[ignore]
fn scan_snippets() {
    let long =
        "delivery schedule quote revision contract clause warehouse customs approval ".repeat(8);
    let snippets: Vec<(&str, String)> = vec![
        (
            "em-before-size",
            format!("<p style=\"line-height:1.5em;font-size:22px\">{long}</p><p>after</p>"),
        ),
        (
            "class-em",
            format!(
                "<style>.x{{line-height:1.2em}}</style><p class=x style=\"font-size:24px\">{long}</p><p>after</p>"
            ),
        ),
        (
            "font-shorthand-em",
            format!("<p style=\"font:bold 22px/1.3em Arial\">{long}</p><p>after</p>"),
        ),
        (
            "px-lh-big-child",
            format!(
                "<div style=\"font-size:12px;line-height:14px\"><span style=\"font-size:28px\">{long}</span></div>"
            ),
        ),
        (
            "pct-lh-big-child",
            format!(
                "<p style=\"font-size:11pt;line-height:107%\"><span style=\"font-size:20pt\">{long}</span></p>"
            ),
        ),
        (
            "word-list",
            format!(
                "<p class=MsoListParagraph style=\"margin-left:36pt;text-indent:-18pt;mso-list:l0 level1 lfo1\"><![if !supportLists]><span style=\"font-family:Symbol\">·<span style='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;</span></span><![endif]>{long}</p><p class=MsoListParagraph style=\"margin-left:36pt;text-indent:-18pt\">{long}</p>"
            ),
        ),
        (
            "cell-height",
            format!(
                "<table width=100%><tr><td style=\"height:20px\" width=50%>{long}</td><td>{long}</td></tr><tr><td>next row</td></tr></table>"
            ),
        ),
        (
            "font0-columns",
            format!(
                "<div style=\"font-size:0;line-height:0\"><div style=\"display:inline-block;width:50%;vertical-align:top;font-size:14px;line-height:20px\">{long}</div><div style=\"display:inline-block;width:50%;vertical-align:top;font-size:14px\">{long}</div></div><p>after</p>"
            ),
        ),
        (
            "float-img",
            format!(
                "<img src=\"cid:x\" width=120 height=200 align=left alt=logo><p>{long}</p><p>{long}</p><div style=\"clear:both\">after</div>"
            ),
        ),
        (
            "max-height",
            format!("<div style=\"max-height:40px\">{long}</div><p>after</p>"),
        ),
        (
            "neg-margin",
            format!("<p>{long}</p><p style=\"margin-top:-30px\">{long}</p>"),
        ),
        (
            "neg-table-margin",
            format!(
                "<table style=\"margin-left:-5.4pt\" cellpadding=0><tr><td style=\"padding:0 5.4pt\">{long}</td></tr></table><p>after</p>"
            ),
        ),
        (
            "sup-sub",
            format!("<p>{long}<sup>1</sup> x<sub>2</sub> {long}</p>"),
        ),
        (
            "display-table",
            format!(
                "<div style=\"display:table;width:100%\"><div style=\"display:table-cell;width:50%\">{long}</div><div style=\"display:table-cell\">{long}</div></div><p>after</p>"
            ),
        ),
        (
            "pre-in-quote",
            format!(
                "<blockquote><pre>{}</pre></blockquote><p>after</p>",
                long.replace("customs ", "customs\n")
            ),
        ),
        (
            "huge-group",
            format!(
                "<div>{}</div><p>after</p>",
                (0..4000)
                    .map(|i| format!("line {i} of the quoted plain text reply &gt; &gt; text<br>"))
                    .collect::<String>()
            ),
        ),
        (
            "inline-block-lh",
            format!(
                "<p style=\"line-height:10px\"><span style=\"display:inline-block;font-size:20px\">{long}</span></p><p>after</p>"
            ),
        ),
        (
            "lh-0",
            format!(
                "<div style=\"line-height:0\"><span style=\"font-size:16px\">{long}</span></div><p>after</p>"
            ),
        ),
        (
            "absolute",
            format!("<div style=\"position:absolute;top:0;left:0\">{long}</div><p>{long}</p>"),
        ),
        (
            "div-height",
            format!("<div style=\"height:30px\">{long}</div><p>after</p>"),
        ),
    ];
    for (name, body) in snippets {
        for width in [320.0, 600.0, 960.0] {
            let html = format!("<!DOCTYPE html><html><body>{body}</body></html>");
            let doc = lay_out(html, width, 1.0);
            let lines = doc.text_lines();
            let o = overlaps(&lines);
            if !o.is_empty() {
                let (a, b) = (&lines[o[0].0], &lines[o[0].1]);
                eprintln!(
                    "{name} @{width}: {} overlaps; {:.1},{:.1} h{:.1} {:?} / {:.1},{:.1} h{:.1} {:?}",
                    o.len(),
                    a.x,
                    a.y,
                    a.h,
                    a.text.get(..20.min(a.text.len())),
                    b.x,
                    b.y,
                    b.h,
                    b.text.get(..20.min(b.text.len()))
                );
            }
        }
    }
}

#[test]
fn a_long_thread_gets_a_layout_budget_in_proportion_to_its_length() {
    let input = |html: String| RenderInput {
        html: Some(html),
        ..Default::default()
    };
    assert_eq!(
        render::layout_budget(&input("<p>hi</p>".into())),
        render::LAYOUT_BUDGET
    );
    let big = render::layout_budget(&input(fixture("t60-big-thread.html")));
    assert!(big > render::LAYOUT_BUDGET * 3 / 2, "{big:?}");
    let huge = render::layout_budget(&input("x".repeat(render::MAX_HTML_BYTES)));
    assert_eq!(huge, render::MAX_LAYOUT_BUDGET);
}
