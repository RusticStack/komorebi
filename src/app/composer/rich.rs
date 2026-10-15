//! The formatted editor's controller (docs/WINUI3.md "RichEditBox
//! mapping"): the composer's body and the signature being edited in
//! Settings are each a [`Editor`] (Komorebi's own document model, edited
//! through a plain-text projection) shown by a [`View`] (laid out with the
//! reader's text engine, kept per block, painted in pieces).
//!
//! `ui/editor.slint`'s `RichBody` holds the projection in a hidden
//! TextInput, which owns the caret, IME, dead keys, the clipboard's plain
//! text and the screen reader's view. Each edit it reports is applied to
//! the model (an edit the model cannot hold is refused, and the projection
//! is put back); only the blocks it changed are laid out again, and only
//! the pieces in view that changed are painted again. Up and Down, Page
//! Up and Down, Home and End, clicks and drags are worked out here from
//! the layout, and the selection is set back into the proxy.
//!
//! The proxy holds the whole projection up to [`WINDOW`] bytes; a longer
//! message puts only a window of it there (the lines around the caret and
//! the whole selection), as the TextInput lays out all its text on every
//! key. The window moves when the caret nears its edge.
//!
//! Everything here runs on the UI thread, except a stored quote's layout
//! (full HTML, recursive), which runs on a short-lived thread.

use super::*;
use crate::{EdAction, EdFormat, EdKey, EdPiece, EdRect, EdState, Rich};
use komorebi::mail::render::{self as rnd, Engine};
use komorebi::mail::rich::edit::{CharOp, Editor, ParaOp, TableOp};
use komorebi::mail::rich::view::{self, View};
use komorebi::mail::rich::{self as model, Align, ColorId, FontId, ListKind, SizePt};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// The composer's body.
pub(in crate::app) const BODY: i32 = 0;
/// The signature being edited in Settings.
pub(in crate::app) const SIG: i32 = 1;

/// The page's rows painted above and below the part in view (one view
/// height each way), so a short scroll shows pieces already painted.
const MARGIN_VIEWS: f32 = 1.0;
/// Room kept between the caret and the edge it is scrolled to.
const CARET_ROOM: f32 = 8.0;
/// The stored quote's layout thread's stack (the reader worker's).
const QUOTE_STACK: usize = 8 * 1024 * 1024;
/// The most of the projection the proxy holds (bytes): a message this long
/// is all there (Narrator reads all of it); a longer one, a window.
pub(in crate::app) const WINDOW: usize = 16 * 1024;
/// The window moves once the caret is this close to an edge that is not
/// the message's.
const WINDOW_MARGIN: usize = 2 * 1024;
/// A window edge snaps to a line's start or end this close.
const WINDOW_SNAP: usize = 2 * 1024;

/// The part of `text` the proxy holds for the selection `a..c` (`old`: the
/// window now, kept while the selection stays clear of its edges).
fn window_for(text: &str, a: usize, c: usize, old: (usize, usize)) -> (usize, usize) {
    let len = text.len();
    if len <= WINDOW {
        return (0, len);
    }
    let (lo, hi) = (a.min(c).min(len), a.max(c).min(len));
    let (ows, owe) = (old.0.min(len), old.1.min(len));
    let clear_start = ows == 0 || ows.saturating_add(WINDOW_MARGIN) <= lo;
    let clear_end = owe == len || hi.saturating_add(WINDOW_MARGIN) <= owe;
    if ows < owe && clear_start && clear_end && owe - ows <= 2 * WINDOW.max(hi - lo) {
        return (ows, owe);
    }
    let mut ws = lo.min(c.saturating_sub(WINDOW / 2));
    let mut we = hi.max(c.saturating_add(WINDOW / 2)).min(len);
    // Whole lines where a line edge is near.
    if let Some(nl) = text
        .get(ws.saturating_sub(WINDOW_SNAP)..ws)
        .and_then(|s| s.rfind('\n'))
    {
        ws = ws.saturating_sub(WINDOW_SNAP).min(ws) + nl + 1;
    } else if ws <= WINDOW_SNAP {
        ws = 0;
    }
    if let Some(nl) = text
        .get(we..we.saturating_add(WINDOW_SNAP).min(len))
        .and_then(|s| s.find('\n'))
    {
        we += nl;
    } else if len - we <= WINDOW_SNAP {
        we = len;
    }
    while ws > 0 && !text.is_char_boundary(ws) {
        ws -= 1;
    }
    while we < len && !text.is_char_boundary(we) {
        we += 1;
    }
    (ws, we)
}

