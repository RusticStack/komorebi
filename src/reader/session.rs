//! The reader's UI-side state machine, with no toolkit in it: which message
//! is shown and under which [`Key`], the tiles wanted near the view, the
//! selection gestures, the links (hover, Tab focus, the confirm prompt
//! before a web link opens), remote-image consent and "show original
//! colours". The view feeds it what it measures ([`Frame`]) and what the
//! user does, sends nothing itself, and after each call takes the
//! [`Effect`]s (scroll to, copy, compose, the two timers) and the tile
//! images to release ([`Session::take_dropped`]).
//!
//! It is the same logic as the Slint app's `src/app/reader.rs` (`State`),
//! which keeps its own copy until the cut-over; the tests below are that
//! file's view tests, without the window.
//!
//! `T` is the view's tile image (a GPUI `Arc<RenderImage>`): the session
//! only stores, replaces and hands them back.

use super::{
    Backend, Job, KEEP, Key, LineRange, MAX_SHOWN_URL, MULTI_CLICK, MULTI_CLICK_SLOP, Page,
    PageLink, Pixels, Reply, SelectOp, Shell, Source, TILE_H, page_width, tile_count, tile_range,
    tile_span, wanted_tiles,
};
use crate::mail::mime::Body;
use crate::mail::render::links::{self, Mailto, Warning};
use crate::mail::render::{Rect, RenderError, TextLine};
use std::time::Instant;

/// What the view measured: the body column, the device scale, the theme,
/// and the visible band of the page (page coordinates: `top` is negative
/// while the header above the page is in view).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The body column's width in logical px (the page's width comes
    /// from it, `page_width`).
    pub column: f32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// The app shows the dark theme.
    pub dark: bool,
    pub top: f32,
    pub height: f32,
}

impl Default for Frame {
    fn default() -> Self {
        Frame {
            column: 0.0,
            scale: 1.0,
            dark: false,
            top: 0.0,
            height: 0.0,
        }
    }
}

/// What the view does after a call.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Scroll so that page y `top` is at the top of the view (as far as
    /// the content allows).
    ScrollTo(f32),
    /// Put the selected text on the clipboard.
    Copy(String),
    /// A clicked `mailto:` link: a prefilled composer.
    Mailto(Mailto),
    /// Call [`Session::fallback_due`] with this token after
    /// [`super::FALLBACK_AFTER`].
    Fallback(u64),
    /// Call [`Session::resize_due`] with this token after
    /// [`super::RESIZE_DEBOUNCE`] (a newer token replaces it).
    Debounce(u64),
}

/// A web link waiting in the confirm prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    /// The URL split around its host, each part at most [`MAX_SHOWN_URL`]
    /// characters.
    pub before: String,
    pub host: String,
    pub after: String,
    /// The link's shown text (for the text-mismatch warning).
    pub text: String,
    pub warnings: Vec<Warning>,
    /// What opens.
    url: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TileState {
    Missing,
    Requested,
    Shown,
}

/// One tile of the page shown.
#[derive(Clone, Debug)]
pub struct Tile<T> {
    /// The image (a relayout keeps the old one, stretched, until the new
    /// one arrives).
    pub image: Option<T>,
    /// Logical height (the last tile may be shorter than `TILE_H`).
    pub height: f32,
    state: TileState,
}

