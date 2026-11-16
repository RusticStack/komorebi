//! The composer's model, shared by both apps: drafts live in Rust, keyed by
//! id, and a composer view shows the active one. One draft shows at a time
//! in a view; the others wait in memory until they are sent or discarded.
//!
//! Drafts worth keeping are also saved to the cache (`db::DraftRow`): two
//! seconds after the last edit, and at once when the composer closes, another
//! draft takes its place, its account changes or the window closes. Their
//! attachments are copied into the drafts folder once, when added.
//!
//! The sync worker also copies saved drafts to the server's Drafts folder
//! (`Job::SaveDraft`): at most every 30 seconds while one is typed, and when
//! the composer closes or another draft takes its place.
//!
//! Nothing here knows a toolkit or shows a sentence: the views own the
//! words (in the language shown) and the timers; this module owns what a
//! draft is, how it is saved, signed, quoted, checked and sent.

pub mod address;
pub mod attach;
pub mod markup;
pub mod quote;
pub mod recipients;
pub mod rich;
pub mod sample;
pub mod send;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::{self, Db};
use crate::i18n::Lang;
use crate::mail::mime::Body;
use crate::mail::rich as model;
use crate::mail::rich::edit::Editor;
use crate::mail::smtp::OutAttachment;
use crate::mail::{self, smtp};

pub use crate::tabs::DraftId;
pub use address::{
    address_at, bare_address, caret_after_edit, complete_address, reply_recipients, typed_address,
};
pub use attach::{AttachFail, AttachWhy, attachments_size, read_attachment, read_files};
pub use quote::{
    FORWARDED, attribution, prefixed, quotable, quote, quote_block, signature_block,
    strip_signature, swap_signature, text_html,
};
pub use send::{BuildJob, OFFER_SUBJECT_CHARS, PendingSend, outgoing, short_subject};

/// A composer field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    To,
    Cc,
    Bcc,
    Subject,
    Body,
}

/// What a draft is: a new message, a reply, a reply to all or a forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl Kind {
    /// The `drafts.kind` value.
    pub fn code(self) -> i64 {
        match self {
            Kind::New => 0,
            Kind::Reply => 1,
            Kind::ReplyAll => 2,
            Kind::Forward => 3,
        }
    }

    pub fn from_code(code: i64) -> Self {
        match code {
            1 => Kind::Reply,
            2 => Kind::ReplyAll,
            3 => Kind::Forward,
            _ => Kind::New,
        }
    }
}

/// A draft is saved this long after the last edit.
pub const AUTOSAVE_AFTER: Duration = Duration::from_secs(2);

/// While a draft is typed, its autosaves copy it to the server at most
/// this often.
pub const SERVER_SAVE_EVERY: Duration = Duration::from_secs(30);

/// Recipient suggestions shown at most, and the characters of an address
/// typed before they show.
pub const SUGGESTIONS: usize = 6;
pub const SUGGEST_AFTER: usize = 2;

/// The address fields, in the order of `Draft::errors`.
pub const ADDRESS_FIELDS: [Field; 3] = [Field::To, Field::Cc, Field::Bcc];

/// An attachment of a draft, and its saved copy once the draft is saved.
#[derive(Clone)]
pub struct Attached {
    pub file: OutAttachment,
    /// Its `draft_attachments` row.
    pub saved: Option<i64>,
}

impl From<OutAttachment> for Attached {
    fn from(file: OutAttachment) -> Self {
        Attached { file, saved: None }
    }
}

/// The settings a new message starts with (Settings > General > Compose):
/// formatted or plain, and the formatted default font, size and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComposeDefaults {
    pub rich: bool,
    pub font: model::FontId,
    pub size: model::SizePt,
    pub color: Option<model::ColorId>,
}

impl ComposeDefaults {
    /// From the saved settings.
    pub fn from_settings(s: &db::Settings) -> Self {
        ComposeDefaults {
            rich: s.compose_rich,
            font: s.compose_font,
            size: s.compose_size,
            color: s.compose_color,
        }
    }

    /// The (font, size, colour) a formatted document starts with.
    pub fn doc(&self) -> (model::FontId, model::SizePt, Option<model::ColorId>) {
        (self.font, self.size, self.color)
    }
}

impl Default for ComposeDefaults {
    fn default() -> Self {
        ComposeDefaults {
            rich: true,
            font: model::FontId::default(),
            size: model::SizePt::DEFAULT,
            color: None,
        }
    }
}

#[derive(Default)]
pub struct Draft {
    pub account_id: i64,
    pub kind: Kind,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    /// The Cc and Bcc rows are shown.
    pub show_cc: bool,
    /// The message this replies to, to refresh its row once sent.
    pub reply_to: Option<i64>,
    pub in_reply_to: String,
    pub references: String,
    pub attachments: Vec<Attached>,
    /// Minted when the draft is first saved or sent, and kept for resends
    /// (also across restarts).
    pub message_id: String,
    /// Its row in the cache, once saved.
    pub row: Option<i64>,
    /// Changed since it was last saved.
    pub dirty: bool,
    /// The user changed it: typed, picked another account or attachments.
    pub touched: bool,
    /// For To, Cc and Bcc, the first address typed that isn't valid (none:
    /// no error shown). The inline error says so in the language shown.
    pub errors: [Option<String>; 3],
    /// The signature block the app put in the body (`signature_block`;
    /// empty: none). `None` for a saved draft whose body lacks its
    /// account's current block: never swapped.
    pub signature: Option<String>,
    /// A reply's or forward's quote of the original's HTML, for the HTML
    /// part (`compose_html::attach`); the body holds the text quote.
    pub quoted_html: Option<String>,
    /// The last formatting edit, for Ctrl+Z (not saved).
    pub unformat: Option<Unformat>,
    /// Send was pressed with no recipient: To says so (inline, announced)
    /// until an address is typed.
    pub need_recipient: bool,
    /// A reply's or forward's quote, folded behind "Show quoted text":
    /// `body` holds what the composer shows and edits, this the rest. What
    /// is saved and sent is always [`Draft::full_body`].
    pub quote: Option<String>,
    /// When it was last saved (unix seconds), for the header caption.
    pub saved_at: Option<i64>,
    /// High or low importance, set in the formatting bar (normal: none).
    pub importance: db::Importance,
    /// A formatted draft: the document the composer edits (`body`,
    /// `quote`, `quoted_html` and `signature` are then unused; what is
    /// saved and sent comes from the document).
    pub rich: Option<Editor>,
    /// A reply's or forward's quote as HTML (the original's, or its text),
    /// for a draft that starts formatted (`Draft::make_rich`).
    pub rich_quote: Option<String>,
    /// The formatted document a switch to plain text replaced, and the text
    /// it left: Ctrl+Z, or switching back while the text is unchanged,
    /// brings it back.
    pub unplain: Option<(Editor, String)>,
    /// The pictures of the body saved with the draft: (Content-ID, its
    /// `draft_attachments` row).
    pub saved_images: Vec<(String, i64)>,
}

