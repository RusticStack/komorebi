//! The Slint app's reader view tests (`src/app/reader.rs`, GPUI-TEST-PORT.md
//! rows 207, 210, 212, 216, 220 and 224), on the session without a window:
//! the view's measurements become [`Frame`]s, its timers the effect
//! tokens, its clock an `Instant` passed in.

use super::*;
use crate::mail::render::RenderInput;
use crate::reader::{PixelOrder, RESIZE_DEBOUNCE, run_to_end};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Records jobs; the test plays the worker.
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Vec<Job>>>);

impl Backend for Fake {
    fn send(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }
}

impl Fake {
    fn take(&self) -> Vec<Job> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// Records the URLs it is asked to open.
#[derive(Clone, Default)]
struct NoShell(Rc<RefCell<Vec<String>>>);

impl Shell for NoShell {
    fn open_url(&self, url: &str) {
        self.0.borrow_mut().push(url.into());
    }
}

/// A session whose tile images are the tiles' indices.
fn session() -> (Session<u32>, Fake, NoShell) {
    let (fake, shell) = (Fake::default(), NoShell::default());
    let s = Session::new(Box::new(fake.clone()), Box::new(shell.clone()));
    (s, fake, shell)
}

fn html(body: &str) -> Source {
    Source::Input(Arc::new(RenderInput {
        html: Some(body.into()),
        ..Default::default()
    }))
}

fn layouts(jobs: Vec<Job>) -> Vec<(u64, Key)> {
    jobs.into_iter()
        .filter_map(|j| match j {
            Job::Layout {
                generation, key, ..
            } => Some((generation, key)),
            _ => None,
        })
        .collect()
}

fn tile_jobs(jobs: Vec<Job>) -> Vec<(u64, Vec<u32>)> {
    jobs.into_iter()
        .filter_map(|j| match j {
            Job::Tiles {
                generation, tiles, ..
            } => Some((generation, tiles)),
            _ => None,
        })
        .collect()
}

fn selects(jobs: Vec<Job>) -> Vec<SelectOp> {
    jobs.into_iter()
        .filter_map(|j| match j {
            Job::Select { op, .. } => Some(op),
            _ => None,
        })
        .collect()
}

fn page(width: f32, height: f32) -> Page {
    Page {
        width,
        height,
        bg: [255; 4],
        truncated: false,
        partial: false,
        remote_images: 0,
        pending_images: false,
        failed_images: 0,
        dimmed: false,
    }
}

fn shown(s: &Session<u32>, row: u32) -> bool {
    s.tiles()
        .get(row as usize)
        .is_some_and(|t| t.image.is_some())
}

fn token(effects: &[Effect], debounce: bool) -> Option<u64> {
    effects.iter().rev().find_map(|e| match (e, debounce) {
        (Effect::Debounce(t), true) | (Effect::Fallback(t), false) => Some(*t),
        _ => None,
    })
}

/// A frame of a 700 px column at 100 percent with the header in view.
const FRAME: Frame = Frame {
    column: 700.0,
    scale: 1.0,
    dark: false,
    top: -200.0,
    height: 600.0,
};

#[test]
fn tile_requests_follow_scroll_and_resize() {
    let (mut s, fake, _) = session();
    s.set_frame(FRAME);
    s.open(7, html("<p>x</p>"), false);
    assert!(s.waiting() && !s.html());
    let [(generation, key)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    assert_eq!(key.width, page_width(FRAME.column).unwrap());
    assert_eq!(key.scale(), 1.0);
    let fallback = token(&s.take_effects(), false).expect("the fallback timer");
    let p = page(key.width as f32, 5000.0);
    s.on_reply(
        Reply::Page {
            generation,
            page: p,
            links: vec![],
        },
        |_| 0,
    );
    assert!(s.html() && !s.waiting());
    assert_eq!(s.tiles().len(), 20);

    // The header sits above the page, so the view starts above it.
    let (top, height) = (FRAME.top, FRAME.height);
    let requests = tile_jobs(fake.take());
    let [(g, ref tiles)] = requests[..] else {
        panic!("expected one tile job");
    };
    assert_eq!(g, generation);
    assert_eq!(tiles, &wanted_tiles(top, height, p.height));
    assert_eq!(tiles[0], 0);
    for &index in tiles {
        s.show_tile(generation, index, index);
    }
    assert!(shown(&s, 0));
    s.set_frame(FRAME);
    assert!(fake.take().is_empty(), "nothing re-requested");
    // A late fallback finds the page up: nothing changes.
    s.fallback_due(fallback);
    assert!(s.html());

    // Scroll down: the tiles under the new view come first, and those
    // far above are dropped.
    let new_top = top + 3000.0;
    s.set_frame(Frame {
        top: new_top,
        ..FRAME
    });
    let requests = tile_jobs(fake.take());
    let [(_, ref tiles)] = requests[..] else {
        panic!("expected one tile job after scrolling");
    };
    assert_eq!(tiles[0], (new_top / TILE_H) as u32);
    assert_eq!(tiles, &wanted_tiles(new_top, height, p.height));
    assert!(!shown(&s, 0));
    assert!(s.take_dropped().contains(&0), "released from the GPU");
    let visible = tiles[0];
    s.show_tile(generation, visible, visible);
    assert!(shown(&s, visible));

    // Narrow the window: one relayout once the resize pauses.
    let narrow = Frame {
        column: 600.0,
        top: new_top,
        ..FRAME
    };
    s.set_frame(narrow);
    assert!(layouts(fake.take()).is_empty(), "debounced");
    let resize = token(&s.take_effects(), true).expect("the debounce");
    // Another step of the drag restarts it; the first timer is stale.
    s.set_frame(Frame {
        column: 601.0,
        ..narrow
    });
    s.set_frame(narrow);
    let latest = token(&s.take_effects(), true).expect("the debounce");
    s.resize_due(resize);
    assert!(layouts(fake.take()).is_empty(), "a stale timer");
    s.resize_due(latest);
    let [(g2, k2)] = layouts(fake.take())[..] else {
        panic!("expected a relayout");
    };
    assert_eq!(g2, generation + 1);
    assert!(k2.width < key.width);
    assert_eq!(k2.width, page_width(narrow.column).unwrap());
    // A tile of the old generation arriving late is ignored.
    s.show_tile(generation, visible + 1, visible + 1);
    assert!(!shown(&s, visible + 1));
    // The old tiles stay up, stretched, until new ones arrive.
    s.on_reply(
        Reply::Page {
            generation: g2,
            page: page(k2.width as f32, 6000.0),
            links: vec![],
        },
        |_| 0,
    );
    assert_eq!(s.tiles().len(), 24);
    assert!(shown(&s, visible));
    let requests = tile_jobs(fake.take());
    assert!(matches!(&requests[..], [(g, tiles)] if *g == g2 && tiles.contains(&visible)));

    // A display scale change re-renders at the new scale (the window
    // keeps its logical size, as on a move to another monitor).
    s.set_frame(Frame {
        scale: 1.5,
        ..narrow
    });
    let t = token(&s.take_effects(), true).expect("the debounce");
    s.resize_due(t);
    let relayouts = layouts(fake.take());
    assert_eq!(relayouts.len(), 1);
    assert_eq!(relayouts[0].1.scale(), 1.5);
    assert_eq!(relayouts[0].1.width, k2.width);
    let _ = RESIZE_DEBOUNCE;

    // No page within a second: the text view stands in.
    s.open(8, html("<p>y</p>"), false);
    assert!(s.waiting());
    let t = token(&s.take_effects(), false).expect("the fallback timer");
    s.fallback_due(t);
    assert!(!s.waiting() && !s.html() && s.text_shown());
    s.close();
    assert!(matches!(fake.take().last(), Some(Job::Cancel { .. })));
}

#[test]
fn page_gestures_and_keys_reach_the_worker_and_clipboard() {
    let (mut s, fake, _) = session();
    s.set_frame(FRAME);
    s.open(7, html("<p>x</p>"), false);
    let [(generation, key)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    let p = page(key.width as f32, 2000.0);
    s.on_reply(
        Reply::Page {
            generation,
            page: p,
            links: vec![],
        },
        |_| 0,
    );
    fake.take();

    // Press, drag and release over the page: page coordinates.
    let t0 = Instant::now();
    let (x, y) = (300.0, 200.0);
    s.press(x, y, t0);
    s.extend(x + 50.0, y + 60.0);
    s.release(x + 50.0, y + 60.0);
    assert_eq!(
        selects(fake.take()),
        [
            SelectOp::Press { x, y, clicks: 1 },
            SelectOp::Extend {
                x: x + 50.0,
                y: y + 60.0
            }
        ]
    );

    // Highlights from the worker show over the page.
    let rect = Rect {
        x: 10.0,
        y: 20.0,
        w: 100.0,
        h: 18.0,
    };
    s.on_reply(
        Reply::Selection {
            generation,
            rects: vec![rect],
        },
        |_| 0,
    );
    assert_eq!(s.selection().len(), 1);
    // A stale reply is ignored.
    s.on_reply(
        Reply::Selection {
            generation: generation - 1,
            rects: vec![],
        },
        |_| 0,
    );
    assert_eq!(s.selection().len(), 1);

    // A second and third press in place: word, then paragraph; a fourth
    // after the double-click time starts again.
    s.press(x, y, t0 + Duration::from_millis(100));
    s.press(x, y, t0 + Duration::from_millis(200));
    s.press(x, y, t0 + Duration::from_millis(200) + MULTI_CLICK);
    let clicks: Vec<u8> = selects(fake.take())
        .into_iter()
        .filter_map(|op| match op {
            SelectOp::Press { clicks, .. } => Some(clicks),
            _ => None,
        })
        .collect();
    assert_eq!(clicks, [2, 3, 1]);

    // Ctrl+A, Ctrl+C and Escape act on the selection.
    s.select(SelectOp::All);
    s.select(SelectOp::Copy);
    s.clear();
    assert_eq!(
        selects(fake.take()),
        [SelectOp::All, SelectOp::Copy, SelectOp::Clear]
    );

    // Copied text goes to the clipboard.
    s.on_reply(
        Reply::Copied {
            generation,
            text: "Olá Ana,\nObrigada".into(),
        },
        |_| 0,
    );
    assert_eq!(
        s.take_effects()
            .into_iter()
            .filter(|e| matches!(e, Effect::Copy(_)))
            .collect::<Vec<_>>(),
        [Effect::Copy("Olá Ana,\nObrigada".into())]
    );

    // The plain-text view hides the page and stops tile requests until
    // it is turned off again.
    s.set_plain(true);
    assert!(s.text_shown() && !s.page_shown());
    fake.take();
    s.set_frame(Frame {
        top: 1000.0,
        ..FRAME
    });
    assert!(tile_jobs(fake.take()).is_empty());
    s.set_plain(false);
    assert!(!tile_jobs(fake.take()).is_empty());

    // A new message drops the highlights.
    s.on_reply(
        Reply::Selection {
            generation,
            rects: vec![rect],
        },
        |_| 0,
    );
    s.open(8, html("<p>y</p>"), false);
    assert!(s.selection().is_empty());
}

fn link(href: &str, text: &str, y: f32, target: Option<f32>) -> PageLink {
    PageLink {
        href: href.into(),
        text: text.into(),
        rects: vec![Rect {
            x: 20.0,
            y,
            w: 120.0,
            h: 20.0,
        }],
        target,
    }
}

#[test]
fn links_hover_open_compose_and_take_keyboard_focus() {
    let (mut s, fake, shell) = session();
    s.set_frame(FRAME);
    s.open(7, html("<p>x</p>"), false);
    let [(generation, key)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key.width as f32, 3000.0),
            links: vec![
                link(
                    "https://evil.example.invalid/x",
                    "https://bank.example.com",
                    20.0,
                    None,
                ),
                link("mailto:ana@example.com?subject=Ol%C3%A1", "Ana", 60.0, None),
                link("#end", "End", 100.0, Some(1500.0)),
            ],
        },
        |_| 0,
    );
    s.take_effects();
    assert_eq!(s.links().len(), 3);
    let on = |y: f32| (70.0, y + 10.0);
    let now = std::cell::Cell::new(Instant::now());
    let click = |s: &mut Session<u32>, y: f32| {
        // Past the double-click interval, so each is a single click.
        now.set(now.get() + MULTI_CLICK);
        let (x, y) = on(y);
        s.press(x, y, now.get());
        s.release(x, y);
    };

    // Hover: pointer cursor and the real target in the strip.
    assert!(s.hover_at(Some(on(20.0))));
    assert!(s.hover_link());
    assert_eq!(s.target(), "https://evil.example.invalid/x");
    assert!(
        !s.hover_at(Some(on(21.0))),
        "the same link: nothing to redraw"
    );
    s.hover_at(Some(on(400.0)));
    assert!(!s.hover_link());
    assert_eq!(s.target(), "");

    // A web link asks first, with the host apart and the warning.
    click(&mut s, 20.0);
    let prompt = s.prompt().expect("the confirm prompt");
    assert_eq!(
        (
            prompt.before.as_str(),
            prompt.host.as_str(),
            prompt.after.as_str()
        ),
        ("https://", "evil.example.invalid", "/x")
    );
    assert_eq!(prompt.warnings, [Warning::TextMismatch]);
    assert!(shell.0.borrow().is_empty());
    s.cancel_prompt();
    assert!(s.prompt().is_none() && shell.0.borrow().is_empty());
    click(&mut s, 20.0);
    s.open_prompt();
    assert!(s.prompt().is_none());
    assert_eq!(*shell.0.borrow(), ["https://evil.example.invalid/x"]);

    // A drag that starts on a link selects instead of opening it.
    now.set(now.get() + MULTI_CLICK);
    let (x, y) = on(20.0);
    s.press(x, y, now.get());
    s.extend(on(300.0).0, on(300.0).1);
    s.release(x, y);
    assert!(s.prompt().is_none());

    // mailto: goes to the composer, decoded.
    click(&mut s, 60.0);
    assert!(s.prompt().is_none());
    let mail = s
        .take_effects()
        .into_iter()
        .find_map(|e| match e {
            Effect::Mailto(m) => Some(m),
            _ => None,
        })
        .expect("a mailto");
    assert_eq!(
        (mail.to.as_str(), mail.subject.as_str()),
        ("ana@example.com", "Olá")
    );

    // Tab walks the links, Enter follows the focused one, and Tab past the
    // last lets focus leave. (The strip shows the hovered link first.)
    s.hover_at(None);
    assert!(s.focus_next(false));
    assert_eq!(s.focus_rects().len(), 1);
    assert_eq!(s.target(), "https://evil.example.invalid/x");
    assert!(s.focus_next(false));
    assert!(s.focus_next(false));
    assert_eq!(s.target(), "#end");
    s.take_effects();
    s.activate_focused();
    assert_eq!(s.take_effects(), [Effect::ScrollTo(1492.0)]);
    assert!(!s.focus_next(false));
    assert!(s.focus_rects().is_empty());
    assert_eq!(s.target(), "");
    // Shift+Tab from no focus starts at the last link.
    assert!(s.focus_next(true));
    assert_eq!(s.target(), "#end");
    // A link below the view is scrolled into view when focused.
    let r = s.links()[2].rects[0];
    s.set_frame(Frame {
        top: 500.0,
        height: 300.0,
        ..FRAME
    });
    s.take_effects();
    s.set_focus(Some(2));
    assert_eq!(s.take_effects(), [Effect::ScrollTo(r.y - 16.0)]);
    // Escape drops the focus; a new message drops the links.
    s.clear();
    assert!(s.focus_rects().is_empty());
    s.open(8, html("<p>y</p>"), false);
    assert!(s.links().is_empty());
}

/// A 600 px fixed-width table (as the worker's tests).
const FIXED: &str = "<table width='600' style='width:600px;background:#eef' \
     cellpadding='0' cellspacing='0'><tr><td><p>Alpha beta gamma delta epsilon zeta eta \
     theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega, \
     and enough words after them to wrap onto a second line.</p>\
     <p>Second paragraph with <a href='https://example.com/x'>a link</a> in it.</p>\
     </td></tr></table>";

fn real_page(width: u32) -> (Page, Vec<PageLink>) {
    let key = Key {
        msg: 1,
        width,
        scale: 1f32.to_bits(),
        dark: false,
        dim: false,
        remote: false,
    };
    let replies = run_to_end(
        vec![Job::Layout {
            generation: 1,
            key,
            src: html(FIXED),
        }],
        PixelOrder::Rgba,
    );
    replies
        .into_iter()
        .find_map(|r| match r {
            Reply::Page { page, links, .. } => Some((page, links)),
            _ => None,
        })
        .expect("a page")
}

#[test]
fn links_are_hit_under_scale() {
    let (_, l600) = real_page(600);
    let (p450, l450) = real_page(450);
    let (a, b) = (l600[0].rects[0], l450[0].rects[0]);
    let (mut s, fake, _) = session();
    // The column the 450 px page was laid out for.
    s.set_frame(Frame {
        column: 450.0,
        ..FRAME
    });
    s.open(1, html(FIXED), false);
    let [(generation, _)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    s.on_reply(
        Reply::Page {
            generation,
            page: p450,
            links: l450.clone(),
        },
        |_| 0,
    );
    let center = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    assert_eq!(s.link_at(center.0, center.1), Some(0));
    // The same point in page coordinates is off the link at this scale.
    assert_eq!(s.link_at(a.x + a.w - 1.0, a.y + a.h - 1.0), None);
    s.hover_at(Some(center));
    assert_eq!(s.target(), "https://example.com/x");

    // A page wider than the column (the 0.6 floor) scrolls sideways, as
    // far as it goes; a press still reaches the page.
    let wide = Page {
        width: 1500.0,
        height: 3000.0,
        ..p450
    };
    s.on_reply(
        Reply::Page {
            generation,
            page: wide,
            links: Vec::new(),
        },
        |_| 0,
    );
    s.set_page_x(-120.0);
    assert_eq!(s.page_x(), -120.0);
    s.set_page_x(-5000.0);
    assert_eq!(s.page_x(), 450.0 - 1500.0);
    s.set_page_x(30.0);
    assert_eq!(s.page_x(), 0.0);
    s.set_page_x(-120.0);
    // A relayout that fits the column again leaves no sideways offset.
    s.on_reply(
        Reply::Page {
            generation,
            page: p450,
            links: Vec::new(),
        },
        |_| 0,
    );
    assert_eq!(s.page_x(), 0.0);
    s.on_reply(
        Reply::Page {
            generation,
            page: wide,
            links: Vec::new(),
        },
        |_| 0,
    );
    s.set_page_x(-120.0);
    s.press(900.0, 400.0, Instant::now());
    assert!(fake.take().iter().any(|j| matches!(
        j,
        Job::Select {
            op: SelectOp::Press { .. },
            ..
        }
    )));
    // A new message starts at the left edge again.
    s.open(2, html(FIXED), false);
    assert_eq!(s.page_x(), 0.0);
}

#[test]
fn remote_banner_loads_once_and_lays_out_again() {
    let (mut s, fake, _) = session();
    s.set_frame(FRAME);
    let take = || layouts(fake.take());
    let page = |key: Key, remote_images| Page {
        remote_images,
        ..page(key.width as f32, 600.0)
    };

    s.open(7, html("<p>x</p>"), false);
    let [(generation, key)] = take()[..] else {
        panic!("expected one layout job");
    };
    assert!(!key.remote);
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key, 3),
            links: vec![],
        },
        |_| 0,
    );
    assert_eq!(s.remote_blocked(), 3);

    // Load once: laid out again with consent, the page kept meanwhile.
    s.allow_remote();
    assert_eq!(s.remote_blocked(), 0);
    let [(generation, key)] = take()[..] else {
        panic!("expected a relayout");
    };
    assert!(key.remote);
    assert!(s.html());
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key, 3),
            links: vec![],
        },
        |_| 0,
    );
    assert_eq!(s.remote_blocked(), 0, "no banner while fetching");
    // Fetches for another message change nothing; ours lay out again.
    s.on_reply(Reply::Fetched { msg: 8 }, |_| 0);
    assert!(take().is_empty());
    s.on_reply(Reply::Fetched { msg: 7 }, |_| 0);
    let [(_, again)] = take()[..] else {
        panic!("expected a relayout");
    };
    assert_eq!(again, key);

    // Consent lasts while the message stays open, and no further.
    s.open(7, html("<p>x</p>"), false);
    assert!(take().iter().all(|(_, k)| k.remote));
    s.open(8, html("<p>y</p>"), false);
    assert!(take().iter().all(|(_, k)| !k.remote));
    s.on_reply(Reply::Fetched { msg: 8 }, |_| 0);
    assert!(take().is_empty());
    // An allowlisted sender's message opens with its images.
    s.open(9, html("<p>z</p>"), true);
    assert!(take().iter().all(|(_, k)| k.remote));
    s.close();
    s.allow_remote();
    assert!(take().is_empty());
}

