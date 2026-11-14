//! The HTML reader's engine side, shared by both apps and free of any UI
//! toolkit: a render worker thread lays messages out with
//! `crate::mail::render` and rasterizes 256 px tiles on demand ([`worker`]);
//! the UI side feeds it [`Job`]s as the reader scrolls and resizes and gets
//! [`Reply`]s back. The laid-out `Doc` never leaves the worker: the UI gets a
//! [`Page`] summary, the page's [`PageLink`]s and tile [`Pixels`].
//!
//! Fixed-width mail wider than the column is laid out at its natural width
//! and drawn scaled down (`PageTransform`); the UI side only ever sees view
//! coordinates.
//!
//! Every job carries a generation; the latest open or relayout wins, and
//! the worker drops stale jobs when it pops them and again before replying.
//!
//! Text selection also runs on the worker, against the retained `Doc`:
//! pointer gestures go there as [`SelectOp`]s and come back as highlight
//! rectangles drawn over the tiles (no re-raster); copy returns the
//! selected text.
//!
//! Remote images stay blocked unless the user consents for the message or
//! its sender: then the page is laid out again under a key with `remote`
//! set, the images it misses are fetched off the UI thread by a small pool
//! (tracking pixels and remote CSS and fonts never are), and once they are
//! in, the page is laid out once more with them. Fetched bytes stay in
//! memory for one message at a time.
//!
//! [`session`] is the UI-side state machine over the worker (what is shown,
//! which tiles are wanted, links, selection gestures, remote images), with
//! no toolkit in it; the GPUI app drives it, the Slint app keeps its own
//! copy (`src/app/reader.rs`) until the cut-over.

pub mod session;
mod worker;

pub use worker::{Backend, Fetch, Images, Job, Notify, Worker, run_to_end};

use crate::db::MarkRead;
use crate::mail::mime::Body;
use crate::mail::render::{Doc, LineMap, Rect, RenderError, RenderInput, TextLine, TextPos};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Logical height of a tile (the last one of a page may be shorter).
pub const TILE_H: f32 = 256.0;
/// Page width limits; the width snaps down to a multiple of `WIDTH_STEP`,
/// which also keeps tiles on whole device pixels at 125% and 150%.
pub const MAX_WIDTH: f32 = 960.0;
pub const MIN_WIDTH: f32 = 320.0;
pub const WIDTH_STEP: f32 = 16.0;
/// Mail whose content cannot shrink to the column (a fixed-width table) is
/// laid out at its natural width, at most this wide, and scaled down to
/// the column, but not below `MIN_FIT_SCALE`: a narrower column scrolls
/// sideways. Overflow up to `FIT_SLACK` is left alone.
pub const MAX_FIT_WIDTH: f32 = 1000.0;
pub const MIN_FIT_SCALE: f32 = 0.6;
pub const FIT_SLACK: f32 = 4.0;
/// Laid-out messages kept by the worker.
pub const DOC_CACHE: usize = 3;
/// A message window's reader keeps less: one page and 8 MB of tiles.
pub const POPOUT_DOC_CACHE: usize = 1;
pub const POPOUT_TILE_CACHE_BYTES: usize = 8 * 1024 * 1024;
/// Rasterized tiles kept by the worker (estimated as width x height x 4).
pub const TILE_CACHE_BYTES: usize = 24 * 1024 * 1024;
/// Tiles are requested this many screens below the view, and half a screen
/// above it; tiles further than `KEEP` screens away are dropped.
pub const AHEAD: f32 = 1.5;
pub const BEHIND: f32 = 0.5;
pub const KEEP: f32 = 2.5;
/// A resize lays the page out again once it pauses this long.
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(150);
/// The text view shows if no page arrives by then; a later page replaces it.
pub const FALLBACK_AFTER: Duration = Duration::from_secs(1);
/// The render worker's stack: layout recurses per nesting level.
pub const WORKER_STACK: usize = 8 * 1024 * 1024;
/// Presses this close in time and space count as a double or triple click.
pub const MULTI_CLICK: Duration = Duration::from_millis(500);
pub const MULTI_CLICK_SLOP: f32 = 4.0;
/// Link hit areas sent per page; links past this are not clickable.
pub const MAX_LINK_RECTS: usize = 20_000;
/// Lines of text sent per page for screen readers ([`Reply::Text`]).
pub const MAX_TEXT_LINES: usize = 5000;
/// The confirm prompt shows at most this much of a URL (it opens whole).
pub const MAX_SHOWN_URL: usize = 2000;
/// Consented remote images: fetched at most this many at a time, at most
/// this many per message, and at most this many bytes kept in all.
pub const FETCHERS: usize = 4;
pub const MAX_FETCHES: usize = 100;
pub const MAX_FETCHED_BYTES: usize = 32 * 1024 * 1024;

