//! The formatted editor's commands, shared by both apps' editors: a
//! toolbar button or a formatting key on the selection of an [`Editor`]
//! (Komorebi's own document model, `mail::rich::edit`), and the documents
//! a formatted message starts from. Each app's editor owns its caret,
//! picture and input; what a command does to the document is here.

use std::collections::HashMap;
use std::sync::Arc;

use crate::db::Db;
use crate::mail::rich::edit::{CharOp, Editor, ParaOp, TableOp};
use crate::mail::rich::{self as model, Align, ColorId, FontId, ListKind, SizePt};

/// A toolbar button or a formatting key.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Bold,
    Italic,
    Underline,
    Strike,
    Font(FontId),
    Size(SizePt),
    /// Ctrl+] (true) and Ctrl+[.
    SizeStep(bool),
    Color(Option<ColorId>),
    Bullets,
    Numbers,
    /// Indent (true) or outdent.
    Indent(bool),
    Align(Align),
    Quote,
    Clear,
    Table(TableOp),
    Undo,
    Redo,
    /// "Show quoted text" on a folded quote.
    ShowQuote,
    /// Turn a stored quote into text that can be edited.
    EditQuote,
}

/// What a command did: whether the document changed, and the selection
/// (anchor, cursor) to show after it, when it moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub changed: bool,
    pub sel: Option<(usize, usize)>,
}

/// `action` on the selection `anchor..cursor` of `ed`.
pub fn apply(ed: &mut Editor, (a, c): (usize, usize), action: &Action) -> Applied {
    let chars = |ed: &mut Editor, op: CharOp| Applied {
        changed: ed.format_chars(a, c, &op),
        sel: None,
    };
    let paras = |ed: &mut Editor, op: ParaOp| Applied {
        changed: ed.format_paras(a, c, op),
        sel: None,
    };
    match action {
        Action::Bold => chars(ed, CharOp::Bold),
        Action::Italic => chars(ed, CharOp::Italic),
        Action::Underline => chars(ed, CharOp::Underline),
        Action::Strike => chars(ed, CharOp::Strike),
        Action::Font(f) => chars(ed, CharOp::Font(*f)),
        Action::Size(s) => chars(ed, CharOp::Size(*s)),
        Action::SizeStep(up) => chars(ed, CharOp::SizeStep(*up)),
        Action::Color(col) => chars(ed, CharOp::Color(*col)),
        Action::Bullets => paras(ed, ParaOp::List(ListKind::Bullet)),
        Action::Numbers => paras(ed, ParaOp::List(ListKind::Number)),
        Action::Indent(on) => paras(ed, ParaOp::Indent(*on)),
        Action::Align(al) => paras(ed, ParaOp::Align(*al)),
        Action::Quote => paras(ed, ParaOp::Quote),
        Action::Clear => Applied {
            changed: ed.clear_formatting(a, c),
            sel: None,
        },
        Action::Table(op) => {
            let caret = ed.table_op(c, *op);
            Applied {
                changed: caret.is_some(),
                sel: caret.map(|x| (x, x)),
            }
        }
        Action::Undo | Action::Redo => {
            let sel = if *action == Action::Undo {
                ed.undo()
            } else {
                ed.redo()
            };
            Applied {
                changed: sel.is_some(),
                sel,
            }
        }
        Action::ShowQuote => Applied {
            changed: ed.fold_quote(false),
            sel: None,
        },
        Action::EditQuote => Applied {
            changed: ed.expand_quote(),
            sel: None,
        },
    }
}

/// A document from the settings' defaults and plain `text`.
pub fn doc_from_text(text: &str, defaults: (FontId, SizePt, Option<ColorId>)) -> model::Doc {
    let mut doc = if text.is_empty() {
        model::Doc::default()
    } else {
        model::Doc::from_plain(text)
    };
    (doc.font, doc.size, doc.color) = defaults;
    doc
}

/// Whether the message shows nothing written (the placeholder shows).
pub fn is_blank(ed: &Editor) -> bool {
    ed.doc().blocks.iter().all(|b| match b {
        model::Block::Para(p) => p.is_empty(),
        _ => false,
    })
}

/// The pictures of the signature logos in `doc` not in `known` yet, read
/// from the cache (a logo whose picture changed since is left out).
pub fn logos(db: &Db, doc: &model::Doc, known: &HashMap<u64, Arc<[u8]>>) -> Vec<(u64, Arc<[u8]>)> {
    let mut out = Vec::new();
    for b in &doc.blocks {
        let model::Block::Signature(s) = b else {
            continue;
        };
        let (Some(l), Some(id)) = (s.logo, s.source) else {
            continue;
        };
        if known.contains_key(&l.hash) || out.iter().any(|(h, _)| *h == l.hash) {
            continue;
        }
        if let Ok(Some(sig)) = db.signature(id)
            && let Some(bytes) = sig.logo
            && model::fnv(&bytes) == l.hash
        {
            out.push((l.hash, Arc::from(bytes)));
        }
    }
    out
}

/// A painted piece's pixels as straight (not premultiplied) BGRA, the
/// order and form GPUI's `RenderImage` holds: (width, height, bytes).
pub fn straight_bgra(pm: tiny_skia::Pixmap) -> (u32, u32, Vec<u8>) {
    let (w, h) = (pm.width(), pm.height());
    let mut data = pm.take();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
        let a = u16::from(px[3]);
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    (w, h, data)
}

/// A followable link address from what was typed in the link field, set
/// on the selection (or inserted at the caret with the address as its
/// text). `None`: not a web or e-mail address, nothing changed.
pub fn link(ed: &mut Editor, (a, c): (usize, usize), url: &str) -> Option<(usize, usize)> {
    let target = model::link_target(url)?;
    Some(ed.link(a, c, &target, "").unwrap_or((a, c)))
}
