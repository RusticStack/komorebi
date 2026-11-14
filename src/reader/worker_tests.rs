//! The worker's tests (moved from the Slint app's `src/app/reader.rs` with
//! the worker; GPUI-TEST-PORT.md rows 204 to 223).

use super::*;
use crate::mail::render::{Rect, TextLine};
use crate::reader::{MAX_TEXT_LINES, PixelOrder};
use std::cell::RefCell;
use std::time::Duration;

fn key(msg: i64, width: u32) -> Key {
    Key {
        msg,
        width,
        scale: 1f32.to_bits(),
        dark: false,
        dim: false,
        remote: false,
    }
}

/// The worker loop without remote images.
fn run_plain(rx: Receiver<Job>, latest: &AtomicU64, notify: &dyn Fn(Reply)) {
    run(
        rx,
        latest,
        notify,
        &Mutex::default(),
        &|_, _| {},
        Config::default(),
    );
}

fn html(body: &str) -> Source {
    Source::Input(Arc::new(RenderInput {
        html: Some(body.into()),
        ..Default::default()
    }))
}

#[test]
fn lru_accounts_bytes_and_evicts_oldest() {
    let mut c = Lru::new(10);
    c.insert(1, 'a', 4);
    c.insert(2, 'b', 4);
    assert_eq!(c.cost, 8);
    assert_eq!(c.get(&1), Some(&'a'));
    // Over the cap: the least recently used entry (2) goes.
    c.insert(3, 'c', 4);
    assert_eq!(c.cost, 8);
    assert_eq!(c.get(&2), None);
    // Replacing an entry swaps its cost.
    c.insert(1, 'A', 6);
    assert_eq!(c.cost, 10);
    assert_eq!(c.get(&1), Some(&'A'));
    // An entry over the cap on its own is kept, alone.
    c.insert(4, 'd', 20);
    assert_eq!((c.cost, c.entries.len()), (20, 1));

    // The tile cache holds about 24 full-width tiles at 100%.
    let tile = 960 * 256 * 4;
    let mut tiles = Lru::new(TILE_CACHE_BYTES);
    for i in 0..40 {
        tiles.insert(i, (), tile);
    }
    let kept = TILE_CACHE_BYTES / tile;
    assert_eq!(tiles.entries.len(), kept);
    assert!(tiles.cost <= TILE_CACHE_BYTES);
    assert_eq!(tiles.entries.front().map(|e| e.0), Some(40 - kept));
}

#[test]
fn worker_drops_stale_generations() {
    let (tx, rx) = mpsc::channel();
    let raw: &[u8] = b"Content-Type: text/html\r\n\r\n<p>new</p>";
    let jobs = [
        Job::Layout {
            generation: 1,
            key: key(1, 320),
            src: html("<p>old</p>"),
        },
        Job::Tiles {
            generation: 1,
            key: key(1, 320),
            tiles: vec![0],
        },
        Job::Layout {
            generation: 2,
            key: key(2, 480),
            src: Source::Raw(Arc::from(raw)),
        },
        Job::Tiles {
            generation: 2,
            key: key(2, 480),
            tiles: vec![0, 1],
        },
    ];
    for job in jobs {
        tx.send(job).unwrap();
    }
    drop(tx);
    // The UI side has moved on to generation 2 already.
    let latest = AtomicU64::new(2);
    let replies = RefCell::new(Vec::new());
    run_plain(rx, &latest, &|r| replies.borrow_mut().push(r));
    let replies = replies.into_inner();
    assert_eq!(replies.len(), 3, "parsed, page and one tile");
    assert!(
        matches!(&replies[0], Reply::Parsed { generation: 2, body } if body.text.contains("new"))
    );
    assert!(matches!(&replies[1], Reply::Page { generation: 2, page, .. } if page.width == 480.0));
    // The page is one tile tall; tile 1 does not exist.
    assert!(matches!(
        &replies[2],
        Reply::Tile { generation: 2, index: 0, pixels } if pixels.width == 480
    ));
    assert!(current(3, &latest) && current(2, &latest) && !current(1, &latest));
}

