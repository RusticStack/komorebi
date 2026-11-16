//! Sending a draft: what goes out, the message built off the UI thread
//! while the undo window runs, and the Preview of what the recipient gets.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{Draft, quote::text_html};
use crate::db::{self, Db};
use crate::mail::rich as model;
use crate::mail::smtp::{self, Outgoing};
use crate::mail::{self, render};

/// A send in its undo window (outbox row in `Outbox::Pending`).
pub struct PendingSend {
    /// Tells this send apart from a later one of the same row.
    pub seq: u64,
    pub account: i64,
    /// Set by Undo; the build then stores nothing.
    pub cancel: Arc<AtomicBool>,
    /// The built message is stored.
    pub built: bool,
    /// The undo window is over.
    pub due: bool,
    /// The subject for the offer, shortened (`short_subject`).
    pub subject: String,
}

impl PendingSend {
    /// As Undo does: a build finishing now stores nothing.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Subjects longer than this are cut (with an ellipsis) in the undo-send
/// offer, which floats over the list and must stay one or two lines.
pub const OFFER_SUBJECT_CHARS: usize = 40;

/// `subject` for the undo-send offer: `none` ("(no subject)" in the
/// language shown) when empty, cut at [`OFFER_SUBJECT_CHARS`].
pub fn short_subject(subject: &str, none: &str) -> String {
    let s = subject.trim();
    if s.is_empty() {
        return none.to_string();
    }
    match s.char_indices().nth(OFFER_SUBJECT_CHARS) {
        Some((i, _)) => format!("{}\u{2026}", s[..i].trim_end()),
        None => s.to_string(),
    }
}

/// What draft `d` sends from `account`: a formatted draft its text and,
/// when it needs one, an HTML alternative with the pictures it shows (a
/// `multipart/related`); a plain-text draft its text only.
pub fn outgoing(db: &Db, account: &db::Account, d: Draft) -> Outgoing {
    let (body, body_html, inline) = match &d.rich {
        // A picture whose bytes are gone is left out (`rich::send_parts`).
        Some(ed) => {
            let images = db
                .with_signature_images(ed.doc(), ed.images())
                .unwrap_or_else(|_| ed.images().clone());
            model::send_parts(ed.doc(), &images)
        }
        // Plain text is sent as plain text only, as typed: marks such as
        // `**bold**` or `- ` stay text, and no HTML quote goes with it.
        None => (d.full_body(), None, Vec::new()),
    };
    Outgoing {
        from_name: account.name.clone(),
        from_email: account.email.clone(),
        to: d.to,
        cc: d.cc,
        bcc: d.bcc,
        subject: d.subject,
        body,
        body_html,
        inline,
        attachments: d.attachments.into_iter().map(|a| a.file).collect(),
        in_reply_to: d.in_reply_to,
        references: d.references,
        message_id: d.message_id,
        calendar: None,
        importance: d.importance,
    }
}

/// Build the message of outbox row `row` off the UI thread.
pub struct BuildJob {
    pub row: i64,
    pub seq: u64,
    pub db_path: PathBuf,
    pub outgoing: Outgoing,
    pub cancel: Arc<AtomicBool>,
}

impl BuildJob {
    /// Build the raw message and store it with the row (unless the send
    /// was undone meanwhile): `Ok(false)` if it wasn't stored.
    pub fn run(self) -> Result<bool, String> {
        let raw = smtp::build_raw(self.outgoing).map_err(|e| format!("{e:#}"))?;
        if self.cancel.load(Ordering::SeqCst) {
            return Ok(false);
        }
        Db::open_worker(&self.db_path)
            .and_then(|db| db.store_raw(self.row, &raw, &self.cancel))
            .map_err(|e| format!("{e:#}"))
    }
}

/// The composer's Preview, rendered: the page's pixels (premultiplied
/// RGBA, at the device scale), its size in logical px, its canvas colour,
/// and whether it was cut at the longest band one image holds.
pub struct PreviewPage {
    pub pixmap: tiny_skia::Pixmap,
    /// Logical px: the page is never drawn wider than it was laid out.
    pub width: f32,
    pub height: f32,
    pub bg: [u8; 4],
    pub cut: bool,
    /// The page's lines of text, for screen readers (CSS px on the page;
    /// the ones drawn).
    pub lines: Vec<render::TextLine>,
}

/// The tallest preview drawn, in device pixels (one raster band).
pub const PREVIEW_MAX_ROWS: u32 = 4096;
/// The preview thread's stack: layout recurses per nesting level (the
/// reader worker's).
pub const PREVIEW_STACK: usize = 8 * 1024 * 1024;

/// The HTML Preview shows for draft `d`: what the recipient gets.
pub fn preview_html(d: &Draft) -> String {
    match &d.rich {
        Some(ed) if model::needs_html(ed.doc()) => model::html(ed.doc(), model::Mode::Send),
        Some(ed) => mail::compose_html::preview_html(&model::plain(ed.doc()), None),
        None => format!(
            "<div style=\"white-space:pre-wrap\">{}</div>",
            text_html(&d.full_body())
        ),
    }
}

/// Lay out and draw `html` (the draft's HTML part) as Preview shows it,
/// under the renderer's caps and deadline. Off the UI thread, on a stack
/// of [`PREVIEW_STACK`].
pub fn preview_page(
    html: String,
    width: f32,
    scale: f32,
    dark: bool,
) -> Result<PreviewPage, render::RenderError> {
    let input = render::RenderInput {
        html: Some(html),
        ..Default::default()
    };
    let opts = render::Opts {
        width_px: width.clamp(160.0, 960.0),
        scale,
        dim: dark,
        ..Default::default()
    };
    let mut engine = render::Engine::new();
    let deadline = render::Deadline::after(render::LAYOUT_BUDGET);
    let doc = render::layout(&input, &opts, &mut engine, &deadline)?;
    let scale = scale.clamp(0.5, 4.0);
    let full = (doc.height * scale).ceil().max(1.0) as u32;
    let rows = full.min(PREVIEW_MAX_ROWS);
    let pixmap = render::rasterize(&doc, &mut engine, 0, rows, scale)
        .ok_or(render::RenderError::TooLarge)?;
    let height = rows as f32 / scale;
    let lines = doc
        .text_lines()
        .into_iter()
        .filter(|l| l.y < height)
        .collect();
    Ok(PreviewPage {
        width: pixmap.width() as f32 / scale,
        height,
        bg: doc.bg,
        cut: full > rows || doc.truncated,
        pixmap,
        lines,
    })
}
