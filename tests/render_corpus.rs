//! The native HTML renderer on the synthetic corpus (`tests/fixtures/html`)
//! at 600 and 900 px. Assertions are structural (heights within a recorded
//! range, text runs, links, blocked remote content, images) plus a few
//! background colour probes, because pixel goldens of system fonts change
//! between Windows builds. Nothing here touches the network.

mod common;

use common::{FILES, big_stack, fixture, input, raw};
use komorebi::mail::render::links::{self, Warning};
use komorebi::mail::render::{
    self, Deadline, Doc, Engine, Item, LAYOUT_BUDGET, MAX_DOC_H, MAX_HTML_BYTES, Opts,
    RemotePolicy, RenderError, RenderInput, Resolver,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Per file: heights at 600 and 900 px, link counts at 600 and 900 px and
/// distinct blocked remote URLs at 600 and 900 px, as recorded when the
/// renderer was ported (the corpus manifest has only the cid maps).
/// (file, [height; 2], [links; 2], [blocked remote; 2]) at 600 and 900 px.
type Expect = (&'static str, [f32; 2], [usize; 2], [usize; 2]);

const EXPECT: [Expect; 14] = [
    (
        "01-mailchimp-campaign",
        [4501.0, 4501.0],
        [12, 12],
        [11, 11],
    ),
    ("02-substack-post", [14713.0, 14713.0], [52, 52], [7, 7]),
    ("03-github-review", [749.0, 707.0], [3, 3], [1, 1]),
    ("04-order-receipt", [1390.0, 1350.0], [11, 12], [4, 4]),
    ("05-bank-statement", [9099.0, 9099.0], [1, 1], [0, 0]),
    ("06-outlook-word", [2727.0, 2457.0], [2, 2], [0, 0]),
    ("07-gmail-reply-chain", [13173.0, 7635.0], [4, 4], [0, 0]),
    ("08-dark-marketing", [2159.0, 2183.0], [7, 7], [6, 6]),
    ("09-image-heavy-promo", [1468.0, 1468.0], [14, 14], [17, 16]),
    ("10-transactional-buttons", [788.0, 788.0], [3, 3], [0, 0]),
    ("11-newsletter-2col", [4647.0, 3574.0], [21, 21], [6, 6]),
    ("12-personal-applemail", [756.0, 734.0], [2, 2], [0, 0]),
    ("13-calendar-invite", [709.0, 697.0], [6, 6], [1, 1]),
    (
        "14-adversarial-malformed",
        [3681.0, 3371.0],
        [3, 3],
        [22, 22],
    ),
];

fn render(input: &RenderInput, width: f32) -> Result<Doc, RenderError> {
    let opts = Opts {
        width_px: width,
        ..Default::default()
    };
    render::layout(input, &opts, &mut Engine::new(), &Deadline::none())
}

fn doc(name: &'static str, width: f32) -> Doc {
    big_stack(move || render(&input(&raw(name)), width).unwrap())
}

fn lines(doc: &Doc) -> Vec<String> {
    doc.text_lines().into_iter().map(|l| l.text).collect()
}

fn has_line(doc: &Doc, pred: impl Fn(&str) -> bool) -> bool {
    doc.text_lines().iter().any(|l| pred(&l.text))
}

/// The cids `_manifest.json` lists for a file.
fn manifest_cids(name: &str) -> Vec<String> {
    let m = std::fs::read_to_string(fixture("_manifest.json")).unwrap();
    let at = m.find(&format!("\"name\": \"{name}\"")).unwrap();
    let rest = &m[at..];
    let rest = &rest[rest.find("\"cids\"").unwrap()..];
    let body = &rest[rest.find('{').unwrap() + 1..rest.find('}').unwrap()];
    body.split(',')
        .filter_map(|kv| kv.split('"').nth(1).map(str::to_string))
        .collect()
}

#[test]
fn corpus_renders_at_both_widths() {
    big_stack(|| {
        let mut engine = Engine::new();
        for (name, heights, links, blocked) in EXPECT {
            let input = input(&raw(name));
            let html = input.html.clone().unwrap().to_ascii_lowercase();
            let mut loaded = BTreeSet::new();
            for (i, width) in [600.0, 900.0].into_iter().enumerate() {
                let opts = Opts {
                    width_px: width,
                    ..Default::default()
                };
                let doc = render::layout(
                    &input,
                    &opts,
                    &mut engine,
                    &Deadline::after(Duration::from_secs(30)),
                )
                .unwrap_or_else(|e| panic!("{name} at {width}: {e:?}"));
                let want = heights[i];
                assert!(
                    (doc.height - want).abs() <= want * 0.15,
                    "{name} at {width}: height {} not within 15% of {want}",
                    doc.height
                );
                // Distinct targets (each `<a>` is its own link).
                let hrefs: BTreeSet<&str> =
                    doc.links.iter().skip(1).map(|l| l.href.as_str()).collect();
                assert_eq!(hrefs.len(), links[i], "{name} at {width}: links {hrefs:?}");
                for h in &hrefs {
                    let l = h.to_ascii_lowercase();
                    assert!(
                        l.starts_with("https:")
                            || l.starts_with("http:")
                            || l.starts_with("mailto:")
                            || l.starts_with('#'),
                        "{name}: inert link kept: {h}"
                    );
                }
                assert_eq!(
                    doc.blocked_remote, blocked[i],
                    "{name} at {width}: {:?}",
                    doc.blocked_urls
                );
                assert!(doc.blocked_urls.iter().all(|u| render_is_remote(u)));
                loaded.extend(doc.loaded.iter().map(|s| s.to_ascii_lowercase()));
            }
            // Every manifest cid the HTML uses is resolved (at some width:
            // media queries may swap a background image).
            for cid in manifest_cids(name) {
                let src = format!("cid:{}", cid.to_ascii_lowercase());
                if html.contains(&src) {
                    assert!(
                        loaded.contains(&src),
                        "{name}: {src} not resolved: {loaded:?}"
                    );
                }
            }
        }
    });
}

/// A fake fetcher that counts what the renderer asks it for and supplies
/// nothing.
#[derive(Default)]
struct Counting(Vec<String>);

impl Resolver for Counting {
    fn remote(&mut self, url: &str) -> Option<Arc<[u8]>> {
        self.0.push(url.to_string());
        None
    }
}

#[test]
fn remote_images_are_requested_only_with_consent() {
    big_stack(|| {
        let mut engine = Engine::new();
        let mut listed = 0;
        for name in FILES {
            let input = input(&raw(name));
            for width in [600.0, 900.0] {
                let mut opts = Opts {
                    width_px: width,
                    ..Default::default()
                };
                let mut block = Counting::default();
                let doc =
                    render::layout_with(&input, &opts, &mut engine, &Deadline::none(), &mut block)
                        .unwrap();
                assert!(block.0.is_empty(), "{name}: fetched {:?}", block.0);
                // What consent would load: distinct remote images, all among
                // the blocked URLs.
                let wanted: BTreeSet<&String> = doc.remote_images.iter().collect();
                assert_eq!(wanted.len(), doc.remote_images.len(), "{name}");
                assert!(doc.remote_images.iter().all(|u| render_is_remote(u)));
                assert!(
                    wanted.len() <= doc.blocked_remote,
                    "{name}: {wanted:?} vs {:?}",
                    doc.blocked_urls
                );
                // No tracking pixel (14) and no remote CSS or font (01, 06).
                assert!(
                    doc.remote_images
                        .iter()
                        .all(|u| !u.contains("pixel.gif") && !u.contains("fonts.")),
                    "{name}: {wanted:?}"
                );
                listed += wanted.len();

                // With consent the resolver is asked for exactly those.
                opts.remote = RemotePolicy::Allow;
                let mut allow = Counting::default();
                let doc2 =
                    render::layout_with(&input, &opts, &mut engine, &Deadline::none(), &mut allow)
                        .unwrap();
                let asked: BTreeSet<&String> = allow.0.iter().collect();
                assert_eq!(asked, wanted, "{name} at {width}");
                assert_eq!(doc2.remote_images, doc.remote_images, "{name}");
            }
        }
        assert!(listed > 0);
    });
}

#[test]
fn tracking_backgrounds_are_never_requested() {
    big_stack(|| {
        let input = RenderInput {
            html: Some(
                r#"<div style="width:1px;height:1px;background:url(https://t.example.invalid/bg.gif)"></div>
                <table><tr><td width="1" height="1" background="https://t.example.invalid/td.gif">x</td></tr></table>
                <div style="height:40px;background-image:url(https://cdn.example.invalid/hero.png)">Hi</div>"#
                    .into(),
            ),
            ..Default::default()
        };
        let opts = Opts {
            remote: RemotePolicy::Allow,
            ..Default::default()
        };
        let mut allow = Counting::default();
        let doc = render::layout_with(
            &input,
            &opts,
            &mut Engine::new(),
            &Deadline::none(),
            &mut allow,
        )
        .unwrap();
        assert_eq!(allow.0, ["https://cdn.example.invalid/hero.png"]);
        assert_eq!(doc.remote_images, allow.0);
    });
}

fn render_is_remote(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.starts_with("http:")
        || l.starts_with("https:")
        || l.starts_with("//")
        || l.starts_with("ftp:")
}

#[test]
fn receipt_cells_stack_and_keep_line_breaks() {
    for width in [600.0, 900.0] {
        let d = doc("04-order-receipt", width);
        let tl = d.text_lines();
        let thursday = tl
            .iter()
            .find(|l| l.text == "Thursday, 9 October")
            .expect("Thursday line");
        let marta = tl
            .iter()
            .find(|l| l.text == "Marta Silva")
            .expect("Marta Silva line");
        assert!(tl.iter().any(|l| l.text == "Portugal"));
        // `<br>` in item cells: each line stands alone, left aligned.
        let title = tl
            .iter()
            .find(|l| l.text == "Terra Stoneware Mug, 350 ml, Rust glaze")
            .expect("title line");
        let sold = tl
            .iter()
            .find(|l| l.text == "Sold by: Lumen Studio")
            .expect("sold-by line");
        assert!(sold.y > title.y && (sold.x - title.x).abs() < 1.0);
        assert!(tl.iter().any(|l| l.text == "Qty: 2"));
        if width == 600.0 {
            // `.addr td { display: block }`: the cells stack.
            assert!(marta.y > thursday.y && (marta.x - thursday.x).abs() < 1.0);
        } else {
            assert!(marta.x > thursday.x + 100.0);
        }
    }
}

#[test]
fn deep_quotes_are_flattened_not_dropped() {
    for width in [600.0, 900.0] {
        let d = doc("07-gmail-reply-chain", width);
        // Deep content is narrow, so attribution lines may wrap.
        let all = lines(&d).join(" ");
        assert!(all.contains("On Thu, Sep 26, 2026 at 2:25"));
        assert!(
            all.contains("On 26 Sep 2026, at 09:26, Beatriz Nunes"),
            "deepest quote missing"
        );
    }
}

#[test]
fn newsletter_columns_sit_side_by_side_when_they_fit() {
    let d = doc("11-newsletter-2col", 900.0);
    let tl = d.text_lines();
    let s1 = tl.iter().find(|l| l.text.starts_with("Story 1:")).unwrap();
    let s2 = tl.iter().find(|l| l.text.starts_with("Story 2:")).unwrap();
    assert!(s2.x > s1.x + 200.0, "story columns stacked");
    // At 600 px the 564 px wrapper cannot hold two 290 px columns.
    let d = doc("11-newsletter-2col", 600.0);
    let tl = d.text_lines();
    let s1 = tl.iter().find(|l| l.text.starts_with("Story 1:")).unwrap();
    let s2 = tl.iter().find(|l| l.text.starts_with("Story 2:")).unwrap();
    assert!(s2.y > s1.y);
}

#[test]
fn blocked_images_get_an_outlined_placeholder() {
    let d = doc("09-image-heavy-promo", 600.0);
    let outlines = d
        .items
        .iter()
        .filter(|it| matches!(it, Item::Stroke { width, .. } if *width == 1.0))
        .count();
    assert!(outlines >= 12, "{outlines} outlines");
    assert!(has_line(&d, |t| t.contains("Category 1: up to 20% off")));
}

#[test]
fn hidden_preheaders_and_scripts_never_show() {
    let d = doc("01-mailchimp-campaign", 600.0);
    assert!(!has_line(&d, |t| t.contains("Four new glazes")));
    let d = doc("02-substack-post", 600.0);
    let n = lines(&d)
        .iter()
        .filter(|t| t.contains("What a decade of offline"))
        .count();
    assert_eq!(n, 1, "only the visible subtitle");
    let d = doc("10-transactional-buttons", 600.0);
    assert!(!has_line(&d, |t| t.contains("lido por 10 minutos")));
    let d = doc("03-github-review", 600.0);
    assert!(!has_line(&d, |t| t.contains("@context") || t.contains("schema.org")));
    let d = doc("14-adversarial-malformed", 600.0);
    for bad in [
        "document.cookie",
        "svg-script",
        "noscript-mxss",
        "evil.example.invalid",
    ] {
        assert!(!has_line(&d, |t| t.contains(bad)), "{bad} shown");
    }
}

#[test]
fn adversarial_message_is_contained() {
    big_stack(|| {
        let input = input(&raw("14-adversarial-malformed"));
        let mut engine = Engine::new();
        let opts = Opts {
            width_px: 900.0,
            ..Default::default()
        };
        render::layout(&input, &opts, &mut engine, &Deadline::none()).unwrap();
        let t = Instant::now();
        let d =
            render::layout(&input, &opts, &mut engine, &Deadline::after(LAYOUT_BUDGET)).unwrap();
        let took = t.elapsed();
        if !cfg!(debug_assertions) {
            assert!(took < LAYOUT_BUDGET, "14 took {took:?}");
        }
        assert!(d.height <= MAX_DOC_H);
        assert!(
            d.links
                .iter()
                .all(|l| !l.href.to_ascii_lowercase().contains("script"))
        );
        // The 5000-character word is broken to the line, not overflowing.
        for l in d
            .text_lines()
            .iter()
            .filter(|l| l.text.contains("AAAAAAAAAA"))
        {
            assert!(l.w <= 900.0 + 2.0, "line {} px wide", l.w);
        }
        // The fake "verified" banner (absolute, negative offset) is dropped.
        assert!(
            !d.text_lines()
                .iter()
                .any(|l| l.text.contains("Verified sender"))
        );
    });
}

#[test]
fn tall_documents_are_capped() {
    let html = "<p>line</p>".repeat(3000);
    let d = big_stack(move || {
        render(
            &RenderInput {
                html: Some(html),
                ..Default::default()
            },
            600.0,
        )
        .unwrap()
    });
    assert!(d.truncated);
    assert_eq!(d.height, MAX_DOC_H);
    assert!(d.full_height > MAX_DOC_H);
}

/// The cell cap keeps even this table fast in release builds, so a short
/// deadline exercises the cut-off; it must also return promptly.
#[test]
fn deadline_fires_on_a_huge_table() {
    let mut html = String::from("<table>");
    while html.len() < MAX_HTML_BYTES - 64 {
        html.push_str("<tr><td>cell text<td><table><tr><td>n</table></tr>");
    }
    html.push_str("</table>");
    assert!(html.len() <= MAX_HTML_BYTES);
    let input = RenderInput {
        html: Some(html),
        ..Default::default()
    };
    let (r, took) = big_stack(move || {
        let t = Instant::now();
        let opts = Opts::default();
        let r = render::layout(
            &input,
            &opts,
            &mut Engine::new(),
            &Deadline::after(Duration::from_millis(10)),
        );
        (r.map(|_| ()), t.elapsed())
    });
    assert_eq!(r, Err(RenderError::Deadline));
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

/// Thousands of long descendant and sibling selectors over a deep or wide
/// tree must be cut off by the deadline within a bounded overshoot.
#[test]
fn selector_bombs_respect_the_deadline() {
    let style = "<style>".to_string()
        + &"div div div div div div div div div div div div div div div div{color:red}
"
        .repeat(1500)
        + &"div + div + div + div + div ~ div ~ div ~ div{color:red}
"
        .repeat(1500)
        + "</style>";
    let bodies = ["<div>".repeat(2000) + "x", "<div>x</div>".repeat(20_000)];
    for body in bodies {
        let input = RenderInput {
            html: Some(format!("{style}{body}")),
            ..Default::default()
        };
        let (r, took) = big_stack(move || {
            let t = Instant::now();
            let r = render::layout(
                &input,
                &Opts::default(),
                &mut Engine::new(),
                &Deadline::after(LAYOUT_BUDGET),
            );
            (r.map(|_| ()), t.elapsed())
        });
        // Cut off, or finished within budget: never a long overrun.
        assert!(r.is_ok() || r == Err(RenderError::Deadline));
        assert!(took < Duration::from_secs(10), "took {took:?}");
    }
}

#[test]
fn oversized_and_empty_input_fall_back() {
    let big = RenderInput {
        html: Some("x".repeat(MAX_HTML_BYTES + 1)),
        ..Default::default()
    };
    assert_eq!(render(&big, 600.0).err(), Some(RenderError::TooLarge));
    assert_eq!(
        render(&RenderInput::default(), 600.0).err(),
        Some(RenderError::NoHtml)
    );
}

#[test]
fn line_breaks_and_preserved_text() {
    let d = big_stack(|| {
        let input = RenderInput {
            html: Some("<p>a<br>b</p><pre>x  y\nz</pre>".into()),
            ..Default::default()
        };
        render(&input, 600.0).unwrap()
    });
    assert_eq!(lines(&d), ["a", "b", "x  y", "z"]);
}

/// Word's pseudo-bullets: the bullet is literal text followed by a span of
/// `&nbsp;`s, which must keep their width (they touched the text: "•Ship").
#[test]
fn word_bullets_keep_their_gap() {
    for width in [600.0, 900.0] {
        let d = doc("06-outlook-word", width);
        let tl = d.text_lines();
        let ship = tl
            .iter()
            .find(|l| l.text.contains("Ship the plain-text fallback"))
            .expect("bullet line");
        let rest = ship.text.strip_prefix('\u{b7}').expect("bullet first");
        let gap = rest.chars().take_while(|&c| c == '\u{a0}').count();
        assert_eq!(gap, 7, "{:?}", ship.text);
        assert!(rest.trim_start().starts_with("Ship"), "{:?}", ship.text);
    }
    // No-break spaces at either edge of a span keep their width.
    let width = |html: &'static str| {
        big_stack(move || {
            let input = RenderInput {
                html: Some(html.into()),
                ..Default::default()
            };
            render(&input, 600.0).unwrap().text_lines()[0].w
        })
    };
    let tight = width("<p>a<span>b</span>c</p>");
    let font_size = 16.0;
    for spaced in [
        "<p>a<span>&nbsp;&nbsp;b</span>c</p>",
        "<p>a<span>b&nbsp;&nbsp;</span>c</p>",
        "<p>a<span>&nbsp; &nbsp;</span>b<span>c</span></p>",
    ] {
        assert!(width(spaced) >= tight + 0.4 * font_size, "{spaced}");
    }
    // Line separators next to them still collapse rather than break.
    let d = big_stack(|| {
        let input = RenderInput {
            html: Some("<p>a<span>&nbsp;\u{2028}\u{2029}\u{85}</span>b</p>".into()),
            ..Default::default()
        };
        render(&input, 600.0).unwrap()
    });
    assert_eq!(lines(&d).len(), 1, "{:?}", lines(&d));
}

/// Background colours at fixed probe points, from two 256-row tiles per
/// file at device scale 1.5.
#[test]
fn tiles_paint_backgrounds() {
    big_stack(|| {
        let mut engine = Engine::new();
        let scale = 1.5;
        for name in FILES {
            let input = input(&raw(name));
            let opts = Opts {
                width_px: 600.0,
                scale,
                ..Default::default()
            };
            let doc = render::layout(&input, &opts, &mut engine, &Deadline::none()).unwrap();
            let mid = ((doc.height * scale / 2.0) as u32).saturating_sub(128);
            for y in [0, mid] {
                let pm = render::rasterize(&doc, &mut engine, y, 256, scale).unwrap();
                assert_eq!(pm.width(), 900);
                assert_eq!(pm.height(), 256);
                let first = pm.pixels()[0];
                assert!(
                    pm.pixels().iter().any(|p| p != &first),
                    "{name}: blank tile at {y}"
                );
            }
            let pm = render::rasterize(&doc, &mut engine, 0, 256, scale).unwrap();
            let at = |x: f32, y: f32| {
                let p = pm
                    .pixel((x * scale) as u32, (y * scale) as u32)
                    .unwrap()
                    .demultiply();
                [p.red(), p.green(), p.blue()]
            };
            match name {
                "04-order-receipt" => {
                    assert_eq!(at(2.0, 2.0), [0xea, 0xed, 0xed]);
                    assert_eq!(at(300.0, 30.0), [0x23, 0x2f, 0x3e]);
                }
                "08-dark-marketing" => assert_eq!(at(3.0, 3.0), [0x0b, 0x0b, 0x12]),
                "12-personal-applemail" => assert_eq!(at(590.0, 3.0), [255, 255, 255]),
                _ => {}
            }
        }
    });
}

#[test]
fn every_file_renders_in_dark_mode_too() {
    big_stack(|| {
        let mut engine = Engine::new();
        for name in FILES {
            let opts = Opts {
                dark: true,
                ..Default::default()
            };
            let d =
                render::layout(&input(&raw(name)), &opts, &mut engine, &Deadline::none()).unwrap();
            assert_eq!(d.has_dark_rules, name == "08-dark-marketing", "{name}");
        }
    });
}

#[test]
fn link_texts_warnings_and_anchors() {
    big_stack(|| {
        let d = doc("14-adversarial-malformed", 600.0);
        let by_text = |t: &str| d.links.iter().find(|l| l.text == t);
        let userinfo = by_text("userinfo URL").unwrap();
        assert_eq!(
            userinfo.href,
            "https://bank.example.com@evil.example.invalid/"
        );
        let web = links::web(&userinfo.href, &userinfo.text).unwrap();
        assert_eq!(web.host, "evil.example.invalid");
        assert_eq!(web.warnings, [Warning::UserInfo]);
        let idn = by_text("IDN homograph").unwrap();
        let web = links::web(&idn.href, &idn.text).unwrap();
        assert_eq!(web.warnings, [Warning::Punycode]);
        for inert in [
            "Verify account",
            "entity-obfuscated js",
            "tab-obfuscated js",
            "vbscript",
            "data: html link",
            "file link",
            "cid link",
        ] {
            assert!(by_text(inert).is_none(), "{inert} is followable");
        }

        // `#fragment` targets: an empty `<a name>` inside a paragraph and
        // an `id` inside a table cell (laid out at the origin, then moved).
        let input = RenderInput {
            html: Some(
                "<p><a href='#b'>to b</a> <a href='#c'>to c</a> <a href='#zz'>none</a></p>\
                 <div style='height:900px'></div><p>text <a name='b'></a>here</p>\
                 <table><tr><td style='height:300px'>x</td></tr>\
                 <tr><td><span id='c'>c</span></td></tr></table>"
                    .into(),
            ),
            ..Default::default()
        };
        let d = render(&input, 600.0).unwrap();
        let b = d.anchor_y("#b").unwrap();
        let c = d.anchor_y("#c").unwrap();
        assert!(b >= 900.0 && c >= b + 300.0, "b {b}, c {c}");
        assert_eq!(d.anchor_y("#zz"), None);
        assert_eq!(d.anchors, ["b", "c"]);
    });
}

/// Fixed-width mail in a narrow reader column: the receipt, the statement
/// and the promo cannot shrink to 400 px (the statement and the promo not
/// even to the 464 px column of a 900 px window), so `natural_width` says
/// how wide they really are; laid out at that width (at most 1000 px, as
/// the reader does before drawing the page scaled), nothing is cut at the
/// page edge, so e.g. the statement's amounts stay on the page. Mail that
/// reflows reports no overflow and is left alone.
#[test]
fn fixed_width_mail_reports_its_natural_width() {
    big_stack(|| {
        let mut engine = Engine::new();
        let mut at = |name: &'static str, width: f32| {
            let opts = Opts {
                width_px: width,
                ..Default::default()
            };
            render::layout(&input(&raw(name)), &opts, &mut engine, &Deadline::none()).unwrap()
        };
        for (name, column) in [
            ("04-order-receipt", 400.0),
            ("05-bank-statement", 400.0),
            ("09-image-heavy-promo", 400.0),
            ("05-bank-statement", 464.0),
            ("09-image-heavy-promo", 464.0),
        ] {
            let narrow = at(name, column);
            assert!(
                narrow.natural_width > column + 4.0,
                "{name} at {column}: natural {}",
                narrow.natural_width
            );
            let width = narrow.natural_width.ceil().min(1000.0);
            let page = at(name, width);
            assert!(
                page.natural_width <= page.width + 4.0,
                "{name} at {width}: still {} wide",
                page.natural_width
            );
            assert!(column / width < 1.0, "{name}: scale {}", column / width);
            if name.starts_with("05") {
                // The amounts column is on the page.
                assert!(
                    page.text_lines()
                        .iter()
                        .any(|l| l.text.contains("-18.47") && l.x + l.w <= page.width),
                    "{name}: amounts cut"
                );
            }
        }
        for name in [
            "06-outlook-word",
            "07-gmail-reply-chain",
            "11-newsletter-2col",
            "12-personal-applemail",
        ] {
            let d = at(name, 464.0);
            assert!(d.natural_width <= 468.0, "{name}: {}", d.natural_width);
        }
    });
}

/// Inline margins and padding push their neighbours aside, and
/// `text-transform` changes the drawn text.
#[test]
fn inline_spacing_and_text_transform() {
    let line = |html: &str| {
        let html = html.to_string();
        big_stack(move || {
            let input = RenderInput {
                html: Some(html),
                ..Default::default()
            };
            let d = render(&input, 600.0).unwrap();
            let tl = d.text_lines();
            assert_eq!(tl.len(), 1, "{:?}", lines(&d));
            (tl[0].w, tl[0].text.clone())
        })
    };
    let (plain, _) = line("<p><span>446 mtrs</span><span>&gt;</span></p>");
    let (spaced, _) = line(
        "<p><span style=\"margin-right:6px\">446 mtrs</span><span style=\"padding-left:10px\">&gt;</span></p>",
    );
    assert!((spaced - plain - 16.0).abs() < 1.0, "{plain} -> {spaced}");
    let (_, text) = line("<p style=\"text-transform:capitalize\">olá mário vitó silva,</p>");
    assert_eq!(text, "Olá Mário Vitó Silva,");
    let (_, text) = line("<p style=\"text-transform:lowercase\">MORADA</p>");
    assert_eq!(text, "morada");
}