#[test]
fn worker_reports_text_fallbacks() {
    let (tx, rx) = mpsc::channel();
    let plain = Source::Input(Arc::new(RenderInput {
        text: "hi".into(),
        prefers_plain: true,
        ..Default::default()
    }));
    let big = html(&"x".repeat(render::MAX_HTML_BYTES + 1));
    for (generation, src) in [(1, plain), (2, big)] {
        let key = key(generation as i64, 480);
        tx.send(Job::Layout {
            generation,
            key,
            src,
        })
        .unwrap();
    }
    drop(tx);
    let replies = RefCell::new(Vec::new());
    run_plain(rx, &AtomicU64::new(0), &|r| replies.borrow_mut().push(r));
    let errors: Vec<_> = replies
        .into_inner()
        .into_iter()
        .map(|r| match r {
            Reply::Failed { error, .. } => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(
        errors,
        [Some(RenderError::NoHtml), Some(RenderError::TooLarge)]
    );
}

// ----------------------------------------------------------- selection

fn corpus_12() -> Arc<[u8]> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/html/12-personal-applemail.eml");
    std::fs::read(path).unwrap().into()
}

/// What the worker answers to selection `ops` on corpus 12 at 600 px:
/// highlight rectangle counts (`Ok`) and copied texts (`Err`), in order.
fn select_on_12(ops: Vec<SelectOp>) -> Vec<Result<usize, String>> {
    let key = key(12, 600);
    let mut jobs = vec![Job::Layout {
        generation: 1,
        key,
        src: Source::Raw(corpus_12()),
    }];
    jobs.extend(ops.into_iter().map(|op| Job::Select {
        generation: 1,
        key,
        op,
    }));
    run_to_end(jobs, PixelOrder::Rgba)
        .into_iter()
        .filter_map(|r| match r {
            Reply::Selection { rects, .. } => Some(Ok(rects.len())),
            Reply::Copied { text, .. } => Some(Err(text)),
            _ => None,
        })
        .collect()
}

/// Runs `f` on a thread with the worker's stack (layout recurses).
fn on_worker_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(WORKER_STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Corpus 12 laid out as the worker does for `key(12, 600)`.
fn doc_12() -> Doc {
    on_worker_stack(|| {
        let input = mime::parse_render_input(&corpus_12());
        lay_out(
            &input,
            key(12, 600),
            &mut Engine::new(),
            &Images::default(),
            None,
            None,
        )
        .unwrap()
    })
}

#[test]
fn worker_selects_and_copies_across_layouts() {
    let doc = doc_12();
    let lines: Vec<TextLine> = doc.text_lines();
    let find = |p: &str| lines.iter().find(|l| l.text.starts_with(p)).unwrap();
    let (first, last) = (find("Olá Ana"), find("Ligas-me"));
    // The point on the "Envio" line where a double-click takes "Gerês".
    let envio = find("Envio");
    let geres = (1..100)
        .map(|i| {
            (
                envio.x + envio.w * i as f32 / 100.0,
                envio.y + envio.h / 2.0,
            )
        })
        .find(|&(x, y)| {
            doc.word_at(x, y)
                .is_some_and(|(a, b)| doc.selection_text(a, b).trim_end() == "Gerês")
        })
        .unwrap();
    let replies = select_on_12(vec![
        // Drag from the left margin of the first line to past the end
        // of the fourth paragraph, then copy.
        SelectOp::Press {
            x: first.x - 4.0,
            y: first.y + 2.0,
            clicks: 1,
        },
        SelectOp::Extend {
            x: last.x + last.w + 30.0,
            y: last.y + 2.0,
        },
        SelectOp::Copy,
        // A double-click takes one word.
        SelectOp::Press {
            x: geres.0,
            y: geres.1,
            clicks: 2,
        },
        SelectOp::Copy,
        // Nothing selected: nothing to copy.
        SelectOp::Clear,
        SelectOp::Copy,
        SelectOp::All,
    ]);
    let [
        Ok(0),
        Ok(dragged),
        Err(copied),
        Ok(1),
        Err(word),
        Ok(0),
        Ok(all),
    ] = &replies[..]
    else {
        panic!("unexpected replies {replies:?}");
    };
    assert!(*dragged >= 4);
    assert_eq!(
        copied,
        "Olá Ana,\nObrigada pelo jantar de sábado - foi mesmo bom rever-te! 😊\n\
         Envio as fotos do Gerês neste álbum. A da cascata ficou incrível.\n\
         Ligas-me amanhã? Estou livre depois das 18h."
    );
    assert_eq!(word.trim_end(), "Gerês");
    assert!(*all > *dragged);
}

#[test]
fn a_new_layout_drops_the_worker_selection() {
    let key = key(12, 600);
    let layout = || Job::Layout {
        generation: 1,
        key,
        src: Source::Raw(corpus_12()),
    };
    let select = |op| Job::Select {
        generation: 1,
        key,
        op,
    };
    let replies = run_to_end(
        vec![
            layout(),
            select(SelectOp::All),
            layout(),
            select(SelectOp::Copy),
        ],
        PixelOrder::Rgba,
    );
    assert!(!replies.iter().any(|r| matches!(r, Reply::Copied { .. })));
}

// --------------------------------------------------------------- links

#[test]
fn worker_sends_links_with_text_and_anchors() {
    let replies = run_to_end(
        vec![Job::Layout {
            generation: 1,
            key: key(3, 600),
            src: html(
                "<p><a href='#end'>Jump <b>down</b></a> or \
                 <a href=' https://example.com/a b '>example.com</a></p>\
                 <div style='height:3000px'></div>\
                 <table><tr><td><h2 id='end'>The end</h2></td></tr></table>\
                 <p><a href='mailto:a@example.com?subject=Hi'>Write</a> \
                 <a href='java&#9;script:x'>bad</a> <a href='#nowhere'>gone</a></p>",
            ),
        }],
        PixelOrder::Rgba,
    );
    let [Reply::Page { links, .. }] = &replies[..] else {
        panic!("expected a page");
    };
    let hrefs: Vec<&str> = links.iter().map(|l| l.href.as_str()).collect();
    assert_eq!(
        hrefs,
        [
            "#end",
            "https://example.com/a%20b",
            "mailto:a@example.com?subject=Hi",
            "#nowhere"
        ]
    );
    assert_eq!(links[0].text, "Jump down");
    assert_eq!(links[1].text, "example.com");
    assert!(links.iter().all(|l| !l.rects.is_empty()));
    // The anchor sits inside a table cell, below the 3000 px spacer.
    let end = links[0].target.unwrap();
    assert!(end > 3000.0 && end < links[2].rects[0].y, "anchor at {end}");
    assert_eq!(links[3].target, None);
}

// ------------------------------------------------------ scale to fit

/// A 600 px fixed-width table (painted, so its box is what overflows):
/// two paragraphs, the second with a link.
pub(crate) const FIXED: &str = "<table width='600' style='width:600px;background:#eef' \
     cellpadding='0' cellspacing='0'><tr><td><p>Alpha beta gamma delta epsilon zeta eta \
     theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega, \
     and enough words after them to wrap onto a second line.</p>\
     <p>Second paragraph with <a href='https://example.com/x'>a link</a> in it.</p>\
     </td></tr></table>";

fn layout_job(key: Key, body: &str) -> Job {
    Job::Layout {
        generation: 1,
        key,
        src: html(body),
    }
}

fn page_and_links(replies: &[Reply]) -> (Page, Vec<super::super::PageLink>) {
    replies
        .iter()
        .find_map(|r| match r {
            Reply::Page { page, links, .. } => Some((*page, links.clone())),
            _ => None,
        })
        .expect("a page")
}

fn worker_replies(jobs: Vec<Job>) -> Vec<Reply> {
    run_to_end(jobs, PixelOrder::Rgba)
}

#[test]
fn worker_scales_fixed_width_mail_to_the_column() {
    let tile = |replies: &[Reply]| {
        replies
            .iter()
            .find_map(|r| match r {
                Reply::Tile { pixels, .. } => Some((pixels.width, pixels.height)),
                _ => None,
            })
            .unwrap()
    };
    let at = |width: u32| {
        let key = key(1, width);
        let tiles = Job::Tiles {
            generation: 1,
            key,
            tiles: vec![0],
        };
        worker_replies(vec![layout_job(key, FIXED), tiles])
    };
    let (wide, narrow) = (at(600), at(450));
    let ((p600, _), (p450, _)) = (page_and_links(&wide), page_and_links(&narrow));
    // At 450 px the 600 px table is drawn at 0.75, whole: the page is
    // the column wide and its tiles are rasterized at that width.
    assert_eq!(p600.width, 600.0);
    assert_eq!(p450.width, 450.0);
    assert!(
        (p450.height - p600.height * 0.75).abs() < 0.5,
        "{p450:?} {p600:?}"
    );
    assert_eq!(tile(&wide).0, 600);
    assert_eq!(tile(&narrow).0, 450);
    // At 125% the tile is the column's device width: drawn 1:1.
    let k125 = Key {
        scale: 1.25f32.to_bits(),
        ..key(5, 480)
    };
    let tiles = Job::Tiles {
        generation: 1,
        key: k125,
        tiles: vec![0],
    };
    let r125 = worker_replies(vec![layout_job(k125, FIXED), tiles]);
    assert_eq!(page_and_links(&r125).0.width, 480.0);
    assert_eq!(tile(&r125).0, 600);
    // Mail that reflows is laid out at the column, unscaled.
    let reflow = worker_replies(vec![layout_job(key(2, 450), "<p>Short text.</p>")]);
    assert_eq!(page_and_links(&reflow).0.width, 450.0);
    // Below the 0.6 floor the page is wider than the column.
    let huge = "<table style='width:1000px;background:#eef'><tr><td>Statement</td></tr></table>";
    let pinned = worker_replies(vec![layout_job(key(3, 320), huge)]);
    assert_eq!(page_and_links(&pinned).0.width, 600.0);
    // Content sized relative to the page (wider on a wider page) cannot
    // be made to fit: it stays at the column, as before.
    let relative = "<div style='width:300%;background:#eef'>Wide</div><p>Text</p>";
    let rel = worker_replies(vec![layout_job(key(4, 400), relative)]);
    assert_eq!(page_and_links(&rel).0.width, 400.0);
}

#[test]
fn tiles_come_in_the_order_asked_for() {
    let key = key(1, 320);
    let jobs = |order| {
        let tiles = Job::Tiles {
            generation: 1,
            key,
            tiles: vec![0],
        };
        let body = "<div style='background:#204080;height:40px'>x</div>";
        run_to_end(vec![layout_job(key, body), tiles], order)
    };
    let pixels = |replies: Vec<Reply>| {
        replies
            .into_iter()
            .find_map(|r| match r {
                Reply::Tile { pixels, .. } => Some(pixels),
                _ => None,
            })
            .unwrap()
    };
    let (rgba, bgra) = (
        pixels(jobs(PixelOrder::Rgba)),
        pixels(jobs(PixelOrder::Bgra)),
    );
    assert_eq!((rgba.width, rgba.height), (bgra.width, bgra.height));
    // The first pixel is the box's blue: red and blue change places.
    assert_eq!(&rgba.data[..4], &[0x20, 0x40, 0x80, 0xff]);
    assert_eq!(&bgra.data[..4], &[0x80, 0x40, 0x20, 0xff]);
}

#[test]
fn drag_at_scale_selects_the_same_text() {
    // Points on the page laid out at 600 px (scale 1).
    let doc = on_worker_stack(|| {
        let input = RenderInput {
            html: Some(FIXED.into()),
            ..Default::default()
        };
        lay_out(
            &input,
            key(1, 600),
            &mut Engine::new(),
            &Images::default(),
            None,
            None,
        )
        .unwrap()
    });
    let lines = doc.text_lines();
    let (first, second) = (&lines[0], &lines[1]);
    let from = (first.x + first.w * 0.3, first.y + first.h / 2.0);
    let to = (second.x + second.w * 0.5, second.y + second.h / 2.0);
    let drag = |width: u32, s: f32| {
        let key = key(1, width);
        let select = |op| Job::Select {
            generation: 1,
            key,
            op,
        };
        let replies = worker_replies(vec![
            layout_job(key, FIXED),
            select(SelectOp::Press {
                x: from.0 * s,
                y: from.1 * s,
                clicks: 1,
            }),
            select(SelectOp::Extend {
                x: to.0 * s,
                y: to.1 * s,
            }),
            select(SelectOp::Copy),
        ]);
        let rects = replies.iter().rev().find_map(|r| match r {
            Reply::Selection { rects, .. } => Some(rects.clone()),
            _ => None,
        });
        let text = replies.iter().find_map(|r| match r {
            Reply::Copied { text, .. } => Some(text.clone()),
            _ => None,
        });
        (rects.unwrap(), text.unwrap())
    };
    let (rects1, text1): (Vec<Rect>, String) = drag(600, 1.0);
    let (rects75, text75) = drag(450, 0.75);
    assert!(
        text1.contains("omicron pi") && text1.len() > 40,
        "{text1:?}"
    );
    assert_eq!(text75, text1);
    // The highlight comes back in view coordinates.
    assert_eq!(rects75.len(), rects1.len());
    for (a, b) in rects75.iter().zip(&rects1) {
        assert!((a.x - b.x * 0.75).abs() < 0.01 && (a.w - b.w * 0.75).abs() < 0.01);
        assert!((a.y - b.y * 0.75).abs() < 0.01 && (a.h - b.h * 0.75).abs() < 0.01);
    }
}

#[test]
fn links_are_hit_under_scale() {
    // The worker half of the Slint test: the link's hit area comes back
    // scaled with the page (the UI half is in `session`).
    let (_, l600) = page_and_links(&worker_replies(vec![layout_job(key(1, 600), FIXED)]));
    let (page, l450) = page_and_links(&worker_replies(vec![layout_job(key(1, 450), FIXED)]));
    let (a, b) = (l600[0].rects[0], l450[0].rects[0]);
    assert_eq!(l450[0].href, "https://example.com/x");
    assert!((b.x - a.x * 0.75).abs() < 0.01 && (b.y - a.y * 0.75).abs() < 0.01);
    assert!((b.w - a.w * 0.75).abs() < 0.01 && (b.h - a.h * 0.75).abs() < 0.01);
    assert!(b.x + b.w <= page.width);
}

// ------------------------------------------------------ remote images

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, 2, 2);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .unwrap()
        .write_image_data(&[200; 16])
        .unwrap();
    out
}

