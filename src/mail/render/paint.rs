//! Rasterize a band of the display list into a tiny-skia pixmap at a
//! device scale. Glyph outlines come from skrifa and are cached as alpha
//! masks keyed by (font, glyph, size, subpixel x / 4), blitted with our
//! own src-over loop. Colour glyphs (emoji in Segoe UI Emoji) are painted
//! from their COLR layers in CPAL colours (COLRv0; a COLRv1 gradient
//! falls back to its first stop) into small cached pixmaps. The cache is
//! least-recently-used within [`MAX_CACHE_BYTES`].

use super::layout::{Brush, Doc, Item, Rect};
use parley::PositionedLayoutItem;
use skrifa::{
    GlyphId, MetadataProvider,
    color::{self, ColorGlyphFormat, ColorPainter, ColorPalettes, CompositeMode},
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
    raw::{FontRef, TableProvider, types::BoundingBox},
};
use std::collections::HashMap;
use tiny_skia::{
    FillRule, FilterQuality, Mask, Paint, Path, PathBuilder, Pattern, Pixmap, SpreadMode, Stroke,
    Transform,
};

struct GlyphMask {
    w: u32,
    h: u32,
    left: i32,
    top: i32,
    data: Vec<u8>,
}

/// A colour glyph drawn at one size: premultiplied pixels placed at
/// (left, top) from the pen position.
struct ColorMask {
    left: i32,
    top: i32,
    pix: Pixmap,
}

type PathKey = (u64, u32, u32);
type MaskKey = (u64, u32, u32, u8);
/// A mask key plus the text colour (palette entry 0xFFFF draws in it).
type ColorKey = (MaskKey, [u8; 4]);

/// A cached value with its estimated size and when it was last used.
struct Entry<T> {
    v: T,
    cost: usize,
    used: u64,
}

/// Glyph outlines, coverage masks and colour glyphs, kept across
/// rasterizations.
#[derive(Default)]
pub struct GlyphCache {
    paths: HashMap<PathKey, Entry<Option<Path>>>,
    masks: HashMap<MaskKey, Entry<Option<GlyphMask>>>,
    colors: HashMap<ColorKey, Entry<Option<ColorMask>>>,
    bytes: usize,
    /// Advances once per rasterization: the "last used" clock.
    clock: u64,
}

/// The glyph cache's budget: past it, the least recently used entries go
/// (down to three quarters of it) before the next band is painted.
pub const MAX_CACHE_BYTES: usize = 4 * 1024 * 1024;
/// Layers painted per colour glyph.
const MAX_LAYERS: usize = 256;
/// The largest colour glyph bitmap side, in device px (bigger ones are
/// drawn as plain outlines).
const MAX_COLOR_PX: u32 = 512;

impl GlyphCache {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Evict the least recently used entries until the cache is under
    /// three quarters of [`MAX_CACHE_BYTES`].
    fn trim(&mut self) {
        if self.bytes <= MAX_CACHE_BYTES {
            return;
        }
        let mut stamps: Vec<(u64, usize)> = self
            .paths
            .values()
            .map(|e| (e.used, e.cost))
            .chain(self.masks.values().map(|e| (e.used, e.cost)))
            .chain(self.colors.values().map(|e| (e.used, e.cost)))
            .collect();
        stamps.sort_unstable();
        let target = MAX_CACHE_BYTES / 4 * 3;
        let mut left = self.bytes;
        // Entries used at or before `cut` go.
        let mut cut = None;
        for (used, cost) in stamps {
            if left <= target && cut != Some(used) {
                break;
            }
            left = left.saturating_sub(cost);
            cut = Some(used);
        }
        let Some(cut) = cut else {
            return;
        };
        self.paths.retain(|_, e| e.used > cut);
        self.masks.retain(|_, e| e.used > cut);
        self.colors.retain(|_, e| e.used > cut);
        self.bytes = self
            .paths
            .values()
            .map(|e| e.cost)
            .chain(self.masks.values().map(|e| e.cost))
            .chain(self.colors.values().map(|e| e.cost))
            .fold(0, usize::saturating_add);
    }
}