/// One editor's picture and selection.
#[derive(Default)]
pub(in crate::app) struct Pane {
    view: Option<View>,
    /// The page's width and the part of it in view (logical px).
    width: f32,
    top: f32,
    height: f32,
    anchor: usize,
    cursor: usize,
    /// The x a run of Up and Down keys keeps to.
    goal_x: Option<f32>,
    /// The selection this controller set last: its echo from the proxy
    /// changes nothing.
    set: Option<(usize, usize)>,
    /// Where a drag started.
    drag_from: usize,
    seq: i32,
    scroll_seq: i32,
    /// The part of the projection the proxy holds (bytes `ws..we`), as far
    /// as this controller knows.
    pushed: SharedString,
    ws: usize,
    we: usize,
    /// The proxy reported a caret move: the window stays until the event
    /// is over (its edit, if any, still refers to it), then moves.
    hold_window: bool,
    /// A move of the window is waiting for the event to end.
    window_due: bool,
    /// Painted pieces by key.
    images: HashMap<u64, Image>,
    /// Stored quotes being laid out, by key.
    quotes: HashSet<u64>,
    /// Logo marks handed to the view.
    logos: HashSet<u64>,
}

impl Pane {
    /// Another document: nothing of the old one's picture is kept.
    fn reset(&mut self) {
        let (width, top, height) = (self.width, 0.0, self.height);
        *self = Pane {
            width,
            top,
            height,
            seq: self.seq,
            scroll_seq: self.scroll_seq.wrapping_add(1),
            ..Pane::default()
        };
    }
}

/// Both editors and what they share.
#[derive(Default)]
pub(in crate::app) struct EdUi {
    engine: Option<Engine>,
    panes: [Pane; 2],
    /// The signature being edited in Settings.
    pub sig: Option<Editor>,
    /// Signature logos' bytes, by mark.
    logos: HashMap<u64, Arc<[u8]>>,
}

/// An editor, its pane, the engine and the logos, borrowed together.
type Parts<'a> = (
    &'a mut Editor,
    &'a mut Pane,
    &'a mut Engine,
    &'a mut HashMap<u64, Arc<[u8]>>,
);

fn slot(id: i32) -> usize {
    usize::from(id == SIG)
}

/// Item `i` of a closed list (a toolbar box's index).
fn pick<T: Copy>(all: &[T], i: i32) -> Option<T> {
    usize::try_from(i).ok().and_then(|i| all.get(i).copied())
}

fn rgba(c: slint::Color) -> [u8; 4] {
    [c.red(), c.green(), c.blue(), c.alpha()]
}

/// The window's theme resources the page is drawn with.
fn view_theme(ui: &AppWindow) -> view::Theme {
    let t = ui.global::<Theme>();
    view::Theme {
        text: rgba(t.get_text()),
        link: rgba(t.get_link()),
        subtle: rgba(t.get_text_2()),
        border: rgba(t.get_stroke_accessible()),
        bg: rgba(t.get_card()),
        dark: t.get_dark(),
        contrast: t.get_contrast(),
    }
}

