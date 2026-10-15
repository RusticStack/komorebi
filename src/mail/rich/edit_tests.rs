#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic
)]

use super::edit::*;
use super::*;

fn ed(text: &str) -> Editor {
    Editor::new(Doc::from_plain(text))
}

fn quote() -> QuoteBlock {
    QuoteBlock::new(
        "<p>On Monday, Bob wrote:</p><blockquote>Noon?</blockquote>",
        false,
    )
    .unwrap()
}

/// Type `s` at the caret, one character at a time, as the proxy reports.
fn type_at(e: &mut Editor, mut caret: usize, s: &str) -> usize {
    for c in s.chars() {
        let mut t = e.text().to_string();
        t.insert(caret, c);
        caret = e
            .apply(&t, Some(caret + c.len_utf8()))
            .expect("typing is accepted");
        assert_eq!(e.text(), t);
    }
    caret
}

#[test]
fn typing_and_enter_make_paragraphs() {
    let mut e = ed("");
    assert_eq!(e.text(), "");
    let c = type_at(&mut e, 0, "ação\nbé 😀");
    assert_eq!(e.text(), "ação\nbé 😀");
    assert_eq!(c, e.text().len());
    assert_eq!(e.doc().blocks.len(), 2);
    // Backspace joins them again.
    let at = e.text().find('\n').unwrap();
    let mut t = e.text().to_string();
    t.remove(at);
    assert_eq!(e.apply(&t, Some(at)), Some(at));
    assert_eq!(e.doc().blocks.len(), 1);
    assert_eq!(emit::plain(e.doc()), "açãobé 😀");
}

#[test]
fn bold_applies_to_the_selection_and_to_what_is_typed_next() {
    let mut e = ed("hello world");
    assert!(e.format_chars(0, 5, &CharOp::Bold));
    let Block::Para(p) = &e.doc().blocks[0] else {
        panic!()
    };
    assert_eq!(p.runs.len(), 2);
    assert!(p.runs[0].style.b && !p.runs[1].style.b);
    assert!(e.format_at(0, 5).b);
    assert!(!e.format_at(0, 11).b);
    // Again: off.
    assert!(e.format_chars(0, 5, &CharOp::Bold));
    assert!(!e.format_at(0, 5).b);
    // A collapsed caret: the next character typed is italic.
    let end = e.text().len();
    e.format_chars(end, end, &CharOp::Italic);
    assert!(e.format_at(end, end).i);
    type_at(&mut e, end, "!");
    let Block::Para(p) = &e.doc().blocks[0] else {
        panic!()
    };
    assert!(p.runs.last().unwrap().style.i);
    assert_eq!(p.runs.last().unwrap().text, "!");
    assert!(emit::needs_html(e.doc()));
}

#[test]
fn undo_and_redo_cover_typing_and_formatting_together() {
    let mut e = ed("");
    let original = e.doc().clone();
    type_at(&mut e, 0, "hello");
    e.format_chars(0, 5, &CharOp::Underline);
    let formatted = e.doc().clone();
    // Formatting first, then the typing as one step.
    assert_eq!(e.undo(), Some((0, 5)));
    assert!(!e.format_at(0, 5).u);
    assert_eq!(e.text(), "hello");
    e.undo();
    assert_eq!(e.doc(), &original);
    assert!(e.undo().is_none());
    e.redo();
    e.redo();
    assert_eq!(e.doc(), &formatted);
    // A new edit drops the redo steps.
    e.undo();
    type_at(&mut e, 5, "!");
    assert!(!e.can_redo());
}

#[test]
fn lists_indent_and_enter_on_an_empty_item() {
    let mut e = ed("one\ntwo");
    assert!(e.format_paras(0, 7, ParaOp::List(ListKind::Bullet)));
    assert_eq!(emit::plain(e.doc()), "- one\n- two");
    assert!(e.format_paras(4, 4, ParaOp::Indent(true)));
    assert_eq!(emit::plain(e.doc()), "- one\n  - two");
    // Enter at the end makes an item; Enter on that empty item ends the list.
    let end = e.text().len();
    let mut t = e.text().to_string();
    t.push('\n');
    let c = e.apply(&t, Some(end + 1)).unwrap();
    assert_eq!(e.text(), t);
    let mut t2 = e.text().to_string();
    t2.insert(c, '\n');
    let c2 = e.apply(&t2, Some(c + 1)).unwrap();
    // Not split: the item stopped being one.
    assert_eq!(e.text(), t);
    assert_eq!(c2, c);
    assert_eq!(e.format_at(c, c).list, None);
    // Toggling the same list kind on all of them takes it off.
    assert!(e.format_paras(0, 7, ParaOp::List(ListKind::Bullet)));
    assert_eq!(e.format_at(0, 0).list, None);
}

