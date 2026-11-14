//! What screen readers get of the painted body (phase 13): the whole text
//! as the value of its `MultilineTextInput` node, one `TextRun` node per
//! line of the document (a paragraph, a list item, a table cell) with its
//! bounds, and the caret and the selection as the node's text selection,
//! so Narrator reads what was typed, line by line, and where the caret is.
//! Only the lines near the view are given (the editor's paint margin),
//! with the caret's and the selection's lines always among them.

use gpui::{A11ySubtreeBuilder, Bounds, Pixels, Role, accesskit};
use komorebi::mail::render::Rect;
use komorebi::mail::rich::edit::Editor;
use komorebi::mail::rich::view::View;

/// One line of the document as a screen reader gets it.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Byte offset of the line in the editor's text (its node's key).
    pub start: usize,
    pub text: String,
    /// The line's box on the page (CSS px).
    pub rect: Rect,
}

/// The body's text for the accessibility tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyText {
    pub value: String,
    pub lines: Vec<Line>,
    /// Anchor and caret: (index in `lines`, character in its line).
    pub selection: Option<((usize, usize), (usize, usize))>,
}

/// The lines of `ed` between `top` and `bottom` (CSS px on the page), the
/// ones holding `anchor` and `cursor` (byte offsets), and the selection.
pub fn body_text(
    ed: &Editor,
    view: &View,
    top: f32,
    bottom: f32,
    anchor: usize,
    cursor: usize,
) -> BodyText {
    let text = ed.text();
    let all = ed.lines();
    let index_of = |o: usize| all.partition_point(|l| l.start <= o).saturating_sub(1);
    let (ia, ic) = (index_of(anchor), index_of(cursor));
    // Lines are in page order: the first one reaching `top`.
    let first = all.partition_point(|l| {
        let c = view.caret(ed, l.end());
        c.y + c.h < top
    });
    let mut picked: Vec<usize> = Vec::new();
    for (i, l) in all.iter().enumerate().skip(first) {
        if view.caret(ed, l.start).y > bottom {
            break;
        }
        picked.push(i);
    }
    for i in [ia, ic] {
        if i < all.len() && !picked.contains(&i) {
            picked.push(i);
        }
    }
    picked.sort_unstable();
    let lines: Vec<Line> = picked
        .iter()
        .map(|&i| {
            let l = &all[i];
            let end = l.end().min(text.len());
            let start = l.start.min(end);
            Line {
                start: l.start,
                text: text.get(start..end).unwrap_or_default().to_string(),
                rect: line_rect(ed, view, start, end),
            }
        })
        .collect();
    let at = |line: usize, o: usize| -> Option<(usize, usize)> {
        let k = picked.iter().position(|&p| p == line)?;
        let l = &lines[k];
        let upto = o.saturating_sub(l.start).min(l.text.len());
        let chars = l.text.get(..upto).map_or(0, |s| s.chars().count());
        Some((k, chars))
    };
    let selection = at(ia, anchor).zip(at(ic, cursor));
    BodyText {
        value: text.to_string(),
        lines,
        selection,
    }
}

/// The box around a line's visual lines (the caret's box when it is
/// empty).
fn line_rect(ed: &Editor, view: &View, start: usize, end: usize) -> Rect {
    let rects = view.selection(ed, start, end);
    let Some(first) = rects.first().copied() else {
        return view.caret(ed, start);
    };
    rects.iter().fold(first, |a, r| {
        let (x0, y0) = (a.x.min(r.x), a.y.min(r.y));
        let (x1, y1) = ((a.x + a.w).max(r.x + r.w), (a.y + a.h).max(r.y + r.h));
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    })
}

/// Puts `body` under the body's node, for a page drawn at `page` (window
/// logical px) at device `scale`.
pub fn push(builder: &mut A11ySubtreeBuilder, body: &BodyText, page: Bounds<Pixels>, scale: f32) {
    let (ox, oy) = (f32::from(page.origin.x), f32::from(page.origin.y));
    let d = |v: f32| f64::from(v * scale);
    let mut ids = Vec::with_capacity(body.lines.len());
    for l in &body.lines {
        let id = builder.synthetic_node_id(l.start);
        let mut node = accesskit::Node::new(Role::TextRun);
        node.set_value(l.text.clone());
        node.set_character_lengths(
            l.text
                .chars()
                .map(|c| u8::try_from(c.len_utf8()).unwrap_or(1))
                .collect::<Vec<_>>(),
        );
        node.set_bounds(accesskit::Rect {
            x0: d(ox + l.rect.x),
            y0: d(oy + l.rect.y),
            x1: d(ox + l.rect.x + l.rect.w),
            y1: d(oy + l.rect.y + l.rect.h),
        });
        builder.push_child(id, node);
        ids.push(id);
    }
    let parent = builder.parent_node();
    parent.set_value(body.value.clone());
    if let Some(((la, ca), (lc, cc))) = body.selection
        && let (Some(&a), Some(&c)) = (ids.get(la), ids.get(lc))
    {
        parent.set_text_selection(accesskit::TextSelection {
            anchor: accesskit::TextPosition {
                node: a,
                character_index: ca,
            },
            focus: accesskit::TextPosition {
                node: c,
                character_index: cc,
            },
        });
    }
}
