//! The render worker thread: lays pages out, rasterizes tiles, answers
//! selection gestures and fetches consented remote images, with a cache of
//! laid-out pages and a byte-bounded tile cache.

use super::{
    DOC_CACHE, FETCHERS, Key, MAX_FETCHED_BYTES, MAX_FETCHES, Page, PageTransform, PixelOrder,
    Pixels, Reply, SelectOp, Selection, Source, TILE_CACHE_BYTES, WORKER_STACK, fit_width,
    lines_of, links_of, select, tile_span,
};
use crate::mail::mime;
use crate::mail::render::{
    self, Deadline, Doc, Engine, LineMap, Opts, RemotePolicy, RenderError, RenderInput, Resolver,
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// HTML past this size is shown first from a layout of its top
/// ([`PREVIEW_H`]), then whole: a long reply chain takes 100 ms and more to
/// lay out, its first screen a fraction of that.
const PREVIEW_BYTES: usize = 96 * 1024;
/// How far the first look at a long message is laid out (CSS px): the
/// first screen and the read-ahead below it.
const PREVIEW_H: f32 = 4000.0;
/// How long the worker waits, after a first look, for the UI's request for
/// its tiles before laying out the whole message.
const PREVIEW_WAIT: Duration = Duration::from_millis(50);

/// The first look at a long message, shown while the whole of it waits.
struct Later {
    generation: u64,
    key: Key,
    input: Arc<RenderInput>,
    /// Laid out to [`PREVIEW_H`]; its tiles are drawn from it meanwhile.
    doc: Doc,
    /// The whole layout starts once this passes, or as soon as the tiles of
    /// the first look are drawn.
    until: Instant,
}

/// What the UI side asks of the worker.
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

/// Where the worker's replies go. It runs on the worker (and fetch)
/// threads: hand the reply to the UI thread from there.
pub type Notify = Arc<dyn Fn(Reply) + Send + Sync>;
/// Fetches a consented remote image (`crate::fetch`, or a fake).
pub type Fetch = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Where jobs go: the render worker, or a fake in tests.
pub trait Backend {
    fn send(&mut self, job: Job);
}

/// The handle to the render worker thread, which starts on the first job.
pub struct Worker {
    tx: Option<Sender<Job>>,
    latest: Arc<AtomicU64>,
    notify: Notify,
    fetch: Fetch,
    config: Config,
}

/// How the worker loop runs.
#[derive(Clone, Copy, Debug)]
struct Config {
    /// Laid-out pages and tile bytes kept (the main reader's, or a message
    /// window's smaller ones).
    caches: (usize, usize),
    order: PixelOrder,
    /// Send each page's lines of text ([`Reply::Text`]).
    text: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            caches: (DOC_CACHE, TILE_CACHE_BYTES),
            order: PixelOrder::Rgba,
            text: false,
        }
    }
}

impl Worker {
    pub fn new(notify: Notify, fetch: Fetch) -> Self {
        Worker {
            tx: None,
            latest: Arc::default(),
            notify,
            fetch,
            config: Config::default(),
        }
    }

    /// Keep at most `docs` laid-out pages and `tile_bytes` of tiles. The
    /// thread ends when this handle is dropped (its channel closes).
    pub fn with_caches(mut self, docs: usize, tile_bytes: usize) -> Self {
        self.config.caches = (docs.max(1), tile_bytes);
        self
    }

    /// Hand tiles over in this channel order (BGRA for GPUI).
    pub fn with_order(mut self, order: PixelOrder) -> Self {
        self.config.order = order;
        self
    }

    /// Also send each page's lines of text, for screen readers.
    pub fn with_text_lines(mut self) -> Self {
        self.config.text = true;
        self
    }

