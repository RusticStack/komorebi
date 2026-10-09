//! Native HTML mail rendering: our own small layout engine for the HTML
//! that mail actually uses (tables, inline styles, `<style>` with media
//! queries, legacy attributes), drawn with tiny-skia. No scripts, no
//! network: remote images, CSS and fonts are blocked and only counted
//! unless the caller opts in per message (images only), and tracking pixels
//! are never loaded.
//!
//! Untrusted input must never take the app down (release builds abort on
//! panic), hang or exhaust memory. So this module denies the lints below,
//! every size cap is enforced before allocating, block nesting past
//! [`layout::MAX_LAYOUT_DEPTH`] is flattened instead of recursed into, and
//! a cooperative [`Deadline`] bounds parse, cascade and layout time.
//!
//! The engine has no UI types: callers get a [`Doc`] and rasterize bands
//! of it into pixmaps.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

mod css;
pub(crate) mod dom;
mod float;
mod img;
mod layout;
pub mod links;
mod paint;
mod select;
mod style;
mod table;

pub use layout::{Brush, Doc, Item, Link, MAX_DOC_H, MAX_GROUP_TEXT, Rect, TextLine};
pub use paint::{GlyphCache, MAX_CACHE_BYTES as MAX_GLYPH_CACHE_BYTES};
pub use select::TextPos;

use parley::{FontContext, LayoutContext};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Decoded HTML accepted; larger bodies fall back to the text view.
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
/// Remote images listed per message in [`Doc::remote_images`].
pub const MAX_REMOTE_IMAGES: usize = 1000;
/// The layout budget per message.
pub const LAYOUT_BUDGET: Duration = Duration::from_millis(250);

/// What a message offers for rendering.
#[derive(Debug, Clone, Default)]
pub struct RenderInput {
    /// The decoded `text/html` body, if any.
    pub html: Option<String>,
    /// Parts referenced by `cid:` URLs.
    pub inline: Vec<InlinePart>,
    /// The plain-text rendering, for the fallback view.
    pub text: String,
    /// Show `text` instead of rendering: there is no HTML body, or the
    /// sender ranked a text/plain alternative above it.
    pub prefers_plain: bool,
}

#[derive(Debug, Clone)]
pub struct InlinePart {
    /// The Content-ID without angle brackets.
    pub cid: String,
    pub mime: String,
    pub bytes: Arc<[u8]>,
}

/// Whether remote images may be loaded for this message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RemotePolicy {
    #[default]
    Block,
    /// The user consented for this message or sender. Remote CSS and fonts
    /// stay blocked, and tracking pixels are never requested.
    Allow,
}

#[derive(Debug, Clone, Copy)]
pub struct Opts {
    /// Layout width in CSS px.
    pub width_px: f32,
    /// Device pixels per CSS px; images are prescaled for it.
    pub scale: f32,
    pub remote: RemotePolicy,
    /// Evaluate `prefers-color-scheme: dark` rules.
    pub dark: bool,
    /// Dim the colours for a dark theme, unless the message has dark rules
    /// of its own (see [`Doc::dimmed`]).
    pub dim: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            width_px: 600.0,
            scale: 1.0,
            remote: RemotePolicy::Block,
            dark: false,
            dim: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderError {
    /// The HTML is over [`MAX_HTML_BYTES`].
    TooLarge,
    /// The [`Deadline`] passed.
    Deadline,
    /// Nothing renderable came out of the HTML.
    Malformed,
    /// The message has no HTML body.
    NoHtml,
}

/// A cooperative time limit, checked per node and per table row.
#[derive(Debug, Clone, Copy)]
pub struct Deadline(Option<Instant>);

impl Deadline {
    pub fn after(budget: Duration) -> Self {
        Deadline(Instant::now().checked_add(budget))
    }
    /// No limit (development tools).
    pub fn none() -> Self {
        Deadline(None)
    }
    pub fn expired(&self) -> bool {
        self.0.is_some_and(|end| Instant::now() >= end)
    }
}

/// Supplies remote images the user consented to load. Consulted only under
/// [`RemotePolicy::Allow`], and never for tracking pixels.
pub trait Resolver {
    fn remote(&mut self, url: &str) -> Option<Arc<[u8]>>;
}

/// The resolver that loads nothing.
pub struct NoRemote;

impl Resolver for NoRemote {
    fn remote(&mut self, _url: &str) -> Option<Arc<[u8]>> {
        None
    }
}

/// Fonts, shaping state and the glyph cache, reused across messages.
#[derive(Default)]
pub struct Engine {
    fcx: FontContext,
    lcx: LayoutContext<Brush>,
    glyphs: GlyphCache,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the glyph cache holds now (estimated bytes).
    pub fn glyph_cache_bytes(&self) -> usize {
        self.glyphs.bytes()
    }
}

/// Image sources for one message: its own `cid:` parts and `data:` URLs,
/// and remote images only when the policy allows.
pub(crate) struct Sources<'a> {
    inline: &'a [InlinePart],
    remote: RemotePolicy,
    resolver: &'a mut dyn Resolver,
    /// Remote images the layout asked for and did not get.
    missing: Vec<String>,
}