#[test]
fn tables_hold_cells_and_refuse_edits_across_their_edges() {
    let mut e = ed("before");
    let c = e.insert_table(6, 2, 2).unwrap();
    // Table after the text: four cell lines and the paragraph after it.
    assert_eq!(e.text(), "before\n\n\n\n\n");
    assert_eq!(c, 7);
    let c = type_at(&mut e, c, "a1");
    // Tab moves to the next cell and selects it.
    let (s, t) = e.tab(c, c, false).unwrap();
    assert_eq!((s, t), (10, 10));
    type_at(&mut e, s, "b1");
    assert_eq!(e.text(), "before\na1\nb1\n\n\n");
    // Enter in a cell: a second paragraph in the same cell.
    let mut t = e.text().to_string();
    t.insert(9, '\n');
    assert_eq!(e.apply(&t, Some(10)), Some(10));
    assert_eq!(e.text(), "before\na1\n\nb1\n\n\n");
    let Block::Table(tb) = &e.doc().blocks[1] else {
        panic!()
    };
    assert_eq!(tb.rows[0][0].paras.len(), 2);
    e.undo();
    // Backspace at the start of the paragraph after the table: refused.
    let before = e.text().to_string();
    let last = before.len();
    let mut t = before.clone();
    t.remove(last - 1);
    assert_eq!(e.apply(&t, Some(last - 1)), None);
    assert_eq!(e.text(), before);
    // Deleting across two cells: refused.
    assert_eq!(e.replace(8, 11, "", (8, 11)), None);
    assert_eq!(e.text(), before);
    // Row and column operations, and undo.
    let doc = e.doc().clone();
    e.table_op(8, TableOp::RowBelow).unwrap();
    e.table_op(8, TableOp::ColRight).unwrap();
    let Block::Table(tb) = &e.doc().blocks[1] else {
        panic!()
    };
    assert_eq!((tb.row_count(), tb.col_count()), (3, 3));
    e.undo();
    e.undo();
    assert_eq!(e.doc(), &doc);
    e.table_op(8, TableOp::DeleteTable).unwrap();
    assert_eq!(e.text(), "before\n");
    // Tables are capped at 10 x 10.
    assert!(e.insert_table(0, 11, 2).is_none());
    // The plain text lays the cells out in columns.
    e.undo();
    assert_eq!(emit::plain(e.doc()), "before\na1  b1\n\n");
}

#[test]
fn the_quote_is_one_object() {
    let mut doc = Doc::from_plain("hi");
    doc.blocks.push(Block::Quote(quote()));
    let mut e = Editor::new(doc);
    assert_eq!(e.text(), "hi\n\u{fffc}");
    // Typing at its start: a paragraph above it.
    let before = e.text().to_string();
    let at = 3;
    let mut t = before.clone();
    t.insert(at, 'x');
    let c = e.apply(&t, Some(at + 1)).unwrap();
    assert_eq!(e.text(), "hi\nx\n\u{fffc}");
    assert_eq!(c, 4);
    // Typing after it: a paragraph below it.
    let end = e.text().len();
    let mut t = e.text().to_string();
    t.push('y');
    e.apply(&t, Some(end + 1)).unwrap();
    assert_eq!(e.text(), "hi\nx\n\u{fffc}\ny");
    // Backspace after it: refused.
    let t = "hi\nx\n\u{fffc}y".to_string();
    assert_eq!(e.apply(&t, Some(t.len() - 1)), None);
    // Folded, it leaves the text (still sent).
    assert!(e.fold_quote(true));
    assert_eq!(e.text(), "hi\nx\ny");
    assert!(emit::plain(e.doc()).contains("> Noon?"));
    e.fold_quote(false);
    // Select all and delete: everything goes, quote included.
    let len = e.text().len();
    assert_eq!(e.replace(0, len, "", (0, len)), Some(0));
    assert_eq!(e.text(), "");
    assert!(!e.has_quote());
    e.undo();
    assert!(e.has_quote());
    // Edit quoted text: ordinary quoted paragraphs.
    assert!(e.expand_quote());
    assert!(!e.has_quote());
    assert!(
        emit::plain(e.doc()).contains("> Noon?"),
        "{}",
        emit::plain(e.doc())
    );
}