fn ed_rect(r: rnd::Rect) -> EdRect {
    EdRect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

/// A picture for Slint from premultiplied pixels.
fn image_of(pm: &tiny_skia::Pixmap) -> Image {
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(pm.data(), pm.width(), pm.height());
    Image::from_rgba8_premultiplied(buf)
}

/// The toolbar's state for `f`.
pub(in crate::app) fn format_state(f: &model::edit::Format, ed: &Editor) -> EdFormat {
    let index = |i: Option<usize>| i.map_or(-1, |i| i as i32);
    EdFormat {
        b: f.b,
        i: f.i,
        u: f.u,
        s: f.s,
        font: index(FontId::ALL.iter().position(|x| *x == f.font)),
        size: index(SizePt::ALL.iter().position(|x| *x == f.size.get())),
        color: index(
            f.color
                .and_then(|c| ColorId::ALL.iter().position(|x| *x == c)),
        ),
        align: match f.align {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
            Align::Justify => 3,
        },
        list_kind: match f.list {
            None => 0,
            Some(ListKind::Bullet) => 1,
            Some(ListKind::Number) => 2,
        },
        quote: f.quote,
        in_table: f.in_table,
        in_signature: f.in_signature,
        quote_shown: ed.has_quote() && !ed.quote_folded(),
        link: f.link.is_some(),
        can_undo: ed.can_undo(),
        can_redo: ed.can_redo(),
    }
}

/// Whether the message shows nothing written (the placeholder shows).
fn is_blank(ed: &Editor) -> bool {
    ed.doc().blocks.iter().all(|b| match b {
        model::Block::Para(p) => p.is_empty(),
        _ => false,
    })
}

/// A document from settings' defaults and plain `text`.
pub(in crate::app) fn doc_from_text(
    text: &str,
    defaults: (FontId, SizePt, Option<ColorId>),
) -> model::Doc {
    let mut doc = if text.is_empty() {
        model::Doc::default()
    } else {
        model::Doc::from_plain(text)
    };
    (doc.font, doc.size, doc.color) = defaults;
    doc
}

/// Load the logos of the signatures in `doc` from the cache (once) and
/// hand them to the view.
fn give_logos(
    doc: &model::Doc,
    view: &mut View,
    given: &mut HashSet<u64>,
    logos: &mut HashMap<u64, Arc<[u8]>>,
    db: &Db,
) {
    for b in &doc.blocks {
        let model::Block::Signature(s) = b else {
            continue;
        };
        let Some(l) = s.logo else {
            continue;
        };
        if given.contains(&l.hash) {
            continue;
        }
        if !logos.contains_key(&l.hash)
            && let Some(id) = s.source
            && let Ok(Some(sig)) = db.signature(id)
            && let Some(bytes) = sig.logo
            && model::fnv(&bytes) == l.hash
        {
            logos.insert(l.hash, Arc::from(bytes));
        }
        if let Some(bytes) = logos.get(&l.hash) {
            view.set_logo(l.hash, bytes.clone());
            given.insert(l.hash);
        }
    }
}

impl App {
    /// The editor `id` and its pane, the engine and the logos (none: no
    /// formatted draft is shown, or no signature is being edited).
    fn ed_parts(&mut self, id: i32) -> Option<Parts<'_>> {
        let EdUi {
            engine,
            panes,
            sig,
            logos,
        } = &mut self.ed;
        let editor = if id == SIG {
            sig.as_mut()?
        } else {
            self.drafts.active_mut()?.rich.as_mut()?
        };
        let pane = panes.get_mut(slot(id))?;
        let engine = engine.get_or_insert_with(Engine::new);
        Some((editor, pane, engine, logos))
    }

    /// Put the caret of editor `id` at `offset` (shown at the next
    /// [`App::ed_show`]).
    pub(in crate::app) fn ed_set_caret(&mut self, id: i32, offset: usize) {
        if let Some(p) = self.ed.panes.get_mut(slot(id)) {
            p.anchor = offset;
            p.cursor = offset;
        }
    }

    /// A new document in editor `id` (another draft shown, a signature
    /// opened): its picture starts again.
    pub(in crate::app) fn ed_reset(&mut self, id: i32) {
        if let Some(p) = self.ed.panes.get_mut(slot(id)) {
            p.reset();
        }
    }

    /// Show editor `id` as it is now: lay out what changed, find the caret
    /// and the selection, paint the pieces in view not painted yet.
    /// `push`: the projection changed (put it into the proxy); `select`:
    /// set the selection into the proxy; `scroll`: keep the caret in view.
    pub(in crate::app) fn ed_show(&mut self, id: i32, push: bool, select: bool, scroll: bool) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let scale = ui.window().scale_factor();
        let theme = view_theme(&ui);
        let zoom = ui.global::<Theme>().get_body() / 14.0;
        let db = self.db.clone();
        let mut jobs = Vec::new();
        let mut rewindow: Vec<i32> = Vec::new();
        let (state, fmt) = {
            let Some((editor, pane, engine, logos)) = self.ed_parts(id) else {
                return;
            };
            let width = if pane.width > 1.0 { pane.width } else { 600.0 };
            let view = pane
                .view
                .get_or_insert_with(|| View::new(width, scale, zoom, theme));
            if view.set_geometry(width, scale, zoom, theme) {
                pane.images.clear();
                pane.quotes.clear();
            }
            give_logos(editor.doc(), view, &mut pane.logos, logos, &db);
            view.sync(editor, engine);
            view.prepare_logos(editor);
            for job in view.quote_jobs(editor) {
                if pane.quotes.insert(job.0) {
                    jobs.push(job);
                }
            }
            let len = editor.text().len();
            pane.anchor = pane.anchor.min(len);
            pane.cursor = pane.cursor.min(len);
            let caret = view.caret(editor, pane.cursor);
            let (a, c) = (pane.anchor.min(pane.cursor), pane.anchor.max(pane.cursor));
            let sel: Vec<EdRect> = if a == c {
                Vec::new()
            } else {
                view.selection(editor, a, c)
                    .into_iter()
                    .map(ed_rect)
                    .collect()
            };
            if scroll && pane.height > 0.0 {
                let top = if caret.y < pane.top {
                    Some((caret.y - CARET_ROOM).max(0.0))
                } else if caret.y + caret.h > pane.top + pane.height {
                    Some((caret.y + caret.h - pane.height + CARET_ROOM).max(0.0))
                } else {
                    None
                };
                if let Some(top) = top {
                    pane.top = top;
                    pane.scroll_seq = pane.scroll_seq.wrapping_add(1);
                }
            }
            // The pieces around the part in view.
            let h = if pane.height > 0.0 {
                pane.height
            } else {
                800.0
            };
            let (lo, hi) = (
                pane.top - h * MARGIN_VIEWS,
                pane.top + h * (1.0 + MARGIN_VIEWS),
            );
            // Only the selection's rectangles near the part in view (Ctrl+A
            // on a long message has thousands).
            let sel: Vec<EdRect> = sel
                .into_iter()
                .filter(|r| r.y + r.h >= lo && r.y <= hi)
                .collect();
            let pieces = view.pieces(lo, hi);
            let mut images = HashMap::with_capacity(pieces.len());
            let mut shown = Vec::with_capacity(pieces.len());
            for p in &pieces {
                let img = match pane.images.remove(&p.key) {
                    Some(img) => img,
                    None => match view.paint_piece(editor, p, engine) {
                        Some(pm) => image_of(&pm),
                        None => continue,
                    },
                };
                shown.push(EdPiece {
                    img: img.clone(),
                    y: p.y_px as f32 / view.scale(),
                    h: p.h_px as f32 / view.scale(),
                });
                images.insert(p.key, img);
            }
            pane.images = images;
            // The part of the projection the proxy holds (all of a short
            // message).
            let text_now = editor.text();
            let wanted = window_for(text_now, pane.anchor, pane.cursor, (pane.ws, pane.we));
            let (ws, we) = if pane.hold_window && pane.we <= text_now.len() {
                if wanted != (pane.ws, pane.we) && !pane.window_due {
                    pane.window_due = true;
                    rewindow.push(id);
                }
                (pane.ws, pane.we)
            } else {
                wanted
            };
            let part = text_now.get(ws..we).unwrap_or_default();
            let moved_window = (ws, we) != (pane.ws, pane.we);
            let push = push || moved_window || pane.pushed.as_str() != part;
            if push {
                pane.pushed = SharedString::from(part);
                pane.ws = ws;
                pane.we = we;
            }
            let (pws, pwe) = (pane.ws, pane.we);
            let rel = |o: usize| i32::try_from(o.clamp(pws, pwe) - pws).unwrap_or(i32::MAX);
            if push || select {
                pane.seq = pane.seq.wrapping_add(1);
                pane.set = Some((pane.anchor, pane.cursor));
            }
            let fmt = format_state(&editor.format_at(pane.anchor, pane.cursor), editor);
            let state = EdState {
                pieces: ModelRc::new(VecModel::from(shown)),
                page_h: view.height(),
                caret: ed_rect(caret),
                sel: ModelRc::new(VecModel::from(sel)),
                text: pane.pushed.clone(),
                push_text: push,
                anchor: rel(pane.anchor),
                cursor: rel(pane.cursor),
                seq: pane.seq,
                scroll_y: pane.top,
                scroll_seq: pane.scroll_seq,
                empty: is_blank(editor),
            };
            (state, fmt)
        };
        let rich = ui.global::<Rich>();
        if id == SIG {
            rich.set_sig(state);
            rich.set_sig_fmt(fmt);
        } else {
            rich.set_body(state);
            rich.set_body_fmt(fmt);
        }
        for (key, html, width, dark) in jobs {
            spawn_quote(id, key, html, width, dark, scale);
        }
        // Once the event that moved the caret is over.
        for id in rewindow {
            Timer::single_shot(Duration::ZERO, move || {
                with_app(|a| {
                    if let Some(p) = a.ed.panes.get_mut(slot(id)) {
                        p.window_due = false;
                    }
                    a.ed_show(id, false, false, false);
                });
            });
        }
    }

    /// A stored quote's layout arrived (none: it could not be laid out; its
    /// place keeps the waiting height).
    fn ed_quote_done(&mut self, id: i32, key: u64, doc: Option<rnd::Doc>) {
        let Some(doc) = doc else {
            return;
        };
        let Some((_, pane, _, _)) = self.ed_parts(id) else {
            return;
        };
        let shown = pane
            .view
            .as_mut()
            .is_some_and(|v| v.set_quote(key, Arc::new(doc)));
        pane.quotes.remove(&key);
        if shown {
            self.ed_show(id, false, false, false);
        }
    }

    /// The page's width or the part of it in view changed.
    pub(in crate::app) fn ed_geometry(&mut self, id: i32, width: f32, top: f32, height: f32) {
        let Some(pane) = self.ed.panes.get_mut(slot(id)) else {
            return;
        };
        let resized = (pane.width - width).abs() >= 0.5;
        let moved = (pane.top - top).abs() >= 0.5 || (pane.height - height).abs() >= 0.5;
        if !resized && !moved {
            return;
        }
        pane.width = width;
        pane.top = top.max(0.0);
        pane.height = height;
        self.ed_show(id, false, false, false);
    }

    /// Something in an editor changed the message: the draft is saved soon
    /// (or the signature is marked changed).
    fn ed_changed(&mut self, id: i32) {
        if id == SIG {
            self.sig_dirty = true;
            return;
        }
        if let Some(d) = self.drafts.active_mut() {
            d.touched = true;
            d.dirty = true;
        }
        self.ui().set_c_guard(SharedString::new());
        self.schedule_autosave();
    }

    /// The proxy's text changed (typing, Backspace, a plain-text paste or
    /// drop, IME).
    pub(in crate::app) fn ed_edited(&mut self, id: i32, text: &str, _anchor: i32, cursor: i32) {
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return;
        };
        // The proxy's part, back in the whole projection.
        let old = editor.text();
        let old_len = old.len();
        let (ws, we) = (
            pane.ws.min(old_len),
            pane.we.min(old_len).max(pane.ws.min(old_len)),
        );
        let full = if (ws, we) == (0, old_len) {
            text.to_string()
        } else {
            let (Some(head), Some(tail)) = (old.get(..ws), old.get(we..)) else {
                return;
            };
            [head, text, tail].concat()
        };
        let cursor = usize::try_from(cursor).ok().map(|c| c + ws);
        let (caret, changed) = match editor.apply(&full, cursor) {
            Some(mut caret) => {
                // A space typed after "- ", "* " or "1." at a line start.
                let typed_space = full.len() == old_len + 1
                    && caret > 0
                    && full.as_bytes().get(caret - 1) == Some(&b' ');
                if typed_space && let Some(c) = editor.autoformat(caret) {
                    caret = c;
                }
                (caret, true)
            }
            // Refused: the projection goes back, the caret where it was.
            None => (pane.cursor, false),
        };
        pane.anchor = caret;
        pane.cursor = caret;
        pane.goal_x = None;
        // The proxy holds `text` over the window, which grew or shrank by
        // what the edit did; a difference is pushed by `ed_show`.
        pane.pushed = SharedString::from(text);
        pane.ws = ws;
        pane.we = (we + full.len()).saturating_sub(old_len);
        if changed {
            self.ed_changed(id);
        }
        self.ed_show(id, false, !changed, true);
    }

    /// The proxy's selection moved (arrows, Shift+arrows, Ctrl+A, a
    /// selection set from here).
    pub(in crate::app) fn ed_moved(&mut self, id: i32, anchor: i32, cursor: i32) {
        let (Ok(a), Ok(c)) = (usize::try_from(anchor), usize::try_from(cursor)) else {
            return;
        };
        let Some((_, pane, _, _)) = self.ed_parts(id) else {
            return;
        };
        // Offsets in the proxy's part of the projection.
        let (a, c) = (a + pane.ws, c + pane.ws);
        if (pane.anchor, pane.cursor) == (a, c) {
            return;
        }
        if pane.set.take() != Some((a, c)) {
            pane.goal_x = None;
        }
        pane.anchor = a;
        pane.cursor = c;
        pane.hold_window = true;
        self.ed_show(id, false, false, true);
        if let Some(p) = self.ed.panes.get_mut(slot(id)) {
            p.hold_window = false;
        }
    }

    /// A key the page handles. False: the proxy (or the focus chain) gets
    /// it.
    pub(in crate::app) fn ed_key(&mut self, id: i32, key: EdKey, shift: bool) -> bool {
        match key {
            EdKey::Paste => return self.ed_paste(id),
            EdKey::Undo | EdKey::Redo => {
                let Some((editor, pane, _, _)) = self.ed_parts(id) else {
                    return false;
                };
                let sel = if key == EdKey::Undo {
                    editor.undo()
                } else {
                    editor.redo()
                };
                if let Some((a, c)) = sel {
                    pane.anchor = a;
                    pane.cursor = c;
                    self.ed_changed(id);
                    self.ed_show(id, true, true, true);
                }
                return true;
            }
            _ => {}
        }
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return false;
        };
        let (a, c) = (pane.anchor, pane.cursor);
        if matches!(key, EdKey::Tab | EdKey::BackTab) {
            let Some((na, nc)) = editor.tab(a, c, key == EdKey::BackTab) else {
                return false;
            };
            pane.anchor = na;
            pane.cursor = nc;
            pane.goal_x = None;
            self.ed_changed(id);
            self.ed_show(id, true, true, true);
            return true;
        }
        let (goal_x, height) = (pane.goal_x, pane.height);
        let Some(view) = pane.view.as_ref() else {
            return false;
        };
        let (to, goal) = match key {
            EdKey::Up | EdKey::Down | EdKey::PageUp | EdKey::PageDown => {
                let goal = goal_x.unwrap_or_else(|| view.caret(editor, c).x);
                let down = matches!(key, EdKey::Down | EdKey::PageDown);
                let page = if matches!(key, EdKey::PageUp | EdKey::PageDown) {
                    (height - 24.0).max(24.0)
                } else {
                    0.0
                };
                (view.vertical(editor, c, goal, down, page), Some(goal))
            }
            EdKey::Home | EdKey::End => (view.line_edge(editor, c, key == EdKey::End), None),
            // The message's start and end (the proxy may hold only part).
            EdKey::DocStart => (0, None),
            EdKey::DocEnd => (editor.text().len(), None),
            EdKey::SelectAll => {
                pane.anchor = 0;
                pane.cursor = editor.text().len();
                pane.goal_x = None;
                self.ed_show(id, false, true, false);
                return true;
            }
            _ => return false,
        };
        pane.goal_x = goal;
        pane.cursor = to;
        if !shift {
            pane.anchor = to;
        }
        self.ed_show(id, false, true, true);
        true
    }

    /// A press, Shift+press, drag or double click on the page at `x, y`.
    pub(in crate::app) fn ed_point(&mut self, id: i32, x: f32, y: f32, kind: i32) {
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return;
        };
        let Some(view) = pane.view.as_ref() else {
            return;
        };
        let at = view.hit(editor, x, y);
        match kind {
            1 => pane.cursor = at,
            2 => {
                pane.anchor = pane.drag_from;
                pane.cursor = at;
            }
            3 => {
                let (a, b) = view.word(editor, at);
                pane.anchor = a;
                pane.cursor = b;
            }
            _ => {
                pane.anchor = at;
                pane.cursor = at;
                pane.drag_from = at;
            }
        }
        pane.goal_x = None;
        self.ed_show(id, false, true, kind == 2);
    }

    /// A toolbar button or a formatting key.
    pub(in crate::app) fn ed_act(&mut self, id: i32, action: EdAction, arg: i32) {
        if action == EdAction::Color {
            self.ui().global::<Rich>().set_last_color(arg);
        }
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return;
        };
        let (a, c) = (pane.anchor, pane.cursor);
        let mut caret = None;
        let changed = match action {
            EdAction::Bold => editor.format_chars(a, c, &CharOp::Bold),
            EdAction::Italic => editor.format_chars(a, c, &CharOp::Italic),
            EdAction::Underline => editor.format_chars(a, c, &CharOp::Underline),
            EdAction::Strike => editor.format_chars(a, c, &CharOp::Strike),
            EdAction::Font => match pick(&FontId::ALL, arg) {
                Some(f) => editor.format_chars(a, c, &CharOp::Font(f)),
                None => false,
            },
            EdAction::Size => match pick(&SizePt::ALL, arg).and_then(SizePt::new) {
                Some(s) => editor.format_chars(a, c, &CharOp::Size(s)),
                None => false,
            },
            EdAction::SizeUp => editor.format_chars(a, c, &CharOp::SizeStep(true)),
            EdAction::SizeDown => editor.format_chars(a, c, &CharOp::SizeStep(false)),
            EdAction::Color => editor.format_chars(a, c, &CharOp::Color(pick(&ColorId::ALL, arg))),
            EdAction::Bullets => editor.format_paras(a, c, ParaOp::List(ListKind::Bullet)),
            EdAction::Numbers => editor.format_paras(a, c, ParaOp::List(ListKind::Number)),
            EdAction::Indent => editor.format_paras(a, c, ParaOp::Indent(true)),
            EdAction::Outdent => editor.format_paras(a, c, ParaOp::Indent(false)),
            EdAction::Align => {
                let al = match arg {
                    1 => Align::Center,
                    2 => Align::Right,
                    3 => Align::Justify,
                    _ => Align::Left,
                };
                editor.format_paras(a, c, ParaOp::Align(al))
            }
            EdAction::Quote => editor.format_paras(a, c, ParaOp::Quote),
            EdAction::Clear => editor.clear_formatting(a, c),
            EdAction::RowAbove
            | EdAction::RowBelow
            | EdAction::ColLeft
            | EdAction::ColRight
            | EdAction::DeleteRow
            | EdAction::DeleteCol
            | EdAction::DeleteTable => {
                let op = match action {
                    EdAction::RowAbove => TableOp::RowAbove,
                    EdAction::RowBelow => TableOp::RowBelow,
                    EdAction::ColLeft => TableOp::ColLeft,
                    EdAction::ColRight => TableOp::ColRight,
                    EdAction::DeleteRow => TableOp::DeleteRow,
                    EdAction::DeleteCol => TableOp::DeleteCol,
                    _ => TableOp::DeleteTable,
                };
                caret = editor.table_op(c, op);
                caret.is_some()
            }
            EdAction::Undo => editor.undo().is_some_and(|(x, y)| {
                caret = Some(y);
                pane.anchor = x;
                true
            }),
            EdAction::Redo => editor.redo().is_some_and(|(x, y)| {
                caret = Some(y);
                pane.anchor = x;
                true
            }),
            EdAction::ShowQuote => editor.fold_quote(false),
            EdAction::EditQuote => editor.expand_quote(),
        };
        if let Some(c) = caret {
            if !matches!(action, EdAction::Undo | EdAction::Redo) {
                pane.anchor = c;
            }
            pane.cursor = c;
        }
        if changed {
            self.ed_changed(id);
        }
        self.ed_show(id, false, true, caret.is_some());
    }

    /// The link field's address on the selection (or inserted at the
    /// caret). False: not a web or e-mail address.
    pub(in crate::app) fn ed_link(&mut self, id: i32, url: &str) -> bool {
        let Some(target) = model::link_target(url) else {
            return false;
        };
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return false;
        };
        if let Some((a, c)) = editor.link(pane.anchor, pane.cursor, &target, "") {
            pane.anchor = a;
            pane.cursor = c;
            self.ed_changed(id);
        }
        self.ed_show(id, false, true, true);
        true
    }

    /// Insert a `rows` x `cols` table at the caret.
    pub(in crate::app) fn ed_insert_table(&mut self, id: i32, rows: i32, cols: i32) {
        let (Ok(r), Ok(c)) = (usize::try_from(rows), usize::try_from(cols)) else {
            return;
        };
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return;
        };
        if let Some(caret) = editor.insert_table(pane.cursor, r, c) {
            pane.anchor = caret;
            pane.cursor = caret;
            self.ed_changed(id);
        }
        self.ed_show(id, false, true, true);
    }

    /// Ctrl+V: formatted text from the clipboard (sanitized, then what the
    /// model holds of it). False: there is none, and the proxy pastes the
    /// plain text.
    fn ed_paste(&mut self, id: i32) -> bool {
        let Some(frag) = files::clipboard_html()
            .and_then(|h| mail::compose_html::sanitize(&h))
            .and_then(|h| model::import::lossy(&h))
        else {
            return false;
        };
        let Some((editor, pane, _, _)) = self.ed_parts(id) else {
            return false;
        };
        let Some(caret) = editor.paste(pane.anchor, pane.cursor, frag) else {
            return false;
        };
        pane.anchor = caret;
        pane.cursor = caret;
        self.ed_changed(id);
        self.ed_show(id, true, true, true);
        true
    }
}

