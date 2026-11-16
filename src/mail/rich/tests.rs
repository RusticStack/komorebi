#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic
)]

use super::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

const WORDS: &[&str] = &[
    "a",
    "hello",
    "bé",
    "x y",
    "  ",
    " ",
    "&<>\"'",
    "ç ã",
    "日本語",
    "two  spaces",
    " lead",
    "trail ",
    "https://example.org/a?b=1&c=2",
    "a\u{a0}b",
    "tab\there",
    "line\nbreak",
];

fn gen_style(r: &mut Rng) -> Style {
    Style {
        b: r.chance(4),
        i: r.chance(4),
        u: r.chance(6),
        s: r.chance(8),
        font: r.chance(5).then(|| FontId::ALL[r.below(10)]),
        size: r
            .chance(5)
            .then(|| SizePt::new(SizePt::ALL[r.below(12)]).unwrap()),
        color: r.chance(5).then(|| match r.below(12) {
            // An exact colour, sometimes one of the named ten's values.
            10 => ColorId::Rgb(r.below(256) as u8, r.below(256) as u8, r.below(256) as u8),
            11 => ColorId::Rgb(0x0F, 0x6C, 0xBD),
            n => ColorId::ALL[n],
        }),
        link: r.chance(8).then(|| {
            [
                "https://example.org/",
                "mailto:a@example.org",
                "http://x.test/p(1)?q=[2]",
            ][r.below(3)]
            .to_string()
        }),
    }
}

fn gen_para(r: &mut Rng, cell: bool) -> Para {
    let n = r.below(5);
    let mut p = Para {
        runs: (0..n)
            .map(|_| {
                if r.chance(6) {
                    // A picture, a link at most (`Doc::normalize`).
                    let iw = 1 + r.below(3000) as u32;
                    let ih = 1 + r.below(900) as u32;
                    let img = InlineImage::new(
                        format!("img-{:016x}@komorebi", r.next()),
                        iw,
                        ih,
                        ["", "Step 1", "a \"q\" & <b>"][r.below(3)],
                    )
                    .sized(1 + r.below(2500) as u16);
                    Run::picture(img, gen_style(r))
                } else {
                    Run::styled(WORDS[r.below(WORDS.len())], gen_style(r))
                }
            })
            .collect(),
        ..Para::default()
    };
    if r.chance(5) {
        p.align = [Align::Center, Align::Right, Align::Justify][r.below(3)];
    }
    if !cell {
        p.indent = if r.chance(5) { r.below(6) as u8 } else { 0 };
        p.quote = if r.chance(4) { r.below(10) as u8 } else { 0 };
        if r.chance(3) {
            let kind = if r.chance(2) {
                ListKind::Bullet
            } else {
                ListKind::Number
            };
            p.list = Some((kind, r.below(5) as u8));
        }
    }
    p
}

fn gen_table(r: &mut Rng) -> Table {
    let rows = 1 + r.below(4);
    let cols = 1 + r.below(4);
    Table {
        rows: (0..rows)
            .map(|_| {
                (0..cols)
                    .map(|_| Cell {
                        paras: (0..1 + r.below(2)).map(|_| gen_para(r, true)).collect(),
                        valign: [VAlign::Top, VAlign::Middle, VAlign::Bottom][r.below(3)],
                        // Spans that may run off the grid or overlap: the
                        // model trims them.
                        colspan: if r.chance(4) { r.below(4) as u8 } else { 0 },
                        rowspan: if r.chance(5) { r.below(4) as u8 } else { 0 },
                    })
                    .collect()
            })
            .collect(),
        borderless: r.chance(2),
        widths: if r.chance(2) {
            (0..cols).map(|_| r.below(400) as u16).collect()
        } else {
            Vec::new()
        },
    }
}

fn gen_blocks(r: &mut Rng, top: bool) -> Vec<Block> {
    (0..r.below(8))
        .map(|_| match r.below(12) {
            0 => Block::Table(gen_table(r)),
            1 if top => {
                let inner = gen_blocks(r, false);
                Block::Signature(SignatureBlock::new(inner, r.chance(2).then_some(7)))
            }
            2 if top => Block::Quote(
                QuoteBlock::new(
                    "<div class=\"komorebi_quote\">On x wrote:<blockquote><b>hi</b> there</blockquote></div>",
                    r.chance(2),
                )
                .unwrap(),
            ),
            _ => Block::Para(gen_para(r, false)),
        })
        .collect()
}

fn gen_doc(r: &mut Rng) -> Doc {
    Doc {
        font: FontId::ALL[r.below(10)],
        size: SizePt::new(SizePt::ALL[r.below(12)]).unwrap(),
        color: r.chance(3).then(|| ColorId::ALL[r.below(10)]),
        blocks: gen_blocks(r, true),
    }
    .normalized()
}

#[test]
fn emit_then_strict_import_returns_the_same_document() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for i in 0..2000 {
        let doc = gen_doc(&mut r);
        let h = html(&doc, Mode::Draft);
        let back = import::strict(&h).unwrap_or_else(|| panic!("case {i} refused:\n{h}\n{doc:#?}"));
        assert_eq!(back, doc, "case {i}:\n{h}");
        // And the stored form is stable.
        assert_eq!(html(&back, Mode::Draft), h, "case {i}");
    }
}

#[test]
fn normalize_is_idempotent() {
    let mut r = Rng(42);
    for _ in 0..300 {
        let doc = gen_doc(&mut r);
        assert_eq!(doc.clone().normalized(), doc);
    }
}

#[test]
fn sent_html_has_no_draft_marks() {
    let mut r = Rng(7);
    for _ in 0..200 {
        let doc = gen_doc(&mut r);
        let sent = html(&doc, Mode::Send);
        assert!(!sent.contains("data-"), "{sent}");
        assert!(!sent.contains("<!--"), "{sent}");
        assert!(!sent.contains("<script"));
        assert!(!sent.contains("src=\"http"));
    }
}