#[test]
fn worker_fetches_consented_images_once_and_lays_out_again() {
    let src = html(
        r#"<p>hi</p><img src="https://img.test/a.png" width="20" height="20">
           <img src="https://img.test/pixel.gif" width="1" height="1">
           <div style="background-image:url(https://img.test/bg.png)">x</div>
           <link rel="stylesheet" href="https://img.test/s.css">"#,
    );
    let allowed = Key {
        remote: true,
        ..key(1, 480)
    };
    let (tx, rx) = mpsc::channel();
    for (generation, key) in [(1, key(1, 480)), (2, allowed), (3, allowed), (4, allowed)] {
        let src = src.clone();
        tx.send(Job::Layout {
            generation,
            key,
            src,
        })
        .unwrap();
        tx.send(Job::Tiles {
            generation,
            key,
            tiles: vec![0],
        })
        .unwrap();
    }
    drop(tx);
    let images = Mutex::default();
    let started = RefCell::new(Vec::new());
    // The fake fetch completes at once, before the next layout job.
    let start = |msg: i64, urls: Vec<String>| {
        let mut im = lock(&images);
        for u in &urls {
            im.store(u, Some(png()));
        }
        started.borrow_mut().push((msg, urls));
    };
    let replies = RefCell::new(Vec::new());
    run(
        rx,
        &AtomicU64::new(0),
        &|r| replies.borrow_mut().push(r),
        &images,
        &start,
        Config::default(),
    );
    // Only with consent, only once, and never the tracking pixel or the
    // stylesheet.
    assert_eq!(
        started.into_inner(),
        [(
            1,
            vec![
                "https://img.test/a.png".to_string(),
                "https://img.test/bg.png".to_string()
            ]
        )]
    );
    let pages: Vec<(u64, u32)> = replies
        .borrow()
        .iter()
        .filter_map(|r| match r {
            Reply::Page {
                generation, page, ..
            } => Some((*generation, page.remote_images)),
            _ => None,
        })
        .collect();
    // Blocked, then fetching, then shown (laid out again), then cached.
    assert_eq!(pages, [(1, 2), (2, 2), (3, 0), (4, 0)]);
    // The tiles of the stale layout were dropped, not served again.
    let tiles: Vec<Vec<u8>> = replies
        .borrow()
        .iter()
        .filter_map(|r| match r {
            Reply::Tile { pixels, .. } => Some(pixels.data.to_vec()),
            _ => None,
        })
        .collect();
    assert_eq!(tiles.len(), 4);
    assert!(tiles[0] == tiles[1] && tiles[1] != tiles[2] && tiles[2] == tiles[3]);
    assert_eq!(lock(&images).msg, 1);
}