/// What the worker lays out: the raw message (parsed on the worker), or a
/// render input already parsed on the UI thread.
#[derive(Clone)]
pub enum Source {
    Raw(Arc<[u8]>),
    Input(Arc<RenderInput>),
}

/// What a laid-out message is cached under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub msg: i64,
    /// Page width in logical px.
    pub width: u32,
    /// `f32` bits of the device scale.
    pub scale: u32,
    /// The app theme is dark (the message's dark rules apply, if any).
    pub dark: bool,
    /// Dim the colours of mail without dark rules (dark theme, unless the
    /// user asked for the original colours of the message shown).
    pub dim: bool,
    /// The user consented to remote images for this message.
    pub remote: bool,
}

impl Key {
    /// The device scale.
    pub fn scale(&self) -> f32 {
        f32::from_bits(self.scale)
    }

    /// How `doc`, laid out under this key, is drawn in the column.
    fn transform(&self, doc: &Doc) -> PageTransform {
        PageTransform::new(self.width as f32, doc.width)
    }
}

/// Maps page coordinates (CSS px of a laid-out `Doc`) to view coordinates
/// (logical px on the paper card) and back. A page laid out wider than its
/// column is drawn scaled down; otherwise this is the identity. Every
/// conversion (tiles, pointer, selection, links, anchors) goes through
/// here, on the worker, so nothing is mapped twice.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PageTransform {
    /// View px per page px, in `MIN_FIT_SCALE..=1`.
    scale: f32,
}

impl PageTransform {
    /// For a page laid out `layout` px wide in a column `column` px wide.
    /// The view is the column, or when that would scale below
    /// `MIN_FIT_SCALE`, that scale's width on a 4 px grid (whole device
    /// pixels at 125%, 150% and 175%).
    fn new(column: f32, layout: f32) -> Self {
        if !(column > 0.0 && layout > column && layout.is_finite()) {
            return PageTransform { scale: 1.0 };
        }
        let view = column.max((layout * MIN_FIT_SCALE / 4.0).ceil() * 4.0);
        PageTransform {
            scale: (view / layout).min(1.0),
        }
    }

    fn y_to_view(self, y: f32) -> f32 {
        y * self.scale
    }

    fn to_view(self, r: Rect) -> Rect {
        Rect {
            x: r.x * self.scale,
            y: r.y * self.scale,
            w: r.w * self.scale,
            h: r.h * self.scale,
        }
    }

    fn to_page(self, x: f32, y: f32) -> (f32, f32) {
        (x / self.scale, y / self.scale)
    }

    /// `op` with its point in page coordinates.
    fn op_to_page(self, op: SelectOp) -> SelectOp {
        match op {
            SelectOp::Press { x, y, clicks } => {
                let (x, y) = self.to_page(x, y);
                SelectOp::Press { x, y, clicks }
            }
            SelectOp::Extend { x, y } => {
                let (x, y) = self.to_page(x, y);
                SelectOp::Extend { x, y }
            }
            op => op,
        }
    }
}

/// The width to lay out at for content reaching `natural` px in a column
/// `column` px wide: the column, or for content that cannot shrink to it,
/// its natural width up to `MAX_FIT_WIDTH`.
fn fit_width(column: f32, natural: f32) -> f32 {
    if natural.is_finite() && natural > column + FIT_SLACK {
        natural.ceil().min(MAX_FIT_WIDTH).max(column)
    } else {
        column
    }
}

/// Selection input, in view coordinates (logical px on the paper).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectOp {
    /// Button down: a caret (1 click), a word (2) or a paragraph (3).
    Press {
        x: f32,
        y: f32,
        clicks: u8,
    },
    /// The pointer moved with the button down, or a Shift+click.
    Extend {
        x: f32,
        y: f32,
    },
    All,
    Clear,
    /// Reply with the selected text.
    Copy,
}