/// The lines a reply's or forward's quote starts with, in the language
/// shown. Each app fills them in with its own words ([`Heads::en`] is the
/// English one the tests use).
pub struct Heads {
    pub lang: Lang,
    /// "On {date}, {name} wrote:" with its two places.
    pub wrote: fn(&str, &str) -> String,
    /// A message's date as the quote heads show it, in `lang`.
    pub when: fn(Lang, i64) -> String,
    pub forwarded: String,
    pub from: String,
    pub date: String,
    pub subject: String,
    pub to: String,
}

impl Heads {
    /// The English heads, with a plain date.
    pub fn en() -> Self {
        Heads {
            lang: Lang::En,
            wrote: |date, name| format!("On {date}, {name} wrote:"),
            when: plain_date,
            forwarded: FORWARDED.into(),
            from: "From".into(),
            date: "Date".into(),
            subject: "Subject".into(),
            to: "To".into(),
        }
    }
}

/// A time of day as `22:13` in local time (empty for none), for the
/// composer's "Draft saved" caption.
pub fn clock(ts: i64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// A date as `2023-11-14 22:13` in local time (empty for none).
fn plain_date(_: Lang, ts: i64) -> String {
    use chrono::TimeZone as _;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) if ts != 0 => t.format("%Y-%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

/// The body before a formatting edit and its selection then. Replacing the
/// text clears the field's own undo history, so Ctrl+Z puts this back
/// while the body is still `after` (typing since is undone first).
pub struct Unformat {
    pub before: String,
    pub after: String,
    pub anchor: usize,
    pub cursor: usize,
}

impl Draft {
    pub fn new(account_id: i64) -> Self {
        Draft {
            account_id,
            ..Default::default()
        }
    }

    /// A reply (or reply all) to `m`, from the account it arrived in.
    pub fn reply(m: &db::Message, body: &Body, me: &str, all: bool, heads: &Heads) -> Self {
        let (to, cc) = reply_recipients(m, &body.reply_to, me, all);
        Draft {
            account_id: m.account_id,
            kind: if all { Kind::ReplyAll } else { Kind::Reply },
            show_cc: !cc.is_empty(),
            to: to.join(", "),
            cc: cc.join(", "),
            subject: prefixed("Re:", &m.subject),
            body: format!(
                "\n\n{}\n{}",
                attribution(m, heads),
                quote(strip_signature(&body.text))
            ),
            reply_to: Some(m.id),
            in_reply_to: m.msgid.clone(),
            references: body.references.clone(),
            ..Default::default()
        }
    }

    /// A forward of `m` with its listed attachments.
    pub fn forward(
        m: &db::Message,
        body: &Body,
        attachments: Vec<Attached>,
        heads: &Heads,
    ) -> Self {
        Draft {
            account_id: m.account_id,
            kind: Kind::Forward,
            subject: prefixed("Fwd:", &m.subject),
            body: format!(
                "\n\n{}\n{}: {} <{}>\n{}: {}\n{}: {}\n{}: {}\n\n{}",
                heads.forwarded,
                heads.from,
                m.from_name,
                m.from_addr,
                heads.date,
                (heads.when)(heads.lang, m.date),
                heads.subject,
                m.subject,
                heads.to,
                m.to,
                body.text
            ),
            attachments,
            ..Default::default()
        }
    }

    /// A forward of `m` as an attachment: its bytes as received, `raw`,
    /// attached as `<subject>.eml` (sent as message/rfc822), and nothing
    /// quoted.
    pub fn forward_attached(m: &db::Message, raw: Vec<u8>) -> Self {
        let file = OutAttachment {
            filename: mail::source::attachment_name(&m.subject),
            data: raw,
        };
        Draft {
            account_id: m.account_id,
            kind: Kind::Forward,
            subject: prefixed("Fwd:", &m.subject),
            attachments: vec![file.into()],
            ..Default::default()
        }
    }

    /// Quote `original`, the sanitized HTML of the message replied to or
    /// forwarded (`compose_html::sanitize`), in the HTML part in place of
    /// the text quote the composer put in the body, for as long as that
    /// text quote stays as it is.
    pub fn quote_html(&mut self, m: &db::Message, original: &str, heads: &Heads) {
        let Some((boundary, block)) = quote_block(self.kind, m, original, heads) else {
            return;
        };
        self.quoted_html = mail::compose_html::attach(&self.full_body(), &boundary, &block);
        self.rich_quote = Some(block);
    }

    /// For a text message replied to or forwarded: its text as the quote of
    /// a draft that starts formatted (the HTML original wins).
    pub fn quote_text(&mut self, m: &db::Message, text: &str, heads: &Heads) {
        if self.rich_quote.is_none() {
            self.rich_quote = quote_block(self.kind, m, &text_html(text), heads).map(|q| q.1);
        }
    }

    /// Start formatted: the text typed so far (a `mailto:` body; a reply's
    /// text quote is left out) in the settings' default font, then the
    /// reply's or forward's quote, folded.
    pub fn make_rich(
        &mut self,
        defaults: (model::FontId, model::SizePt, Option<model::ColorId>),
        signature: Option<model::SignatureBlock>,
    ) {
        let above = match quote_start(&self.body) {
            Some(i) if self.kind != Kind::New => self.body.get(..i).unwrap_or_default(),
            _ => self.body.as_str(),
        };
        let mut doc = rich::doc_from_text(above.trim_end_matches(['\n', '\r']), defaults);
        if doc.blocks.is_empty() {
            doc.blocks.push(model::Block::Para(model::Para::default()));
        }
        // The signature under the text after an empty line, above the
        // quote (as in plain text).
        if let Some(s) = signature {
            doc.blocks.push(model::Block::Para(model::Para::default()));
            doc.blocks.push(model::Block::Signature(s));
        }
        if let Some(q) = self
            .rich_quote
            .take()
            .and_then(|h| model::QuoteBlock::new(&h, true))
        {
            doc.blocks.push(model::Block::Quote(q));
        }
        self.rich = Some(Editor::new(doc));
        self.body.clear();
        self.quote = None;
        self.quoted_html = None;
        self.signature = None;
        self.unformat = None;
    }

    /// The formatted signature in the message (none: plain, or no
    /// signature).
    pub fn rich_signature(&self) -> Option<&model::SignatureBlock> {
        self.rich.as_ref()?.signature()
    }

    /// What is saved and sent: the body with its folded quote, if any (a
    /// formatted draft: its plain text).
    pub fn full_body(&self) -> String {
        if let Some(ed) = &self.rich {
            return model::plain(ed.doc());
        }
        match &self.quote {
            Some(q) => with_quote(&self.body, q),
            None => self.body.clone(),
        }
    }

    /// Fold a reply's or forward's quote out of the composer (it stays in
    /// what is saved and sent). The quote takes the blank lines above it
    /// with it, so a fresh reply's body is empty (the placeholder shows)
    /// and text typed over the whole body keeps its distance from the
    /// quote. Nothing to fold: no change.
    pub fn fold_quote(&mut self) {
        if self.kind == Kind::New || self.quote.is_some() {
            return;
        }
        if let Some(at) = quote_start(&self.body) {
            self.quote = Some(self.body.split_off(at));
        }
    }

    /// "Show quoted text": the whole body in the composer again.
    pub fn unfold_quote(&mut self) -> bool {
        match self.quote.take() {
            Some(q) => {
                self.body = with_quote(&self.body, &q);
                true
            }
            None => false,
        }
    }

    /// The quote is folded out of the composer (plain or formatted).
    pub fn quote_folded(&self) -> bool {
        match &self.rich {
            Some(ed) => ed.has_quote() && ed.quote_folded(),
            None => self.quote.is_some(),
        }
    }

    /// A saved draft, as loaded from the cache.
    pub fn from_row(r: db::DraftRow, attachments: Vec<Attached>) -> Self {
        Draft {
            account_id: r.account_id,
            kind: Kind::from_code(r.kind),
            show_cc: !r.cc.is_empty() || !r.bcc.is_empty(),
            to: r.to,
            cc: r.cc,
            bcc: r.bcc,
            subject: r.subject,
            body: r.body,
            reply_to: r.reply_to_msg,
            in_reply_to: r.in_reply_to,
            references: r.refs,
            attachments,
            message_id: r.message_id,
            quoted_html: r.quoted_html,
            importance: r.importance,
            saved_at: (r.updated > 0).then_some(r.updated),
            // Saved formatted: the document back (a draft saved as plain
            // text stays plain).
            rich: r
                .body_rich
                .as_deref()
                .and_then(model::from_stored)
                .map(Editor::new),
            row: Some(r.id),
            // Saved because the user wrote it.
            touched: true,
            ..Default::default()
        }
    }

    /// The draft's fields as saved in row `id`.
    pub fn to_row(&self, id: i64) -> db::DraftRow {
        db::DraftRow {
            id,
            account_id: self.account_id,
            kind: self.kind.code(),
            reply_to_msg: self.reply_to,
            in_reply_to: self.in_reply_to.clone(),
            refs: self.references.clone(),
            to: self.to.clone(),
            cc: self.cc.clone(),
            bcc: self.bcc.clone(),
            subject: self.subject.clone(),
            body: self.full_body(),
            quoted_html: if self.rich.is_some() {
                None
            } else {
                self.quoted_html.clone()
            },
            message_id: self.message_id.clone(),
            updated: 0,
            importance: self.importance,
            body_rich: self
                .rich
                .as_ref()
                .map(|ed| model::html(ed.doc(), model::Mode::Draft)),
            signature_id: self.rich_signature().and_then(|s| s.source),
        }
    }

    pub fn field(&self, field: Field) -> &str {
        match field {
            Field::To => &self.to,
            Field::Cc => &self.cc,
            Field::Bcc => &self.bcc,
            Field::Subject => &self.subject,
            Field::Body => &self.body,
        }
    }

    /// The user typed into `field`. An address error already shown is
    /// checked again at once, so it clears as soon as the address is right.
    pub fn edit(&mut self, field: Field, text: &str) {
        let slot = match field {
            Field::To => &mut self.to,
            Field::Cc => &mut self.cc,
            Field::Bcc => &mut self.bcc,
            Field::Subject => &mut self.subject,
            Field::Body => &mut self.body,
        };
        slot.clear();
        slot.push_str(text);
        self.touched = true;
        self.dirty = true;
        if address_index(field).is_some() && !text.trim().is_empty() {
            self.need_recipient = false;
        }
        if let Some(i) = address_index(field)
            && self.errors[i].is_some()
        {
            self.validate(field);
        }
    }

    /// Check an address field (on leaving it, and before sending).
    pub fn validate(&mut self, field: Field) {
        if let Some(i) = address_index(field) {
            self.errors[i] = address_error(self.field(field));
        }
    }

    /// The inline error of an address field (the address at fault).
    pub fn error(&self, field: Field) -> Option<&str> {
        address_index(field).and_then(|i| self.errors[i].as_deref())
    }

    /// Nothing typed and nothing attached (the signature the app added
    /// doesn't count).
    pub fn is_empty(&self) -> bool {
        if let Some(ed) = &self.rich {
            let blank = ed.doc().blocks.iter().all(|b| match b {
                model::Block::Para(p) => p.is_empty(),
                model::Block::Signature(s) => s.is_untouched(),
                _ => false,
            });
            return blank
                && [&self.to, &self.cc, &self.bcc, &self.subject]
                    .iter()
                    .all(|s| s.trim().is_empty())
                && self.attachments.is_empty();
        }
        let sig = self.signature.as_deref().unwrap_or_default();
        let full = self.full_body();
        let body = match full.find(sig) {
            Some(i) if !sig.is_empty() => format!("{}{}", &full[..i], &full[i + sig.len()..]),
            _ => full,
        };
        [&self.to, &self.cc, &self.bcc, &self.subject, &body]
            .iter()
            .all(|s| s.trim().is_empty())
            && self.attachments.is_empty()
    }

    /// Add the From account's signature block, once, at compose start:
    /// after what a new message already holds (a `mailto:` body), and above
    /// the quote or forwarded message otherwise.
    pub fn sign(&mut self, block: &str) {
        if self.kind == Kind::New {
            self.body.push_str(block);
        } else {
            self.body.insert_str(0, block);
        }
        self.signature = Some(block.to_string());
    }

    /// The From account changed to one whose signature block is `block`:
    /// swap it in if the block the app added is still there as it was.
    /// True if the body changed.
    pub fn resign(&mut self, block: &str) -> bool {
        let Some(old) = self.signature.as_deref() else {
            return false;
        };
        let swapped = swap_signature(&self.body, self.kind, old, block);
        let Some(body) = swapped else {
            return false;
        };
        self.body = body;
        self.signature = Some(block.to_string());
        true
    }

    /// A plain draft's signature picked from the Signature menu: `new` (a
    /// [`signature_block`], empty for none) takes the place of the block
    /// the app put in; one edited since stays, and the new one goes where
    /// a signature goes.
    pub fn replace_signature(&mut self, new: &str) {
        let old = self.signature.clone().unwrap_or_default();
        if old == new {
            return;
        }
        match swap_signature(&self.body, self.kind, &old, new) {
            Some(body) => self.body = body,
            None if self.kind == Kind::New => self.body.push_str(new),
            None => self.body.insert_str(0, new),
        }
        self.signature = Some(new.to_string());
        self.touched = true;
        self.dirty = true;
    }

    /// Worth keeping when another draft takes its place: the user changed
    /// it. An untouched reply or forward is only what the app filled in.
    pub fn keep(&self) -> bool {
        self.touched && !self.is_empty()
    }

    /// Why Send is off, shown under the toolbar (none: it is on).
    pub fn send_block(&self) -> Option<SendBlock> {
        if let Some(i) = self.errors.iter().position(Option::is_some) {
            return Some(SendBlock::FixAddress(ADDRESS_FIELDS[i]));
        }
        if [&self.to, &self.cc, &self.bcc]
            .iter()
            .all(|s| s.trim().is_empty())
        {
            return Some(SendBlock::NoRecipient);
        }
        // No size check: the server is the only judge of how big a
        // message can be (`mail::smtp`'s "too large" refusal).
        None
    }

    /// Send was asked for: every address is checked, then what stops it,
    /// what to ask first (unless `anyway`), or that it can go.
    pub fn check_send(&mut self, anyway: bool) -> SendCheck {
        for field in ADDRESS_FIELDS {
            self.validate(field);
        }
        match self.send_block() {
            Some(SendBlock::NoRecipient) => {
                self.need_recipient = true;
                SendCheck::Blocked(SendBlock::NoRecipient)
            }
            Some(block) => SendCheck::Blocked(block),
            None => match send_guard(self).filter(|_| !anyway) {
                Some(guard) => SendCheck::Ask(guard),
                None => SendCheck::Ready,
            },
        }
    }

    /// The formatting bar's importance: true if it changed. A blank
    /// message stays untouched (importance alone is not writing, so Esc
    /// still drops it); on anything else it is a choice worth keeping.
    pub fn set_importance(&mut self, importance: db::Importance) -> bool {
        if self.importance == importance {
            return false;
        }
        self.importance = importance;
        self.touched |= !self.is_empty();
        self.dirty = true;
        true
    }

    /// The message's format: plain text, or formatted again (`defaults`
    /// for a new document). A switch to plain text keeps the document, so
    /// switching back before typing gives it back unchanged. True if the
    /// draft changed.
    pub fn set_rich(
        &mut self,
        on: bool,
        defaults: (model::FontId, model::SizePt, Option<model::ColorId>),
    ) -> bool {
        if on {
            if self.rich.is_some() {
                return false;
            }
            let ed = match self.unplain.take() {
                // Nothing typed since the switch: the document as it was.
                Some((ed, text)) if text == self.full_body() => ed,
                _ => Editor::new(rich::doc_from_text(&self.full_body(), defaults)),
            };
            self.rich = Some(ed);
            self.body.clear();
        } else {
            let Some(ed) = self.rich.take() else {
                return false;
            };
            let text = model::plain(ed.doc());
            self.body = text.clone();
            self.unplain = Some((ed, text));
        }
        self.quote = None;
        self.quoted_html = None;
        self.signature = None;
        self.touched = true;
        self.dirty = true;
        true
    }

    /// Ctrl+Z right after a switch to plain text, while the text is still
    /// what the switch left: the formatted message back.
    pub fn can_unplain(&self) -> bool {
        self.unplain
            .as_ref()
            .is_some_and(|(_, text)| *text == self.body)
    }

    /// Ctrl+Z right after a plain-text formatting edit: the body as it was
    /// and the selection then (none: nothing to put back).
    pub fn take_unformat(&mut self) -> Option<Unformat> {
        let body = &self.body;
        self.unformat.take_if(|u| u.after == *body)
    }

    /// A signature picked from the Signature menu (`sig` none: no
    /// signature): it replaces the one in the message, even an edited one
    /// (Ctrl+Z puts it back in a formatted message). True if it changed.
    pub fn pick_signature(&mut self, sig: Option<&db::Signature>) -> bool {
        if let Some(ed) = self.rich.as_mut() {
            if ed.set_signature(sig.and_then(db::Signature::block)) {
                self.touched = true;
                self.dirty = true;
                return true;
            }
            return false;
        }
        let new = sig.map_or_else(String::new, |s| signature_block(&s.body_text));
        let before = self.body.clone();
        self.replace_signature(&new);
        self.body != before
    }

    /// Another From account, whose signature for this kind of mail is
    /// `sig`: it replaces the old one while that is untouched. `None`: the
    /// same account, nothing changed; else whether the body changed (save
    /// the draft either way).
    pub fn switch_account(&mut self, account: i64, sig: Option<&db::Signature>) -> Option<bool> {
        if self.account_id == account {
            return None;
        }
        self.account_id = account;
        // A blank message stays untouched: switching From is not writing,
        // so Esc still drops it silently (a reply keeps the choice).
        self.touched |= !self.is_empty();
        self.dirty = true;
        if let Some(ed) = self.rich.as_mut() {
            // The new account's signature, while the one there is as the
            // app put it in.
            let untouched = ed.signature().is_none_or(|s| s.is_untouched());
            let new = sig.and_then(db::Signature::block);
            let same = ed.signature().map(|s| s.source) == new.as_ref().map(|s| s.source);
            return Some(untouched && !same && ed.set_signature(new));
        }
        let block = sig.map_or_else(String::new, |s| signature_block(&s.body_text));
        Some(self.resign(&block))
    }

    /// Remove attachment `index`, and its saved copy at once. True if one
    /// went.
    pub fn remove_attachment(&mut self, db: &Db, index: usize) -> anyhow::Result<bool> {
        if index >= self.attachments.len() {
            return Ok(false);
        }
        let gone = self.attachments.remove(index);
        self.touched = true;
        self.dirty = true;
        if let (Some(row), Some(file)) = (self.row, gone.saved) {
            db.remove_draft_file(row, file)?;
        }
        Ok(true)
    }

    /// Add files read for this draft, whatever their size (Komorebi sets
    /// no limit). True if any was added (save the draft then: the copies
    /// are made once).
    pub fn add_attachments(&mut self, added: Vec<Attached>) -> bool {
        let any = !added.is_empty();
        self.attachments.extend(added);
        self.touched |= any;
        self.dirty |= any;
        any
    }
}

/// Where an address field is in `Draft::errors`.
pub fn address_index(field: Field) -> Option<usize> {
    ADDRESS_FIELDS.iter().position(|f| *f == field)
}

/// Why Send is off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SendBlock {
    /// An address of this field (To, Cc or Bcc) is not valid.
    FixAddress(Field),
    NoRecipient,
}

/// What the send guards ask about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    NoSubject,
    NoAttachment,
    Both,
}