#[test]
fn strict_refuses_what_it_did_not_write() {
    let doc = Doc::from_plain("hi");
    let ok = html(&doc, Mode::Draft);
    assert!(import::strict(&ok).is_some());
    // No tag.
    assert!(import::strict(&html(&doc, Mode::Send)).is_none());
    // A foreign element, attribute or style property.
    for bad in [
        ok.replace("<div>hi", "<div onclick=\"x\">hi"),
        ok.replace("<div>hi", "<div style=\"position:absolute\">hi"),
        ok.replace("<div>hi", "<div><img src=\"http://x/y.png\">hi"),
        ok.replace("<div>hi", "<div><font color=red>hi</font>"),
        ok.replace(
            "<div>hi",
            "<div><span style=\"font-family:Comic Sans\">hi</span>",
        ),
        // Any exact colour is written now; a malformed one never is.
        ok.replace("<div>hi", "<div><span style=\"color:#12345\">hi</span>"),
        ok.replace("<div>hi", "<div><a href=\"x\" target=\"_blank\">hi</a>"),
        ok.replace("<div>hi", "<div><div>hi</div>"),
        ok.replace("<div>hi", "<div><p>hi</p>"),
    ] {
        assert!(import::strict(&bad).is_none(), "{bad}");
    }
    assert!(import::strict("").is_none());
    assert!(import::strict("<!--komorebi:rich 1-->").is_none());
    assert!(import::strict(&format!("{ok}{ok}")).is_none());
}

#[test]
fn spaces_survive_collapsing() {
    let p = Para {
        runs: vec![
            Run::new("  a   b "),
            Run::styled(
                " c ",
                Style {
                    b: true,
                    ..Style::default()
                },
            ),
        ],
        ..Para::default()
    };
    let doc = Doc {
        blocks: vec![Block::Para(p)],
        ..Doc::default()
    }
    .normalized();
    let h = html(&doc, Mode::Send);
    assert!(h.contains("&nbsp; a &nbsp; b "), "{h}");
    let back = import::strict(&html(&doc, Mode::Draft)).unwrap();
    assert_eq!(back, doc);
}

#[test]
fn plain_text_message_is_plain_only() {
    for text in [
        "",
        "hello",
        "two\nlines\n\nand a gap",
        "a - b *c* _d_ [e](f)",
        "> quoted as typed",
    ] {
        let doc = Doc::from_plain(text);
        let (plain, h) = parts(&doc);
        assert!(h.is_none(), "{text:?}");
        assert_eq!(plain, text.trim_end_matches('\n'), "{text:?}");
    }
}

fn bold() -> Style {
    Style {
        b: true,
        ..Style::default()
    }
}

#[test]
fn any_formatting_needs_html() {
    let styled = |st: Style| {
        Doc {
            blocks: vec![Block::Para(Para {
                runs: vec![Run::styled("x", st)],
                ..Para::default()
            })],
            ..Doc::default()
        }
        .normalized()
    };
    assert!(needs_html(&styled(bold())));
    assert!(needs_html(&styled(Style {
        color: Some(ColorId::Red),
        ..Style::default()
    })));
    assert!(needs_html(&styled(Style {
        link: Some("https://a.test/".into()),
        ..Style::default()
    })));
    let mut d = Doc::from_plain("x");
    assert!(!needs_html(&d));
    d.color = Some(ColorId::Blue);
    assert!(needs_html(&d));
    let mut d = Doc::from_plain("x");
    d.size = SizePt::new(14).unwrap();
    assert!(needs_html(&d));
    let mut d = Doc::from_plain("x");
    d.font = FontId::Georgia;
    assert!(needs_html(&d));
    let mut d = Doc::from_plain("x");
    if let Some(Block::Para(p)) = d.blocks.first_mut() {
        p.align = Align::Center;
    }
    assert!(needs_html(&d));
    let mut d = Doc::from_plain("x");
    d.blocks.push(Block::Table(Table::new(2, 2).unwrap()));
    assert!(needs_html(&d));
}

fn doc_of(blocks: Vec<Block>) -> Doc {
    Doc {
        blocks,
        ..Doc::default()
    }
    .normalized()
}

fn para(text: &str) -> Para {
    Para::text(text)
}

#[test]
fn plain_output_golden() {
    let link = |u: &str| Style {
        link: Some(u.into()),
        ..Style::default()
    };
    let doc = doc_of(vec![
        Block::Para(Para {
            runs: vec![Run::new("Hello "), Run::styled("world", bold())],
            ..Para::default()
        }),
        Block::Para(Para {
            runs: vec![
                Run::new("see "),
                Run::styled("the site", link("https://example.org/a")),
            ],
            ..Para::default()
        }),
        Block::Para(Para {
            runs: vec![Run::styled(
                "https://example.org",
                link("https://example.org/"),
            )],
            ..Para::default()
        }),
        Block::Para(Para {
            list: Some((ListKind::Bullet, 0)),
            ..para("one")
        }),
        Block::Para(Para {
            list: Some((ListKind::Number, 1)),
            ..para("deep a")
        }),
        Block::Para(Para {
            list: Some((ListKind::Number, 1)),
            ..para("deep b")
        }),
        Block::Para(Para {
            list: Some((ListKind::Bullet, 0)),
            ..para("two")
        }),
        Block::Para(Para {
            quote: 2,
            ..para("deeply quoted")
        }),
        Block::Para(Para::default()),
        Block::Para(Para {
            quote: 1,
            ..Para::default()
        }),
    ]);
    assert_eq!(
        plain(&doc),
        "Hello world\nsee the site (https://example.org/a)\nhttps://example.org\n- one\n  1. deep a\n  2. deep b\n- two\n> > deeply quoted\n\n>"
    );
}

#[test]
fn plain_tables_align_or_fall_back() {
    let mut t = Table::new(2, 3).unwrap();
    t.rows[0][0] = Cell::text("Name");
    t.rows[0][1] = Cell::text("Qty");
    t.rows[0][2] = Cell::text("Price");
    t.rows[1][0] = Cell::text("Apples");
    t.rows[1][1] = Cell::text("3");
    t.rows[1][2] = Cell::text("1.20");
    let doc = doc_of(vec![Block::Table(t.clone())]);
    assert_eq!(plain(&doc), "Name    Qty  Price\nApples  3    1.20");
    t.rows[1][0] = Cell::text(&"w".repeat(80));
    let doc = doc_of(vec![Block::Table(t)]);
    assert_eq!(
        plain(&doc),
        format!("Name | Qty | Price\n{} | 3 | 1.20", "w".repeat(80))
    );
}

