//! The painted rich editor (REPORT.md S5, docs/WINUI3.md "RichEditBox
//! mapping"): Komorebi's own document model (`mail::rich::edit::Editor`,
//! edited through its plain-text projection) laid out by the reader's text
//! engine (`mail::rich::view::View`, kept per block) and painted in pieces
//! (tiny-skia pixmaps, handed to GPUI as `RenderImage`s and released with
//! `drop_image` when they change), with the selection and the caret as
//! quads.
//!
//! The view implements `EntityInputHandler`, so typing, IME composition
//! and the candidate window go through the handler Windows calls (IMM32 in
//! gpui-pre-windows). The composition is kept beside the model, never in
//! it, and drawn over the text at the caret in a paper box, underlined;
//! its commit is one `Editor::replace`, so one undo step. Offsets: the
//! model's are UTF-8 bytes of `Editor::text()`; the input handler's are
//! UTF-16 of the text with the composition spliced in at the caret.
//!
//! Keys are actions in the `RichBody` key context; formatting goes through
//! `komorebi::compose::rich` (the same commands as the Slint app's
//! toolbar). Only the blocks an edit changed are laid out again, and only
//! the pieces in view that changed are painted again.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, ClipboardItem, Context, Corners, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Point, Render, RenderImage, Role,
    ScrollHandle, ShapedLine, SharedString, Style, Styled, TextRun, UTF16Selection, UnderlineStyle,
    Window, div, fill, point, prelude::*, px, relative, size,
};
use gpui_kit::TestSupportExt as _;
use komorebi::compose::rich::{self as commands, Action};
use komorebi::db::Db;
use komorebi::mail::render::{self as rnd, Engine, Rect};
use komorebi::mail::rich::edit::{Editor, Format};
use komorebi::mail::rich::view::View;
use komorebi::mail::rich::{self as model, Align};
use unicode_segmentation::GraphemeCursor;

use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo};

/// The key context of the formatted body.
pub const CONTEXT: &str = "RichBody";

/// The stored quote's layout thread's stack (the reader worker's).
const QUOTE_STACK: usize = 8 * 1024 * 1024;

gpui_kit::actions!(
    rich_body,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        WordLeft,
        WordRight,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectUp,
        SelectDown,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        PageUp,
        PageDown,
        Newline,
        Tab,
        BackTab,
        Undo,
        Redo,
        SelectAll,
        Copy,
        Cut,
        Paste,
        PastePlain,
        Bold,
        Italic,
        Underline,
        Strike,
        Bullets,
        Numbers,
        Indent,
        Outdent,
        AlignLeft,
        AlignCenter,
        AlignRight,
        Justify,
        SizeUp,
        SizeDown,
        ClearFormat,
        QuoteBlock
    ]
);

/// The body's keys (the Slint editor's `format-key` and `nav-key`).
pub fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("shift-backspace", Backspace, ctx),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("ctrl-left", WordLeft, ctx),
        KeyBinding::new("ctrl-right", WordRight, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, ctx),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new("ctrl-home", DocStart, ctx),
        KeyBinding::new("ctrl-end", DocEnd, ctx),
        KeyBinding::new("ctrl-shift-home", SelectDocStart, ctx),
        KeyBinding::new("ctrl-shift-end", SelectDocEnd, ctx),
        KeyBinding::new("pageup", PageUp, ctx),
        KeyBinding::new("pagedown", PageDown, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", BackTab, ctx),
        KeyBinding::new("ctrl-z", Undo, ctx),
        KeyBinding::new("ctrl-y", Redo, ctx),
        KeyBinding::new("ctrl-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-a", SelectAll, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("ctrl-insert", Copy, ctx),
        KeyBinding::new("ctrl-x", Cut, ctx),
        KeyBinding::new("shift-delete", Cut, ctx),
        KeyBinding::new("ctrl-v", Paste, ctx),
        KeyBinding::new("shift-insert", Paste, ctx),
        KeyBinding::new("ctrl-shift-v", PastePlain, ctx),
        KeyBinding::new("ctrl-b", Bold, ctx),
        KeyBinding::new("ctrl-i", Italic, ctx),
        KeyBinding::new("ctrl-u", Underline, ctx),
        KeyBinding::new("ctrl-shift-x", Strike, ctx),
        KeyBinding::new("ctrl-shift-8", Bullets, ctx),
        KeyBinding::new("ctrl-shift-7", Numbers, ctx),
        KeyBinding::new("ctrl-m", Indent, ctx),
        KeyBinding::new("ctrl-shift-m", Outdent, ctx),
        KeyBinding::new("ctrl-l", AlignLeft, ctx),
        KeyBinding::new("ctrl-e", AlignCenter, ctx),
        KeyBinding::new("ctrl-r", AlignRight, ctx),
        KeyBinding::new("ctrl-j", Justify, ctx),
        KeyBinding::new("ctrl-]", SizeUp, ctx),
        KeyBinding::new("ctrl-[", SizeDown, ctx),
        KeyBinding::new("ctrl-space", ClearFormat, ctx),
        KeyBinding::new("ctrl-shift-q", QuoteBlock, ctx),
    ]);
}

/// What the body tells its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorEvent {
    /// The document changed (save it soon).
    Changed,
    /// The caret or the selection moved (the format bar follows).
    Moved,
    /// Ctrl+V with files on the clipboard: attach them.
    PasteFiles(Vec<std::path::PathBuf>),
}

/// One painted piece placed on the page (logical px).
#[derive(Clone)]
struct Placed {
    y: f32,
    w: f32,
    h: f32,
    image: Arc<RenderImage>,
}

/// What the element draws, rebuilt by `render`.
#[derive(Clone, Default)]
struct Picture {
    height: f32,
    pieces: Vec<Placed>,
    selection: Vec<Rect>,
    caret: Option<Rect>,
    /// The composition at the caret.
    ime: Option<(Rect, SharedString)>,
    /// "Write your message" on an empty message.
    placeholder: Option<(Rect, SharedString)>,
    /// What screen readers get: the text, its lines near the view, the
    /// selection (`a11y.rs`).
    a11y: Rc<super::a11y::BodyText>,
}