/// The estimated heap size of an outline.
fn path_cost(p: Option<&Path>) -> usize {
    p.map_or(16, |p| {
        p.points()
            .len()
            .saturating_mul(8)
            .saturating_add(p.verbs().len())
            .saturating_add(64)
    })
}

struct Pen(PathBuilder);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y)
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, -cy0, x, -y)
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, -cy0, cx1, -cy1, x, -y)
    }
    fn close(&mut self) {
        self.0.close()
    }
}

fn paint_of(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn rrect(r: Rect, rad: f32) -> Option<Path> {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0);
    let mut pb = PathBuilder::new();
    if rad.is_nan() || rad <= 0.5 {
        pb.push_rect(tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h)?);
        return pb.finish();
    }
    let k = 0.5523 * rad;
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}

/// Paint the document band starting at device row `y_px` into `pm`, at
/// `scale` device pixels per CSS px.
pub fn render(doc: &Doc, pm: &mut Pixmap, y_px: f32, scale: f32, cache: &mut GlyphCache) {
    cache.trim();
    cache.clock = cache.clock.wrapping_add(1);
    let (vw, vh) = (pm.width() as f32, pm.height() as f32);
    pm.fill(tiny_skia::Color::from_rgba8(
        doc.bg[0], doc.bg[1], doc.bg[2], 255,
    ));
    // Device rectangle of a CSS rectangle.
    let dev = |r: &Rect| Rect {
        x: r.x * scale,
        y: r.y * scale - y_px,
        w: r.w * scale,
        h: r.h * scale,
    };
    let visible = |d: &Rect| d.y <= vh && d.y + d.h >= 0.0 && d.x <= vw && d.x + d.w >= 0.0;
    for it in &doc.items {
        match it {
            Item::Rect { r, color, radius } => {
                let d = dev(r);
                if !visible(&d) || color[3] == 0 {
                    continue;
                }
                if *radius > 0.5 {
                    if let Some(p) = rrect(d, radius * scale) {
                        pm.fill_path(
                            &p,
                            &paint_of(*color),
                            FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                } else if let Some(rc) = clip_rect(d, vw, vh) {
                    let mut p = paint_of(*color);
                    p.anti_alias = false;
                    pm.fill_rect(rc, &p, Transform::identity(), None);
                }
            }
            Item::Stroke {
                r,
                width,
                color,
                radius,
            } => {
                let d = dev(r);
                if !visible(&d) || color[3] == 0 {
                    continue;
                }
                let w = width * scale;
                let inset = Rect {
                    x: d.x + w / 2.0,
                    y: d.y + w / 2.0,
                    w: d.w - w,
                    h: d.h - w,
                };
                if let Some(p) = rrect(inset, (radius * scale - w / 2.0).max(0.0)) {
                    let st = Stroke {
                        width: w,
                        ..Default::default()
                    };
                    pm.stroke_path(&p, &paint_of(*color), &st, Transform::identity(), None);
                }
            }
            Item::Image {
                r,
                img,
                clip,
                radius,
            } => {
                let d = dev(r);
                let Some(im) = doc.images.get(*img) else {
                    continue;
                };
                if !visible(&d) || d.w < 0.5 || d.h < 0.5 {
                    continue;
                }
                let (iw, ih) = (im.pix.width() as f32, im.pix.height() as f32);
                // Background images cover their box; others stretch to it.
                let (sx, sy) = if clip.is_some() {
                    let s = (d.w / iw).max(d.h / ih).max(0.0001);
                    (s, s)
                } else {
                    (d.w / iw, d.h / ih)
                };
                let area = clip.as_ref().map_or(d, dev);
                let unit = (sx - 1.0).abs() < 0.01 && (sy - 1.0).abs() < 0.01;
                if unit && *radius <= 0.5 {
                    // Pre-scaled at layout: a plain clipped src-over copy.
                    blit_image(pm, &im.pix, d.x.round() as i32, d.y.round() as i32, area);
                    continue;
                }
                let quality = if unit {
                    FilterQuality::Nearest
                } else {
                    FilterQuality::Bilinear
                };
                let t = Transform::from_row(sx, 0.0, 0.0, sy, d.x, d.y);
                let shader = Pattern::new(im.pix.as_ref(), SpreadMode::Pad, quality, 1.0, t);
                let paint = Paint {
                    shader,
                    anti_alias: true,
                    ..Default::default()
                };
                if *radius > 0.5 {
                    if let Some(p) = rrect(area, radius * scale) {
                        pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
                    }
                } else if let Some(rc) = clip_rect(area, vw, vh) {
                    pm.fill_rect(rc, &paint, Transform::identity(), None);
                }
            }
            Item::Link { .. } | Item::Anchor { .. } => {}
            Item::Text {
                layout,
                x,
                y,
                clip_h,
                ..
            } => {
                let Some(l) = doc.layouts.get(*layout) else {
                    continue;
                };
                let (ox, oy) = (x * scale, y * scale - y_px);
                if oy > vh || oy + l.height() * scale < 0.0 || ox > vw {
                    continue;
                }
                draw_text(l, pm, ox, oy, *clip_h, scale, cache);
            }
        }
    }
}

fn clip_rect(d: Rect, vw: f32, vh: f32) -> Option<tiny_skia::Rect> {
    let x0 = d.x.max(-1.0);
    let y0 = d.y.max(-1.0);
    let x1 = (d.x + d.w).min(vw + 1.0);
    let y1 = (d.y + d.h).min(vh + 1.0);
    if x1.is_nan() || y1.is_nan() || x1 <= x0 || y1 <= y0 {
        return None;
    }
    tiny_skia::Rect::from_ltrb(x0, y0, x1, y1)
}

fn draw_text(
    l: &parley::Layout<Brush>,
    pm: &mut Pixmap,
    ox: f32,
    oy: f32,
    clip_h: f32,
    scale: f32,
    cache: &mut GlyphCache,
) {
    let (vw, vh) = (pm.width() as f32, pm.height() as f32);
    for line in l.lines() {
        let lm = line.metrics();
        if lm.block_max_coord > clip_h + 1.0 {
            break;
        }
        if oy + lm.block_max_coord * scale < 0.0 || oy + lm.block_min_coord * scale > vh {
            continue;
        }
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else {
                continue;
            };
            let style = gr.style();
            let col = style.brush.color;
            let run = gr.run();
            let size = run.font_size() * scale;
            if col[3] == 0 || size.is_nan() || size < 0.5 {
                continue;
            }
            let font = run.font();
            let blob_id = font.data.id();
            let coords = run.normalized_coords();
            let coords_hash = coords
                .iter()
                .fold(17u32, |h, c| h.wrapping_mul(31).wrapping_add(*c as u32))
                ^ (font.index << 20);
            let font_ref = FontRef::from_index(font.data.as_ref(), font.index).ok();
            let upem = font_ref
                .as_ref()
                .and_then(|f| f.head().ok())
                .map_or(1000.0, |h| f32::from(h.units_per_em()).max(1.0));
            let unit = size / upem;
            let c: Vec<NormalizedCoord> = coords
                .iter()
                .map(|c| NormalizedCoord::from_bits(*c))
                .collect();
            let font = Font {
                font: font_ref.as_ref(),
                blob: blob_id,
                coords_hash,
                coords: &c,
            };
            // Colour glyphs only come from fonts with a COLR table.
            let colr = font_ref.as_ref().is_some_and(|f| f.colr().is_ok());
            let clock = cache.clock;
            let full = Rect {
                x: 0.0,
                y: 0.0,
                w: vw,
                h: vh,
            };
            for g in gr.positioned_glyphs() {
                let gx = ox + g.x * scale;
                let gy = oy + g.y * scale;
                if gx > vw + size || gx < -size * 2.0 {
                    continue;
                }
                let px = gx.floor();
                let sub = ((gx - px) * 4.0).round().clamp(0.0, 4.0) as u8;
                let (px, sub) = if sub == 4 { (px + 1.0, 0) } else { (px, sub) };
                let py = gy.round();
                let key = (
                    blob_id ^ (u64::from(coords_hash) << 32),
                    g.id,
                    size.to_bits(),
                    sub,
                );
                if colr && let Some(m) = color_glyph(cache, &font, g.id, (key, col), unit, sub) {
                    let (x, y) = (px as i32, py as i32);
                    blit_image(
                        pm,
                        &m.pix,
                        x.saturating_add(m.left),
                        y.saturating_add(m.top),
                        full,
                    );
                    continue;
                }
                let Some(path) = outline(&mut cache.paths, &mut cache.bytes, clock, &font, g.id)
                else {
                    continue;
                };
                let bytes = &mut cache.bytes;
                let m = cache.masks.entry(key).or_insert_with(|| {
                    let m = glyph_mask(path, unit, f32::from(sub) / 4.0);
                    let cost = m.as_ref().map_or(16, |m| m.data.len().saturating_add(32));
                    *bytes = bytes.saturating_add(cost);
                    Entry {
                        v: m,
                        cost,
                        used: clock,
                    }
                });
                m.used = clock;
                match &m.v {
                    Some(m) => blit_mask(
                        pm,
                        m,
                        (px as i32).saturating_add(m.left),
                        (py as i32).saturating_add(m.top),
                        col,
                    ),
                    None => {
                        let t = Transform::from_row(unit, 0.0, 0.0, unit, gx, gy);
                        pm.fill_path(path, &paint_of(col), FillRule::Winding, t, None);
                    }
                }
            }
            let m = run.metrics();
            let x0 = ox + gr.offset() * scale;
            let base = oy + gr.baseline() * scale;
            for (deco, off, sz) in [
                (&style.underline, m.underline_offset, m.underline_size),
                (
                    &style.strikethrough,
                    m.strikethrough_offset,
                    m.strikethrough_size,
                ),
            ] {
                let Some(u) = deco else {
                    continue;
                };
                let off = u.offset.unwrap_or(off) * scale;
                let sz = (u.size.unwrap_or(sz) * scale).round().max(1.0);
                if let Some(r) =
                    tiny_skia::Rect::from_xywh(x0, (base - off).round(), gr.advance() * scale, sz)
                {
                    pm.fill_rect(r, &paint_of(u.brush.color), Transform::identity(), None);
                }
            }
        }
    }
}

/// The font of a glyph run, as the caches key it.
struct Font<'a> {
    font: Option<&'a FontRef<'a>>,
    blob: u64,
    coords_hash: u32,
    coords: &'a [NormalizedCoord],
}

