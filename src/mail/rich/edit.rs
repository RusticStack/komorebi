//! The editor's engine: a [`Doc`] edited through a plain-text projection.
//!
//! The composer's input proxy (a hidden text field that owns the caret,
//! IME, dead keys and the screen reader's text) holds [`Editor::text`]:
//! one line per paragraph (a table cell's paragraphs included), and one
//! object character (U+FFFC) for the stored quote and for a signature's
//! logo. Every change the proxy reports comes back as a replacement of a
//! byte range ([`Editor::apply`]), which this module maps onto the model.
//! An edit the model cannot hold (joining a cell with the text after its
//! table, typing inside the quote) is refused, and the caller puts the
//! projection back into the proxy.
//!
//! Formatting works on byte ranges of the projection too, and every change
//! records the top-level blocks it replaced, both for undo (one stack of
//! text and formatting steps, [`MAX_UNDO`] deep) and for the view, which
//! lays out only those blocks again ([`Editor::take_changes`]).

use super::model::*;

/// The character that stands for an object (the quote, a logo).
pub const OBJ: char = '\u{FFFC}';
/// Undo steps kept.
pub const MAX_UNDO: usize = 200;
/// Characters typed in a row that undo as one step.
const MAX_BURST: usize = 32;

/// A sequence of blocks: the message's, or a signature's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Top,
    /// The signature at this top-level index.
    Sig(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atom {
    Quote,
    Logo,
}

/// Where a projection line comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub flow: Flow,
    /// Index in the flow's blocks (a logo: one past the last).
    pub item: usize,
    /// Row and column of a table cell.
    pub cell: Option<(usize, usize)>,
    /// The paragraph in a cell (0 outside tables).
    pub para: usize,
    pub atom: Option<Atom>,
}

impl Pos {
    fn para(flow: Flow, item: usize) -> Pos {
        Pos {
            flow,
            item,
            cell: None,
            para: 0,
            atom: None,
        }
    }

    /// The top-level block the line belongs to.
    pub fn top(&self) -> usize {
        match self.flow {
            Flow::Top => self.item,
            Flow::Sig(i) => i,
        }
    }

    /// The sequence a paragraph line can be joined with (none for an
    /// object).
    fn container(&self) -> Option<Container> {
        if self.atom.is_some() {
            return None;
        }
        Some(match self.cell {
            None => Container::Flow(self.flow),
            Some((r, c)) => Container::Cell(self.flow, self.item, r, c),
        })
    }