#[test]
fn failed_images_and_original_colours_reach_the_reader() {
    let (mut s, fake, _) = session();
    s.set_frame(Frame {
        dark: true,
        ..FRAME
    });
    let take = || layouts(fake.take());
    let page = |key: Key, pending_images, failed_images| Page {
        bg: [32, 32, 32, 255],
        remote_images: failed_images,
        pending_images,
        failed_images,
        dimmed: key.dim,
        ..page(key.width as f32, 600.0)
    };

    // An allowlisted sender: no count while fetches are pending.
    s.open(7, html("<p>x</p>"), true);
    let [(generation, key)] = take()[..] else {
        panic!("expected one layout job");
    };
    assert!(key.remote && key.dark && key.dim);
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key, true, 2),
            links: vec![],
        },
        |_| 0,
    );
    assert_eq!(s.remote_failed(), 0);
    assert_eq!(s.remote_blocked(), 0);
    assert!(s.dimmed() && !s.original());
    // All in: two couldn't be loaded.
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key, false, 2),
            links: vec![],
        },
        |_| 0,
    );
    assert_eq!(s.remote_failed(), 2);

    // Show original colours: laid out again without dimming, for this
    // message only.
    s.show_original(true);
    let [(generation, key)] = take()[..] else {
        panic!("expected a relayout");
    };
    assert!(key.dark && !key.dim);
    assert!(s.original() && s.html(), "the page stays up");
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key, false, 0),
            links: vec![],
        },
        |_| 0,
    );
    assert!(!s.dimmed() && s.original());
    s.open(7, html("<p>x</p>"), true);
    assert!(take().iter().all(|(_, k)| !k.dim));
    s.open(8, html("<p>y</p>"), false);
    assert!(take().iter().all(|(_, k)| k.dim));
    assert!(!s.original());
    assert_eq!(s.remote_failed(), 0);
    s.close();
    assert!(!s.dimmed());
}