/// The reader's UI-side state.
pub struct Session<T> {
    backend: Box<dyn Backend>,
    shell: Box<dyn Shell>,
    frame: Frame,
    generation: u64,
    /// The message shown and its source, kept for relayouts.
    src: Option<(i64, Source)>,
    /// What the latest layout job was for.
    key: Option<Key>,
    /// The latest job is a relayout of the page shown, whose tiles stay up
    /// (stretched) until new ones replace them.
    relayout: bool,
    page: Option<Page>,
    tiles: Vec<Tile<T>>,
    dropped: Vec<T>,
    /// A layout was sent without a page up: the text view stands in
    /// unless a page arrives before [`super::FALLBACK_AFTER`].
    fallback: Option<u64>,
    resize: u64,
    waiting: bool,
    html: bool,
    /// "View as plain text": the text view instead of the page.
    plain: bool,
    /// Why the formatted view is not shown, while its note is.
    failed: Option<RenderError>,
    /// The sideways scroll of a page wider than the column (0 or less).
    page_x: f32,
    /// Highlight rectangles of the selection on the page shown.
    selection: Vec<Rect>,
    /// The selection in the page's lines, for screen readers.
    text_selection: Option<LineRange>,
    /// The page's lines of text, for screen readers.
    lines: Vec<TextLine>,
    /// The last button press (time, position, click count), to tell double
    /// and triple clicks.
    last_press: Option<(Instant, f32, f32, u8)>,
    links: Vec<PageLink>,
    hover: Option<usize>,
    /// The link with keyboard focus (Tab).
    focus: Option<usize>,
    /// The link under the last single press, opened if the button comes up
    /// on it without a drag.
    pressed_link: Option<usize>,
    prompt: Option<Prompt>,
    /// Remote images may load for the message shown.
    remote: bool,
    /// The message shown keeps its original colours in the dark theme.
    original: bool,
    effects: Vec<Effect>,
    /// When the message was opened, for `KOMOREBI_TIMING`.
    opened: Option<Instant>,
    timing: bool,
}

impl<T: Clone> Session<T> {
    pub fn new(backend: Box<dyn Backend>, shell: Box<dyn Shell>) -> Self {
        Session {
            backend,
            shell,
            frame: Frame::default(),
            generation: 0,
            src: None,
            key: None,
            relayout: false,
            page: None,
            tiles: Vec::new(),
            dropped: Vec::new(),
            fallback: None,
            resize: 0,
            waiting: false,
            html: false,
            plain: false,
            failed: None,
            page_x: 0.0,
            selection: Vec::new(),
            text_selection: None,
            lines: Vec::new(),
            last_press: None,
            links: Vec::new(),
            hover: None,
            focus: None,
            pressed_link: None,
            prompt: None,
            remote: false,
            original: false,
            effects: Vec::new(),
            opened: None,
            timing: crate::dev_var("KOMOREBI_TIMING").is_some(),
        }
    }

    // ------------------------------------------------------- reading

    /// The message shown (or being laid out).
    pub fn message(&self) -> Option<i64> {
        self.src.as_ref().map(|(m, _)| *m)
    }

    /// The page is laid out and shown (else the text view, or nothing
    /// while [`Session::waiting`]).
    pub fn html(&self) -> bool {
        self.html
    }

    /// A render is pending: neither the page nor the text shows yet.
    pub fn waiting(&self) -> bool {
        self.waiting
    }

    /// The page shown is the first look at a long message; the whole of it
    /// is still being laid out.
    pub fn partial(&self) -> bool {
        self.page.is_some_and(|p| p.partial)
    }

    pub fn plain(&self) -> bool {
        self.plain
    }

    /// The page is drawn: laid out, and not in the plain-text view.
    pub fn page_shown(&self) -> bool {
        self.html && !self.plain
    }

    /// The text view stands in (failed render, plain text, or no HTML).
    pub fn text_shown(&self) -> bool {
        self.plain || (!self.html && !self.waiting)
    }

    pub fn page(&self) -> Option<&Page> {
        self.page.as_ref()
    }