/// The worker's selection: the unit first pressed (a caret, word or
/// paragraph), extended to the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Selection {
    key: Key,
    anchor: (TextPos, TextPos),
    focus: TextPos,
}

impl Selection {
    fn range(&self) -> (TextPos, TextPos) {
        let (a0, a1) = self.anchor;
        if self.focus < a0 {
            (self.focus, a1)
        } else {
            (a0, self.focus.max(a1))
        }
    }
}

/// Apply `op` to the selection on `doc`; `None` for an empty one.
fn select(sel: Option<Selection>, key: Key, doc: &Doc, op: SelectOp) -> Option<Selection> {
    let unit = |anchor: (TextPos, TextPos)| Selection {
        key,
        anchor,
        focus: anchor.1,
    };
    let sel = sel.filter(|s| s.key == key);
    match op {
        SelectOp::Press { x, y, clicks } => match clicks {
            2 => doc.word_at(x, y).map(unit),
            3 => doc.block_at(x, y).map(unit),
            _ => doc.text_at(x, y).map(|p| unit((p, p))),
        },
        SelectOp::Extend { x, y } => match (sel, doc.text_at(x, y)) {
            (Some(s), Some(focus)) => Some(Selection { focus, ..s }),
            (None, Some(p)) => Some(unit((p, p))),
            (s, None) => s,
        },
        SelectOp::All => doc.text_bounds().map(unit),
        SelectOp::Clear => None,
        SelectOp::Copy => sel,
    }
}

/// A link on a laid-out page, as the UI thread needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLink {
    /// Sanitized: http(s), mailto or `#fragment`.
    pub href: String,
    pub text: String,
    /// Hit areas in view coordinates (logical px).
    pub rects: Vec<Rect>,
    /// For a `#fragment` link, the top of its target, if the page has it.
    pub target: Option<f32>,
}

/// The links of `doc` that have a hit area, in display order, in view
/// coordinates.
fn links_of(doc: &Doc, tf: PageTransform) -> Vec<PageLink> {
    let mut out: Vec<PageLink> = Vec::new();
    let mut index: HashMap<u32, usize> = HashMap::new();
    for (r, link) in doc.link_rects().take(MAX_LINK_RECTS) {
        if !(r.w > 0.0 && r.h > 0.0) {
            continue;
        }
        let Some(l) = doc.links.get(link as usize) else {
            continue;
        };
        let i = *index.entry(link).or_insert_with(|| {
            out.push(PageLink {
                href: l.href.clone(),
                text: l.text.clone(),
                rects: Vec::new(),
                target: doc.anchor_y(&l.href).map(|y| tf.y_to_view(y)),
            });
            out.len() - 1
        });
        out[i].rects.push(tf.to_view(r));
    }
    out
}

/// The lines of text of `doc` in view coordinates, at most
/// [`MAX_TEXT_LINES`], and what maps a selection onto them.
fn lines_of(doc: &Doc, tf: PageTransform) -> (Vec<TextLine>, LineMap) {
    let (mut lines, map) = doc.text_lines_upto(MAX_TEXT_LINES);
    for l in &mut lines {
        let r = tf.to_view(Rect {
            x: l.x,
            y: l.y,
            w: l.w,
            h: l.h,
        });
        (l.x, l.y, l.w, l.h) = (r.x, r.y, r.w, r.h);
        for (x, w) in &mut l.chars {
            (*x, *w) = (*x * tf.scale, *w * tf.scale);
        }
    }
    (lines, map)
}

/// Opens a confirmed web link outside the app. Behind a trait so tests
/// never start a browser.
pub trait Shell {
    fn open_url(&self, url: &str);
}

/// The default browser, through `ShellExecuteW`.
pub struct SystemShell;

impl Shell for SystemShell {
    fn open_url(&self, url: &str) {
        // Re-checked at the last step: only a web URL ever reaches the shell.
        let Ok(url) = url::Url::parse(url) else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        let url = String::from(url);
        // ShellExecuteW can block (DDE, a slow handler): keep it off the UI.
        std::thread::Builder::new()
            .name("open-url".into())
            .spawn(move || {
                shell_execute(std::ffi::OsStr::new(&url));
            })
            .ok();
    }
}

