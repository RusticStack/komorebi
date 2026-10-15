//! The HTML reader. A render worker thread, spawned on first use, lays out
//! messages with `komorebi::mail::render` and rasterizes 256 px tiles on
//! demand; the UI side feeds the tile model as the reader scrolls and
//! resizes. The laid-out `Doc` never leaves the worker: the UI gets a
//! [`Page`] summary and pixel buffers.
//!
//! Fixed-width mail wider than the column is laid out at its natural width
//! and drawn scaled down (see [`PageTransform`]); the UI side only ever
//! sees view coordinates.
//!
//! Every job carries a generation; the latest open or relayout wins, and
//! the worker drops stale jobs when it pops them and again before replying.
//!
//! Text selection also runs on the worker, against the retained `Doc`:
//! pointer gestures go there as [`SelectOp`]s and come back as highlight
//! rectangles drawn over the tiles (no re-raster); copy returns the
//! selected text, which goes to the clipboard through a hidden TextInput.
//!
//! Links are answered on the UI thread from the [`PageLink`]s that come
//! with the page: hover, Tab focus, `#anchor` scrolls, `mailto:` into the
//! composer, and web links only after a confirm prompt, through [`Shell`].
//!
//! Remote images stay blocked unless the user consents for the message or
//! its sender: then the page is laid out again under a key with `remote`
//! set, the images it misses are fetched off this thread by a small pool
//! (tracking pixels and remote CSS and fonts never are), and once they are
//! in, the page is laid out once more with them. Fetched bytes stay in
//! memory for one message at a time.

use crate::tr::t;
use crate::{AppWindow, LinkItem, Palette, SelRect, Tile};
use komorebi::mail::mime::{self, Body};
use komorebi::mail::render::links::{self, Mailto, Warning, WebLink};
use komorebi::mail::render::{
    self, Deadline, Doc, Engine, Opts, Rect, RemotePolicy, RenderError, RenderInput, Resolver,
    TextPos,
};
use slint::{
    ComponentHandle, Image, Model, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode,
    VecModel, Weak,
};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Logical height of a tile (the last one of a page may be shorter).
const TILE_H: f32 = 256.0;
/// Page width limits; the width snaps down to a multiple of `WIDTH_STEP`,
/// which also keeps tiles on whole device pixels at 125% and 150%.
const MAX_WIDTH: f32 = 960.0;
const MIN_WIDTH: f32 = 320.0;
const WIDTH_STEP: f32 = 16.0;
/// Mail whose content cannot shrink to the column (a fixed-width table) is
/// laid out at its natural width, at most this wide, and scaled down to
/// the column, but not below `MIN_FIT_SCALE`: a narrower column scrolls
/// sideways. Overflow up to `FIT_SLACK` is left alone.
const MAX_FIT_WIDTH: f32 = 1000.0;
const MIN_FIT_SCALE: f32 = 0.6;
const FIT_SLACK: f32 = 4.0;
/// Laid-out messages kept by the worker.
const DOC_CACHE: usize = 3;
/// A message window's reader keeps less: one page and 8 MB of tiles.
pub const POPOUT_DOC_CACHE: usize = 1;
pub const POPOUT_TILE_CACHE_BYTES: usize = 8 * 1024 * 1024;
/// Rasterized tiles kept by the worker (estimated as width x height x 4).
const TILE_CACHE_BYTES: usize = 24 * 1024 * 1024;
/// Tiles are requested this many screens below the view, and half a screen
/// above it; tiles further than `KEEP` screens away are dropped.
const AHEAD: f32 = 1.5;
const BEHIND: f32 = 0.5;
const KEEP: f32 = 2.5;
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(150);
/// The text view shows if no page arrives by then; a later page replaces it.
const FALLBACK_AFTER: Duration = Duration::from_secs(1);
/// The render worker's stack: layout recurses per nesting level.
const WORKER_STACK: usize = 8 * 1024 * 1024;
/// Presses this close in time and space count as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(500);
const MULTI_CLICK_SLOP: f32 = 4.0;
/// Link hit areas sent per page; links past this are not clickable.
const MAX_LINK_RECTS: usize = 20_000;
/// The confirm prompt shows at most this much of a URL (it opens whole).
const MAX_SHOWN_URL: usize = 2000;
/// Consented remote images: fetched at most this many at a time, at most
/// this many per message, and at most this many bytes kept in all.
const FETCHERS: usize = 4;
const MAX_FETCHES: usize = 100;
const MAX_FETCHED_BYTES: usize = 32 * 1024 * 1024;

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
    msg: i64,
    /// Page width in logical px.
    width: u32,
    /// `f32` bits of the device scale.
    scale: u32,
    /// The app theme is dark (the message's dark rules apply, if any).
    dark: bool,
    /// Dim the colours of mail without dark rules (dark theme, unless the
    /// user asked for the original colours of the message shown).
    dim: bool,
    /// The user consented to remote images for this message.
    remote: bool,
}

impl Key {
    fn scale(&self) -> f32 {
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

pub enum Job {
    /// Load fonts ahead of the first open.
    Warm,
    Layout {
        generation: u64,
        key: Key,
        src: Source,
    },
    /// Rasterize these tiles of the page laid out under `key`, in order.
    Tiles {
        generation: u64,
        key: Key,
        tiles: Vec<u32>,
    },
    /// A selection gesture or command on the page laid out under `key`.
    Select {
        generation: u64,
        key: Key,
        op: SelectOp,
    },
    /// Nothing is shown any more: drop work older than `generation`.
    Cancel { generation: u64 },
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
        let open = move || {
            shell_execute(std::ffi::OsStr::new(&url));
        };
        super::background("open-url", Box::new(open)).ok();
    }
}

/// Open a web URL or a file with its default app (on a thread of its own,
/// which ends right after). False if nothing opened it.
#[cfg(windows)]
pub(super) fn shell_execute(target: &std::ffi::OsStr) -> bool {
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
pub(super) fn shell_execute(_target: &std::ffi::OsStr) -> bool {
    false
}

/// The confirm prompt's line for a warning about `link`.
fn warning_text(w: Warning, link: &WebLink, text: &str) -> String {
    let host = link.host.as_str();
    match w {
        Warning::TextMismatch => {
            let shown: String = text.chars().take(80).collect();
            t!(invoke_link_text_mismatch, shown.into(), host.into())
        }
        Warning::Punycode => t!(invoke_link_punycode, host.into()),
        Warning::UserInfo => t!(invoke_link_user_info, host.into()),
        Warning::RawIp => t!(invoke_link_raw_ip, host.into()),
    }
}

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
    /// Remote images not shown (blocked, or not fetched yet).
    pub remote_images: u32,
    /// With consent: images still being fetched, and images that could not
    /// be loaded (fetch failed, refused or over a limit).
    pub pending_images: bool,
    pub failed_images: u32,
    /// The colours were dimmed for the dark theme.
    pub dimmed: bool,
}

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
        pixels: SharedPixelBuffer<Rgba8Pixel>,
    },
    /// The selection's highlight, in view coordinates.
    Selection { generation: u64, rects: Vec<Rect> },
    /// The selected text, for the clipboard.
    Copied { generation: u64, text: String },
    /// Remote images for message `msg` were fetched (or failed).
    Fetched { msg: i64 },
}

pub type Notify = Arc<dyn Fn(Reply) + Send + Sync>;
/// Fetches a consented remote image (`komorebi::fetch`, or a fake).
pub type Fetch = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Where jobs go: the render worker, or a fake in tests.
pub trait Backend {
    fn send(&mut self, job: Job);
}

// ------------------------------------------------------------- worker

/// The handle to the render worker thread, which starts on the first job.
pub struct Worker {
    tx: Option<Sender<Job>>,
    latest: Arc<AtomicU64>,
    notify: Notify,
    fetch: Fetch,
    /// Laid-out pages and tile bytes kept (the main reader's, or a message
    /// window's smaller ones).
    caches: (usize, usize),
}

impl Worker {
    pub fn new(notify: Notify, fetch: Fetch) -> Self {
        Worker {
            tx: None,
            latest: Arc::default(),
            notify,
            fetch,
            caches: (DOC_CACHE, TILE_CACHE_BYTES),
        }
    }

    /// Keep at most `docs` laid-out pages and `tile_bytes` of tiles. The
    /// thread ends when this handle is dropped (its channel closes).
    pub fn with_caches(mut self, docs: usize, tile_bytes: usize) -> Self {
        self.caches = (docs.max(1), tile_bytes);
        self
    }

    fn spawn(&self) -> Option<Sender<Job>> {
        let (tx, rx) = mpsc::channel();
        let latest = self.latest.clone();
        let notify = self.notify.clone();
        let fetch = self.fetch.clone();
        let caches = self.caches;
        std::thread::Builder::new()
            .name("render".into())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let images: Arc<Mutex<Images>> = Arc::default();
                let start = |msg, urls| fetch_images(msg, urls, &images, &fetch, &notify);
                run(rx, &latest, &*notify, &images, &start, caches)
            })
            .ok()?;
        Some(tx)
    }
}

/// A consented remote image of the message in [`Images`].
enum Fetched {
    Pending,
    Loaded(Arc<[u8]>),
    Failed,
}

/// Remote images fetched for one message (the latest one laid out with
/// consent), in memory only.
#[derive(Default)]
pub struct Images {
    msg: i64,
    urls: HashMap<String, Fetched>,
    bytes: usize,
}

impl Images {
    fn store(&mut self, url: &str, got: Option<Vec<u8>>) {
        let got = got.filter(|b| self.bytes + b.len() <= MAX_FETCHED_BYTES);
        self.bytes += got.as_ref().map_or(0, Vec::len);
        let state = got.map_or(Fetched::Failed, |b| Fetched::Loaded(b.into()));
        self.urls.insert(url.to_string(), state);
    }

    fn loaded(&self, url: &str) -> bool {
        matches!(self.urls.get(url), Some(Fetched::Loaded(_)))
    }
}

impl Resolver for &Images {
    fn remote(&mut self, url: &str) -> Option<Arc<[u8]>> {
        match self.urls.get(url) {
            Some(Fetched::Loaded(b)) => Some(b.clone()),
            _ => None,
        }
    }
}

fn lock(images: &Mutex<Images>) -> MutexGuard<'_, Images> {
    images.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Fetch `urls` for message `msg` on a pool of [`FETCHERS`] threads, then
/// tell the UI. Work stops early once another message takes the cache.
/// Without a thread the images stay pending (outlined placeholders).
fn fetch_images(
    msg: i64,
    urls: Vec<String>,
    images: &Arc<Mutex<Images>>,
    fetch: &Fetch,
    notify: &Notify,
) {
    let (images, fetch, notify) = (images.clone(), fetch.clone(), notify.clone());
    let pool = move || {
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..FETCHERS.min(urls.len()) {
                s.spawn(|| {
                    while let Some(url) = urls.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if lock(&images).msg != msg {
                            return;
                        }
                        let got = fetch(url);
                        let mut images = lock(&images);
                        if images.msg != msg {
                            return;
                        }
                        images.store(url, got);
                    }
                });
            }
        });
        notify(Reply::Fetched { msg });
    };
    std::thread::Builder::new()
        .name("fetch".into())
        .spawn(pool)
        .ok();
}

impl Backend for Worker {
    fn send(&mut self, job: Job) {
        match &job {
            Job::Layout { generation, .. } | Job::Cancel { generation } => {
                self.latest.fetch_max(*generation, Ordering::Relaxed);
            }
            _ => {}
        }
        // A cancel frees the worker's pages; it never starts a worker.
        if matches!(job, Job::Cancel { .. }) && self.tx.is_none() {
            return;
        }
        if self.tx.is_none() {
            self.tx = self.spawn();
        }
        let failed = match self.tx.as_ref() {
            Some(tx) => tx.send(job).err().map(|e| e.0),
            None => Some(job),
        };
        // No worker: the text view stands in.
        if let Some(Job::Layout { generation, .. }) = failed {
            (self.notify)(Reply::Failed {
                generation,
                error: RenderError::Malformed,
            });
        }
    }
}

/// Whether a job of generation `generation` is still wanted.
fn current(generation: u64, latest: &AtomicU64) -> bool {
    generation >= latest.load(Ordering::Relaxed)
}