/// What Send found ([`Draft::check_send`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SendCheck {
    /// It cannot go: the field at fault says why.
    Blocked(SendBlock),
    /// Ask first (the guard bar); "Send anyway" sends.
    Ask(Guard),
    Ready,
}

/// The address at fault in an address field as typed (none: all fine).
pub fn address_error(text: &str) -> Option<String> {
    smtp::first_invalid_address(text)
}

#[derive(Default)]
pub struct Drafts {
    items: HashMap<DraftId, Draft>,
    active: Option<DraftId>,
    next: u64,
    /// The recipient suggestions showing for the active draft.
    pub suggest: Option<Suggest>,
    /// Drafts shown in tabs of their own: never dropped when another one
    /// takes the composer, even with nothing written.
    tabbed: HashSet<DraftId>,
}

/// Recipient suggestions from the address index for the address being
/// typed in an address field.
pub struct Suggest {
    pub field: Field,
    /// Where the caret is taken to be (`caret_after_edit`).
    pub cursor: usize,
    pub items: Vec<db::contacts::Suggested>,
}

impl Drafts {
    /// Add `draft` and make it the active one (see `switch_to`).
    pub fn open(&mut self, draft: Draft) -> DraftId {
        self.next += 1;
        let id = DraftId::new(self.next);
        self.items.insert(id, draft);
        self.switch_to(id);
        id
    }