#[test]
fn fetched_images_are_capped_in_bytes() {
    let mut im = Images::default();
    im.store("a", Some(vec![0; MAX_FETCHED_BYTES - 1]));
    im.store("b", Some(vec![0; 2]));
    im.store("c", None);
    assert!(im.loaded("a") && !im.loaded("b") && !im.loaded("c"));
    assert!(matches!(im.urls.get("b"), Some(Fetched::Failed)));
}

#[test]
fn fetch_pool_runs_four_at_a_time_for_the_current_message() {
    let images: Arc<Mutex<Images>> = Arc::default();
    lock(&images).msg = 5;
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (a, p, c) = (active.clone(), peak.clone(), calls.clone());
    let fetch: Fetch = Arc::new(move |url: &str| {
        c.fetch_add(1, Ordering::SeqCst);
        let now = a.fetch_add(1, Ordering::SeqCst) + 1;
        p.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        a.fetch_sub(1, Ordering::SeqCst);
        (!url.ends_with("bad")).then(|| url.as_bytes().to_vec())
    });
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let notify: Notify = Arc::new(move |r| {
        if let Reply::Fetched { msg } = r {
            tx.lock().unwrap().send(msg).unwrap();
        }
    });
    let mut urls: Vec<String> = (0..9).map(|i| format!("https://img.test/{i}")).collect();
    urls.push("https://img.test/bad".into());
    fetch_images(5, urls, &images, &fetch, &notify);
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(5));
    assert_eq!(calls.load(Ordering::SeqCst), 10);
    assert!(peak.load(Ordering::SeqCst) <= FETCHERS);
    let im = lock(&images);
    assert!((0..9).all(|i| im.loaded(&format!("https://img.test/{i}"))));
    assert!(matches!(
        im.urls.get("https://img.test/bad"),
        Some(Fetched::Failed)
    ));
    drop(im);

    // Another message took the cache: nothing more is fetched or kept.
    lock(&images).msg = 6;
    fetch_images(
        5,
        vec!["https://img.test/x".into()],
        &images,
        &fetch,
        &notify,
    );
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(5));
    assert_eq!(calls.load(Ordering::SeqCst), 10);
    assert!(!lock(&images).urls.contains_key("https://img.test/x"));
}