#[test]
fn plain_signature_keeps_the_delimiter_line() {
    let sig = SignatureBlock::new(
        vec![
            Block::Para(para("Ana")),
            Block::Para(para("ana@example.org")),
        ],
        Some(1),
    );
    let doc = doc_of(vec![Block::Para(para("Hi")), Block::Signature(sig)]);
    assert_eq!(plain(&doc), "Hi\n\n-- \nAna\nana@example.org");
    let empty = doc_of(vec![
        Block::Para(para("Hi")),
        Block::Signature(SignatureBlock::new(vec![], None)),
    ]);
    assert_eq!(plain(&empty), "Hi");
    assert_eq!(emit::signature_text(&[Block::Para(para("Ana"))]), "Ana");
}

#[test]
fn signature_untouched_mark() {
    let mut sig = SignatureBlock::new(vec![Block::Para(para("Ana"))], Some(1));
    assert!(sig.is_untouched());
    sig.blocks.push(Block::Para(para("more")));
    assert!(!sig.is_untouched());
    let doc = doc_of(vec![Block::Signature(SignatureBlock::new(
        vec![
            Block::Para(para("Ana")),
            Block::Para(pic_para("img-00000000000000aa@komorebi", 30, 40)),
        ],
        None,
    ))]);
    let back = import::strict(&html(&doc, Mode::Draft)).unwrap();
    match back.blocks.first() {
        Some(Block::Signature(s)) => assert!(s.is_untouched() && s.blocks.len() == 2),
        other => panic!("{other:?}"),
    }
}

fn pic_para(cid: &str, w: u32, h: u32) -> Para {
    Para {
        runs: vec![Run::picture(
            InlineImage::new(cid, w, h, "Logo"),
            Style::default(),
        )],
        ..Para::default()
    }
}

/// A picture on a line of its own is sent with half a line above and below
/// it, as the editor draws it; one beside text, one in a table cell and a
/// draft's copy are not.
#[test]
fn a_block_picture_is_sent_with_room_around_it() {
    let pad = "<div style=\"padding:.65em 0\"><img";
    let block = doc_of(vec![
        Block::Para(para("Open Settings:")),
        Block::Para(pic_para("img-00000000000000ab@komorebi", 300, 200)),
    ]);
    assert!(html(&block, Mode::Send).contains(pad));
    let draft = html(&block, Mode::Draft);
    assert!(!draft.contains("padding"), "{draft}");
    assert_eq!(import::strict(&draft).unwrap(), block);
    let mut beside = pic_para("img-00000000000000ac@komorebi", 300, 200);
    beside.runs.push(Run::styled(" Save", Style::default()));
    let h = html(&doc_of(vec![Block::Para(beside)]), Mode::Send);
    assert!(!h.contains("padding:.65em"), "{h}");
}

#[test]
fn a_picture_is_sent_by_cid_with_its_size_and_text() {
    let cid = "img-0123456789abcdef@komorebi";
    let mut p = para("Step 1: ");
    p.runs.push(Run::picture(
        InlineImage::new(cid, 1200, 600, "The \"Save\" button").sized(300),
        Style {
            b: true,
            link: Some("https://example.org/".into()),
            ..Style::default()
        },
    ));
    p.runs.push(Run::styled(" then OK", Style::default()));
    let doc = doc_of(vec![Block::Para(p)]);
    let h = html(&doc, Mode::Send);
    assert!(
        h.contains(&format!(
            "<a href=\"https://example.org/\"><img src=\"cid:{cid}\" width=\"300\" height=\"150\" alt=\"The &quot;Save&quot; button\" style=\"{}\"></a>",
            emit::IMG_STYLE
        )),
        "{h}"
    );
    // Bold does not reach a picture.
    assert!(!h.contains("<b><img"), "{h}");
    assert_eq!(h.matches("<img").count(), 1);
    assert!(!h.contains("data-iw"), "sent HTML has no editor marks: {h}");
    assert!(needs_html(&doc));
    // Its text in the text part, the link's address after it.
    assert_eq!(
        plain(&doc),
        "Step 1: [The \"Save\" button] (https://example.org/) then OK"
    );
    // In a draft it reads back exactly.
    assert_eq!(import::strict(&html(&doc, Mode::Draft)).unwrap(), doc);
    // A picture without text is "[image]" in the text part.
    let mut bare = doc_of(vec![Block::Para(pic_para("img-1@komorebi", 5, 5))]);
    if let Some(Block::Para(p)) = bare.blocks.first_mut() {
        p.runs[0].image.as_mut().unwrap().alt.clear();
    }
    assert_eq!(plain(&bare), "[image]");
}

#[test]
fn stored_quote_is_atomic_and_round_trips_verbatim() {
    let html_q = "<div class=\"komorebi_quote\" style=\"margin-top:1em\">On Mon Ana wrote:<blockquote><p>Hi <b>there</b> &amp; you</p></blockquote></div>";
    let q = QuoteBlock::new(html_q, true).unwrap();
    assert!(q.text().contains("Hi there & you"), "{:?}", q.text());
    let doc = doc_of(vec![Block::Para(para("Thanks")), Block::Quote(q.clone())]);
    let draft = html(&doc, Mode::Draft);
    assert_eq!(import::strict(&draft).unwrap(), doc);
    let sent = html(&doc, Mode::Send);
    assert!(sent.contains(q.html()));
    assert!(q.html().contains("On Mon Ana wrote:<blockquote>"));
    assert!(QuoteBlock::new("  ", false).is_none());
    assert!(QuoteBlock::new("<img src=\"http://x/y.png\">", false).is_none());
    let quoted: Vec<String> = q
        .text()
        .lines()
        .map(|l| {
            if l.is_empty() {
                ">".to_string()
            } else {
                format!("> {l}")
            }
        })
        .collect();
    assert_eq!(plain(&doc), format!("Thanks\n\n{}", quoted.join("\n")));
}

