//! What screen readers get of a page of pixels: its lines of text as
//! AccessKit `TextRun` nodes under the page's `Document` node
//! (`Element::a11y_synthetic_children`), each with its bounds, its
//! characters' lengths, left edges and widths and where its words start,
//! and the page's selection as the node's text selection, so UI
//! Automation's text pattern reads the message line by line, by word or by
//! character, gives each range its rectangle (what Narrator highlights)
//! and knows what is selected. A line that wraps ends with a space and a
//! paragraph's last line with a line break, so text read across lines
//! keeps its words apart; the page's value is the runs' text, which
//! AccessKit builds from them. The lines come from the render worker with
//! the page (`komorebi::reader::Reply::Text`), in view coordinates, the
//! selection as (line, character) (`Reply::TextSelection`); the bounds are
//! window device pixels, as GPUI gives its own nodes. The same shape as
//! the composer's body (`views::composer::a11y`).

use gpui::{A11ySubtreeBuilder, Bounds, Pixels, Role, accesskit};
use komorebi::mail::render::TextLine;
use komorebi::reader::LineRange;

/// One line of the page as a screen reader gets it.
#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    /// The line's text, with the space or line break that follows it.
    pub text: String,
    /// Window device pixels.
    pub bounds: accesskit::Rect,
    /// Each character's left edge from the line's start and its width
    /// (device pixels), the ending's at the line's end, zero wide.
    pub positions: Vec<f32>,
    pub widths: Vec<f32>,
}

/// The nodes for `lines` on a page drawn at `page` (window logical px) at
/// device `scale`.
pub fn text_nodes(lines: &[TextLine], page: Bounds<Pixels>, scale: f32) -> Vec<TextNode> {
    let (ox, oy) = (f32::from(page.origin.x), f32::from(page.origin.y));
    let d = |v: f32| f64::from(v * scale);
    lines
        .iter()
        .map(|l| {
            let mut text = l.text.clone();
            text.push(if l.ends_paragraph { '\n' } else { ' ' });
            let (mut positions, mut widths): (Vec<f32>, Vec<f32>) =
                l.chars.iter().map(|&(x, w)| (x * scale, w * scale)).unzip();
            // A line without its boxes (older callers) still counts its
            // characters; the ending sits at the line's end.
            let n = l.text.chars().count();
            positions.resize(n, 0.0);
            widths.resize(n, 0.0);
            let end = positions
                .last()
                .zip(widths.last())
                .map_or(0.0, |(x, w)| x + w);
            positions.push(end);
            widths.push(0.0);
            TextNode {
                text,
                bounds: accesskit::Rect {
                    x0: d(ox + l.x),
                    y0: d(oy + l.y),
                    x1: d(ox + l.x + l.w),
                    y1: d(oy + l.y + l.h),
                },
                positions,
                widths,
            }
        })
        .collect()
}

/// The UTF-8 length of each character of `text` (a `TextRun`'s character
/// lengths).
pub fn character_lengths(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(c.len_utf8()).unwrap_or(1))
        .collect()
}

/// The characters of `text` where a word starts: one that is not a space
/// after one that is (or at the run's start), as AccessKit's own examples
/// count them. AccessKit keeps them as `u8`, so a word past the 255th
/// character of a line is not among them.
pub fn word_starts(text: &str) -> Vec<u8> {
    let mut prev_space = true;
    let mut out = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let space = c.is_whitespace();
        if prev_space
            && !space
            && let Ok(i) = u8::try_from(i)
        {
            out.push(i);
        }
        prev_space = space;
    }
    out
}

/// The page node's text selection for `range` over the line nodes `ids`
/// (`None` when nothing is selected or a line is not among them).
pub fn text_selection(
    range: Option<LineRange>,
    ids: &[accesskit::NodeId],
) -> Option<accesskit::TextSelection> {
    let ((la, ca), (lb, cb)) = range?;
    Some(accesskit::TextSelection {
        anchor: accesskit::TextPosition {
            node: *ids.get(la)?,
            character_index: ca,
        },
        focus: accesskit::TextPosition {
            node: *ids.get(lb)?,
            character_index: cb,
        },
    })
}

/// The `TextRun` node of a line.
fn run_node(n: TextNode) -> accesskit::Node {
    let mut node = accesskit::Node::new(Role::TextRun);
    node.set_character_lengths(character_lengths(&n.text));
    node.set_character_positions(n.positions);
    node.set_character_widths(n.widths);
    node.set_word_starts(word_starts(&n.text));
    node.set_value(n.text);
    node.set_bounds(n.bounds);
    node
}

/// Adds `nodes` as the page node's children (the composer's Preview, which
/// has no selection of its own).
pub fn push(builder: &mut A11ySubtreeBuilder, nodes: Vec<TextNode>) {
    push_selected(builder, nodes, None);
}