/// The formatted body: an [`Editor`] while a formatted draft shows (none
/// for a plain one), its picture, its caret and its composition.
pub struct RichEditor {
    focus: FocusHandle,
    db: Option<Rc<Db>>,
    ed: Option<Editor>,
    view: Option<View>,
    engine: Option<Engine>,
    anchor: usize,
    cursor: usize,
    /// The x a run of Up and Down keys keeps to.
    goal_x: Option<f32>,
    drag_from: usize,
    /// The composition: inserted at `.0` (model bytes) for display only,
    /// and the IME's caret inside it (bytes of the composition).
    marked: Option<(usize, String)>,
    marked_caret: usize,
    /// Painted pieces by key, and the ones to release from the atlas.
    images: HashMap<u64, Arc<RenderImage>>,
    stale: Vec<Arc<RenderImage>>,
    /// Stored quotes being laid out, by key; logo marks handed to the view.
    quotes: HashSet<u64>,
    logos: HashMap<u64, Arc<[u8]>>,
    given: HashSet<u64>,
    /// The page's width (logical px) and its scroll container.
    width: f32,
    scroll: ScrollHandle,
    /// The element's bounds at the last paint (hit testing, the IME).
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Scroll the caret into view at the next render.
    reveal: bool,
    picture: Rc<Picture>,
    name: SharedString,
    placeholder: SharedString,
}

impl EventEmitter<EditorEvent> for RichEditor {}

impl RichEditor {
    /// `db` reads signature logos (none in tests that need none).
    pub fn new(db: Option<Rc<Db>>, cx: &mut Context<Self>) -> Self {
        RichEditor {
            focus: cx.focus_handle(),
            db,
            ed: None,
            view: None,
            engine: None,
            anchor: 0,
            cursor: 0,
            goal_x: None,
            drag_from: 0,
            marked: None,
            marked_caret: 0,
            images: HashMap::new(),
            stale: Vec::new(),
            quotes: HashSet::new(),
            logos: HashMap::new(),
            given: HashSet::new(),
            width: 0.,
            scroll: ScrollHandle::new(),
            bounds: Rc::new(Cell::new(None)),
            reveal: false,
            picture: Rc::new(Picture::default()),
            name: SharedString::default(),
            placeholder: SharedString::default(),
        }
    }

    /// The accessible name and the empty message's placeholder, in the
    /// language shown (the owner passes them at each render).
    pub fn set_words(&mut self, name: SharedString, placeholder: SharedString) {
        self.name = name;
        self.placeholder = placeholder;
    }

    /// Another document (another draft shows, or none): nothing of the old
    /// one's picture is kept. The caret goes to `caret`.
    pub fn set_doc(&mut self, ed: Option<Editor>, caret: usize, cx: &mut Context<Self>) {
        self.ed = ed;
        self.view = None;
        self.marked = None;
        self.goal_x = None;
        self.quotes.clear();
        self.given.clear();
        self.stale.extend(self.images.drain().map(|(_, i)| i));
        let len = self.ed.as_ref().map_or(0, |e| e.text().len());
        self.anchor = caret.min(len);
        self.cursor = self.anchor;
        self.scroll.set_offset(point(Pixels::ZERO, Pixels::ZERO));
        self.reveal = true;
        cx.notify();
    }

    /// The document, taken out (the draft holds it while it is saved or
    /// sent); [`RichEditor::put`] gives it back with its picture kept.
    pub fn take(&mut self) -> Option<Editor> {
        self.ed.take()
    }

    pub fn put(&mut self, ed: Option<Editor>) {
        self.ed = ed;
    }

    pub fn editor(&self) -> Option<&Editor> {
        self.ed.as_ref()
    }

    pub fn is_formatted(&self) -> bool {
        self.ed.is_some()
    }

    /// The selection (anchor, cursor) in model bytes.
    pub fn selection(&self) -> (usize, usize) {
        (self.anchor, self.cursor)
    }

    pub fn set_selection(&mut self, anchor: usize, cursor: usize, cx: &mut Context<Self>) {
        let len = self.text_len();
        self.anchor = anchor.min(len);
        self.cursor = cursor.min(len);
        self.goal_x = None;
        self.reveal = true;
        cx.emit(EditorEvent::Moved);
        cx.notify();
    }

    /// The formatting at the caret or common to the selection (the bar).
    pub fn format(&self) -> Option<Format> {
        Some(self.ed.as_ref()?.format_at(self.anchor, self.cursor))
    }

    /// The projection's text (tests, and the screen reader's value).
    pub fn text(&self) -> &str {
        self.ed.as_ref().map_or("", |e| e.text())
    }

    /// The page's width once laid out at the body's width (logical px).
    pub fn page_width(&self) -> Option<f32> {
        (self.width > 1.).then_some(self.width)
    }

    /// The page's height, laid out (logical px).
    pub fn page_height(&self) -> f32 {
        self.view.as_ref().map_or(0., View::height)
    }

    fn text_len(&self) -> usize {
        self.text().len()
    }

    /// A toolbar button or a formatting key on the selection.
    pub fn act(&mut self, action: &Action, cx: &mut Context<Self>) {
        let sel = (self.anchor, self.cursor);
        let Some(ed) = self.ed.as_mut() else {
            return;
        };
        let applied = commands::apply(ed, sel, action);
        if let Some((a, c)) = applied.sel {
            self.anchor = a;
            self.cursor = c;
            self.reveal = true;
        }
        if applied.changed {
            cx.emit(EditorEvent::Changed);
        }
        cx.emit(EditorEvent::Moved);
        cx.notify();
    }

    /// The link field's address on the selection (or at the caret). False:
    /// not a web or e-mail address.
    pub fn link(&mut self, url: &str, cx: &mut Context<Self>) -> bool {
        let sel = (self.anchor, self.cursor);
        let Some(ed) = self.ed.as_mut() else {
            return false;
        };
        let Some((a, c)) = commands::link(ed, sel, url) else {
            return false;
        };
        self.anchor = a;
        self.cursor = c;
        cx.emit(EditorEvent::Changed);
        cx.notify();
        true
    }

    /// A `rows` x `cols` table at the caret.
    pub fn insert_table(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        let at = self.cursor;
        let Some(ed) = self.ed.as_mut() else {
            return;
        };
        if let Some(caret) = ed.insert_table(at, rows, cols) {
            self.anchor = caret;
            self.cursor = caret;
            self.reveal = true;
            cx.emit(EditorEvent::Changed);
            cx.notify();
        }
    }

    /// The message's signature replaced or its quote folded from outside
    /// (the Signature menu, another From account): the document changed.
    pub fn changed_outside(&mut self, cx: &mut Context<Self>) {
        let len = self.text_len();
        self.anchor = self.anchor.min(len);
        self.cursor = self.cursor.min(len);
        cx.notify();
    }

    // ---- model edits ----------------------------------------------------

    /// One model edit of `a..b`; the caret goes where the model says. False
    /// if the model refused it (nothing changed).
    fn edit(&mut self, a: usize, b: usize, ins: &str, cx: &mut Context<Self>) -> bool {
        let sel = (self.anchor, self.cursor);
        let Some(ed) = self.ed.as_mut() else {
            return false;
        };
        let Some(mut caret) = ed.replace(a, b, ins, sel) else {
            return false;
        };
        // A space typed after "- ", "* " or "1." at a line start.
        if ins == " "
            && let Some(c) = ed.autoformat(caret)
        {
            caret = c;
        }
        self.anchor = caret;
        self.cursor = caret;
        self.goal_x = None;
        self.reveal = true;
        cx.emit(EditorEvent::Changed);
        cx.emit(EditorEvent::Moved);
        cx.notify();
        true
    }