    pub fn key(&self) -> Option<Key> {
        self.key
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn tiles(&self) -> &[Tile<T>] {
        &self.tiles
    }

    pub fn selection(&self) -> &[Rect] {
        &self.selection
    }

    /// Where the selection starts and ends in [Session::lines], as (line,
    /// character).
    pub fn text_selection(&self) -> Option<LineRange> {
        self.text_selection
    }

    /// The page's lines of text in view coordinates (with a worker that
    /// sends them, `Worker::with_text_lines`).
    pub fn lines(&self) -> &[TextLine] {
        &self.lines
    }

    pub fn links(&self) -> &[PageLink] {
        &self.links
    }

    pub fn page_x(&self) -> f32 {
        self.page_x
    }

    /// Why the formatted view failed, for the note above the text.
    pub fn failed(&self) -> Option<RenderError> {
        self.failed
    }

    /// Remote images blocked (the banner offers to load them).
    pub fn remote_blocked(&self) -> u32 {
        match &self.page {
            Some(p) if !self.remote => p.remote_images,
            _ => 0,
        }
    }

    /// After consent: the images that could not be loaded, once none is
    /// still coming.
    pub fn remote_failed(&self) -> u32 {
        match &self.page {
            Some(p) if self.remote && !p.pending_images => p.failed_images,
            _ => 0,
        }
    }

    /// Remote images may load for the message shown.
    pub fn remote(&self) -> bool {
        self.remote
    }

    pub fn dimmed(&self) -> bool {
        self.page.is_some_and(|p| p.dimmed)
    }

    pub fn original(&self) -> bool {
        self.original
    }

    pub fn truncated(&self) -> bool {
        self.page.is_some_and(|p| p.truncated)
    }

    /// The pointer is over a link (pointer cursor).
    pub fn hover_link(&self) -> bool {
        self.hover.is_some()
    }

    /// The bottom strip: the hovered link, else the focused one.
    pub fn target(&self) -> String {
        let href = self
            .hover
            .or(self.focus)
            .and_then(|i| self.links.get(i))
            .map_or("", |l| l.href.as_str());
        href.chars().take(MAX_SHOWN_URL).collect()
    }

    pub fn focused_link(&self) -> Option<usize> {
        self.focus
    }

    /// The focused link's ring, around each of its hit areas.
    pub fn focus_rects(&self) -> Vec<Rect> {
        self.focus
            .and_then(|i| self.links.get(i))
            .map_or_else(Vec::new, |l| {
                l.rects
                    .iter()
                    .map(|r| Rect {
                        x: r.x - 2.0,
                        y: r.y - 1.0,
                        w: r.w + 4.0,
                        h: r.h + 2.0,
                    })
                    .collect()
            })
    }

    /// The web link waiting in the confirm prompt.
    pub fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }

    /// What the view does next.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Tile images no longer shown (release them from the GPU).
    pub fn take_dropped(&mut self) -> Vec<T> {
        std::mem::take(&mut self.dropped)
    }

    // ------------------------------------------------------- opening

    /// Load fonts on the worker (once the window is up).
    pub fn warm(&mut self) {
        self.backend.send(Job::Warm);
    }

    /// Render message `msg` from `src`, with its remote images if `remote`
    /// (the sender is on the allowlist). Consent given for the message
    /// shown lasts while it stays open. The body stays blank until the page
    /// arrives, or until the text view stands in after a second.
    pub fn open(&mut self, msg: i64, src: Source, remote: bool) {
        let same = self.message() == Some(msg);
        self.remote = remote || same && self.remote;
        self.original = same && self.original;
        if !same {
            // "View as plain text" lasts for one message.
            self.plain = false;
        }
        self.src = Some((msg, src));
        self.key = None;
        self.opened = Some(Instant::now());
        self.show_waiting();
        self.layout(false);
    }

    /// Load the remote images of the message shown: lay it out again with
    /// them, keeping the current page up until the new one arrives.
    pub fn allow_remote(&mut self) {
        if self.src.is_none() || self.remote {
            return;
        }
        self.remote = true;
        let shown = self.page.is_some();
        self.layout(shown);
    }

    /// Show the text view; renders in flight are dropped.
    pub fn close(&mut self) {
        self.src = None;
        self.remote = false;
        self.original = false;
        self.plain = false;
        self.failed = None;
        self.key = None;
        self.page = None;
        self.generation += 1;
        let generation = self.generation;
        self.backend.send(Job::Cancel { generation });
        self.fallback = None;
        self.resize += 1;
        self.clear_tiles();
        self.selection.clear();
        self.text_selection = None;
        self.lines.clear();
        self.set_links(Vec::new(), false);
        self.cancel_prompt();
        self.html = false;
        self.waiting = false;
    }

