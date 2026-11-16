//! The formatted editor's commands, shared by both apps' editors: a
//! toolbar button or a formatting key on the selection of an [`Editor`]
//! (Komorebi's own document model, `mail::rich::edit`), and the documents
//! a formatted message starts from. Each app's editor owns its caret,
//! picture and input; what a command does to the document is here.

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
    /// "Insert picture": the app opens its file picker (nothing here).
    InsertPicture,
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
        Action::InsertPicture => Applied::default(),
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

/// A picture of the document decoded (first frame of a GIF) and scaled
/// down towards `width_px` device px, for the editor's view. Slow for a big
/// file: never on the UI thread. `None` if it does not decode.
pub fn decode_picture(file: &model::ImageFile, width_px: u32) -> Option<Arc<tiny_skia::Pixmap>> {
    crate::mail::render::canvas::decode_image(&file.data, width_px.max(1)).map(Arc::new)
}

/// Put a stored signature in the message (`None`: take it out), its
/// pictures copied in with it. True if anything changed.
pub fn put_signature(ed: &mut Editor, sig: Option<&crate::db::Signature>) -> bool {
    if let Some(s) = sig {
        ed.add_images(&s.images);
    }
    ed.set_signature(sig.and_then(crate::db::Signature::block))
}

/// The pictures a saved draft shows that were not saved with it (an old
/// signature logo), from the stored signatures.
pub fn missing_images(db: &Db, ed: &mut Editor) {
    let doc = ed.doc().clone();
    if let Ok(all) = db.with_signature_images(&doc, ed.images()) {
        ed.add_images(&all);
    }
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