/// Lay out a stored quote (the full renderer) on a thread of its own.
fn spawn_quote(id: i32, key: u64, html: String, width: f32, dark: bool, scale: f32) {
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
        on_ui(Box::new(move |a| a.ed_quote_done(id, key, doc)));
    };
    std::thread::Builder::new()
        .name("quote".into())
        .stack_size(QUOTE_STACK)
        .spawn(run)
        .ok();
}

/// Connect `Rich`'s calls.
pub(in crate::app) fn wire(ui: &AppWindow) {
    let rich = ui.global::<Rich>();
    rich.on_edited(|id, text, anchor, cursor| {
        with_app(|a| a.ed_edited(id, &text, anchor, cursor));
    });
    rich.on_moved(|id, anchor, cursor| {
        with_app(|a| a.ed_moved(id, anchor, cursor));
    });
    rich.on_key(|id, key, shift| with_app(|a| a.ed_key(id, key, shift)).unwrap_or(false));
    rich.on_point(|id, x, y, kind| {
        with_app(|a| a.ed_point(id, x, y, kind));
    });
    rich.on_geometry(|id, w, top, h| {
        with_app(|a| a.ed_geometry(id, w, top, h));
    });
    rich.on_restyle(|id| {
        with_app(|a| a.ed_show(id, false, false, false));
    });
    rich.on_act(|id, action, arg| {
        with_app(|a| a.ed_act(id, action, arg));
    });
    rich.on_link(|id, url| with_app(|a| a.ed_link(id, &url)).unwrap_or(false));
    rich.on_insert_table(|id, rows, cols| {
        with_app(|a| a.ed_insert_table(id, rows, cols));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proxy_holds_a_window_of_a_long_message() {
        let short = "a\nb\n".repeat(100);
        assert_eq!(window_for(&short, 3, 3, (0, 0)), (0, short.len()));
        let line = "0123456789 abcdefghi\n";
        let long = line.repeat(10_000);
        let mid = long.len() / 2;
        let (ws, we) = window_for(&long, mid, mid, (0, 0));
        assert!(ws < mid && mid < we && we - ws <= WINDOW + 2 * WINDOW_SNAP);
        // Whole lines.
        assert!(ws == 0 || long.as_bytes()[ws - 1] == b'\n');
        assert!(we == long.len() || long.as_bytes()[we] == b'\n');
        // Kept while the caret stays clear of its edges...
        assert_eq!(window_for(&long, mid + 100, mid + 100, (ws, we)), (ws, we));
        // ...moved near one, and stretched over a whole selection.
        let near = we - 10;
        let (ws2, we2) = window_for(&long, near, near, (ws, we));
        assert!(ws2 > ws && we2 > we);
        let (ws3, we3) = window_for(&long, 10, long.len() - 10, (ws, we));
        assert!(ws3 <= 10 && we3 >= long.len() - 10);
        // One huge line is cut at character boundaries.
        let one = "ação ".repeat(20_000);
        let (a, b) = window_for(&one, one.len() / 2 + 1, one.len() / 2 + 1, (0, 0));
        assert!(one.is_char_boundary(a) && one.is_char_boundary(b) && a < b);
    }
}