    /// Make `id` the active draft. The draft it replaces stays if it is
    /// worth keeping or has a tab, and is dropped otherwise (save it first).
    pub fn switch_to(&mut self, id: DraftId) {
        if !self.items.contains_key(&id) {
            return;
        }
        if let Some(old) = self.active
            && old != id
            && !self.tabbed.contains(&old)
            && self.items.get(&old).is_some_and(|d| !d.keep())
        {
            self.items.remove(&old);
        }
        self.active = Some(id);
    }

    /// Add `draft` without showing it (a tab of its own holds it).
    pub fn add(&mut self, draft: Draft) -> DraftId {
        self.next += 1;
        let id = DraftId::new(self.next);
        self.items.insert(id, draft);
        id
    }

    /// Make `id` the active draft, dropping nothing (a tab switch).
    pub fn show(&mut self, id: DraftId) {
        if self.items.contains_key(&id) {
            self.active = Some(id);
        }
    }

    /// Swap the shown draft and its suggestions with another view's
    /// (a composer window of its own): nothing is dropped.
    pub fn swap_view(&mut self, active: &mut Option<DraftId>, suggest: &mut Option<Suggest>) {
        std::mem::swap(&mut self.active, active);
        std::mem::swap(&mut self.suggest, suggest);
    }

    /// No draft is shown (the composer went with it into a tab).
    pub fn hide(&mut self) {
        self.active = None;
    }