    fn ordered(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    fn prev_boundary(&self, o: usize) -> usize {
        let t = self.text();
        let o = o.min(t.len());
        GraphemeCursor::new(o, t.len(), true)
            .prev_boundary(t, 0)
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    fn next_boundary(&self, o: usize) -> usize {
        let t = self.text();
        let o = o.min(t.len());
        GraphemeCursor::new(o, t.len(), true)
            .next_boundary(t, 0)
            .ok()
            .flatten()
            .unwrap_or(t.len())
    }

    /// The start of the word before `o` (Ctrl+Left), or the end of the one
    /// after it (Ctrl+Right): spaces skipped, then the word.
    fn word_edge(&self, o: usize, right: bool) -> usize {
        let t = self.text();
        let o = o.min(t.len());
        if right {
            let rest = &t[o..];
            let start = rest.len() - rest.trim_start().len();
            let word = rest[start..]
                .find(char::is_whitespace)
                .unwrap_or(rest.len() - start);
            o + start + word
        } else {
            let head = t[..o].trim_end();
            head.rfind(char::is_whitespace).map_or(0, |i| {
                i + head[i..].chars().next().map_or(1, char::len_utf8)
            })
        }
    }

    /// Move the caret to `to`, extending the selection with `extend`.
    fn move_to(&mut self, to: usize, extend: bool, cx: &mut Context<Self>) {
        self.cursor = to.min(self.text_len());
        if !extend {
            self.anchor = self.cursor;
        }
        self.reveal = true;
        cx.emit(EditorEvent::Moved);
        cx.notify();
    }

    fn horizontal(&mut self, right: bool, word: bool, extend: bool, cx: &mut Context<Self>) {
        self.goal_x = None;
        let (lo, hi) = self.ordered();
        let to = if !extend && !word && lo != hi {
            if right { hi } else { lo }
        } else if word {
            self.word_edge(self.cursor, right)
        } else if right {
            self.next_boundary(self.cursor)
        } else {
            self.prev_boundary(self.cursor)
        };
        self.move_to(to, extend, cx);
    }

    /// Up and Down (a line), Page Up and Page Down (a view).
    fn vertical(&mut self, down: bool, page: bool, extend: bool, cx: &mut Context<Self>) {
        let (Some(ed), Some(view)) = (self.ed.as_ref(), self.view.as_ref()) else {
            return;
        };
        let goal = self.goal_x.unwrap_or_else(|| view.caret(ed, self.cursor).x);
        let step = if page {
            (self.viewport().1 - geo::CARET_ROOM * 3.).max(geo::CARET_ROOM * 3.)
        } else {
            0.
        };
        let to = view.vertical(ed, self.cursor, goal, down, step);
        self.move_to(to, extend, cx);
        self.goal_x = Some(goal);
    }

    fn line_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let (Some(ed), Some(view)) = (self.ed.as_ref(), self.view.as_ref()) else {
            return;
        };
        let to = view.line_edge(ed, self.cursor, end);
        self.goal_x = None;
        self.move_to(to, extend, cx);
    }

    fn doc_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let to = if end { self.text_len() } else { 0 };
        self.goal_x = None;
        self.move_to(to, extend, cx);
    }