#[test]
fn the_signature_goes_above_the_quote_and_is_swapped_whole() {
    let mut doc = Doc::from_plain("");
    doc.blocks.push(Block::Quote(quote()));
    let mut e = Editor::new(doc);
    let sig = SignatureBlock::new(vec![Block::Para(Para::text("Ana"))], None, Some(7));
    assert!(e.set_signature(Some(sig.clone())));
    assert_eq!(e.text(), "\nAna\n\u{fffc}");
    assert!(e.signature().unwrap().is_untouched());
    // Typing in it: touched.
    type_at(&mut e, 4, " Silva");
    assert!(!e.signature().unwrap().is_untouched());
    // Another one replaces it whole; undo brings the edited one back.
    let other = SignatureBlock::new(vec![Block::Para(Para::text("Bob"))], None, Some(8));
    assert!(e.set_signature(Some(other)));
    assert_eq!(e.text(), "\nBob\n\u{fffc}");
    e.undo();
    assert_eq!(e.text(), "\nAna Silva\n\u{fffc}");
    // None takes it out.
    assert!(e.set_signature(None));
    assert_eq!(e.text(), "\n\u{fffc}");
    // Joining the message with the signature is refused.
    e.undo();
    assert_eq!(e.replace(0, 1, "", (0, 1)), None);
    // The plain text keeps the delimiter.
    assert!(emit::plain(e.doc()).contains("-- \nAna Silva"));
}

#[test]
fn links_and_clear_formatting() {
    let mut e = ed("see here");
    e.link(4, 8, "https://example.org/", "").unwrap();
    assert_eq!(emit::plain(e.doc()), "see here (https://example.org/)");
    // Typing at the end of a link does not extend it.
    type_at(&mut e, 8, ".");
    assert_eq!(emit::plain(e.doc()), "see here (https://example.org/).");
    // A link inserted at a collapsed caret, with its address as the label.
    let (c, _) = e.link(0, 0, "https://a.example/", "").unwrap();
    assert_eq!(c, "https://a.example/".len());
    e.format_chars(0, 3, &CharOp::Bold);
    e.format_paras(0, 0, ParaOp::Align(Align::Center));
    assert!(e.clear_formatting(0, e.text().len()));
    let f = e.format_at(0, 3);
    assert!(!f.b);
    assert_eq!(f.align, Align::Left);
    // Links stay.
    assert!(emit::plain(e.doc()).contains("(https://example.org/)"));
}

#[test]
fn paste_keeps_what_the_model_holds() {
    let mut e = ed("ab");
    let frag = import::lossy("<p><b>bold</b> one</p><p>two</p>").unwrap();
    let c = e.paste(1, 1, frag).unwrap();
    assert_eq!(e.text(), "abold one\ntwob");
    assert_eq!(c, "abold one\ntwo".len());
    assert!(e.format_at(1, 5).b);
    // In a table cell, a table is pasted as text.
    let mut e = ed("");
    e.insert_table(0, 1, 1).unwrap();
    let frag = import::lossy("<table><tr><td>x</td><td>y</td></tr></table>").unwrap();
    e.paste(0, 0, frag).unwrap();
    let Block::Table(t) = &e.doc().blocks[0] else {
        panic!()
    };
    assert_eq!(t.row_count(), 1);
    assert_eq!(t.rows[0][0].plain_text(), "x  y");
}

#[test]
fn message_defaults_change_every_block() {
    let mut e = ed("x");
    e.set_defaults(
        FontId::Georgia,
        SizePt::new(14).unwrap(),
        Some(ColorId::Blue),
    );
    let ch = e.take_changes();
    assert!(ch.last().unwrap().all);
    let f = e.format_at(0, 0);
    assert_eq!(
        (f.font, f.size.get(), f.color),
        (FontId::Georgia, 14, Some(ColorId::Blue))
    );
    e.undo();
    assert_eq!(e.format_at(0, 0).font, FontId::SegoeUi);
}