/// The outline of glyph `id` in font units (y down), cached.
fn outline<'p>(
    paths: &'p mut HashMap<PathKey, Entry<Option<Path>>>,
    bytes: &mut usize,
    clock: u64,
    font: &Font,
    id: u32,
) -> Option<&'p Path> {
    let e = paths
        .entry((font.blob, id, font.coords_hash))
        .or_insert_with(|| {
            let draw = || {
                let glyph = font.font?.outline_glyphs().get(GlyphId::new(id))?;
                let mut pen = Pen(PathBuilder::new());
                let loc = if font.coords.is_empty() {
                    LocationRef::default()
                } else {
                    LocationRef::new(font.coords)
                };
                glyph
                    .draw(DrawSettings::unhinted(Size::unscaled(), loc), &mut pen)
                    .ok()?;
                pen.0.finish()
            };
            let v = draw();
            let cost = path_cost(v.as_ref());
            *bytes = bytes.saturating_add(cost);
            Entry {
                v,
                cost,
                used: clock,
            }
        });
    e.used = clock;
    e.v.as_ref()
}

/// The COLR layers of a colour glyph: (glyph, transform in font units with
/// y down, palette index, alpha).
#[derive(Default)]
struct Layers {
    transforms: Vec<Transform>,
    /// Clip glyphs with the transform they were pushed under; `None` for a
    /// clip box (not applied).
    clips: Vec<Option<(u32, Transform)>>,
    out: Vec<(u32, Transform, u16, f32)>,
}