#[test]
fn worker_counts_images_that_could_not_be_loaded() {
    let src = html(
        r#"<p>hi</p><img src="https://img.test/a.png" width="20" height="20">
           <img src="https://img.test/gone.png" width="20" height="20">"#,
    );
    let allowed = Key {
        remote: true,
        ..key(1, 480)
    };
    let (tx, rx) = mpsc::channel();
    for generation in [1, 2] {
        let src = src.clone();
        tx.send(Job::Layout {
            generation,
            key: allowed,
            src,
        })
        .unwrap();
    }
    // Another message without consent: nothing is counted.
    tx.send(Job::Layout {
        generation: 3,
        key: key(2, 480),
        src,
    })
    .unwrap();
    drop(tx);
    let images = Mutex::default();
    // One image loads, the other fails (404, refused, too large...).
    let start = |_: i64, urls: Vec<String>| {
        let mut im = lock(&images);
        for u in &urls {
            im.store(u, u.ends_with("a.png").then(png));
        }
    };
    let replies = RefCell::new(Vec::new());
    run(
        rx,
        &AtomicU64::new(0),
        &|r| replies.borrow_mut().push(r),
        &images,
        &start,
        Config::default(),
    );
    let pages: Vec<(u32, bool, u32)> = replies
        .borrow()
        .iter()
        .filter_map(|r| match r {
            Reply::Page { page, .. } => {
                Some((page.remote_images, page.pending_images, page.failed_images))
            }
            _ => None,
        })
        .collect();
    // Fetching (nothing failed yet), then one image in and one failed.
    assert_eq!(pages, [(2, true, 0), (1, false, 1), (2, false, 0)]);
    // The other message took the image cache: the fetched bytes are gone.
    assert_eq!(lock(&images).msg, 2);
    assert!(lock(&images).urls.is_empty());
}