    /// Handle a worker reply; returns the body parsed by the worker for the
    /// message shown. `tile` turns tile pixels into the view's image.
    pub fn on_reply(&mut self, reply: Reply, tile: impl FnOnce(Pixels) -> T) -> Option<Body> {
        match reply {
            Reply::Parsed { generation, body } => (generation == self.generation).then_some(body),
            Reply::Page {
                generation,
                page,
                links,
            } => {
                if generation == self.generation {
                    self.show_page(page, links);
                }
                None
            }
            Reply::Failed { generation, error } => {
                if generation == self.generation {
                    self.show_failed(error);
                }
                None
            }
            Reply::Tile {
                generation,
                index,
                pixels,
            } => {
                if generation == self.generation {
                    self.show_tile(generation, index, tile(pixels));
                }
                None
            }
            Reply::Text { generation, lines } => {
                if generation == self.generation {
                    self.lines = lines;
                }
                None
            }
            Reply::Selection { generation, rects } => {
                if generation == self.generation {
                    self.selection = rects;
                }
                None
            }
            Reply::TextSelection { generation, range } => {
                if generation == self.generation {
                    self.text_selection = range;
                }
                None
            }
            Reply::Copied { generation, text } => {
                if generation == self.generation {
                    self.effects.push(Effect::Copy(text));
                }
                None
            }
            // Lay the page out again with the images now in.
            Reply::Fetched { msg } => {
                if self.remote && self.message() == Some(msg) {
                    let shown = self.page.is_some();
                    self.layout(shown);
                }
                None
            }
        }
    }

    /// The view's tile images can no longer be trusted to hold their pixels
    /// (the GPU device was lost, or the images dropped their CPU copy once
    /// uploaded): ask the worker for the tiles near the view again. Each
    /// image stays up, stretched, until its replacement arrives, and goes to
    /// [`Session::take_dropped`] then; the worker answers from its tile cache
    /// when it can, so nothing is rasterized twice.
    pub fn refresh_tiles(&mut self) {
        for tile in &mut self.tiles {
            if tile.state == TileState::Shown {
                tile.state = TileState::Missing;
            }
        }
        self.request_tiles();
    }