    /// The item of `flow` this line lies in (a signature's lines lie in
    /// the signature's item of the top flow).
    fn item_in(&self, flow: Flow) -> Option<usize> {
        if self.flow == flow {
            return Some(self.item);
        }
        match (flow, self.flow) {
            (Flow::Top, Flow::Sig(i)) => Some(i),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Flow(Flow),
    Cell(Flow, usize, usize, usize),
}

/// One line of the projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub pos: Pos,
    /// Byte offset in [`Editor::text`].
    pub start: usize,
    pub len: usize,
}

impl Line {
    pub fn end(&self) -> usize {
        self.start.saturating_add(self.len)
    }
}

/// The top-level blocks `at..at + old_len` were replaced by
/// `at..at + new_len`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub at: usize,
    pub old_len: usize,
    pub new_len: usize,
    /// The message defaults (font, size, colour) changed: every block.
    pub all: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepKind {
    Typing,
    Deleting,
    Other,
}

type Defaults = (FontId, SizePt, Option<ColorId>);

struct Step {
    at: usize,
    before: Vec<Block>,
    after_len: usize,
    defaults: Defaults,
    sel_before: (usize, usize),
    sel_after: (usize, usize),
    kind: StepKind,
    burst: usize,
}

/// The blocks an edit may change, copied before it starts.
struct Snap {
    at: usize,
    before: Vec<Block>,
    old_total: usize,
    defaults: Defaults,
}

/// Where the caret goes after an edit.
enum Target {
    Para(Pos, usize),
    Offset(usize),
}

/// A character format toggled or set on a range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharOp {
    Bold,
    Italic,
    Underline,
    Strike,
    Font(FontId),
    Size(SizePt),
    /// Ctrl+] (true) and Ctrl+[.
    SizeStep(bool),
    Color(Option<ColorId>),
    /// Clear character formatting (links stay).
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParaOp {
    List(ListKind),
    Indent(bool),
    Align(Align),
    Quote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableOp {
    RowAbove,
    RowBelow,
    ColLeft,
    ColRight,
    DeleteRow,
    DeleteCol,
    DeleteTable,
}

/// The formatting at the caret (or common to the selection), for the
/// toolbar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Format {
    pub b: bool,
    pub i: bool,
    pub u: bool,
    pub s: bool,
    pub font: FontId,
    pub size: SizePt,
    pub color: Option<ColorId>,
    pub link: Option<String>,
    pub align: Align,
    pub list: Option<ListKind>,
    pub quote: bool,
    pub in_table: bool,
    /// The caret is in the signature.
    pub in_signature: bool,
}

pub struct Editor {
    doc: Doc,
    lines: Vec<Line>,
    text: String,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Formatting for the next characters typed at a collapsed caret.
    pending: Option<(usize, Style)>,
    changes: Vec<Change>,
    /// Where the last typing or deleting step left the caret.
    last_caret: Option<usize>,
}

/// Remove characters the model never holds from typed or pasted text.
fn clean(ins: &str) -> String {
    let mut out = String::with_capacity(ins.len());
    let mut chars = ins.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            '\u{a0}' => out.push(' '),
            OBJ => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn floor_boundary(s: &str, mut at: usize) -> usize {
    at = at.min(s.len());
    while at > 0 && !s.is_char_boundary(at) {
        at = at.saturating_sub(1);
    }
    at
}

// ---- paragraph editing ---------------------------------------------------

impl Para {
    /// Bytes of text.
    pub fn len(&self) -> usize {
        self.runs
            .iter()
            .fold(0usize, |n, r| n.saturating_add(r.text.len()))
    }

    /// Make a run boundary at byte `at`; the index of the run that starts
    /// there (the run count at the end).
    fn cut(&mut self, at: usize) -> usize {
        let mut pos = 0usize;
        let mut i = 0usize;
        while let Some(r) = self.runs.get_mut(i) {
            let len = r.text.len();
            if at <= pos {
                return i;
            }
            if at < pos.saturating_add(len) {
                let k = floor_boundary(&r.text, at.saturating_sub(pos));
                if k == 0 {
                    return i;
                }
                let tail = r.text.split_off(k);
                let style = r.style.clone();
                self.runs
                    .insert(i.saturating_add(1), Run::styled(tail, style));
                return i.saturating_add(1);
            }
            pos = pos.saturating_add(len);
            i = i.saturating_add(1);
        }
        self.runs.len()
    }

    /// Empty runs out, equal neighbours merged.
    fn tidy(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        for r in runs {
            if r.text.is_empty() {
                continue;
            }
            match self.runs.last_mut() {
                Some(last) if last.style == r.style => last.text.push_str(&r.text),
                _ => self.runs.push(r),
            }
        }
    }

    /// The style of the character before byte `at`, else of the first.
    fn style_near(&self, at: usize) -> Option<&Style> {
        let mut pos = 0usize;
        let mut prev = None;
        for r in &self.runs {
            let end = pos.saturating_add(r.text.len());
            if at > pos && at <= end {
                return Some(&r.style);
            }
            if at <= pos {
                return prev.or(Some(&r.style));
            }
            prev = Some(&r.style);
            pos = end;
        }
        prev
    }

    /// Whether a link ends at `at` (typing there does not extend it).
    fn link_ends_at(&self, at: usize) -> bool {
        let mut pos = 0usize;
        let mut before: Option<&Style> = None;
        let mut after: Option<&Style> = None;
        for r in &self.runs {
            let end = pos.saturating_add(r.text.len());
            if end == at {
                before = Some(&r.style);
            }
            if pos == at {
                after = Some(&r.style);
            }
            pos = end;
        }
        match before {
            Some(b) if b.link.is_some() => after.is_none_or(|a| a.link != b.link),
            _ => false,
        }
    }

    /// Replace bytes `a..b` with `text` in `style`.
    fn put(&mut self, a: usize, b: usize, text: &str, style: Style) {
        let i = self.cut(a);
        let j = self.cut(b.max(a));
        if let Some(gone) = self.runs.get(i..j.max(i)) {
            let _ = gone;
            self.runs.drain(i..j.max(i));
        }
        if !text.is_empty() {
            self.runs
                .insert(i.min(self.runs.len()), Run::styled(text, style));
        }
        self.tidy();
    }

    /// The paragraph from byte `at` on, with this one's alignment, list,
    /// indent and quote; this one keeps the text before.
    fn split_tail(&mut self, at: usize) -> Para {
        let i = self.cut(at);
        let tail = self.runs.split_off(i.min(self.runs.len()));
        Para {
            runs: tail,
            ..self.props()
        }
    }

    /// An empty paragraph with this one's paragraph formatting.
    fn props(&self) -> Para {
        Para {
            runs: Vec::new(),
            align: self.align,
            indent: self.indent,
            list: self.list,
            quote: self.quote,
        }
    }

    fn join(&mut self, mut other: Para) {
        self.runs.append(&mut other.runs);
        self.tidy();
    }

    /// Change the style of bytes `a..b`.
    fn restyle(&mut self, a: usize, b: usize, f: &dyn Fn(&mut Style)) {
        if a >= b {
            return;
        }
        let i = self.cut(a);
        let j = self.cut(b);
        for r in self.runs.iter_mut().take(j).skip(i) {
            f(&mut r.style);
        }
        self.tidy();
    }

    /// Whether every character of `a..b` passes `f` (true when empty).
    fn all(&self, a: usize, b: usize, f: &dyn Fn(&Style) -> bool) -> bool {
        let mut pos = 0usize;
        for r in &self.runs {
            let end = pos.saturating_add(r.text.len());
            if end > a && pos < b && !f(&r.style) {
                return false;
            }
            pos = end;
        }
        true
    }
}

fn flow_blocks(doc: &Doc, f: Flow) -> Option<&Vec<Block>> {
    match f {
        Flow::Top => Some(&doc.blocks),
        Flow::Sig(i) => match doc.blocks.get(i)? {
            Block::Signature(s) => Some(&s.blocks),
            _ => None,
        },
    }
}

fn flow_blocks_mut(doc: &mut Doc, f: Flow) -> Option<&mut Vec<Block>> {
    match f {
        Flow::Top => Some(&mut doc.blocks),
        Flow::Sig(i) => match doc.blocks.get_mut(i)? {
            Block::Signature(s) => Some(&mut s.blocks),
            _ => None,
        },
    }
}

fn para_of(doc: &Doc, pos: Pos) -> Option<&Para> {
    if pos.atom.is_some() {
        return None;
    }
    let blocks = flow_blocks(doc, pos.flow)?;
    match (blocks.get(pos.item)?, pos.cell) {
        (Block::Para(p), None) => Some(p),
        (Block::Table(t), Some((r, c))) => t.rows.get(r)?.get(c)?.paras.get(pos.para),
        _ => None,
    }
}

fn para_of_mut(doc: &mut Doc, pos: Pos) -> Option<&mut Para> {
    if pos.atom.is_some() {
        return None;
    }
    let blocks = flow_blocks_mut(doc, pos.flow)?;
    match (blocks.get_mut(pos.item)?, pos.cell) {
        (Block::Para(p), None) => Some(p),
        (Block::Table(t), Some((r, c))) => t.rows.get_mut(r)?.get_mut(c)?.paras.get_mut(pos.para),
        _ => None,
    }
}

fn cell_paras_mut(doc: &mut Doc, pos: Pos) -> Option<&mut Vec<Para>> {
    let (r, c) = pos.cell?;
    match flow_blocks_mut(doc, pos.flow)?.get_mut(pos.item)? {
        Block::Table(t) => Some(&mut t.rows.get_mut(r)?.get_mut(c)?.paras),
        _ => None,
    }
}

/// Insert `paras` after the paragraph at `pos` in its container; the
/// position of the last one.
fn insert_after(doc: &mut Doc, pos: Pos, paras: Vec<Para>) -> Option<Pos> {
    let n = paras.len();
    if n == 0 {
        return Some(pos);
    }
    if pos.cell.is_some() {
        let v = cell_paras_mut(doc, pos)?;
        let at = pos.para.saturating_add(1).min(v.len());
        v.splice(at..at, paras);
        return Some(Pos {
            para: pos.para.saturating_add(n),
            ..pos
        });
    }
    let v = flow_blocks_mut(doc, pos.flow)?;
    let at = pos.item.saturating_add(1).min(v.len());
    v.splice(at..at, paras.into_iter().map(Block::Para));
    Some(Pos::para(pos.flow, pos.item.saturating_add(n)))
}

/// Insert `parts` (text split at line breaks) into the paragraph at `pos`
/// at byte `at`. Where the caret ends.
fn insert_parts(doc: &mut Doc, pos: Pos, at: usize, parts: &[&str], style: &Style) -> Target {
    let Some(p) = para_of_mut(doc, pos) else {
        return Target::Offset(0);
    };
    let at = floor_boundary(&p.plain_text(), at);
    let Some((first, rest)) = parts.split_first() else {
        return Target::Para(pos, at);
    };
    if rest.is_empty() {
        p.put(at, at, first, style.clone());
        return Target::Para(pos, at.saturating_add(first.len()));
    }
    // Enter on an empty list item ends the list (Word, Outlook).
    if parts.len() == 2
        && parts.iter().all(|s| s.is_empty())
        && p.is_empty()
        && p.list.is_some()
        && pos.cell.is_none()
    {
        p.list = None;
        return Target::Para(pos, 0);
    }
    let tail = p.split_tail(at);
    p.put(at, at, first, style.clone());
    let props = p.props();
    let mut news: Vec<Para> = Vec::with_capacity(rest.len());
    for s in rest {
        let mut np = props.clone();
        if !s.is_empty() {
            np.runs.push(Run::styled(*s, style.clone()));
        }
        news.push(np);
    }
    let last_len = rest.last().map_or(0, |s| s.len());
    if let Some(last) = news.last_mut() {
        last.join(tail);
    }
    match insert_after(doc, pos, news) {
        Some(p) => Target::Para(p, last_len),
        None => Target::Offset(0),
    }
}

/// Paragraphs made of `parts`, unformatted but for `style`.
fn paras_of(parts: &[&str], style: &Style) -> Vec<Para> {
    parts
        .iter()
        .map(|s| {
            let mut p = Para::default();
            if !s.is_empty() {
                p.runs.push(Run::styled(*s, style.clone()));
            }
            p
        })
        .collect()
}

// ---- the editor ------------------------------------------------------------

impl Editor {
    pub fn new(mut doc: Doc) -> Editor {
        doc.normalize();
        fix_structure(&mut doc);
        let mut e = Editor {
            doc,
            lines: Vec::new(),
            text: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            changes: Vec::new(),
            last_caret: None,
        };
        e.rebuild();
        e
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// The projection the input proxy holds.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The block splices since the last call, oldest first.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    /// The paragraph a line shows (none for an object).
    pub fn para(&self, pos: Pos) -> Option<&Para> {
        para_of(&self.doc, pos)
    }

    /// The line holding byte `offset` and the offset in it.
    pub fn locate(&self, offset: usize) -> Option<(usize, usize)> {
        if self.lines.is_empty() {
            return None;
        }
        let offset = offset.min(self.text.len());
        let i = self
            .lines
            .partition_point(|l| l.start <= offset)
            .saturating_sub(1);
        let l = self.lines.get(i)?;
        Some((i, offset.saturating_sub(l.start).min(l.len)))
    }

    fn rebuild(&mut self) {
        self.lines.clear();
        self.text.clear();
        let push = |pos: Pos, s: &str, lines: &mut Vec<Line>, text: &mut String| {
            if !lines.is_empty() {
                text.push('\n');
            }
            lines.push(Line {
                pos,
                start: text.len(),
                len: s.len(),
            });
            text.push_str(s);
        };
        let obj = OBJ.to_string();
        let (lines, text) = (&mut self.lines, &mut self.text);
        for (i, b) in self.doc.blocks.iter().enumerate() {
            match b {
                Block::Para(p) => push(Pos::para(Flow::Top, i), &p.plain_text(), lines, text),
                Block::Table(t) => {
                    table_lines(Flow::Top, i, t, &mut |pos, s| push(pos, s, lines, text))
                }
                Block::Signature(s) => {
                    let f = Flow::Sig(i);
                    for (j, ib) in s.blocks.iter().enumerate() {
                        match ib {
                            Block::Para(p) => push(Pos::para(f, j), &p.plain_text(), lines, text),
                            Block::Table(t) => {
                                table_lines(f, j, t, &mut |pos, s| push(pos, s, lines, text))
                            }
                            Block::Signature(_) | Block::Quote(_) => {}
                        }
                    }
                    if s.logo.is_some() {
                        let pos = Pos {
                            atom: Some(Atom::Logo),
                            ..Pos::para(f, s.blocks.len())
                        };
                        push(pos, &obj, lines, text);
                    }
                }
                Block::Quote(q) => {
                    if !q.folded {
                        let pos = Pos {
                            atom: Some(Atom::Quote),
                            ..Pos::para(Flow::Top, i)
                        };
                        push(pos, &obj, lines, text);
                    }
                }
            }
        }
    }

    fn line(&self, i: usize) -> Option<Line> {
        self.lines.get(i).copied()
    }

    /// The projection offset of a caret target.
    fn resolve(&self, t: Target) -> usize {
        match t {
            Target::Offset(o) => floor_boundary(&self.text, o),
            Target::Para(pos, off) => self
                .lines
                .iter()
                .find(|l| l.pos == pos)
                .map_or(0, |l| l.start.saturating_add(off.min(l.len))),
        }
    }

    fn snap(&self, t0: usize, t1: usize) -> Snap {
        let t1 = t1.min(self.doc.blocks.len().saturating_sub(1));
        let t0 = t0.min(t1);
        Snap {
            at: t0,
            before: self
                .doc
                .blocks
                .get(t0..=t1)
                .map(<[Block]>::to_vec)
                .unwrap_or_default(),
            old_total: self.doc.blocks.len(),
            defaults: (self.doc.font, self.doc.size, self.doc.color),
        }
    }

    fn snap_all(&self) -> Snap {
        Snap {
            at: 0,
            before: self.doc.blocks.clone(),
            old_total: self.doc.blocks.len(),
            defaults: (self.doc.font, self.doc.size, self.doc.color),
        }
    }

    /// The length of a snapshot's range now.
    fn now_len(&self, s: &Snap) -> usize {
        (s.before.len().saturating_add(self.doc.blocks.len())).saturating_sub(s.old_total)
    }

    fn rollback(&mut self, s: Snap) {
        let n = self.now_len(&s);
        let end = s.at.saturating_add(n).min(self.doc.blocks.len());
        let at = s.at.min(end);
        self.doc.blocks.splice(at..end, s.before);
        (self.doc.font, self.doc.size, self.doc.color) = s.defaults;
    }

    /// Record a finished edit: undo step (typing coalesced), change for the
    /// view, projection again.
    fn commit(
        &mut self,
        s: Snap,
        sel_before: (usize, usize),
        caret: Target,
        kind: StepKind,
    ) -> Option<usize> {
        fix_structure(&mut self.doc);
        let new_len = self.now_len(&s);
        let unchanged = new_len == s.before.len()
            && s.defaults == (self.doc.font, self.doc.size, self.doc.color)
            && self
                .doc
                .blocks
                .get(s.at..s.at.saturating_add(new_len))
                .is_some_and(|now| now == s.before.as_slice());
        if unchanged
            || self.doc.text_bytes() > MAX_TEXT_BYTES
            || self.doc.blocks.len() > MAX_BLOCKS
            || too_many_runs(&self.doc)
        {
            self.rollback(s);
            return None;
        }
        let all = s.defaults != (self.doc.font, self.doc.size, self.doc.color);
        self.changes.push(Change {
            at: s.at,
            old_len: s.before.len(),
            new_len,
            all,
        });
        self.rebuild();
        let caret = self.resolve(caret);
        let coalesce = kind != StepKind::Other
            && self.last_caret == Some(sel_before.1)
            && self.undo.last().is_some_and(|last| {
                last.kind == kind
                    && last.at == s.at
                    && last.after_len == s.before.len()
                    && new_len == s.before.len()
                    && last.burst < MAX_BURST
            });
        if coalesce {
            if let Some(last) = self.undo.last_mut() {
                last.sel_after = (caret, caret);
                last.burst = last.burst.saturating_add(1);
            }
        } else {
            self.undo.push(Step {
                at: s.at,
                before: s.before,
                after_len: new_len,
                defaults: s.defaults,
                sel_before,
                sel_after: (caret, caret),
                kind,
                burst: 1,
            });
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_caret = (kind != StepKind::Other).then_some(caret);
        self.pending = None;
        Some(caret)
    }

    /// The style typed text takes at byte `at` of line `li`.
    fn typing_style(&self, li: usize, off: usize) -> Style {
        if let Some((at, st)) = &self.pending
            && self
                .line(li)
                .is_some_and(|l| l.start.saturating_add(off) == *at)
        {
            return st.clone();
        }
        let Some(p) = self.line(li).and_then(|l| para_of(&self.doc, l.pos)) else {
            return Style::default();
        };
        let mut st = p.style_near(off).cloned().unwrap_or_default();
        if st.link.is_some() && (off == 0 || p.link_ends_at(off)) {
            st.link = None;
        }
        st
    }

    /// The proxy's text changed to `new` (the caret now at `cursor`, if
    /// known): the same change in the model. The caret, or `None` if the
    /// change was refused (put [`Editor::text`] back in the proxy).
    pub fn apply(&mut self, new: &str, cursor: Option<usize>) -> Option<usize> {
        let old = self.text.as_bytes();
        let nb = new.as_bytes();
        let min = old.len().min(nb.len());
        let most_p = old.iter().zip(nb).take_while(|(a, b)| a == b).count();
        let most_s = old
            .iter()
            .rev()
            .zip(nb.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(min);
        // The same change can often be read at several places ("a" typed
        // into "aa"): the one that ends at the caret is the one made.
        let (p, s) = match cursor.filter(|c| *c <= nb.len()) {
            Some(c) if nb.len().saturating_sub(c) <= most_s => {
                let s = nb.len().saturating_sub(c);
                (most_p.min(min.saturating_sub(s)), s)
            }
            _ => (most_p, most_s.min(min.saturating_sub(most_p))),
        };
        let p = floor_boundary(new, floor_boundary(&self.text, p));
        let mut old_end = old.len().saturating_sub(s).max(p);
        let mut new_end = nb.len().saturating_sub(s).max(p);
        while old_end < old.len() && !self.text.is_char_boundary(old_end) {
            old_end = old_end.saturating_add(1);
        }
        while new_end < nb.len() && !new.is_char_boundary(new_end) {
            new_end = new_end.saturating_add(1);
        }
        let ins = new.get(p..new_end)?;
        let sel = (p, old_end);
        self.replace(p, old_end, ins, sel)
    }

    /// Replace bytes `a..b` of the projection with `ins`. The caret after
    /// it, or `None` if the model cannot hold the change.
    pub fn replace(&mut self, a: usize, b: usize, ins: &str, sel: (usize, usize)) -> Option<usize> {
        let (a, b) = (a.min(b), a.max(b));
        let a = floor_boundary(&self.text, a);
        let b = floor_boundary(&self.text, b);
        let ins = clean(ins);
        if a == b && ins.is_empty() {
            return Some(a);
        }
        let kind = if a == b && !ins.contains('\n') && ins.chars().count() == 1 {
            StepKind::Typing
        } else if ins.is_empty() && b.saturating_sub(a) <= 4 {
            StepKind::Deleting
        } else {
            StepKind::Other
        };
        let (ls, ao) = self.locate(a)?;
        let (le, bo) = self.locate(b)?;
        let style = self.typing_style(ls, ao);
        let parts: Vec<&str> = ins.split('\n').collect();
        let s_line = self.line(ls)?;
        let e_line = self.line(le)?;
        let snap = self.snap(s_line.pos.top(), e_line.pos.top());
        let target = if ls == le {
            self.edit_one(s_line, ao, bo, &parts, &style)
        } else {
            self.edit_many(ls, ao, le, bo, &parts, &style)
        };
        match target {
            Some(t) => self.commit(snap, sel, t, kind),
            None => {
                self.rollback(snap);
                None
            }
        }
    }

    fn edit_one(
        &mut self,
        l: Line,
        ao: usize,
        bo: usize,
        parts: &[&str],
        style: &Style,
    ) -> Option<Target> {
        let pos = l.pos;
        match pos.atom {
            None => {
                let p = para_of_mut(&mut self.doc, pos)?;
                if bo > ao {
                    p.put(ao, bo, "", Style::default());
                }
                Some(insert_parts(&mut self.doc, pos, ao, parts, style))
            }
            Some(atom) => {
                let replace = bo > ao;
                let at_start = ao == 0 && !replace;
                let ins_empty = parts.iter().all(|s| s.is_empty()) && parts.len() <= 1;
                if replace {
                    // The object itself is replaced: it goes.
                    let (flow, at) = match atom {
                        Atom::Quote => {
                            self.doc.blocks.get(pos.item)?;
                            self.doc.blocks.remove(pos.item);
                            (Flow::Top, pos.item)
                        }
                        Atom::Logo => {
                            let Some(Block::Signature(s)) = self.doc.blocks.get_mut(pos.top())
                            else {
                                return None;
                            };
                            s.logo = None;
                            (pos.flow, s.blocks.len())
                        }
                    };
                    if ins_empty {
                        return Some(Target::Offset(l.start));
                    }
                    let paras = paras_of(parts, style);
                    let n = paras.len();
                    let v = flow_blocks_mut(&mut self.doc, flow)?;
                    let at = at.min(v.len());
                    v.splice(at..at, paras.into_iter().map(Block::Para));
                    let last = parts.last().map_or(0, |s| s.len());
                    return Some(Target::Para(
                        Pos::para(flow, at.saturating_add(n).saturating_sub(1)),
                        last,
                    ));
                }
                // Text typed before or after the object: paragraphs of
                // their own next to it.
                let mut parts: Vec<&str> = parts.to_vec();
                if at_start {
                    let ends_with_break =
                        parts.len() > 1 && parts.last().is_some_and(|s| s.is_empty());
                    if ends_with_break {
                        parts.pop();
                    }
                    let (flow, at) = match atom {
                        Atom::Quote => (Flow::Top, pos.item),
                        Atom::Logo => (pos.flow, pos.item),
                    };
                    let paras = paras_of(&parts, style);
                    let n = paras.len();
                    let v = flow_blocks_mut(&mut self.doc, flow)?;
                    let at = at.min(v.len());
                    v.splice(at..at, paras.into_iter().map(Block::Para));
                    if ends_with_break {
                        // The caret stays with the object.
                        let obj = match atom {
                            Atom::Quote => Pos {
                                item: pos.item.saturating_add(n),
                                ..pos
                            },
                            Atom::Logo => Pos {
                                item: pos.item.saturating_add(n),
                                ..pos
                            },
                        };
                        return Some(Target::Para(obj, 0));
                    }
                    let last = parts.last().map_or(0, |s| s.len());
                    Some(Target::Para(
                        Pos::para(flow, at.saturating_add(n).saturating_sub(1)),
                        last,
                    ))
                } else {
                    if parts.len() > 1 && parts.first().is_some_and(|s| s.is_empty()) {
                        parts.remove(0);
                    }
                    let at = pos.top().saturating_add(1);
                    let paras = paras_of(&parts, style);
                    let n = paras.len();
                    let at = at.min(self.doc.blocks.len());
                    self.doc
                        .blocks
                        .splice(at..at, paras.into_iter().map(Block::Para));
                    let last = parts.last().map_or(0, |s| s.len());
                    Some(Target::Para(
                        Pos::para(Flow::Top, at.saturating_add(n).saturating_sub(1)),
                        last,
                    ))
                }
            }
        }
    }

    fn edit_many(
        &mut self,
        ls: usize,
        ao: usize,
        le: usize,
        bo: usize,
        parts: &[&str],
        style: &Style,
    ) -> Option<Target> {
        let s = self.line(ls)?;
        let e = self.line(le)?;
        let (sp, ep) = (s.pos, e.pos);
        // (E) whole items of one flow: from the start of one to the end of
        // another.
        for flow in [sp.flow, Flow::Top] {
            let (Some(x), Some(y)) = (sp.item_in(flow), ep.item_in(flow)) else {
                continue;
            };
            let first = self.first_line_of(flow, x);
            let last = self.last_line_of(flow, y);
            if first == Some(ls) && ao == 0 && last == Some(le) && bo == e.len {
                let v = flow_blocks_mut(&mut self.doc, flow)?;
                let end = y.saturating_add(1).min(v.len());
                let x = x.min(end);
                v.drain(x..end);
                let paras = paras_of(parts, style);
                let n = paras.len();
                v.splice(x..x, paras.into_iter().map(Block::Para));
                let last = parts.last().map_or(0, |s| s.len());
                return Some(Target::Para(
                    Pos::para(flow, x.saturating_add(n).saturating_sub(1)),
                    last,
                ));
            }
        }
        let (cs, ce) = (sp.container(), ep.container());
        // (A) two paragraphs of one container.
        if cs.is_some() && cs == ce {
            let tail = para_of_mut(&mut self.doc, ep)?.split_tail(bo);
            match cs? {
                Container::Cell(..) => {
                    let v = cell_paras_mut(&mut self.doc, sp)?;
                    let from = sp.para.saturating_add(1);
                    let to = ep.para.saturating_add(1).min(v.len());
                    if from <= to {
                        v.drain(from..to);
                    }
                }
                Container::Flow(f) => {
                    let v = flow_blocks_mut(&mut self.doc, f)?;
                    let from = sp.item.saturating_add(1);
                    let to = ep.item.saturating_add(1).min(v.len());
                    if from <= to {
                        v.drain(from..to);
                    }
                }
            }
            let p = para_of_mut(&mut self.doc, sp)?;
            let len = p.len();
            p.put(ao, len, "", Style::default());
            let t = insert_parts(&mut self.doc, sp, ao, parts, style);
            let (tp, toff) = match t {
                Target::Para(p, o) => (p, o),
                Target::Offset(_) => return None,
            };
            para_of_mut(&mut self.doc, tp)?.join(tail);
            return Some(Target::Para(tp, toff));
        }
        // (B) from the start of an item to text in a later paragraph of
        // the same flow.
        if ao == 0 && ep.atom.is_none() && ep.cell.is_none() {
            let flow = ep.flow;
            if let Some(x) = sp.item_in(flow)
                && self.first_line_of(flow, x) == Some(ls)
                && x < ep.item
            {
                let v = flow_blocks_mut(&mut self.doc, flow)?;
                let to = ep.item.min(v.len());
                v.drain(x..to);
                let np = Pos::para(flow, x);
                para_of_mut(&mut self.doc, np)?.put(0, bo, "", Style::default());
                return Some(insert_parts(&mut self.doc, np, 0, parts, style));
            }
        }
        // (D) from text in a paragraph to the start of a later item of the
        // same flow.
        if bo == 0 && sp.atom.is_none() && sp.cell.is_none() {
            let flow = sp.flow;
            if let Some(y) = ep.item_in(flow)
                && self.first_line_of(flow, y) == Some(le)
                && y > sp.item
            {
                let p = para_of_mut(&mut self.doc, sp)?;
                let len = p.len();
                p.put(ao, len, "", Style::default());
                let v = flow_blocks_mut(&mut self.doc, flow)?;
                let from = sp.item.saturating_add(1);
                let to = y.min(v.len());
                if from < to {
                    v.drain(from..to);
                }
                return Some(insert_parts(&mut self.doc, sp, ao, parts, style));
            }
        }
        None
    }

    fn first_line_of(&self, flow: Flow, item: usize) -> Option<usize> {
        self.lines
            .iter()
            .position(|l| l.pos.item_in(flow) == Some(item))
    }

    fn last_line_of(&self, flow: Flow, item: usize) -> Option<usize> {
        self.lines
            .iter()
            .rposition(|l| l.pos.item_in(flow) == Some(item))
    }

    /// Paragraph lines between `a` and `b` with the byte range of each in
    /// the range (a collapsed range: the caret's paragraph, empty).
    fn para_spans(&self, a: usize, b: usize) -> Vec<(Pos, usize, usize)> {
        let (a, b) = (a.min(b), a.max(b));
        let (Some((ls, ao)), Some((le, bo))) = (self.locate(a), self.locate(b)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, l) in self
            .lines
            .iter()
            .enumerate()
            .take(le.saturating_add(1))
            .skip(ls)
        {
            if l.pos.atom.is_some() {
                continue;
            }
            let from = if i == ls { ao } else { 0 };
            let to = if i == le { bo } else { l.len };
            out.push((l.pos, from, to.max(from)));
        }
        out
    }

    /// Character formatting on `a..b`; at a collapsed caret, the format of
    /// what is typed next. True if anything changed.
    pub fn format_chars(&mut self, a: usize, b: usize, op: &CharOp) -> bool {
        let (a, b) = (a.min(b), a.max(b));
        let defaults = (self.doc.font, self.doc.size, self.doc.color);
        if a == b {
            let Some((li, off)) = self.locate(a) else {
                return false;
            };
            let mut st = self.typing_style(li, off);
            let fmt = self.format_at(a, a);
            let on = match op {
                CharOp::Bold => !fmt.b,
                CharOp::Italic => !fmt.i,
                CharOp::Underline => !fmt.u,
                CharOp::Strike => !fmt.s,
                _ => false,
            };
            apply_char(&mut st, op, on, defaults);
            self.pending = Some((a, st));
            return true;
        }
        let spans = self.para_spans(a, b);
        if spans.is_empty() {
            return false;
        }
        let on = match op {
            CharOp::Bold | CharOp::Italic | CharOp::Underline | CharOp::Strike => {
                let has = |st: &Style| match op {
                    CharOp::Bold => st.b,
                    CharOp::Italic => st.i,
                    CharOp::Underline => st.u,
                    _ => st.s,
                };
                !spans
                    .iter()
                    .all(|(pos, f, t)| self.para(*pos).is_none_or(|p| p.all(*f, *t, &has)))
            }
            _ => false,
        };
        let t0 = spans.first().map_or(0, |s| s.0.top());
        let t1 = spans.last().map_or(0, |s| s.0.top());
        let snap = self.snap(t0, t1);
        let before = self.doc.clone_range(t0, t1);
        for (pos, f, t) in &spans {
            if let Some(p) = para_of_mut(&mut self.doc, *pos) {
                p.restyle(*f, *t, &|st| apply_char(st, op, on, defaults));
            }
        }
        if self.doc.clone_range(t0, t1) == before {
            return false;
        }
        self.commit(snap, (a, b), Target::Offset(b), StepKind::Other);
        true
    }

    /// Make `a..b` a link to `url` (already checked), or at a collapsed
    /// caret insert `label` (the address if empty) as one. The new
    /// selection.
    pub fn link(&mut self, a: usize, b: usize, url: &str, label: &str) -> Option<(usize, usize)> {
        let (a, b) = (a.min(b), a.max(b));
        if a == b {
            let text = if label.trim().is_empty() { url } else { label };
            let (li, off) = self.locate(a)?;
            let mut st = self.typing_style(li, off);
            st.link = Some(url.to_string());
            self.pending = Some((a, st));
            let caret = self.replace(a, a, text, (a, a))?;
            return Some((caret, caret));
        }
        let spans = self.para_spans(a, b);
        let t0 = spans.first().map_or(0, |s| s.0.top());
        let t1 = spans.last().map_or(0, |s| s.0.top());
        let snap = self.snap(t0, t1);
        let link = Some(url.to_string());
        for (pos, f, t) in &spans {
            if let Some(p) = para_of_mut(&mut self.doc, *pos) {
                p.restyle(*f, *t, &|st| st.link.clone_from(&link));
            }
        }
        let caret = self.commit(snap, (a, b), Target::Offset(b), StepKind::Other)?;
        Some((caret, caret))
    }

    /// Paragraph formatting on every paragraph `a..b` touches. True if
    /// anything changed.
    pub fn format_paras(&mut self, a: usize, b: usize, op: ParaOp) -> bool {
        let spans = self.para_spans(a, b);
        let spans: Vec<Pos> = spans
            .into_iter()
            .map(|s| s.0)
            .filter(|p| p.cell.is_none() || matches!(op, ParaOp::Align(_)))
            .collect();
        if spans.is_empty() {
            return false;
        }
        let paras: Vec<&Para> = spans.iter().filter_map(|p| self.para(*p)).collect();
        let all_list = |k: ListKind| paras.iter().all(|p| p.list.map(|l| l.0) == Some(k));
        let all_quoted = paras.iter().all(|p| p.quote > 0);
        let t0 = spans.first().map_or(0, Pos::top);
        let t1 = spans.last().map_or(0, Pos::top);
        let list_off = matches!(op, ParaOp::List(k) if all_list(k));
        let snap = self.snap(t0, t1);
        let before = self.doc.clone_range(t0, t1);
        for pos in &spans {
            let Some(p) = para_of_mut(&mut self.doc, *pos) else {
                continue;
            };
            match op {
                ParaOp::List(k) => {
                    p.list = if list_off {
                        None
                    } else {
                        Some((k, p.list.map_or(0, |l| l.1)))
                    };
                }
                ParaOp::Indent(more) => match p.list {
                    Some((k, l)) => {
                        let l = if more {
                            l.saturating_add(1).min(MAX_LIST_LEVEL)
                        } else {
                            l.saturating_sub(1)
                        };
                        p.list = Some((k, l));
                    }
                    None => {
                        p.indent = if more {
                            p.indent.saturating_add(1).min(MAX_INDENT)
                        } else {
                            p.indent.saturating_sub(1)
                        };
                    }
                },
                ParaOp::Align(al) => p.align = al,
                ParaOp::Quote => p.quote = if all_quoted { 0 } else { p.quote.max(1) },
            }
        }
        if self.doc.clone_range(t0, t1) == before {
            return false;
        }
        let (a, b) = (a.min(b), a.max(b));
        self.commit(snap, (a, b), Target::Offset(b), StepKind::Other);
        true
    }

    /// Clear formatting: character formatting of the range (of the caret's
    /// paragraph when collapsed), and the paragraphs' alignment and indent.
    pub fn clear_formatting(&mut self, a: usize, b: usize) -> bool {
        let (a, b) = (a.min(b), a.max(b));
        let spans = self.para_spans(a, b);
        if spans.is_empty() {
            return false;
        }
        let t0 = spans.first().map_or(0, |s| s.0.top());
        let t1 = spans.last().map_or(0, |s| s.0.top());
        let snap = self.snap(t0, t1);
        let before = self.doc.clone_range(t0, t1);
        for (pos, f, t) in &spans {
            let Some(p) = para_of_mut(&mut self.doc, *pos) else {
                continue;
            };
            let (f, t) = if a == b { (0, p.len()) } else { (*f, *t) };
            p.restyle(f, t, &|st| {
                let link = st.link.take();
                *st = Style::default();
                st.link = link;
            });
            p.align = Align::Left;
            p.indent = 0;
        }
        self.pending = None;
        if self.doc.clone_range(t0, t1) == before {
            return false;
        }
        self.commit(snap, (a, b), Target::Offset(b), StepKind::Other);
        true
    }

    /// The message defaults (font, size, colour): the settings' default
    /// font, or the font box with nothing selected in an empty message.
    pub fn set_defaults(&mut self, font: FontId, size: SizePt, color: Option<ColorId>) {
        if (self.doc.font, self.doc.size, self.doc.color) == (font, size, color) {
            return;
        }
        let snap = self.snap_all();
        self.doc.font = font;
        self.doc.size = size;
        self.doc.color = color;
        self.doc.normalize();
        self.commit(snap, (0, 0), Target::Offset(0), StepKind::Other);
    }

    /// The formatting at the caret, or common to the selection.
    pub fn format_at(&self, a: usize, b: usize) -> Format {
        let (a, b) = (a.min(b), a.max(b));
        let mut f = Format {
            font: self.doc.font,
            size: self.doc.size,
            color: self.doc.color,
            ..Format::default()
        };
        let Some((li, off)) = self.locate(a) else {
            return f;
        };
        let line = self.line(li);
        let st = if a == b {
            self.typing_style(li, off)
        } else {
            // The style at the start of the selection.
            let p = line.and_then(|l| para_of(&self.doc, l.pos));
            let at = off.saturating_add(1);
            p.and_then(|p| p.style_near(at.min(p.len())))
                .cloned()
                .unwrap_or_default()
        };
        if a != b {
            let spans = self.para_spans(a, b);
            let all = |g: &dyn Fn(&Style) -> bool| {
                spans
                    .iter()
                    .all(|(p, x, y)| self.para(*p).is_none_or(|q| q.all(*x, *y, g)))
            };
            f.b = all(&|s| s.b);
            f.i = all(&|s| s.i);
            f.u = all(&|s| s.u);
            f.s = all(&|s| s.s);
        } else {
            f.b = st.b;
            f.i = st.i;
            f.u = st.u;
            f.s = st.s;
        }
        f.font = st.font.unwrap_or(self.doc.font);
        f.size = st.size.unwrap_or(self.doc.size);
        f.color = st.color.or(self.doc.color);
        f.link = st.link;
        if let Some(l) = line {
            f.in_table = l.pos.cell.is_some();
            f.in_signature = matches!(l.pos.flow, Flow::Sig(_));
            if let Some(p) = para_of(&self.doc, l.pos) {
                f.align = p.align;
                f.list = p.list.map(|x| x.0);
                f.quote = p.quote > 0;
            }
        }
        f
    }

    /// After a space typed at `caret`: a paragraph that starts with `- `,
    /// `* ` or `1. ` (and is not a list item yet) becomes a bulleted or
    /// numbered list item without those marks, as an undo step of its own
    /// (Ctrl+Z puts the marks back). The caret, if it did.
    pub fn autoformat(&mut self, caret: usize) -> Option<usize> {
        let (li, off) = self.locate(caret)?;
        let l = self.line(li)?;
        if l.pos.cell.is_some() || l.pos.atom.is_some() {
            return None;
        }
        let p = para_of(&self.doc, l.pos)?;
        if p.list.is_some() || p.quote > 0 {
            return None;
        }
        let text = p.plain_text();
        let kind = match text.get(..off)? {
            "- " | "* " => ListKind::Bullet,
            "1. " => ListKind::Number,
            _ => return None,
        };
        let t = l.pos.top();
        let snap = self.snap(t, t);
        let p = para_of_mut(&mut self.doc, l.pos)?;
        p.put(0, off, "", Style::default());
        p.list = Some((kind, 0));
        self.commit(
            snap,
            (caret, caret),
            Target::Para(l.pos, 0),
            StepKind::Other,
        )
    }

    /// Tab (or Shift+Tab) at the caret: the next (previous) table cell, or
    /// a list level. The new selection, or `None` to move the focus on.
    pub fn tab(&mut self, a: usize, b: usize, back: bool) -> Option<(usize, usize)> {
        let (li, _) = self.locate(b)?;
        let l = self.line(li)?;
        if let Some((r, c)) = l.pos.cell {
            let Some(Block::Table(t)) = flow_blocks(&self.doc, l.pos.flow)?.get(l.pos.item) else {
                return None;
            };
            let cols = t.col_count();
            let rows = t.row_count();
            let idx = r.saturating_mul(cols).saturating_add(c);
            let next = if back {
                idx.checked_sub(1)
            } else {
                idx.checked_add(1)
                    .filter(|n| *n < rows.saturating_mul(cols))
            };
            let Some(n) = next else {
                // Past the last cell: the paragraph after the table.
                if back {
                    return Some((a, b));
                }
                let last = self.last_line_of(l.pos.flow, l.pos.item)?;
                let after = self.line(last.saturating_add(1))?;
                return Some((after.start, after.start));
            };
            let (nr, nc) = (n.checked_div(cols)?, n.checked_rem(cols)?);
            let first = self.lines.iter().position(|x| {
                x.pos.flow == l.pos.flow && x.pos.item == l.pos.item && x.pos.cell == Some((nr, nc))
            })?;
            let last = self
                .lines
                .iter()
                .rposition(|x| {
                    x.pos.flow == l.pos.flow
                        && x.pos.item == l.pos.item
                        && x.pos.cell == Some((nr, nc))
                })
                .unwrap_or(first);
            let (s, e) = (self.line(first)?, self.line(last)?);
            return Some((s.start, e.end()));
        }
        let p = para_of(&self.doc, l.pos)?;
        if p.list.is_some() {
            self.format_paras(a, b, ParaOp::Indent(!back));
            return Some((a, b));
        }
        None
    }

    /// Insert a `rows` x `cols` table at the caret (top-level text only):
    /// it splits the paragraph there. The caret, in the first cell.
    pub fn insert_table(&mut self, at: usize, rows: usize, cols: usize) -> Option<usize> {
        let table = Table::new(rows, cols)?;
        let (li, off) = self.locate(at)?;
        let l = self.line(li)?;
        let pos = l.pos;
        if pos.flow != Flow::Top || pos.cell.is_some() {
            return None;
        }
        let snap = self.snap(pos.top(), pos.top());
        let (at_item, tail) = match pos.atom {
            Some(_) => (pos.item, None),
            None => {
                let p = para_of_mut(&mut self.doc, pos)?;
                if p.is_empty() {
                    let props = p.props();
                    self.doc.blocks.remove(pos.item);
                    (
                        pos.item,
                        Some(Para {
                            list: None,
                            ..props
                        }),
                    )
                } else {
                    let tail = p.split_tail(off);
                    (pos.item.saturating_add(1), Some(tail))
                }
            }
        };
        let mut news = vec![Block::Table(table)];
        if let Some(t) = tail {
            news.push(Block::Para(t));
        }
        let at_item = at_item.min(self.doc.blocks.len());
        self.doc.blocks.splice(at_item..at_item, news);
        let target = Target::Para(
            Pos {
                cell: Some((0, 0)),
                ..Pos::para(Flow::Top, at_item)
            },
            0,
        );
        self.commit(snap, (at, at), target, StepKind::Other)
    }

    /// A row or column operation on the table at the caret. The caret.
    pub fn table_op(&mut self, at: usize, op: TableOp) -> Option<usize> {
        let (li, _) = self.locate(at)?;
        let l = self.line(li)?;
        let (r, c) = l.pos.cell?;
        let snap = self.snap(l.pos.top(), l.pos.top());
        let blocks = flow_blocks_mut(&mut self.doc, l.pos.flow)?;
        let target = match op {
            TableOp::DeleteTable => {
                blocks.get(l.pos.item)?;
                blocks.remove(l.pos.item);
                if blocks.is_empty() || l.pos.flow == Flow::Top && l.pos.item >= blocks.len() {
                    blocks.push(Block::Para(Para::default()));
                }
                let item = l.pos.item.min(blocks.len().saturating_sub(1));
                Target::Para(Pos::para(l.pos.flow, item), 0)
            }
            _ => {
                let Some(Block::Table(t)) = blocks.get_mut(l.pos.item) else {
                    return None;
                };
                let (ok, cell) = match op {
                    TableOp::RowAbove => (t.insert_row(r), (r, c)),
                    TableOp::RowBelow => {
                        (t.insert_row(r.saturating_add(1)), (r.saturating_add(1), c))
                    }
                    TableOp::ColLeft => (t.insert_col(c), (r, c)),
                    TableOp::ColRight => {
                        (t.insert_col(c.saturating_add(1)), (r, c.saturating_add(1)))
                    }
                    TableOp::DeleteRow => {
                        (t.delete_row(r), (r.min(t.row_count().saturating_sub(1)), c))
                    }
                    TableOp::DeleteCol => {
                        (t.delete_col(c), (r, c.min(t.col_count().saturating_sub(1))))
                    }
                    TableOp::DeleteTable => (false, (r, c)),
                };
                if !ok {
                    self.rollback(snap);
                    return None;
                }
                Target::Para(
                    Pos {
                        cell: Some(cell),
                        para: 0,
                        ..l.pos
                    },
                    0,
                )
            }
        };
        self.commit(snap, (at, at), target, StepKind::Other)
    }

    /// Insert pasted formatted content at `a..b`: its paragraphs and
    /// tables (no tables inside a table: there it is pasted as text). The
    /// caret.
    pub fn paste(&mut self, a: usize, b: usize, frag: Doc) -> Option<usize> {
        let (a, b) = (a.min(b), a.max(b));
        let mut blocks: Vec<Block> = frag
            .blocks
            .into_iter()
            .filter(|b| matches!(b, Block::Para(_) | Block::Table(_)))
            .collect();
        if blocks.is_empty() {
            return None;
        }
        let caret = if a == b {
            a
        } else {
            self.replace(a, b, "", (a, b))?
        };
        let (li, off) = self.locate(caret)?;
        let pos = self.line(li)?.pos;
        let has_table = blocks.iter().any(|b| matches!(b, Block::Table(_)));
        if pos.atom.is_some() || (has_table && pos.cell.is_some()) {
            let text = super::emit::plain(&Doc {
                blocks,
                ..Doc::default()
            });
            return self.replace(caret, caret, &text, (a, b));
        }
        if pos.cell.is_some() {
            for bl in &mut blocks {
                if let Block::Para(p) = bl {
                    p.list = None;
                    p.indent = 0;
                    p.quote = 0;
                }
            }
        }
        let snap = self.snap(pos.top(), pos.top());
        let first = if matches!(blocks.first(), Some(Block::Para(_))) {
            match blocks.remove(0) {
                Block::Para(fp) => Some(fp),
                _ => None,
            }
        } else {
            None
        };
        let tail = {
            let p = para_of_mut(&mut self.doc, pos)?;
            let tail = p.split_tail(off);
            if let Some(fp) = first {
                p.join(Para {
                    runs: fp.runs,
                    ..Para::default()
                });
            }
            tail
        };
        let target = match blocks.last_mut() {
            None => {
                let p = para_of_mut(&mut self.doc, pos)?;
                let o = p.len();
                p.join(tail);
                Target::Para(pos, o)
            }
            Some(Block::Para(lp)) => {
                let o = lp.len();
                lp.join(tail);
                let last = self.insert_blocks(pos, blocks)?;
                Target::Para(last, o)
            }
            Some(_) => {
                blocks.push(Block::Para(tail));
                let last = self.insert_blocks(pos, blocks)?;
                Target::Para(last, 0)
            }
        };
        self.commit(snap, (a, b), target, StepKind::Other)
    }

    /// Insert `blocks` after the paragraph at `pos` (only paragraphs in a
    /// cell); the position of the last, a paragraph.
    fn insert_blocks(&mut self, pos: Pos, blocks: Vec<Block>) -> Option<Pos> {
        if pos.cell.is_some() {
            let paras: Vec<Para> = blocks
                .into_iter()
                .filter_map(|b| match b {
                    Block::Para(p) => Some(p),
                    _ => None,
                })
                .collect();
            return insert_after(&mut self.doc, pos, paras);
        }
        let n = blocks.len();
        let v = flow_blocks_mut(&mut self.doc, pos.flow)?;
        let at = pos.item.saturating_add(1).min(v.len());
        v.splice(at..at, blocks);
        Some(Pos::para(pos.flow, at.saturating_add(n).saturating_sub(1)))
    }

    /// The formatted content of `a..b` as a document of its own (for the
    /// clipboard's HTML): its paragraphs and tables, cut at the ends. A
    /// signature or a stored quote at either end is left out (they are
    /// copied as plain text); `None` for an empty range or one with such a
    /// block inside it.
    pub fn fragment(&self, a: usize, b: usize) -> Option<Doc> {
        let (a, b) = (a.min(b), a.max(b).min(self.text.len()));
        let plain = |t: usize| {
            matches!(
                self.doc.blocks.get(t),
                Some(Block::Para(_) | Block::Table(_))
            )
        };
        let top_at = |o: usize| -> Option<usize> { Some(self.line(self.locate(o)?.0)?.pos.top()) };
        let (mut ta, mut tb) = (top_at(a)?, top_at(b)?);
        let (mut a, mut b) = (a, b);
        // Signature and quote blocks at the ends drop out.
        while ta < tb && !plain(ta) {
            ta = ta.saturating_add(1);
            a = self.lines.iter().find(|l| l.pos.top() == ta)?.start;
        }
        while tb > ta && !plain(tb) {
            tb = tb.saturating_sub(1);
            b = self.lines.iter().rfind(|l| l.pos.top() == tb)?.end();
        }
        if a >= b || !(ta..=tb).all(plain) {
            return None;
        }
        let base = self.lines.iter().find(|l| l.pos.top() == ta)?.start;
        let mut ed = Editor::new(Doc {
            font: self.doc.font,
            size: self.doc.size,
            color: self.doc.color,
            blocks: self.doc.blocks.get(ta..=tb)?.to_vec(),
        });
        let (a, b) = (a.checked_sub(base)?, b.checked_sub(base)?);
        let end = ed.text().len();
        ed.replace(b, end, "", (b, end))?;
        ed.replace(0, a, "", (0, a))?;
        Some(ed.doc)
    }

    /// The signature block, if the message has one.
    pub fn signature(&self) -> Option<&SignatureBlock> {
        self.doc.blocks.iter().find_map(|b| match b {
            Block::Signature(s) => Some(s),
            _ => None,
        })
    }

    /// Put `sig` in place of the message's signature (`None`: take it
    /// out). A message without one gets it above the quote, or at the end,
    /// after an empty paragraph to write in. True if anything changed.
    pub fn set_signature(&mut self, sig: Option<SignatureBlock>) -> bool {
        let at = self
            .doc
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Signature(_)));
        if at.is_none() && sig.is_none() {
            return false;
        }
        let snap = self.snap_all();
        match (at, sig) {
            (Some(i), Some(s)) => {
                if let Some(b) = self.doc.blocks.get_mut(i) {
                    *b = Block::Signature(s);
                }
            }
            (Some(i), None) => {
                self.doc.blocks.remove(i);
            }
            (None, Some(s)) => {
                let q = self
                    .doc
                    .blocks
                    .iter()
                    .position(|b| matches!(b, Block::Quote(_)))
                    .unwrap_or(self.doc.blocks.len());
                // Room to write above it: one empty paragraph.
                let room = match q.checked_sub(1).and_then(|i| self.doc.blocks.get(i)) {
                    Some(Block::Para(p)) => !p.is_empty(),
                    _ => true,
                };
                let mut news = Vec::new();
                if room {
                    news.push(Block::Para(Para::default()));
                }
                news.push(Block::Signature(s));
                self.doc.blocks.splice(q..q, news);
            }
            (None, None) => {}
        }
        self.doc.normalize();
        if self.doc.blocks == snap.before {
            self.rollback(snap);
            return false;
        }
        self.commit(snap, (0, 0), Target::Offset(0), StepKind::Other);
        true
    }

    /// The stored quote: folded or shown. True if it changed.
    pub fn fold_quote(&mut self, folded: bool) -> bool {
        let Some(i) = self
            .doc
            .blocks
            .iter()
            .position(|b| matches!(b, Block::Quote(q) if q.folded != folded))
        else {
            return false;
        };
        if let Some(Block::Quote(q)) = self.doc.blocks.get_mut(i) {
            q.folded = folded;
        }
        // Not an edit: no undo step, but the view lays it out again.
        self.changes.push(Change {
            at: i,
            old_len: 1,
            new_len: 1,
            all: false,
        });
        self.rebuild();
        true
    }

    pub fn has_quote(&self) -> bool {
        self.doc.blocks.iter().any(|b| matches!(b, Block::Quote(_)))
    }

    pub fn quote_folded(&self) -> bool {
        self.doc
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Quote(q) if q.folded))
    }

    /// "Edit quoted text": the stored quote becomes ordinary quoted
    /// paragraphs (what the model can hold of it). True if it changed.
    pub fn expand_quote(&mut self) -> bool {
        let Some((i, html)) = self
            .doc
            .blocks
            .iter()
            .enumerate()
            .find_map(|(i, b)| match b {
                Block::Quote(q) => Some((i, q.html().to_string())),
                _ => None,
            })
        else {
            return false;
        };
        let mut blocks = super::import::lossy(&html)
            .map(|d| d.blocks)
            .unwrap_or_default();
        for b in &mut blocks {
            if let Block::Para(p) = b {
                p.quote = p.quote.saturating_add(1).min(MAX_QUOTE_LEVEL);
            }
        }
        if blocks.is_empty() {
            blocks.push(Block::Para(Para::default()));
        }
        let snap = self.snap(i, i);
        self.doc.blocks.splice(i..=i, blocks);
        self.commit(
            snap,
            (0, 0),
            Target::Para(Pos::para(Flow::Top, i), 0),
            StepKind::Other,
        );
        true
    }

    /// Undo the last step; the selection it restores.
    pub fn undo(&mut self) -> Option<(usize, usize)> {
        let step = self.undo.pop()?;
        let back = self.swap(step);
        let sel = back.sel_before;
        self.redo.push(back);
        Some(sel)
    }

    pub fn redo(&mut self) -> Option<(usize, usize)> {
        let step = self.redo.pop()?;
        let back = self.swap(step);
        let sel = back.sel_after;
        self.undo.push(back);
        Some(sel)
    }

    /// Put a step's blocks back; the step that reverses it.
    fn swap(&mut self, step: Step) -> Step {
        let end = step
            .at
            .saturating_add(step.after_len)
            .min(self.doc.blocks.len());
        let at = step.at.min(end);
        let restored = step.before.len();
        let now: Vec<Block> = self.doc.blocks.splice(at..end, step.before).collect();
        let defaults = (self.doc.font, self.doc.size, self.doc.color);
        (self.doc.font, self.doc.size, self.doc.color) = step.defaults;
        self.changes.push(Change {
            at,
            old_len: now.len(),
            new_len: restored,
            all: defaults != step.defaults,
        });
        self.rebuild();
        self.pending = None;
        self.last_caret = None;
        Step {
            at,
            before: now,
            after_len: restored,
            defaults,
            sel_before: step.sel_before,
            sel_after: step.sel_after,
            kind: StepKind::Other,
            burst: 1,
        }
    }
}

/// Character formatting `op` on one style, `on` for the toggles.
fn apply_char(st: &mut Style, op: &CharOp, on: bool, d: Defaults) {
    match op {
        CharOp::Bold => st.b = on,
        CharOp::Italic => st.i = on,
        CharOp::Underline => st.u = on,
        CharOp::Strike => st.s = on,
        CharOp::Font(f) => st.font = (*f != d.0).then_some(*f),
        CharOp::Size(s) => st.size = (*s != d.1).then_some(*s),
        CharOp::SizeStep(up) => {
            let s = st.size.unwrap_or(d.1).step(*up);
            st.size = (s != d.1).then_some(s);
        }
        CharOp::Color(c) => st.color = if *c == d.2 { None } else { *c },
        CharOp::Clear => {
            let link = st.link.take();
            *st = Style::default();
            st.link = link;
        }
    }
}

fn table_lines(flow: Flow, item: usize, t: &Table, push: &mut dyn FnMut(Pos, &str)) {
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            for (k, p) in cell.paras.iter().enumerate() {
                let pos = Pos {
                    cell: Some((r, c)),
                    para: k,
                    ..Pos::para(flow, item)
                };
                push(pos, &p.plain_text());
            }
        }
    }
}