    /// Draft `id` has a tab of its own (or no longer has one).
    pub fn set_tabbed(&mut self, id: DraftId, on: bool) {
        if on {
            self.tabbed.insert(id);
        } else {
            self.tabbed.remove(&id);
        }
    }

    pub fn contains(&self, id: DraftId) -> bool {
        self.items.contains_key(&id)
    }

    pub fn active_id(&self) -> Option<DraftId> {
        self.active
    }

    pub fn active(&self) -> Option<&Draft> {
        self.items.get(&self.active?)
    }

    pub fn active_mut(&mut self) -> Option<&mut Draft> {
        self.items.get_mut(&self.active?)
    }

    pub fn get(&self, id: DraftId) -> Option<&Draft> {
        self.items.get(&id)
    }

    pub fn get_mut(&mut self, id: DraftId) -> Option<&mut Draft> {
        self.items.get_mut(&id)
    }

    /// The draft in memory whose cache row is `row`.
    pub fn find_row(&self, row: i64) -> Option<DraftId> {
        self.find(|d| d.row == Some(row))
    }

    /// The draft in memory replying to message `message`.
    pub fn find_reply(&self, message: i64) -> Option<DraftId> {
        self.find(|d| d.reply_to == Some(message))
    }

    fn find(&self, f: impl Fn(&Draft) -> bool) -> Option<DraftId> {
        self.items
            .iter()
            .filter(|(_, d)| f(d))
            .map(|(id, _)| *id)
            .max()
    }