/// Adds `nodes` as the page node's children, and `range` as its text
/// selection.
pub fn push_selected(
    builder: &mut A11ySubtreeBuilder,
    nodes: Vec<TextNode>,
    range: Option<LineRange>,
) {
    let mut ids = Vec::with_capacity(nodes.len());
    for (i, n) in nodes.into_iter().enumerate() {
        let id = builder.synthetic_node_id(i);
        builder.push_child(id, run_node(n));
        ids.push(id);
    }
    let parent = builder.parent_node();
    // Character boxes run from the line's left edge.
    parent.set_text_direction(accesskit::TextDirection::LeftToRight);
    if let Some(selection) = text_selection(range, &ids) {
        parent.set_text_selection(selection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn line(x: f32, y: f32, text: &str, ends_paragraph: bool) -> TextLine {
        // 8 px a character.
        let n = text.chars().count();
        TextLine {
            x,
            y,
            w: n as f32 * 8.0,
            h: 20.0,
            text: text.into(),
            chars: (0..n).map(|i| (i as f32 * 8.0, 8.0)).collect(),
            ends_paragraph,
        }
    }

    fn page() -> Bounds<Pixels> {
        Bounds::new(point(px(100.), px(300.)), size(px(600.), px(800.)))
    }

    #[test]
    fn lines_get_window_device_bounds() {
        let lines = vec![
            line(4.0, 10.0, "Olá Ana,", true),
            TextLine {
                w: 80.0,
                ..line(4.0, 34.0, "Obrigada", true)
            },
        ];
        let nodes = text_nodes(&lines, page(), 1.5);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].text, "Olá Ana,\n");
        let b = nodes[1].bounds;
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (156.0, 501.0, 276.0, 531.0));
        // The characters' boxes in device pixels, the ending's at the end.
        assert_eq!(nodes[1].positions.len(), "Obrigada\n".chars().count());
        assert_eq!((nodes[1].positions[1], nodes[1].widths[1]), (12.0, 12.0));
        assert_eq!((nodes[1].positions[8], nodes[1].widths[8]), (96.0, 0.0));
    }

    /// The run's character lengths add up to its UTF-8 value (UI
    /// Automation counts characters with them).
    #[test]
    fn character_lengths_follow_utf8() {
        assert_eq!(character_lengths("Olá"), [1, 1, 2]);
        assert_eq!(
            character_lengths("Olá Ana, ✓")
                .iter()
                .map(|&n| usize::from(n))
                .sum::<usize>(),
            "Olá Ana, ✓".len()
        );
    }

    /// Words start after spaces, a no-break space among them; none past
    /// what a `u8` holds.
    #[test]
    fn words_start_after_spaces() {
        assert_eq!(word_starts("Olá Ana,  rever-te!\n"), [0, 4, 10]);
        assert_eq!(word_starts(" a\u{a0}b "), [1, 3]);
        let long = "a ".repeat(200);
        assert_eq!(word_starts(&long).len(), 128);
    }

    /// The worker's (line, character) range becomes the page node's text
    /// selection over its line nodes; none, or a line not given, gives none.
    #[test]
    fn the_selection_lands_on_the_line_nodes() {
        let ids = [accesskit::NodeId(7), accesskit::NodeId(8)];
        let sel = text_selection(Some(((0, 2), (1, 5))), &ids).expect("a selection");
        assert_eq!(
            (sel.anchor.node, sel.anchor.character_index),
            (accesskit::NodeId(7), 2)
        );
        assert_eq!(
            (sel.focus.node, sel.focus.character_index),
            (accesskit::NodeId(8), 5)
        );
        assert!(text_selection(None, &ids).is_none());
        assert!(text_selection(Some(((0, 0), (2, 0))), &ids).is_none());
    }

    /// What UI Automation's text pattern gets, through the AccessKit
    /// consumer the Windows adapter reads: the page's text with its words
    /// apart across a wrapped line, words one by one, each line as a line,
    /// and a rectangle for a range (what Narrator highlights).
    #[test]
    fn the_text_pattern_reads_words_lines_and_rectangles() {
        use accesskit::{Node, NodeId, Tree, TreeId, TreeUpdate};
        let lines = [
            line(4.0, 10.0, "Obrigada pelo", false),
            line(4.0, 34.0, "jantar.", true),
            line(4.0, 58.0, "Beijos", true),
        ];
        let nodes = text_nodes(&lines, page(), 1.5);
        let ids: Vec<NodeId> = (1..=nodes.len() as u64).map(NodeId).collect();
        let mut root = Node::new(Role::Document);
        root.set_children(ids.clone());
        root.set_text_direction(accesskit::TextDirection::LeftToRight);
        let mut all = vec![(NodeId(0), root)];
        all.extend(ids.iter().copied().zip(nodes.into_iter().map(run_node)));
        let tree = accesskit_consumer::Tree::new(
            TreeUpdate {
                nodes: all,
                tree: Some(Tree::new(NodeId(0))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            false,
        );
        let doc = tree.state().root();
        assert_eq!(
            doc.document_range().text(),
            "Obrigada pelo jantar.\nBeijos\n"
        );
        // By word: "pelo", then "jantar." on the next line, then "Beijos".
        let w1 = doc.document_start().forward_to_word_start();
        let w2 = w1.forward_to_word_start();
        let w3 = w2.forward_to_word_start();
        assert_eq!([w1, w2, w3].map(|p| p.to_global_usv_index()), [9, 14, 22]);
        // By line.
        let second = doc.line_range_from_index(1).expect("a second line");
        assert_eq!(second.text(), "jantar.\n");
        // "pelo " has its own rectangle in window device pixels: its
        // line's left edge plus its characters'.
        let mut word = w1.to_degenerate_range();
        word.set_end(w1.forward_to_word_end());
        let boxes = word.bounding_boxes();
        assert_eq!(boxes.len(), 1, "{boxes:?}");
        let b = boxes[0];
        assert_eq!((b.x0, b.y0, b.y1), ((104.0 + 72.0) * 1.5, 465.0, 495.0));
        assert!(b.x1 > b.x0 && b.x1 <= (104.0 + 13.0 * 8.0) * 1.5, "{b:?}");
        // A point on "jantar" is a position in it.
        let at = doc.text_position_at_point(accesskit::Point::new(
            (104.0 + 20.0) * 1.5,
            (300.0 + 40.0) * 1.5,
        ));
        assert_eq!(at.to_global_usv_index(), 16);
    }
}