/// Open a web URL or a file with its default app (call it on a thread of
/// its own, which ends right after). False if nothing opened it.
#[cfg(windows)]
pub fn shell_execute(target: &std::ffi::OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb: Vec<u16> = "open".encode_utf16().chain([0]).collect();
    let file: Vec<u16> = target.encode_wide().chain([0]).collect();
    // SAFETY: `verb` and `file` are NUL-terminated UTF-16 strings that
    // outlive the calls; the null window, parameters and directory are
    // allowed by the API. COM is initialized on this thread, as
    // ShellExecuteW asks, and the thread ends right after.
    unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        );
        // Above 32 is success.
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        ) as isize
            > 32
    }
}

#[cfg(not(windows))]
pub fn shell_execute(_target: &std::ffi::OsStr) -> bool {
    false
}

/// A text selection over the page's lines: (line, character) where it
/// starts and where it ends, in reading order.
pub type LineRange = ((usize, usize), (usize, usize));

/// The UI-side summary of a laid-out message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    /// Logical px.
    pub width: f32,
    pub height: f32,
    /// The page colour (RGBA).
    pub bg: [u8; 4],
    /// The page was cut at the height cap.
    pub truncated: bool,
    /// Only the top of a long message, laid out first: the whole page
    /// follows in another [`Reply::Page`] of the same generation.
    pub partial: bool,
    /// Remote images not shown (blocked, or not fetched yet).
    pub remote_images: u32,
    /// With consent: images still being fetched, and images that could not
    /// be loaded (fetch failed, refused or over a limit).
    pub pending_images: bool,
    pub failed_images: u32,
    /// The colours were dimmed for the dark theme.
    pub dimmed: bool,
}

/// The channel order of tile pixels. tiny-skia draws premultiplied RGBA
/// (what Slint's images take); GPUI's `RenderImage` holds BGRA. The worker
/// converts once, before the tile is cached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PixelOrder {
    #[default]
    Rgba,
    Bgra,
}

/// One rasterized tile: premultiplied, 4 bytes a pixel, rows top to bottom,
/// in the worker's [`PixelOrder`]. Cloning shares the bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Arc<[u8]>,
}

impl Pixels {
    /// A tile of `width` x `height` pixels, if `data` holds exactly that.
    pub fn new(width: u32, height: u32, data: Arc<[u8]>) -> Option<Self> {
        (data.len() as u64 == u64::from(width) * u64::from(height) * 4).then_some(Pixels {
            width,
            height,
            data,
        })
    }

    /// Bytes held (the tile cache's cost).
    pub fn bytes(&self) -> usize {
        self.data.len()
    }
}

/// What the worker answers.
pub enum Reply {
    /// The worker parsed a raw message.
    Parsed { generation: u64, body: Body },
    Page {
        generation: u64,
        page: Page,
        links: Vec<PageLink>,
    },
    /// Show the text view instead.
    Failed { generation: u64, error: RenderError },
    Tile {
        generation: u64,
        index: u32,
        pixels: Pixels,
    },
    /// The page's lines of text in view coordinates, sent right after its
    /// [`Reply::Page`]: what screen readers read of a page of pixels, with
    /// bounds for their highlight (at most [`MAX_TEXT_LINES`]).
    Text {
        generation: u64,
        lines: Vec<TextLine>,
    },
    /// The selection's highlight, in view coordinates.
    Selection { generation: u64, rects: Vec<Rect> },
    /// Where the selection starts and ends in the page's lines of text
    /// ([`Reply::Text`]), as (line, character): the page's text selection
    /// for screen readers, sent after [`Reply::Selection`] when the lines
    /// are (`None`: nothing selected).
    TextSelection {
        generation: u64,
        range: Option<LineRange>,
    },
    /// The selected text, for the clipboard.
    Copied { generation: u64, text: String },
    /// Remote images for message `msg` were fetched (or failed).
    Fetched { msg: i64 },
}

// ----------------------------------------------------------- geometry

/// The page width for a body column `body` logical px wide.
pub fn page_width(body: f32) -> Option<u32> {
    (body.is_finite() && body > 0.0)
        .then(|| ((body.min(MAX_WIDTH) / WIDTH_STEP).floor() * WIDTH_STEP).max(MIN_WIDTH) as u32)
}

/// How many tiles a page `page_h` logical px tall has.
pub fn tile_count(page_h: f32) -> u32 {
    if page_h.is_finite() && page_h > 0.0 {
        (page_h / TILE_H).ceil() as u32
    } else {
        0
    }
}