    pub fn ids(&self) -> Vec<DraftId> {
        self.items.keys().copied().collect()
    }

    /// Forget the drafts of a removed account. True if the active draft
    /// went.
    pub fn drop_account(&mut self, account: i64) -> bool {
        self.items.retain(|_, d| d.account_id != account);
        self.tabbed.retain(|id| self.items.contains_key(id));
        let gone = self.active.is_some_and(|id| !self.items.contains_key(&id));
        if gone {
            self.active = None;
        }
        gone
    }

    pub fn remove(&mut self, id: DraftId) -> Option<Draft> {
        if self.active == Some(id) {
            self.active = None;
        }
        self.tabbed.remove(&id);
        self.items.remove(&id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The newest draft worth keeping other than the active one.
    pub fn newest_kept(&self) -> Option<DraftId> {
        self.items
            .iter()
            .filter(|(id, d)| Some(**id) != self.active && d.keep())
            .map(|(id, _)| *id)
            .max()
    }
}

/// The send guards (an empty subject; the text mentions an attachment and
/// none is attached). None: nothing to ask about.
pub fn send_guard(d: &Draft) -> Option<Guard> {
    let no_subject = d.subject.trim().is_empty();
    let text = match &d.rich {
        Some(ed) => std::borrow::Cow::Owned(model::plain(ed.doc())),
        None => std::borrow::Cow::Borrowed(d.body.as_str()),
    };
    let no_attachment = d.attachments.is_empty() && mentions_attachment(&text);
    match (no_subject, no_attachment) {
        (true, true) => Some(Guard::Both),
        (true, false) => Some(Guard::NoSubject),
        (false, true) => Some(Guard::NoAttachment),
        (false, false) => None,
    }
}

/// Whether the user's own text mentions an attachment, in English or
/// Portuguese ("attached", "attachment", "anexo", "em anexo", "anexado",
/// ...). Quoted lines (`>`), a forwarded message and everything from the
/// signature line (`-- `) on are not the user's.
pub fn mentions_attachment(body: &str) -> bool {
    body.lines()
        .take_while(|l| !quote::is_forwarded_line(l) && *l != "-- ")
        .filter(|l| !l.trim_start().starts_with('>'))
        .flat_map(|l| l.split(|c: char| !c.is_alphanumeric()))
        .any(|w| {
            let w = w.to_lowercase();
            w.starts_with("attach") || w.starts_with("anex")
        })
}

/// Where a reply's or forward's quote starts in `body` (a byte offset at
/// a line start), for folding it: a forward's heading line, or the line
/// above a trailing run of `>` lines (the attribution). `None`: no quote.
pub fn quote_start(body: &str) -> Option<usize> {
    let mut at = 0;
    let mut starts = Vec::new();
    let mut found = None;
    for line in body.split_inclusive('\n') {
        if quote::is_forwarded_line(line) {
            found = Some(at);
            break;
        }
        starts.push((at, line.trim_end_matches(['\r', '\n'])));
        at += line.len();
    }
    if found.is_none() {
        // The trailing run of quoted (or blank) lines, then the attribution.
        let mut quoted = 0;
        let mut i = starts.len();
        while i > 0 {
            let (_, l) = starts[i - 1];
            if l.starts_with('>') {
                quoted += 1;
            } else if !l.trim().is_empty() {
                break;
            }
            i -= 1;
        }
        let (start, line) = *starts.get(i.checked_sub(1)?)?;
        found = (quoted > 0 && !line.starts_with('>') && !line.trim().is_empty() && line != "-- ")
            .then_some(start);
    }
    // The blank lines above the heading belong to the quote: the text
    // above keeps its distance from it however it is edited.
    let mut start = found?;
    while start > 0 && matches!(body.as_bytes()[start - 1], b'\n' | b'\r') {
        start -= 1;
    }
    Some(start)
}

/// `body` with its folded `quote` after it: what is saved and sent. A
/// quote folded without its blank lines (none were typed) still gets a
/// blank line between the text and the heading.
pub fn with_quote(body: &str, quote: &str) -> String {
    if body.is_empty() || body.ends_with('\n') || quote.starts_with(['\n', '\r']) {
        format!("{body}{quote}")
    } else {
        format!("{body}\n\n{quote}")
    }
}

/// The text of a draft above its reply or forward quote (the whole text
/// with none): what the Drafts row previews.
pub fn above_quote(body: &str) -> &str {
    &body[..quote_start(body).unwrap_or(body.len())]
}

/// What saving a draft did to the cache.
#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    Nothing,
    Inserted,
    Updated,
    /// With the copy on the server, to remove there.
    Deleted(Option<db::ServerCopy>),
}

/// A copy of a draft's file to write off the UI thread ([`store_rows`]).
#[derive(Debug, Clone)]
pub struct FileJob {
    pub draft: i64,
    /// Its `draft_attachments` row.
    pub file: i64,
    pub copy: db::DraftCopy,
}

impl FileJob {
    pub fn run(&self) -> anyhow::Result<()> {
        self.copy.write()
    }
}

/// Save `d`: a draft worth keeping is written (inserted, with copies of its
/// attachments and pictures, the first time; then one UPDATE of its
/// fields), and one that is not (emptied, or never changed by the user)
/// loses its saved row. `email` is the From address its Message-ID is
/// minted with. The copies are written here: the app writes them off the
/// UI thread instead ([`store_rows`]).
pub fn store(db: &Db, d: &mut Draft, email: &str) -> anyhow::Result<Stored> {
    let (stored, jobs) = store_rows(db, d, email)?;
    for j in &jobs {
        if let Err(e) = j.run() {
            copy_failed(db, d, j);
            return Err(e);
        }
    }
    Ok(stored)
}

/// A copy that could not be written: its row goes, and the next save
/// tries it again.
pub fn copy_failed(db: &Db, d: &mut Draft, job: &FileJob) {
    db.remove_draft_file(job.draft, job.file).ok();
    if d.row == Some(job.draft) {
        for a in &mut d.attachments {
            if a.saved == Some(job.file) {
                a.saved = None;
            }
        }
        d.saved_images.retain(|(_, id)| *id != job.file);
    }
}

/// [`store`] without writing the copies of new attachments and pictures:
/// their rows are recorded and the copies come back, to be written off the
/// UI thread (a large file never holds the window).
pub fn store_rows(db: &Db, d: &mut Draft, email: &str) -> anyhow::Result<(Stored, Vec<FileJob>)> {
    if !d.keep() {
        d.dirty = false;
        d.saved_at = None;
        let Some(row) = d.row.take() else {
            return Ok((Stored::Nothing, Vec::new()));
        };
        for a in &mut d.attachments {
            a.saved = None;
        }
        d.saved_images.clear();
        return Ok((Stored::Deleted(db.delete_draft(row)?), Vec::new()));
    }
    let shown: Vec<(String, model::ImageFile)> = d
        .rich
        .as_ref()
        .map(Editor::shown_images)
        .unwrap_or_default();
    let unsaved = d.attachments.iter().any(|a| a.saved.is_none())
        || shown
            .iter()
            .any(|(c, _)| !d.saved_images.iter().any(|(s, _)| s == c))
        || d.saved_images
            .iter()
            .any(|(s, _)| !shown.iter().any(|(c, _)| c == s));
    if d.row.is_some() && !d.dirty && !unsaved {
        return Ok((Stored::Nothing, Vec::new()));
    }
    if d.message_id.is_empty() {
        d.message_id = smtp::message_id(email);
    }
    let (row, stored) = match d.row {
        Some(row) if !d.dirty || db.update_draft(&d.to_row(row))? => (row, Stored::Updated),
        // The saved row is gone: save the draft again, copies included.
        Some(_) => {
            for a in &mut d.attachments {
                a.saved = None;
            }
            d.saved_images.clear();
            let row = db.insert_draft(&d.to_row(0))?;
            d.row = Some(row);
            (row, Stored::Inserted)
        }
        None => {
            let row = db.insert_draft(&d.to_row(0))?;
            d.row = Some(row);
            (row, Stored::Inserted)
        }
    };
    d.dirty = false;
    d.saved_at = Some(chrono::Utc::now().timestamp());
    let mut jobs = Vec::new();
    for a in d.attachments.iter_mut().filter(|a| a.saved.is_none()) {
        let mime = smtp::content_type(&a.file.filename);
        let (file, copy) = db.add_draft_row(
            row,
            &a.file.filename,
            mime,
            Arc::from(&a.file.data[..]),
            None,
        )?;
        a.saved = Some(file.id);
        jobs.push(FileJob {
            draft: row,
            file: file.id,
            copy,
        });
    }
    // Pictures no longer shown go; new ones are saved.
    let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut d.saved_images)
        .into_iter()
        .partition(|(s, _)| shown.iter().any(|(c, _)| c == s));
    d.saved_images = keep;
    for (_, id) in gone {
        db.remove_draft_file(row, id).ok();
    }
    for (cid, f) in shown {
        if d.saved_images.iter().any(|(s, _)| *s == cid) {
            continue;
        }
        let (file, copy) =
            db.add_draft_row(row, &f.file_name(&cid), f.mime, f.data.clone(), Some(&cid))?;
        d.saved_images.push((cid, file.id));
        jobs.push(FileJob {
            draft: row,
            file: file.id,
            copy,
        });
    }
    Ok((stored, jobs))
}

