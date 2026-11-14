//! Preview (the More menu): what the recipient gets, the draft's HTML part
//! laid out and drawn by the reader's renderer (`compose::send::preview_page`,
//! off the UI thread on the renderer's stack), in place of the body until
//! it is turned off. The newest render wins; a failed one says so.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Bounds, Context, Corners, IntoElement, ParentElement, RenderImage, Role, SharedString, Styled,
    canvas, div, point, prelude::*, px, size,
};
use gpui_kit::TestSupportExt as _;
use komorebi::compose::send::{PREVIEW_STACK, preview_html, preview_page};
use komorebi::mail::render::TextLine;

use super::{ComposerView, text};
use crate::theme::{ActiveTokens as _, TextStyled as _, composer as geo};
use crate::views::reader::a11y;

/// A preview being drawn or drawn.
#[derive(Default)]
pub struct Preview {
    seq: u64,
    image: Option<(Arc<RenderImage>, f32, f32)>,
    /// The page's lines of text, for screen readers.
    lines: Rc<Vec<TextLine>>,
    failed: bool,
}

/// A drawn preview: its picture, its size (logical px) and its lines.
type Page = (Arc<RenderImage>, f32, f32, Vec<TextLine>);

impl ComposerView {
    pub(crate) fn previewing(&self) -> bool {
        self.preview.is_some()
    }

    /// Preview off (another draft, or the composer closes).
    pub(crate) fn end_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(old) = self.preview.take()
            && let Some((image, _, _)) = old.image
        {
            cx.defer(move |cx| cx.drop_image(image, None));
        }
    }

    /// Preview on or off.
    pub fn toggle_preview(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if self.preview.is_some() {
            self.end_preview(cx);
            cx.notify();
            return;
        }
        let Some(html) = self.lend(cx, |d| preview_html(d)) else {
            return;
        };
        self.preview_seq += 1;
        let seq = self.preview_seq;
        self.preview = Some(Preview {
            seq,
            ..Preview::default()
        });
        let width = self.body.read(cx).page_width().unwrap_or(geo::BODY_MAX_W);
        let scale = window.scale_factor();
        self.preview_scale = scale;
        let dark = cx.tokens().scheme.is_dark();
        let (tx, rx) = futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("preview".into())
            .stack_size(PREVIEW_STACK)
            .spawn(move || {
                let page = preview_page(html, width, scale, dark).ok().map(|p| {
                    let (w, h) = (p.width, p.height);
                    (
                        komorebi::compose::rich::straight_bgra(p.pixmap),
                        w,
                        h,
                        p.lines,
                    )
                });
                let _ = tx.send(page);
            });
        if spawned.is_err() {
            return self.preview_done(seq, None, cx);
        }
        cx.spawn(async move |this, cx| {
            let page = rx.await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                let image = page.and_then(|((w, h, data), lw, lh, lines)| {
                    let buf = image::RgbaImage::from_raw(w, h, data)?;
                    let img = Arc::new(RenderImage::new(vec![image::Frame::new(buf)]));
                    Some((img, lw, lh, lines))
                });
                this.preview_done(seq, image, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn preview_done(&mut self, seq: u64, page: Option<Page>, cx: &mut Context<Self>) {
        match self.preview.as_mut() {
            Some(p) if p.seq == seq => {
                p.failed = page.is_none();
                if let Some((image, w, h, lines)) = page {
                    p.image = Some((image, w, h));
                    p.lines = Rc::new(lines);
                }
            }
            // Turned off, or a later one started: this picture goes.
            _ => {
                if let Some((image, _, _, _)) = page {
                    cx.defer(move |cx| cx.drop_image(image, None));
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn render_preview(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let p = self.preview.as_ref()?;
        let t = cx.tokens().clone();
        // Where the page was laid out this frame, for its text nodes.
        let page_at: Rc<Cell<Option<Bounds<gpui::Pixels>>>> = Rc::default();
        let (lines, at, scale) = (p.lines.clone(), page_at.clone(), self.preview_scale);
        let body = match (&p.image, p.failed) {
            (Some((image, w, h)), _) => {
                let (image, w, h) = (image.clone(), *w, *h);
                div()
                    .id("compose-preview-page")
                    .overflow_y_scroll()
                    .size_full()
                    .child(
                        canvas(
                            move |bounds, _, _| page_at.set(Some(bounds)),
                            move |bounds: Bounds<gpui::Pixels>, _, window, _| {
                                let b = Bounds::new(
                                    point(bounds.left(), bounds.top()),
                                    size(px(w), px(h)),
                                );
                                let _ = window.paint_image(
                                    b,
                                    b,
                                    Corners::default(),
                                    image.clone(),
                                    0,
                                    false,
                                );
                            },
                        )
                        .w(px(w))
                        .h(px(h)),
                    )
                    .into_any_element()
            }
            (None, true) => div()
                .typo(&t.text.body)
                .text_color(t.color.ink_2)
                .child(SharedString::from(text::words::preview_failed()))
                .into_any_element(),
            (None, false) => div().into_any_element(),
        };
        Some(
            div()
                .id("compose-preview")
                .role(Role::Document)
                .aria_label(SharedString::from(text::words::preview()))
                // What the recipient gets, a line at a time (the reader's
                // text nodes).
                .a11y_synthetic_children(move |builder| {
                    if let Some(page) = at.get() {
                        a11y::push(builder, a11y::text_nodes(&lines, page, scale));
                    }
                })
                .flex_1()
                .min_h_0()
                .px(px(geo::INSET_LEFT))
                .py(px(geo::BODY_PAD_Y))
                .child(body)
                .test_support(),
        )
    }
}
