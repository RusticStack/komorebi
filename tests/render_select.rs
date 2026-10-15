//! Text selection and copy on the synthetic corpus. Points come from the
//! laid-out lines, so the assertions hold whatever fonts Windows has.

mod common;

use common::{big_stack, input, raw};
use komorebi::mail::render::{self, Deadline, Doc, Engine, Opts, TextLine};

fn doc(name: &'static str, width: f32) -> Doc {
    big_stack(move || {
        let opts = Opts {
            width_px: width,
            ..Default::default()
        };
        render::layout(
            &input(&raw(name)),
            &opts,
            &mut Engine::new(),
            &Deadline::none(),
        )
        .unwrap()
    })
}

fn line(doc: &Doc, pred: impl Fn(&str) -> bool) -> TextLine {
    doc.text_lines()
        .into_iter()
        .find(|l| pred(&l.text))
        .expect("line present")
}

/// A point inside `l`, `frac` of the way along it.
fn at(l: &TextLine, frac: f32) -> (f32, f32) {
    (l.x + l.w * frac, l.y + l.h / 2.0)
}

#[test]
fn drag_across_layouts_selects_the_exact_text() {
    for width in [600.0, 900.0] {
        let d = doc("12-personal-applemail", width);
        // From just left of "Olá" (a margin) to past the end of "Ligas-me
        // ... 18h.": four paragraph layouts, with blank lines between.
        let first = line(&d, |t| t.starts_with("Olá Ana"));
        let last = line(&d, |t| t.starts_with("Ligas-me"));
        let a = d.text_at(first.x - 4.0, first.y + 2.0).unwrap();
        let b = d.text_at(last.x + last.w + 30.0, last.y + 2.0).unwrap();
        assert!(a < b);
        assert_eq!(
            d.selection_text(a, b),
            "Olá Ana,\nObrigada pelo jantar de sábado - foi mesmo bom rever-te! 😊\n\
             Envio as fotos do Gerês neste álbum. A da cascata ficou incrível.\n\
             Ligas-me amanhã? Estou livre depois das 18h."
        );
        // Dragging backwards selects the same text.
        assert_eq!(d.selection_text(b, a), d.selection_text(a, b));
        // One highlight per line, within the page, top to bottom.
        let rects = d.selection_rects(a, b);
        assert!(rects.len() >= 4, "{rects:?}");
        assert!(rects.windows(2).all(|w| w[0].y <= w[1].y));
        assert!(rects.iter().all(|r| r.x >= 0.0 && r.x + r.w <= d.width));
        assert!((rects[0].y - first.y).abs() < 1.0);
    }
}

#[test]
fn partial_drag_inside_lines() {
    let d = doc("12-personal-applemail", 600.0);
    // Mid-word to mid-word across two layouts: the cut points come from
    // the hit test, and the text between them is exact.
    let envio = line(&d, |t| t.starts_with("Envio"));
    let ligas = line(&d, |t| t.starts_with("Ligas-me"));
    let a = d.text_at(envio.x + envio.w * 0.5, envio.y + 2.0).unwrap();
    let b = d.text_at(ligas.x + ligas.w * 0.5, ligas.y + 2.0).unwrap();
    let full = "Envio as fotos do Gerês neste álbum. A da cascata ficou incrível.\n\
                Ligas-me amanhã? Estou livre depois das 18h.";
    let got = d.selection_text(a, b);
    assert!(got.contains('\n'), "{got:?}");
    let (head, tail) = got.split_once('\n').unwrap();
    assert!(full.split_once('\n').unwrap().0.ends_with(head), "{got:?}");
    assert!(
        full.split_once('\n').unwrap().1.starts_with(tail),
        "{got:?}"
    );
    assert!(head.len() > 10 && tail.len() > 10, "{got:?}");
    // A press without movement selects nothing.
    assert_eq!(d.selection_text(a, a), "");
    assert!(d.selection_rects(a, a).is_empty());
}