/// The logical top and bottom of tile `index`.
pub fn tile_span(index: u32, page_h: f32) -> Option<(f32, f32)> {
    let top = index as f32 * TILE_H;
    (index < tile_count(page_h)).then(|| (top, (top + TILE_H).min(page_h)))
}

/// Tile indices `[first, last]` overlapping `top..bottom` (page coords),
/// or `None` when the band misses the page.
pub fn tile_range(top: f32, bottom: f32, page_h: f32) -> Option<(u32, u32)> {
    let n = tile_count(page_h);
    if n == 0 || !(top.is_finite() && bottom.is_finite()) || bottom <= 0.0 || top >= page_h {
        return None;
    }
    let first = (top.max(0.0) / TILE_H).floor() as u32;
    let last = ((bottom.min(page_h) / TILE_H).ceil() as u32).saturating_sub(1);
    Some((first.min(n - 1), last.clamp(first.min(n - 1), n - 1)))
}

/// The tiles wanted for a view of `top..top + height` (page coords): the
/// visible ones, then up to `AHEAD` screens below and `BEHIND` above,
/// nearest first (below before above at equal distance).
pub fn wanted_tiles(top: f32, height: f32, page_h: f32) -> Vec<u32> {
    let height = height.max(0.0);
    let Some((lo, hi)) = tile_range(top - BEHIND * height, top + height + AHEAD * height, page_h)
    else {
        return Vec::new();
    };
    let (v0, v1) = tile_range(top, top + height, page_h).unwrap_or((lo, lo));
    let mut tiles: Vec<u32> = (lo..=hi).collect();
    tiles.sort_by_key(|&i| (i.saturating_sub(v1).max(v0.saturating_sub(i)), i < v0));
    tiles
}

// ---------------------------------------------------------- read delay

/// The opened message waiting to be marked read: one at a time, so a
/// message passed on the way (arrowing through the list) stays unread.
#[derive(Debug, Default)]
pub struct ReadWait(Option<i64>);

impl ReadWait {
    /// Message `id` opened: how long until it is marked read (zero: at
    /// once), or `None` when it isn't (already read, or `Never`).
    pub fn opened(&mut self, id: i64, unread: bool, mode: MarkRead) -> Option<Duration> {
        self.0 = None;
        match mode {
            _ if !unread => None,
            MarkRead::Now => Some(Duration::ZERO),
            MarkRead::After(delay) => {
                self.0 = Some(id);
                Some(delay)
            }
            MarkRead::Never => None,
        }
    }

    pub fn cancel(&mut self) {
        self.0 = None;
    }