    fn spawn(&self) -> Option<Sender<Job>> {
        let (tx, rx) = mpsc::channel();
        let latest = self.latest.clone();
        let notify = self.notify.clone();
        let fetch = self.fetch.clone();
        let config = self.config;
        std::thread::Builder::new()
            .name("render".into())
            .stack_size(WORKER_STACK)
            .spawn(move || {
                let images: Arc<Mutex<Images>> = Arc::default();
                let start = |msg, urls| fetch_images(msg, urls, &images, &fetch, &notify);
                run(rx, &latest, &*notify, &images, &start, config)
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

/// Runs the worker loop over `jobs` on a thread of its own (with the
/// worker's stack), without remote images, and returns every reply in
/// order: for tests and tools that need real pages without a UI.
pub fn run_to_end(jobs: Vec<Job>, order: PixelOrder) -> Vec<Reply> {
    let config = Config {
        order,
        ..Config::default()
    };
    let run_all = move || {
        let (tx, rx) = mpsc::channel();
        for job in jobs {
            if tx.send(job).is_err() {
                break;
            }
        }
        drop(tx);
        let replies = std::cell::RefCell::new(Vec::new());
        run(
            rx,
            &AtomicU64::new(0),
            &|r| replies.borrow_mut().push(r),
            &Mutex::default(),
            &|_, _| {},
            config,
        );
        replies.into_inner()
    };
    std::thread::Builder::new()
        .name("render".into())
        .stack_size(WORKER_STACK)
        .spawn(run_all)
        .ok()
        .and_then(|t| t.join().ok())
        .unwrap_or_default()
}

/// The worker loop: runs until the UI side hangs up. Consented remote
/// images come from `images`; `start` fetches more for a message.
fn run(
    rx: Receiver<Job>,
    latest: &AtomicU64,
    notify: &dyn Fn(Reply),
    images: &Mutex<Images>,
    start: &dyn Fn(i64, Vec<String>),
    config: Config,
) {
    let (doc_cache, tile_bytes) = config.caches;
    let order = config.order;
    // The shown page's line map (with its lines), so each selection change
    // maps onto the lines without walking the layout again.
    let mut line_map: Option<(Key, LineMap)> = None;
    let lines = |doc: &Doc, key: Key, line_map: &mut Option<(Key, LineMap)>| {
        if config.text {
            let (lines, map) = lines_of(doc, key.transform(doc));
            *line_map = Some((key, map));
            lines
        } else {
            Vec::new()
        }
    };
    let mut engine = Engine::new();
    let mut docs: Lru<Key, Doc> = Lru::new(doc_cache);
    let mut tiles: Lru<(Key, u32), Pixels> = Lru::new(tile_bytes);
    let mut sel: Option<Selection> = None;
    let mut later: Option<Later> = None;
    loop {
        let job = match later.take() {
            None => match rx.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
            Some(l) => match rx.recv_timeout(l.until.saturating_duration_since(Instant::now())) {
                Ok(job) => {
                    later = Some(l);
                    job
                }
                // Nothing more asked for now (or the UI hung up): the whole
                // message, in place of its first look.
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    if !current(l.generation, latest) {
                        continue;
                    }
                    let mut cache = lock(images);
                    if cache.msg != l.key.msg {
                        continue;
                    }
                    let key = l.key;
                    let mut wanted = Vec::new();
                    let doc = lay_out(&l.input, key, &mut engine, &cache, Some(&l.doc), None)
                        // Out of time: the first look stays, cut.
                        .unwrap_or(l.doc);
                    if key.remote {
                        let room = MAX_FETCHES.saturating_sub(cache.urls.len());
                        for u in &doc.remote_images {
                            if wanted.len() < room && !cache.urls.contains_key(u) {
                                cache.urls.insert(u.clone(), Fetched::Pending);
                                wanted.push(u.clone());
                            }
                        }
                    }
                    let page = page_of(&doc, key, &cache);
                    let links = links_of(&doc, key.transform(&doc));
                    let text = lines(&doc, key, &mut line_map);
                    docs.insert(key, doc, 1);
                    tiles.retain(|(k, _)| *k != key);
                    drop(cache);
                    if !wanted.is_empty() {
                        start(key.msg, wanted);
                    }
                    if current(l.generation, latest) {
                        let generation = l.generation;
                        notify(Reply::Page {
                            generation,
                            page,
                            links,
                        });
                        if config.text {
                            notify(Reply::Text {
                                generation,
                                lines: text,
                            });
                        }
                    }
                    continue;
                }
            },
        };
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
                    Some(doc) => Ok((
                        page_of(doc, key, &cache),
                        links_of(doc, key.transform(doc)),
                        lines(doc, key, &mut line_map),
                    )),
                    None => {
                        // A long message: its top first, the rest once its
                        // tiles are drawn.
                        let long = input.html.as_ref().is_some_and(|h| h.len() > PREVIEW_BYTES);
                        let stop = long.then_some(PREVIEW_H);
                        lay_out(&input, key, &mut engine, &cache, docs.newest(), stop).map(|doc| {
                            if key.remote {
                                let room = MAX_FETCHES.saturating_sub(cache.urls.len());
                                for u in &doc.remote_images {
                                    if wanted.len() < room && !cache.urls.contains_key(u) {
                                        cache.urls.insert(u.clone(), Fetched::Pending);
                                        wanted.push(u.clone());
                                    }
                                }
                            }
                            let mut page = (
                                page_of(&doc, key, &cache),
                                links_of(&doc, key.transform(&doc)),
                                lines(&doc, key, &mut line_map),
                            );
                            tiles.retain(|(k, _)| *k != key);
                            if long && doc.full_height > PREVIEW_H {
                                page.0.truncated = false;
                                page.0.partial = true;
                                later = Some(Later {
                                    generation,
                                    key,
                                    input: input.clone(),
                                    doc,
                                    until: Instant::now() + PREVIEW_WAIT,
                                });
                            } else {
                                docs.insert(key, doc, 1);
                            }
                            page
                        })
                    }
                };
                drop(cache);
                if !wanted.is_empty() {
                    start(key.msg, wanted);
                }
                if current(generation, latest) {
                    match page {
                        Ok((page, links, lines)) => {
                            notify(Reply::Page {
                                generation,
                                page,
                                links,
                            });
                            if config.text {
                                notify(Reply::Text { generation, lines });
                            }
                        }
                        Err(error) => notify(Reply::Failed { generation, error }),
                    }
                }
            }
            Job::Tiles {
                generation,
                key,
                tiles: wanted,
            } => {
                let first_look = later.as_ref().filter(|l| l.key == key);
                for index in wanted {
                    if !current(generation, latest) {
                        break;
                    }
                    let pixels = match tiles.get(&(key, index)) {
                        Some(p) => Some(p.clone()),
                        None => docs
                            .get(&key)
                            .or(first_look.map(|l| &l.doc))
                            .and_then(|doc| rasterize(doc, key, &mut engine, index, order))
                            .inspect(|p| tiles.insert((key, index), p.clone(), p.bytes())),
                    };
                    if let Some(pixels) = pixels {
                        notify(Reply::Tile {
                            generation,
                            index,
                            pixels,
                        });
                    }
                }
                // The first look is drawn: on to the whole message.
                if let Some(l) = later.as_mut().filter(|l| l.key == key) {
                    l.until = Instant::now();
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
                    let lines = config.text && op != SelectOp::Copy;
                    notify(reply);
                    if lines {
                        let range = range.and_then(|(doc, (a, b))| {
                            if !matches!(&line_map, Some((k, _)) if *k == key) {
                                line_map = Some((key, lines_of(doc, key.transform(doc)).1));
                            }
                            line_map.as_ref()?.1.positions(doc, a, b)
                        });
                        notify(Reply::TextSelection { generation, range });
                    }
                }
            }
            // Nothing is shown: free everything kept for the page.
            Job::Cancel { .. } => {
                later = None;
                sel = None;
                line_map = None;
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
/// `earlier` is another layout of the same message (at another width or in
/// other colours, still cached), whose shaped text this one may reuse.
fn lay_out(
    input: &RenderInput,
    key: Key,
    engine: &mut Engine,
    images: &Images,
    earlier: Option<&Doc>,
    stop_at: Option<f32>,
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
        stop_at,
        ..Default::default()
    };
    let budget = || Deadline::after(render::layout_budget(input));
    let mut reuse = render::Reuse::new(input);
    let doc = render::layout_reusing(
        &mut reuse,
        &opts,
        engine,
        &budget(),
        &mut { images },
        earlier,
    )?;
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
    // The same message again: it keeps the parsed document and the text
    // the first pass shaped.
    match render::layout_reusing(
        &mut reuse,
        &opts,
        engine,
        &budget(),
        &mut { images },
        Some(&doc),
    ) {
        // Content that overflows further on a wider page is sized relative
        // to the page, not fixed: scaling would never make it fit.
        Ok(wide) if width > column && wide.natural_width > doc.natural_width + super::FIT_SLACK => {
            if !opts.dark {
                return Ok(doc);
            }
            opts.width_px = column;
            opts.scale = key.scale();
            let dark = render::layout_reusing(
                &mut reuse,
                &opts,
                engine,
                &budget(),
                &mut { images },
                Some(&doc),
            );
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
        partial: false,
        remote_images: u32::try_from(doc.remote_images.len()).unwrap_or(u32::MAX),
        pending_images: pending,
        failed_images: failed,
        dimmed: doc.dimmed,
    }
}

/// Tile `index` of `doc` (laid out under `key`) at device pixels, in
/// `order`. Tiles span `TILE_H` view px; a scaled page is rasterized at the
/// device scale times its own, so the tile is drawn 1:1 and stays sharp.
fn rasterize(
    doc: &Doc,
    key: Key,
    engine: &mut Engine,
    index: u32,
    order: PixelOrder,
) -> Option<Pixels> {
    let tf = key.transform(doc);
    let scale = key.scale();
    let (top, bottom) = tile_span(index, tf.y_to_view(doc.height))?;
    let y0 = (top * scale).round() as u32;
    let y1 = ((bottom * scale).round() as u32).max(y0 + 1);
    let pm = render::rasterize(doc, engine, y0, y1 - y0, scale * tf.scale)?;
    let (w, h) = (pm.width(), pm.height());
    let mut data = pm.take();
    if order == PixelOrder::Bgra {
        for px in data.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
    Pixels::new(w, h, data.into())
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

    /// The entry used last, without counting as a use.
    fn newest(&self) -> Option<&V> {
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

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