    fn delete_back(&mut self, word: bool, cx: &mut Context<Self>) {
        let (lo, hi) = self.ordered();
        let (a, b) = if lo != hi {
            (lo, hi)
        } else if word {
            (self.word_edge(self.cursor, false), self.cursor)
        } else {
            (self.prev_boundary(self.cursor), self.cursor)
        };
        if a != b {
            self.edit(a, b, "", cx);
        }
    }

    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        let (lo, hi) = self.ordered();
        let (a, b) = if lo != hi {
            (lo, hi)
        } else {
            (self.cursor, self.next_boundary(self.cursor))
        };
        if a != b {
            self.edit(a, b, "", cx);
        }
    }

    fn history(&mut self, redo: bool, cx: &mut Context<Self>) {
        let Some(ed) = self.ed.as_mut() else {
            return;
        };
        let sel = if redo { ed.redo() } else { ed.undo() };
        if let Some((a, c)) = sel {
            self.anchor = a;
            self.cursor = c;
            self.reveal = true;
            cx.emit(EditorEvent::Changed);
            cx.emit(EditorEvent::Moved);
            cx.notify();
        }
    }

    /// Tab and Shift+Tab move between table cells; elsewhere the focus
    /// moves on.
    fn tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (a, c) = (self.anchor, self.cursor);
        let moved = self.ed.as_mut().and_then(|ed| ed.tab(a, c, back));
        match moved {
            Some((na, nc)) => {
                self.anchor = na;
                self.cursor = nc;
                self.goal_x = None;
                self.reveal = true;
                cx.emit(EditorEvent::Changed);
                cx.emit(EditorEvent::Moved);
                cx.notify();
            }
            None if back => window.focus_prev(cx),
            None => window.focus_next(cx),
        }
    }

    /// Ctrl+C and Ctrl+X: the selection as plain text and HTML (the
    /// model's own, as sent); Cut then deletes it.
    fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let (a, b) = self.ordered();
        if a == b {
            return;
        }
        let Some(ed) = self.ed.as_ref() else {
            return;
        };
        let rich = ed.fragment(a, b).is_some_and(|frag| {
            let html = model::emit::html(&frag, model::Mode::Send);
            let text = model::emit::plain(&frag);
            komorebi::files::set_clipboard_rich(text.trim_end_matches('\n'), &html)
        });
        if !rich {
            let plain = ed.text().get(a..b).unwrap_or_default().to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(plain));
        }
        if cut {
            self.edit(a, b, "", cx);
        }
    }

    /// Ctrl+V: files are attached by the owner; formatted text is pasted
    /// as the model holds it (sanitized, then the lossy import); plain text
    /// as typed.
    fn paste(&mut self, plain: bool, cx: &mut Context<Self>) {
        if !plain {
            let files = komorebi::files::clipboard_files();
            if !files.is_empty() {
                cx.emit(EditorEvent::PasteFiles(files));
                return;
            }
            let frag = komorebi::files::clipboard_html()
                .and_then(|h| komorebi::mail::compose_html::sanitize(&h))
                .and_then(|h| model::import::lossy(&h));
            let (a, c) = (self.anchor, self.cursor);
            if let Some(frag) = frag
                && let Some(caret) = self.ed.as_mut().and_then(|ed| ed.paste(a, c, frag))
            {
                self.anchor = caret;
                self.cursor = caret;
                self.reveal = true;
                cx.emit(EditorEvent::Changed);
                cx.emit(EditorEvent::Moved);
                cx.notify();
                return;
            }
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            let (a, b) = self.ordered();
            self.edit(a, b, &text.replace("\r\n", "\n"), cx);
        }
    }

    // ---- the picture ----------------------------------------------------

    /// The part of the page in view: its top and height (logical px).
    fn viewport(&self) -> (f32, f32) {
        let top = (-self.scroll.offset().y.as_f32()).max(0.);
        let h = self.scroll.bounds().size.height.as_f32();
        (top, if h > 1. { h } else { geo::BODY_MIN_H })
    }

    /// Lay out what changed, paint the pieces around the part in view not
    /// painted yet, and rebuild what the element draws.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tokens = cx.tokens().clone();
        let Some(ed) = self.ed.as_mut() else {
            self.picture = Rc::new(Picture::default());
            return;
        };
        let scale = window.scale_factor();
        let zoom = tokens.text.body.size.as_f32() / geo::ZOOM_BASE;
        let theme = geo::editor_theme(&tokens);
        let width = if self.width > 1. {
            self.width
        } else {
            geo::BODY_MAX_W
        };
        let view = self
            .view
            .get_or_insert_with(|| View::new(width, scale, zoom, theme));
        if view.set_geometry(width, scale, zoom, theme) {
            self.stale.extend(self.images.drain().map(|(_, i)| i));
            self.quotes.clear();
        }
        let engine = self.engine.get_or_insert_with(Engine::new);
        // Signature logos, read from the cache once.
        if let Some(db) = &self.db {
            for (hash, bytes) in commands::logos(db, ed.doc(), &self.logos) {
                self.logos.insert(hash, bytes);
            }
        }
        for b in &ed.doc().blocks {
            if let model::Block::Signature(s) = b
                && let Some(l) = s.logo
                && !self.given.contains(&l.hash)
                && let Some(bytes) = self.logos.get(&l.hash)
            {
                view.set_logo(l.hash, bytes.clone());
                self.given.insert(l.hash);
            }
        }
        view.sync(ed, engine);
        view.prepare_logos(ed);
        let jobs: Vec<_> = view
            .quote_jobs(ed)
            .into_iter()
            .filter(|j| self.quotes.insert(j.0))
            .collect();
        let len = ed.text().len();
        self.anchor = self.anchor.min(len);
        self.cursor = self.cursor.min(len);
        let caret = view.caret(ed, self.cursor);
        // Keep the caret in view after an edit or a move.
        let (top, h) = {
            let top = (-self.scroll.offset().y.as_f32()).max(0.);
            let vh = self.scroll.bounds().size.height.as_f32();
            (top, if vh > 1. { vh } else { geo::BODY_MIN_H })
        };
        let mut top = top;
        if std::mem::take(&mut self.reveal) && self.scroll.bounds().size.height.as_f32() > 1. {
            let want = if caret.y < top {
                Some((caret.y - geo::CARET_ROOM).max(0.))
            } else if caret.y + caret.h > top + h {
                Some((caret.y + caret.h - h + geo::CARET_ROOM).max(0.))
            } else {
                None
            };
            if let Some(t) = want {
                top = t;
                self.scroll.set_offset(point(Pixels::ZERO, px(-t)));
            }
        }
        let (lo, hi) = (
            top - h * geo::MARGIN_VIEWS,
            top + h * (1. + geo::MARGIN_VIEWS),
        );
        let (a, c) = (self.anchor.min(self.cursor), self.anchor.max(self.cursor));
        let selection = if a == c {
            Vec::new()
        } else {
            view.selection(ed, a, c)
                .into_iter()
                .filter(|r| r.y + r.h >= lo && r.y <= hi)
                .collect()
        };
        let mut images = HashMap::new();
        let mut pieces = Vec::new();
        let vscale = view.scale();
        for p in view.pieces(lo, hi) {
            let image = match self.images.remove(&p.key) {
                Some(i) => i,
                None => match view
                    .paint_piece(ed, &p, engine)
                    .map(commands::straight_bgra)
                    .and_then(to_image)
                {
                    Some(i) => i,
                    None => continue,
                },
            };
            pieces.push(Placed {
                y: p.y_px as f32 / vscale,
                w: view.width_px() as f32 / vscale,
                h: p.h_px as f32 / vscale,
                image: image.clone(),
            });
            images.insert(p.key, image);
        }
        // What is no longer shown leaves the atlas after this frame.
        self.stale.extend(self.images.drain().map(|(_, i)| i));
        self.images = images;
        let ime = self
            .marked
            .as_ref()
            .map(|(_, m)| (caret, SharedString::from(m.clone())));
        let placeholder = commands::is_blank(ed)
            .then(|| (caret, self.placeholder.clone()))
            .filter(|_| self.marked.is_none());
        let a11y = Rc::new(super::a11y::body_text(
            ed,
            view,
            lo,
            hi,
            self.anchor,
            self.cursor,
        ));
        self.picture = Rc::new(Picture {
            height: view.height(),
            pieces,
            selection,
            caret: Some(caret),
            ime,
            placeholder,
            a11y,
        });
        let stale = std::mem::take(&mut self.stale);
        if !stale.is_empty() {
            cx.defer(move |cx| {
                for image in stale {
                    cx.drop_image(image, None);
                }
            });
        }
        for (key, html, width, dark) in jobs {
            self.lay_out_quote(key, html, width, dark, scale, cx);
        }
    }

    /// A stored quote (full HTML, recursive layout) on a thread of its own;
    /// its place keeps the waiting height until it arrives.
    fn lay_out_quote(
        &mut self,
        key: u64,
        html: String,
        width: f32,
        dark: bool,
        scale: f32,
        cx: &mut Context<Self>,
    ) {
        let (tx, rx) = futures::channel::oneshot::channel();
        let run = move || {
            let mut engine = Engine::new();
            let input = rnd::RenderInput {
                html: Some(html),
                ..Default::default()
            };
            let opts = rnd::Opts {
                width_px: width,
                scale,
                dark,
                dim: dark,
                ..Default::default()
            };
            let doc = rnd::layout(
                &input,
                &opts,
                &mut engine,
                &rnd::Deadline::after(rnd::LAYOUT_BUDGET),
            )
            .ok();
            let _ = tx.send(doc);
        };
        let spawned = std::thread::Builder::new()
            .name("quote".into())
            .stack_size(QUOTE_STACK)
            .spawn(run);
        if spawned.is_err() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let Ok(Some(doc)) = rx.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let shown = this
                    .view
                    .as_mut()
                    .is_some_and(|v| v.set_quote(key, Arc::new(doc)));
                this.quotes.remove(&key);
                if shown {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A press, Shift+press, drag or double click at a page point.
    fn point_at(&self, position: Point<Pixels>) -> Option<usize> {
        let b = self.bounds.get()?;
        let (Some(ed), Some(view)) = (self.ed.as_ref(), self.view.as_ref()) else {
            return None;
        };
        let x = (position.x - b.left()).as_f32();
        let y = (position.y - b.top()).as_f32();
        Some(view.hit(ed, x, y))
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(at) = self.point_at(e.position) else {
            return;
        };
        self.goal_x = None;
        match (e.click_count, e.modifiers.shift) {
            (2, _) => {
                if let (Some(ed), Some(view)) = (self.ed.as_ref(), self.view.as_ref()) {
                    let (a, b) = view.word(ed, at);
                    self.anchor = a;
                    self.cursor = b;
                }
            }
            (_, true) => self.cursor = at,
            _ => {
                self.anchor = at;
                self.cursor = at;
                self.drag_from = at;
            }
        }
        cx.emit(EditorEvent::Moved);
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(at) = self.point_at(e.position) else {
            return;
        };
        if (self.anchor, self.cursor) != (self.drag_from, at) {
            self.anchor = self.drag_from;
            self.cursor = at;
            self.reveal = true;
            cx.emit(EditorEvent::Moved);
            cx.notify();
        }
    }

    // ---- offsets for the input handler ----------------------------------

    /// The text on screen: the projection with the composition spliced in.
    fn display(&self) -> std::borrow::Cow<'_, str> {
        let t = self.text();
        match &self.marked {
            Some((at, m)) => {
                let at = (*at).min(t.len());
                std::borrow::Cow::Owned(format!("{}{}{}", &t[..at], m, &t[at..]))
            }
            None => std::borrow::Cow::Borrowed(t),
        }
    }

    /// A display byte offset to a model byte offset.
    fn to_model(&self, d: usize) -> usize {
        match &self.marked {
            Some((at, m)) if d >= *at + m.len() => d - m.len(),
            Some((at, _)) if d >= *at => *at,
            _ => d,
        }
    }

    fn to_display(&self, m: usize) -> usize {
        match &self.marked {
            Some((at, s)) if m > *at => m + s.len(),
            _ => m,
        }
    }

    fn utf16_to_display(&self, u: usize) -> usize {
        utf16_to_byte(&self.display(), u)
    }

    fn display_to_utf16(&self, b: usize) -> usize {
        let d = self.display();
        d.get(..b.min(d.len()))
            .map_or(0, |s| s.encode_utf16().count())
    }
}