    /// The delay ran out with `open` showing: the message to mark read,
    /// if it is still the one that waited.
    pub fn due(&mut self, open: Option<i64>) -> Option<i64> {
        self.0.take().filter(|&id| Some(id) == open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_width_snaps_and_clamps() {
        assert_eq!(page_width(1500.0), Some(960));
        assert_eq!(page_width(700.0), Some(688));
        assert_eq!(page_width(200.0), Some(320));
        assert_eq!(page_width(0.0), None);
        assert_eq!(page_width(f32::NAN), None);
        // 16 px steps stay whole device pixels at 125% and 150%.
        assert_eq!(688.0 * 1.25 % 1.0, 0.0);
        assert_eq!(688.0 * 1.5 % 1.0, 0.0);
    }

    #[test]
    fn tiles_cover_the_page() {
        assert_eq!(tile_count(1000.0), 4);
        assert_eq!(tile_count(1024.0), 4);
        assert_eq!(tile_count(0.0), 0);
        assert_eq!(tile_span(3, 1000.0), Some((768.0, 1000.0)));
        assert_eq!(tile_span(4, 1000.0), None);
        assert_eq!(tile_range(-100.0, 10.0, 1000.0), Some((0, 0)));
        assert_eq!(tile_range(300.0, 600.0, 1000.0), Some((1, 2)));
        assert_eq!(tile_range(900.0, 5000.0, 1000.0), Some((3, 3)));
        assert_eq!(tile_range(-500.0, -10.0, 1000.0), None);
        assert_eq!(tile_range(1000.0, 1200.0, 1000.0), None);
    }

    #[test]
    fn wanted_tiles_are_nearest_first() {
        // Viewing 1000..1500 of a 10000 px page: tiles 3..5 are visible,
        // then 6..8 below (1.5 screens) and 2 above (half a screen).
        assert_eq!(wanted_tiles(1000.0, 500.0, 10_000.0), [3, 4, 5, 6, 2, 7, 8]);
        // The header fills the view: the first tiles are still prefetched.
        assert_eq!(wanted_tiles(-600.0, 500.0, 10_000.0), [0, 1, 2]);
        assert_eq!(wanted_tiles(-2000.0, 500.0, 10_000.0), Vec::<u32>::new());
        assert_eq!(wanted_tiles(-200.0, 500.0, 10_000.0), [0, 1, 2, 3, 4]);
        assert_eq!(wanted_tiles(0.0, 500.0, 0.0), Vec::<u32>::new());
        assert_eq!(wanted_tiles(f32::NAN, 500.0, 1000.0), Vec::<u32>::new());
    }

    #[test]
    fn fit_width_scales_only_content_that_cannot_shrink() {
        assert_eq!(fit_width(600.0, 600.0), 600.0);
        assert_eq!(fit_width(600.0, 380.0), 600.0);
        // A hanging glyph or rounding is not a fixed-width box.
        assert_eq!(fit_width(600.0, 603.0), 600.0);
        assert_eq!(fit_width(600.0, 647.2), 648.0);
        assert_eq!(fit_width(400.0, 2408.0), MAX_FIT_WIDTH);
        assert_eq!(fit_width(960.0, f32::NAN), 960.0);
        assert_eq!(fit_width(960.0, f32::INFINITY), 960.0);
        // The view is the column, down to the 0.6 floor; then wider.
        assert_eq!(PageTransform::new(600.0, 600.0).scale, 1.0);
        assert_eq!(PageTransform::new(600.0, f32::NAN).scale, 1.0);
        assert_eq!(PageTransform::new(450.0, 600.0).scale, 0.75);
        let pinned = PageTransform::new(320.0, 1000.0);
        assert_eq!(pinned.scale, MIN_FIT_SCALE);
        assert_eq!(pinned.y_to_view(1000.0), 600.0);
        // Page and view coordinates map back and forth exactly.
        let tf = PageTransform::new(450.0, 600.0);
        let r = tf.to_view(Rect {
            x: 40.0,
            y: 80.0,
            w: 120.0,
            h: 20.0,
        });
        assert_eq!((r.x, r.y, r.w, r.h), (30.0, 60.0, 90.0, 15.0));
        assert_eq!(tf.to_page(30.0, 60.0), (40.0, 80.0));
        assert_eq!(
            tf.op_to_page(SelectOp::Extend { x: 30.0, y: 60.0 }),
            SelectOp::Extend { x: 40.0, y: 80.0 }
        );
        assert_eq!(tf.op_to_page(SelectOp::All), SelectOp::All);
    }

    #[test]
    fn pixels_must_fill_their_size() {
        assert!(Pixels::new(2, 2, Arc::from(vec![0u8; 16])).is_some());
        assert!(Pixels::new(2, 2, Arc::from(vec![0u8; 15])).is_none());
        let p = Pixels::new(4, 1, Arc::from(vec![0u8; 16])).unwrap();
        assert_eq!(p.bytes(), 16);
    }

    #[test]
    fn the_read_delay_is_cancelled_by_moving_on() {
        let delay = Duration::from_secs(1);
        let mut wait = ReadWait::default();
        // Arrowing through 1, 2, 3: only 3, still open, is marked.
        assert_eq!(wait.opened(1, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.opened(2, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.opened(3, true, MarkRead::After(delay)), Some(delay));
        assert_eq!(wait.due(Some(3)), Some(3));
        // It fires once only.
        assert_eq!(wait.due(Some(3)), None); // A read message never waits, and opening it drops an earlier wait.
        wait.opened(4, true, MarkRead::After(delay));
        assert_eq!(wait.opened(5, false, MarkRead::After(delay)), None);
        assert_eq!(wait.due(Some(4)), None);
        // Closed (or another message open) when the delay ends: nothing.
        wait.opened(6, true, MarkRead::After(delay));
        assert_eq!(wait.due(None), None);
        wait.opened(7, true, MarkRead::After(delay));
        wait.cancel();
        assert_eq!(wait.due(Some(7)), None);
        // At once, or never.
        assert_eq!(wait.opened(8, true, MarkRead::Now), Some(Duration::ZERO));
        assert_eq!(wait.due(Some(8)), None);
        assert_eq!(wait.opened(9, true, MarkRead::Never), None);
    }
}