    /// A tile arrived (already the view's image).
    pub fn show_tile(&mut self, generation: u64, index: u32, image: T) {
        if generation != self.generation {
            self.dropped.push(image);
            return;
        }
        let Some(tile) = self.tiles.get_mut(index as usize) else {
            self.dropped.push(image);
            return;
        };
        if let Some(old) = tile.image.replace(image) {
            self.dropped.push(old);
        }
        tile.state = TileState::Shown;
        if let Some(opened) = self.opened.take()
            && self.timing
        {
            eprintln!(
                "[timing] open to first tile: {:.1} ms",
                opened.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    // ------------------------------------------------------- geometry

    /// What the view measured. A new width, scale or theme lays the page
    /// out again once resizing pauses; a scroll asks for the tiles near
    /// the view and drops the ones far from it.
    pub fn set_frame(&mut self, frame: Frame) {
        if frame == self.frame {
            return;
        }
        let resized = (frame.column, frame.scale, frame.dark)
            != (self.frame.column, self.frame.scale, self.frame.dark);
        self.frame = frame;
        if resized {
            self.body_resized();
        }
        self.request_tiles();
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    /// The key a layout would use now; `None` until the body column has
    /// reported its width.
    fn current_key(&self) -> Option<Key> {
        let (msg, _) = self.src.as_ref()?;
        let scale = self.frame.scale;
        let dark = self.frame.dark;
        Some(Key {
            msg: *msg,
            width: page_width(self.frame.column)?,
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            }
            .to_bits(),
            dark,
            dim: dark && !self.original,
            remote: self.remote,
        })
    }

    fn show_waiting(&mut self) {
        self.failed = None;
        self.page = None;
        self.relayout = false;
        self.clear_tiles();
        self.selection.clear();
        self.text_selection = None;
        self.lines.clear();
        self.last_press = None;
        self.set_links(Vec::new(), false);
        self.cancel_prompt();
        self.page_x = 0.0;
        self.html = false;
        self.waiting = true;
    }

    fn clear_tiles(&mut self) {
        self.dropped
            .extend(self.tiles.drain(..).filter_map(|t| t.image));
    }

    /// Send a layout job for the current key; `relayout` keeps the page
    /// shown until the new one arrives.
    fn layout(&mut self, relayout: bool) {
        let (Some(key), Some((_, src))) = (self.current_key(), self.src.clone()) else {
            return;
        };
        self.generation += 1;
        self.key = Some(key);
        self.relayout = relayout;
        let generation = self.generation;
        self.backend.send(Job::Layout {
            generation,
            key,
            src,
        });
        if !relayout {
            self.fallback = Some(generation);
            self.effects.push(Effect::Fallback(generation));
        }
    }

    /// [`super::FALLBACK_AFTER`] passed since the layout of `token`: with no page
    /// yet, the text view stands in.
    pub fn fallback_due(&mut self, token: u64) {
        if self.fallback == Some(token) {
            self.fallback = None;
            if self.page.is_none() {
                self.waiting = false;
            }
        }
    }

    /// The body column, the scale or the theme changed: lay out again once
    /// resizing pauses, or at once if the first width just arrived.
    fn body_resized(&mut self) {
        if self.src.is_none() {
            return;
        }
        let Some(key) = self.key else {
            self.layout(false);
            return;
        };
        self.resize += 1;
        if self.current_key() == Some(key) {
            return;
        }
        self.effects.push(Effect::Debounce(self.resize));
    }

    /// The resize debounce of `token` ran out: lay out at the new width,
    /// scale or theme.
    pub fn resize_due(&mut self, token: u64) {
        if token == self.resize
            && self.src.is_some()
            && self.key.is_some()
            && self.current_key() != self.key
        {
            let shown = self.page.is_some();
            self.layout(shown);
        }
    }

    /// "Show original colours" for the message shown (dark theme): lay it
    /// out again without (or with) dimming, keeping the page up meanwhile.
    pub fn show_original(&mut self, on: bool) {
        if self.src.is_none() || self.original == on {
            return;
        }
        self.original = on;
        let shown = self.page.is_some();
        self.layout(shown);
    }

    /// "View as plain text" on or off; the page's tiles wait while it is.
    pub fn set_plain(&mut self, on: bool) {
        if self.plain != on {
            self.plain = on;
            self.request_tiles();
        }
    }

    /// Scroll sideways (a page wider than the column), clamped.
    pub fn set_page_x(&mut self, x: f32) {
        let min_x = self
            .page
            .map_or(0.0, |p| (self.frame.column - p.width).min(0.0));
        self.page_x = x.clamp(min_x, 0.0);
    }

    fn show_page(&mut self, page: Page, links: Vec<PageLink>) {
        let n = tile_count(page.height) as usize;
        // The whole of a long message replaces its first look as a relayout
        // does: the same top, its tiles kept up until drawn again.
        let relayout = self.relayout || self.partial();
        let mut old: Vec<Option<T>> = if relayout {
            self.tiles.drain(..).map(|t| t.image).collect()
        } else {
            self.clear_tiles();
            Vec::new()
        };
        if old.len() > n {
            self.dropped.extend(old.drain(n..).flatten());
        }
        old.resize(n, None);
        self.tiles = old
            .into_iter()
            .enumerate()
            .map(|(i, image)| Tile {
                image,
                height: tile_span(i as u32, page.height).map_or(TILE_H, |(t, b)| b - t),
                state: TileState::Missing,
            })
            .collect();
        // Positions on the old layout mean nothing on the new one.
        self.selection.clear();
        self.text_selection = None;
        // A relayout keeps the focused link (the same links, moved).
        self.set_links(links, relayout);
        self.page = Some(page);
        self.fallback = None;
        // A relayout keeps the sideways scroll only as far as it goes.
        self.set_page_x(self.page_x);
        self.waiting = false;
        self.html = true;
        self.failed = None;
        self.request_tiles();
    }

    fn show_failed(&mut self, error: RenderError) {
        self.failed = Some(error);
        self.page = None;
        self.fallback = None;
        self.clear_tiles();
        self.selection.clear();
        self.text_selection = None;
        self.lines.clear();
        self.set_links(Vec::new(), false);
        self.html = false;
        self.waiting = false;
    }

    /// Ask for the tiles near the view that are not there yet, and drop
    /// the ones far from it.
    fn request_tiles(&mut self) {
        let (Some(page), Some(key)) = (self.page, self.key) else {
            return;
        };
        // The plain-text view hides the page; its tiles wait until it is back.
        if self.plain {
            return;
        }
        // A scale change shows first as a scroll or resize.
        if self.current_key() != Some(key) {
            self.body_resized();
        }
        let (top, height) = (self.frame.top, self.frame.height);
        let keep = tile_range(
            top - KEEP * height,
            top + (1.0 + KEEP) * height,
            page.height,
        );
        for (i, tile) in self.tiles.iter_mut().enumerate() {
            let near = keep.is_some_and(|(a, b)| (a as usize..=b as usize).contains(&i));
            if !near && tile.state != TileState::Missing {
                tile.state = TileState::Missing;
                if let Some(img) = tile.image.take() {
                    self.dropped.push(img);
                }
            }
        }
        let wanted: Vec<u32> = wanted_tiles(top, height, page.height)
            .into_iter()
            .filter(|&i| match self.tiles.get_mut(i as usize) {
                Some(t) if t.state == TileState::Missing => {
                    t.state = TileState::Requested;
                    true
                }
                _ => false,
            })
            .collect();
        if !wanted.is_empty() {
            let generation = self.generation;
            self.backend.send(Job::Tiles {
                generation,
                key,
                tiles: wanted,
            });
        }
    }

    // ------------------------------------------------------- pointer

    /// A button press on the page at `now`: a caret, or a word or paragraph
    /// when it follows the previous press closely enough.
    pub fn press(&mut self, x: f32, y: f32, now: Instant) {
        let clicks = match self.last_press {
            Some((t, px, py, n))
                if now.duration_since(t) < MULTI_CLICK
                    && (x - px).abs() <= MULTI_CLICK_SLOP
                    && (y - py).abs() <= MULTI_CLICK_SLOP =>
            {
                n % 3 + 1
            }
            _ => 1,
        };
        self.last_press = Some((now, x, y, clicks));
        self.pressed_link = if clicks == 1 {
            self.link_at(x, y)
        } else {
            None
        };
        self.select(SelectOp::Press { x, y, clicks });
    }

    /// The pointer moved with the button down (or Shift+click): extend the
    /// selection. A drag past the click slop is no longer a link click.
    pub fn extend(&mut self, x: f32, y: f32) {
        let near = self.last_press.is_some_and(|(_, px, py, _)| {
            (x - px).abs() <= MULTI_CLICK_SLOP && (y - py).abs() <= MULTI_CLICK_SLOP
        });
        if !near {
            self.pressed_link = None;
        }
        self.select(SelectOp::Extend { x, y });
    }

    /// The button came up: a click on the link it went down on (with no
    /// drag in between, see `extend`) opens it.
    pub fn release(&mut self, x: f32, y: f32) {
        let Some(i) = self.pressed_link.take() else {
            return;
        };
        if self.link_at(x, y) == Some(i) {
            self.activate(i);
        }
    }

    /// The pointer moved over the page (`None`: it left). True when the
    /// hovered link changed.
    pub fn hover_at(&mut self, at: Option<(f32, f32)>) -> bool {
        let hover = at.and_then(|(x, y)| self.link_at(x, y));
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    /// Send a selection gesture or command for the page shown.
    pub fn select(&mut self, op: SelectOp) {
        let (Some(key), Some(_)) = (self.key, self.page) else {
            return;
        };
        let generation = self.generation;
        self.backend.send(Job::Select {
            generation,
            key,
            op,
        });
    }

    /// Escape on the page: the selection and the link focus go.
    pub fn clear(&mut self) {
        self.select(SelectOp::Clear);
        self.set_focus(None);
    }

    // ------------------------------------------------------- links

    /// Show `links` for the page; `keep_focus` keeps the focused link when
    /// the set is the same.
    fn set_links(&mut self, links: Vec<PageLink>, keep_focus: bool) {
        let same = keep_focus
            && links.len() == self.links.len()
            && links.iter().zip(&self.links).all(|(a, b)| a.href == b.href);
        let focus = self.focus.filter(|_| same);
        self.links = links;
        self.hover = None;
        self.pressed_link = None;
        self.focus = None;
        self.set_focus(focus);
    }

    /// The link under page point (x, y): the topmost hit area.
    pub fn link_at(&self, x: f32, y: f32) -> Option<usize> {
        self.page?;
        self.links
            .iter()
            .rposition(|l| l.rects.iter().any(|r| contains(r, x, y)))
    }

    /// Move keyboard focus to the next (or previous) link; `false` once it
    /// runs off either end, so focus can leave the page.
    pub fn focus_next(&mut self, back: bool) -> bool {
        let n = self.links.len();
        if self.plain || self.page.is_none() || n == 0 {
            return false;
        }
        let next = match (self.focus, back) {
            (None, false) => Some(0),
            (None, true) => Some(n - 1),
            (Some(i), false) => Some(i + 1).filter(|&j| j < n),
            (Some(i), true) => i.checked_sub(1),
        };
        self.set_focus(next);
        next.is_some()
    }

    /// Focus link `focus` and scroll it into view.
    pub fn set_focus(&mut self, focus: Option<usize>) {
        self.focus = focus;
        let first = focus
            .and_then(|i| self.links.get(i))
            .and_then(|l| l.rects.first().copied());
        if let Some(r) = first {
            let (top, height) = (self.frame.top, self.frame.height);
            if r.y < top {
                self.effects.push(Effect::ScrollTo(r.y - 16.0));
            } else if r.y + r.h > top + height {
                self.effects
                    .push(Effect::ScrollTo(r.y + r.h + 16.0 - height));
            }
        }
    }

    /// Enter on the focused link.
    pub fn activate_focused(&mut self) {
        if let Some(i) = self.focus {
            self.activate(i);
        }
    }

    /// Follow link `i`: scroll to an anchor, prefill the composer for
    /// `mailto:`, or ask before opening a web link in the browser.
    pub fn activate(&mut self, i: usize) {
        let Some(link) = self.links.get(i) else {
            return;
        };
        if link.href.starts_with('#') {
            if let Some(y) = link.target {
                self.effects.push(Effect::ScrollTo(y - 8.0));
            }
        } else {
            let (href, text) = (link.href.clone(), link.text.clone());
            self.follow(&href, &text);
        }
    }

    /// Prefill the composer for a `mailto:` address, or ask before opening
    /// a web address (shown as `text`) in the browser. Also for addresses
    /// that are not links on the page (List-Unsubscribe, an invite's Join).
    pub fn follow(&mut self, href: &str, text: &str) {
        if let Some(m) = links::mailto(href) {
            self.effects.push(Effect::Mailto(m));
        } else if let Some(web) = links::web(href, text) {
            let cap = |s: &str| -> String { s.chars().take(MAX_SHOWN_URL).collect() };
            self.prompt = Some(Prompt {
                before: cap(&web.before),
                host: cap(&web.host),
                after: cap(&web.after),
                text: text.to_string(),
                warnings: web.warnings,
                url: web.url,
            });
        }
    }

    /// Open the web link the user confirmed.
    pub fn open_prompt(&mut self) {
        if let Some(p) = self.prompt.take() {
            self.shell.open_url(&p.url);
        }
    }

    pub fn cancel_prompt(&mut self) {
        self.prompt = None;
    }
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    (r.x..r.x + r.w).contains(&x) && (r.y..r.y + r.h).contains(&y)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