impl Layers {
    fn current(&self) -> Transform {
        self.transforms
            .last()
            .copied()
            .unwrap_or_else(Transform::identity)
    }
}

impl ColorPainter for Layers {
    fn push_transform(&mut self, t: color::Transform) {
        // Font units are y up; outlines here are y down.
        let t = Transform::from_row(t.xx, -t.yx, -t.xy, t.yy, t.dx, -t.dy);
        let t = self.current().pre_concat(t);
        self.transforms.push(t);
    }
    fn pop_transform(&mut self) {
        self.transforms.pop();
    }
    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let t = self.current();
        self.clips.push(Some((glyph_id.to_u32(), t)));
    }
    fn push_clip_box(&mut self, _clip_box: BoundingBox<f32>) {
        self.clips.push(None);
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
    fn fill(&mut self, brush: color::Brush<'_>) {
        use color::Brush::*;
        let (index, alpha) = match brush {
            Solid {
                palette_index,
                alpha,
            } => (palette_index, alpha),
            // No gradients: the first stop's colour.
            LinearGradient { color_stops, .. }
            | RadialGradient { color_stops, .. }
            | SweepGradient { color_stops, .. } => match color_stops.first() {
                Some(s) => (s.palette_index, s.alpha),
                None => return,
            },
        };
        if let Some(Some((glyph, t))) = self.clips.iter().rev().find(|c| c.is_some())
            && self.out.len() < MAX_LAYERS
        {
            self.out.push((*glyph, *t, index, alpha));
        }
    }
    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}