#[test]
fn a_theme_change_lays_the_page_out_again() {
    let (mut s, fake, _) = session();
    s.set_frame(FRAME);
    s.open(7, html("<p>x</p>"), false);
    let [(generation, key)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key.width as f32, 600.0),
            links: vec![],
        },
        |_| 0,
    );
    s.take_effects();
    s.set_frame(Frame {
        dark: true,
        ..FRAME
    });
    let t = token(&s.take_effects(), true).expect("the debounce");
    s.resize_due(t);
    let [(_, dark)] = layouts(fake.take())[..] else {
        panic!("expected a relayout");
    };
    assert!(dark.dark && dark.dim && dark.width == key.width);
}

#[test]
fn refresh_tiles_asks_for_the_visible_ones_again() {
    let (mut s, fake, _) = session();
    s.set_frame(FRAME);
    s.open(7, html("<p>x</p>"), false);
    let [(generation, key)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    let p = page(key.width as f32, 5000.0);
    s.on_reply(
        Reply::Page {
            generation,
            page: p,
            links: vec![],
        },
        |_| 0,
    );
    let wanted = wanted_tiles(FRAME.top, FRAME.height, p.height);
    assert_eq!(tile_jobs(fake.take()), vec![(generation, wanted.clone())]);
    for &index in &wanted {
        s.show_tile(generation, index, index);
    }
    // Nothing to do while the images are good.
    s.set_frame(FRAME);
    assert!(fake.take().is_empty());

    // The device was lost: the same tiles are asked for, and the images
    // stay up until the new ones come.
    s.refresh_tiles();
    assert_eq!(tile_jobs(fake.take()), vec![(generation, wanted.clone())]);
    assert!(wanted.iter().all(|&i| shown(&s, i)));
    assert!(s.take_dropped().is_empty());
    for &index in &wanted {
        s.show_tile(generation, index, index + 100);
    }
    let mut dropped = s.take_dropped();
    dropped.sort_unstable();
    assert_eq!(dropped, wanted, "the old images are released");
    assert!(wanted.iter().all(|&i| shown(&s, i)));
    // Asked once: a second refresh with the tiles requested but not yet
    // back does not send the same request twice.
    s.refresh_tiles();
    assert_eq!(tile_jobs(fake.take()).len(), 1);
    s.refresh_tiles();
    assert!(fake.take().is_empty());
}

#[test]
fn the_whole_of_a_long_message_replaces_its_first_look_without_a_blank() {
    let (mut s, fake, _) = session();
    s.set_frame(FRAME);
    s.open(7, html("<p>x</p>"), false);
    let [(generation, key)] = layouts(fake.take())[..] else {
        panic!("expected one layout job");
    };
    let first = Page {
        partial: true,
        ..page(key.width as f32, 4000.0)
    };
    s.on_reply(
        Reply::Page {
            generation,
            page: first,
            links: vec![],
        },
        |_| 0,
    );
    assert!(s.html() && s.partial() && !s.truncated());
    let requests = tile_jobs(fake.take());
    let [(_, ref tiles)] = requests[..] else {
        panic!("expected one tile job");
    };
    for &index in tiles {
        s.show_tile(generation, index, index);
    }
    assert!(shown(&s, 0));
    s.on_reply(
        Reply::Page {
            generation,
            page: page(key.width as f32, 60_000.0),
            links: vec![],
        },
        |_| 0,
    );
    assert!(!s.partial());
    assert_eq!(s.tiles().len(), tile_count(60_000.0) as usize);
    // The first look's tiles stay up until the whole page's replace them.
    assert!(shown(&s, 0));
    let requests = tile_jobs(fake.take());
    let [(_, ref again)] = requests[..] else {
        panic!("expected the tiles in view asked for again");
    };
    assert_eq!(again[0], 0);
}