#[test]
fn caps_refuse_what_would_go_over() {
    let mut e = ed("");
    let big = "x".repeat(MAX_TEXT_BYTES);
    let len = big.len();
    assert!(e.apply(&big, Some(len)).is_some());
    assert!(e.apply(&format!("{big}y"), Some(len + 1)).is_none());
    assert_eq!(e.text().len(), len);
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const BITS: &[&str] = &[
    "a", "ç", "😀", " ", "\n", "hello", "x\ny", "\n\n", "日本", "\t", "\u{fffc}",
];

fn boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Random edits, formatting, tables and undo never panic, keep the
/// document in its canonical form, keep the projection in step with the
/// model, and undoing everything gives the original back.
#[test]
fn random_edits_keep_the_model_and_the_projection_in_step() {
    for seed in 1..=60u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut doc = Doc::from_plain("first line\nsecond");
        if seed % 3 == 0 {
            doc.blocks.push(Block::Quote(quote()));
        }
        let mut e = Editor::new(doc);
        let original = e.doc().clone();
        if seed % 2 == 0 {
            e.set_signature(Some(SignatureBlock::new(
                vec![Block::Para(Para::text("Sig"))],
                None,
                None,
            )));
        }
        for _ in 0..120 {
            let len = e.text().len();
            let a = boundary(e.text(), r.below(len + 1));
            let b = boundary(e.text(), (a + r.below(8)).min(len));
            match r.below(10) {
                0..=4 => {
                    let ins = BITS[r.below(BITS.len())];
                    e.replace(a, b, ins, (a, b));
                }
                5 => {
                    let op = [
                        CharOp::Bold,
                        CharOp::Italic,
                        CharOp::Font(FontId::Arial),
                        CharOp::SizeStep(true),
                        CharOp::Color(Some(ColorId::Red)),
                    ];
                    e.format_chars(a, b, &op[r.below(5)]);
                }
                6 => {
                    let op = [
                        ParaOp::List(ListKind::Number),
                        ParaOp::Indent(true),
                        ParaOp::Align(Align::Right),
                        ParaOp::Quote,
                    ];
                    e.format_paras(a, b, op[r.below(4)]);
                }
                7 => {
                    e.insert_table(a, 1 + r.below(3), 1 + r.below(3));
                }
                8 => {
                    let op = [
                        TableOp::RowBelow,
                        TableOp::ColLeft,
                        TableOp::DeleteRow,
                        TableOp::DeleteCol,
                    ];
                    e.table_op(a, op[r.below(4)]);
                }
                _ => {
                    e.undo();
                }
            }
            // The projection is the one the document makes.
            let fresh = Editor::new(e.doc().clone());
            assert_eq!(fresh.text(), e.text(), "seed {seed}");
            assert_eq!(e.doc().clone().normalized(), *e.doc(), "seed {seed}");
            // The HTML reads back as the same document.
            let back = import::strict(&emit::html(e.doc(), Mode::Draft)).expect("strict import");
            assert_eq!(&back, e.doc(), "seed {seed}");
        }
        while e.undo().is_some() {}
        assert_eq!(e.doc(), &original, "seed {seed}");
    }
}

#[test]
fn autoformat_starts_a_list_only_at_a_line_start() {
    let mut e = ed("");
    let c = type_at(&mut e, 0, "- ");
    let c = e.autoformat(c).expect("a bulleted list");
    assert_eq!(c, 0);
    assert_eq!(e.text(), "");
    match &e.doc().blocks[0] {
        Block::Para(p) => assert_eq!(p.list, Some((ListKind::Bullet, 0))),
        other => panic!("{other:?}"),
    }
    // Undo puts the marks back, as typed.
    e.undo();
    assert_eq!(e.text(), "- ");
    // Not after other text, not in a list already, not "2. ".
    let mut e = ed("");
    let c = type_at(&mut e, 0, "a - ");
    assert!(e.autoformat(c).is_none());
    let mut e = ed("");
    let c = type_at(&mut e, 0, "2. ");
    assert!(e.autoformat(c).is_none());
    let mut e = ed("");
    let c = type_at(&mut e, 0, "1. ");
    assert!(e.autoformat(c).is_some());
    let c = type_at(&mut e, 0, "* ");
    assert!(e.autoformat(c).is_none(), "a list item already");
}

#[test]
fn a_fragment_is_the_selection_with_its_formatting() {
    let mut e = ed("one two\nthree four");
    assert!(e.format_chars(4, 7, &CharOp::Bold));
    // "two, three": the bold word and the next paragraph's start.
    let f = e.fragment(4, 13).unwrap();
    assert_eq!(super::emit::plain(&f).trim_end(), "two\nthree");
    let html = super::emit::html(&f, super::emit::Mode::Send);
    assert!(html.contains("<strong>two</strong>"), "{html}");
    assert!(!html.contains("one") && !html.contains("four"), "{html}");
    // Backwards, the same; empty, none.
    assert_eq!(e.fragment(13, 4), Some(f));
    assert_eq!(e.fragment(5, 5), None);
    // A signature at the end is left out of a whole-message copy.
    let mut s = ed("hi");
    assert!(s.set_signature(Some(SignatureBlock::new(
        vec![Block::Para(Para::text("Ana"))],
        None,
        None
    ))));
    let len = s.text().len();
    let f = s.fragment(0, len).unwrap();
    assert_eq!(super::emit::plain(&f).trim_end(), "hi");
}