/// Glyph `id` painted in colour at `unit` px per font unit, shifted right
/// by `sub` quarter pixels; `None` when the glyph has no colour layers (or
/// is too big), so it is drawn as a plain outline.
fn color_glyph<'c>(
    cache: &'c mut GlyphCache,
    font: &Font,
    id: u32,
    key: ColorKey,
    unit: f32,
    sub: u8,
) -> Option<&'c ColorMask> {
    let clock = cache.clock;
    if !cache.colors.contains_key(&key) {
        let v = paint_color(cache, font, id, key.1, unit, f32::from(sub) / 4.0);
        let cost = v
            .as_ref()
            .map_or(16, |m| m.pix.data().len().saturating_add(64));
        cache.bytes = cache.bytes.saturating_add(cost);
        cache.colors.insert(
            key,
            Entry {
                v,
                cost,
                used: clock,
            },
        );
    }
    let e = cache.colors.get_mut(&key)?;
    e.used = clock;
    e.v.as_ref()
}

fn paint_color(
    cache: &mut GlyphCache,
    font: &Font,
    id: u32,
    text: [u8; 4],
    unit: f32,
    fx: f32,
) -> Option<ColorMask> {
    let fr = font.font?;
    let glyphs = fr.color_glyphs();
    let gid = GlyphId::new(id);
    let glyph = glyphs
        .get_with_format(gid, ColorGlyphFormat::ColrV0)
        .or_else(|| glyphs.get_with_format(gid, ColorGlyphFormat::ColrV1))?;
    let loc = if font.coords.is_empty() {
        LocationRef::default()
    } else {
        LocationRef::new(font.coords)
    };
    let mut layers = Layers::default();
    glyph.paint(loc, &mut layers).ok()?;
    let palettes = ColorPalettes::new(fr);
    let palette = palettes.get(0);
    let colors = palette.as_ref().map_or(&[][..], |p| p.colors());
    // Each layer's outline in device px, with its colour.
    let scale = Transform::from_scale(unit, unit);
    let mut paths: Vec<(Path, [u8; 4])> = Vec::new();
    for &(layer, t, index, alpha) in &layers.out {
        let Some(path) = outline(&mut cache.paths, &mut cache.bytes, cache.clock, font, layer)
        else {
            continue;
        };
        let Some(path) = path.clone().transform(scale.pre_concat(t)) else {
            continue;
        };
        let c = if index == 0xFFFF {
            text
        } else {
            let c = colors.get(usize::from(index))?;
            [c.red, c.green, c.blue, c.alpha]
        };
        let a = (f32::from(c[3]) * alpha.clamp(0.0, 1.0)).round() as u8;
        if a > 0 {
            paths.push((path, [c[0], c[1], c[2], a]));
        }
    }
    let mut it = paths.iter().map(|(p, _)| p.bounds());
    let first = it.next()?;
    let (l, t, r, b) = it.fold(
        (first.left(), first.top(), first.right(), first.bottom()),
        |(l, t, r, b), x| {
            (
                l.min(x.left()),
                t.min(x.top()),
                r.max(x.right()),
                b.max(x.bottom()),
            )
        },
    );
    let x0 = ((l + fx).floor() as i32).saturating_sub(1);
    let y0 = (t.floor() as i32).saturating_sub(1);
    let x1 = ((r + fx).ceil() as i32).saturating_add(1);
    let y1 = (b.ceil() as i32).saturating_add(1);
    let w = u32::try_from(x1.saturating_sub(x0)).ok()?.max(1);
    let h = u32::try_from(y1.saturating_sub(y0)).ok()?.max(1);
    if w > MAX_COLOR_PX || h > MAX_COLOR_PX {
        return None;
    }
    let mut pix = Pixmap::new(w, h)?;
    let at = Transform::from_translate(fx - x0 as f32, -(y0 as f32));
    for (path, c) in &paths {
        pix.fill_path(path, &paint_of(*c), FillRule::Winding, at, None);
    }
    Some(ColorMask {
        left: x0,
        top: y0,
        pix,
    })
}