/// The worker loop: runs until the UI side hangs up. Consented remote
/// images come from `images`; `start` fetches more for a message.
fn run(
    rx: Receiver<Job>,
    latest: &AtomicU64,
    notify: &dyn Fn(Reply),
    images: &Mutex<Images>,
    start: &dyn Fn(i64, Vec<String>),
    (doc_cache, tile_bytes): (usize, usize),
) {
    let mut engine = Engine::new();
    let mut docs: Lru<Key, Doc> = Lru::new(doc_cache);
    let mut tiles: Lru<(Key, u32), SharedPixelBuffer<Rgba8Pixel>> = Lru::new(tile_bytes);
    let mut sel: Option<Selection> = None;
    while let Ok(job) = rx.recv() {
        match job {
            // The system font list (fontique) and the emoji fallback load
            // here, at low priority, once the window shows.
            Job::Warm => {
                below_normal(true);
                let input = RenderInput {
                    html: Some("<p>Komorebi \u{1F642}</p>".into()),
                    ..Default::default()
                };
                let deadline = Deadline::after(render::LAYOUT_BUDGET);
                render::layout(&input, &Opts::default(), &mut engine, &deadline).ok();
                below_normal(false);
            }
            Job::Layout {
                generation,
                key,
                src,
            } => {
                if !current(generation, latest) {
                    continue;
                }
                // The page shown is replaced, and the UI clears its highlight.
                sel = None;
                let input = match src {
                    Source::Input(input) => input,
                    Source::Raw(raw) => {
                        let (body, input) = mime::parse_reader(&raw);
                        notify(Reply::Parsed { generation, body });
                        Arc::new(input)
                    }
                };
                let mut cache = lock(images);
                // Another message: free the previous one's pages, tiles and
                // fetched images (which also stops its fetches).
                if cache.msg != key.msg {
                    *cache = Images {
                        msg: key.msg,
                        ..Default::default()
                    };
                    docs.retain(|k| k.msg == key.msg);
                    tiles.retain(|(k, _)| k.msg == key.msg);
                }
                // A page laid out before its remote images arrived is stale.
                let fresh = |doc: &Doc| !doc.remote_images.iter().any(|u| cache.loaded(u));
                let mut wanted = Vec::new();
                let page = match docs.get(&key).filter(|doc| !key.remote || fresh(doc)) {
                    Some(doc) => Ok((page_of(doc, key, &cache), links_of(doc, key.transform(doc)))),
                    None => lay_out(&input, key, &mut engine, &cache).map(|doc| {
                        if key.remote {
                            let room = MAX_FETCHES.saturating_sub(cache.urls.len());
                            for u in &doc.remote_images {
                                if wanted.len() < room && !cache.urls.contains_key(u) {
                                    cache.urls.insert(u.clone(), Fetched::Pending);
                                    wanted.push(u.clone());
                                }
                            }
                        }
                        let page = (
                            page_of(&doc, key, &cache),
                            links_of(&doc, key.transform(&doc)),
                        );
                        docs.insert(key, doc, 1);
                        tiles.retain(|(k, _)| *k != key);
                        page
                    }),
                };
                drop(cache);
                if !wanted.is_empty() {
                    start(key.msg, wanted);
                }
                if current(generation, latest) {
                    notify(match page {
                        Ok((page, links)) => Reply::Page {
                            generation,
                            page,
                            links,
                        },
                        Err(error) => Reply::Failed { generation, error },
                    });
                }
            }
            Job::Tiles {
                generation,
                key,
                tiles: wanted,
            } => {
                for index in wanted {
                    if !current(generation, latest) {
                        break;
                    }
                    let pixels = match tiles.get(&(key, index)) {
                        Some(p) => Some(p.clone()),
                        None => docs
                            .get(&key)
                            .and_then(|doc| rasterize(doc, key, &mut engine, index))
                            .inspect(|p| {
                                let bytes = p.as_bytes().len();
                                tiles.insert((key, index), p.clone(), bytes);
                            }),
                    };
                    if let Some(pixels) = pixels {
                        notify(Reply::Tile {
                            generation,
                            index,
                            pixels,
                        });
                    }
                }
            }
            Job::Select {
                generation,
                key,
                op,
            } => {
                if !current(generation, latest) {
                    continue;
                }
                // A page gone from the cache has no selection any more.
                let doc = docs.get(&key);
                sel = doc.and_then(|doc| select(sel, key, doc, key.transform(doc).op_to_page(op)));
                let range = doc.zip(sel.map(|s| s.range()));
                let reply = if op == SelectOp::Copy {
                    match range.map(|(doc, (a, b))| doc.selection_text(a, b)) {
                        Some(text) if !text.is_empty() => Reply::Copied { generation, text },
                        _ => continue,
                    }
                } else {
                    let rects = range.map_or_else(Vec::new, |(doc, (a, b))| {
                        let tf = key.transform(doc);
                        let rects = doc.selection_rects(a, b);
                        rects.into_iter().map(|r| tf.to_view(r)).collect()
                    });
                    Reply::Selection { generation, rects }
                };
                if current(generation, latest) {
                    notify(reply);
                }
            }
            // Nothing is shown: free everything kept for the page.
            Job::Cancel { .. } => {
                sel = None;
                docs.retain(|_| false);
                tiles.retain(|_| false);
                *lock(images) = Images::default();
            }
        }
    }
}

/// Run the calling thread below normal priority (`true`) or at normal
/// priority again, so warming fonts never competes with the UI.
#[cfg(windows)]
fn below_normal(on: bool) {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_NORMAL,
    };
    let priority = if on {
        THREAD_PRIORITY_BELOW_NORMAL
    } else {
        THREAD_PRIORITY_NORMAL
    };
    // SAFETY: the pseudo-handle of the calling thread, always valid; only
    // its scheduling priority changes.
    unsafe { SetThreadPriority(GetCurrentThread(), priority) };
}

#[cfg(not(windows))]
fn below_normal(_on: bool) {}

/// Lay out `input` under `key`; with consent, remote images come from
/// `images` (what has been fetched so far).
fn lay_out(
    input: &RenderInput,
    key: Key,
    engine: &mut Engine,
    images: &Images,
) -> Result<Doc, RenderError> {
    if input.prefers_plain {
        return Err(RenderError::NoHtml);
    }
    let mut opts = Opts {
        width_px: key.width as f32,
        scale: key.scale(),
        remote: if key.remote {
            RemotePolicy::Allow
        } else {
            RemotePolicy::Block
        },
        // Dimming changes no geometry, so the first pass dims already (the
        // renderer skips it for mail with dark rules of its own).
        dim: key.dim,
        ..Default::default()
    };
    let budget = || Deadline::after(render::LAYOUT_BUDGET);
    let doc = render::layout_with(input, &opts, engine, &budget(), &mut { images })?;
    // Content that cannot shrink to the column is laid out once more at
    // its natural width, to be drawn scaled; in a dark app, a message with
    // dark rules is drawn with them. Both go into one more pass.
    let column = opts.width_px;
    let width = fit_width(column, doc.natural_width);
    opts.dark = key.dark && doc.has_dark_rules;
    if width == column && !opts.dark {
        return Ok(doc);
    }
    opts.width_px = width;
    opts.scale = key.scale() * PageTransform::new(column, width).scale;
    match render::layout_with(input, &opts, engine, &budget(), &mut { images }) {
        // Content that overflows further on a wider page is sized relative
        // to the page, not fixed: scaling would never make it fit.
        Ok(wide) if width > column && wide.natural_width > doc.natural_width + FIT_SLACK => {
            if !opts.dark {
                return Ok(doc);
            }
            opts.width_px = column;
            opts.scale = key.scale();
            let dark = render::layout_with(input, &opts, engine, &budget(), &mut { images });
            Ok(dark.unwrap_or(doc))
        }
        Ok(page) => Ok(page),
        Err(_) => Ok(doc),
    }
}

/// The summary of `doc`, laid out under `key`; with consent, `images`
/// says which of its missing images are still coming and which failed (or
/// were never asked for, past [`MAX_FETCHES`]).
fn page_of(doc: &Doc, key: Key, images: &Images) -> Page {
    let tf = key.transform(doc);
    let (mut pending, mut failed) = (false, 0u32);
    if key.remote {
        for u in &doc.remote_images {
            match images.urls.get(u) {
                Some(Fetched::Pending) => pending = true,
                Some(Fetched::Loaded(_)) => {}
                Some(Fetched::Failed) | None => failed = failed.saturating_add(1),
            }
        }
    }
    Page {
        width: tf.y_to_view(doc.width).round(),
        height: tf.y_to_view(doc.height),
        bg: doc.bg,
        truncated: doc.truncated,
        remote_images: u32::try_from(doc.remote_images.len()).unwrap_or(u32::MAX),
        pending_images: pending,
        failed_images: failed,
        dimmed: doc.dimmed,
    }
}

/// Tile `index` of `doc` (laid out under `key`) at device pixels. Tiles
/// span `TILE_H` view px; a scaled page is rasterized at the device scale
/// times its own, so the tile is drawn 1:1 and stays sharp.
fn rasterize(
    doc: &Doc,
    key: Key,
    engine: &mut Engine,
    index: u32,
) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let tf = key.transform(doc);
    let scale = key.scale();
    let (top, bottom) = tile_span(index, tf.y_to_view(doc.height))?;
    let y0 = (top * scale).round() as u32;
    let y1 = ((bottom * scale).round() as u32).max(y0 + 1);
    let pm = render::rasterize(doc, engine, y0, y1 - y0, scale * tf.scale)?;
    let (w, h) = (pm.width(), pm.height());
    (pm.data().len() as u64 == u64::from(w) * u64::from(h) * 4)
        .then(|| SharedPixelBuffer::clone_from_slice(pm.data(), w, h))
}

/// A least-recently-used cache bounded by a total cost (bytes, or 1 per
/// entry for a count limit). The newest entry is always kept.
struct Lru<K, V> {
    entries: VecDeque<(K, V, usize)>,
    cost: usize,
    cap: usize,
}

impl<K: PartialEq, V> Lru<K, V> {
    fn new(cap: usize) -> Self {
        Lru {
            entries: VecDeque::new(),
            cost: 0,
            cap,
        }
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        let i = self.entries.iter().position(|(k, ..)| k == key)?;
        let entry = self.entries.remove(i)?;
        self.entries.push_back(entry);
        self.entries.back().map(|(_, v, _)| v)
    }

    fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.entries.retain(|(k, ..)| keep(k));
        self.cost = self.entries.iter().map(|(.., c)| c).sum();
    }

    fn insert(&mut self, key: K, value: V, cost: usize) {
        if let Some(i) = self.entries.iter().position(|(k, ..)| *k == key)
            && let Some((_, _, old)) = self.entries.remove(i)
        {
            self.cost -= old;
        }
        self.entries.push_back((key, value, cost));
        self.cost += cost;
        while self.cost > self.cap && self.entries.len() > 1 {
            if let Some((_, _, c)) = self.entries.pop_front() {
                self.cost -= c;
            }
        }
    }
}

// ----------------------------------------------------------- geometry

/// The page width for a body column `body` logical px wide.
fn page_width(body: f32) -> Option<u32> {
    (body.is_finite() && body > 0.0)
        .then(|| ((body.min(MAX_WIDTH) / WIDTH_STEP).floor() * WIDTH_STEP).max(MIN_WIDTH) as u32)
}

fn tile_count(page_h: f32) -> u32 {
    if page_h.is_finite() && page_h > 0.0 {
        (page_h / TILE_H).ceil() as u32
    } else {
        0
    }
}

/// The logical top and bottom of tile `index`.
fn tile_span(index: u32, page_h: f32) -> Option<(f32, f32)> {
    let top = index as f32 * TILE_H;
    (index < tile_count(page_h)).then(|| (top, (top + TILE_H).min(page_h)))
}