#[test]
fn worker_frees_the_previous_message_and_everything_on_cancel() {
    let jobs = vec![
        Job::Layout {
            generation: 1,
            key: key(1, 320),
            src: html("<p>one</p>"),
        },
        Job::Layout {
            generation: 2,
            key: key(2, 320),
            src: html("<p>two</p>"),
        },
        // The first message's page was freed on the switch.
        Job::Tiles {
            generation: 2,
            key: key(1, 320),
            tiles: vec![0],
        },
        Job::Tiles {
            generation: 2,
            key: key(2, 320),
            tiles: vec![0],
        },
        // The reader closed: the second one's page and tiles go too.
        Job::Cancel { generation: 3 },
        Job::Tiles {
            generation: 3,
            key: key(2, 320),
            tiles: vec![0],
        },
    ];
    let kinds: Vec<&str> = run_to_end(jobs, PixelOrder::Rgba)
        .iter()
        .map(|r| match r {
            Reply::Page { .. } => "page",
            Reply::Tile { .. } => "tile",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["page", "page", "tile"]);
}

#[test]
fn worker_forwards_cancel_only_to_a_running_worker() {
    let replies = Arc::new(Mutex::new(Vec::new()));
    let r = replies.clone();
    let notify: Notify = Arc::new(move |reply| r.lock().unwrap().push(reply));
    let mut worker = Worker::new(notify, Arc::new(|_: &str| None));
    worker.send(Job::Cancel { generation: 1 });
    assert!(worker.tx.is_none(), "a cancel starts no thread");
    assert_eq!(worker.latest.load(Ordering::Relaxed), 1);
}

#[test]
fn text_lines_follow_the_page_in_view_coordinates() {
    let at = |width: u32| {
        let (tx, rx) = mpsc::channel();
        tx.send(layout_job(key(1, width), FIXED)).unwrap();
        drop(tx);
        let replies = RefCell::new(Vec::new());
        let config = Config {
            text: true,
            ..Config::default()
        };
        let images = Mutex::default();
        run(
            rx,
            &AtomicU64::new(0),
            &|r| replies.borrow_mut().push(r),
            &images,
            &|_, _| {},
            config,
        );
        replies.into_inner()
    };
    let lines = |replies: &[Reply]| -> Vec<TextLine> {
        let [Reply::Page { .. }, Reply::Text { lines, .. }] = replies else {
            panic!("expected a page, then its text");
        };
        lines.clone()
    };
    let (l600, l450) = (lines(&at(600)), lines(&at(450)));
    assert!(l600.len() >= 3, "{l600:?}");
    assert!(l600[0].text.starts_with("Alpha beta"));
    assert_eq!(l600.len(), l450.len());
    // At 450 px the page is drawn at 0.75: so are its lines.
    for (a, b) in l600.iter().zip(&l450) {
        assert_eq!(a.text, b.text);
        assert!((b.y - a.y * 0.75).abs() < 0.01 && (b.h - a.h * 0.75).abs() < 0.01);
    }
    // Without text lines asked for, the page comes alone.
    let plain = run_to_end(vec![layout_job(key(1, 600), FIXED)], PixelOrder::Rgba);
    assert!(matches!(&plain[..], [Reply::Page { .. }]));
}

/// Screen readers' text selection: a selection on the page maps onto the
/// lines of `text_lines` as (line, character), a word to its own
/// characters, the whole text from the first line's start to the last's
/// end; the worker sends it after the highlight when it sends the lines.
#[test]
fn the_selection_maps_onto_the_text_lines() {
    let doc = on_worker_stack(|| {
        let input = RenderInput {
            html: Some(FIXED.into()),
            ..Default::default()
        };
        lay_out(
            &input,
            key(1, 600),
            &mut Engine::new(),
            &Images::default(),
            None,
            None,
        )
        .unwrap()
    });
    let lines = doc.text_lines();
    assert!(lines.len() >= 3, "{lines:?}");
    let (a, b) = doc.text_bounds().expect("text");
    let last = lines.len() - 1;
    let end = lines[last].text.chars().count();
    assert_eq!(
        doc.line_positions(a, b, MAX_TEXT_LINES),
        Some(((0, 0), (last, end)))
    );
    // Either order reads the same.
    assert_eq!(
        doc.line_positions(b, a, MAX_TEXT_LINES),
        Some(((0, 0), (last, end)))
    );
    // An end past the line cap goes to the last line given; a selection
    // starting past it has none there.
    let capped = lines[last - 1].text.chars().count();
    assert_eq!(
        doc.line_positions(a, b, last),
        Some(((0, 0), (last - 1, capped)))
    );
    assert_eq!(doc.line_positions(b, b, last), None);
    // A word on the first line: its characters there.
    let l = &lines[0];
    let (w0, w1) = doc
        .word_at(l.x + l.w * 0.5, l.y + l.h / 2.0)
        .expect("a word");
    let ((i0, c0), (i1, c1)) = doc.line_positions(w0, w1, MAX_TEXT_LINES).unwrap();
    assert_eq!((i0, i1), (0, 0));
    let word: String = l.text.chars().skip(c0).take(c1 - c0).collect();
    assert_eq!(word, doc.selection_text(w0, w1).trim());

    // The worker sends it with the lines asked for, and only then.
    let jobs = |text: bool| {
        let (tx, rx) = mpsc::channel();
        let k = key(1, 600);
        tx.send(layout_job(k, FIXED)).unwrap();
        for op in [SelectOp::All, SelectOp::Clear] {
            tx.send(Job::Select {
                generation: 1,
                key: k,
                op,
            })
            .unwrap();
        }
        drop(tx);
        let replies = RefCell::new(Vec::new());
        let config = Config {
            text,
            ..Config::default()
        };
        let images = Mutex::default();
        run(
            rx,
            &AtomicU64::new(0),
            &|r| replies.borrow_mut().push(r),
            &images,
            &|_, _| {},
            config,
        );
        replies
            .into_inner()
            .into_iter()
            .filter_map(|r| match r {
                Reply::TextSelection { range, .. } => Some(range),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(jobs(true), [Some(((0, 0), (last, end))), None]);
    assert!(jobs(false).is_empty());
}

/// A resize lays the message out again from the layout still cached: the
/// result is the layout from scratch.
#[test]
fn a_relayout_from_the_cached_layout_matches_one_from_scratch() {
    on_worker_stack(|| {
        let input = mime::parse_render_input(&corpus_12());
        let mut engine = Engine::new();
        let images = Images::default();
        let wide = lay_out(&input, key(12, 700), &mut engine, &images, None, None).unwrap();
        let again = lay_out(
            &input,
            key(12, 520),
            &mut engine,
            &images,
            Some(&wide),
            None,
        )
        .unwrap();
        let fresh = lay_out(
            &input,
            key(12, 520),
            &mut Engine::new(),
            &images,
            None,
            None,
        )
        .unwrap();
        assert!(again.reused > 0, "nothing was reused");
        assert_eq!(again.height, fresh.height);
        assert_eq!(again.natural_width, fresh.natural_width);
        let text =
            |d: &Doc| -> Vec<String> { d.text_lines().into_iter().map(|l| l.text).collect() };
        assert_eq!(text(&again), text(&fresh));
        assert_eq!(again.items.len(), fresh.items.len());
    });
}

const LONG_THREAD: &str = include_str!("../../tests/fixtures/threads/t60-thread.html");

#[test]
fn a_long_message_shows_its_top_first_and_draws_it_before_laying_out_the_rest() {
    assert!(LONG_THREAD.len() > PREVIEW_BYTES);
    let k = key(1, 600);
    let jobs = vec![
        layout_job(k, LONG_THREAD),
        Job::Tiles {
            generation: 1,
            key: k,
            tiles: vec![0, 1],
        },
    ];
    let replies = run_to_end(jobs, PixelOrder::Rgba);
    let seen: Vec<String> = replies
        .iter()
        .map(|r| match r {
            Reply::Page { page, .. } if page.partial => "first look".to_string(),
            Reply::Page { page, .. } => format!("whole {}", page.height > 10_000.0),
            Reply::Tile { index, .. } => format!("tile {index}"),
            _ => "other".to_string(),
        })
        .collect();
    assert_eq!(seen, ["first look", "tile 0", "tile 1", "whole true"]);
    let [Reply::Page { page: first, .. }, ..] = &replies[..] else {
        panic!("no page");
    };
    // Neither cut nor flagged as cut: the rest is on its way.
    assert!(!first.truncated);
    assert!(first.height >= PREVIEW_H && first.height < PREVIEW_H + 2000.0);
}

#[test]
fn the_first_look_at_a_long_message_draws_what_the_whole_page_draws() {
    let k = key(1, 600);
    let tile_of = |replies: &[Reply]| {
        replies.iter().find_map(|r| match r {
            Reply::Tile { pixels, .. } => Some(pixels.clone()),
            _ => None,
        })
    };
    let run_on = move |after_whole: bool| {
        std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let (tx, rx) = mpsc::channel();
                tx.send(layout_job(k, LONG_THREAD)).unwrap();
                let tiles = Job::Tiles {
                    generation: 1,
                    key: k,
                    tiles: vec![1],
                };
                let tx = RefCell::new(Some(tx));
                let mut later = Some(tiles);
                if !after_whole {
                    tx.borrow()
                        .as_ref()
                        .unwrap()
                        .send(later.take().unwrap())
                        .unwrap();
                    tx.borrow_mut().take();
                }
                let later = RefCell::new(later);
                let replies = RefCell::new(Vec::new());
                let notify = |r: Reply| {
                    if let Reply::Page { page, .. } = &r
                        && !page.partial
                        && let (Some(tx), Some(job)) =
                            (tx.borrow_mut().take(), later.borrow_mut().take())
                    {
                        tx.send(job).unwrap();
                    }
                    replies.borrow_mut().push(r);
                };
                run_plain(rx, &AtomicU64::new(0), &notify);
                replies.into_inner()
            })
            .unwrap()
            .join()
            .unwrap()
    };
    let early = run_on(false);
    let whole = run_on(true);
    let pages = |rs: &[Reply]| {
        rs.iter()
            .filter(|r| matches!(r, Reply::Page { .. }))
            .count()
    };
    assert_eq!((pages(&early), pages(&whole)), (2, 2));
    let (a, b) = (tile_of(&early).unwrap(), tile_of(&whole).unwrap());
    assert_eq!((a.width, a.height), (b.width, b.height));
    assert!(
        a.data == b.data,
        "tile 1 differs between the first look and the whole page"
    );
}