/// Coverage mask of `path` (font units) at `unit` px per unit, shifted
/// right by `fx` px; `None` for huge glyphs (drawn as paths instead).
fn glyph_mask(path: &Path, unit: f32, fx: f32) -> Option<GlyphMask> {
    let b = path.bounds();
    let x0 = ((b.left() * unit + fx).floor() as i32).saturating_sub(1);
    let y0 = ((b.top() * unit).floor() as i32).saturating_sub(1);
    let x1 = ((b.right() * unit + fx).ceil() as i32).saturating_add(1);
    let y1 = ((b.bottom() * unit).ceil() as i32).saturating_add(1);
    let w = u32::try_from(x1.saturating_sub(x0)).ok()?.max(1);
    let h = u32::try_from(y1.saturating_sub(y0)).ok()?.max(1);
    if w > 2048 || h > 2048 {
        return None;
    }
    let mut mask = Mask::new(w, h)?;
    let t = Transform::from_row(unit, 0.0, 0.0, unit, fx - x0 as f32, -(y0 as f32));
    mask.fill_path(path, FillRule::Winding, true, t);
    Some(GlyphMask {
        w,
        h,
        left: x0,
        top: y0,
        data: mask.data().to_vec(),
    })
}

/// `a * b / 255`, rounded, for 8-bit channel values.
fn mul255(a: u32, b: u32) -> u32 {
    a.wrapping_mul(b).wrapping_add(127) / 255
}

/// The rows and columns where a `w` x `h` source placed at (dx, dy)
/// overlaps the clip rectangle, as (x0, y0, x1, y1) in destination pixels.
fn overlap(
    dx: i32,
    dy: i32,
    w: u32,
    h: u32,
    clip: (i32, i32, i32, i32),
) -> Option<(usize, usize, usize, usize)> {
    let w = i32::try_from(w).ok()?;
    let h = i32::try_from(h).ok()?;
    let x0 = dx.max(clip.0).max(0);
    let y0 = dy.max(clip.1).max(0);
    let x1 = dx.saturating_add(w).min(clip.2);
    let y1 = dy.saturating_add(h).min(clip.3);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((
        usize::try_from(x0).ok()?,
        usize::try_from(y0).ok()?,
        usize::try_from(x1).ok()?,
        usize::try_from(y1).ok()?,
    ))
}