/// A byte offset of `s` from a UTF-16 one (the end when past it).
fn utf16_to_byte(s: &str, u: usize) -> usize {
    let (mut b, mut n) = (0, 0);
    for ch in s.chars() {
        if n >= u {
            break;
        }
        n += ch.len_utf16();
        b += ch.len_utf8();
    }
    b
}

/// A painted piece for GPUI: the straight BGRA a `RenderImage` holds
/// (`komorebi::compose::rich::straight_bgra`, as GPUI's own SVG raster
/// converts tiny-skia's premultiplied RGBA).
fn to_image((w, h, data): (u32, u32, Vec<u8>)) -> Option<Arc<RenderImage>> {
    let buf = image::RgbaImage::from_raw(w, h, data)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

impl Focusable for RichEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for RichEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let a = self.utf16_to_display(range_utf16.start);
        let b = self.utf16_to_display(range_utf16.end);
        adjusted.replace(self.display_to_utf16(a)..self.display_to_utf16(b));
        self.display().get(a..b).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = match &self.marked {
            Some((at, _)) => {
                let c = self.display_to_utf16(at + self.marked_caret);
                c..c
            }
            None => {
                let (lo, hi) = self.ordered();
                self.display_to_utf16(lo)..self.display_to_utf16(hi)
            }
        };
        Some(UTF16Selection {
            range,
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|(at, m)| self.display_to_utf16(*at)..self.display_to_utf16(at + m.len()))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // The composition is dropped as it is (Windows commits through
        // replace_text_in_range first).
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (a, b) = match (&range_utf16, &self.marked) {
            (Some(r), _) => {
                let a = self.to_model(self.utf16_to_display(r.start));
                let b = self.to_model(self.utf16_to_display(r.end));
                (a, b)
            }
            (None, Some((at, _))) => (*at, *at),
            (None, None) => self.ordered(),
        };
        self.marked = None;
        // Line breaks typed or pasted as text become paragraphs.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.edit(a, b, &text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Where the composition sits in the model: an existing one keeps
        // its place; a new one replaces the selection (deleted from the
        // model).
        let at = match (&self.marked, &range_utf16) {
            (Some((at, _)), _) => *at,
            (None, Some(r)) => {
                let a = self.to_model(self.utf16_to_display(r.start));
                let b = self.to_model(self.utf16_to_display(r.end));
                if a != b {
                    self.edit(a, b, "", cx);
                }
                a.min(b)
            }
            (None, None) => {
                let (a, b) = self.ordered();
                if a != b {
                    self.edit(a, b, "", cx);
                }
                a
            }
        };
        if new_text.is_empty() {
            self.marked = None;
        } else {
            self.marked = Some((at, new_text.to_string()));
            // The IME's caret, UTF-16 within the composition.
            self.marked_caret =
                new_selected_range.map_or(new_text.len(), |r| utf16_to_byte(new_text, r.end));
        }
        self.anchor = at;
        self.cursor = at;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The candidate window goes under the composition's start.
        let m = self.to_model(self.utf16_to_display(range_utf16.start));
        let (ed, view) = (self.ed.as_ref()?, self.view.as_ref()?);
        let r = view.caret(ed, m);
        Some(Bounds::new(
            point(element.left() + px(r.x), element.top() + px(r.y)),
            size(px(geo::CARET_W), px(r.h)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let o = self.point_at(p)?;
        Some(self.display_to_utf16(self.to_display(o)))
    }
}

macro_rules! on {
    ($el:expr, $cx:expr, $($action:ty => |$this:ident, $window:ident, $c:ident| $body:expr),* $(,)?) => {
        $el$(.on_action($cx.listener(|$this: &mut RichEditor, _: &$action, $window: &mut Window, $c: &mut Context<RichEditor>| {
            let _ = &$window;
            $body
        })))*
    };
}

impl Render for RichEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(window, cx);
        let t = cx.tokens().clone();
        let (text, page, scale) = (
            self.picture.a11y.clone(),
            self.bounds.clone(),
            window.scale_factor(),
        );
        let el = div()
            .id("rich-body")
            .role(Role::MultilineTextInput)
            .aria_label(self.name.clone())
            // The text, a node per line and the caret (`a11y.rs`).
            .a11y_synthetic_children(move |builder| {
                if let Some(page) = page.get() {
                    super::a11y::push(builder, &text, page, scale);
                }
            })
            .key_context(CONTEXT)
            .test_support()
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .text_color(t.color.ink)
            .typo(&t.text.message)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move));
        let el = on!(el, cx,
            Backspace => |this, _w, cx| this.delete_back(false, cx),
            DeleteWordLeft => |this, _w, cx| this.delete_back(true, cx),
            Delete => |this, _w, cx| this.delete_forward(cx),
            Left => |this, _w, cx| this.horizontal(false, false, false, cx),
            Right => |this, _w, cx| this.horizontal(true, false, false, cx),
            WordLeft => |this, _w, cx| this.horizontal(false, true, false, cx),
            WordRight => |this, _w, cx| this.horizontal(true, true, false, cx),
            SelectLeft => |this, _w, cx| this.horizontal(false, false, true, cx),
            SelectRight => |this, _w, cx| this.horizontal(true, false, true, cx),
            SelectWordLeft => |this, _w, cx| this.horizontal(false, true, true, cx),
            SelectWordRight => |this, _w, cx| this.horizontal(true, true, true, cx),
            Up => |this, _w, cx| this.vertical(false, false, false, cx),
            Down => |this, _w, cx| this.vertical(true, false, false, cx),
            SelectUp => |this, _w, cx| this.vertical(false, false, true, cx),
            SelectDown => |this, _w, cx| this.vertical(true, false, true, cx),
            PageUp => |this, _w, cx| this.vertical(false, true, false, cx),
            PageDown => |this, _w, cx| this.vertical(true, true, false, cx),
            Home => |this, _w, cx| this.line_edge(false, false, cx),
            End => |this, _w, cx| this.line_edge(true, false, cx),
            SelectHome => |this, _w, cx| this.line_edge(false, true, cx),
            SelectEnd => |this, _w, cx| this.line_edge(true, true, cx),
            DocStart => |this, _w, cx| this.doc_edge(false, false, cx),
            DocEnd => |this, _w, cx| this.doc_edge(true, false, cx),
            SelectDocStart => |this, _w, cx| this.doc_edge(false, true, cx),
            SelectDocEnd => |this, _w, cx| this.doc_edge(true, true, cx),
            Newline => |this, _w, cx| {
                let (a, b) = this.ordered();
                this.edit(a, b, "\n", cx);
            },
            Tab => |this, w, cx| this.tab(false, w, cx),
            BackTab => |this, w, cx| this.tab(true, w, cx),
            Undo => |this, _w, cx| this.history(false, cx),
            Redo => |this, _w, cx| this.history(true, cx),
            SelectAll => |this, _w, cx| {
                let len = this.text_len();
                this.anchor = 0;
                this.cursor = len;
                cx.emit(EditorEvent::Moved);
                cx.notify();
            },
            Copy => |this, _w, cx| this.copy(false, cx),
            Cut => |this, _w, cx| this.copy(true, cx),
            Paste => |this, _w, cx| this.paste(false, cx),
            PastePlain => |this, _w, cx| this.paste(true, cx),
            Bold => |this, _w, cx| this.act(&Action::Bold, cx),
            Italic => |this, _w, cx| this.act(&Action::Italic, cx),
            Underline => |this, _w, cx| this.act(&Action::Underline, cx),
            Strike => |this, _w, cx| this.act(&Action::Strike, cx),
            Bullets => |this, _w, cx| this.act(&Action::Bullets, cx),
            Numbers => |this, _w, cx| this.act(&Action::Numbers, cx),
            Indent => |this, _w, cx| this.act(&Action::Indent(true), cx),
            Outdent => |this, _w, cx| this.act(&Action::Indent(false), cx),
            AlignLeft => |this, _w, cx| this.act(&Action::Align(Align::Left), cx),
            AlignCenter => |this, _w, cx| this.act(&Action::Align(Align::Center), cx),
            AlignRight => |this, _w, cx| this.act(&Action::Align(Align::Right), cx),
            Justify => |this, _w, cx| this.act(&Action::Align(Align::Justify), cx),
            SizeUp => |this, _w, cx| this.act(&Action::SizeStep(true), cx),
            SizeDown => |this, _w, cx| this.act(&Action::SizeStep(false), cx),
            ClearFormat => |this, _w, cx| this.act(&Action::Clear, cx),
            QuoteBlock => |this, _w, cx| this.act(&Action::Quote, cx),
        );
        el.child(BodyElement {
            view: cx.entity(),
            picture: self.picture.clone(),
            bounds: self.bounds.clone(),
            focus: self.focus.clone(),
            selection: t.color.wash,
            caret: t.color.ink,
            paper: t.color.paper,
            hint: t.color.ink_3,
        })
    }
}

