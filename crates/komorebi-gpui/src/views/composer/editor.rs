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
    /// Ctrl+V with files on the clipboard, or files dropped on the text,
    /// that are not pictures it shows: attach them.
    PasteFiles(Vec<std::path::PathBuf>),
    /// Files dragged onto the text (true) or off it.
    DropOver(bool),
    /// The page's height changed (an owner that sizes it to fit follows).
    Resized,
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
    /// The selected picture's frame (its handles at the corners).
    frame: Option<Rect>,
    /// Image files dragged over the text: where they go in.
    drop_caret: Option<Rect>,
    /// What screen readers get: the text, its lines near the view, the
    /// selection (`a11y.rs`).
    a11y: Rc<super::a11y::BodyText>,
}

/// A picture's corner held: the picture's offset, the pointer's x and the
/// width when it was pressed, and which way widens it (+1 right corners).
#[derive(Clone, Copy, Debug)]
struct Resize {
    at: usize,
    x0: f32,
    w0: f32,
    dir: f32,
}

/// The side of a picture's corner handle (logical px).
pub const HANDLE: f32 = 8.;
/// Ctrl+] and Ctrl+[ on a selected picture: this much wider or narrower.
pub const RESIZE_STEP: f32 = 0.1;

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
    /// Stored quotes being laid out, by key; pictures being decoded (or
    /// that did not decode), by cid.
    quotes: HashSet<u64>,
    decoding: HashSet<String>,
    /// A selected picture's corner being dragged.
    resizing: Option<Resize>,
    /// Pictures scaled down when put in: the file as it was (and its size)
    /// by the cid of the one shown, for "Use original picture".
    originals: HashMap<String, (model::ImageFile, u32, u32)>,
    /// Dropped image files over the text (the drop shows where).
    drop_over: bool,
    /// The page height last told ([`EditorEvent::Resized`]).
    told_height: f32,
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
    /// `db` reads the pictures of stored signatures a document shows and
    /// does not hold (none in tests that need none).
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
            decoding: HashSet::new(),
            resizing: None,
            originals: HashMap::new(),
            drop_over: false,
            told_height: 0.,
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
        // A signature's pictures it shows and does not hold (one put in
        // before pictures were saved with drafts): from the signatures.
        if let (Some(db), Some(ed)) = (&self.db, self.ed.as_mut()) {
            commands::missing_images(db, ed);
        }
        self.view = None;
        self.marked = None;
        self.goal_x = None;
        self.quotes.clear();
        self.decoding.clear();
        self.resizing = None;
        self.originals.clear();
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
        match action {
            Action::InsertPicture => return self.pick_pictures(cx),
            // A selected picture: Ctrl+] and Ctrl+[ resize it.
            Action::SizeStep(up) if self.step_picture(*up, cx) => return,
            _ => {}
        }
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
            // Pictures go with it as `data:` addresses: another app (and
            // this one's paste) gets them without the network.
            let images = ed.images();
            let html = model::emit::html_with(&frag, model::Mode::Send, &|img| {
                images.get(&img.cid).map(model::images::data_uri)
            });
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

    /// Ctrl+V: copied picture files go in the text, other files are
    /// attached by the owner; formatted text is pasted as the model holds
    /// it (the lossy import, its pictures read from the HTML itself or
    /// this PC, never the network); a copied bitmap goes in as a PNG; plain
    /// text as typed. The import and the pictures are read on the
    /// background executor.
    fn paste(&mut self, plain: bool, cx: &mut Context<Self>) {
        if !plain {
            let files = komorebi::files::clipboard_files();
            if !files.is_empty() {
                let (pics, rest) = super::pictures::split_paths(files);
                if !rest.is_empty() {
                    cx.emit(EditorEvent::PasteFiles(rest));
                }
                if !pics.is_empty() {
                    self.insert_picture_files(pics, cx);
                }
                return;
            }
            if komorebi::files::clipboard_has_rich() && self.ed.is_some() {
                self.paste_rich(cx);
                return;
            }
        }
        self.paste_text(cx);
    }

    fn paste_text(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            let (a, b) = self.ordered();
            self.edit(a, b, &text.replace("\r\n", "\n"), cx);
        }
    }

    fn paste_rich(&mut self, cx: &mut Context<Self>) {
        let known = self
            .ed
            .as_ref()
            .map(|ed| ed.images().clone())
            .unwrap_or_default();
        let read = cx.background_spawn(async move {
            if let Some(html) = komorebi::files::clipboard_html() {
                let file = |p: &std::path::Path| model::images::read_small_file(p);
                let mut r = model::images::Resolver::new(&known, &file);
                if let Some(doc) = model::import::lossy_with_images(&html, &mut |f| r.resolve(f)) {
                    return Pasted::Doc(doc, r.found);
                }
            }
            match komorebi::files::clipboard_picture().and_then(super::pictures::from_clipboard) {
                Some(p) => Pasted::Picture(p),
                None => Pasted::Nothing,
            }
        });
        cx.spawn(async move |this, cx| {
            let pasted = read.await;
            let _ = this.update(cx, |this, cx| match pasted {
                Pasted::Doc(frag, found) => {
                    let (a, c) = (this.anchor, this.cursor);
                    let caret = this.ed.as_mut().and_then(|ed| {
                        ed.add_images(&found);
                        ed.paste(a, c, frag)
                    });
                    if let Some(caret) = caret {
                        this.anchor = caret;
                        this.cursor = caret;
                        this.reveal = true;
                        cx.emit(EditorEvent::Changed);
                        cx.emit(EditorEvent::Moved);
                        cx.notify();
                    } else {
                        this.paste_text(cx);
                    }
                }
                Pasted::Picture(p) => this.insert_pictures(vec![p], cx),
                Pasted::Nothing => this.paste_text(cx),
            });
        })
        .detach();
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
        view.sync(ed, engine);
        // Pictures shown and not decoded yet at this size: decoded on the
        // background executor (never here), a waiting box meanwhile.
        let pictures: Vec<_> = view
            .picture_jobs(ed)
            .into_iter()
            .filter(|(cid, _)| self.decoding.insert(cid.clone()))
            .filter_map(|(cid, want)| Some((ed.images().get(&cid)?.clone(), cid, want)))
            .collect();
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
        let frame =
            selected_picture(ed, self.anchor, self.cursor).and_then(|at| view.image_rect(ed, at));
        let drop_caret = self.drop_over.then_some(caret);
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
            frame,
            drop_caret,
            a11y,
        });
        if (view.height() - self.told_height).abs() >= 0.5 {
            self.told_height = view.height();
            let me = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = me.update(cx, |_, cx| cx.emit(EditorEvent::Resized));
            });
        }
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
        for (file, cid, want) in pictures {
            self.decode_picture(file, cid, want, cx);
        }
    }

    /// A picture's pixels at `want` device px wide, decoded on the
    /// background executor. One that does not decode stays in `decoding`
    /// (its waiting box stays; it is not tried again).
    fn decode_picture(
        &mut self,
        file: model::ImageFile,
        cid: String,
        want: u32,
        cx: &mut Context<Self>,
    ) {
        let task = cx.background_spawn(async move { commands::decode_picture(&file, want) });
        cx.spawn(async move |this, cx| {
            let Some(pix) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.decoding.remove(&cid);
                if let (Some(ed), Some(view)) = (this.ed.as_ref(), this.view.as_mut()) {
                    view.set_picture(ed, &cid, pix, want);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ---- pictures ---------------------------------------------------------

    /// The selected picture's offset: a selection of exactly one picture.
    pub fn selected_picture(&self) -> Option<usize> {
        selected_picture(self.ed.as_ref()?, self.anchor, self.cursor)
    }

    /// The selected picture's alt text, if a picture is selected.
    pub fn picture_alt(&self) -> Option<String> {
        let at = self.selected_picture()?;
        Some(self.ed.as_ref()?.image_at(at)?.alt.clone())
    }

    /// Whether the selected picture was scaled down when put in (its
    /// original can be used instead).
    pub fn has_original(&self) -> bool {
        self.selected_picture()
            .and_then(|at| self.ed.as_ref()?.image_at(at))
            .is_some_and(|img| self.originals.contains_key(&img.cid))
    }

    /// Change the selected picture (one undo step; `burst`: a run of
    /// resizes is one step). False if none is selected.
    fn change_picture(
        &mut self,
        burst: bool,
        cx: &mut Context<Self>,
        f: impl FnOnce(&model::InlineImage) -> model::InlineImage,
    ) -> bool {
        let Some(at) = self.selected_picture() else {
            return false;
        };
        let Some(ed) = self.ed.as_mut() else {
            return false;
        };
        let Some(img) = ed.image_at(at).map(f) else {
            return false;
        };
        if !ed.set_image(at, img, burst) {
            return false;
        }
        self.anchor = at;
        self.cursor = at + model::edit::OBJ.len_utf8();
        cx.emit(EditorEvent::Changed);
        cx.emit(EditorEvent::Moved);
        cx.notify();
        true
    }

    /// The selected picture shown at `width` CSS px (its aspect kept).
    pub fn resize_picture(&mut self, width: f32, burst: bool, cx: &mut Context<Self>) -> bool {
        let w = width.round().clamp(1., f32::from(u16::MAX)) as u16;
        self.change_picture(burst, cx, |img| img.sized(w))
    }

    /// Ctrl+] and Ctrl+[ on a selected picture: 10 % wider or narrower.
    fn step_picture(&mut self, up: bool, cx: &mut Context<Self>) -> bool {
        let Some(w) = self
            .selected_picture()
            .and_then(|at| self.ed.as_ref()?.image_at(at))
            .map(|img| f32::from(img.width))
        else {
            return false;
        };
        let f = if up {
            1. + RESIZE_STEP
        } else {
            1. - RESIZE_STEP
        };
        self.resize_picture(w * f, true, cx)
    }

    /// The selected picture at its own width (at most the default).
    pub fn picture_own_size(&mut self, cx: &mut Context<Self>) -> bool {
        self.change_picture(false, cx, |img| {
            model::InlineImage::new(img.cid.clone(), img.iw, img.ih, img.alt.clone())
        })
    }

    /// The selected picture's alt text (what the plain-text part and
    /// screen readers get).
    pub fn set_picture_alt(&mut self, alt: &str, cx: &mut Context<Self>) -> bool {
        let alt = alt.trim().to_string();
        self.change_picture(false, cx, |img| model::InlineImage { alt, ..img.clone() })
    }

    /// The selected picture's original file instead of the scaled-down
    /// one (it keeps its shown width).
    pub fn use_original(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(img) = self
            .selected_picture()
            .and_then(|at| self.ed.as_ref()?.image_at(at))
            .cloned()
        else {
            return false;
        };
        let Some((file, iw, ih)) = self.originals.get(&img.cid).cloned() else {
            return false;
        };
        let cid = file.cid();
        if let Some(ed) = self.ed.as_mut() {
            let mut one = model::Images::default();
            one.insert(cid.clone(), file);
            ed.add_images(&one);
        }
        self.change_picture(false, cx, |old| model::InlineImage {
            cid,
            iw,
            ih,
            ..old.clone()
        })
    }

    /// Prepared pictures put in at the caret (or over the selection), one
    /// after another; the caret goes after the last. The originals of the
    /// scaled-down ones are kept for "Use original picture".
    pub fn insert_pictures(
        &mut self,
        pics: Vec<super::pictures::Prepared>,
        cx: &mut Context<Self>,
    ) {
        let (mut a, mut b) = self.ordered();
        let mut put = false;
        for p in pics {
            let Some(ed) = self.ed.as_mut() else {
                return;
            };
            let img = model::InlineImage::new(p.file.cid(), p.iw, p.ih, "");
            if let Some(orig) = p.original {
                self.originals.insert(img.cid.clone(), orig);
            }
            if let Some(caret) = ed.insert_image(a, b, img, p.file) {
                (a, b) = (caret, caret);
                put = true;
            }
        }
        if put {
            self.anchor = a;
            self.cursor = b;
            self.goal_x = None;
            self.reveal = true;
            cx.emit(EditorEvent::Changed);
            cx.emit(EditorEvent::Moved);
            cx.notify();
        }
    }

    /// Picture files read and prepared on the background executor, then
    /// put in at the caret. Files that are not pictures it can show are
    /// handed back to the owner to attach.
    pub fn insert_picture_files(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let read = cx.background_spawn(async move {
            let mut pics = Vec::new();
            let mut rest = Vec::new();
            for p in paths {
                match super::pictures::read(&p) {
                    Some(pic) => pics.push(pic),
                    None => rest.push(p),
                }
            }
            (pics, rest)
        });
        cx.spawn(async move |this, cx| {
            let (pics, rest) = read.await;
            let _ = this.update(cx, |this, cx| {
                this.insert_pictures(pics, cx);
                if !rest.is_empty() {
                    cx.emit(EditorEvent::PasteFiles(rest));
                }
            });
        })
        .detach();
    }

    /// "Insert picture": the file picker, then the pictures at the caret.
    pub fn pick_pictures(&mut self, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = answer.await {
                let _ = this.update(cx, |this, cx| this.insert_picture_files(paths, cx));
            }
        })
        .detach();
    }

    /// Files dropped on the text: pictures go in where they were dropped;
    /// the rest are attached (by the owner).
    fn drop_paths(&mut self, paths: &gpui::ExternalPaths, cx: &mut Context<Self>) {
        self.drop_over = false;
        let (pics, rest) = super::pictures::split_paths(paths.paths().to_vec());
        if !rest.is_empty() {
            cx.emit(EditorEvent::PasteFiles(rest));
        }
        if !pics.is_empty() {
            self.insert_picture_files(pics, cx);
        }
        cx.notify();
    }

    fn drag_over(&mut self, e: &gpui::DragMoveEvent<gpui::ExternalPaths>, cx: &mut Context<Self>) {
        let over = e.bounds.contains(&e.event.position);
        if over && let Some(at) = self.point_at(e.event.position) {
            self.anchor = at;
            self.cursor = at;
        }
        if over != self.drop_over {
            self.drop_over = over;
            cx.emit(EditorEvent::DropOver(over));
        }
        cx.notify();
    }

    /// Whether image files are being dragged over the text.
    pub fn drop_over(&self) -> bool {
        self.drop_over
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

    /// A page point in logical px of the page.
    fn page_point(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let b = self.bounds.get()?;
        Some((
            (position.x - b.left()).as_f32(),
            (position.y - b.top()).as_f32(),
        ))
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.goal_x = None;
        // A selected picture's corner: resize it; a picture: select it.
        if let Some((x, y)) = self.page_point(e.position)
            && !e.modifiers.shift
        {
            if let Some(frame) = self.picture.frame
                && let Some(dir) = handle_at(&frame, x, y)
                && let Some(at) = self.selected_picture()
            {
                self.resizing = Some(Resize {
                    at,
                    x0: x,
                    w0: frame.w,
                    dir,
                });
                return;
            }
            let hit = match (self.ed.as_ref(), self.view.as_ref()) {
                (Some(ed), Some(view)) => view.image_hit(ed, x, y),
                _ => None,
            };
            if let Some(at) = hit {
                self.anchor = at;
                self.cursor = at + model::edit::OBJ.len_utf8();
                self.drag_from = at;
                cx.emit(EditorEvent::Moved);
                cx.notify();
                return;
            }
        }
        let Some(at) = self.point_at(e.position) else {
            return;
        };
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
            self.resizing = None;
            return;
        }
        if let Some(r) = self.resizing {
            let Some((x, _)) = self.page_point(e.position) else {
                return;
            };
            // The frame is drawn scaled to the page; the width is CSS px.
            let shown = self.picture.frame.map_or(r.w0, |f| f.w).max(1.);
            let css = self
                .ed
                .as_ref()
                .and_then(|ed| ed.image_at(r.at))
                .map_or(shown, |img| f32::from(img.width));
            let ratio = css / shown;
            let w = (r.w0 + (x - r.x0) * r.dir).max(1.) * ratio;
            self.resize_picture(w, true, cx);
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

/// What a formatted paste read off the UI thread.
enum Pasted {
    Doc(model::Doc, model::Images),
    Picture(super::pictures::Prepared),
    Nothing,
}

/// The picture a selection `a..c` covers exactly, if any: its offset.
fn selected_picture(ed: &Editor, a: usize, c: usize) -> Option<usize> {
    let (lo, hi) = (a.min(c), a.max(c));
    (hi - lo == model::edit::OBJ.len_utf8() && ed.image_at(lo).is_some()).then_some(lo)
}

/// The corner handle of `frame` under `(x, y)`, if any: +1 for the right
/// corners (dragging right widens), -1 for the left.
fn handle_at(frame: &Rect, x: f32, y: f32) -> Option<f32> {
    let near = |cx: f32, cy: f32| (x - cx).abs() <= HANDLE && (y - cy).abs() <= HANDLE;
    let (l, r, t, b) = (frame.x, frame.x + frame.w, frame.y, frame.y + frame.h);
    if near(r, t) || near(r, b) {
        Some(1.)
    } else if near(l, t) || near(l, b) {
        Some(-1.)
    } else {
        None
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
        // The drag ended or left the window.
        if self.drop_over && !cx.has_active_drag() {
            self.drop_over = false;
        }
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
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, _| this.resizing = None),
            )
            // Image files dropped on the text go in where they are dropped
            // (the caret shows where); other files are attached.
            .on_drag_move(cx.listener(
                |this, e: &gpui::DragMoveEvent<gpui::ExternalPaths>, _, cx| this.drag_over(e, cx),
            ))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.drop_paths(paths, cx);
                cx.stop_propagation();
            }));
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
        } else if let Some(c) = &self.picture.drop_caret {
            // Where dropped pictures go in: a caret twice as wide.
            let r = Rect {
                w: geo::CARET_W * 2.,
                ..*c
            };
            window.paint_quad(fill(at(&r), self.caret));
        } else if focused
            && self.picture.frame.is_none()
            && let Some(c) = &self.picture.caret
        {
            window.paint_quad(fill(at(c), self.caret));
        }
        if self.picture.drop_caret.is_some() {
            // The text as a drop target: an ink frame round the page, apart
            // from the composer's "Drop to attach" wash.
            window.paint_quad(gpui::quad(
                bounds,
                px(crate::theme::radius::CARD),
                gpui::transparent_black(),
                px(crate::theme::layout::FOCUS_RING),
                self.caret,
                gpui::BorderStyle::Dashed,
            ));
        }
        if let Some(f) = &self.picture.frame {
            // The selected picture: an ink frame and four corner handles
            // (paper squares ringed in ink), as in Outlook and Word.
            let ring = crate::theme::layout::FOCUS_RING;
            let framed = Rect {
                x: f.x - ring,
                y: f.y - ring,
                w: f.w + ring * 2.,
                h: f.h + ring * 2.,
            };
            window.paint_quad(gpui::quad(
                at(&framed),
                Pixels::ZERO,
                gpui::transparent_black(),
                px(ring),
                self.caret,
                gpui::BorderStyle::Solid,
            ));
            for (x, y) in [
                (f.x, f.y),
                (f.x + f.w, f.y),
                (f.x, f.y + f.h),
                (f.x + f.w, f.y + f.h),
            ] {
                let h = Rect {
                    x: x - HANDLE / 2.,
                    y: y - HANDLE / 2.,
                    w: HANDLE,
                    h: HANDLE,
                };
                window.paint_quad(gpui::quad(
                    at(&h),
                    Pixels::ZERO,
                    self.paper,
                    px(crate::theme::layout::HAIRLINE),
                    self.caret,
                    gpui::BorderStyle::Solid,
                ));
            }
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
    /// A real PNG of `w` x `h` (a flat colour), prepared as a paste or a
    /// drop prepares it.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([30, 90, 160]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        });
    }

    /// The picture at `at` in window coordinates.
    fn picture_bounds(view: &Entity<RichEditor>, at: usize, cx: &mut VisualTestContext) -> Rect {
        view.read_with(cx, |v, _| {
            let b = v.bounds.get().unwrap();
            let r = v
                .view
                .as_ref()
                .unwrap()
                .image_rect(v.ed.as_ref().unwrap(), at)
                .unwrap();
            Rect {
                x: b.left().as_f32() + r.x,
                y: b.top().as_f32() + r.y,
                ..r
            }
        })
    }

    /// Pictures go in the text at the caret, are decoded off the UI
    /// thread, and a click selects one: its frame and handles show, a
    /// corner drag and Ctrl+] resize it (one undo step each), Backspace
    /// deletes it and Ctrl+Z brings it back. A large one is scaled down and
    /// its original kept for "Use original picture".
    #[gpui::test]
    fn a_picture_goes_in_is_selected_resized_and_deleted(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "Step 1:");
        cx.simulate_keystrokes("ctrl-end");
        let small = super::super::pictures::prepare(png(200, 100)).unwrap();
        let big = super::super::pictures::prepare(png(2400, 600)).unwrap();
        assert_eq!((big.iw, big.ih), (1600, 400));
        view.update(cx, |v, cx| v.insert_pictures(vec![small, big], cx));
        let obj = model::edit::OBJ;
        assert_eq!(text(&view, cx), format!("Step 1:{obj}{obj}"));
        redraw(cx);
        let cids: Vec<String> = view.read_with(cx, |v, _| {
            v.editor()
                .unwrap()
                .image_offsets()
                .iter()
                .map(|(_, i)| i.cid.clone())
                .collect()
        });
        assert_eq!(cids.len(), 2);
        redraw(cx);
        let decoded = view.read_with(cx, |v, _| {
            cids.iter().all(|c| v.view.as_ref().unwrap().has_picture(c))
        });
        assert!(decoded, "both decoded for the view");
        // A click on the first selects it; its frame shows.
        let at = "Step 1:".len();
        let r = picture_bounds(&view, at, cx);
        let middle = point(px(r.x + r.w / 2.), px(r.y + r.h / 2.));
        cx.simulate_mouse_down(middle, MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(middle, MouseButton::Left, gpui::Modifiers::none());
        redraw(cx);
        assert_eq!(view.read_with(cx, |v, _| v.selected_picture()), Some(at));
        assert!(view.read_with(cx, |v, _| v.picture.frame.is_some()));
        assert!(
            !view.read_with(cx, |v, _| v.has_original()),
            "small: kept as it was"
        );
        let width = |cx: &mut VisualTestContext| {
            view.read_with(cx, |v, _| v.editor().unwrap().image_at(at).unwrap().width)
        };
        assert_eq!(width(cx), 200);
        // The bottom-right handle dragged 40 px right: wider, aspect kept.
        let corner = point(px(r.x + r.w), px(r.y + r.h));
        cx.simulate_mouse_down(corner, MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(
            point(px(r.x + r.w + 40.), px(r.y + r.h)),
            Some(MouseButton::Left),
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_up(
            point(px(r.x + r.w + 40.), px(r.y + r.h)),
            MouseButton::Left,
            gpui::Modifiers::none(),
        );
        let dragged = width(cx);
        assert!(dragged > 220, "{dragged}");
        // Ctrl+] makes it a tenth wider.
        cx.simulate_keystrokes("ctrl-]");
        let stepped = width(cx);
        assert!(stepped > dragged, "{stepped} after {dragged}");
        assert_eq!(
            text(&view, cx),
            format!("Step 1:{obj}{obj}"),
            "no text changed"
        );
        // Backspace deletes it; Ctrl+Z puts it back, at its new width.
        cx.simulate_keystrokes("backspace");
        assert_eq!(text(&view, cx), format!("Step 1:{obj}"));
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(text(&view, cx), format!("Step 1:{obj}{obj}"));
        assert_eq!(width(cx), stepped);
        // The large one: its original can be used instead.
        let at2 = at + obj.len_utf8();
        view.update(cx, |v, cx| v.set_selection(at2, at2 + obj.len_utf8(), cx));
        assert!(view.read_with(cx, |v, _| v.has_original()));
        assert!(view.update(cx, |v, cx| v.use_original(cx)));
        let iw = view.read_with(cx, |v, _| v.editor().unwrap().image_at(at2).unwrap().iw);
        assert_eq!(iw, 2400);
        // Alt text, for the plain-text part and screen readers.
        assert!(view.update(cx, |v, cx| v.set_picture_alt(" The Save button ", cx)));
        assert_eq!(
            view.read_with(cx, |v, _| v.picture_alt()),
            Some("The Save button".to_string())
        );
    }

    /// Dropped or pasted files: the pictures go in the text (read on the
    /// background executor), the rest come back to be attached; an SVG is
    /// never shown.
    #[gpui::test]
    fn picture_files_go_in_the_text_and_others_are_attached(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, "");
        let dir = std::env::temp_dir().join(format!("komorebi-pictures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pic = dir.join("screen.png");
        std::fs::write(&pic, png(64, 48)).unwrap();
        let svg = dir.join("logo.svg");
        std::fs::write(&svg, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let pdf = dir.join("notes.pdf");
        std::fs::write(&pdf, b"%PDF-1.4").unwrap();
        let attached = Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = attached.clone();
        let _sub = cx.update(|_, cx| {
            cx.subscribe(&view, move |_, e: &EditorEvent, _| {
                if let EditorEvent::PasteFiles(p) = e {
                    seen.borrow_mut().extend(p.clone());
                }
            })
        });
        let (pics, rest) =
            super::super::pictures::split_paths(vec![pic.clone(), svg.clone(), pdf.clone()]);
        assert_eq!(pics, [pic]);
        view.update(cx, |v, cx| v.insert_picture_files(pics, cx));
        cx.run_until_parked();
        assert_eq!(text(&view, cx), model::edit::OBJ.to_string());
        assert_eq!(rest, [svg.clone(), pdf.clone()]);
        // A file named like a picture that is not one comes back too.
        let fake = dir.join("fake.png");
        std::fs::write(&fake, b"not a picture").unwrap();
        view.update(cx, |v, cx| v.insert_picture_files(vec![fake.clone()], cx));
        cx.run_until_parked();
        assert_eq!(*attached.borrow(), [fake]);
        assert_eq!(text(&view, cx), model::edit::OBJ.to_string());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