/// Src-over copy of a premultiplied image placed at (dx, dy), clipped to
/// `area` and the pixmap.
fn blit_image(pm: &mut Pixmap, src: &Pixmap, dx: i32, dy: i32, area: Rect) {
    let (pw, ph) = (pm.width(), pm.height());
    let clip = (
        area.x.round() as i32,
        area.y.round() as i32,
        ((area.x + area.w).round() as i32).min(pw as i32),
        ((area.y + area.h).round() as i32).min(ph as i32),
    );
    let Some((x0, y0, x1, y1)) = overlap(dx, dy, src.width(), src.height(), clip) else {
        return;
    };
    let (sx0, sy0) = (x0.wrapping_sub(dx as usize), y0.wrapping_sub(dy as usize));
    let (sw, pw) = (src.width() as usize, pw as usize);
    let s = src.data().as_chunks::<4>().0;
    let d = pm.data_mut().as_chunks_mut::<4>().0;
    let n = x1.saturating_sub(x0);
    for (row, y) in (y0..y1).enumerate() {
        let si = sy0
            .saturating_add(row)
            .saturating_mul(sw)
            .saturating_add(sx0);
        let di = y.saturating_mul(pw).saturating_add(x0);
        let (Some(srow), Some(drow)) = (
            s.get(si..si.saturating_add(n)),
            d.get_mut(di..di.saturating_add(n)),
        ) else {
            continue;
        };
        for (sp, dp) in srow.iter().zip(drow) {
            let a = u32::from(sp[3]);
            if a == 255 {
                *dp = *sp;
            } else if a != 0 {
                let inv = 255u32.wrapping_sub(a);
                for (dc, &sc) in dp.iter_mut().zip(sp) {
                    *dc = u32::from(sc)
                        .wrapping_add(mul255(u32::from(*dc), inv))
                        .min(255) as u8;
                }
            }
        }
    }
}

/// Src-over blit of an A8 coverage mask in a solid colour.
fn blit_mask(pm: &mut Pixmap, m: &GlyphMask, dx: i32, dy: i32, c: [u8; 4]) {
    let (pw, ph) = (pm.width(), pm.height());
    let clip = (0, 0, pw as i32, ph as i32);
    let Some((x0, y0, x1, y1)) = overlap(dx, dy, m.w, m.h, clip) else {
        return;
    };
    let ca = u32::from(c[3]);
    // Premultiplied source colour.
    let pre = [
        mul255(u32::from(c[0]), ca),
        mul255(u32::from(c[1]), ca),
        mul255(u32::from(c[2]), ca),
        ca,
    ];
    let (mw, pw) = (m.w as usize, pw as usize);
    let (mx0, my0) = (x0.wrapping_sub(dx as usize), y0.wrapping_sub(dy as usize));
    let d = pm.data_mut().as_chunks_mut::<4>().0;
    let n = x1.saturating_sub(x0);
    for (row, y) in (y0..y1).enumerate() {
        let mi = my0
            .saturating_add(row)
            .saturating_mul(mw)
            .saturating_add(mx0);
        let di = y.saturating_mul(pw).saturating_add(x0);
        let (Some(mrow), Some(drow)) = (
            m.data.get(mi..mi.saturating_add(n)),
            d.get_mut(di..di.saturating_add(n)),
        ) else {
            continue;
        };
        for (&cov, px) in mrow.iter().zip(drow) {
            let cov = u32::from(cov);
            if cov == 0 {
                continue;
            }
            if cov == 255 && ca == 255 {
                *px = [pre[0] as u8, pre[1] as u8, pre[2] as u8, 255];
                continue;
            }
            let inv = 255u32.wrapping_sub(mul255(ca, cov));
            for (dc, &sc) in px.iter_mut().zip(&pre) {
                *dc = mul255(sc, cov)
                    .wrapping_add(mul255(u32::from(*dc), inv))
                    .min(255) as u8;
            }
        }
    }
}