/// Load saved draft `row` with its attachments and pictures. Copies that
/// can't be read are left out (and stay saved); their names come back.
pub fn load(db: &Db, row: i64) -> anyhow::Result<Option<(Draft, Vec<String>)>> {
    let Some(job) = load_job(db, row)? else {
        return Ok(None);
    };
    Ok(Some(finish_load(db, job.run())))
}

/// What reading a saved draft needs: its row and where its copies are.
/// [`LoadJob::run`] reads the copies (off the UI thread).
pub struct LoadJob {
    row: db::DraftRow,
    files: Vec<(db::DraftFile, std::path::PathBuf)>,
}

/// A saved draft with its copies read.
pub struct Loaded {
    row: db::DraftRow,
    attachments: Vec<Attached>,
    images: Vec<(String, i64, Vec<u8>)>,
    missing: Vec<String>,
}

/// The first step of [`load`] (quick: rows only).
pub fn load_job(db: &Db, row: i64) -> anyhow::Result<Option<LoadJob>> {
    let Some(r) = db.draft(row)? else {
        return Ok(None);
    };
    let mut files = Vec::new();
    for f in db.draft_files(row)? {
        let path = db.draft_file_path(row, &f)?;
        files.push((f, path));
    }
    Ok(Some(LoadJob { row: r, files }))
}