/// The painted page.
struct BodyElement {
    view: Entity<RichEditor>,
    picture: Rc<Picture>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    focus: FocusHandle,
    selection: Hsla,
    caret: Hsla,
    paper: Hsla,
    hint: Hsla,
}

/// The composition and the placeholder, shaped in prepaint.
struct Shaped {
    ime: Option<(Rect, ShapedLine)>,
    placeholder: Option<(Rect, ShapedLine)>,
    line_h: Pixels,
}

impl IntoElement for BodyElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for BodyElement {
    type RequestLayoutState = ();
    type PrepaintState = Shaped;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(self.picture.height.max(geo::BODY_MIN_H)).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Shaped {
        // This frame's place, for the accessibility nodes made after it.
        self.bounds.set(Some(bounds));
        // A new width: the page is laid out again at the next render.
        let width = bounds.size.width.as_f32().min(geo::BODY_MAX_W);
        if (width - self.view.read(cx).width).abs() >= 0.5 {
            let view = self.view.clone();
            window.defer(cx, move |_, cx| {
                view.update(cx, |v, cx| {
                    v.width = width;
                    cx.notify();
                });
            });
        }
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_h = window.line_height();
        let shape = |text: &SharedString, color: Hsla, underline: bool, window: &mut Window| {
            let run = TextRun {
                len: text.len(),
                font: style.font(),
                color,
                background_color: None,
                underline: underline.then_some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(geo::IME_UNDERLINE),
                    wavy: false,
                }),
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.clone(), font_size, &[run], None)
        };
        let ime = self
            .picture
            .ime
            .as_ref()
            .map(|(r, text)| (*r, shape(text, self.caret, true, window)));
        let placeholder = self
            .picture
            .placeholder
            .as_ref()
            .filter(|(_, text)| !text.is_empty())
            .map(|(r, text)| (*r, shape(text, self.hint, false, window)));
        Shaped {
            ime,
            placeholder,
            line_h,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        shaped: &mut Shaped,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.bounds.set(Some(bounds));
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        let at = |r: &Rect| {
            Bounds::new(
                point(bounds.left() + px(r.x), bounds.top() + px(r.y)),
                size(px(r.w), px(r.h)),
            )
        };
        for r in &self.picture.selection {
            window.paint_quad(fill(at(r), self.selection));
        }
        for p in &self.picture.pieces {
            let b = Bounds::new(
                point(bounds.left(), bounds.top() + px(p.y)),
                size(px(p.w), px(p.h)),
            );
            let _ = window.paint_image(b, b, Corners::default(), p.image.clone(), 0, false);
        }
        if let Some((r, line)) = &shaped.placeholder {
            let origin = point(bounds.left() + px(r.x), bounds.top() + px(r.y));
            let _ = line.paint(
                origin,
                shaped.line_h,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        }
        let focused = self.focus.is_focused(window);
        if let Some((r, line)) = &shaped.ime {
            // Over the text at the caret, in a paper box.
            let pad = px(geo::IME_PAD);
            let origin = point(bounds.left() + px(r.x), bounds.top() + px(r.y));
            let boxed = Bounds::new(
                point(origin.x - pad, origin.y - pad),
                size(line.width + pad + pad, shaped.line_h + pad + pad),
            );
            window.paint_quad(fill(boxed, self.paper));
            let _ = line.paint(
                origin,
                shaped.line_h,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        } else if focused && let Some(c) = &self.picture.caret {
            window.paint_quad(fill(at(c), self.caret));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{InputHandler as _, TestAppContext, VisualTestContext};
    use komorebi::compose::ComposeDefaults;

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<RichEditor>, &'a mut VisualTestContext) {
        crate::test_support::init_app(cx);
        cx.update(bind_keys);
        let doc = commands::doc_from_text(text, ComposeDefaults::default().doc());
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut e = RichEditor::new(None, cx);
            e.set_doc(Some(Editor::new(doc)), 0, cx);
            e
        });
        cx.update(|window, cx| {
            let f = view.read(cx).focus.clone();
            window.focus(&f, cx);
        });
        cx.run_until_parked();
        // One frame so the element registers its input handler.
        cx.update(|window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        });
        (view, cx)
    }

    fn text(view: &Entity<RichEditor>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |v, _| v.text().to_string())
    }

    /// Phase 13 (docs/GPUI-MIGRATION.md phase 4 "Not done"): the painted
    /// body gives screen readers its whole text as the value, a node per
    /// line with its place, and the caret and selection in those lines.
    #[gpui::test]
    fn screen_readers_get_the_text_its_lines_and_the_caret(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "First line\nSecond");
        let read = |cx: &mut VisualTestContext| {
            cx.update(|window, cx| {
                window.refresh();
                let _ = window.draw(cx);
            });
            view.read_with(cx, |v, _| (*v.picture.a11y).clone())
        };
        cx.simulate_keystrokes("ctrl-end");
        let t = read(cx);
        assert_eq!(t.value, "First line\nSecond");
        let lines: Vec<&str> = t.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(lines, ["First line", "Second"]);
        assert!(t.lines[1].rect.y > t.lines[0].rect.y, "{t:?}");
        assert!(t.lines[0].rect.w > t.lines[1].rect.w, "{t:?}");
        assert_eq!(t.selection, Some(((1, 6), (1, 6))));
        cx.simulate_keystrokes("shift-home");
        assert_eq!(read(cx).selection, Some(((1, 6), (1, 0))));
        // Words beyond ASCII count as characters, not bytes.
        cx.simulate_keystrokes("ctrl-end");
        cx.simulate_input(" ção");
        assert_eq!(read(cx).selection, Some(((1, 10), (1, 10))));
    }

    /// The spike's `typing_backspace_and_arrows` on the real editor.
    #[gpui::test]
    fn typing_backspace_arrows_and_undo(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "");
        cx.simulate_input("Hello");
        assert_eq!(text(&view, cx), "Hello");
        cx.simulate_keystrokes("backspace left left");
        cx.simulate_input("X");
        assert_eq!(text(&view, cx), "HeXll");
        cx.simulate_keystrokes("shift-right shift-right");
        assert_eq!(view.read_with(cx, |v, _| v.selection()), (3, 5));
        cx.simulate_input("y");
        assert_eq!(text(&view, cx), "HeXy");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("second");
        assert_eq!(text(&view, cx), "HeXy\nsecond");
        // The model made a second paragraph.
        let blocks = view.read_with(cx, |v, _| v.editor().map_or(0, |e| e.doc().blocks.len()));
        assert_eq!(blocks, 2);
        cx.simulate_keystrokes("ctrl-z");
        assert_ne!(text(&view, cx), "HeXy\nsecond");
        cx.simulate_keystrokes("ctrl-y");
        assert_eq!(text(&view, cx), "HeXy\nsecond");
    }

    /// Port of the Slint view test `keys_type_format_and_undo_in_the_body`
    /// (src/app/composer/tests/rich_tests.rs): typing, a formatting key on
    /// a selection, undo of only that.
    #[gpui::test]
    fn keys_type_format_and_undo_in_the_body(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "");
        cx.simulate_input("Hi there");
        cx.simulate_keystrokes("ctrl-shift-left");
        assert_eq!(view.read_with(cx, |v, _| v.selection()), (8, 3));
        cx.simulate_keystrokes("ctrl-b");
        let bold = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| v.editor().unwrap().format_at(3, 8).b)
        };
        assert!(bold(cx));
        // Typing goes on in bold at the end of the bold word.
        cx.simulate_keystrokes("ctrl-end");
        cx.simulate_input("!");
        assert_eq!(text(&view, cx), "Hi there!");
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(text(&view, cx), "Hi there");
        assert!(bold(cx));
        cx.simulate_keystrokes("ctrl-z");
        assert!(!bold(cx), "undo takes the bold back");
        assert_eq!(text(&view, cx), "Hi there");
    }

    /// Port of `a_list_starts_as_typed_and_undo_takes_back_only_that`: "- "
    /// at a line start makes a bulleted list; undo puts the marks back.
    #[gpui::test]
    fn a_list_starts_as_typed_and_undo_takes_back_only_that(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "");
        cx.simulate_input("- ");
        let list = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| v.editor().unwrap().format_at(0, 0).list)
        };
        assert!(list(cx).is_some());
        assert_eq!(text(&view, cx), "");
        cx.simulate_keystrokes("ctrl-z");
        assert!(list(cx).is_none());
        assert_eq!(text(&view, cx), "- ");
    }

    /// Port of `tables_insert_and_tab_moves_between_cells`.
    #[gpui::test]
    fn tables_insert_and_tab_moves_between_cells(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "");
        view.update(cx, |v, cx| v.insert_table(2, 2, cx));
        let in_table = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| v.format().is_some_and(|f| f.in_table))
        };
        assert!(in_table(cx));
        cx.simulate_input("a");
        let first = view.read_with(cx, |v, _| v.selection().1);
        cx.simulate_keystrokes("tab");
        cx.simulate_input("b");
        let second = view.read_with(cx, |v, _| v.selection().1);
        assert!(second > first, "Tab moved to the next cell");
        assert!(in_table(cx));
        cx.simulate_keystrokes("shift-tab");
        assert!(view.read_with(cx, |v, _| v.selection().1) < second);
    }

    /// The calls a Japanese IME makes for "nihon", then 日本, through the
    /// handler WM_IME_COMPOSITION reaches (the spike's
    /// `ime_composition_and_commit`, on the real editor).
    #[gpui::test]
    fn ime_composition_and_commit(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "ab");
        cx.simulate_keystrokes("right");
        cx.update(|window, cx| {
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(300.)));
            let mut h = ElementInputHandler::new(bounds, view.clone());
            h.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
            h.replace_and_mark_text_in_range(None, "にほ", Some(2..2), window, cx);
            h.replace_and_mark_text_in_range(None, "にほん", Some(3..3), window, cx);
            assert_eq!(h.marked_text_range(window, cx), Some(1..4));
            assert_eq!(
                h.text_for_range(0..5, &mut None, window, cx).as_deref(),
                Some("aにほんb")
            );
            assert!(h.bounds_for_range(1..1, window, cx).is_some());
            h.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
            h.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(h.marked_text_range(window, cx), None);
            assert_eq!(
                h.selected_text_range(false, window, cx).map(|s| s.range),
                Some(3..3)
            );
        });
        assert_eq!(text(&view, cx), "a日本b");
    }

    /// The composition never reaches the model: one undo removes the
    /// commit (the spike's `ime_commit_is_one_undo_step`).
    #[gpui::test]
    fn ime_commit_is_one_undo_step(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "");
        cx.update(|window, cx| {
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(300.)));
            let mut h = ElementInputHandler::new(bounds, view.clone());
            h.replace_and_mark_text_in_range(None, "z", None, window, cx);
            h.replace_and_mark_text_in_range(None, "zh", None, window, cx);
            h.replace_and_mark_text_in_range(None, "zhong", None, window, cx);
            assert_eq!(view.read(cx).text(), "", "not in the model yet");
            h.replace_text_in_range(None, "中", window, cx);
        });
        assert_eq!(text(&view, cx), "中");
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(text(&view, cx), "");
    }

    /// Ports of `typing_far_down_a_long_message_reaches_the_caret` and
    /// `a_message_longer_than_the_proxy_window_types_everywhere`: a long
    /// message types at its end and in its middle, and the page is laid
    /// out and painted only around the part in view.
    #[gpui::test]
    fn a_long_message_types_everywhere(cx: &mut TestAppContext) {
        let line = "0123456789 abcdefghi jklmnopqr\n";
        let long = line.repeat(2_000);
        let (view, cx) = setup(cx, long.trim_end());
        cx.simulate_keystrokes("ctrl-end");
        cx.simulate_input("END");
        assert!(text(&view, cx).ends_with("jklmnopqrEND"));
        let mid = long.len() / 2;
        view.update(cx, |v, cx| v.set_selection(mid, mid, cx));
        cx.simulate_input("MID");
        let t = text(&view, cx);
        assert_eq!(t.find("MID"), Some(mid));
        cx.update(|window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        });
        let (pieces, height) = view.read_with(cx, |v, _| (v.picture.pieces.len(), v.page_height()));
        assert!(height > 10_000., "the whole page is laid out: {height}");
        // One piece or more per paragraph: only those near the view (of
        // 2,000).
        assert!(
            pieces > 0 && pieces < 400,
            "only pieces near the view: {pieces}"
        );
    }

    /// Port of `typing_latency_on_a_long_message` (a measurement, ignored
    /// by default): a formatted draft of 2,000 lines with 20 tables, the
    /// caret in its middle; each key goes through the input handler and
    /// one frame is drawn (the model, the changed block's layout, the
    /// pieces in view, GPUI's scene on the test platform). Run with
    /// `cargo test -p komorebi-gpui --release --lib -- --ignored --nocapture typing_latency`.
    #[gpui::test]
    #[ignore = "a measurement (release build)"]
    fn typing_latency_on_a_long_message(cx: &mut TestAppContext) {
        use komorebi::mail::rich::{Block, Cell, Doc, Para, Table};
        let (view, cx) = setup(cx, "");
        let words = "The quick brown fox jumps over the lazy dog and keeps running";
        let mut blocks = Vec::new();
        for i in 0..2000 {
            blocks.push(Block::Para(Para::text(&format!("{i}: {words}"))));
            if i % 100 == 50 {
                let mut t = Table::new(4, 4).unwrap();
                for (r, row) in t.rows.iter_mut().enumerate() {
                    for (c, cell) in row.iter_mut().enumerate() {
                        *cell = Cell::text(&format!("r{r} c{c}"));
                    }
                }
                blocks.push(Block::Table(t));
            }
        }
        let doc = Doc {
            blocks,
            ..Doc::default()
        };
        view.update(cx, |v, cx| {
            v.set_doc(Some(Editor::new(doc)), 0, cx);
            let mid = v.text().len() / 2;
            let mid = v.text()[..mid].rfind('\n').map_or(0, |i| i + 5);
            v.set_selection(mid, mid, cx);
        });
        cx.update(|window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        });
        let mut times = Vec::new();
        for k in 0..200 {
            let c = if k % 10 == 9 { " " } else { "x" };
            let t0 = std::time::Instant::now();
            cx.simulate_input(c);
            cx.update(|window, cx| {
                window.refresh();
                let _ = window.draw(cx);
            });
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "typing latency (key to drawn frame): median {:.2} ms, p95 {:.2} ms over 200 keys",
            times[times.len() / 2],
            times[times.len() * 95 / 100]
        );
        assert!(text(&view, cx).contains("xxxxxxxxx xxxxxxxxx"));
    }

    #[test]
    fn utf16_offsets_follow_surrogate_pairs() {
        assert_eq!(utf16_to_byte("a😀b", 0), 0);
        assert_eq!(utf16_to_byte("a😀b", 1), 1);
        assert_eq!(utf16_to_byte("a😀b", 3), 5);
        assert_eq!(utf16_to_byte("a😀b", 9), 6);
    }
}