/// Tile indices `[first, last]` overlapping `top..bottom` (page coords),
/// or `None` when the band misses the page.
fn tile_range(top: f32, bottom: f32, page_h: f32) -> Option<(u32, u32)> {
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
fn wanted_tiles(top: f32, height: f32, page_h: f32) -> Vec<u32> {
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

// ------------------------------------------------------------ UI side

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TileState {
    Missing,
    Requested,
    Shown,
}

struct State {
    me: std::rc::Weak<RefCell<State>>,
    ui: Weak<AppWindow>,
    backend: Box<dyn Backend>,
    tiles: Rc<VecModel<Tile>>,
    generation: u64,
    /// The message shown and its source, kept for relayouts.
    src: Option<(i64, Source)>,
    /// What the latest layout job was for.
    key: Option<Key>,
    /// The latest job is a relayout of the page shown, whose tiles stay up
    /// (stretched) until new ones replace them.
    relayout: bool,
    page: Option<Page>,
    state: Vec<TileState>,
    resize_timer: Timer,
    fallback_timer: Timer,
    /// When the message was opened, for `KOMOREBI_TIMING`.
    opened: Option<Instant>,
    timing: bool,
    /// Highlight rectangles of the selection on the page shown.
    selection: Rc<VecModel<SelRect>>,
    /// The last button press (time, position, click count), to tell double
    /// and triple clicks.
    last_press: Option<(Instant, f32, f32, u8)>,
    shell: Box<dyn Shell>,
    /// Where a clicked `mailto:` link goes (the composer).
    mailto: Option<Rc<dyn Fn(Mailto)>>,
    /// The links of the page shown.
    links: Vec<PageLink>,
    links_model: Rc<VecModel<LinkItem>>,
    focus_model: Rc<VecModel<SelRect>>,
    hover: Option<usize>,
    /// The link with keyboard focus (Tab).
    focus: Option<usize>,
    /// The link under the last single press, opened if the button comes up
    /// on it without a drag.
    pressed_link: Option<usize>,
    /// A web link waiting in the confirm prompt.
    pending: Option<String>,
    /// Remote images may load for the message shown.
    remote: bool,
    /// The message shown keeps its original colours in the dark theme.
    original: bool,
    /// Why the formatted view is not shown, while its note is (the note is
    /// said again when the language changes).
    failed: Option<RenderError>,
}

/// The UI-side controller of the HTML reader.
#[derive(Clone)]
pub struct Reader(Rc<RefCell<State>>);

impl Reader {
    pub fn new(ui: &AppWindow, backend: Box<dyn Backend>, shell: Box<dyn Shell>) -> Self {
        let tiles = Rc::new(VecModel::default());
        ui.set_r_tiles(tiles.clone().into());
        let selection = Rc::new(VecModel::default());
        ui.set_r_selection(selection.clone().into());
        let links_model = Rc::new(VecModel::default());
        ui.set_r_links(links_model.clone().into());
        let focus_model = Rc::new(VecModel::default());
        ui.set_r_focus(focus_model.clone().into());
        let reader = Reader(Rc::new_cyclic(|me| {
            RefCell::new(State {
                me: me.clone(),
                ui: ui.as_weak(),
                backend,
                tiles,
                generation: 0,
                src: None,
                key: None,
                relayout: false,
                page: None,
                state: Vec::new(),
                resize_timer: Timer::default(),
                fallback_timer: Timer::default(),
                opened: None,
                timing: komorebi::dev_var("KOMOREBI_TIMING").is_some(),
                selection,
                last_press: None,
                shell,
                mailto: None,
                links: Vec::new(),
                links_model,
                focus_model,
                hover: None,
                focus: None,
                pressed_link: None,
                pending: None,
                remote: false,
                original: false,
                failed: None,
            })
        }));
        let r = reader.clone();
        ui.on_r_body_resized(move || r.with(State::body_resized));
        let r = reader.clone();
        ui.on_r_view_changed(move || r.with(State::request_tiles));
        let r = reader.clone();
        ui.on_r_press(move |x, y| r.with(move |s| s.press(x, y)));
        let r = reader.clone();
        ui.on_r_extend(move |x, y| r.with(move |s| s.extend(x, y)));
        let r = reader.clone();
        ui.on_r_select_all(move || r.with(|s| s.select(SelectOp::All)));
        let r = reader.clone();
        ui.on_r_clear_selection(move || {
            r.with(|s| {
                s.select(SelectOp::Clear);
                s.set_focus(None);
            })
        });
        let r = reader.clone();
        ui.on_r_copy(move || r.with(|s| s.select(SelectOp::Copy)));
        let r = reader.clone();
        ui.on_r_hover(move |x, y| r.with(move |s| s.hover_at(x, y)));
        let r = reader.clone();
        ui.on_r_release(move |x, y| r.with(move |s| s.release(x, y)));
        let r = reader.clone();
        // Answered at once: `false` lets Tab move focus on out of the page.
        ui.on_r_focus_link(move |back| r.0.try_borrow_mut().is_ok_and(|mut s| s.focus_next(back)));
        let r = reader.clone();
        ui.on_r_activate_focused(move || {
            r.with(|s| {
                if let Some(i) = s.focus {
                    s.activate(i);
                }
            })
        });
        let r = reader.clone();
        ui.on_r_activate(move |i| r.with(move |s| s.activate(i.max(0) as usize)));
        let r = reader.clone();
        ui.on_l_open(move || r.with(State::open_pending));
        let r = reader.clone();
        ui.on_r_show_original(move |on| r.with(move |s| s.show_original(on)));
        let r = reader.clone();
        ui.on_l_cancel(move || r.with(|s| s.cancel_pending()));
        reader
    }

    /// Send clicked `mailto:` links to `f` (the composer). It runs outside
    /// the reader's own callbacks.
    pub fn on_mailto(&self, f: impl Fn(Mailto) + 'static) {
        self.state().mailto = Some(Rc::new(f));
    }

    /// Follow a sanitized `mailto:` or web address that is not a link on
    /// the page (List-Unsubscribe): into the composer, or through the
    /// confirm prompt, as a clicked link would.
    pub fn follow(&self, href: String) {
        self.with(move |s| s.follow(&href, ""));
    }

    /// Run `f` on the state. Slint can run change handlers while the state
    /// is borrowed (a component created during a property read), so a busy
    /// state defers the call instead of panicking.
    fn with(&self, f: impl FnOnce(&mut State) + 'static) {
        match self.0.try_borrow_mut() {
            Ok(mut s) => f(&mut s),
            Err(_) => {
                let me = Rc::downgrade(&self.0);
                Timer::single_shot(Duration::ZERO, move || {
                    if let Some(s) = me.upgrade() {
                        Reader(s).with(f);
                    }
                });
            }
        }
    }

    fn state(&self) -> std::cell::RefMut<'_, State> {
        self.0.borrow_mut()
    }

    /// Load fonts on the worker (once the window is up).
    pub fn warm(&self) {
        self.state().backend.send(Job::Warm);
    }

    /// Render message `msg` from `src`, with its remote images if `remote`
    /// (the sender is on the allowlist). Consent given for the message
    /// shown lasts while it stays open. The body stays blank until the page
    /// arrives, or until the text view stands in after a second.
    pub fn open(&self, msg: i64, src: Source, remote: bool) {
        let mut s = self.state();
        let same = s.src.as_ref().is_some_and(|(m, _)| *m == msg);
        s.remote = remote || same && s.remote;
        s.original = same && s.original;
        s.src = Some((msg, src));
        s.key = None;
        s.opened = Some(Instant::now());
        s.show_waiting();
        s.layout(false);
    }

    /// Load the remote images of the message shown: lay it out again with
    /// them, keeping the current page up until the new one arrives.
    pub fn allow_remote(&self) {
        let mut s = self.state();
        if s.src.is_none() || s.remote {
            return;
        }
        s.remote = true;
        if let Some(ui) = s.ui() {
            ui.set_r_remote_blocked(0);
        }
        let shown = s.page.is_some();
        s.layout(shown);
    }

    /// The language changed: the note about a failed render says so in it.
    pub fn language_changed(&self) {
        let s = self.state();
        if let (Some(error), Some(ui)) = (s.failed, s.ui()) {
            ui.set_r_note(State::failed_note(error).into());
        }
    }

    /// Show the text view; renders in flight are dropped.
    pub fn close(&self) {
        let mut s = self.state();
        s.src = None;
        s.remote = false;
        s.original = false;
        s.failed = None;
        s.key = None;
        s.page = None;
        s.generation += 1;
        let generation = s.generation;
        s.backend.send(Job::Cancel { generation });
        s.resize_timer.stop();
        s.fallback_timer.stop();
        s.tiles.set_vec(Vec::new());
        s.state.clear();
        s.selection.set_vec(Vec::new());
        s.set_links(Vec::new(), false);
        s.cancel_pending();
        if let Some(ui) = s.ui.upgrade() {
            ui.set_r_html(false);
            ui.set_r_waiting(false);
            ui.set_r_note(SharedString::new());
            ui.set_r_remote_blocked(0);
            ui.set_r_remote_failed(0);
            ui.set_r_dimmed(false);
            ui.set_r_original(false);
        }
    }

    /// Handle a worker reply; returns the body parsed by the worker for the
    /// message shown.
    pub fn on_reply(&self, reply: Reply) -> Option<Body> {
        let mut s = self.state();
        match reply {
            Reply::Parsed { generation, body } => (generation == s.generation).then_some(body),
            Reply::Page {
                generation,
                page,
                links,
            } => {
                if generation == s.generation {
                    s.show_page(page, links);
                }
                None
            }
            Reply::Failed { generation, error } => {
                if generation == s.generation {
                    s.show_failed(error);
                }
                None
            }
            Reply::Tile {
                generation,
                index,
                pixels,
            } => {
                if generation == s.generation {
                    s.show_tile(index, pixels);
                }
                None
            }
            Reply::Selection { generation, rects } => {
                if generation == s.generation {
                    s.selection
                        .set_vec(rects.into_iter().map(sel_rect).collect::<Vec<_>>());
                }
                None
            }
            Reply::Copied { generation, text } => {
                if generation == s.generation
                    && let Some(ui) = s.ui()
                {
                    ui.invoke_r_copy_text(text.into());
                }
                None
            }
            // Lay the page out again with the images now in.
            Reply::Fetched { msg } => {
                if s.remote && s.src.as_ref().is_some_and(|(m, _)| *m == msg) {
                    let shown = s.page.is_some();
                    s.layout(shown);
                }
                None
            }
        }
    }
}

fn sel_rect(r: Rect) -> SelRect {
    SelRect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

fn contains(r: &Rect, x: f32, y: f32) -> bool {
    (r.x..r.x + r.w).contains(&x) && (r.y..r.y + r.h).contains(&y)
}

/// A timer callback that runs `f` on the state if the reader still exists.
fn later(me: std::rc::Weak<RefCell<State>>, f: fn(&mut State)) -> impl FnMut() + 'static {
    move || {
        if let Some(s) = me.upgrade() {
            Reader(s).with(f);
        }
    }
}

impl State {
    fn ui(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    /// The key a layout would use now; `None` until the body column has
    /// reported its width.
    fn current_key(&self) -> Option<Key> {
        let ui = self.ui()?;
        let (msg, _) = self.src.as_ref()?;
        let dark = ui.global::<Palette>().get_color_scheme() == slint::language::ColorScheme::Dark;
        let scale = ui.window().scale_factor();
        Some(Key {
            msg: *msg,
            width: page_width(ui.get_r_body_width())?,
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
        self.tiles.set_vec(Vec::new());
        self.state.clear();
        self.selection.set_vec(Vec::new());
        self.last_press = None;
        self.set_links(Vec::new(), false);
        self.cancel_pending();
        if let Some(ui) = self.ui() {
            ui.set_r_page_x(0.0);
            ui.set_r_html(false);
            ui.set_r_waiting(true);
            ui.set_r_note(SharedString::new());
            ui.set_r_remote_blocked(0);
            ui.set_r_remote_failed(0);
            ui.set_r_dimmed(false);
            ui.set_r_original(self.original);
        }
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
            let show_text = |s: &mut State| {
                if s.page.is_none()
                    && let Some(ui) = s.ui()
                {
                    ui.set_r_waiting(false);
                }
            };
            self.fallback_timer.start(
                TimerMode::SingleShot,
                FALLBACK_AFTER,
                later(self.me.clone(), show_text),
            );
        }
    }

    /// The body column or the scale changed: lay out again once resizing
    /// pauses, or at once if the first width just arrived.
    fn body_resized(&mut self) {
        if self.src.is_none() {
            return;
        }
        let Some(key) = self.key else {
            self.layout(false);
            return;
        };
        if self.current_key() == Some(key) {
            self.resize_timer.stop();
            return;
        }
        self.resize_timer.start(
            TimerMode::SingleShot,
            RESIZE_DEBOUNCE,
            later(self.me.clone(), State::resize_done),
        );
    }

    /// The resize debounce ran out: lay out at the new width or scale.
    fn resize_done(&mut self) {
        if self.src.is_some() && self.key.is_some() && self.current_key() != self.key {
            let shown = self.page.is_some();
            self.layout(shown);
        }
    }

    /// "Show original colours" for the message shown (dark theme): lay it
    /// out again without (or with) dimming, keeping the page up meanwhile.
    fn show_original(&mut self, on: bool) {
        if self.src.is_none() || self.original == on {
            return;
        }
        self.original = on;
        if let Some(ui) = self.ui() {
            ui.set_r_original(on);
        }
        let shown = self.page.is_some();
        self.layout(shown);
    }

    fn show_page(&mut self, page: Page, links: Vec<PageLink>) {
        let n = tile_count(page.height);
        let old: Vec<Tile> = if self.relayout {
            self.tiles.iter().collect()
        } else {
            Vec::new()
        };
        let rows: Vec<Tile> = (0..n)
            .map(|i| Tile {
                img: old
                    .get(i as usize)
                    .map(|t| t.img.clone())
                    .unwrap_or_default(),
                h: tile_span(i, page.height).map_or(TILE_H, |(t, b)| b - t),
            })
            .collect();
        self.tiles.set_vec(rows);
        self.state = vec![TileState::Missing; n as usize];
        // Positions on the old layout mean nothing on the new one.
        self.selection.set_vec(Vec::new());
        // A relayout keeps the focused link (the same links, moved).
        let relayout = self.relayout;
        self.set_links(links, relayout);
        self.page = Some(page);
        self.fallback_timer.stop();
        if let Some(ui) = self.ui() {
            let [r, g, b, _] = page.bg;
            ui.set_r_page_width(page.width);
            ui.set_r_page_height(page.height);
            // A relayout keeps the sideways scroll only as far as it goes.
            let min_x = (ui.get_r_body_width() - page.width).min(0.0);
            ui.set_r_page_x(ui.get_r_page_x().clamp(min_x, 0.0));
            ui.set_r_paper(slint::Color::from_rgb_u8(r, g, b));
            ui.set_r_truncated(page.truncated);
            let blocked = if self.remote { 0 } else { page.remote_images };
            ui.set_r_remote_blocked(blocked.try_into().unwrap_or(i32::MAX));
            // After consent: the images that could not be loaded, once none
            // is still coming.
            let failed = if self.remote && !page.pending_images {
                page.failed_images
            } else {
                0
            };
            ui.set_r_remote_failed(failed.try_into().unwrap_or(i32::MAX));
            ui.set_r_dimmed(page.dimmed);
            ui.set_r_original(self.original);
            ui.set_r_note(SharedString::new());
            ui.set_r_waiting(false);
            ui.set_r_html(true);
            self.failed = None;
        }
        self.request_tiles();
    }

    /// What the note says of a failed render (empty: nothing).
    fn failed_note(error: RenderError) -> String {
        match error {
            RenderError::TooLarge => t!(invoke_reader_too_large),
            RenderError::Deadline => t!(invoke_reader_too_slow),
            RenderError::Malformed | RenderError::NoHtml => String::new(),
        }
    }

    fn show_failed(&mut self, error: RenderError) {
        self.failed = Some(error);
        self.page = None;
        self.fallback_timer.stop();
        self.tiles.set_vec(Vec::new());
        self.state.clear();
        self.selection.set_vec(Vec::new());
        self.set_links(Vec::new(), false);
        let note = Self::failed_note(error);
        if let Some(ui) = self.ui() {
            ui.set_r_note(note.into());
            ui.set_r_html(false);
            ui.set_r_waiting(false);
            ui.set_r_remote_blocked(0);
            ui.set_r_remote_failed(0);
            ui.set_r_dimmed(false);
        }
    }

    fn show_tile(&mut self, index: u32, pixels: SharedPixelBuffer<Rgba8Pixel>) {
        let i = index as usize;
        let Some(mut tile) = self.tiles.row_data(i) else {
            return;
        };
        tile.img = Image::from_rgba8_premultiplied(pixels);
        self.tiles.set_row_data(i, tile);
        if let Some(st) = self.state.get_mut(i) {
            *st = TileState::Shown;
        }
        if let Some(opened) = self.opened.take()
            && self.timing
        {
            eprintln!(
                "[timing] open to first tile: {:.1} ms",
                opened.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// A button press on the page: a caret, or a word or paragraph when it
    /// follows the previous press closely enough.
    fn press(&mut self, x: f32, y: f32) {
        let now = Instant::now();
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
    fn extend(&mut self, x: f32, y: f32) {
        let near = self.last_press.is_some_and(|(_, px, py, _)| {
            (x - px).abs() <= MULTI_CLICK_SLOP && (y - py).abs() <= MULTI_CLICK_SLOP
        });
        if !near {
            self.pressed_link = None;
        }
        self.select(SelectOp::Extend { x, y });
    }

    // ---------------------------------------------------------- links

    /// Show `links` for the page; `keep_focus` keeps the focused link when
    /// the set is the same.
    fn set_links(&mut self, links: Vec<PageLink>, keep_focus: bool) {
        let same = keep_focus
            && links.len() == self.links.len()
            && links.iter().zip(&self.links).all(|(a, b)| a.href == b.href);
        let focus = self.focus.filter(|_| same);
        self.links = links;
        self.links_model.set_vec(
            self.links
                .iter()
                .filter_map(|l| {
                    let r = l.rects.first()?;
                    Some(LinkItem {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                        text: if l.text.is_empty() { &l.href } else { &l.text }.into(),
                        href: l.href.as_str().into(),
                    })
                })
                .collect::<Vec<_>>(),
        );
        self.hover = None;
        self.pressed_link = None;
        self.focus = None;
        self.set_focus(focus);
    }

    /// The link under page point (x, y): the topmost hit area.
    fn link_at(&self, x: f32, y: f32) -> Option<usize> {
        self.page?;
        self.links
            .iter()
            .rposition(|l| l.rects.iter().any(|r| contains(r, x, y)))
    }

    /// The pointer moved over the page (negative: it left).
    fn hover_at(&mut self, x: f32, y: f32) {
        let hover = self.link_at(x, y);
        if hover != self.hover {
            self.hover = hover;
            self.show_target();
        }
    }

    /// The bottom strip shows the hovered link, else the focused one.
    fn show_target(&self) {
        let Some(ui) = self.ui() else {
            return;
        };
        let href = self
            .hover
            .or(self.focus)
            .and_then(|i| self.links.get(i))
            .map_or("", |l| l.href.as_str());
        let shown: String = href.chars().take(MAX_SHOWN_URL).collect();
        ui.set_r_hover_link(self.hover.is_some());
        ui.set_r_hover_href(shown.into());
    }

    /// The button came up: a click on the link it went down on (with no
    /// drag in between, see `extend`) opens it.
    fn release(&mut self, x: f32, y: f32) {
        let Some(i) = self.pressed_link.take() else {
            return;
        };
        if self.link_at(x, y) == Some(i) {
            self.activate(i);
        }
    }

    /// Move keyboard focus to the next (or previous) link; `false` once it
    /// runs off either end, so focus can leave the page.
    fn focus_next(&mut self, back: bool) -> bool {
        let plain = self.ui().is_none_or(|ui| ui.get_r_plain());
        let n = self.links.len();
        if plain || self.page.is_none() || n == 0 {
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

    /// Focus link `focus`: draw its ring and scroll it into view.
    fn set_focus(&mut self, focus: Option<usize>) {
        self.focus = focus;
        let rects = focus
            .and_then(|i| self.links.get(i))
            .map_or(&[][..], |l| &l.rects[..]);
        self.focus_model.set_vec(
            rects
                .iter()
                .map(|r| SelRect {
                    x: r.x - 2.0,
                    y: r.y - 1.0,
                    w: r.w + 4.0,
                    h: r.h + 2.0,
                })
                .collect::<Vec<_>>(),
        );
        if let (Some(r), Some(ui)) = (rects.first(), self.ui()) {
            let (top, height) = (ui.get_r_view_top(), ui.get_r_view_height());
            if r.y < top {
                self.scroll_to(r.y - 16.0);
            } else if r.y + r.h > top + height {
                self.scroll_to(r.y + r.h + 16.0 - height);
            }
        }
        self.show_target();
    }

    /// Scroll so that page y `top` is at the top of the view (as far as
    /// the content allows).
    fn scroll_to(&self, top: f32) {
        let Some(ui) = self.ui() else {
            return;
        };
        let y = ui.get_r_content_y() - (top - ui.get_r_view_top());
        ui.set_r_content_y(y.clamp(ui.get_r_scroll_min().min(0.0), 0.0));
    }

    /// Follow link `i`: scroll to an anchor, prefill the composer for
    /// `mailto:`, or ask before opening a web link in the browser.
    fn activate(&mut self, i: usize) {
        let Some(link) = self.links.get(i) else {
            return;
        };
        if link.href.starts_with('#') {
            if let Some(y) = link.target {
                self.scroll_to(y - 8.0);
            }
        } else {
            let (href, text) = (link.href.clone(), link.text.clone());
            self.follow(&href, &text);
        }
    }

    /// Prefill the composer for a `mailto:` address, or ask before opening
    /// a web address (shown as `text`) in the browser.
    fn follow(&mut self, href: &str, text: &str) {
        if let Some(m) = links::mailto(href) {
            if let Some(f) = self.mailto.clone() {
                Timer::single_shot(Duration::ZERO, move || f(m));
            }
        } else if let Some(web) = links::web(href, text) {
            let warnings: Vec<SharedString> = web
                .warnings
                .iter()
                .map(|&w| warning_text(w, &web, text).into())
                .collect();
            if let Some(ui) = self.ui() {
                let cap = |s: &str| -> SharedString {
                    s.chars().take(MAX_SHOWN_URL).collect::<String>().into()
                };
                ui.set_l_before(cap(&web.before));
                ui.set_l_host(cap(&web.host));
                ui.set_l_after(cap(&web.after));
                ui.set_l_warnings(Rc::new(VecModel::from(warnings)).into());
                ui.set_l_show(true);
            }
            self.pending = Some(web.url);
        }
    }

    /// Open the web link the user confirmed.
    fn open_pending(&mut self) {
        if let Some(url) = self.pending.take() {
            self.shell.open_url(&url);
        }
        self.cancel_pending();
    }

    fn cancel_pending(&mut self) {
        self.pending = None;
        if let Some(ui) = self.ui()
            && ui.get_l_show()
        {
            ui.set_l_show(false);
        }
    }

    /// Send a selection gesture or command for the page shown.
    fn select(&mut self, op: SelectOp) {
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

    /// Ask for the tiles near the view that are not there yet, and drop
    /// the ones far from it.
    fn request_tiles(&mut self) {
        let (Some(page), Some(key), Some(ui)) = (self.page, self.key, self.ui()) else {
            return;
        };
        // The plain-text view hides the page; its tiles wait until it is back.
        if ui.get_r_plain() {
            return;
        }
        // A scale change shows first as a scroll or resize callback.
        if self.current_key() != Some(key) {
            self.body_resized();
        }
        let (top, height) = (ui.get_r_view_top(), ui.get_r_view_height());
        let keep = tile_range(
            top - KEEP * height,
            top + (1.0 + KEEP) * height,
            page.height,
        );
        for (i, st) in self.state.iter_mut().enumerate() {
            let near = keep.is_some_and(|(a, b)| (a as usize..=b as usize).contains(&i));
            if !near && *st != TileState::Missing {
                *st = TileState::Missing;
                if let Some(mut tile) = self.tiles.row_data(i) {
                    tile.img = Image::default();
                    self.tiles.set_row_data(i, tile);
                }
            }
        }
        let wanted: Vec<u32> = wanted_tiles(top, height, page.height)
            .into_iter()
            .filter(|&i| {
                let st = self.state.get_mut(i as usize);
                match st {
                    Some(st) if *st == TileState::Missing => {
                        *st = TileState::Requested;
                        true
                    }
                    _ => false,
                }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Rgb8Pixel;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, WindowAdapter, WindowEvent};

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
    fn lru_accounts_bytes_and_evicts_oldest() {
        let mut c = Lru::new(10);
        c.insert(1, 'a', 4);
        c.insert(2, 'b', 4);
        assert_eq!(c.cost, 8);
        assert_eq!(c.get(&1), Some(&'a'));
        // Over the cap: the least recently used entry (2) goes.
        c.insert(3, 'c', 4);
        assert_eq!(c.cost, 8);
        assert_eq!(c.get(&2), None);
        // Replacing an entry swaps its cost.
        c.insert(1, 'A', 6);
        assert_eq!(c.cost, 10);
        assert_eq!(c.get(&1), Some(&'A'));
        // An entry over the cap on its own is kept, alone.
        c.insert(4, 'd', 20);
        assert_eq!((c.cost, c.entries.len()), (20, 1));

        // The tile cache holds about 24 full-width tiles at 100%.
        let tile = 960 * 256 * 4;
        let mut tiles = Lru::new(TILE_CACHE_BYTES);
        for i in 0..40 {
            tiles.insert(i, (), tile);
        }
        let kept = TILE_CACHE_BYTES / tile;
        assert_eq!(tiles.entries.len(), kept);
        assert!(tiles.cost <= TILE_CACHE_BYTES);
        assert_eq!(tiles.entries.front().map(|e| e.0), Some(40 - kept));
    }

    fn key(msg: i64, width: u32) -> Key {
        Key {
            msg,
            width,
            scale: 1f32.to_bits(),
            dark: false,
            dim: false,
            remote: false,
        }
    }

    /// The worker loop without remote images.
    fn run_plain(rx: Receiver<Job>, latest: &AtomicU64, notify: &dyn Fn(Reply)) {
        run(
            rx,
            latest,
            notify,
            &Mutex::default(),
            &|_, _| {},
            (DOC_CACHE, TILE_CACHE_BYTES),
        );
    }

    fn html(body: &str) -> Source {
        Source::Input(Arc::new(RenderInput {
            html: Some(body.into()),
            ..Default::default()
        }))
    }

    #[test]
    fn worker_drops_stale_generations() {
        let (tx, rx) = mpsc::channel();
        let raw: &[u8] = b"Content-Type: text/html\r\n\r\n<p>new</p>";
        let jobs = [
            Job::Layout {
                generation: 1,
                key: key(1, 320),
                src: html("<p>old</p>"),
            },
            Job::Tiles {
                generation: 1,
                key: key(1, 320),
                tiles: vec![0],
            },
            Job::Layout {
                generation: 2,
                key: key(2, 480),
                src: Source::Raw(Arc::from(raw)),
            },
            Job::Tiles {
                generation: 2,
                key: key(2, 480),
                tiles: vec![0, 1],
            },
        ];
        for job in jobs {
            tx.send(job).unwrap();
        }
        drop(tx);
        // The UI side has moved on to generation 2 already.
        let latest = AtomicU64::new(2);
        let replies = RefCell::new(Vec::new());
        run_plain(rx, &latest, &|r| replies.borrow_mut().push(r));
        let replies = replies.into_inner();
        assert_eq!(replies.len(), 3, "parsed, page and one tile");
        assert!(
            matches!(&replies[0], Reply::Parsed { generation: 2, body } if body.text.contains("new"))
        );
        assert!(
            matches!(&replies[1], Reply::Page { generation: 2, page, .. } if page.width == 480.0)
        );
        // The page is one tile tall; tile 1 does not exist.
        assert!(matches!(
            &replies[2],
            Reply::Tile { generation: 2, index: 0, pixels } if pixels.width() == 480
        ));
        assert!(current(3, &latest) && current(2, &latest) && !current(1, &latest));
    }

    #[test]
    fn worker_reports_text_fallbacks() {
        let (tx, rx) = mpsc::channel();
        let plain = Source::Input(Arc::new(RenderInput {
            text: "hi".into(),
            prefers_plain: true,
            ..Default::default()
        }));
        let big = html(&"x".repeat(render::MAX_HTML_BYTES + 1));
        for (generation, src) in [(1, plain), (2, big)] {
            let key = key(generation as i64, 480);
            tx.send(Job::Layout {
                generation,
                key,
                src,
            })
            .unwrap();
        }
        drop(tx);
        let replies = RefCell::new(Vec::new());
        run_plain(rx, &AtomicU64::new(0), &|r| replies.borrow_mut().push(r));
        let errors: Vec<_> = replies
            .into_inner()
            .into_iter()
            .map(|r| match r {
                Reply::Failed { error, .. } => Some(error),
                _ => None,
            })
            .collect();
        assert_eq!(
            errors,
            [Some(RenderError::NoHtml), Some(RenderError::TooLarge)]
        );
    }

    // ------------------------------------------------- headless window

    struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<RefCell<Option<String>>>);

    impl TestPlatform {
        fn new(window: Rc<MinimalSoftwareWindow>) -> Self {
            TestPlatform(window, Rc::default())
        }
    }

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
        fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
            if clipboard == slint::platform::Clipboard::DefaultClipboard {
                *self.1.borrow_mut() = Some(text.into());
            }
        }
    }

    /// Records jobs; the test plays the worker.
    struct Fake(Rc<RefCell<Vec<Job>>>);

    impl Backend for Fake {
        fn send(&mut self, job: Job) {
            self.0.borrow_mut().push(job);
        }
    }

    /// Records the URLs it is asked to open.
    #[derive(Clone, Default)]
    struct NoShell(Rc<RefCell<Vec<String>>>);

    impl Shell for NoShell {
        fn open_url(&self, url: &str) {
            self.0.borrow_mut().push(url.into());
        }
    }

    /// A flat magenta tile, as the fake renderer draws it.
    fn flat_tile(width: f32, h: f32) -> SharedPixelBuffer<Rgba8Pixel> {
        let mut buf = SharedPixelBuffer::new(width.round() as u32, h.round() as u32);
        buf.make_mut_slice().fill(Rgba8Pixel {
            r: 255,
            g: 0,
            b: 255,
            a: 255,
        });
        buf
    }

    fn layouts(jobs: Vec<Job>) -> Vec<(u64, Key)> {
        jobs.into_iter()
            .filter_map(|j| match j {
                Job::Layout {
                    generation, key, ..
                } => Some((generation, key)),
                _ => None,
            })
            .collect()
    }

    fn tile_jobs(jobs: Vec<Job>) -> Vec<(u64, Vec<u32>)> {
        jobs.into_iter()
            .filter_map(|j| match j {
                Job::Tiles {
                    generation, tiles, ..
                } => Some((generation, tiles)),
                _ => None,
            })
            .collect()
    }

    fn shown(ui: &AppWindow, row: u32) -> bool {
        ui.get_r_tiles()
            .row_data(row as usize)
            .is_some_and(|t| t.img.size().width > 0)
    }

    #[test]
    fn tile_requests_follow_scroll_and_resize() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform::new(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1200, 800));
        ui.show().unwrap();
        ui.set_has_message(true);
        // Run change handlers and timers and draw a frame; returns how many
        // pixels show a fake tile.
        let frame = || {
            let mut magenta = 0;
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    let size = window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    let mut buf = vec![Rgb8Pixel::default(); w * h];
                    r.render(&mut buf, w);
                    magenta = buf
                        .iter()
                        .filter(|p| (p.r, p.g, p.b) == (255, 0, 255))
                        .count();
                });
            }
            magenta
        };
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let take = || std::mem::take(&mut *jobs.borrow_mut());
        let reader = Reader::new(
            &ui,
            Box::new(Fake(jobs.clone())),
            Box::new(NoShell::default()),
        );
        frame();
        let body = ui.get_r_body_width();
        assert!(body > 400.0, "body column {body}");

        reader.open(7, html("<p>x</p>"), false);
        assert!(ui.get_r_waiting() && !ui.get_r_html());
        let [(generation, key)] = layouts(take())[..] else {
            panic!("expected one layout job");
        };
        assert_eq!(key.width, page_width(body).unwrap());
        assert_eq!(key.scale(), 1.0);
        let page = Page {
            width: key.width as f32,
            height: 5000.0,
            bg: [255; 4],
            truncated: false,
            remote_images: 0,
            pending_images: false,
            failed_images: 0,
            dimmed: false,
        };
        reader.on_reply(Reply::Page {
            generation,
            page,
            links: vec![],
        });
        assert!(ui.get_r_html() && !ui.get_r_waiting());
        assert_eq!(ui.get_r_tiles().row_count(), 20);

        // The header sits above the page, so the view starts above it.
        let (top, height) = (ui.get_r_view_top(), ui.get_r_view_height());
        assert!((-400.0..0.0).contains(&top), "view top {top}");
        assert!((500.0..800.0).contains(&height), "view height {height}");
        let requests = tile_jobs(take());
        let [(g, ref tiles)] = requests[..] else {
            panic!("expected one tile job");
        };
        assert_eq!(g, generation);
        assert_eq!(tiles, &wanted_tiles(top, height, page.height));
        assert_eq!(tiles[0], 0);
        for &index in tiles {
            let (t, b) = tile_span(index, page.height).unwrap();
            let pixels = flat_tile(page.width, b - t);
            reader.on_reply(Reply::Tile {
                generation,
                index,
                pixels,
            });
        }
        assert!(frame() > 10_000, "tiles are drawn");
        frame();
        assert!(take().is_empty(), "nothing re-requested");

        // Scroll down: the tiles under the new view come first, and those
        // far above are dropped.
        ui.set_r_content_y(-3000.0);
        frame();
        let new_top = ui.get_r_view_top();
        assert_eq!(new_top, top + 3000.0);
        let requests = tile_jobs(take());
        let [(_, ref tiles)] = requests[..] else {
            panic!("expected one tile job after scrolling");
        };
        assert_eq!(tiles[0], (new_top / TILE_H) as u32);
        assert_eq!(tiles, &wanted_tiles(new_top, height, page.height));
        assert!(!shown(&ui, 0));
        assert_eq!(frame(), 0, "nothing in view is rasterized yet");
        let visible = tiles[0];
        let (t, b) = tile_span(visible, page.height).unwrap();
        reader.on_reply(Reply::Tile {
            generation,
            index: visible,
            pixels: flat_tile(page.width, b - t),
        });
        assert!(frame() > 10_000);

        // Narrow the window: one relayout once the resize pauses.
        ui.window().set_size(slint::PhysicalSize::new(1100, 800));
        frame();
        assert!(layouts(take()).is_empty(), "debounced");
        std::thread::sleep(RESIZE_DEBOUNCE + Duration::from_millis(50));
        frame();
        let [(g2, k2)] = layouts(take())[..] else {
            panic!("expected a relayout");
        };
        assert_eq!(g2, generation + 1);
        assert!(k2.width < key.width);
        assert_eq!(k2.width, page_width(ui.get_r_body_width()).unwrap());
        // A tile of the old generation arriving late is ignored.
        reader.on_reply(Reply::Tile {
            generation,
            index: visible + 1,
            pixels: flat_tile(page.width, TILE_H),
        });
        assert!(!shown(&ui, visible + 1));
        // The old tiles stay up, stretched, until new ones arrive.
        let page2 = Page {
            width: k2.width as f32,
            height: 6000.0,
            ..page
        };
        reader.on_reply(Reply::Page {
            generation: g2,
            page: page2,
            links: vec![],
        });
        assert_eq!(ui.get_r_tiles().row_count(), 24);
        assert!(shown(&ui, visible));
        assert!(frame() > 10_000);
        let requests = tile_jobs(take());
        assert!(matches!(&requests[..], [(g, tiles)] if *g == g2 && tiles.contains(&visible)));

        // A display scale change re-renders at the new scale (the window
        // keeps its logical size, as on a move to another monitor).
        ui.window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
        ui.window().set_size(slint::PhysicalSize::new(1650, 1200));
        frame();
        std::thread::sleep(RESIZE_DEBOUNCE + Duration::from_millis(50));
        frame();
        let relayouts = layouts(take());
        assert_eq!(relayouts.len(), 1);
        assert_eq!(relayouts[0].1.scale(), 1.5);
        assert_eq!(relayouts[0].1.width, k2.width);

        // No page within a second: the text view stands in.
        reader.open(8, html("<p>y</p>"), false);
        assert!(ui.get_r_waiting());
        std::thread::sleep(FALLBACK_AFTER + Duration::from_millis(50));
        frame();
        assert!(!ui.get_r_waiting() && !ui.get_r_html());
        reader.close();
        assert!(matches!(take().last(), Some(Job::Cancel { .. })));
    }

    // ----------------------------------------------------------- selection

    fn corpus_12() -> Arc<[u8]> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/html/12-personal-applemail.eml");
        std::fs::read(path).unwrap().into()
    }

    /// What the worker answers to selection `ops` on corpus 12 at 600 px:
    /// highlight rectangle counts (`Ok`) and copied texts (`Err`), in order.
    fn select_on_12(ops: Vec<SelectOp>) -> Vec<Result<usize, String>> {
        std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let (tx, rx) = mpsc::channel();
                let key = key(12, 600);
                let src = Source::Raw(corpus_12());
                tx.send(Job::Layout {
                    generation: 1,
                    key,
                    src,
                })
                .unwrap();
                for op in ops {
                    tx.send(Job::Select {
                        generation: 1,
                        key,
                        op,
                    })
                    .unwrap();
                }
                drop(tx);
                let replies = RefCell::new(Vec::new());
                run_plain(rx, &AtomicU64::new(1), &|r| match r {
                    Reply::Selection { rects, .. } => replies.borrow_mut().push(Ok(rects.len())),
                    Reply::Copied { text, .. } => replies.borrow_mut().push(Err(text)),
                    _ => {}
                });
                replies.into_inner()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    /// Corpus 12 laid out as the worker does for `key(12, 600)`.
    fn doc_12() -> Doc {
        std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(|| {
                let input = mime::parse_render_input(&corpus_12());
                lay_out(&input, key(12, 600), &mut Engine::new(), &Images::default()).unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn worker_selects_and_copies_across_layouts() {
        let doc = doc_12();
        let lines = doc.text_lines();
        let find = |p: &str| lines.iter().find(|l| l.text.starts_with(p)).unwrap();
        let (first, last) = (find("Olá Ana"), find("Ligas-me"));
        // The point on the "Envio" line where a double-click takes "Gerês".
        let envio = find("Envio");
        let geres = (1..100)
            .map(|i| {
                (
                    envio.x + envio.w * i as f32 / 100.0,
                    envio.y + envio.h / 2.0,
                )
            })
            .find(|&(x, y)| {
                doc.word_at(x, y)
                    .is_some_and(|(a, b)| doc.selection_text(a, b).trim_end() == "Gerês")
            })
            .unwrap();
        let replies = select_on_12(vec![
            // Drag from the left margin of the first line to past the end
            // of the fourth paragraph, then copy.
            SelectOp::Press {
                x: first.x - 4.0,
                y: first.y + 2.0,
                clicks: 1,
            },
            SelectOp::Extend {
                x: last.x + last.w + 30.0,
                y: last.y + 2.0,
            },
            SelectOp::Copy,
            // A double-click takes one word.
            SelectOp::Press {
                x: geres.0,
                y: geres.1,
                clicks: 2,
            },
            SelectOp::Copy,
            // Nothing selected: nothing to copy.
            SelectOp::Clear,
            SelectOp::Copy,
            SelectOp::All,
        ]);
        let [
            Ok(0),
            Ok(dragged),
            Err(copied),
            Ok(1),
            Err(word),
            Ok(0),
            Ok(all),
        ] = &replies[..]
        else {
            panic!("unexpected replies {replies:?}");
        };
        assert!(*dragged >= 4);
        assert_eq!(
            copied,
            "Olá Ana,\nObrigada pelo jantar de sábado - foi mesmo bom rever-te! 😊\n\
             Envio as fotos do Gerês neste álbum. A da cascata ficou incrível.\n\
             Ligas-me amanhã? Estou livre depois das 18h."
        );
        assert_eq!(word.trim_end(), "Gerês");
        assert!(*all > *dragged);
    }

    #[test]
    fn a_new_layout_drops_the_worker_selection() {
        let copied = std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(|| {
                let (tx, rx) = mpsc::channel();
                let key = key(12, 600);
                let layout = || Job::Layout {
                    generation: 1,
                    key,
                    src: Source::Raw(corpus_12()),
                };
                let select = |op| Job::Select {
                    generation: 1,
                    key,
                    op,
                };
                for job in [
                    layout(),
                    select(SelectOp::All),
                    layout(),
                    select(SelectOp::Copy),
                ] {
                    tx.send(job).unwrap();
                }
                drop(tx);
                let copied = RefCell::new(false);
                run_plain(rx, &AtomicU64::new(1), &|r| {
                    if matches!(r, Reply::Copied { .. }) {
                        *copied.borrow_mut() = true;
                    }
                });
                copied.into_inner()
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(!copied);
    }

    #[test]
    fn page_gestures_and_keys_reach_the_worker_and_clipboard() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let platform = TestPlatform::new(window.clone());
        let clipboard = platform.1.clone();
        slint::platform::set_platform(Box::new(platform)).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1200, 800));
        ui.show().unwrap();
        ui.set_has_message(true);
        let deleted = Rc::new(std::cell::Cell::new(0));
        let d = deleted.clone();
        ui.on_delete(move || d.set(d.get() + 1));
        let frame = || {
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    let size = window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    let mut buf = vec![Rgb8Pixel::default(); w * h];
                    r.render(&mut buf, w);
                });
            }
        };
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let take = || std::mem::take(&mut *jobs.borrow_mut());
        let selects = || -> Vec<SelectOp> {
            take()
                .into_iter()
                .filter_map(|j| match j {
                    Job::Select { op, .. } => Some(op),
                    _ => None,
                })
                .collect()
        };
        let reader = Reader::new(
            &ui,
            Box::new(Fake(jobs.clone())),
            Box::new(NoShell::default()),
        );
        frame();
        reader.open(7, html("<p>x</p>"), false);
        let [(generation, key)] = layouts(take())[..] else {
            panic!("expected one layout job");
        };
        let page = Page {
            width: key.width as f32,
            height: 2000.0,
            bg: [255; 4],
            truncated: false,
            remote_images: 0,
            pending_images: false,
            failed_images: 0,
            dimmed: false,
        };
        reader.on_reply(Reply::Page {
            generation,
            page,
            links: vec![],
        });
        frame();
        take();

        // Press, drag and release over the page: page coordinates, which
        // move with the pointer.
        let win = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
        let left = slint::platform::PointerEventButton::Left;
        let (wx, wy) = (900.0, 500.0);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: win(wx, wy),
            button: left,
        });
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: win(wx + 50.0, wy + 60.0),
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: win(wx + 50.0, wy + 60.0),
            button: left,
        });
        let ops = selects();
        let [
            SelectOp::Press { x, y, clicks: 1 },
            SelectOp::Extend { x: x2, y: y2 },
        ] = ops[..]
        else {
            panic!("unexpected ops {ops:?}");
        };
        assert!((0.0..page.width).contains(&x) && (0.0..page.height).contains(&y));
        assert_eq!((x2 - x, y2 - y), (50.0, 60.0));
        // The header above the page is in view, so the page y is less than
        // the window y by more than the command bar.
        assert!(ui.get_r_view_top() < 0.0 && y < wy - 100.0, "y {y}");

        // Highlights from the worker show over the page.
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 18.0,
        };
        reader.on_reply(Reply::Selection {
            generation,
            rects: vec![rect],
        });
        assert_eq!(ui.get_r_selection().row_count(), 1);
        // A stale reply is ignored.
        reader.on_reply(Reply::Selection {
            generation: generation - 1,
            rects: vec![],
        });
        assert_eq!(ui.get_r_selection().row_count(), 1);

        // A second and third press in place: word, then paragraph.
        for _ in 0..2 {
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: win(wx, wy),
                button: left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: win(wx, wy),
                button: left,
            });
        }
        let clicks: Vec<u8> = selects()
            .into_iter()
            .filter_map(|op| match op {
                SelectOp::Press { clicks, .. } => Some(clicks),
                _ => None,
            })
            .collect();
        assert_eq!(clicks, [2, 3]);

        // The click focused the page: Ctrl+A, Ctrl+C and Escape act on the
        // selection, and other keys still reach the window's bindings.
        let press_key = |text: SharedString, ctrl: bool| {
            let control: SharedString = slint::platform::Key::Control.into();
            if ctrl {
                ui.window().dispatch_event(WindowEvent::KeyPressed {
                    text: control.clone(),
                });
            }
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
            if ctrl {
                ui.window()
                    .dispatch_event(WindowEvent::KeyReleased { text: control });
            }
        };
        press_key("a".into(), true);
        press_key("c".into(), true);
        press_key(slint::platform::Key::Escape.into(), false);
        assert_eq!(selects(), [SelectOp::All, SelectOp::Copy, SelectOp::Clear]);
        press_key(slint::platform::Key::Delete.into(), false);
        assert_eq!(deleted.get(), 1);

        // Copied text lands on the clipboard through the hidden buffer.
        reader.on_reply(Reply::Copied {
            generation,
            text: "Olá Ana,\nObrigada".into(),
        });
        assert_eq!(clipboard.borrow().as_deref(), Some("Olá Ana,\nObrigada"));

        // The plain-text view hides the page and stops tile requests until
        // it is turned off again.
        ui.set_r_plain(true);
        frame();
        take();
        ui.set_r_content_y(-1000.0);
        frame();
        assert!(tile_jobs(take()).is_empty());
        ui.set_r_plain(false);
        frame();
        assert!(!tile_jobs(take()).is_empty());

        // A new message drops the highlights.
        reader.on_reply(Reply::Selection {
            generation,
            rects: vec![rect],
        });
        reader.open(8, html("<p>y</p>"), false);
        assert_eq!(ui.get_r_selection().row_count(), 0);
    }

    // --------------------------------------------------------------- links

    #[test]
    fn worker_sends_links_with_text_and_anchors() {
        let replies = std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(|| {
                let (tx, rx) = mpsc::channel();
                tx.send(Job::Layout {
                    generation: 1,
                    key: key(3, 600),
                    src: html(
                        "<p><a href='#end'>Jump <b>down</b></a> or \
                         <a href=' https://example.com/a b '>example.com</a></p>\
                         <div style='height:3000px'></div>\
                         <table><tr><td><h2 id='end'>The end</h2></td></tr></table>\
                         <p><a href='mailto:a@example.com?subject=Hi'>Write</a> \
                         <a href='java&#9;script:x'>bad</a> <a href='#nowhere'>gone</a></p>",
                    ),
                })
                .unwrap();
                drop(tx);
                let replies = RefCell::new(Vec::new());
                run_plain(rx, &AtomicU64::new(1), &|r| replies.borrow_mut().push(r));
                replies.into_inner()
            })
            .unwrap()
            .join()
            .unwrap();
        let [Reply::Page { links, .. }] = &replies[..] else {
            panic!("expected a page");
        };
        let hrefs: Vec<&str> = links.iter().map(|l| l.href.as_str()).collect();
        assert_eq!(
            hrefs,
            [
                "#end",
                "https://example.com/a%20b",
                "mailto:a@example.com?subject=Hi",
                "#nowhere"
            ]
        );
        assert_eq!(links[0].text, "Jump down");
        assert_eq!(links[1].text, "example.com");
        assert!(links.iter().all(|l| !l.rects.is_empty()));
        // The anchor sits inside a table cell, below the 3000 px spacer.
        let end = links[0].target.unwrap();
        assert!(end > 3000.0 && end < links[2].rects[0].y, "anchor at {end}");
        assert_eq!(links[3].target, None);
    }

    #[test]
    fn links_hover_open_compose_and_take_keyboard_focus() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform::new(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1200, 800));
        ui.show().unwrap();
        ui.set_has_message(true);
        let frame = || {
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    let size = window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    let mut buf = vec![Rgb8Pixel::default(); w * h];
                    r.render(&mut buf, w);
                });
            }
        };
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let shell = NoShell::default();
        let opened = shell.0.clone();
        let reader = Reader::new(&ui, Box::new(Fake(jobs.clone())), Box::new(shell));
        let mailed = Rc::new(RefCell::new(Vec::new()));
        let m = mailed.clone();
        reader.on_mailto(move |mail| m.borrow_mut().push(mail));
        frame();
        reader.open(7, html("<p>x</p>"), false);
        let [(generation, key)] = layouts(std::mem::take(&mut *jobs.borrow_mut()))[..] else {
            panic!("expected one layout job");
        };
        let link = |href: &str, text: &str, y: f32, target: Option<f32>| PageLink {
            href: href.into(),
            text: text.into(),
            rects: vec![Rect {
                x: 20.0,
                y,
                w: 120.0,
                h: 20.0,
            }],
            target,
        };
        let page = Page {
            width: key.width as f32,
            height: 3000.0,
            bg: [255; 4],
            truncated: false,
            remote_images: 0,
            pending_images: false,
            failed_images: 0,
            dimmed: false,
        };
        reader.on_reply(Reply::Page {
            generation,
            page,
            links: vec![
                link(
                    "https://evil.example.invalid/x",
                    "https://bank.example.com",
                    20.0,
                    None,
                ),
                link("mailto:ana@example.com?subject=Ol%C3%A1", "Ana", 60.0, None),
                link("#end", "End", 100.0, Some(1500.0)),
            ],
        });
        frame();
        assert_eq!(ui.get_r_links().row_count(), 3);

        // Where page coordinates are in the window: a press reports them.
        let win = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
        let left = slint::platform::PointerEventButton::Left;
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: win(900.0, 500.0),
            button: left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: win(900.0, 500.0),
            button: left,
        });
        let (dx, dy) = jobs
            .borrow()
            .iter()
            .find_map(|j| match j {
                Job::Select {
                    op: SelectOp::Press { x, y, .. },
                    ..
                } => Some((900.0 - x, 500.0 - y)),
                _ => None,
            })
            .unwrap();
        let on = |y: f32| win(dx + 50.0, dy + y + 10.0);
        let click = |y: f32| {
            // Past the double-click interval, so each is a single click.
            std::thread::sleep(MULTI_CLICK);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: on(y),
                button: left,
            });
            ui.window().dispatch_event(WindowEvent::PointerReleased {
                position: on(y),
                button: left,
            });
            frame();
        };

        // Hover: pointer cursor and the real target in the strip.
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position: on(20.0) });
        frame();
        assert!(ui.get_r_hover_link());
        assert_eq!(ui.get_r_hover_href(), "https://evil.example.invalid/x");
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: on(400.0),
        });
        frame();
        assert!(!ui.get_r_hover_link());
        assert_eq!(ui.get_r_hover_href(), "");

        // A web link asks first, with the host apart and the warning.
        click(20.0);
        assert!(ui.get_l_show());
        assert_eq!(
            (ui.get_l_before(), ui.get_l_host(), ui.get_l_after()),
            (
                "https://".into(),
                "evil.example.invalid".into(),
                "/x".into()
            )
        );
        assert_eq!(ui.get_l_warnings().row_count(), 1);
        assert!(opened.borrow().is_empty());
        ui.invoke_l_cancel();
        assert!(!ui.get_l_show() && opened.borrow().is_empty());
        click(20.0);
        ui.invoke_l_open();
        assert!(!ui.get_l_show());
        assert_eq!(*opened.borrow(), ["https://evil.example.invalid/x"]);

        // A drag that starts on a link selects instead of opening it.
        std::thread::sleep(MULTI_CLICK);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: on(20.0),
            button: left,
        });
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: on(300.0),
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: on(20.0),
            button: left,
        });
        frame();
        assert!(!ui.get_l_show());

        // mailto: goes to the composer, decoded.
        click(60.0);
        assert!(!ui.get_l_show());
        let mail = mailed.borrow_mut().pop().unwrap();
        assert_eq!(
            (mail.to.as_str(), mail.subject.as_str()),
            ("ana@example.com", "Olá")
        );

        // The clicks focused the page: Tab walks the links, Enter follows
        // the focused one, and Tab past the last lets focus leave. (The
        // strip shows the hovered link first, so the pointer moves off.)
        ui.window().dispatch_event(WindowEvent::PointerMoved {
            position: on(400.0),
        });
        let press = |text: SharedString| {
            ui.window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(WindowEvent::KeyReleased { text });
            frame();
        };
        let tab: SharedString = slint::platform::Key::Tab.into();
        press(tab.clone());
        assert_eq!(ui.get_r_focus().row_count(), 1);
        assert_eq!(ui.get_r_hover_href(), "https://evil.example.invalid/x");
        press(tab.clone());
        press(tab.clone());
        assert_eq!(ui.get_r_hover_href(), "#end");
        let before = ui.get_r_content_y();
        press(slint::platform::Key::Return.into());
        let top = ui.get_r_view_top();
        assert!(ui.get_r_content_y() < before, "scrolled down");
        assert!((top - 1492.0).abs() < 1.0, "anchor at the top: {top}");
        press(tab.clone());
        assert_eq!(ui.get_r_focus().row_count(), 0);
        assert_eq!(ui.get_r_hover_href(), "");
        // Shift+Tab from no focus starts at the last link (a click on empty
        // paper focuses the page again).
        click(400.0);
        press(slint::platform::Key::Backtab.into());
        assert_eq!(ui.get_r_hover_href(), "#end");
        // Escape drops the focus; a new message drops the links.
        press(slint::platform::Key::Escape.into());
        assert_eq!(ui.get_r_focus().row_count(), 0);
        reader.open(8, html("<p>y</p>"), false);
        assert_eq!(ui.get_r_links().row_count(), 0);
    }

    // ------------------------------------------------------ scale to fit

    /// A 600 px fixed-width table (painted, so its box is what overflows):
    /// two paragraphs, the second with a link.
    const FIXED: &str = "<table width='600' style='width:600px;background:#eef' cellpadding='0' \
         cellspacing='0'><tr><td><p>Alpha beta gamma delta epsilon zeta eta theta iota \
         kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega, \
         and enough words after them to wrap onto a second line.</p>\
         <p>Second paragraph with <a href='https://example.com/x'>a link</a> in it.</p>\
         </td></tr></table>";

    /// Run `jobs` through the worker loop; every reply, in order.
    fn worker_replies(jobs: Vec<Job>) -> Vec<Reply> {
        std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let (tx, rx) = mpsc::channel();
                for job in jobs {
                    tx.send(job).unwrap();
                }
                drop(tx);
                let replies = RefCell::new(Vec::new());
                run_plain(rx, &AtomicU64::new(1), &|r| replies.borrow_mut().push(r));
                replies.into_inner()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    fn layout_job(key: Key, body: &str) -> Job {
        Job::Layout {
            generation: 1,
            key,
            src: html(body),
        }
    }

    fn page_and_links(replies: &[Reply]) -> (Page, Vec<PageLink>) {
        replies
            .iter()
            .find_map(|r| match r {
                Reply::Page { page, links, .. } => Some((*page, links.clone())),
                _ => None,
            })
            .expect("a page")
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
    fn worker_scales_fixed_width_mail_to_the_column() {
        let tile = |replies: &[Reply]| {
            replies
                .iter()
                .find_map(|r| match r {
                    Reply::Tile { pixels, .. } => Some((pixels.width(), pixels.height())),
                    _ => None,
                })
                .unwrap()
        };
        let at = |width: u32| {
            let key = key(1, width);
            let tiles = Job::Tiles {
                generation: 1,
                key,
                tiles: vec![0],
            };
            worker_replies(vec![layout_job(key, FIXED), tiles])
        };
        let (wide, narrow) = (at(600), at(450));
        let ((p600, _), (p450, _)) = (page_and_links(&wide), page_and_links(&narrow));
        // At 450 px the 600 px table is drawn at 0.75, whole: the page is
        // the column wide and its tiles are rasterized at that width.
        assert_eq!(p600.width, 600.0);
        assert_eq!(p450.width, 450.0);
        assert!(
            (p450.height - p600.height * 0.75).abs() < 0.5,
            "{p450:?} {p600:?}"
        );
        assert_eq!(tile(&wide).0, 600);
        assert_eq!(tile(&narrow).0, 450);
        // At 125% the tile is the column's device width: drawn 1:1.
        let k125 = Key {
            scale: 1.25f32.to_bits(),
            ..key(5, 480)
        };
        let tiles = Job::Tiles {
            generation: 1,
            key: k125,
            tiles: vec![0],
        };
        let r125 = worker_replies(vec![layout_job(k125, FIXED), tiles]);
        assert_eq!(page_and_links(&r125).0.width, 480.0);
        assert_eq!(tile(&r125).0, 600);
        // Mail that reflows is laid out at the column, unscaled.
        let reflow = worker_replies(vec![layout_job(key(2, 450), "<p>Short text.</p>")]);
        assert_eq!(page_and_links(&reflow).0.width, 450.0);
        // Below the 0.6 floor the page is wider than the column.
        let huge =
            "<table style='width:1000px;background:#eef'><tr><td>Statement</td></tr></table>";
        let pinned = worker_replies(vec![layout_job(key(3, 320), huge)]);
        assert_eq!(page_and_links(&pinned).0.width, 600.0);
        // Content sized relative to the page (wider on a wider page) cannot
        // be made to fit: it stays at the column, as before.
        let relative = "<div style='width:300%;background:#eef'>Wide</div><p>Text</p>";
        let rel = worker_replies(vec![layout_job(key(4, 400), relative)]);
        assert_eq!(page_and_links(&rel).0.width, 400.0);
    }

    #[test]
    fn drag_at_scale_selects_the_same_text() {
        // Points on the page laid out at 600 px (scale 1).
        let doc = std::thread::Builder::new()
            .stack_size(WORKER_STACK)
            .spawn(|| {
                let input = RenderInput {
                    html: Some(FIXED.into()),
                    ..Default::default()
                };
                lay_out(&input, key(1, 600), &mut Engine::new(), &Images::default()).unwrap()
            })
            .unwrap()
            .join()
            .unwrap();
        let lines = doc.text_lines();
        let (first, second) = (&lines[0], &lines[1]);
        let from = (first.x + first.w * 0.3, first.y + first.h / 2.0);
        let to = (second.x + second.w * 0.5, second.y + second.h / 2.0);
        let drag = |width: u32, s: f32| {
            let key = key(1, width);
            let select = |op| Job::Select {
                generation: 1,
                key,
                op,
            };
            let replies = worker_replies(vec![
                layout_job(key, FIXED),
                select(SelectOp::Press {
                    x: from.0 * s,
                    y: from.1 * s,
                    clicks: 1,
                }),
                select(SelectOp::Extend {
                    x: to.0 * s,
                    y: to.1 * s,
                }),
                select(SelectOp::Copy),
            ]);
            let rects = replies.iter().rev().find_map(|r| match r {
                Reply::Selection { rects, .. } => Some(rects.clone()),
                _ => None,
            });
            let text = replies.iter().find_map(|r| match r {
                Reply::Copied { text, .. } => Some(text.clone()),
                _ => None,
            });
            (rects.unwrap(), text.unwrap())
        };
        let (rects1, text1) = drag(600, 1.0);
        let (rects75, text75) = drag(450, 0.75);
        assert!(
            text1.contains("omicron pi") && text1.len() > 40,
            "{text1:?}"
        );
        assert_eq!(text75, text1);
        // The highlight comes back in view coordinates.
        assert_eq!(rects75.len(), rects1.len());
        for (a, b) in rects75.iter().zip(&rects1) {
            assert!((a.x - b.x * 0.75).abs() < 0.01 && (a.w - b.w * 0.75).abs() < 0.01);
            assert!((a.y - b.y * 0.75).abs() < 0.01 && (a.h - b.h * 0.75).abs() < 0.01);
        }
    }

    #[test]
    fn links_are_hit_under_scale() {
        let (_, l600) = page_and_links(&worker_replies(vec![layout_job(key(1, 600), FIXED)]));
        let (page, l450) = page_and_links(&worker_replies(vec![layout_job(key(1, 450), FIXED)]));
        let (a, b) = (l600[0].rects[0], l450[0].rects[0]);
        assert_eq!(l450[0].href, "https://example.com/x");
        assert!((b.x - a.x * 0.75).abs() < 0.01 && (b.y - a.y * 0.75).abs() < 0.01);
        assert!((b.w - a.w * 0.75).abs() < 0.01 && (b.h - a.h * 0.75).abs() < 0.01);
        assert!(b.x + b.w <= page.width);

        // On the UI side, the pointer over the scaled link finds it.
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform::new(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1200, 800));
        ui.show().unwrap();
        ui.set_has_message(true);
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let reader = Reader::new(
            &ui,
            Box::new(Fake(jobs.clone())),
            Box::new(NoShell::default()),
        );
        let frame = || {
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    let size = window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    r.render(&mut vec![Rgb8Pixel::default(); w * h], w);
                });
            }
        };
        // A frame lays the window out, so the body column reports its width.
        frame();
        reader.open(1, html(FIXED), false);
        let [(generation, _)] = layouts(std::mem::take(&mut *jobs.borrow_mut()))[..] else {
            panic!("expected one layout job");
        };
        reader.on_reply(Reply::Page {
            generation,
            page,
            links: l450.clone(),
        });
        let center = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        let mut s = reader.state();
        assert_eq!(s.link_at(center.0, center.1), Some(0));
        // The same point in page coordinates is off the link at this scale.
        assert_eq!(s.link_at(a.x + a.w - 1.0, a.y + a.h - 1.0), None);
        s.hover_at(center.0, center.1);
        assert_eq!(ui.get_r_hover_href(), "https://example.com/x");
        drop(s);

        // A page wider than the column (the 0.6 floor) scrolls sideways
        // with a horizontal wheel; the vertical wheel still scrolls the
        // message, and a press still reaches the page.
        let wide = Page {
            width: 1500.0,
            height: 3000.0,
            ..page
        };
        reader.on_reply(Reply::Page {
            generation,
            page: wide,
            links: Vec::new(),
        });
        frame();
        let at = slint::LogicalPosition::new(900.0, 400.0);
        let wheel = |delta_x: f32, delta_y: f32| {
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: at,
                delta_x,
                delta_y,
            });
            // Past the wheel animation and the scroll filter.
            std::thread::sleep(Duration::from_millis(900));
            frame();
        };
        let y0 = ui.get_r_content_y();
        wheel(-120.0, 0.0);
        assert!(ui.get_r_page_x() < 0.0, "{}", ui.get_r_page_x());
        assert_eq!(ui.get_r_content_y(), y0);
        wheel(0.0, -120.0);
        assert!(ui.get_r_content_y() < y0);
        // A relayout that fits the column again leaves no sideways offset.
        reader.on_reply(Reply::Page {
            generation,
            page,
            links: Vec::new(),
        });
        frame();
        assert_eq!(ui.get_r_page_x(), 0.0);
        reader.on_reply(Reply::Page {
            generation,
            page: wide,
            links: Vec::new(),
        });
        frame();
        let left = slint::platform::PointerEventButton::Left;
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: at,
            button: left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position: at,
            button: left,
        });
        assert!(jobs.borrow().iter().any(|j| matches!(
            j,
            Job::Select {
                op: SelectOp::Press { .. },
                ..
            }
        )));
        // A new message starts at the left edge again.
        reader.open(2, html(FIXED), false);
        assert_eq!(ui.get_r_page_x(), 0.0);
    }

    // ------------------------------------------------------ remote images

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, 2, 2);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[200; 16])
            .unwrap();
        out
    }

    #[test]
    fn worker_fetches_consented_images_once_and_lays_out_again() {
        let src = html(
            r#"<p>hi</p><img src="https://img.test/a.png" width="20" height="20">
               <img src="https://img.test/pixel.gif" width="1" height="1">
               <div style="background-image:url(https://img.test/bg.png)">x</div>
               <link rel="stylesheet" href="https://img.test/s.css">"#,
        );
        let allowed = Key {
            remote: true,
            ..key(1, 480)
        };
        let (tx, rx) = mpsc::channel();
        for (generation, key) in [(1, key(1, 480)), (2, allowed), (3, allowed), (4, allowed)] {
            let src = src.clone();
            tx.send(Job::Layout {
                generation,
                key,
                src,
            })
            .unwrap();
            tx.send(Job::Tiles {
                generation,
                key,
                tiles: vec![0],
            })
            .unwrap();
        }
        drop(tx);
        let images = Mutex::default();
        let started = RefCell::new(Vec::new());
        // The fake fetch completes at once, before the next layout job.
        let start = |msg: i64, urls: Vec<String>| {
            let mut im = lock(&images);
            for u in &urls {
                im.store(u, Some(png()));
            }
            started.borrow_mut().push((msg, urls));
        };
        let replies = RefCell::new(Vec::new());
        run(
            rx,
            &AtomicU64::new(0),
            &|r| replies.borrow_mut().push(r),
            &images,
            &start,
            (DOC_CACHE, TILE_CACHE_BYTES),
        );
        // Only with consent, only once, and never the tracking pixel or the
        // stylesheet.
        assert_eq!(
            started.into_inner(),
            [(
                1,
                vec![
                    "https://img.test/a.png".to_string(),
                    "https://img.test/bg.png".to_string()
                ]
            )]
        );
        let pages: Vec<(u64, u32)> = replies
            .borrow()
            .iter()
            .filter_map(|r| match r {
                Reply::Page {
                    generation, page, ..
                } => Some((*generation, page.remote_images)),
                _ => None,
            })
            .collect();
        // Blocked, then fetching, then shown (laid out again), then cached.
        assert_eq!(pages, [(1, 2), (2, 2), (3, 0), (4, 0)]);
        // The tiles of the stale layout were dropped, not served again.
        let tiles: Vec<Vec<u8>> = replies
            .borrow()
            .iter()
            .filter_map(|r| match r {
                Reply::Tile { pixels, .. } => Some(pixels.as_bytes().to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(tiles.len(), 4);
        assert!(tiles[0] == tiles[1] && tiles[1] != tiles[2] && tiles[2] == tiles[3]);
        assert_eq!(lock(&images).msg, 1);
    }

    #[test]
    fn fetched_images_are_capped_in_bytes() {
        let mut im = Images::default();
        im.store("a", Some(vec![0; MAX_FETCHED_BYTES - 1]));
        im.store("b", Some(vec![0; 2]));
        im.store("c", None);
        assert!(im.loaded("a") && !im.loaded("b") && !im.loaded("c"));
        assert!(matches!(im.urls.get("b"), Some(Fetched::Failed)));
    }

    #[test]
    fn fetch_pool_runs_four_at_a_time_for_the_current_message() {
        let images: Arc<Mutex<Images>> = Arc::default();
        lock(&images).msg = 5;
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let (a, p, c) = (active.clone(), peak.clone(), calls.clone());
        let fetch: Fetch = Arc::new(move |url: &str| {
            c.fetch_add(1, Ordering::SeqCst);
            let now = a.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(30));
            a.fetch_sub(1, Ordering::SeqCst);
            (!url.ends_with("bad")).then(|| url.as_bytes().to_vec())
        });
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let notify: Notify = Arc::new(move |r| {
            if let Reply::Fetched { msg } = r {
                tx.lock().unwrap().send(msg).unwrap();
            }
        });
        let mut urls: Vec<String> = (0..9).map(|i| format!("https://img.test/{i}")).collect();
        urls.push("https://img.test/bad".into());
        fetch_images(5, urls, &images, &fetch, &notify);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(5));
        assert_eq!(calls.load(Ordering::SeqCst), 10);
        assert!(peak.load(Ordering::SeqCst) <= FETCHERS);
        let im = lock(&images);
        assert!((0..9).all(|i| im.loaded(&format!("https://img.test/{i}"))));
        assert!(matches!(
            im.urls.get("https://img.test/bad"),
            Some(Fetched::Failed)
        ));
        drop(im);

        // Another message took the cache: nothing more is fetched or kept.
        lock(&images).msg = 6;
        fetch_images(
            5,
            vec!["https://img.test/x".into()],
            &images,
            &fetch,
            &notify,
        );
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(5));
        assert_eq!(calls.load(Ordering::SeqCst), 10);
        assert!(!lock(&images).urls.contains_key("https://img.test/x"));
    }

    #[test]
    fn remote_banner_loads_once_and_lays_out_again() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform::new(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1200, 800));
        ui.show().unwrap();
        ui.set_has_message(true);
        let frame = || {
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    let size = window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    let mut buf = vec![Rgb8Pixel::default(); w * h];
                    r.render(&mut buf, w);
                });
            }
        };
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let take = || layouts(std::mem::take(&mut *jobs.borrow_mut()));
        let reader = Reader::new(
            &ui,
            Box::new(Fake(jobs.clone())),
            Box::new(NoShell::default()),
        );
        frame();
        let page = |key: Key, remote_images| Page {
            width: key.width as f32,
            height: 600.0,
            bg: [255; 4],
            truncated: false,
            remote_images,
            pending_images: false,
            failed_images: 0,
            dimmed: false,
        };

        reader.open(7, html("<p>x</p>"), false);
        let [(generation, key)] = take()[..] else {
            panic!("expected one layout job");
        };
        assert!(!key.remote);
        reader.on_reply(Reply::Page {
            generation,
            page: page(key, 3),
            links: vec![],
        });
        assert_eq!(ui.get_r_remote_blocked(), 3);
        frame();

        // Load once: laid out again with consent, the page kept meanwhile.
        reader.allow_remote();
        assert_eq!(ui.get_r_remote_blocked(), 0);
        let [(generation, key)] = take()[..] else {
            panic!("expected a relayout");
        };
        assert!(key.remote);
        assert!(ui.get_r_html());
        reader.on_reply(Reply::Page {
            generation,
            page: page(key, 3),
            links: vec![],
        });
        assert_eq!(ui.get_r_remote_blocked(), 0, "no banner while fetching");
        // Fetches for another message change nothing; ours lay out again.
        reader.on_reply(Reply::Fetched { msg: 8 });
        assert!(take().is_empty());
        reader.on_reply(Reply::Fetched { msg: 7 });
        let [(_, again)] = take()[..] else {
            panic!("expected a relayout");
        };
        assert_eq!(again, key);

        // Consent lasts while the message stays open, and no further.
        reader.open(7, html("<p>x</p>"), false);
        assert!(take().iter().all(|(_, k)| k.remote));
        reader.open(8, html("<p>y</p>"), false);
        assert!(take().iter().all(|(_, k)| !k.remote));
        reader.on_reply(Reply::Fetched { msg: 8 });
        assert!(take().is_empty());
        // An allowlisted sender's message opens with its images.
        reader.open(9, html("<p>z</p>"), true);
        assert!(take().iter().all(|(_, k)| k.remote));
        reader.close();
        reader.allow_remote();
        assert!(take().is_empty());
    }

    #[test]
    fn worker_counts_images_that_could_not_be_loaded() {
        let src = html(
            r#"<p>hi</p><img src="https://img.test/a.png" width="20" height="20">
               <img src="https://img.test/gone.png" width="20" height="20">"#,
        );
        let allowed = Key {
            remote: true,
            ..key(1, 480)
        };
        let (tx, rx) = mpsc::channel();
        for generation in [1, 2] {
            let src = src.clone();
            tx.send(Job::Layout {
                generation,
                key: allowed,
                src,
            })
            .unwrap();
        }
        // Another message without consent: nothing is counted.
        tx.send(Job::Layout {
            generation: 3,
            key: key(2, 480),
            src,
        })
        .unwrap();
        drop(tx);
        let images = Mutex::default();
        // One image loads, the other fails (404, refused, too large...).
        let start = |_: i64, urls: Vec<String>| {
            let mut im = lock(&images);
            for u in &urls {
                im.store(u, u.ends_with("a.png").then(png));
            }
        };
        let replies = RefCell::new(Vec::new());
        run(
            rx,
            &AtomicU64::new(0),
            &|r| replies.borrow_mut().push(r),
            &images,
            &start,
            (DOC_CACHE, TILE_CACHE_BYTES),
        );
        let pages: Vec<(u32, bool, u32)> = replies
            .borrow()
            .iter()
            .filter_map(|r| match r {
                Reply::Page { page, .. } => {
                    Some((page.remote_images, page.pending_images, page.failed_images))
                }
                _ => None,
            })
            .collect();
        // Fetching (nothing failed yet), then one image in and one failed.
        assert_eq!(pages, [(2, true, 0), (1, false, 1), (2, false, 0)]);
        // The other message took the image cache: the fetched bytes are gone.
        assert_eq!(lock(&images).msg, 2);
        assert!(lock(&images).urls.is_empty());
    }

    #[test]
    fn worker_frees_the_previous_message_and_everything_on_cancel() {
        let (tx, rx) = mpsc::channel();
        let jobs = [
            Job::Layout {
                generation: 1,
                key: key(1, 320),
                src: html("<p>one</p>"),
            },
            Job::Layout {
                generation: 2,
                key: key(2, 320),
                src: html("<p>two</p>"),
            },
            // The first message's page was freed on the switch.
            Job::Tiles {
                generation: 2,
                key: key(1, 320),
                tiles: vec![0],
            },
            Job::Tiles {
                generation: 2,
                key: key(2, 320),
                tiles: vec![0],
            },
            // The reader closed: the second one's page and tiles go too.
            Job::Cancel { generation: 3 },
            Job::Tiles {
                generation: 3,
                key: key(2, 320),
                tiles: vec![0],
            },
        ];
        for job in jobs {
            tx.send(job).unwrap();
        }
        drop(tx);
        let replies = RefCell::new(Vec::new());
        run_plain(rx, &AtomicU64::new(0), &|r| replies.borrow_mut().push(r));
        let kinds: Vec<&str> = replies
            .borrow()
            .iter()
            .map(|r| match r {
                Reply::Page { .. } => "page",
                Reply::Tile { .. } => "tile",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["page", "page", "tile"]);
    }

    #[test]
    fn worker_forwards_cancel_only_to_a_running_worker() {
        let replies = Arc::new(Mutex::new(Vec::new()));
        let r = replies.clone();
        let notify: Notify = Arc::new(move |reply| r.lock().unwrap().push(reply));
        let mut worker = Worker::new(notify, Arc::new(|_: &str| None));
        worker.send(Job::Cancel { generation: 1 });
        assert!(worker.tx.is_none(), "a cancel starts no thread");
        assert_eq!(worker.latest.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn failed_images_and_original_colours_reach_the_reader() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform::new(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::PhysicalSize::new(1200, 800));
        ui.show().unwrap();
        ui.set_has_message(true);
        ui.global::<Palette>()
            .set_color_scheme(slint::language::ColorScheme::Dark);
        let frame = || {
            for _ in 0..3 {
                slint::platform::update_timers_and_animations();
                window.request_redraw();
                window.draw_if_needed(|r| {
                    let size = window.size();
                    let (w, h) = (size.width as usize, size.height as usize);
                    let mut buf = vec![Rgb8Pixel::default(); w * h];
                    r.render(&mut buf, w);
                });
            }
        };
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let take = || layouts(std::mem::take(&mut *jobs.borrow_mut()));
        let reader = Reader::new(
            &ui,
            Box::new(Fake(jobs.clone())),
            Box::new(NoShell::default()),
        );
        frame();
        let page = |key: Key, pending_images, failed_images| Page {
            width: key.width as f32,
            height: 600.0,
            bg: [32, 32, 32, 255],
            truncated: false,
            remote_images: failed_images,
            pending_images,
            failed_images,
            dimmed: key.dim,
        };

        // An allowlisted sender: no count while fetches are pending.
        reader.open(7, html("<p>x</p>"), true);
        let [(generation, key)] = take()[..] else {
            panic!("expected one layout job");
        };
        assert!(key.remote && key.dark && key.dim);
        reader.on_reply(Reply::Page {
            generation,
            page: page(key, true, 2),
            links: vec![],
        });
        assert_eq!(ui.get_r_remote_failed(), 0);
        assert_eq!(ui.get_r_remote_blocked(), 0);
        assert!(ui.get_r_dimmed() && !ui.get_r_original());
        // All in: two couldn't be loaded.
        reader.on_reply(Reply::Page {
            generation,
            page: page(key, false, 2),
            links: vec![],
        });
        assert_eq!(ui.get_r_remote_failed(), 2);
        frame();

        // Show original colours: laid out again without dimming, for this
        // message only.
        ui.invoke_r_show_original(true);
        let [(generation, key)] = take()[..] else {
            panic!("expected a relayout");
        };
        assert!(key.dark && !key.dim);
        assert!(ui.get_r_original() && ui.get_r_html(), "the page stays up");
        reader.on_reply(Reply::Page {
            generation,
            page: page(key, false, 0),
            links: vec![],
        });
        assert!(!ui.get_r_dimmed() && ui.get_r_original());
        reader.open(7, html("<p>x</p>"), true);
        assert!(take().iter().all(|(_, k)| !k.dim));
        reader.open(8, html("<p>y</p>"), false);
        assert!(take().iter().all(|(_, k)| k.dim));
        assert!(!ui.get_r_original());
        assert_eq!(ui.get_r_remote_failed(), 0);
        reader.close();
        assert!(!ui.get_r_dimmed());
    }
}