impl LoadJob {
    /// Read the copies.
    /// Whether it has copies to read (none: run it anywhere at once).
    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    pub fn run(self) -> Loaded {
        let mut attachments = Vec::new();
        let mut images = Vec::new();
        let mut missing = Vec::new();
        for (f, path) in self.files {
            // No limit: Komorebi sets none on attachments.
            match db::read_draft_copy(&path, &f.name, u64::MAX) {
                Ok(data) => match f.cid {
                    Some(cid) => images.push((cid, f.id, data)),
                    None => attachments.push(Attached {
                        file: OutAttachment {
                            filename: f.name,
                            data,
                        },
                        saved: Some(f.id),
                    }),
                },
                // A picture lost is left out of the message as it is
                // shown (`rich::send_parts`); an attachment is named.
                Err(_) if f.cid.is_some() => {}
                Err(_) => missing.push(f.name),
            }
        }
        Loaded {
            row: self.row,
            attachments,
            images,
            missing,
        }
    }
}

/// The last step of [`load`]: the draft from what was read.
pub fn finish_load(db: &Db, l: Loaded) -> (Draft, Vec<String>) {
    let mut d = Draft::from_row(l.row, l.attachments);
    let mut images = model::Images::default();
    for (cid, id, data) in l.images {
        if let Some((file, _, _)) = model::ImageFile::new(data) {
            images.insert(cid.clone(), file);
            d.saved_images.push((cid, id));
        }
    }
    if let Some(ed) = &mut d.rich {
        ed.add_images(&images);
        self::rich::missing_images(db, ed);
    }
    (d, l.missing)
}

/// Whether an autosave also copies its draft to the server, the last copy
/// having been asked for at `last`: at most one per `SERVER_SAVE_EVERY`.
pub fn server_save_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= SERVER_SAVE_EVERY)
}

/// How much longer an autosave waits when it comes due: edits less than
/// `AUTOSAVE_AFTER` before `now` push it back (`None`: save now). So edits
/// in quick succession end in one save, two seconds after the last.
pub fn autosave_wait(last_edit: Instant, now: Instant) -> Option<Duration> {
    AUTOSAVE_AFTER
        .checked_sub(now.saturating_duration_since(last_edit))
        .filter(|d| !d.is_zero())
}

/// The signature an account puts in a new message, or in a reply or
/// forward (none: it has none, or the cache could not be read).
pub fn signature_for(db: &Db, account: i64, kind: Kind) -> Option<db::Signature> {
    db.default_signature(account, kind != Kind::New)
        .ok()
        .flatten()
}

/// A draft's form as the composer edits it: formatted (unless the
/// settings say plain text) with its account's signature, the quote
/// folded.
pub fn prepare(db: &Db, draft: &mut Draft, defaults: &ComposeDefaults) {
    // A new message starts formatted unless Settings says plain text.
    if draft.row.is_none() && draft.rich.is_none() && defaults.rich {
        let sig = signature_for(db, draft.account_id, draft.kind);
        draft.make_rich(defaults.doc(), sig.as_ref().and_then(db::Signature::block));
        // The signature's pictures are copied into the draft with it.
        if let (Some(s), Some(ed)) = (&sig, &mut draft.rich) {
            ed.add_images(&s.images);
        }
    }
    if draft.rich.is_none() {
        let block = signature_for(db, draft.account_id, draft.kind)
            .map_or_else(String::new, |s| signature_block(&s.body_text));
        if draft.row.is_none() {
            draft.sign(&block);
        } else if block.is_empty() || draft.body.contains(&block) {
            // Saved earlier: swappable while it holds the current block.
            draft.signature = Some(block);
        }
        draft.fold_quote();
    }
    draft.rich_quote = None;
}

/// The addresses a draft has already (bare and lowercase), which
/// suggestions leave out.
fn taken(d: &Draft) -> HashSet<String> {
    [&d.to, &d.cc, &d.bcc]
        .into_iter()
        .flat_map(|list| smtp::split_addresses(list))
        .map(|entry| smtp::mailbox_parts(&entry).1.to_lowercase())
        .collect()
}

/// Recipient suggestions for the address at `cursor` in `field`, from the
/// draft account's address index: once it has [`SUGGEST_AFTER`]
/// characters, leaving out addresses the draft already has. None: nothing
/// to show.
pub fn suggest(db: &Db, d: &Draft, field: Field, cursor: usize) -> Option<Suggest> {
    let typed = typed_address(d.field(field), cursor);
    if typed.chars().count() < SUGGEST_AFTER {
        return None;
    }
    let taken = taken(d);
    let items: Vec<_> = db
        .suggest_recipients(d.account_id, typed, SUGGESTIONS + taken.len())
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !taken.contains(&r.addr))
        .take(SUGGESTIONS)
        .collect();
    (!items.is_empty()).then_some(Suggest {
        field,
        cursor,
        items,
    })
}

/// Suggestion `index` taken: the field's new text (it replaces the address
/// being typed, as `Name <addr>, `) and the caret after it. A group adds
/// each member not already in the message.
pub fn accept_suggestion(d: &Draft, s: &Suggest, index: usize) -> Option<(String, usize)> {
    let r = s.items.get(index)?;
    let mailbox = match &r.kind {
        db::contacts::SuggestKind::Group(members) => {
            let taken = taken(d);
            members
                .iter()
                .filter(|m| !taken.contains(&m.addr))
                .map(|m| smtp::mailbox(&m.name, &m.addr))
                .collect::<Vec<_>>()
                .join(", ")
        }
        _ => smtp::mailbox(&r.name, &r.addr),
    };
    Some(complete_address(d.field(s.field), s.cursor, &mailbox))
}