#[test]
fn double_click_selects_a_word() {
    let d = doc("12-personal-applemail", 600.0);
    let l = line(&d, |t| t.starts_with("Envio"));
    let words = [
        "Envio",
        "as",
        "fotos",
        "do",
        "Gerês",
        "neste",
        "álbum",
        "A",
        "da",
        "cascata",
        "ficou",
        "incrível",
    ];
    let mut seen = Vec::new();
    for i in 1..40 {
        let (x, y) = at(&l, i as f32 / 40.0);
        let (a, b) = d.word_at(x, y).unwrap();
        let word = d.selection_text(a, b);
        // A word, with at most its trailing space; never punctuation-glued
        // neighbours or part of a word.
        let w = word.trim_end();
        assert!(
            words.contains(&w) || w.chars().all(|c| c.is_ascii_punctuation()) || w.is_empty(),
            "{word:?}"
        );
        if !seen.contains(&w.to_string()) {
            seen.push(w.to_string());
        }
    }
    for w in ["Envio", "fotos", "Gerês", "álbum", "cascata", "incrível"] {
        assert!(seen.iter().any(|s| s == w), "{w} in {seen:?}");
    }
}

#[test]
fn triple_click_selects_the_paragraph() {
    let d = doc("12-personal-applemail", 600.0);
    // The P.S. wraps over three lines; a click on the middle one takes all.
    let mid = line(&d, |t| t.starts_with("que a reserva"));
    let (x, y) = at(&mid, 0.5);
    let (a, b) = d.block_at(x, y).unwrap();
    let text = d.selection_text(a, b);
    assert!(text.starts_with("P.S. - não te esqueças"), "{text:?}");
    assert!(text.ends_with("de 24 e 25."), "{text:?}");
    assert!(!text.contains('\n'));
    assert_eq!(d.selection_rects(a, b).len(), 3);
}

#[test]
fn select_all_copies_visible_text_only() {
    let d = doc("12-personal-applemail", 600.0);
    let (a, b) = d.text_bounds().unwrap();
    let all = d.selection_text(a, b);
    assert!(all.starts_with("Olá Ana,\nObrigada"), "{all:?}");
    assert!(
        all.ends_with("+351 000 000 000 · marta@example.org"),
        "{all:?}"
    );
    // List items without their bullets, as a browser copies them.
    assert!(
        all.contains("pediste:\n4 ovos\n200 g de açúcar\nraspa e sumo de 2 laranjas\n"),
        "{all:?}"
    );
    assert!(!all.contains('•'));

    // Hidden preheaders are neither selectable nor copied.
    for (name, hidden) in [
        ("01-mailchimp-campaign", "Four new glazes"),
        ("10-transactional-buttons", "lido por 10 minutos"),
    ] {
        let d = doc(name, 600.0);
        let (a, b) = d.text_bounds().unwrap();
        let all = d.selection_text(a, b);
        assert!(!all.is_empty(), "{name}");
        assert!(!all.contains(hidden), "{name}");
    }
}

#[test]
fn hit_tests_survive_odd_points() {
    let d = doc("14-adversarial-malformed", 600.0);
    for (x, y) in [
        (f32::NAN, 10.0),
        (10.0, f32::INFINITY),
        (-1e30, -1e30),
        (1e30, 1e30),
        (0.0, d.height + 1000.0),
    ] {
        if let Some(p) = d.text_at(x, y) {
            assert!(d.selection_rects(p, p).is_empty());
        }
        d.word_at(x, y);
        d.block_at(x, y);
    }
    let (a, b) = d.text_bounds().unwrap();
    assert!(!d.selection_text(a, b).contains("<script"));
    let rects = d.selection_rects(a, b);
    assert!(rects.iter().all(|r| r.y < d.height));
}

/// The statement as the reader lays it out for a narrow column (at its
/// natural width, drawn scaled): every highlight is on the page, and the
/// amounts column is part of the selectable text.
#[test]
fn fixed_width_page_selects_inside_its_width() {
    let narrow = doc("05-bank-statement", 464.0);
    assert!(narrow.natural_width > 464.0);
    let d = doc("05-bank-statement", narrow.natural_width.ceil().min(1000.0));
    let (a, b) = d.text_bounds().unwrap();
    assert!(d.selection_text(a, b).contains("-18.47"));
    let rects = d.selection_rects(a, b);
    assert!(!rects.is_empty());
    assert!(
        rects
            .iter()
            .all(|r| r.x >= 0.0 && r.x + r.w <= d.width + 0.5)
    );
}