impl Sources<'_> {
    fn load(&mut self, src: &str) -> Option<Arc<[u8]>> {
        let s = src.trim();
        let scheme = |p: &str| s.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p));
        let bytes = if scheme("cid:") {
            let id = s.get(4..)?.trim_matches(['<', '>']);
            self.inline
                .iter()
                .find(|p| p.cid.eq_ignore_ascii_case(id))
                .map(|p| p.bytes.clone())
        } else if scheme("data:") {
            data_url(s.get(5..)?).map(Arc::from)
        } else if css::is_remote(s) {
            let got = match self.remote {
                RemotePolicy::Allow => self.resolver.remote(s),
                RemotePolicy::Block => None,
            };
            if got.is_none()
                && self.missing.len() < MAX_REMOTE_IMAGES
                && !self.missing.iter().any(|m| m == s)
            {
                self.missing.push(s.to_string());
            }
            got
        } else {
            None
        };
        bytes.filter(|b| b.len() <= img::MAX_ENCODED)
    }
}

/// The bytes of a base64 `data:` image URL (png, jpeg, gif or webp).
fn data_url(rest: &str) -> Option<Vec<u8>> {
    if rest.len() > img::MAX_ENCODED.saturating_mul(4) / 3 {
        return None;
    }
    let (meta, data) = rest.split_once(',')?;
    let meta = meta.to_ascii_lowercase();
    let image = [
        "image/png",
        "image/jpeg",
        "image/jpg",
        "image/gif",
        "image/webp",
    ]
    .iter()
    .any(|t| meta.starts_with(t));
    if !image || !meta.ends_with(";base64") {
        return None;
    }
    let clean: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    mail_parser::decoders::base64::base64_decode(&clean)
}

/// Lay out a message. Remote images stay blocked whatever `opts.remote`
/// says, since this resolver fetches nothing; see [`layout_with`].
pub fn layout(
    input: &RenderInput,
    opts: &Opts,
    cx: &mut Engine,
    deadline: &Deadline,
) -> Result<Doc, RenderError> {
    layout_with(input, opts, cx, deadline, &mut NoRemote)
}

/// Lay out a message, asking `resolver` for remote images when
/// `opts.remote` allows them.
pub fn layout_with(
    input: &RenderInput,
    opts: &Opts,
    cx: &mut Engine,
    deadline: &Deadline,
    resolver: &mut dyn Resolver,
) -> Result<Doc, RenderError> {
    let html = input
        .html
        .as_deref()
        .filter(|h| !h.trim().is_empty())
        .ok_or(RenderError::NoHtml)?;
    if html.len() > MAX_HTML_BYTES {
        return Err(RenderError::TooLarge);
    }
    let width = if opts.width_px.is_finite() {
        opts.width_px.clamp(1.0, 4096.0)
    } else {
        600.0
    };
    let dom = dom::parse(html);
    if deadline.expired() {
        return Err(RenderError::Deadline);
    }
    let ctx = css::Ctx {
        width,
        dark: opts.dark,
        dim: opts.dim,
    };
    let styles = style::compute(&dom, &ctx, deadline).ok_or(RenderError::Deadline)?;
    let mut sources = Sources {
        inline: &input.inline,
        remote: opts.remote,
        resolver,
        missing: Vec::new(),
    };
    let scale = clamp_scale(opts.scale);
    let doc = layout::layout(
        &dom,
        styles,
        &mut cx.fcx,
        &mut cx.lcx,
        &mut sources,
        width,
        scale,
        deadline,
    )
    .ok_or(RenderError::Deadline)?;
    if doc.items.is_empty() {
        return Err(RenderError::Malformed);
    }
    Ok(doc)
}

fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.5, 4.0)
    } else {
        1.0
    }
}

/// Rasterize the band of `doc` starting at device row `y_px`, `h_px` rows
/// tall, at `scale` device pixels per CSS px. `None` for an empty or
/// oversized band (over 16 Mpx).
pub fn rasterize(
    doc: &Doc,
    cx: &mut Engine,
    y_px: u32,
    h_px: u32,
    scale: f32,
) -> Option<tiny_skia::Pixmap> {
    let scale = clamp_scale(scale);
    let w = (doc.width * scale).round().clamp(1.0, 16_384.0) as u32;
    let h = h_px.min(4096);
    // At most 16 Mpx (64 MB) per band.
    if u64::from(w).saturating_mul(u64::from(h)) > 16_000_000 {
        return None;
    }
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    paint::render(doc, &mut pm, y_px as f32, scale, &mut cx.glyphs);
    Some(pm)
}