/// Paragraphs past [`MAX_RUNS`] runs (formatting that would split them).
fn too_many_runs(doc: &Doc) -> bool {
    fn blocks(bs: &[Block]) -> bool {
        bs.iter().any(|b| match b {
            Block::Para(p) => p.runs.len() > MAX_RUNS,
            Block::Table(t) => t
                .rows
                .iter()
                .flatten()
                .flat_map(|c| &c.paras)
                .any(|p| p.runs.len() > MAX_RUNS),
            Block::Signature(s) => blocks(&s.blocks),
            Block::Quote(_) => false,
        })
    }
    blocks(&doc.blocks)
}

/// The shape the editor needs: a paragraph to type in at all, after every
/// table and between two objects; every cell with a paragraph.
fn fix_structure(doc: &mut Doc) {
    if doc.blocks.is_empty() {
        doc.blocks.push(Block::Para(Para::default()));
    }
    let mut i = 0usize;
    while i < doc.blocks.len() {
        let is_table = matches!(doc.blocks.get(i), Some(Block::Table(_)));
        if let Some(Block::Table(t)) = doc.blocks.get_mut(i) {
            for c in t.rows.iter_mut().flatten() {
                if c.paras.is_empty() {
                    c.paras.push(Para::default());
                }
            }
        }
        if let Some(Block::Signature(s)) = doc.blocks.get_mut(i) {
            for b in &mut s.blocks {
                if let Block::Table(t) = b {
                    for c in t.rows.iter_mut().flatten() {
                        if c.paras.is_empty() {
                            c.paras.push(Para::default());
                        }
                    }
                }
            }
        }
        let next_needs = match doc.blocks.get(i.saturating_add(1)) {
            None => is_table,
            Some(Block::Table(_)) => is_table,
            _ => false,
        };
        if next_needs {
            doc.blocks
                .insert(i.saturating_add(1), Block::Para(Para::default()));
        }
        i = i.saturating_add(1);
    }
}

impl Doc {
    /// A copy of top-level blocks `t0..=t1` (for change checks).
    fn clone_range(&self, t0: usize, t1: usize) -> Vec<Block> {
        let t1 = t1.min(self.blocks.len().saturating_sub(1));
        self.blocks
            .get(t0.min(t1)..=t1)
            .map(<[Block]>::to_vec)
            .unwrap_or_default()
    }
}
