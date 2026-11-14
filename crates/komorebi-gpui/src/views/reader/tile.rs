//! The tile element: a laid-out page drawn from its 256 px tiles
//! (`komorebi::reader`), painted with `Window::paint_image` on the page's
//! own background, with the selection's highlight and the focused link's
//! ring over the pixels (no re-raster). Every edge is snapped to device
//! pixels on its own (cookbook_device_pixels), so tiles rasterized 1:1 at
//! 125 and 150 percent stay sharp and meet without a seam.
//!
//! The element measures itself while it prepaints (cookbook_measure): its
//! bounds go to the view after the frame, which turns them into the visible
//! band of the page and asks for the tiles there.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, BorderStyle, Bounds, Corners, Hsla, IntoElement, Pixels, Styled, Window, canvas, fill,
    outline, point, px, size,
};
use komorebi::mail::render::Rect;
use komorebi::reader::TILE_H;

use super::worker::TileImage;

/// What the page element paints.
#[derive(Clone)]
pub struct PagePaint {
    /// The page colour.
    pub bg: Hsla,
    /// The tiles to draw: index and image. Each fills its own span, at
    /// the page's width (a relayout's old tiles stretch until new ones
    /// arrive).
    pub tiles: Vec<(u32, f32, TileImage)>,
    pub width: f32,
    pub height: f32,
    /// Highlight rectangles, page coordinates.
    pub selection: Vec<Rect>,
    pub selection_color: Hsla,
    /// The focused link's ring rectangles.
    pub focus: Vec<Rect>,
    pub ring: Hsla,
    pub ring_width: f32,
    pub ring_radius: f32,
}

/// The device-pixel edges of `[v, v + len)` at `scale`, each rounded on
/// its own (cookbook_device_pixels).
pub fn snap(v: f32, len: f32, scale: f32) -> (f32, f32) {
    let r = |x: f32| (x * scale).round() / scale;
    (r(v), r(v + len))
}

/// `r` (page coordinates) on a page whose origin is at `origin`, snapped.
fn on_page(origin: gpui::Point<Pixels>, r: &Rect, scale: f32) -> Bounds<Pixels> {
    let (x0, x1) = snap(f32::from(origin.x) + r.x, r.w, scale);
    let (y0, y1) = snap(f32::from(origin.y) + r.y, r.h, scale);
    Bounds::new(point(px(x0), px(y0)), size(px(x1 - x0), px(y1 - y0)))
}

/// The page element, sized to the page; `measured` receives its bounds
/// each frame they change (in prepaint: defer any state change).
pub fn page_element(
    paint: PagePaint,
    measured: Rc<Cell<Option<Bounds<Pixels>>>>,
    on_measured: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (w, h) = (paint.width, paint.height);
    canvas(
        move |bounds, window, cx| {
            if measured.get() != Some(bounds) {
                measured.set(Some(bounds));
                on_measured(bounds, window, cx);
            }
        },
        move |bounds, (), window, _cx| {
            let scale = window.scale_factor();
            let origin = bounds.origin;
            let whole = Rect {
                x: 0.,
                y: 0.,
                w: paint.width,
                h: paint.height,
            };
            window.paint_quad(fill(on_page(origin, &whole, scale), paint.bg));
            for (index, height, image) in &paint.tiles {
                let span = Rect {
                    x: 0.,
                    y: *index as f32 * TILE_H,
                    w: paint.width,
                    h: *height,
                };
                let b = on_page(origin, &span, scale);
                // A tile that cannot be drawn leaves the page colour.
                let _ = window.paint_image(b, b, Corners::default(), image.clone(), 0, false);
            }
            for r in &paint.selection {
                window.paint_quad(fill(on_page(origin, r, scale), paint.selection_color));
            }
            for r in &paint.focus {
                let ring = outline(on_page(origin, r, scale), paint.ring, BorderStyle::Solid)
                    .border_widths(px(paint.ring_width))
                    .corner_radii(px(paint.ring_radius));
                window.paint_quad(ring);
            }
        },
    )
    .w(px(w))
    .h(px(h))
    .flex_none()
}

/// The tiles of a page `height` px tall that touch the band `top..top +
/// view` (page coordinates).
pub fn visible(top: f32, view: f32, height: f32) -> Option<(u32, u32)> {
    komorebi::reader::tile_range(top, top + view, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbouring_tile_edges_meet_at_every_scale() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for i in 0..20 {
                let (_, bottom) = snap(10.3 + i as f32 * TILE_H, TILE_H, scale);
                let (top, _) = snap(10.3 + (i + 1) as f32 * TILE_H, TILE_H, scale);
                assert_eq!(bottom, top, "scale {scale} tile {i}");
                // Whole device pixels.
                let d = top * scale;
                assert!((d - d.round()).abs() < 1e-3, "{d}");
            }
        }
    }

    #[test]
    fn the_visible_tiles_follow_the_band() {
        assert_eq!(visible(-300.0, 600.0, 5000.0), Some((0, 1)));
        assert_eq!(visible(2700.0, 600.0, 5000.0), Some((10, 12)));
        assert_eq!(visible(-900.0, 600.0, 5000.0), None);
    }
}