#[test]
fn caps_hold() {
    let doc = doc_of(
        (0..MAX_BLOCKS + 50)
            .map(|_| Block::Para(para("x")))
            .collect(),
    );
    assert_eq!(doc.blocks.len(), MAX_BLOCKS);

    let runs: Vec<Run> = (0..MAX_RUNS + 30)
        .map(|i| {
            Run::styled(
                "a",
                Style {
                    b: i % 2 == 0,
                    ..Style::default()
                },
            )
        })
        .collect();
    let doc = doc_of(vec![Block::Para(Para {
        runs,
        ..Para::default()
    })]);
    match doc.blocks.first() {
        Some(Block::Para(p)) => assert_eq!(p.runs.len(), MAX_RUNS),
        other => panic!("{other:?}"),
    }

    let big = "y".repeat(MAX_TEXT_BYTES + 100);
    let doc = doc_of(vec![Block::Para(para(&big)), Block::Para(para("more"))]);
    assert!(doc.text_bytes() <= MAX_TEXT_BYTES);

    let p = Para {
        indent: 99,
        quote: 99,
        list: Some((ListKind::Bullet, 99)),
        ..para("x")
    };
    let doc = doc_of(vec![Block::Para(p)]);
    match doc.blocks.first() {
        Some(Block::Para(p)) => assert_eq!(
            (p.indent, p.quote, p.list),
            (
                MAX_INDENT,
                MAX_QUOTE_LEVEL,
                Some((ListKind::Bullet, MAX_LIST_LEVEL))
            )
        ),
        other => panic!("{other:?}"),
    }

    let wide = Table {
        rows: (0..15)
            .map(|_| (0..15).map(|_| Cell::text("c")).collect())
            .collect(),
        ..Table::default()
    };
    let doc = doc_of(vec![Block::Table(wide)]);
    match doc.blocks.first() {
        Some(Block::Table(t)) => assert_eq!((t.row_count(), t.col_count()), (10, 10)),
        other => panic!("{other:?}"),
    }
    assert!(Table::new(11, 1).is_none() && Table::new(1, 0).is_none());

    // A nested signature in a signature is dropped.
    let inner = SignatureBlock::new(vec![Block::Para(para("a"))], None);
    let sig = SignatureBlock {
        blocks: vec![Block::Signature(inner), Block::Para(para("b"))],
        ..SignatureBlock::default()
    };
    let doc = doc_of(vec![Block::Signature(sig)]);
    match doc.blocks.first() {
        Some(Block::Signature(s)) => assert_eq!(s.blocks.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn table_operations() {
    let mut t = Table::new(2, 2).unwrap();
    assert!(t.insert_row(1) && t.insert_col(0));
    assert_eq!((t.row_count(), t.col_count()), (3, 3));
    assert!(!t.insert_row(9) && !t.delete_row(9) && !t.delete_col(9));
    assert!(t.delete_row(0) && t.delete_col(2));
    assert_eq!((t.row_count(), t.col_count()), (2, 2));
    assert!(t.delete_row(0) && !t.delete_row(0));
    assert!(t.delete_col(0) && !t.delete_col(0));
    let mut t = Table::new(10, 10).unwrap();
    assert!(!t.insert_row(0) && !t.insert_col(0));
}

#[test]
fn switching_to_plain_text_drops_formatting() {
    let mut r = Rng(5);
    for _ in 0..100 {
        let doc = gen_doc(&mut r);
        let p = doc.to_plain();
        assert!(!needs_html(&p));
        // Trailing blank lines aside (an empty last paragraph).
        assert_eq!(
            plain(&p).trim_end_matches('\n'),
            plain(&doc).trim_end_matches('\n')
        );
    }
}

#[test]
fn clear_formatting_keeps_text_and_links() {
    let mut doc = doc_of(vec![Block::Para(Para {
        runs: vec![
            Run::styled(
                "a",
                Style {
                    b: true,
                    color: Some(ColorId::Red),
                    ..Style::default()
                },
            ),
            Run::styled(
                "b",
                Style {
                    i: true,
                    link: Some("https://x.test/".into()),
                    ..Style::default()
                },
            ),
        ],
        ..Para::default()
    })]);
    let text = plain(&doc);
    doc.clear_formatting();
    assert_eq!(plain(&doc), text);
    match doc.blocks.first() {
        Some(Block::Para(p)) => {
            assert_eq!(p.runs.len(), 2);
            assert!(
                p.runs
                    .iter()
                    .all(|r| !r.style.b && !r.style.i && r.style.color.is_none())
            );
            assert!(p.runs[1].style.link.is_some());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn link_targets_are_followable_and_stable() {
    for (given, want) in [
        ("example.org", Some("https://example.org/")),
        ("http://example.org/a b", None),
        ("javascript:alert(1)", None),
        ("a@example.org", Some("mailto:a@example.org")),
        ("ftp://x.test/", None),
        ("", None),
        (
            "https://x.test/p(1)?q=[2]",
            Some("https://x.test/p%281%29?q=%5B2%5D"),
        ),
    ] {
        assert_eq!(link_target(given).as_deref(), want, "{given}");
        if let Some(w) = want {
            assert_eq!(link_target(w).as_deref(), Some(w), "stable {w}");
        }
    }
}

#[test]
fn font_size_and_colour_sets() {
    for f in FontId::ALL {
        assert_eq!(FontId::from_stack(f.stack()), Some(f));
        assert!(!f.stack().contains('"') && !f.stack().contains(';'));
    }
    for c in ColorId::ALL {
        assert_eq!(ColorId::from_hex(&c.hex()), Some(c));
    }
    assert_eq!(SizePt::DEFAULT.get(), 11);
    assert!(SizePt::new(13).is_none());
    assert_eq!(SizePt::nearest(13.4).get(), 14);
    assert_eq!(SizePt::new(36).unwrap().step(true).get(), 36);
    assert_eq!(SizePt::new(8).unwrap().step(false).get(), 8);
    assert_eq!(SizePt::new(11).unwrap().step(true).get(), 12);
    // Pasted colours: near-black and white are Automatic; others snap.
    assert_eq!(ColorId::nearest("#000000"), None);
    assert_eq!(ColorId::nearest("#ffffff"), None);
    assert_eq!(ColorId::nearest("#0f6cbd"), Some(ColorId::Blue));
    assert_eq!(ColorId::nearest("rgb(209, 52, 56)"), Some(ColorId::Red));
    assert_eq!(ColorId::nearest("url(javascript:x)"), None);
    // Any other colour stays exactly as it was.
    assert_eq!(
        ColorId::nearest("#1F3864"),
        Some(ColorId::Rgb(0x1F, 0x38, 0x64))
    );
    assert_eq!(ColorId::Rgb(0x1F, 0x38, 0x64).hex(), "#1F3864");
    assert_eq!(
        ColorId::from_hex("#1f3864"),
        Some(ColorId::Rgb(0x1F, 0x38, 0x64))
    );
    assert_eq!(ColorId::from_hex("#0F6CBD"), Some(ColorId::Blue));
    assert_eq!(ColorId::exact(0x0F, 0x6C, 0xBD), ColorId::Blue);
    assert_eq!(
        FontId::nearest("Helvetica, sans-serif"),
        Some(FontId::Arial)
    );
    assert_eq!(FontId::nearest("'Comic Sans MS', cursive"), None);
    assert_eq!(FontId::nearest("Foo, monospace"), Some(FontId::CourierNew));
}

// ---- lossy import ---------------------------------------------------------

fn lossy(h: &str) -> Doc {
    import::lossy(h).unwrap_or_else(|| panic!("nothing readable in {h}"))
}

fn run_texts(doc: &Doc) -> Vec<String> {
    doc.blocks
        .iter()
        .map(|b| match b {
            Block::Para(p) => p.plain_text(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn lossy_maps_what_the_model_holds() {
    let d = lossy(
        "<p>Hello <b>bold</b> <i>it</i> <u>u</u> <s>s</s> <a href=\"https://example.org\">site</a></p>\
         <p style=\"text-align:center\"><span style=\"font-family:Calibri,sans-serif;font-size:14pt;color:#d13438\">big red</span></p>\
         <ul><li>one</li><li>two<ul><li>nested</li></ul></li></ul>\
         <ol><li>first</li></ol>\
         <blockquote>quoted<blockquote>deeper</blockquote></blockquote>",
    );
    let texts = run_texts(&d);
    assert_eq!(
        texts,
        [
            "Hello bold it u s site",
            "big red",
            "one",
            "two",
            "nested",
            "first",
            "quoted",
            "deeper"
        ]
    );
    let Block::Para(p0) = &d.blocks[0] else {
        panic!()
    };
    assert!(p0.runs.iter().any(|r| r.text == "bold" && r.style.b));
    assert!(p0.runs.iter().any(|r| r.text == "it" && r.style.i));
    assert!(p0.runs.iter().any(|r| r.text == "u" && r.style.u));
    assert!(p0.runs.iter().any(|r| r.text == "s" && r.style.s));
    assert!(
        p0.runs
            .iter()
            .any(|r| r.style.link.as_deref() == Some("https://example.org/"))
    );
    let Block::Para(p1) = &d.blocks[1] else {
        panic!()
    };
    assert_eq!(p1.align, Align::Center);
    let st = &p1.runs[0].style;
    assert_eq!(
        (st.font, st.size.map(SizePt::get), st.color),
        (Some(FontId::Calibri), Some(14), Some(ColorId::Red))
    );
    let lists: Vec<_> = d
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Para(p) => p.list,
            _ => None,
        })
        .collect();
    assert_eq!(
        lists,
        [
            (ListKind::Bullet, 0),
            (ListKind::Bullet, 0),
            (ListKind::Bullet, 1),
            (ListKind::Number, 0)
        ]
    );
    let quotes: Vec<u8> = d
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Para(p) => Some(p.quote),
            _ => None,
        })
        .collect();
    assert_eq!(&quotes[quotes.len() - 2..], &[1, 2]);
}

#[test]
fn lossy_reads_word_and_gmail_style_markup() {
    let d = lossy(
        "<html><body><div dir=\"ltr\"><div>line one<br>line two</div><div><br></div><div>after</div></div>\
         <p class=MsoNormal style='margin:0cm'><span style='font-size:11.0pt;font-family:\"Calibri\",sans-serif;color:#1F497D'>Word text</span><o:p></o:p></p>\
         <span style=\"font-weight:700;font-style:italic;text-decoration:underline line-through\">styled</span></body></html>",
    );
    let t = run_texts(&d);
    assert_eq!(
        t,
        ["line one", "line two", "", "after", "Word text", "styled"]
    );
    let Block::Para(last) = d.blocks.last().unwrap() else {
        panic!()
    };
    let st = &last.runs[0].style;
    assert!(st.b && st.i && st.u && st.s);
}

#[test]
fn lossy_tables_and_font_tags() {
    let d = lossy(
        "<table border=1><tbody><tr><th>H1</th><th>H2</th></tr><tr><td>a<br>b</td><td><table><tr><td>nested</td></tr></table></td></tr></tbody></table>\
         <font face=\"Georgia\" color=\"#107c10\" size=\"5\">green</font>",
    );
    let Block::Table(t) = &d.blocks[0] else {
        panic!("{:?}", d.blocks)
    };
    assert_eq!((t.row_count(), t.col_count()), (2, 2));
    assert_eq!(t.rows[1][0].paras.len(), 2);
    assert_eq!(t.rows[1][1].plain_text(), "nested");
    let Block::Para(p) = d.blocks.last().unwrap() else {
        panic!()
    };
    let st = &p.runs[0].style;
    assert_eq!(
        (st.font, st.color, st.size.map(SizePt::get)),
        (Some(FontId::Georgia), Some(ColorId::Green), Some(18))
    );
}

#[test]
fn lossy_drops_everything_unsafe() {
    let d = lossy(
        "<p>safe</p><script>alert(1)</script><style>p{color:red}</style>\
         <img src=\"http://track.example/p.png\"><img src=\"cid:x\">\
         <iframe src=\"http://x\"></iframe><object data=x></object>\
         <a href=\"javascript:alert(1)\">js</a> <a href=\"data:text/html,x\">data</a> <a href=\"file:///c:/x\">file</a>\
         <p onclick=\"x()\" style=\"background:url(http://x/y);position:fixed\">styled</p>\
         <form action=\"http://x\"><input name=a value=b><button>Go</button></form>",
    );
    let all = format!("{d:?}");
    for bad in [
        "alert",
        "track.example",
        "javascript",
        "data:",
        "file:",
        "http://x",
        "onclick",
        "fixed",
        "color:red",
    ] {
        assert!(!all.contains(bad), "{bad} in {all}");
    }
    assert!(run_texts(&d).iter().any(|t| t == "safe"));
    let sent = html(&d, Mode::Send);
    assert!(
        !sent.contains("<img") && !sent.contains("<script") && !sent.contains("href=\"javascript")
    );
}

#[test]
fn lossy_survives_hostile_input() {
    let deep = format!("{}x{}", "<div>".repeat(20_000), "</div>".repeat(20_000));
    assert!(import::lossy(&deep).is_some_and(|d| plain(&d).contains('x')));

    let deep_quote = format!(
        "{}x{}",
        "<blockquote>".repeat(5_000),
        "</blockquote>".repeat(5_000)
    );
    let d = import::lossy(&deep_quote).unwrap();
    match d.blocks.first() {
        Some(Block::Para(p)) => assert!(p.quote <= MAX_QUOTE_LEVEL),
        other => panic!("{other:?}"),
    }

    let deep_list = format!(
        "{}<li>x{}",
        "<ul><li>".repeat(2_000),
        "</li></ul>".repeat(2_000)
    );
    let d = import::lossy(&deep_list).unwrap();
    assert!(d.blocks.iter().all(|b| match b {
        Block::Para(p) => p.list.is_none_or(|(_, l)| l <= MAX_LIST_LEVEL),
        _ => true,
    }));

    let mut huge = String::from("<table>");
    for _ in 0..200 {
        huge.push_str("<tr>");
        for _ in 0..100 {
            huge.push_str("<td>c</td>");
        }
        huge.push_str("</tr>");
    }
    huge.push_str("</table>");
    let d = import::lossy(&huge).unwrap();
    match d.blocks.first() {
        Some(Block::Table(t)) => assert_eq!((t.row_count(), t.col_count()), (10, 10)),
        other => panic!("{other:?}"),
    }

    let spans = "<b><i><u><s><span style=\"color:red\">".repeat(3_000) + "deep";
    assert!(import::lossy(&spans).is_some());

    assert!(import::lossy("").is_none());
    assert!(import::lossy("   <p> </p> <br> ").is_none());
    assert!(import::lossy("<script>x</script>").is_none());
    assert!(import::lossy(&"a".repeat(3 * 1024 * 1024)).is_none());
    // Junk that is not HTML at all.
    for junk in [
        "<<<>>>",
        "<a href=",
        "<div style=\"",
        "&#xFFFFFFFF;&#0;&bogus;",
        "<!--",
        "<![CDATA[",
        "\u{0}\u{1}<p>\u{2}</p>",
    ] {
        let _ = import::lossy(junk);
        let _ = import::strict(junk);
    }
}

#[test]
fn lossy_text_is_collapsed_like_a_browser() {
    let d = lossy("<div>  one \n   two\t three  <b> four </b> five&nbsp;&nbsp;six </div>");
    assert_eq!(run_texts(&d), ["one two three four five  six"]);
}

#[test]
fn lossy_result_survives_a_round_trip_through_the_draft_form() {
    let d = lossy("<p>x <b>y</b></p><ul><li>a</li></ul><table><tr><td>c</td></tr></table>");
    assert_eq!(import::strict(&html(&d, Mode::Draft)).unwrap(), d);
}

#[test]
fn from_stored_prefers_strict_then_lossy() {
    let doc = doc_of(vec![Block::Para(Para {
        runs: vec![Run::styled("x", bold())],
        ..Para::default()
    })]);
    assert_eq!(from_stored(&html(&doc, Mode::Draft)).unwrap(), doc);
    // The sent form has no tag: lossy reading gets the same text.
    let sent = from_stored(&html(&doc, Mode::Send)).unwrap();
    assert_eq!(plain(&sent), "x");
    assert!(from_stored("<p>server draft</p>").is_some());
}

#[test]
fn old_marker_drafts_upgrade_through_to_html_and_lossy_import() {
    use crate::mail::compose_html;
    let marker = "Hi **there**\n\n- one\n- two\n\n> quoted\n\nSee [site](https://example.org)";
    let d = import::lossy(&compose_html::to_html(marker)).unwrap();
    let t = plain(&d);
    assert!(
        t.contains("Hi there")
            && t.contains("- one")
            && t.contains("- two")
            && t.contains("> quoted")
    );
    assert!(t.contains("site (https://example.org/)"), "{t}");
    assert!(needs_html(&d));
}

#[test]
fn a_stored_quote_is_always_sanitized() {
    let q = QuoteBlock::new(
        "<div class=\"komorebi_quote\" style=\"margin-top:1em\" onclick=\"x()\">Hi<img src=\"http://track.example/p.png\"><script>alert(1)</script>\
         <a href=\"javascript:alert(1)\">js</a> <a href=\"https://ok.test/\">ok</a><p style=\"position:fixed;color:red\">p</p><!-- c --></div>",
        false,
    )
    .unwrap();
    for bad in [
        "img",
        "track.example",
        "script",
        "alert",
        "onclick",
        "javascript",
        "position",
        "<!--",
        "class",
    ] {
        assert!(!q.html().contains(bad), "{bad} in {}", q.html());
    }
    assert!(q.html().contains("https://ok.test/") && q.html().contains("margin-top"));
}

#[test]
fn sanitizing_a_quote_twice_changes_nothing() {
    use crate::mail::compose_html::{forward_quote, reply_quote, sanitize};
    let original = sanitize("<p>Hello <b>you</b> &amp; <a href=\"https://x.test/?a=1&b=2\">link</a></p><ul><li>a</li></ul><table><tr><td>c</td></tr></table>").unwrap();
    let samples = [
        reply_quote("On Monday, Ana <a@x.test> wrote:", &original),
        forward_quote(
            "---------- Forwarded message ----------",
            &[("From", "Ana <a@x.test>"), ("Subject", "A & B < C")],
            &original,
        ),
        "<div>line one\nline two\n  indented &lt;b&gt;</div>".to_string(),
        "<blockquote><blockquote>deep</blockquote></blockquote>".to_string(),
    ];
    for s in samples {
        let once = sanitize(&s).unwrap();
        assert_eq!(sanitize(&once).unwrap(), once, "{s}");
        let q = QuoteBlock::new(&s, false).unwrap();
        assert_eq!(q.html(), once);
        assert_eq!(QuoteBlock::new(q.html(), false).unwrap(), q);
    }
}

#[test]
fn a_hostile_tagged_draft_cannot_smuggle_markup_in_its_quote() {
    let q = QuoteBlock::new("<p>quoted</p>", false).unwrap();
    let doc = doc_of(vec![Block::Para(para("Hi")), Block::Quote(q)]);
    let good = html(&doc, Mode::Draft);
    assert!(import::strict(&good).is_some());
    let hostile = good.replace(
        "<p>quoted</p>",
        "<p onclick=\"steal()\">quoted</p><img src=\"http://track.example/p.png\"><script>alert(1)</script><iframe src=\"http://x\"></iframe>",
    );
    assert_ne!(hostile, good);
    let back = import::strict(&hostile).unwrap();
    let Some(Block::Quote(q)) = back.blocks.last() else {
        panic!("{:?}", back.blocks)
    };
    for bad in ["img", "track", "script", "onclick", "iframe", "alert"] {
        assert!(!q.html().contains(bad), "{bad} in {}", q.html());
    }
    for mode in [Mode::Send, Mode::Draft] {
        let out = html(&back, mode);
        assert!(
            !out.contains("track.example") && !out.contains("<script") && !out.contains("onclick")
        );
    }
    // A quote with nothing readable after sanitizing is refused outright.
    let empty = good.replace("<p>quoted</p>", "<script>x</script><img src=\"http://x\">");
    assert!(import::strict(&empty).is_none());
}

#[test]
fn a_server_drafts_copy_reads_back_as_the_same_document() {
    use crate::mail::smtp;
    let quote = QuoteBlock::new(
        "<div style=\"margin-top:1em\">On Mon,\nAna wrote:<blockquote><p>line one\nline two</p><p>caf\u{e9} \u{65e5}\u{672c}</p></blockquote></div>",
        true,
    )
    .unwrap();
    let sig = SignatureBlock::new(
        vec![
            Block::Para(para("Ana")),
            Block::Table(Table::new(2, 2).unwrap()),
            Block::Para(pic_para("img-6c6f676f6c6f676f@komorebi", 40, 20)),
        ],
        Some(5),
    );
    let mut doc = gen_doc(&mut Rng(99));
    doc.blocks.push(Block::Quote(quote));
    doc.blocks.push(Block::Signature(sig));
    let doc = doc.normalized();
    let stored = html(&doc, Mode::Draft);
    let raw = smtp::build_draft(
        smtp::Outgoing {
            from_email: "me@example.com".into(),
            to: "you@example.com".into(),
            body: plain(&doc),
            body_html: Some(stored.clone()),
            inline: vec![smtp::InlineImage {
                cid: "img-6c6f676f6c6f676f@komorebi".into(),
                mime: "image/png".into(),
                data: b"logo".to_vec(),
            }],
            ..Default::default()
        },
        7,
    )
    .unwrap();
    let msg = mail_parser::MessageParser::default()
        .parse(&raw[..])
        .unwrap();
    let part = msg.body_html(0).unwrap();
    assert!(
        part.contains("\r\n") || !stored.contains('\n'),
        "the server copy has CRLF line ends"
    );
    // Tagged: strict reading gives the identical document.
    assert_eq!(import::strict(&part), Some(doc.clone()));
    assert_eq!(from_stored(&part), Some(doc));
    // An untagged copy (another client's draft) goes through the lossy mapping.
    let foreign = "<div>Hi <b>there</b></div><ul><li>a</li></ul>";
    let d = from_stored(foreign).unwrap();
    assert_eq!(plain(&d), "Hi there\n- a");
}

// ---- pasted pictures and layout --------------------------------------------

/// A signature as Outlook or a web page puts it on the clipboard: a layout
/// table (logo block left, the text right), a row of partner logos under
/// it, pictures as `data:` addresses and a file on this PC, one remote.
/// Everything is made up (no real person or organisation); the pictures
/// are generated placeholders.
#[test]
fn a_pasted_multi_picture_signature_keeps_its_layout() {
    use images::tests::png;
    let logo = ImageFile::new(png(240, 160)).unwrap().0;
    let partner_a = ImageFile::new(png(180, 60)).unwrap().0;
    let partner_b = ImageFile::new(png(120, 60)).unwrap().0;
    let pasted = format!(
        r##"<html><body><table border="0" cellspacing="0" cellpadding="0" style="border-collapse:collapse">
<tr>
<td width="140" valign="top" style="width:105pt;padding:0 12px 0 0"><img src="{logo}" width="120" height="80" alt="Example Co logo"></td>
<td valign="top" style="border-left:solid #1F4E79 1.0pt;padding:0 0 0 12px">
<p style="margin:0"><b><span style="font-family:Arial;font-size:12pt;color:#1F4E79">Alex Example</span></b></p>
<p style="margin:0"><span style="font-family:Arial;font-size:9pt;color:#595959">Product Lead</span></p>
<p style="margin:0"><span style="font-size:9pt">+1 555 0100 &middot; <a href="https://example.org/">example.org</a></span></p>
</td>
</tr>
<tr>
<td colspan="2" style="padding-top:8px"><img src="file:///C:/Users/Someone/AppData/Local/Temp/msohtml1/01/clip_image003.png" width="90" alt="Partner A"> <img src="{b}" style="width:60px" alt="Partner B"> <img src="https://tracker.example/pixel.png" width="1" height="1"></td>
</tr>
</table></body></html>"##,
        logo = images::data_uri(&logo),
        b = images::data_uri(&partner_b),
    );
    let known = Images::default();
    let disk = partner_a.data.to_vec();
    let read = move |p: &std::path::Path| {
        p.to_string_lossy()
            .ends_with("clip_image003.png")
            .then(|| disk.clone())
    };
    let mut r = images::Resolver::new(&known, &read);
    let doc = import::lossy_with_images(&pasted, &mut |f| r.resolve(f)).unwrap();
    // The three pictures that need no network, each once; the remote one
    // is gone (never fetched).
    assert_eq!(r.found.len(), 3);
    assert!(r.found.contains(&logo.cid()) && r.found.contains(&partner_a.cid()));
    let Some(Block::Table(t)) = doc.blocks.first() else {
        panic!("{doc:#?}")
    };
    // Layout: two columns, no rules, the first 140 px wide, both on top.
    assert_eq!((t.row_count(), t.col_count()), (2, 2));
    assert!(t.borderless, "{t:#?}");
    assert_eq!(t.widths.first().copied(), Some(140));
    assert_eq!(t.rows[0][0].valign, VAlign::Top);
    assert_eq!(t.rows[0][1].valign, VAlign::Top);
    // The logo in the left cell at its given size.
    let left = &t.rows[0][0].paras[0].runs;
    let img = left[0].image.as_ref().unwrap();
    assert_eq!(
        (img.cid.as_str(), img.width, img.height()),
        (logo.cid().as_str(), 120, 80)
    );
    assert_eq!(img.alt, "Example Co logo");
    // The text on the right: its lines, bold name, font, size and colour
    // to the nearest allowed, and the link.
    let right = &t.rows[0][1];
    let lines: Vec<String> = right.paras.iter().map(Para::plain_text).collect();
    assert_eq!(
        lines,
        [
            "Alex Example",
            "Product Lead",
            "+1 555 0100 \u{b7} example.org"
        ]
    );
    let name = &right.paras[0].runs[0].style;
    assert!(name.b);
    assert_eq!(name.font, Some(FontId::Arial));
    assert_eq!(name.size, SizePt::new(12));
    // The author's colour exactly, not the nearest named one.
    assert_eq!(name.color, Some(ColorId::Rgb(0x1F, 0x4E, 0x79)), "{name:?}");
    let role = &right.paras[1].runs[0].style;
    assert_eq!(role.size, SizePt::new(9));
    let link = right.paras[2]
        .runs
        .iter()
        .find(|r| r.style.link.is_some())
        .unwrap();
    assert_eq!(link.style.link.as_deref(), Some("https://example.org/"));
    // The partner row: one cell across both columns, both logos side by
    // side in it; the cell it covers holds nothing.
    assert_eq!(t.rows[1][0].span(), (2, 1));
    assert!(t.rows[1][1].paras.is_empty());
    assert!(t.is_merged());
    let row: Vec<&InlineImage> = t.rows[1][0]
        .paras
        .iter()
        .flat_map(|p| &p.runs)
        .filter_map(|r| r.image.as_deref())
        .collect();
    assert_eq!(row.len(), 2);
    assert_eq!((row[0].alt.as_str(), row[0].width), ("Partner A", 90));
    assert_eq!((row[1].alt.as_str(), row[1].width), ("Partner B", 60));
    // As sent: a layout table (width attributes, no rules) and the
    // pictures by cid with their sizes.
    let sent = html_with_cids(&doc);
    assert!(
        sent.contains("<table border=\"0\" cellpadding=\"0\" cellspacing=\"0\""),
        "{sent}"
    );
    assert!(sent.contains("<td width=\"140\""), "{sent}");
    assert!(sent.contains("<td colspan=\"2\""), "{sent}");
    assert_eq!(sent.matches("<td").count(), 3, "{sent}");
    assert!(sent.contains("color:#1F4E79"), "{sent}");
    assert!(sent.contains("vertical-align:top"), "{sent}");
    assert!(sent.contains(&format!(
        "src=\"cid:{}\" width=\"120\" height=\"80\"",
        logo.cid()
    )));
    assert!(!sent.contains("border:1px"), "{sent}");
    // And kept exactly when saved.
    assert_eq!(import::strict(&html(&doc, Mode::Draft)).unwrap(), doc);
}

fn html_with_cids(doc: &Doc) -> String {
    html(doc, Mode::Send)
}

/// Word's and Gmail's usual markup: sizes to the nearest allowed, colours
/// as they are, a bordered table keeps its rules, a percent width is taken
/// of the table's (here 100%: of a typical message's 600 px).
#[test]
fn pasted_word_and_gmail_markup_maps_to_the_nearest_allowed() {
    let pasted = r##"<div dir="ltr"><font face="Georgia" size="4" color="#cc0000">Big red</font> <span style="font-size:13.5pt;color:rgb(0,0,238)">blue</span></div>
<table border="1" style="width:100%"><tr><td valign="middle" width="50%">a</td><td valign="bottom">b</td></tr></table>"##;
    let doc = import::lossy(pasted).unwrap();
    let Some(Block::Para(p)) = doc.blocks.first() else {
        panic!("{doc:#?}")
    };
    let big = &p.runs[0].style;
    assert_eq!(big.font, Some(FontId::Georgia));
    assert!(big.size.is_some() && big.color.is_some(), "{big:?}");
    let blue = &p.runs.last().unwrap().style;
    assert!(blue.size.is_some() && blue.color.is_some(), "{blue:?}");
    let t = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Table(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert!(!t.borderless);
    assert_eq!(t.widths, [300, 0], "{t:?}");
    assert_eq!(t.rows[0][0].valign, VAlign::Middle);
    assert_eq!(t.rows[0][1].valign, VAlign::Bottom);
    // Without a resolver (Edit quoted text, a foreign draft) no picture
    // comes in, and nothing breaks.
    let with_pic = format!("<p>x<img src=\"{}\">y</p>", "data:image/png;base64,AAAA");
    let d = import::lossy(&with_pic).unwrap();
    assert!(d.images().is_empty());
    assert_eq!(plain(&d), "xy");
}
